//! LLM routing / OpenRouter / smart-routing / RAG settings commands.

use crate::settings;
use tauri::{AppHandle, Wry};

#[tauri::command]
pub fn set_openrouter_key(app: AppHandle<Wry>, key: String) -> Result<(), String> {
    settings::set_openrouter_key(&app, &key).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_mode(app: AppHandle<Wry>, mode: String) -> Result<(), String> {
    let m = match mode.as_str() {
        "auto" => komorebi_router::Mode::Auto,
        "local" => komorebi_router::Mode::Local,
        "cloud" => komorebi_router::Mode::Cloud,
        other => return Err(format!("unknown mode: {other}")),
    };
    settings::set_mode(&app, m).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_smart_routing(app: AppHandle<Wry>, enabled: bool) -> Result<(), String> {
    settings::set_smart_routing(&app, enabled).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_classifier_model(app: AppHandle<Wry>, model: String) -> Result<(), String> {
    settings::set_classifier_model(&app, &model).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_rag_enabled(app: AppHandle<Wry>, enabled: bool) -> Result<(), String> {
    settings::set_rag_enabled(&app, enabled).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_acp_agent_command(app: AppHandle<Wry>, command: String) -> Result<(), String> {
    settings::set_acp_agent_command(&app, &command).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_openrouter_model(app: AppHandle<Wry>, model: String) -> Result<(), String> {
    settings::set_openrouter_model(&app, &model).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_chat_tool_calls_enabled(app: AppHandle<Wry>, enabled: bool) -> Result<(), String> {
    settings::set_chat_tool_calls_enabled(&app, enabled).map_err(|e| e.to_string())
}

/// None = auto (CPU, or GPU if the GGML backend has a GPU runtime);
/// Some(0) = force CPU; Some(n>0) = offload n layers to the GPU.
#[tauri::command]
pub fn set_llm_gpu_layers(app: AppHandle<Wry>, layers: Option<i64>) -> Result<(), String> {
    settings::set_gpu_layers(&app, layers).map_err(|e| e.to_string())
}

/// Fetches the OpenRouter model catalog using the configured API key so
/// the settings page can offer a search/autocomplete picker. Returns a
/// pruned list — only id + name + context_length + pricing — to keep the
/// payload small.
#[tauri::command]
pub async fn list_openrouter_models(app: AppHandle<Wry>) -> Result<serde_json::Value, String> {
    let key = settings::get_openrouter_key(&app)
        .ok_or_else(|| "OpenRouter API key is not set.".to_string())?;
    let client = reqwest::Client::builder()
        .user_agent(concat!("komorebi/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get("https://openrouter.ai/api/v1/models")
        .bearer_auth(&key)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("openrouter: {}", resp.status()));
    }
    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let list = body
        .get("data")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();
    let pruned: Vec<_> = list
        .into_iter()
        .map(|m| {
            serde_json::json!({
                "id": m.get("id"),
                "name": m.get("name"),
                "context_length": m.get("context_length"),
                "pricing": m.get("pricing"),
                "architecture": m.get("architecture"),
            })
        })
        .collect();
    Ok(serde_json::Value::Array(pruned))
}
