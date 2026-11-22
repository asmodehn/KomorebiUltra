//! Settings persistence via `tauri-plugin-store`.
//!
//! Phase 1 storage choice: API keys live in a JSON store encrypted only
//! by the OS file permissions on the app-data dir. Upgrade path to the
//! OS keyring is planned for Phase 3 hardening.
//!
//! # Layout
//!
//! Each domain owns a submodule with its own keys, getters and setters:
//!
//! * [`store`] — low-level JSON read/write helpers shared by every submodule.
//! * [`defaults`] — `DEFAULT_*` constants exposed to the UI.
//! * [`api_keys`] — OpenRouter / Deepgram / Replicate / Weather keys.
//! * [`routing`] — chat mode, OpenRouter model, smart routing,
//!   classifier model, local model paths, GPU layers, RAG, tool calls.
//! * [`voice`] — audio devices, wake word, listen toggles, all STT
//!   backends (local Whisper, faster-whisper, Deepgram, OpenRouter STT).
//! * [`tts`] — TTS provider, prosody, Piper, SoVITS, OpenRouter TTS.
//! * [`avatar`] — Live2D model URL, zoom, on-screen offsets.
//! * [`agent`] — agent workspace, proactive, desktop automation.
//! * [`game_coach`] — Game Coach (vision/text split, see proposal 0002).
//! * [`imagegen`] — Image-generation provider + parameters.
//! * [`weather`] — Weather provider + helper to assemble a `WeatherConfig`.
//! * [`user`] — user name, language, relationship layer.
//! * [`telemetry`] — feedback telemetry + anonymous install token.
//! * [`training`] — Phase 2 stub for local LoRA training.
//!
//! Everything is re-exported at the [`crate::settings`] level so call
//! sites (e.g. `settings::get_openrouter_key(app)`) keep working as
//! before — this module is a structural refactor, not an API break.
//!
//! Note on `#![allow(unused_imports)]`: the per-domain `pub use` blocks
//! below act as a stable facade. Some items are not yet consumed via the
//! `crate::settings::*` path (callers may reach them through the
//! submodule directly, or new commands may pick them up later). Rust's
//! `unused_imports` lint flags these even though they remain part of the
//! public API. Allowing the lint here keeps the facade complete without
//! having to thread the lint suppression through each individual block.

#![allow(unused_imports)]

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Wry};

type Result<T> = anyhow::Result<T>;

mod agent;
mod api_keys;
mod avatar;
mod defaults;
mod game_coach;
mod imagegen;
mod routing;
mod store;
mod telemetry;
mod training;
mod tts;
mod user;
mod voice;
mod weather;

// --- Public defaults ------------------------------------------------------

pub use defaults::{
    DEFAULT_DEEPGRAM_MODEL, DEFAULT_FASTER_WHISPER_MODEL, DEFAULT_FASTER_WHISPER_URL,
    DEFAULT_GAME_COACH_MODEL, DEFAULT_IMAGEGEN_DEVICE, DEFAULT_IMAGEGEN_HEIGHT,
    DEFAULT_IMAGEGEN_OR_MODEL, DEFAULT_IMAGEGEN_PROVIDER, DEFAULT_IMAGEGEN_REPLICATE_MODEL,
    DEFAULT_IMAGEGEN_STEPS, DEFAULT_IMAGEGEN_WIDTH, DEFAULT_LANGUAGE, DEFAULT_OPENROUTER_STT_MODEL,
    DEFAULT_OPENROUTER_TTS_MODEL, DEFAULT_OPENROUTER_TTS_VOICE, DEFAULT_RELATIONSHIP_VISIBILITY,
    DEFAULT_TELEMETRY_ENDPOINT, DEFAULT_TRAINING_BATTERY_FLOOR_PCT, DEFAULT_TRAINING_MAX_CPU_PCT,
    DEFAULT_TRAINING_MIN_EXAMPLES, DEFAULT_TRAINING_SCHEDULE, DEFAULT_WEATHER_PROVIDER,
    DEFAULT_WEATHER_UNITS,
};

// --- Public API surface (re-exported per-domain) -------------------------

