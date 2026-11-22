//! Tauri commands for the window overlay / frontmost presentation.
//!
//! Delegates to `crate::window`, which isolates the platform-native code
//! (Wayland layer-shell, future Windows/macOS backends).

use tauri::WebviewWindow;

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

/// Dev harness: UI snapshot pushed by the frontend (`src/agentFrame.ts`).
/// No-op outside debug builds; see `crate::agent_frame`.
#[tauri::command]
pub fn agent_report(json: String) {
    crate::agent_frame::report(json);
}
