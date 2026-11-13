//! Window overlay / frontmost management.
//!
//! Tauri (config) handles: `alwaysOnTop`, `transparent`, `decorations: false`,
//! `skipTaskbar`, `focus: false`.  What Tauri *cannot* do is force a window to
//! float on tiling compositors (Wayland layer-shell / X11 `_NET_WM_STATE`
//! hints) or make a window ignore pointer input on Windows
//! (`WS_EX_TRANSPARENT`) / macOS (`ignoresMouseEvents`).  Those need native
//! code, isolated here so the rest of the app stays platform-agnostic.
//!
//! Modes:
//!   - `normal`        — plain Tauri window (default, tiles on tiling WMs)
//!   - `overlay`       — covers the monitor and stays frontmost, native
//!     non-tiling where the platform allows it.
//!
//! Covering the monitor is *not* compositor fullscreen: fullscreen loses the
//! surface alpha on Wayland (the window renders black despite a transparent
//! page). On Linux the overlay is a layer-shell surface instead; elsewhere it
//! is a maximised window. See `linux.rs`.
//!
//! A layer surface cannot become a toplevel, so switching the window *type*
//! recreates the window: the old window is destroyed and a fresh one is built
//! from `tauri.conf.json` inside the same process, leaving the Tauri context
//! (and everything in it: hotkeys, tray, models, chat state) warm. See
//! `create_main_window` and the `Destroyed` handler in `lib.rs`.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::WebviewWindow;

/// Effective mode last applied, so native/debug code can report it.
static CURRENT_MODE: Mutex<Option<OverlayMode>> = Mutex::new(None);

/// A window-type change in flight: the mode the replacement must be built in,
/// and when the switch was requested (for the timing log). The replacement is
/// created from the `Destroyed` handler (see `lib.rs`), and the app must not
/// exit in between.
static PENDING_SWITCH: Mutex<Option<PendingSwitch>> = Mutex::new(None);

#[derive(Clone, Copy)]
struct PendingSwitch {
    mode: OverlayMode,
    started: std::time::Instant,
}

/// Effective mode of the window (defaults to `Normal` before the first apply).
pub fn current_mode() -> OverlayMode {
    CURRENT_MODE
        .lock()
        .map(|slot| slot.unwrap_or_default())
        .unwrap_or_default()
}

/// Serialized to the frontend so it can reflect the effective mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OverlayMode {
    #[default]
    Normal,
    Overlay,
}

/// Platform capability probe.
#[cfg(target_os = "linux")]
pub fn overlay_supported() -> bool {
    linux::overlay_supported()
}

#[cfg(not(target_os = "linux"))]
pub fn overlay_supported() -> bool {
    false
}

/// Apply an overlay mode to a Tauri webview window.
///
/// Returns the mode actually in effect (may be downgraded on platforms where
/// the requested mode isn't available).
pub fn apply_overlay(window: &WebviewWindow, mode: OverlayMode) -> OverlayMode {
    let effective = platform_apply(window, mode);

    // Tauri's own always-on-top is the cross-platform half of "frontmost".
    // It must be set for every mode, otherwise `normal` would be a no-op
    // because `tauri.conf.json` starts the window as `alwaysOnTop: true`.
    let on_top = matches!(effective, OverlayMode::Overlay);
    if let Err(e) = window.set_always_on_top(on_top) {
        tracing::warn!(?e, on_top, "failed to set always-on-top");
    }

    if let Ok(mut slot) = CURRENT_MODE.lock() {
        *slot = Some(effective);
    }

    // An overlay covers the screen. Compositor *fullscreen* would do that too,
    // but on Wayland it loses the surface alpha (verified: the window painted
    // black while the page snapshot stayed transparent), so fullscreen is
    // never used.
    if matches!(effective, OverlayMode::Overlay) {
        if layer_shell_active() {
            // Anchored to every edge by the compositor: nothing to do.
            tracing::info!("overlay: layer surface covers the monitor (no action needed)");
        } else {
            #[cfg(target_os = "linux")]
            fill_monitor(window);
            // X11/Windows/macOS: the compositor positions a maximised window
            // for us (work area), and alpha survives maximising.
            #[cfg(not(target_os = "linux"))]
            if let Err(e) = window.set_maximized(true) {
                tracing::warn!(?e, "overlay: failed to maximize");
            }
        }
    } else {
        #[cfg(not(target_os = "linux"))]
        if let Err(e) = window.set_maximized(false) {
            tracing::warn!(?e, "overlay: failed to leave maximized");
        }
    }

    effective
}

/// Best-effort monitor coverage when no layer surface is available: the size
/// is honoured, the position is not (Wayland). On X11 this also moves the
/// window to the monitor origin.
#[cfg(target_os = "linux")]
fn fill_monitor(window: &WebviewWindow) {
    let Some(monitor) = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten())
    else {
        tracing::warn!("overlay: no monitor to fill");
        return;
    };
    let size = monitor.size();
    let pos = monitor.position();
    tracing::info!(
        ?size,
        "overlay: fill_monitor -> setting window size to monitor size"
    );
    if let Err(e) = window.set_size(tauri::PhysicalSize::new(size.width, size.height)) {
        tracing::warn!(?e, "overlay: failed to resize the window to the monitor");
    } else {
        tracing::info!("overlay: window resized to monitor size");
    }
    if let Err(e) = window.set_position(tauri::PhysicalPosition::new(pos.x, pos.y)) {
        tracing::debug!(
            ?e,
            "overlay: compositor ignored the window position (expected on Wayland)"
        );
    }
}

