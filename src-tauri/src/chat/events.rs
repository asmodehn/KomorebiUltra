//! Outbound `chat:*` event protocol.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Wry};

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChatEventOut {
    Started { id: String, route: String },
    Token { id: String, text: String },
    Done { id: String, full_text: String },
    Error { id: String, message: String },
}

pub(super) fn emit(app: &AppHandle<Wry>, evt: ChatEventOut) {
    super::turn_log::observe(app, &evt);
    if let Err(e) = app.emit("chat", &evt) {
        tracing::warn!(?e, "failed to emit chat event");
    }
}