pub use agent::{
    get_agent_workspace, get_auto_screen_watch_enabled, get_desktop_automation_enabled,
    get_proactive_enabled, set_agent_workspace, set_auto_screen_watch_enabled,
    set_desktop_automation_enabled, set_proactive_enabled,
};
pub use api_keys::{
    get_deepgram_key, get_openrouter_key, get_replicate_token, get_weather_api_key,
    set_deepgram_key, set_openrouter_key, set_replicate_token, set_weather_api_key,
};
pub use avatar::{set_avatar_offset_x, set_avatar_offset_y, set_avatar_zoom, set_live2d_model_url};
pub use game_coach::{
    get_game_coach_enabled, get_game_coach_model, get_game_coach_use_vision,
    set_game_coach_enabled, set_game_coach_model, set_game_coach_use_vision,
};
pub use imagegen::{
    get_imagegen_device, get_imagegen_height, get_imagegen_local_binary, get_imagegen_local_model,
    get_imagegen_negative_prompt, get_imagegen_openrouter_model, get_imagegen_provider,
    get_imagegen_replicate_model, get_imagegen_steps, get_imagegen_width, set_imagegen_device,
    set_imagegen_local_binary, set_imagegen_local_model, set_imagegen_negative_prompt,
    set_imagegen_openrouter_model, set_imagegen_provider, set_imagegen_replicate_model,
    set_imagegen_size, set_imagegen_steps,
};
pub use routing::{
    get_acp_agent_command, get_chat_tool_calls_enabled, get_classifier_model, get_gpu_layers,
    get_local_classifier_model_path, get_local_model_path, get_mode, get_openrouter_model,
    get_rag_enabled, get_smart_routing, set_acp_agent_command, set_chat_tool_calls_enabled,
    set_classifier_model, set_gpu_layers, set_local_classifier_model_path, set_local_model_path,
    set_mode, set_openrouter_model, set_rag_enabled, set_smart_routing,
};
pub use telemetry::{
    ensure_anon_token, get_anon_token, get_telemetry_enabled, get_telemetry_endpoint,
    rotate_anon_token, set_telemetry_enabled, set_telemetry_endpoint,
};
pub use training::{
    get_training_enabled, set_training_battery_floor_pct, set_training_enabled,
    set_training_max_cpu_pct, set_training_min_examples, set_training_schedule,
};
pub use tts::{
    get_openrouter_tts_enabled, get_openrouter_tts_model, get_openrouter_tts_voice,
    get_piper_binary, get_piper_voice, get_sovits_config, get_tts_enabled, get_tts_length_scale,
    get_tts_noise_scale, get_tts_noise_w, get_tts_provider, set_openrouter_tts_enabled,
    set_openrouter_tts_model, set_openrouter_tts_voice, set_piper_binary, set_piper_voice,
    set_sovits_endpoint, set_sovits_prompt_lang, set_sovits_prompt_text, set_sovits_ref_audio,
    set_sovits_speed, set_sovits_text_lang, set_tts_enabled, set_tts_length_scale,
    set_tts_noise_scale, set_tts_noise_w, set_tts_provider, set_tts_volume,
};
pub use user::{
    clear_relationship_state, get_language, get_relationship_decay_enabled,
    get_relationship_nsfw_allowed, get_relationship_visibility, get_user_name,
    read_relationship_state, resolve_language, set_language, set_relationship_decay_enabled,
    set_relationship_nsfw_allowed, set_relationship_visibility, set_user_name,
    write_relationship_state,
};
pub use voice::{
    get_audio_input, get_auto_listen, get_deepgram_enabled, get_deepgram_language,
    get_deepgram_model, get_faster_whisper_enabled, get_faster_whisper_language,
    get_faster_whisper_model, get_faster_whisper_url, get_listen_enabled,
    get_openrouter_stt_enabled, get_openrouter_stt_model, get_whisper_model_path, set_audio_input,
    set_audio_output, set_auto_listen, set_deepgram_enabled, set_deepgram_language,
    set_deepgram_model, set_faster_whisper_enabled, set_faster_whisper_language,
    set_faster_whisper_model, set_faster_whisper_url, set_listen_enabled,
    set_openrouter_stt_enabled, set_openrouter_stt_model, set_wake_word, set_whisper_model_path,
};
pub use weather::{
    get_weather_default_city, get_weather_provider, get_weather_units, get_weather_use_ip,
    set_weather_default_city, set_weather_provider, set_weather_units, set_weather_use_ip,
    weather_config,
};

