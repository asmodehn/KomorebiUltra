//! Remembered window mode (frontmost overlay vs plain windowed).
//!
//! The mode lives here rather than in the webview's own storage because the
//! backend has to know it *before* it maps the window: a window mapped as the
//! default toplevel is picked up — and focused — by the compositor, and the
//! page can only replace it by asking for a new window, which the user sees as
//! a small window flashing past and taking the keyboard focus of whatever they
//! were typing in.

use super::store::{read_string, write_optional_string};
use super::Result;
use tauri::{AppHandle, Runtime, Wry};

const KEY_OVERLAY_MODE: &str = "overlay_mode";

/// Stored mode name ("overlay" or "normal"), if the user ever chose one.
pub fn read_overlay_mode(app: &AppHandle<Wry>) -> Option<String> {
    read_string(app, KEY_OVERLAY_MODE)
}

pub fn set_overlay_mode<R: Runtime>(app: &AppHandle<R>, mode: &str) -> Result<()> {
    write_optional_string(app, KEY_OVERLAY_MODE, mode)
}
