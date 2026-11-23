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
                for command in parse_commands(&text) {
                    exec(&win, &dir, command, &mut seq, &mut watch_ms);
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

    #[derive(Debug, PartialEq)]
    enum Command<'a> {
        Eval(&'a str),
        Watch(u64),
        Frame,
        Hide,
        Focus,
        Gtk,
        Show,
        Unknown(&'a str),
    }

    fn parse_commands(text: &str) -> Vec<Command<'_>> {
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(parse_command)
            .collect()
    }

    fn parse_command(line: &str) -> Command<'_> {
        if let Some(js) = line.strip_prefix("eval ") {
            return Command::Eval(js);
        }
        if let Some(ms) = line.strip_prefix("watch ") {
            return Command::Watch(ms.trim().parse().unwrap_or(0));
        }
        match line {
            "frame" => Command::Frame,
            "hide" => Command::Hide,
            "focus" => Command::Focus,
            "gtk" => Command::Gtk,
            "show" => Command::Show,
            other => Command::Unknown(other),
        }
    }

    fn exec(
        win: &WebviewWindow,
        dir: &Path,
        command: Command<'_>,
        seq: &mut u32,
        watch_ms: &mut u64,
    ) {
        match command {
            Command::Eval(js) => {
                if let Err(e) = win.eval(js) {
                    tracing::warn!(?e, "agent frame: eval failed");
                }
            }
            Command::Watch(ms) => {
                *watch_ms = ms;
                tracing::info!(watch_ms, "agent frame: watch interval");
            }
            Command::Frame => shoot(win, dir, seq, SETTLE_MS),
            Command::Hide => {
                let _ = win.hide();
                tracing::info!("agent frame: window hidden");
            }
            Command::Focus => {
                // `focus: false` in tauri.conf.json makes tao create the
                // window with accept-focus off; restore it and activate.
                let _ = win.set_focusable(true);
                match win.set_focus() {
                    Ok(()) => tracing::info!("agent frame: window focused"),
                    Err(e) => tracing::warn!(?e, "agent frame: focus failed"),
                }
            }
            Command::Gtk => gtk_probe(win),
            Command::Show => {
                let _ = win.show();
                tracing::info!("agent frame: window shown");
                shoot(win, dir, seq, SETTLE_MS);
            }
            Command::Unknown(other) => tracing::warn!(cmd = other, "agent frame: unknown command"),
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
        let size = win.inner_size().ok().map(|s| (s.width, s.height));
        let scale = win.scale_factor().unwrap_or(1.0);
        let ui = UI_SNAPSHOT
            .lock()
            .ok()
            .and_then(|s| s.clone())
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());

        state_value(StateSnapshot {
            t_ms: started.elapsed().as_millis() as u64,
            backend: crate::platform::surface().name(),
            label: win.label(),
            visible: win.is_visible().unwrap_or(false),
            focused: win.is_focused().unwrap_or(false),
            size,
            scale,
            rects: crate::commands::overlay::last_rects(),
            ui,
        })
        .to_string()
    }

    struct StateSnapshot<'a> {
        t_ms: u64,
        backend: &'a str,
        label: &'a str,
        visible: bool,
        focused: bool,
        size: Option<(u32, u32)>,
        scale: f64,
        rects: Vec<(i32, i32, i32, i32)>,
        ui: Option<serde_json::Value>,
    }

    fn state_value(s: StateSnapshot<'_>) -> serde_json::Value {
        serde_json::json!({
            "t_ms": s.t_ms,
            "mode": "overlay",
            "backend": s.backend,
            "window": {
                "label": s.label,
                "visible": s.visible,
                "focused": s.focused,
                "size": s.size.map(|(w, h)| serde_json::json!({ "w": w, "h": h })),
                "scale": s.scale,
            },
            "input_rects": s.rects,
            "ui": s.ui,
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Given a command file with blank lines and surrounding spaces
        /// When the harness parses its text
        /// Then each nonblank line becomes the command it names
        /// And unknown or differently cased lines are not treated as commands
        #[test]
        fn file_commands_are_newline_separated_and_exact() {
            let commands = parse_commands(
                "frame\n watch 2000 \n\neval document.title\nunknown\nFrame\n watch 0 \n",
            );
            assert_eq!(
                commands,
                vec![
                    Command::Frame,
                    Command::Watch(2000),
                    Command::Eval("document.title"),
                    Command::Unknown("unknown"),
                    Command::Unknown("Frame"),
                    Command::Watch(0),
                ]
            );
        }

        /// Given a frontend snapshot, a selected backend and the last OS input region
        /// When the harness publishes its state
        /// Then the state describes one overlay window and carries the snapshot and region unchanged
        #[test]
        fn state_describes_one_overlay_window() {
            let ui = serde_json::json!({
                "vw": 640,
                "vh": 400,
                "nodes": [{"tag": "button", "label": "Reset conversation", "pe": "auto"}],
                "focus": {"tag": "input", "value": "draft"},
                "keys": [],
            });
            let state = state_value(StateSnapshot {
                t_ms: 1234,
                backend: "plain-window",
                label: "main",
                visible: true,
                focused: false,
                size: Some((640, 400)),
                scale: 2.0,
                rects: vec![(10, 20, 30, 40)],
                ui: Some(ui.clone()),
            });
            assert_eq!(
                state,
                serde_json::json!({
                    "t_ms": 1234,
                    "mode": "overlay",
                    "backend": "plain-window",
                    "window": {
                        "label": "main",
                        "visible": true,
                        "focused": false,
                        "size": {"w": 640, "h": 400},
                        "scale": 2.0,
                    },
                    "input_rects": [[10, 20, 30, 40]],
                    "ui": ui,
                })
            );
        }

        /// Given a harness with no frontend snapshot yet
        /// When the harness publishes its state
        /// Then missing values are null instead of invented data
        /// And the surface is still reported as the one overlay mode
        #[test]
        fn state_without_frontend_snapshot_has_no_invented_ui() {
            let state = state_value(StateSnapshot {
                t_ms: 0,
                backend: "wayland-layer-shell",
                label: "main",
                visible: false,
                focused: false,
                size: None,
                scale: 1.0,
                rects: vec![],
                ui: None,
            });
            assert_eq!(state["mode"], "overlay");
            assert_eq!(state["backend"], "wayland-layer-shell");
            assert!(state["ui"].is_null());
            assert!(state["window"]["size"].is_null());
            assert_eq!(state["input_rects"], serde_json::json!([]));
        }

        /// Given a transparent webview frame containing one opaque control
        /// When the harness prepares the frame for an agent
        /// Then the PNG is at most 480 px wide with opaque checkerboard showing through transparency
        /// And the opaque control stays visible after downscaling
        #[test]
        fn frame_is_downscaled_and_transparency_is_visible() {
            let source = image::RgbaImage::from_fn(960, 480, |x, y| {
                if (100..200).contains(&x) && (100..200).contains(&y) {
                    image::Rgba([255, 0, 0, 255])
                } else {
                    image::Rgba([0, 0, 0, 0])
                }
            });
            let mut png = Vec::new();
            source
                .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .unwrap();

            let shrunk = shrink(&png).unwrap();
            let frame = image::load_from_memory(&shrunk).unwrap().to_rgba8();

            assert!(frame.width() <= 480);
            assert!(frame.height() > 0);
            assert!(frame.pixels().all(|p| p.0[3] == 255));
            let first = frame.get_pixel(0, 0);
            let varied = frame
                .enumerate_pixels()
                .any(|(x, y, p)| !(45..105).contains(&x) && !(45..105).contains(&y) && p != first);
            assert!(varied, "transparent window area must show a checkerboard");
            let control = frame.get_pixel(75, 75);
            assert_eq!(control.0[3], 255);
            assert_ne!(*control, *first);
        }
    }
}
