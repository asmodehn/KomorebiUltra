//! Plain-window surface: the fallback when a layer shell is unavailable.
//!
//! On Linux that is a monitor-sized always-on-top window and the GTK input
//! shape; elsewhere Tauri's own frontmost/maximize behaviour is the whole
//! path. No new backend behaviour is introduced here.
//! Building without the `layer-shell` feature selects this surface.

use tauri::WebviewWindow;

use super::{InputRect, Surface};

/// The fallback surface selected whenever a Wayland layer shell is unavailable
/// or the build has no `layer-shell` feature.
pub(crate) struct PlainSurface;

impl Surface for PlainSurface {
    fn name(&self) -> &'static str {
        "plain-window"
    }

    fn init(&self, _window: &WebviewWindow) {}

    fn apply(&self, window: &WebviewWindow) {
        #[cfg(target_os = "linux")]
        {
            super::warn_layer_shell_once();
            super::fill_monitor(window);
        }
        // X11/Windows/macOS: the compositor positions a maximised window for
        // us (work area), and alpha survives maximising.
        #[cfg(not(target_os = "linux"))]
        if let Err(e) = window.set_maximized(true) {
            tracing::warn!(?e, "overlay: failed to maximize");
        }
    }

    fn set_input_region(&self, window: &WebviewWindow, rects: &[InputRect]) -> tauri::Result<bool> {
        #[cfg(target_os = "linux")]
        {
            super::apply_input_region(window, rects)?;
            Ok(true)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (window, rects);
            Ok(false)
        }
    }

    fn allow_keyboard(&self, _window: &WebviewWindow) {}
}
