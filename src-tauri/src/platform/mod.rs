//! Narrow platform surface for the window Komorebi presents.
//!
//! One backend is chosen at runtime from the OS, a dev/fallback pin and a
//! capability probe: the selection shape Kando uses, without its Electron
//! runtime or broad `Backend` contract. The contract has only the operations
//! the app already calls: prepare the window before first map, apply the one
//! overlay presentation, set the click-through rects, set the keyboard focus
//! mode, and report capability. Desktop state, capture and simulated input are
//! separate later ports, not methods here.

use serde::Deserialize;
use std::sync::OnceLock;
use tauri::WebviewWindow;

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

/// Capability reported by the selected surface.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SurfaceInfo {
    pub(crate) overlay_supported: bool,
}

/// The narrow contract every platform surface satisfies.
pub(crate) trait Surface: Send + Sync {
    /// Stable name for logs and `window_overlay_info`.
    fn name(&self) -> &'static str;

    /// Capability probe.
    fn info(&self) -> SurfaceInfo;

    /// Prepare the window before it is first mapped (layer-shell attach).
    fn init(&self, window: &WebviewWindow);

    /// Apply frontmost and screen coverage.
    fn apply(&self, window: &WebviewWindow);

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

/// The plain surface is a dev/fallback instrument for this process only.
/// `KOMOREBI_START_MODE=normal` pins it; nothing is persisted and there is no
/// runtime switch back to the layer surface.
pub(crate) fn plain_requested() -> bool {
    static PIN: OnceLock<bool> = OnceLock::new();
    *PIN.get_or_init(
        || match std::env::var("KOMOREBI_START_MODE").ok().as_deref() {
            None | Some("overlay") | Some("always_on_top") => false,
            Some("normal") => true,
            Some(other) => {
                tracing::warn!(
                    mode = %other,
                    "unknown KOMOREBI_START_MODE, using the overlay surface"
                );
                false
            }
        },
    )
}

/// Dev pin first, then OS and build capability: the Kando selection order,
/// reduced to the two backends that exist today. Without the `layer-shell`
/// feature the Linux module is not compiled and plain is the only candidate.
fn select() -> &'static dyn Surface {
    if plain_requested() {
        return &plain::PlainSurface;
    }
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

/// Best-effort monitor coverage when no layer surface is available: the size
/// is honoured, the position is not (Wayland). On X11 this also moves the
/// window to the monitor origin.
#[cfg(target_os = "linux")]
pub(crate) fn fill_monitor(window: &WebviewWindow) {
    let Some(monitor) = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten())
    else {
        tracing::warn!("overlay: no monitor to fill");
        return;
    };
    let size = monitor.size();
    let pos = monitor.position();
    tracing::info!(
        ?size,
        "overlay: fill_monitor -> setting window size to monitor size"
    );
    if let Err(e) = window.set_size(tauri::PhysicalSize::new(size.width, size.height)) {
        tracing::warn!(?e, "overlay: failed to resize the window to the monitor");
    } else {
        tracing::info!("overlay: window resized to monitor size");
    }
    if let Err(e) = window.set_position(tauri::PhysicalPosition::new(pos.x, pos.y)) {
        tracing::debug!(
            ?e,
            "overlay: compositor ignored the window position (expected on Wayland)"
        );
    }
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
