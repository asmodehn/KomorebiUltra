//! Overlay input region: which parts of the window take pointer input.
//!
//! Experimental, off unless `KOMOREBI_INPUT_REGION` is set. The frontend
//! reports the rects of its interactive elements (CSS px from the
//! webview's top-left); everywhere else the window is click-through.
//! Linux only for now (X11 and Wayland).

use std::sync::Mutex;
use tauri::WebviewWindow;

// The rect type lives with the surface contract so a backend can consume it
// without depending on this command module.
pub use crate::platform::InputRect;

const ENV_VAR: &str = "KOMOREBI_INPUT_REGION";

/// Last OS input region reported by the frontend, for the dev frame harness.
static LAST_RECTS: Mutex<Vec<(i32, i32, i32, i32)>> = Mutex::new(Vec::new());

/// The last input region as `(x, y, w, h)` tuples (empty = whole window).
pub fn last_rects() -> Vec<(i32, i32, i32, i32)> {
    LAST_RECTS.lock().map(|r| r.clone()).unwrap_or_default()
}

/// Restricts pointer input to `rects`. Returns `false` when the overlay
/// region is unsupported or disabled, so the caller can stop reporting.
/// An empty list restores input on the whole window.
#[tauri::command]
pub fn set_input_region(window: WebviewWindow, rects: Vec<InputRect>) -> Result<bool, String> {
    if let Ok(mut slot) = LAST_RECTS.lock() {
        *slot = rects.iter().map(|r| (r.x, r.y, r.w, r.h)).collect();
    }
    if std::env::var_os(ENV_VAR).is_none() {
        return Ok(false);
    }
    crate::platform::surface()
        .set_input_region(&window, &rects)
        .map_err(|e| e.to_string())
}