// --- Public snapshot (consumed by the frontend) --------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicSettings {
    pub has_openrouter_key: bool,
    pub openrouter_model: String,
    pub mode: String,
    pub local_model_path: Option<String>,
    pub local_classifier_model_path: Option<String>,
    pub tts_enabled: bool,
    pub piper_binary_path: Option<String>,
    pub piper_voice_path: Option<String>,
    pub live2d_model_url: Option<String>,
    pub whisper_model_path: Option<String>,
    pub stt_available: bool,
    pub wake_word: Option<String>,
    pub listen_enabled: bool,
    pub smart_routing: bool,
    pub classifier_model: String,
    pub rag_enabled: bool,
    pub acp_agent_command: Option<String>,
    pub audio_input_device: Option<String>,
    pub audio_output_device: Option<String>,
    pub llm_gpu_layers: Option<i64>,
    pub auto_listen: bool,
    pub tts_provider: String,
    pub tts_length_scale: Option<f64>,
    pub tts_noise_scale: Option<f64>,
    pub tts_noise_w: Option<f64>,
    pub tts_volume: f64,
    pub sovits_endpoint: Option<String>,
    pub sovits_ref_audio: Option<String>,
    pub sovits_prompt_text: Option<String>,
    pub sovits_prompt_lang: String,
    pub sovits_text_lang: String,
    pub sovits_speed: f64,
    pub agent_workspace: Option<String>,
    pub proactive_enabled: bool,
    pub desktop_automation_enabled: bool,
    pub auto_screen_watch_enabled: bool,
    pub chat_tool_calls_enabled: bool,
    pub openrouter_tts_enabled: bool,
    pub openrouter_tts_model: String,
    pub openrouter_tts_voice: String,
    pub openrouter_stt_enabled: bool,
    pub openrouter_stt_model: String,
    pub game_coach_enabled: bool,
    pub game_coach_model: String,
    pub game_coach_use_vision: bool,
    pub faster_whisper_enabled: bool,
    pub faster_whisper_url: String,
    pub faster_whisper_model: String,
    pub faster_whisper_language: Option<String>,
    pub has_deepgram_key: bool,
    pub deepgram_enabled: bool,
    pub deepgram_model: String,
    pub deepgram_language: Option<String>,
    pub avatar_zoom: f64,
    pub avatar_offset_x: f64,
    pub avatar_offset_y: f64,
    pub imagegen_provider: String,
    pub imagegen_openrouter_model: String,
    pub imagegen_replicate_model: String,
    pub imagegen_local_binary: Option<String>,
    pub imagegen_local_model: Option<String>,
    pub imagegen_device: String,
    pub imagegen_width: i64,
    pub imagegen_height: i64,
    pub imagegen_steps: i64,
    pub imagegen_negative_prompt: Option<String>,
    pub has_replicate_token: bool,
    pub weather_provider: String,
    pub weather_default_city: Option<String>,
    pub weather_use_ip: bool,
    pub weather_units: String,
    pub has_weather_api_key: bool,
    pub user_name: Option<String>,
    pub relationship_visibility: String,
    pub relationship_nsfw_allowed: bool,
    pub relationship_decay_enabled: bool,
    pub language: String,
    // --- Phase 1: feedback telemetry --------------------------------------
    pub telemetry_enabled: bool,
    pub telemetry_endpoint: String,
    /// Opaque per-install random token for rate-limiting at the server.
    /// Not a user identifier; can be rotated by purging feedback history.
    pub telemetry_anon_token: Option<String>,
    // --- Phase 2 stub: local LoRA training --------------------------------
    pub training_enabled: bool,
    pub training_max_cpu_pct: i64,
    pub training_battery_floor_pct: i64,
    pub training_min_examples: i64,
    pub training_schedule: String,
}

