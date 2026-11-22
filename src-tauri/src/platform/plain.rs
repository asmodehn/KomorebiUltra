//! Plain-window surface: the fallback when a layer shell is unavailable.
//!
//! On Linux that is a monitor-sized always-on-top window, an opt-in X11 hint
//! stub, and the GTK input shape; elsewhere Tauri's own frontmost/maximize
//! behaviour is the whole path. No new backend behaviour is introduced here.
//! `KOMOREBI_START_MODE=normal` selects this surface for one dev run.

use tauri::WebviewWindow;

use super::{InputRect, Surface, SurfaceInfo};

/// The fallback surface selected whenever a Wayland layer shell is unavailable
/// or the dev pin asks for it.
pub(crate) struct PlainSurface;

impl Surface for PlainSurface {
    fn name(&self) -> &'static str {
        "plain-window"
    }

    fn info(&self) -> SurfaceInfo {
        SurfaceInfo {
            #[cfg(target_os = "linux")]
            overlay_supported: true,
            #[cfg(not(target_os = "linux"))]
            overlay_supported: false,
        }
    }

    fn init(&self, _window: &WebviewWindow) {}

    fn apply(&self, window: &WebviewWindow) {
        #[cfg(target_os = "linux")]
        {
            if super::is_x11() && std::env::var_os("KOMOREBI_X11_WINDOW_HINTS").is_some() {
                // X11 EWMH hints are still a stub; kept behind the opt-in until
                // they are implemented and validated on X11.
                let _ = window.gtk_window();
            }
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
