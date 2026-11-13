//! Overlay input region: which parts of the window take pointer input.
//!
//! Experimental, off unless `KOMOREBI_INPUT_REGION` is set. The frontend
//! reports the rects of its interactive elements (CSS px from the
//! webview's top-left); everywhere else the window is click-through.
//! Linux only for now (X11 and Wayland).

use serde::Deserialize;
use tauri::WebviewWindow;

const ENV_VAR: &str = "KOMOREBI_INPUT_REGION";

#[derive(Debug, Deserialize)]
pub struct InputRect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

/// Restricts pointer input to `rects`. Returns `false` when the overlay
/// region is unsupported or disabled, so the caller can stop reporting.
/// An empty list restores input on the whole window.
#[tauri::command]
pub fn set_input_region(window: WebviewWindow, rects: Vec<InputRect>) -> Result<bool, String> {
    if std::env::var_os(ENV_VAR).is_none() {
        return Ok(false);
    }
    #[cfg(target_os = "linux")]
    {
        linux::apply(&window, rects).map_err(|e| e.to_string())?;
        Ok(true)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (window, rects);
        Ok(false)
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::InputRect;
    use gtk::cairo::{RectangleInt, Region};
    use gtk::prelude::*;
    use std::sync::Once;
    use tauri::WebviewWindow;

    pub(super) fn apply(window: &WebviewWindow, rects: Vec<InputRect>) -> tauri::Result<()> {
        let w = window.clone();
        window.run_on_main_thread(move || {
            let (Ok(gw), Ok(vbox)) = (w.gtk_window(), w.default_vbox()) else {
                return;
            };
            if rects.is_empty() {
                gw.input_shape_combine_region(None);
                return;
            }
            // Webview coordinates start at the vbox, which may sit below
            // a header; the input shape is in window coordinates.
            let (dx, dy) = vbox.translate_coordinates(&gw, 0, 0).unwrap_or((0, 0));
            static LOGGED: Once = Once::new();
            LOGGED.call_once(|| tracing::info!(dx, dy, "input region: webview offset"));
            let rs: Vec<RectangleInt> = rects
                .iter()
                .map(|r| RectangleInt::new(r.x + dx, r.y + dy, r.w, r.h))
                .collect();
            // Set on the widget, not the GdkWindow: with a client-side
            // decorated window GTK re-applies the widget's shape on every
            // allocation and would drop a GDK-level one.
            gw.input_shape_combine_region(Some(&Region::create_rectangles(&rs)));
        })
    }
}
