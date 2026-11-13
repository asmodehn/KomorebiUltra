//! Reply engines: OpenRouter, the local llama.cpp model, and ACP agents.
//!
//! `stream_local` and `stream_cloud` are **model providers**: Komorebi owns
//! the turn around them (routing, tool loop, RAG). `stream_acp` is a **turn
//! engine**: the agent runs its own loop, so the runner must not wrap it in
//! [`super::tool_loop`].

use super::events::{emit, ChatEventOut};
use super::ChatService;
use crate::settings;
use futures::StreamExt;
use komorebi_cloud::{AcpClient, OpenRouterClient, StreamEvent, TokenStream};
use komorebi_router::ChatMessage;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, Wry};

/// Build the bundled llama.cpp engine from the user's settings. Returns
/// the [`StubEngine`](komorebi_llm::StubEngine) when the model path is
/// not configured or the `local-llm` Cargo feature is off — in either
/// case calls into the engine return [`LlmError::NotAvailable`] which
/// the caller turns into a graceful degradation.
fn build_local_engine(app: &AppHandle<Wry>) -> Arc<dyn komorebi_llm::LlmEngine> {
    build_local_engine_at(app, settings::get_local_model_path(app))
}

/// Public re-export of [`build_local_engine`] for sibling modules
/// (currently `coach::run_text`). Sharing one builder keeps GPU-layer
/// and model-path resolution in one place.
pub(crate) fn build_local_engine_public(app: &AppHandle<Wry>) -> Arc<dyn komorebi_llm::LlmEngine> {
    build_local_engine(app)
}

/// Build a llama.cpp engine pinned to a specific GGUF path. Used by the
/// classifier path so the user can pin a smaller, faster model
/// (e.g. Llama-3.2-3B) while keeping a heavier chat model loaded.
pub(super) fn build_local_engine_at(
    app: &AppHandle<Wry>,
    model_path: Option<String>,
) -> Arc<dyn komorebi_llm::LlmEngine> {
    use komorebi_llm::{default_engine, LlmConfig};
    let mut cfg = LlmConfig::default();
    if let Some(p) = model_path {
        cfg.model_path = Some(std::path::PathBuf::from(p));
    }
    if let Some(n) = settings::get_gpu_layers(app) {
        cfg.n_gpu_layers = Some(n as i32);
    }
    default_engine(cfg)
}

pub(super) async fn stream_cloud(
    app: &AppHandle<Wry>,
    service: &ChatService,
    id: &str,
    messages: &[ChatMessage],
) -> Result<String, String> {
    let api_key = settings::get_openrouter_key(app)
        .ok_or_else(|| "OpenRouter API key is not set. Open settings and add one.".to_string())?;
    let model = settings::get_openrouter_model(app);
    let client = OpenRouterClient::new(api_key).map_err(|e| e.to_string())?;
    let stream = client
        .stream_chat(&model, messages)
        .await
        .map_err(|e| e.to_string())?;
    drain(app, service, id, stream).await
}

/// One agent turn. The agent owns its loop and its tools, so this is never
/// called from [`super::tool_loop`]: there is no Komorebi `<tool_call>` in
/// the stream to parse or execute.
pub(super) async fn stream_acp(
    app: &AppHandle<Wry>,
    service: &ChatService,
    id: &str,
    messages: &[ChatMessage],
    command: &str,
) -> Result<String, String> {
    let stream = AcpClient::from_command(command)
        .map_err(|e| e.to_string())?
        .stream_chat(messages)
        .await
        .map_err(|e| e.to_string())?;
    drain(app, service, id, stream).await
}

/// Forward a provider/agent stream to the bubble, accumulating the reply.
/// `stream_cloud` and `stream_acp` end here; `stream_local` has its own
/// event type and its own cancel handling.
async fn drain(
    app: &AppHandle<Wry>,
    service: &ChatService,
    id: &str,
    mut stream: TokenStream,
) -> Result<String, String> {
    let mut acc = String::new();
    while let Some(evt) = stream.next().await {
        if service.cancel.load(Ordering::SeqCst) {
            break;
        }
        match evt {
            Ok(StreamEvent::Token(t)) => {
                acc.push_str(&t);
                emit(
                    app,
                    ChatEventOut::Token {
                        id: id.into(),
                        text: t,
                    },
                );
            }
            Ok(StreamEvent::Done) => break,
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(acc)
}

pub(super) async fn stream_local(
    app: &AppHandle<Wry>,
    _service: &ChatService,
    id: &str,
    messages: &[ChatMessage],
) -> Result<String, String> {
    use komorebi_llm::{LlmError, LlmEvent};

    let engine = build_local_engine(app);
    match engine.stream_chat(messages).await {
        Ok(mut stream) => {
            let mut acc = String::new();
            while let Some(evt) = stream.next().await {
                match evt {
                    Ok(LlmEvent::Token(t)) => {
                        acc.push_str(&t);
                        emit(
                            app,
                            ChatEventOut::Token {
                                id: id.into(),
                                text: t,
                            },
                        );
                    }
                    Ok(LlmEvent::Done) => break,
                    Err(e) => return Err(e.to_string()),
                }
            }
            Ok(acc)
        }
        Err(LlmError::NotAvailable) => {
            // Graceful fallback: tell the user how to proceed.
            let msg = "Local model isn't wired up yet in this build. \
                       Switch to Cloud mode in settings (Auto will still fall back to Cloud for heavy queries).";
            emit(
                app,
                ChatEventOut::Token {
                    id: id.into(),
                    text: msg.into(),
                },
            );
            Ok(msg.to_string())
        }
        Err(e) => Err(e.to_string()),
    }
}
