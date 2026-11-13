//! Linux native window overlay.
//!
//! Tauri config already provides: `alwaysOnTop`, `transparent`,
//! `decorations: false`, `skipTaskbar`, `focus: false`.  That covers
//! "frontmost" on most compositors.  What Tauri *cannot* do is force a
//! window to float on tiling compositors — that needs Wayland layer-shell or
//! X11 `_NET_WM_STATE` hints, which live here.
//!
//! Two separate problems are solved here:
//!
//! 1. **Covering the screen.** On Wayland a client cannot position itself, so
//!    a merely resized window lands wherever the compositor wants (verified:
//!    a 1280-wide window at x=400 on a 1280 screen). Compositor *fullscreen*
//!    does cover the screen, but it also loses the surface alpha: a
//!    transparent GTK window painted black when fullscreened, in this app and
//!    in a minimal GTK test alike. Layer-shell (overlay layer, anchored to all
//!    edges, exclusive zone -1) covers the screen **and** stays transparent,
//!    and tiling compositors never tile it.
//!
//! 2. **Not being tiled.** Layer-shell does that natively. Without it,
//!    `overlay` falls back to always-on-top plus a best-effort monitor fill,
//!    and the compositor may still tile the window (a compositor-side float
//!    rule is the no-code workaround).
//!
//! The layer-shell path is opt-in: enable the `layer-shell` feature and launch
//! with `KOMOREBI_LAYER_SHELL=1`. It must attach while the window is not yet
//! mapped, which is why `tauri.conf.json` creates the window with
//! `visible: false` and `startup` shows it after `init_layer_shell`. Because a
//! GTK window cannot be detached from the layer shell, a run started this way
//! stays a layer surface until it exits: once a layer surface is attached it
//! covers the screen for the process lifetime, so a later `normal` can only
//! change input and stacking, not the window type.

use super::OverlayMode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use tauri::WebviewWindow;

/// Whether the window has been promoted to a layer surface this run.
static LAYER_SHELL: AtomicBool = AtomicBool::new(false);

/// True when the window is a layer surface (covers the screen already, so it
/// must never be fullscreened).
pub(super) fn layer_shell_active() -> bool {
    LAYER_SHELL.load(Ordering::Relaxed)
}

/// Whether this build/run *can* promote the window to a layer surface.
///
/// `overlay` is realized as a layer surface whenever that is possible; the
/// in-process fallback (always-on-top + input region) only exists for
/// compositors/builds where it is not.
pub(super) fn layer_shell_supported() -> bool {
    cfg!(feature = "layer-shell") && is_wayland()
}

/// A layer surface cannot be converted back into a normal toplevel (and a
/// mapped GTK window cannot be promoted), so changing the window *type* means
/// creating a new window. That recreation happens in-process (the app, its
/// backend and its state stay warm); only the window + webview are replaced.
/// Frontmost/geometry changes stay in-process without recreation.
pub(super) fn needs_recreate(mode: OverlayMode) -> bool {
    let wants_layer_shell = matches!(mode, OverlayMode::Overlay) && layer_shell_supported();
    wants_layer_shell != layer_shell_active()
}

pub(super) fn overlay_supported() -> bool {
    true
}

/// `KOMOREBI_LAYER_SHELL` accepts the usual truthy spellings; `0`/`false`
/// explicitly opt out (a windowed start passes `0`).
fn layer_shell_requested() -> bool {
    match std::env::var("KOMOREBI_LAYER_SHELL") {
        Ok(v) => !matches!(v.as_str(), "" | "0" | "false" | "no" | "off"),
        Err(_) => false,
    }
}

/// Startup: attach the layer surface when the environment asks for it.
/// Called from `setup` before the window is first mapped.
pub(super) fn init_layer_shell(window: &WebviewWindow) {
    set_layer_shell(window, layer_shell_requested());
}