/// True when the platform already covers the screen with a layer surface.
#[cfg(target_os = "linux")]
fn layer_shell_active() -> bool {
    linux::layer_shell_active()
}

#[cfg(not(target_os = "linux"))]
fn layer_shell_active() -> bool {
    false
}

/// One-time window setup that must happen before the window is first mapped
/// (`startup` calls it right after the app is built). Attaches the layer
/// surface when `KOMOREBI_LAYER_SHELL` asks for it.
pub fn init_layer_shell(window: &WebviewWindow) {
    #[cfg(target_os = "linux")]
    linux::init_layer_shell(window);
    #[cfg(not(target_os = "linux"))]
    let _ = window;
}

/// Explicit variant used when recreating the window for a mode change: the
/// requested mode, not the (startup-time) environment, decides the window type.
pub fn init_layer_shell_for(window: &WebviewWindow, mode: OverlayMode) {
    #[cfg(target_os = "linux")]
    linux::set_layer_shell(
        window,
        matches!(mode, OverlayMode::Overlay) && layer_shell_supported(),
    );
    #[cfg(not(target_os = "linux"))]
    let _ = (window, mode);
}

/// Whether this build/run can realize `overlay` as a Wayland layer surface.
#[cfg(target_os = "linux")]
pub fn layer_shell_supported() -> bool {
    linux::layer_shell_supported()
}

#[cfg(not(target_os = "linux"))]
pub fn layer_shell_supported() -> bool {
    false
}

/// True when the requested mode cannot be reached by mutating the current
/// window: a layer surface cannot become a toplevel, and a mapped window
/// cannot be promoted to one. Overlay is a layer surface whenever the
/// platform supports it, so the two modes are two different window types.
#[cfg(target_os = "linux")]
pub fn needs_recreate(mode: OverlayMode) -> bool {
    linux::needs_recreate(mode)
}

#[cfg(not(target_os = "linux"))]
pub fn needs_recreate(_mode: OverlayMode) -> bool {
    false
}

/// `KOMOREBI_START_MODE`, when it names a mode (an explicit launch, which
/// overrides the remembered choice). The legacy `always_on_top` label is
/// folded into `overlay`: it was never a mode of its own.
pub fn start_mode() -> Option<OverlayMode> {
    let raw = std::env::var("KOMOREBI_START_MODE").ok()?;
    match raw.as_str() {
        "overlay" | "always_on_top" => Some(OverlayMode::Overlay),
        "normal" => Some(OverlayMode::Normal),
        other => {
            tracing::warn!(mode = other, "unknown KOMOREBI_START_MODE, ignoring");
            None
        }
    }
}

/// Record the mode a replacement window should be created in. Set before the
/// old window is destroyed; cleared by [`finish_switch`].
pub fn request_switch(mode: OverlayMode) {
    if let Ok(mut slot) = PENDING_SWITCH.lock() {
        *slot = Some(PendingSwitch {
            mode,
            started: std::time::Instant::now(),
        });
    }
}

/// The mode the replacement window must be built in, if a switch is pending.
pub fn pending_switch() -> Option<OverlayMode> {
    PENDING_SWITCH
        .lock()
        .ok()
        .and_then(|slot| slot.map(|p| p.mode))
}

/// True while a window is being replaced (nothing must exit the app then).
pub fn switch_in_flight() -> bool {
    pending_switch().is_some()
}

/// The switch is over (the replacement exists, or the rebuild failed): stop
/// preventing the app from exiting. Returns how long the rebuild took.
pub fn finish_switch() -> Option<u64> {
    PENDING_SWITCH
        .lock()
        .ok()
        .and_then(|mut slot| slot.take())
        .map(|pending| pending.started.elapsed().as_millis() as u64)
}

/// Build the main window from `tauri.conf.json` in `mode`.
///
/// The window is created hidden, the layer surface is attached (or not) while
/// it is still unmapped, the mode is applied, and only then is it shown. The
/// Tauri app itself is untouched: hotkeys, tray, RAG index, models and chat
/// state all survive the switch.
///
/// Must run on the main thread (the `Destroyed` handler posts it there).
pub fn create_main_window(
    app: &tauri::AppHandle,
    mode: OverlayMode,
) -> tauri::Result<WebviewWindow> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == "main")
        .cloned()
        .ok_or(tauri::Error::WindowNotFound)?;

    let window = tauri::WebviewWindowBuilder::from_config(app, &config)?.build()?;
    init_layer_shell_for(&window, mode);
    apply_overlay(&window, mode);
    // The window exists from here on: a failing `show` is logged, not fatal.
    if let Err(e) = window.show() {
        tracing::warn!(?e, "overlay: could not show the recreated window");
    }
    Ok(window)
}

#[cfg(target_os = "linux")]
fn platform_apply(window: &WebviewWindow, mode: OverlayMode) -> OverlayMode {
    linux::apply_overlay(window, mode)
}

#[cfg(not(target_os = "linux"))]
fn platform_apply(_window: &WebviewWindow, mode: OverlayMode) -> OverlayMode {
    // `overlay` means something here too (frontmost + covering the screen),
    // so it must not be downgraded to a plain always-on-top window.
    mode
}

#[cfg(target_os = "linux")]
mod linux;
