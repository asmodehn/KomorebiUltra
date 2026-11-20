//! Tauri commands for the window overlay / frontmost mode.
//!
//! Delegates to `crate::window`, which isolates the platform-native code
//! (Wayland layer-shell, X11 hints, future Windows/macOS backends).

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewWindow, Wry};

use crate::window::{
    apply_overlay, finish_switch, needs_recreate, overlay_supported, request_switch, OverlayMode,
};

/// Current overlay capability + effective mode, reflected to the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct OverlayInfo {
    pub supported: bool,
    pub mode: OverlayMode,
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
pub fn set_window_overlay(
    app: AppHandle<Wry>,
    window: WebviewWindow,
    mode: OverlayMode,
) -> OverlayMode {
    // Stored before the window changes: a window-type switch destroys this
    // window and reloads the page, so the next launch must find the choice in
    // the settings store, where `startup` reads it before mapping the window.
    // A platform that cannot honour the choice (the mode is downgraded) stores
    // what is actually in effect instead.
    let _ = crate::settings::set_overlay_mode(&app, mode.name());
    let effective = switch_mode(&window, mode);
    if effective != mode {
        let _ = crate::settings::set_overlay_mode(&app, effective.name());
    }
    effective
}

/// Let the overlay accept keyboard focus after the user has picked it.
///
/// A layer surface must map with `KeyboardMode::None`: on COSMIC, asking for
/// the keyboard while the surface appears hands it the focus immediately. The
/// page calls this on its first pointer event over the character or the UI
/// (elsewhere the overlay is click-through), so a click on the chat input can
/// focus it without the window grabbing the keyboard at startup.
#[tauri::command]
pub fn overlay_keyboard_on_demand(window: WebviewWindow) {
    crate::window::allow_keyboard(&window);
}

#[tauri::command]
pub fn window_overlay_info() -> OverlayInfo {
    OverlayInfo {
        supported: overlay_supported(),
        mode: crate::window::current_mode(),
    }
}

/// Dev harness: UI snapshot pushed by the frontend (`src/agentFrame.ts`).
/// No-op outside debug builds; see `crate::agent_frame`.
#[tauri::command]
pub fn agent_report(json: String) {
    crate::agent_frame::report(json);
}
