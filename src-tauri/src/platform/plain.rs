//! Plain-window surface: today's path when there is no layer shell.
//!
//! On Linux that is an always-on-top window, an opt-in X11 hint stub, and the
//! GTK input shape; elsewhere Tauri's own frontmost/maximize behaviour is the
//! whole path. No new backend behaviour is introduced here.

use tauri::WebviewWindow;

use super::{InputRect, Surface, SurfaceInfo};
use crate::window::OverlayMode;

/// The fallback surface selected whenever a Wayland layer shell is unavailable.
pub(crate) struct PlainSurface;

impl Surface for PlainSurface {
    fn name(&self) -> &'static str {
        "plain-window"
    }

    fn info(&self) -> SurfaceInfo {
        #[cfg(target_os = "linux")]
        let overlay_supported = true;
        #[cfg(not(target_os = "linux"))]
        let overlay_supported = false;

        SurfaceInfo {
            overlay_supported,
            layer_shell_supported: false,
            layer_shell_active: false,
        }
    }

    fn init(&self, _window: &WebviewWindow, _mode: OverlayMode) {}

    fn apply(&self, window: &WebviewWindow, mode: OverlayMode) -> OverlayMode {
        match mode {
            OverlayMode::Normal => OverlayMode::Normal,
            OverlayMode::Overlay => {
                #[cfg(target_os = "linux")]
                {
                    if super::is_x11() && std::env::var_os("KOMOREBI_X11_WINDOW_HINTS").is_some() {
                        if let Some(m) = apply_x11_hints(window) {
                            return m;
                        }
                    }
                    super::warn_layer_shell_once();
                }
                // Native non-tiling is best-effort; the mode is not downgraded.
                OverlayMode::Overlay
            }
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

/// X11 hints: `_NET_WM_STATE_ABOVE`, skip taskbar/pager, `DOCK` window type.
/// Stubbed until someone on X11 validates it; kept minimal and opt-in.
#[cfg(target_os = "linux")]
fn apply_x11_hints(window: &WebviewWindow) -> Option<OverlayMode> {
    let gw = window.gtk_window().ok()?;
    let _ = window.run_on_main_thread(move || {
        let _ = gw;
    });
    Some(OverlayMode::Overlay)
}