pub fn public_snapshot(app: &AppHandle<Wry>) -> PublicSettings {
    PublicSettings {
        has_openrouter_key: api_keys::get_openrouter_key(app).is_some(),
        openrouter_model: routing::get_openrouter_model(app),
        mode: routing::mode_str(app).to_string(),
        local_model_path: routing::get_local_model_path(app),
        local_classifier_model_path: routing::read_local_classifier_model_path(app),
        tts_enabled: tts::get_tts_enabled(app),
        piper_binary_path: tts::get_piper_binary(app),
        piper_voice_path: tts::get_piper_voice(app),
        live2d_model_url: avatar::read_live2d_model_url(app),
        whisper_model_path: voice::get_whisper_model_path(app),
        stt_available: komorebi_voice::stt::is_available(),
        wake_word: voice::read_wake_word(app),
        listen_enabled: voice::get_listen_enabled(app),
        smart_routing: routing::get_smart_routing(app),
        classifier_model: routing::get_classifier_model(app),
        rag_enabled: routing::get_rag_enabled(app),
        acp_agent_command: routing::get_acp_agent_command(app),
        audio_input_device: voice::get_audio_input(app),
        audio_output_device: voice::read_audio_output(app),
        llm_gpu_layers: routing::get_gpu_layers(app),
        auto_listen: voice::get_auto_listen(app),
        tts_provider: tts::get_tts_provider(app),
        tts_length_scale: tts::get_tts_length_scale(app),
        tts_noise_scale: tts::get_tts_noise_scale(app),
        tts_noise_w: tts::get_tts_noise_w(app),
        tts_volume: tts::read_tts_volume(app),
        sovits_endpoint: tts::read_sovits_endpoint(app),
        sovits_ref_audio: tts::read_sovits_ref_audio(app),
        sovits_prompt_text: tts::read_sovits_prompt_text(app),
        sovits_prompt_lang: tts::read_sovits_prompt_lang(app),
        sovits_text_lang: tts::read_sovits_text_lang(app),
        sovits_speed: tts::read_sovits_speed(app),
        agent_workspace: agent::get_agent_workspace(app),
        proactive_enabled: agent::get_proactive_enabled(app),
        desktop_automation_enabled: agent::get_desktop_automation_enabled(app),
        auto_screen_watch_enabled: agent::get_auto_screen_watch_enabled(app),
        chat_tool_calls_enabled: routing::get_chat_tool_calls_enabled(app),
        openrouter_tts_enabled: tts::get_openrouter_tts_enabled(app),
        openrouter_tts_model: tts::get_openrouter_tts_model(app),
        openrouter_tts_voice: tts::get_openrouter_tts_voice(app),
        openrouter_stt_enabled: voice::get_openrouter_stt_enabled(app),
        openrouter_stt_model: voice::get_openrouter_stt_model(app),
        game_coach_enabled: game_coach::get_game_coach_enabled(app),
        game_coach_model: game_coach::get_game_coach_model(app),
        game_coach_use_vision: game_coach::get_game_coach_use_vision(app),
        faster_whisper_enabled: voice::get_faster_whisper_enabled(app),
        faster_whisper_url: voice::get_faster_whisper_url(app),
        faster_whisper_model: voice::get_faster_whisper_model(app),
        faster_whisper_language: voice::get_faster_whisper_language(app),
        has_deepgram_key: api_keys::get_deepgram_key(app).is_some(),
        deepgram_enabled: voice::get_deepgram_enabled(app),
        deepgram_model: voice::get_deepgram_model(app),
        deepgram_language: voice::get_deepgram_language(app),
        avatar_zoom: avatar::read_avatar_zoom(app),
        avatar_offset_x: avatar::read_avatar_offset_x(app),
        avatar_offset_y: avatar::read_avatar_offset_y(app),
        imagegen_provider: imagegen::get_imagegen_provider(app),
        imagegen_openrouter_model: imagegen::get_imagegen_openrouter_model(app),
        imagegen_replicate_model: imagegen::get_imagegen_replicate_model(app),
        imagegen_local_binary: imagegen::get_imagegen_local_binary(app),
        imagegen_local_model: imagegen::get_imagegen_local_model(app),
        imagegen_device: imagegen::get_imagegen_device(app),
        imagegen_width: imagegen::get_imagegen_width(app),
        imagegen_height: imagegen::get_imagegen_height(app),
        imagegen_steps: imagegen::get_imagegen_steps(app),
        imagegen_negative_prompt: imagegen::get_imagegen_negative_prompt(app),
        has_replicate_token: api_keys::get_replicate_token(app).is_some(),
        weather_provider: weather::get_weather_provider(app),
        weather_default_city: weather::get_weather_default_city(app),
        weather_use_ip: weather::get_weather_use_ip(app),
        weather_units: weather::get_weather_units(app),
        has_weather_api_key: api_keys::get_weather_api_key(app).is_some(),
        user_name: user::get_user_name(app),
        relationship_visibility: user::get_relationship_visibility(app),
        relationship_nsfw_allowed: user::get_relationship_nsfw_allowed(app),
        relationship_decay_enabled: user::get_relationship_decay_enabled(app),
        language: user::get_language(app),
        telemetry_enabled: telemetry::get_telemetry_enabled(app),
        telemetry_endpoint: telemetry::get_telemetry_endpoint(app),
        telemetry_anon_token: telemetry::get_anon_token(app),
        training_enabled: training::get_training_enabled(app),
        training_max_cpu_pct: store::get_i64(app, training::KEY_MAX_CPU)
            .unwrap_or(DEFAULT_TRAINING_MAX_CPU_PCT),
        training_battery_floor_pct: store::get_i64(app, training::KEY_BATTERY_FLOOR)
            .unwrap_or(DEFAULT_TRAINING_BATTERY_FLOOR_PCT),
        training_min_examples: store::get_i64(app, training::KEY_MIN_EXAMPLES)
            .unwrap_or(DEFAULT_TRAINING_MIN_EXAMPLES),
        training_schedule: store::read_string(app, training::KEY_SCHEDULE)
            .unwrap_or_else(|| DEFAULT_TRAINING_SCHEDULE.to_string()),
    }
}
