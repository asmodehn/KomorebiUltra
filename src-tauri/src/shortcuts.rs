//! Global hotkey definitions and dispatch.
//!
//! The renderer subscribes to `hotkey:toggle-input` and
//! `hotkey:vision-region` (see `src/hooks/useHotkeys.ts`). Neither changes the
//! presentation: there is one overlay surface.

use tauri::Emitter;
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};

/// The global hotkeys Komorebi registers. Returned as a tuple so the
/// builder can both register them AND capture them in its handler closure
/// for equality checks.
pub(crate) fn defaults() -> (Shortcut, Shortcut) {
    let toggle_input = Shortcut::new(Some(Modifiers::ALT), Code::Space);
    let vision_region = Shortcut::new(Some(Modifiers::ALT), Code::KeyV);
    (toggle_input, vision_region)
}

/// Forward a pressed shortcut to the renderer as a Tauri event.
/// Callers must already have filtered out non-`Pressed` states.
pub(crate) fn dispatch<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    shortcut: &Shortcut,
    toggle_input: &Shortcut,
    vision_region: &Shortcut,
) {
    if shortcut == toggle_input {
        if let Err(e) = app.emit("hotkey:toggle-input", ()) {
            tracing::warn!(?e, "failed to emit toggle-input");
        }
    } else if shortcut == vision_region {
        if let Err(e) = app.emit("hotkey:vision-region", ()) {
            tracing::warn!(?e, "failed to emit vision-region");
        }
    }
}
