//! Global hotkey definitions and dispatch.
//!
//! The renderer subscribes to `hotkey:toggle-input`,
//! `hotkey:vision-region`, `hotkey:overlay` and `hotkey:toggle-window` (see
//! `src/hooks/useHotkeys.ts`).

use tauri::Emitter;
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};

/// The global hotkeys Komorebi registers. Returned as a tuple so the
/// builder can both register them AND capture them in its handler closure
/// for equality checks.
pub(crate) fn defaults() -> (Shortcut, Shortcut, Shortcut, Shortcut) {
    let toggle_input = Shortcut::new(Some(Modifiers::ALT), Code::Space);
    let vision_region = Shortcut::new(Some(Modifiers::ALT), Code::KeyV);
    let overlay = Shortcut::new(Some(Modifiers::ALT), Code::KeyO);
    let toggle_window = Shortcut::new(None, Code::F11);
    (toggle_input, vision_region, overlay, toggle_window)
}

/// Forward a pressed shortcut to the renderer as a Tauri event.
/// Callers must already have filtered out non-`Pressed` states.
pub(crate) fn dispatch<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    shortcut: &Shortcut,
    toggle_input: &Shortcut,
    vision_region: &Shortcut,
    overlay: &Shortcut,
    toggle_window: &Shortcut,
) {
    if shortcut == toggle_input {
        if let Err(e) = app.emit("hotkey:toggle-input", ()) {
            tracing::warn!(?e, "failed to emit toggle-input");
        }
    } else if shortcut == vision_region {
        if let Err(e) = app.emit("hotkey:vision-region", ()) {
            tracing::warn!(?e, "failed to emit vision-region");
        }
    } else if shortcut == overlay {
        if let Err(e) = app.emit("hotkey:overlay", ()) {
            tracing::warn!(?e, "failed to emit overlay");
        }
    } else if shortcut == toggle_window {
        if let Err(e) = app.emit("hotkey:toggle-window", ()) {
            tracing::warn!(?e, "failed to emit toggle-window");
        }
    }
}
