//! Routing & LLM-engine settings: which model to use, where to run it,
//! how aggressively to dispatch to the local engine, and the smaller
//! "smart-routing" knobs (classifier model, RAG, tool calls).

use super::store::{get_bool, get_i64, read_string, write_bool, write_optional_string, STORE_FILE};
use super::Result;
use tauri::{AppHandle, Runtime, Wry};
use tauri_plugin_store::StoreExt;

const KEY_OPENROUTER_MODEL: &str = "openrouter_model";
const KEY_MODE: &str = "mode";
const KEY_LOCAL_MODEL_PATH: &str = "local_model_path";
/// Optional path to a smaller GGUF dedicated to skill-classification
/// when running fully offline. Falls back to [`KEY_LOCAL_MODEL_PATH`]
/// (the chat model) when unset — acceptable but slower for big chat
/// models like 7B+.
const KEY_LOCAL_CLASSIFIER_MODEL_PATH: &str = "local_classifier_model_path";
const KEY_GPU_LAYERS: &str = "llm_gpu_layers";
const KEY_SMART_ROUTING: &str = "smart_routing";
const KEY_CLASSIFIER_MODEL: &str = "classifier_model";
const KEY_RAG_ENABLED: &str = "rag_enabled";
const KEY_CHAT_TOOL_CALLS: &str = "chat_tool_calls_enabled";
/// Command line of an ACP agent (e.g. `claude-agent-acp`). When set,
/// Cloud-routed chat turns go to it instead of OpenRouter; background
/// calls (classifier, coach, vision) stay on OpenRouter.
const KEY_ACP_AGENT_COMMAND: &str = "acp_agent_command";

pub fn get_openrouter_model(app: &AppHandle<Wry>) -> String {
    read_string(app, KEY_OPENROUTER_MODEL)
        .unwrap_or_else(|| komorebi_cloud::DEFAULT_MODEL.to_string())
}

pub fn set_openrouter_model<R: Runtime>(app: &AppHandle<R>, model: &str) -> Result<()> {
    write_optional_string(app, KEY_OPENROUTER_MODEL, model)
}

pub fn get_mode(app: &AppHandle<Wry>) -> komorebi_router::Mode {
    match read_string(app, KEY_MODE).as_deref() {
        Some("local") => komorebi_router::Mode::Local,
        Some("cloud") => komorebi_router::Mode::Cloud,
        _ => komorebi_router::Mode::Auto,
    }
}

pub fn set_mode<R: Runtime>(app: &AppHandle<R>, mode: komorebi_router::Mode) -> Result<()> {
    let v = match mode {
        komorebi_router::Mode::Auto => "auto",
        komorebi_router::Mode::Local => "local",
        komorebi_router::Mode::Cloud => "cloud",
    };
    write_optional_string(app, KEY_MODE, v)
}

pub fn mode_str(app: &AppHandle<Wry>) -> &'static str {
    match get_mode(app) {
        komorebi_router::Mode::Auto => "auto",
        komorebi_router::Mode::Local => "local",
        komorebi_router::Mode::Cloud => "cloud",
    }
}

#[allow(dead_code)] // used by chat::stream_local when the `local-llm` feature is on
pub fn get_local_model_path(app: &AppHandle<Wry>) -> Option<String> {
    read_string(app, KEY_LOCAL_MODEL_PATH)
}

pub fn set_local_model_path<R: Runtime>(app: &AppHandle<R>, path: &str) -> Result<()> {
    write_optional_string(app, KEY_LOCAL_MODEL_PATH, path)
}

/// Read the optional dedicated classifier model path. Returns `None`
/// when the user hasn't configured one — callers fall back to
/// [`get_local_model_path`].
#[allow(dead_code)] // used by chat::pick_skill_intent when local-llm is on
pub fn get_local_classifier_model_path(app: &AppHandle<Wry>) -> Option<String> {
    read_string(app, KEY_LOCAL_CLASSIFIER_MODEL_PATH)
}

pub fn set_local_classifier_model_path<R: Runtime>(app: &AppHandle<R>, path: &str) -> Result<()> {
    write_optional_string(app, KEY_LOCAL_CLASSIFIER_MODEL_PATH, path)
}

pub fn read_local_classifier_model_path(app: &AppHandle<Wry>) -> Option<String> {
    read_string(app, KEY_LOCAL_CLASSIFIER_MODEL_PATH)
}

/// `None` means "auto" (use GPU if available, otherwise CPU). `Some(0)`
/// forces CPU; `Some(n > 0)` offloads n layers to GPU; `Some(-1)` offloads
/// everything. Only meaningful when the `local-llm` feature is compiled
/// with a GPU backend (CUDA / Vulkan).
pub fn get_gpu_layers(app: &AppHandle<Wry>) -> Option<i64> {
    get_i64(app, KEY_GPU_LAYERS)
}

pub fn set_gpu_layers<R: Runtime>(app: &AppHandle<R>, layers: Option<i64>) -> Result<()> {
    let store = app.store(STORE_FILE)?;
    match layers {
        Some(n) => store.set(KEY_GPU_LAYERS, serde_json::Value::from(n)),
        None => {
            store.delete(KEY_GPU_LAYERS);
        }
    }
    store.save()?;
    Ok(())
}

pub fn get_smart_routing(app: &AppHandle<Wry>) -> bool {
    get_bool(app, KEY_SMART_ROUTING, false)
}

pub fn set_smart_routing<R: Runtime>(app: &AppHandle<R>, on: bool) -> Result<()> {
    write_bool(app, KEY_SMART_ROUTING, on)
}

pub fn get_classifier_model(app: &AppHandle<Wry>) -> String {
    read_string(app, KEY_CLASSIFIER_MODEL)
        .unwrap_or_else(|| komorebi_cloud::DEFAULT_CLASSIFIER_MODEL.to_string())
}

pub fn set_classifier_model<R: Runtime>(app: &AppHandle<R>, model: &str) -> Result<()> {
    write_optional_string(app, KEY_CLASSIFIER_MODEL, model)
}

pub fn get_rag_enabled(app: &AppHandle<Wry>) -> bool {
    get_bool(app, KEY_RAG_ENABLED, false)
}

pub fn set_rag_enabled<R: Runtime>(app: &AppHandle<R>, on: bool) -> Result<()> {
    write_bool(app, KEY_RAG_ENABLED, on)
}

pub fn get_chat_tool_calls_enabled(app: &AppHandle<Wry>) -> bool {
    get_bool(app, KEY_CHAT_TOOL_CALLS, true)
}

pub fn set_chat_tool_calls_enabled<R: Runtime>(app: &AppHandle<R>, on: bool) -> Result<()> {
    write_bool(app, KEY_CHAT_TOOL_CALLS, on)
}

pub fn get_acp_agent_command(app: &AppHandle<Wry>) -> Option<String> {
    read_string(app, KEY_ACP_AGENT_COMMAND)
}

pub fn set_acp_agent_command<R: Runtime>(app: &AppHandle<R>, command: &str) -> Result<()> {
    write_optional_string(app, KEY_ACP_AGENT_COMMAND, command)
}
