//! Wayland layer-shell surface: the way Komorebi realizes `overlay` today.
//!
//! The surface attaches while the window is unmapped, covers the output
//! without compositor fullscreen (which would lose the alpha), and maps with
//! `KeyboardMode::None`. Keyboard is switched to `OnDemand` only after the
//! page's first pointer event, so the overlay never steals focus at startup.

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::WebviewWindow;

use super::{fill_monitor, warn_layer_shell_once, InputRect, Surface, SurfaceInfo};

static ACTIVE: AtomicBool = AtomicBool::new(false);

/// The selected surface when the build has layer-shell and the session is
/// Wayland.
pub(crate) struct LayerShellSurface;

impl Surface for LayerShellSurface {
    fn name(&self) -> &'static str {
        "wayland-layer-shell"
    }

    fn info(&self) -> SurfaceInfo {
        SurfaceInfo {
            overlay_supported: true,
        }
    }

    fn init(&self, window: &WebviewWindow) {
        if attach(window) {
            ACTIVE.store(true, Ordering::Relaxed);
        } else {
            ACTIVE.store(false, Ordering::Relaxed);
            tracing::warn!(
                "overlay: gtk-layer-shell could not attach (window already \
                 mapped?); falling back to a sized always-on-top window"
            );
        }
    }

    fn apply(&self, window: &WebviewWindow) {
        if ACTIVE.load(Ordering::Relaxed) {
            // Anchored to every edge: it already covers the monitor.
            tracing::info!("overlay: layer surface covers the monitor (no action needed)");
            return;
        }
        warn_layer_shell_once();
        fill_monitor(window);
    }

    fn set_input_region(&self, window: &WebviewWindow, rects: &[InputRect]) -> tauri::Result<bool> {
        super::apply_input_region(window, rects)?;
        Ok(true)
    }

    fn allow_keyboard(&self, window: &WebviewWindow) {
        let Ok(gw) = window.gtk_window() else {
            return;
        };
        if gtk_layer_shell::is_layer_window(&gw) {
            gtk_layer_shell::set_keyboard_mode(&gw, gtk_layer_shell::KeyboardMode::OnDemand);
            tracing::info!("overlay: keyboard mode switched to OnDemand after pointer input");
        }
    }
}

/// Promote the GTK window to a floating overlay layer. Requires the
/// `layer-shell` feature and `libgtk-layer-shell-dev`, and must run before the
/// widget is mapped (gtk-layer-shell re-creates the GdkWindow).
fn attach(window: &WebviewWindow) -> bool {
    let Ok(gw) = window.gtk_window() else {
        tracing::warn!("overlay: no GTK window for layer-shell");
        return false;
    };
    gtk_layer_shell::init_for_window(&gw);
    if !gtk_layer_shell::is_layer_window(&gw) {
        return false;
    }
    gtk_layer_shell::set_namespace(&gw, "komorebi-overlay");
    gtk_layer_shell::set_layer(&gw, gtk_layer_shell::Layer::Overlay);
    // None while mapped: a layer surface that asks for the keyboard is handed
    // it by the compositor as soon as it appears. The page asks for OnDemand
    // on its first pointer event instead (see `allow_keyboard`).
    gtk_layer_shell::set_keyboard_mode(&gw, gtk_layer_shell::KeyboardMode::None);
    tracing::info!("overlay: keyboard mode set to None before map");
    for edge in [
        gtk_layer_shell::Edge::Left,
        gtk_layer_shell::Edge::Right,
        gtk_layer_shell::Edge::Top,
        gtk_layer_shell::Edge::Bottom,
    ] {
        gtk_layer_shell::set_anchor(&gw, edge, true);
    }
    // Do not reserve space or push other windows around.
    gtk_layer_shell::set_exclusive_zone(&gw, -1);
    tracing::info!("gtk-layer-shell attached: window is an overlay layer surface");
    true
}
