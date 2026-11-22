//! Dev-only agent harness: token-friendly frames plus a file command channel.
//!
//! Enabled by `KOMOREBI_AGENT_DIR=<dir>` (debug builds only). Every
//! `POLL_MS` it:
//!
//! * executes the newline-separated commands in `<dir>/cmd`, then deletes it,
//! * writes `<dir>/state.json` when it changes,
//! * captures `<dir>/frame-<n>.png` on demand (`frame`, `watch <ms>`).
//!
//! Commands:
//!
//! ```text
//! frame                 capture one frame now
//! watch <ms>            capture every <ms> ms (0 = off)
//! eval <js>             run JS in the webview (drives the real UI)
//! ```
//!
//! The frame is the window area only: a PNG of the webview (see
//! `crate::window::frame`), downscaled to at most `MAX_W` px wide and
//! flattened over a checkerboard so transparent (click-through) areas are
//! visible. `state.json` carries the same frame as text: window geometry,
//! the selected surface backend, OS input rects and the frontend's UI snapshot.

use tauri::AppHandle;

/// Stores the latest UI snapshot pushed by the frontend (dev harness only).
pub fn report(json: String) {
    #[cfg(debug_assertions)]
    imp::report(json);
    #[cfg(not(debug_assertions))]
    let _ = json;
}

pub fn spawn(app: AppHandle) {
    #[cfg(debug_assertions)]
    imp::spawn(app);
    #[cfg(not(debug_assertions))]
    let _ = app;
}

#[cfg(debug_assertions)]
mod imp {
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    use tauri::{AppHandle, Manager, WebviewWindow};

    use crate::window::capture_frame;

    const POLL_MS: u64 = 250;
    /// Wait for a repaint before an on-demand capture. A snapshot taken in the
    /// same tick as an `eval` can still show the old frame, which reads as
    /// "the click did nothing".
    const SETTLE_MS: u64 = 300;
    /// Frames are tokenized by area; keep them small by default.
    const MAX_W: u32 = 480;
    const CHECKER: u32 = 8;

    static UI_SNAPSHOT: Mutex<Option<String>> = Mutex::new(None);
    static STARTED: OnceLock<Instant> = OnceLock::new();

    pub(super) fn report(json: String) {
        if let Ok(mut slot) = UI_SNAPSHOT.lock() {
            *slot = Some(json);
        }
    }