/// Attach (or explicitly do not attach) the layer surface to `window`.
///
/// Must run while the widget is not mapped. Used both at startup (env-driven)
/// and when a mode switch recreates the window (mode-driven), so the state
/// flag always describes the *current* window.
pub(super) fn set_layer_shell(window: &WebviewWindow, attach_layer: bool) {
    if !attach_layer {
        LAYER_SHELL.store(false, Ordering::Relaxed);
        return;
    }
    #[cfg(feature = "layer-shell")]
    {
        if attach(window) {
            LAYER_SHELL.store(true, Ordering::Relaxed);
        } else {
            LAYER_SHELL.store(false, Ordering::Relaxed);
            tracing::warn!(
                "overlay: gtk-layer-shell could not attach (window already \
                 mapped?); falling back to always-on-top"
            );
        }
    }
    #[cfg(not(feature = "layer-shell"))]
    {
        let _ = window;
        tracing::warn!(
            "KOMOREBI_LAYER_SHELL is set but this build has no `layer-shell` \
             feature; rebuild with `--features layer-shell`"
        );
    }
}

pub(super) fn apply_overlay(window: &WebviewWindow, mode: OverlayMode) -> OverlayMode {
    match mode {
        OverlayMode::Normal => OverlayMode::Normal,
        OverlayMode::Overlay => {
            // Layer-shell, when attached at startup, already covers the screen
            // without fullscreen (which would make the window opaque).
            if layer_shell_active() {
                return OverlayMode::Overlay;
            }
            if is_x11() && std::env::var_os("KOMOREBI_X11_WINDOW_HINTS").is_some() {
                if let Some(m) = apply_x11_hints(window) {
                    return m;
                }
            }
            // Native non-tiling is best-effort: `overlay` still means
            // frontmost + covering the monitor when the compositor cannot give
            // us a layer surface, so it must not downgrade the mode.
            warn_layer_shell_once();
            OverlayMode::Overlay
        }
    }
}

fn is_wayland() -> bool {
    static RESULT: OnceLock<bool> = OnceLock::new();
    *RESULT.get_or_init(|| {
        std::env::var("WAYLAND_DISPLAY").is_ok()
            || std::env::var_os("GDK_BACKEND")
                .map(|b| b.to_string_lossy().contains("wayland"))
                .unwrap_or(false)
    })
}

fn is_x11() -> bool {
    !is_wayland()
}

/// Wayland layer-shell: promote the GTK window to a floating overlay layer
/// so tiling compositors (COSMIC, wlroots, etc.) don't tile it, and so it
/// covers the output without fullscreen.
///
/// Must run while the widget is not mapped (gtk-layer-shell re-creates the
/// GdkWindow). Requires the `layer-shell` feature + `libgtk-layer-shell-dev`.
#[cfg(feature = "layer-shell")]
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
    // OnDemand, not None: the chat input and the top bar must be able to take
    // keyboard focus when the user clicks them.
    gtk_layer_shell::set_keyboard_mode(&gw, gtk_layer_shell::KeyboardMode::OnDemand);
    for edge in [
        gtk_layer_shell::Edge::Left,
        gtk_layer_shell::Edge::Right,
        gtk_layer_shell::Edge::Top,
        gtk_layer_shell::Edge::Bottom,
    ] {
        gtk_layer_shell::set_anchor(&gw, edge, true);
    }
    // -1: do not reserve space, do not push other windows around.
    gtk_layer_shell::set_exclusive_zone(&gw, -1);
    tracing::info!("gtk-layer-shell attached: window is an overlay layer surface");
    true
}

/// One-shot hint when the compositor keeps tiling the overlay window.
fn warn_layer_shell_once() {
    static WARNED: OnceLock<()> = OnceLock::new();
    WARNED.get_or_init(|| {
        tracing::warn!(
            "overlay: the compositor may still tile this window (no layer-shell). \
             Always-on-top is active regardless."
        );
    });
}

/// X11 hints: `_NET_WM_STATE_ABOVE`, skip taskbar/pager, `DOCK` window type.
/// Stubbed until someone on X11 validates it; kept minimal.
fn apply_x11_hints(window: &WebviewWindow) -> Option<OverlayMode> {
    let gw = window.gtk_window().ok()?;
    let _ = window.run_on_main_thread(move || {
        let _ = gw;
    });
    Some(OverlayMode::Overlay)
}
