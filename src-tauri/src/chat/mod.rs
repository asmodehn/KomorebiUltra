//! Chat orchestration: takes a user prompt, classifies it, streams tokens
//! from the chosen engine back to the frontend via Tauri events.
//!
//! Events emitted (all namespaced `chat:*`, serialized as `ChatEventOut`):
//! - `chat:started` — `{ route: "local" | "cloud" | "skill" | "acp", id: String }`
//! - `chat:token`   — `{ id, text }`
//! - `chat:done`    — `{ id, full_text }`
//! - `chat:error`   — `{ id, message }`
//!
//! Module map:
//! - [`events`]       — outbound event protocol + emitter helper.
//! - [`prompts`]      — system prompts (persona, mood-tag, tool protocol).
//! - [`rag`]          — RAG context assembly from the workspace index.
//! - [`tool_loop`]    — `<tool_call>` parsing & retry loop for the LLM.
//! - [`skill_picker`] — embedding → cloud → local → keyword skill resolution.
//! - [`engines`]      — local/cloud providers + the ACP agent engine.
//! - [`vision`]       — image+text generation pipeline.
//! - [`speak`]        — fire-and-forget TTS + text sanitization.
//! - [`turn_log`]     — opt-in raw turn log for reviewing replies.
//! - [`runner`]       — `run_generation` orchestrator.

#![allow(unused_imports)]

mod engines;
mod events;
mod prompts;
mod rag;
mod runner;
mod skill_picker;
pub(crate) mod speak;
mod tool_loop;
mod turn_log;
mod vision;

use komorebi_router::ChatMessage;
use komorebi_skills::SkillRegistry;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Wry};
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// Public re-exports — the surface the rest of the crate consumes.
// ---------------------------------------------------------------------------

pub use events::ChatEventOut;
pub use vision::downscale_for_vision;

pub(crate) use engines::build_local_engine_public;

// ---------------------------------------------------------------------------
// ChatService — shared orchestrator state held in Tauri's managed state.
// ---------------------------------------------------------------------------

/// Shared orchestrator state held in Tauri's managed state.
pub struct ChatService {
    /// History of the current conversation. For MVP this is in-memory only;
    /// SQLite persistence lands alongside RAG in Phase 3.
    pub(super) history: Mutex<Vec<ChatMessage>>,
    /// Cooperative cancellation flag for the in-flight generation.
    pub(super) cancel: AtomicBool,
    /// Built-in system skills (volume, clipboard, screenshot, open).
    pub(super) skills: SkillRegistry,
}

impl Default for ChatService {
    fn default() -> Self {
        Self {
            history: Mutex::new(Vec::new()),
            cancel: AtomicBool::new(false),
            skills: SkillRegistry::with_defaults(),
        }
    }
}

impl ChatService {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn clear(&self) {
        self.history.lock().await.clear();
        turn_log::reset();
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------------------
// Spawn entry points — the only public entry points for chat generation.
// ---------------------------------------------------------------------------

/// Entry point invoked by the Tauri command. Spawns the generation on the
/// async runtime and returns an id the frontend can correlate events with.
pub fn spawn_generation(app: AppHandle<Wry>, id: String, prompt: String) {
    turn_log::begin(&app, &id, &prompt, false);
    tauri::async_runtime::spawn(async move {
        if let Err(e) = runner::run_generation(app.clone(), id.clone(), prompt).await {
            events::emit(&app, ChatEventOut::Error { id, message: e });
        }
    });
}

/// Vision entry point: same event protocol as `spawn_generation`, but the
/// user message includes a PNG. Always routes through OpenRouter's vision
/// endpoint (currently the only vision-capable backend wired up). The
/// reply is stored in chat history alongside a synthetic user message
/// noting that an image was attached so subsequent text-only turns can
/// reference what was discussed.
pub fn spawn_vision_generation(
    app: AppHandle<Wry>,
    id: String,
    prompt: String,
    png_bytes: Vec<u8>,
) {
    turn_log::begin(&app, &id, &prompt, true);
    tauri::async_runtime::spawn(async move {
        if let Err(e) =
            vision::run_vision_generation(app.clone(), id.clone(), prompt, png_bytes).await
        {
            events::emit(&app, ChatEventOut::Error { id, message: e });
        }
    });
}