    pub(super) fn spawn(app: AppHandle) {
        let Some(dir) = std::env::var_os("KOMOREBI_AGENT_DIR").map(PathBuf::from) else {
            return;
        };
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!(?e, ?dir, "agent frame: cannot create dir");
            return;
        }
        STARTED.get_or_init(Instant::now);
        tracing::info!(?dir, "agent frame harness enabled");
        std::thread::spawn(move || run(app, dir));
    }

    fn run(app: AppHandle, dir: PathBuf) {
        let mut seq = 0u32;
        let mut watch_ms = 0u64;
        let mut last_watch = Instant::now();
        let mut last_state = String::new();

        loop {
            std::thread::sleep(Duration::from_millis(POLL_MS));
            let Some(win) = app.get_webview_window("main") else {
                continue;
            };

            if let Ok(text) = std::fs::read_to_string(dir.join("cmd")) {
                let _ = std::fs::remove_file(dir.join("cmd"));
                for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
                    exec(&win, &dir, line, &mut seq, &mut watch_ms);
                }
            }

            if watch_ms > 0 && last_watch.elapsed() >= Duration::from_millis(watch_ms) {
                last_watch = Instant::now();
                shoot(&win, &dir, &mut seq, 0);
            }

            let state = state_json(&win);
            if state != last_state {
                let _ = std::fs::write(dir.join("state.json"), &state);
                last_state = state;
            }
        }
    }

    fn exec(win: &WebviewWindow, dir: &Path, line: &str, seq: &mut u32, watch_ms: &mut u64) {
        if let Some(js) = line.strip_prefix("eval ") {
            if let Err(e) = win.eval(js) {
                tracing::warn!(?e, "agent frame: eval failed");
            }
            return;
        }
        if let Some(ms) = line.strip_prefix("watch ") {
            *watch_ms = ms.trim().parse().unwrap_or(0);
            tracing::info!(watch_ms, "agent frame: watch interval");
            return;
        }
        match line {
            "frame" => shoot(win, dir, seq, SETTLE_MS),
            "hide" => {
                let _ = win.hide();
                tracing::info!("agent frame: window hidden");
            }
            "focus" => {
                // `focus: false` in tauri.conf.json makes tao create the
                // window with accept-focus off; restore it and activate.
                let _ = win.set_focusable(true);
                match win.set_focus() {
                    Ok(()) => tracing::info!("agent frame: window focused"),
                    Err(e) => tracing::warn!(?e, "agent frame: focus failed"),
                }
            }
            "gtk" => gtk_probe(win),
            "show" => {
                let _ = win.show();
                tracing::info!("agent frame: window shown");
                shoot(win, dir, seq, SETTLE_MS);
            }
            other => tracing::warn!(cmd = other, "agent frame: unknown command"),
        }
    }

    /// Dump GTK focus state: whether keys can reach the webview at all.
    fn gtk_probe(win: &WebviewWindow) {
        let gtk_win = match win.gtk_window() {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(?e, "agent frame: no gtk window");
                return;
            }
        };
        use gtk::prelude::*;
        tracing::info!(
            active = gtk_win.is_active(),
            toplevel_focus = gtk_win.has_toplevel_focus(),
            accept_focus = gtk_win.accepts_focus(),
            focused = win.is_focused().unwrap_or(false),
            "agent frame: gtk window state"
        );
        let _ = win.with_webview(|platform| {
            let view = platform.inner();
            tracing::info!(
                widget_focus = view.has_focus(),
                can_focus = view.can_focus(),
                is_focus = view.is_focus(),
                "agent frame: webview focus state"
            );
        });
    }

    fn shoot(win: &WebviewWindow, dir: &Path, seq: &mut u32, settle_ms: u64) {
        if settle_ms > 0 {
            std::thread::sleep(Duration::from_millis(settle_ms));
        }
        let png = match capture_frame(win) {
            Ok(png) => png,
            Err(e) => {
                tracing::warn!(%e, "agent frame: capture failed");
                return;
            }
        };
        let small = match shrink(&png) {
            Ok(png) => png,
            Err(e) => {
                tracing::warn!(%e, "agent frame: shrink failed");
                return;
            }
        };
        *seq += 1;
        let path = dir.join(format!("frame-{seq:04}.png"));
        if let Err(e) = std::fs::write(&path, small) {
            tracing::warn!(?e, ?path, "agent frame: write failed");
        } else {
            tracing::info!(?path, "agent frame: captured");
        }
    }

    /// Downscale to `MAX_W` and flatten over a checkerboard so transparent
    /// areas are recognizable instead of silently black.
    fn shrink(png: &[u8]) -> Result<Vec<u8>, String> {
        let img = image::load_from_memory(png)
            .map_err(|e| e.to_string())?
            .to_rgba8();
        let (w, h) = img.dimensions();
        if w == 0 || h == 0 {
            return Err("empty frame".into());
        }
        let scale = (MAX_W as f32 / w as f32).min(1.0);
        let nw = ((w as f32 * scale).round() as u32).max(1);
        let nh = ((h as f32 * scale).round() as u32).max(1);
        let small = image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle);

        let mut out = image::RgbaImage::new(nw, nh);
        for (x, y, p) in out.enumerate_pixels_mut() {
            let dark = ((x / CHECKER) + (y / CHECKER)).is_multiple_of(2);
            *p = if dark {
                image::Rgba([32, 32, 40, 255])
            } else {
                image::Rgba([44, 44, 56, 255])
            };
        }
        image::imageops::overlay(&mut out, &small, 0, 0);

        let mut buf = Vec::new();
        out.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        Ok(buf)
    }

    fn state_json(win: &WebviewWindow) -> String {
        let started = STARTED.get_or_init(Instant::now);
        let size = win.inner_size().ok();
        let scale = win.scale_factor().unwrap_or(1.0);
        let ui = UI_SNAPSHOT
            .lock()
            .ok()
            .and_then(|s| s.clone())
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());

        serde_json::json!({
            "t_ms": started.elapsed().as_millis() as u64,
            "mode": "overlay",
            "backend": crate::platform::surface().name(),
            "window": {
                "label": win.label(),
                "visible": win.is_visible().unwrap_or(false),
                "focused": win.is_focused().unwrap_or(false),
                "size": size.map(|s| serde_json::json!({ "w": s.width, "h": s.height })),
                "scale": scale,
            },
            "input_rects": crate::commands::overlay::last_rects(),
            "ui": ui,
        })
        .to_string()
    }
}
