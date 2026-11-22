//! Window overlay / frontmost management.
//!
//! Tauri (config) handles: `alwaysOnTop`, `transparent`, `decorations: false`,
//! `skipTaskbar`, `focus: false`.  What Tauri *cannot* do is force a window to
//! float on tiling compositors (Wayland layer-shell / X11 `_NET_WM_STATE`
//! hints) or make a window ignore pointer input on Windows
//! (`WS_EX_TRANSPARENT`) / macOS (`ignoresMouseEvents`).  Those need native
//! code, isolated in `platform/` so the rest of the app stays
//! platform-agnostic.
//!
//! There is one user-facing window: a frontmost, screen-covering,
//! click-through overlay. The selected surface decides how that is realized —
//! a Wayland layer surface where available, otherwise a plain always-on-top
//! window. `KOMOREBI_START_MODE=normal` forces the plain surface for one dev
//! run; it is a fallback instrument, not a second product mode, and is never
//! persisted or switchable at runtime.
//!
//! Covering the monitor is *not* compositor fullscreen: fullscreen loses the
//! surface alpha on Wayland (the window renders black despite a transparent
//! page), so a layer surface anchored to every edge is used instead.
//!
//! The surface is prepared before the window is first mapped: gtk-layer-shell
//! re-creates the GdkWindow and refuses a mapped widget. Mapping first and
//! replacing the window afterwards would also let the compositor focus the
//! wrong window and steal keys from whatever the user was typing in.

use tauri::WebviewWindow;

/// Whether the selected surface can present the overlay.
pub fn overlay_supported() -> bool {
    crate::platform::surface().info().overlay_supported
}

/// Prepare the window for its surface while it is still unmapped.
pub fn prepare_surface(window: &WebviewWindow) {
    crate::platform::surface().init(window);
}

/// Apply the overlay presentation: native coverage plus Tauri's frontmost.
///
/// Called once at startup before the window is shown. There is no in-process
/// window replacement: a layer surface cannot become a toplevel, and changing
/// the surface means launching with a different capability or dev pin.
pub fn apply_overlay(window: &WebviewWindow) {
    // Keep the order from the old overlay path: Tauri's frontmost flag first,
    // then the platform surface (which may size or attach the window).
    if let Err(e) = window.set_always_on_top(true) {
        tracing::warn!(?e, "failed to set always-on-top");
    }
    crate::platform::surface().apply(window);
}

/// Let the overlay take keyboard focus on demand (Wayland layer surface).
///
/// Called after the window is mapped: while a layer surface is being mapped it
/// must not ask for the keyboard, or the compositor hands it the focus the
/// moment it appears.
pub fn allow_keyboard(window: &WebviewWindow) {
    crate::platform::surface().allow_keyboard(window);
}

mod frame;

/// PNG of the window area (see `frame.rs`).
pub fn capture_frame(window: &WebviewWindow) -> Result<Vec<u8>, String> {
    frame::capture(window)
}
