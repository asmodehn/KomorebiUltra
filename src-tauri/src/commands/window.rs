//! Tauri commands for the window overlay / frontmost mode.
//!
//! Delegates to `crate::window`, which isolates the platform-native code
//! (Wayland layer-shell, X11 hints, future Windows/macOS backends).

use serde::{Deserialize, Serialize};
use tauri::{Manager, WebviewWindow};

use crate::window::{
    apply_overlay, finish_switch, needs_recreate, overlay_supported, request_switch, OverlayMode,
};

/// Current overlay capability + effective mode, reflected to the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct OverlayInfo {
    pub supported: bool,
    pub mode: OverlayMode,
    /// True when the mode was pinned by `KOMOREBI_START_MODE` (an explicit
    /// launch): the frontend must not override it with the remembered
    /// `localStorage` value.
    pub forced: bool,
}

/// Apply `mode`, replacing the window when the window *type* has to change.
///
/// A layer surface cannot be converted back to a normal toplevel (and a mapped
/// window cannot be promoted), so overlay (layer surface) ↔ windowed needs a
/// new window. The window is destroyed here and rebuilt in-process from the
/// `Destroyed` handler in `lib.rs`: the Tauri context, hotkeys, tray, models
/// and chat state survive, only the window + webview are recreated.
///
/// The command returns the requested mode immediately: the replacement window
/// appears a moment later, and the reloaded frontend picks the mode up from
/// `window_overlay_info`.
pub(crate) fn switch_mode(window: &WebviewWindow, mode: OverlayMode) -> OverlayMode {
    if !needs_recreate(mode) {
        return apply_overlay(window, mode);
    }

    request_switch(mode);
    match window.destroy() {
        Ok(()) => {
            tracing::info!(?mode, "overlay: replacing the window (in-process switch)");
            mode
        }
        Err(e) => {
            finish_switch();
            tracing::warn!(
                ?e,
                "overlay: could not destroy the window; applying the mode in place"
            );
            apply_overlay(window, mode)
        }
    }
}

#[tauri::command]
pub fn set_window_overlay(window: WebviewWindow, mode: OverlayMode) -> OverlayMode {
    switch_mode(&window, mode)
}

#[tauri::command]
pub fn window_overlay_info() -> OverlayInfo {
    OverlayInfo {
        supported: overlay_supported(),
        mode: crate::window::current_mode(),
        forced: crate::window::start_mode().is_some(),
    }
}
