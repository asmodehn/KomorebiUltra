//! Tauri command surface — split by domain.
//!
//! All `#[tauri::command]` functions live in submodules and are re-exported
//! here, so `lib.rs` can keep referencing them as `commands::xxx` inside
//! the `tauri::generate_handler!` macro without changes.
//!
//! # Layout
//!
//! * [`util`] — internal helpers (id generation).
//! * [`chat`] — chat lifecycle (send / cancel / reset).
//! * [`models`] — asset catalog + active-model setters.
//! * [`routing`] — chat mode, OpenRouter, smart routing, RAG, GPU layers.
//! * [`tts`] — Piper / SoVITS / OpenRouter TTS, plus shared synthesis
//!   helpers ([`tts::synthesize_via_provider`], [`tts::emit_tts_wav`],
//!   [`tts::reload_tts`]) consumed from `chat.rs` and `lib.rs`.
//! * [`stt`] — recording, Whisper / Deepgram / faster-whisper / OpenRouter
//!   STT + audio device selection.
//! * [`vision`] — screen / region / attached-image vision capture and the
//!   region-picker overlay state ([`vision::RegionPickerState`]).
//! * [`imagegen`] — image-generation provider settings + spawn/cancel.
//! * [`avatar`] — Live2D model URL, zoom, on-screen offset.
//! * [`agent`] — proactive / desktop-automation toggles.
//! * [`game_coach`] — Game Coach (text/vision split, proposal 0002).
//! * [`weather`] — weather provider + report.
//! * [`relationship`] — user identity, relationship layer, language.
//! * [`system`] — settings snapshot + system info.
//! * [`rag`] — RAG folder management.
//! * [`feedback`] — Phase 1 feedback telemetry.
//! * [`training`] — Phase 2 training schedule stub.
//!
//! `#[allow(unused_imports)]` covers items that are only consumed through
//! the flat `commands::xxx` facade by the `generate_handler!` macro
//! invocation in `lib.rs` — Rust's lint can't see through the macro and
//! would otherwise flag every re-exported command as unused.

#![allow(unused_imports)]

pub mod agent;
pub mod avatar;
pub mod chat;
pub mod feedback;
pub mod game_coach;
pub mod imagegen;
pub mod intent;
pub mod models;
pub mod rag;
pub mod relationship;
pub mod routing;
pub mod stt;
pub mod system;
pub mod training;
pub mod tts;
mod util;
pub mod vision;
pub mod weather;

// --- Public command facade (consumed by `tauri::generate_handler!`) ------

pub use agent::{
    set_auto_screen_watch_enabled, set_desktop_automation_enabled, set_proactive_enabled,
};
pub use avatar::{set_avatar_offset, set_avatar_zoom, set_live2d_model};
pub use chat::{cancel_generation, reset_chat, send_message};
pub use feedback::{
    feedback_purge, feedback_record, feedback_stats, set_telemetry_enabled, set_telemetry_endpoint,
    FeedbackStatsDto,
};
pub use game_coach::{set_game_coach_enabled, set_game_coach_model, set_game_coach_use_vision};
pub use imagegen::{
    cancel_image_generation, clear_replicate_token, generate_image, save_generated_image,
    set_imagegen_device, set_imagegen_local_binary, set_imagegen_local_model,
    set_imagegen_negative_prompt, set_imagegen_openrouter_model, set_imagegen_provider,
    set_imagegen_replicate_model, set_imagegen_size, set_imagegen_steps, set_replicate_token,
};
pub use models::{
    clear_local_classifier_model, delete_asset, download_asset, list_assets,
    set_local_classifier_model, set_local_model,
};
pub use rag::{rag_add_folder, rag_list_folders, rag_reindex, rag_remove_folder};
pub use relationship::{
    get_relationship_state, get_resolved_language, reset_relationship, set_language,
    set_relationship_decay_enabled, set_relationship_nsfw_allowed, set_relationship_visibility,
    set_user_name,
};
pub use routing::{
    list_openrouter_models, set_acp_agent_command, set_chat_tool_calls_enabled,
    set_classifier_model, set_llm_gpu_layers, set_mode, set_openrouter_key, set_openrouter_model,
    set_rag_enabled, set_smart_routing,
};
pub use stt::{
    cancel_recording, clear_deepgram_key, list_audio_devices, set_audio_input, set_audio_output,
    set_auto_listen, set_deepgram_enabled, set_deepgram_key, set_deepgram_language,
    set_deepgram_model, set_faster_whisper_enabled, set_faster_whisper_language,
    set_faster_whisper_model, set_faster_whisper_url, set_listen_enabled,
    set_openrouter_stt_enabled, set_openrouter_stt_model, set_wake_word, set_whisper_model,
    start_recording, stop_recording, validate_deepgram_key, validate_faster_whisper,
};
pub use system::{get_settings, system_info};
pub use training::{
    set_training_battery_floor_pct, set_training_enabled, set_training_max_cpu_pct,
    set_training_min_examples, set_training_schedule,
};
pub use tts::{
    emit_tts_wav, react_event, read_tts_bytes, reload_tts, set_openrouter_tts_enabled,
    set_openrouter_tts_model, set_openrouter_tts_voice, set_piper_binary, set_piper_voice,
    set_sovits_config, set_tts_enabled, set_tts_prosody, set_tts_provider, set_tts_volume,
    speak_reaction, speak_text, synthesize_via_provider,
};
pub use vision::{
    enter_region_picker_mode, exit_region_picker_mode, vision_capture_full, vision_capture_region,
    vision_with_image, RegionPickerState, SavedGeometry, VisionRegionArgs,
};
pub use weather::{
    clear_weather_api_key, get_weather, set_weather_api_key, set_weather_default_city,
    set_weather_provider, set_weather_units, set_weather_use_ip,
};
