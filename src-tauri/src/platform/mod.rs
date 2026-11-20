//! Narrow platform surface for the window Komorebi presents.
//!
//! One backend is chosen at runtime from the OS, the session and a capability
//! probe: the selection shape Kando uses, without its Electron runtime or broad
//! `Backend` contract. The contract has only the five operations the app
//! already calls: prepare the window before first map, apply frontmost and
//! coverage, set the click-through rects, set the keyboard focus mode, and
//! report capability. Desktop state, capture and simulated input are separate
//! later ports, not methods here.

use serde::Deserialize;
use std::sync::OnceLock;
use tauri::WebviewWindow;

use crate::window::OverlayMode;

#[cfg(all(target_os = "linux", feature = "layer-shell"))]
mod linux;
mod plain;

/// A click-through rectangle in webview coordinates. `commands::overlay`
/// re-exports this as `InputRect`.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct InputRect {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) w: i32,
    pub(crate) h: i32,
}

/// Capability and active window type reported by the selected surface.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SurfaceInfo {
    pub(crate) overlay_supported: bool,
    pub(crate) layer_shell_supported: bool,
    pub(crate) layer_shell_active: bool,
}

/// The narrow contract every platform surface satisfies.
pub(crate) trait Surface: Send + Sync {
    /// Stable name for logs and `window_overlay_info`.
    fn name(&self) -> &'static str;

    /// Capability probe (the fifth operation).
    fn info(&self) -> SurfaceInfo;

    /// Prepare the window before it is first mapped (layer-shell attach).
    fn init(&self, window: &WebviewWindow, mode: OverlayMode);

    /// Apply frontmost/coverage; returns the mode actually in effect.
    fn apply(&self, window: &WebviewWindow, mode: OverlayMode) -> OverlayMode;

    /// Restrict pointer input to `rects`; returns whether the platform applied it.
    fn set_input_region(&self, window: &WebviewWindow, rects: &[InputRect]) -> tauri::Result<bool>;

    /// Move the layer surface from `None` to `OnDemand` keyboard mode.
    fn allow_keyboard(&self, window: &WebviewWindow);
}

static SELECTED: OnceLock<&'static dyn Surface> = OnceLock::new();

/// The one surface selected for this process, chosen on first use.
pub(crate) fn surface() -> &'static dyn Surface {
    *SELECTED.get_or_init(|| {
        let chosen = select();
        tracing::info!(backend = chosen.name(), "platform surface selected");
        chosen
    })
}

/// OS first, then build capability, then session: the Kando selection order,
/// reduced to the two backends that exist today. When layer-shell is not in
/// this build, the module is not compiled and the plain surface is the only
/// candidate.
fn select() -> &'static dyn Surface {
    #[cfg(all(target_os = "linux", feature = "layer-shell"))]
    {
        if is_wayland() {
            &linux::LayerShellSurface
        } else {
            &plain::PlainSurface
        }
    }
    #[cfg(not(all(target_os = "linux", feature = "layer-shell")))]
    {
        &plain::PlainSurface
    }
}

/// Cached Wayland session check, shared by selection and the X11 path.
#[cfg(target_os = "linux")]
pub(crate) fn is_wayland() -> bool {
    static RESULT: OnceLock<bool> = OnceLock::new();
    *RESULT.get_or_init(|| {
        std::env::var("WAYLAND_DISPLAY").is_ok()
            || std::env::var_os("GDK_BACKEND")
                .map(|b| b.to_string_lossy().contains("wayland"))
                .unwrap_or(false)
    })
}

#[cfg(target_os = "linux")]
pub(crate) fn is_x11() -> bool {
    !is_wayland()
}

/// One-shot hint when the compositor keeps tiling the overlay window.
#[cfg(target_os = "linux")]
pub(crate) fn warn_layer_shell_once() {
    static WARNED: OnceLock<()> = OnceLock::new();
    WARNED.get_or_init(|| {
        tracing::warn!(
            "overlay: the compositor may still tile this window (no layer-shell). \
             Always-on-top is active regardless."
        );
    });
}

/// GTK input shape, shared by both Linux surfaces. The rectangle list is
/// copied into the main-thread closure; an empty list restores whole-window
/// input, as before.
#[cfg(target_os = "linux")]
pub(crate) fn apply_input_region(window: &WebviewWindow, rects: &[InputRect]) -> tauri::Result<()> {
    use gtk::cairo::{RectangleInt, Region};
    use gtk::prelude::*;
    use std::sync::Once;

    let w = window.clone();
    let owned = rects.to_vec();
    window.run_on_main_thread(move || {
        let (Ok(gw), Ok(vbox)) = (w.gtk_window(), w.default_vbox()) else {
            return;
        };
        if owned.is_empty() {
            gw.input_shape_combine_region(None);
            return;
        }
        // Webview coordinates start at the vbox; the shape is in window
        // coordinates.
        let (dx, dy) = vbox.translate_coordinates(&gw, 0, 0).unwrap_or((0, 0));
        static LOGGED: Once = Once::new();
        LOGGED.call_once(|| tracing::info!(dx, dy, "input region: webview offset"));
        let rs: Vec<RectangleInt> = owned
            .iter()
            .map(|r| RectangleInt::new(r.x + dx, r.y + dy, r.w, r.h))
            .collect();
        // Set on the widget, not the GdkWindow: GTK re-applies the widget's
        // shape on every allocation and would drop a GDK-level one.
        gw.input_shape_combine_region(Some(&Region::create_rectangles(&rs)));
    })
}
