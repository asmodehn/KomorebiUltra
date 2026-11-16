//! Window-area frame capture: render the webview's own pixels into a PNG.
//!
//! This is the "framebuffer render of the window area": no screen capture,
//! no portal permission, no other windows in the picture. It doubles as the
//! primitive for a token-friendly agent frame (downscaled PNG + JSON state,
//! see `crate::agent_frame`).
//!
//! Linux: WebKitGTK's `webkit_web_view_get_snapshot`, which renders the page
//! (including composited WebGL layers such as the Live2D canvas) into a cairo
//! surface. The snapshot is requested on the GTK main thread and the result
//! comes back through a channel, because the callback runs on the main loop.
//!
//! Windows (WebView2 `CapturePreview`) and macOS (`WKWebView.takeSnapshot`)
//! belong here as well; they are stubs until someone can validate them.

use tauri::WebviewWindow;

/// PNG bytes of the visible window area, transparent background preserved.
pub fn capture(window: &WebviewWindow) -> Result<Vec<u8>, String> {
    #[cfg(target_os = "linux")]
    {
        linux::capture(window)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = window;
        Err("window frame capture is not implemented on this platform yet".into())
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::sync::mpsc;
    use std::time::Duration;
    use tauri::WebviewWindow;
    use webkit2gtk::{SnapshotOptions, SnapshotRegion, WebViewExt};

    pub(super) fn capture(window: &WebviewWindow) -> Result<Vec<u8>, String> {
        let (tx, rx) = mpsc::channel::<Result<Vec<u8>, String>>();
        window
            .with_webview(move |platform| {
                let view = platform.inner();
                view.snapshot(
                    SnapshotRegion::Visible,
                    SnapshotOptions::TRANSPARENT_BACKGROUND,
                    None::<&webkit2gtk::gio::Cancellable>,
                    move |res| {
                        let out = res
                            .map_err(|e| e.to_string())
                            .and_then(|surface| write_png(&surface));
                        let _ = tx.send(out);
                    },
                );
            })
            .map_err(|e| e.to_string())?;
        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| "window snapshot timed out".to_string())?
    }

    fn write_png(surface: &gtk::cairo::Surface) -> Result<Vec<u8>, String> {
        let mut buf = Vec::new();
        surface
            .write_to_png(&mut buf)
            .map_err(|e| format!("cairo write_to_png: {e}"))?;
        Ok(buf)
    }
}
