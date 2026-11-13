//! Komorebi desktop entrypoint.

mod chat;
mod coach;
mod commands;
mod desktop_cmds;
mod feedback;
mod imagegen;
mod intent;
mod local_classifier;
mod models;
mod proactive;
mod react;
mod relationship;
mod settings;
mod shortcuts;
mod startup;
mod sysctx;
mod tools;
mod tray;
mod weather;
mod window;

use std::sync::Arc;
use tauri::Manager;
use tauri_plugin_global_shortcut::ShortcutState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,komorebi=debug".into()),
        )
        .init();

    let (toggle_input, vision_region, overlay, toggle_window) = shortcuts::defaults();

    tauri::Builder::default()
        .on_window_event(|window, event| {
            match event {
                // Intercept window close → hide to tray instead of quitting.
                // Users still have the tray menu "Quit Komorebi" for a real
                // exit.
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    let _ = window.hide();
                }
                // A mode switch destroys the window on purpose. Rebuild it in
                // the same process (posted to the loop, so the GTK teardown
                // finishes first): the Tauri context, hotkeys, tray, models
                // and chat state all stay warm — only the window and webview
                // are replaced.
                tauri::WindowEvent::Destroyed if window.label() == "main" => {
                    if let Some(mode) = crate::window::pending_switch() {
                        let app = window.app_handle().clone();
                        // Defer: on the main thread `run_on_main_thread`
                        // executes inline, i.e. in the middle of the GTK
                        // teardown. Posting from a helper thread queues the
                        // task on the next loop iteration instead.
                        std::thread::spawn(move || {
                            let inner = app.clone();
                            if let Err(e) = app.run_on_main_thread(move || {
                                recreate_main_window(inner, mode);
                            }) {
                                crate::window::finish_switch();
                                tracing::error!(
                                    ?e,
                                    "overlay: could not queue the window recreation, exiting"
                                );
                                app.exit(1);
                            }
                        });
                    }
                }
                _ => {}
            }
        })
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            use tauri::Manager;
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    shortcuts::dispatch(
                        app,
                        shortcut,
                        &toggle_input,
                        &vision_region,
                        &overlay,
                        &toggle_window,
                    );
                })
                .build(),
        )
        .manage(Arc::new(chat::ChatService::new()))
        .manage(komorebi_voice::tts::PiperTts::new())
        .manage(komorebi_voice::sovits::SoVitsTts::new())
        .manage(komorebi_voice::stt::Recorder::new())
        .manage::<commands::RegionPickerState>(std::sync::Mutex::new(None))
        .manage::<Arc<imagegen::ImageGenState>>(Arc::new(imagegen::ImageGenState::default()))
        .manage::<Arc<intent::IntentState>>(Arc::new(intent::IntentState::default()))
        .invoke_handler(tauri::generate_handler![
            // chat
            commands::chat::send_message,
            commands::chat::cancel_generation,
            commands::chat::reset_chat,
            // system
            commands::system::get_settings,
            commands::system::system_info,
            // routing / OpenRouter
            commands::routing::set_openrouter_key,
            commands::routing::set_mode,
            commands::routing::set_smart_routing,
            commands::routing::set_classifier_model,
            commands::routing::set_rag_enabled,
            commands::routing::set_acp_agent_command,
            commands::routing::set_openrouter_model,
            commands::routing::set_chat_tool_calls_enabled,
            commands::routing::set_llm_gpu_layers,
            commands::routing::list_openrouter_models,
            // assets / models
            commands::models::list_assets,
            commands::models::download_asset,
            commands::models::delete_asset,
            commands::models::set_local_model,
            commands::models::set_local_classifier_model,
            commands::models::clear_local_classifier_model,
            // TTS
            commands::tts::set_piper_binary,
            commands::tts::set_piper_voice,
            commands::tts::set_tts_enabled,
            commands::tts::set_tts_provider,
            commands::tts::set_tts_prosody,
            commands::tts::set_tts_volume,
            commands::tts::set_sovits_config,
            commands::tts::set_openrouter_tts_enabled,
            commands::tts::set_openrouter_tts_model,
            commands::tts::set_openrouter_tts_voice,
            commands::tts::speak_text,
            commands::tts::speak_reaction,
            commands::tts::react_event,
            commands::tts::read_tts_bytes,
            // STT / audio
            commands::stt::set_whisper_model,
            commands::stt::start_recording,
            commands::stt::stop_recording,
            commands::stt::cancel_recording,
            commands::stt::set_wake_word,
            commands::stt::set_listen_enabled,
            commands::stt::set_auto_listen,
            commands::stt::set_openrouter_stt_enabled,
            commands::stt::set_openrouter_stt_model,
            commands::stt::set_faster_whisper_enabled,
            commands::stt::set_faster_whisper_url,
            commands::stt::set_faster_whisper_model,
            commands::stt::set_faster_whisper_language,
            commands::stt::validate_faster_whisper,
            commands::stt::set_deepgram_key,
            commands::stt::clear_deepgram_key,
            commands::stt::validate_deepgram_key,
            commands::stt::set_deepgram_enabled,
            commands::stt::set_deepgram_model,
            commands::stt::set_deepgram_language,
            commands::stt::list_audio_devices,
            commands::stt::set_audio_input,
            commands::stt::set_audio_output,
            // RAG
            commands::rag::rag_list_folders,
            commands::rag::rag_add_folder,
            commands::rag::rag_remove_folder,
            commands::rag::rag_reindex,
            // Vision
            commands::vision::vision_capture_full,
            commands::vision::vision_capture_region,
            commands::vision::vision_with_image,
            commands::vision::enter_region_picker_mode,
            commands::vision::exit_region_picker_mode,
            // Avatar
            commands::avatar::set_avatar_zoom,
            commands::avatar::set_avatar_offset,
            commands::avatar::set_live2d_model,
            commands::overlay::set_input_region,
            commands::window::set_window_overlay,
            commands::window::window_overlay_info,
            // Agent
            commands::agent::set_proactive_enabled,
            commands::agent::set_desktop_automation_enabled,
            commands::agent::set_auto_screen_watch_enabled,
            // Game Coach
            commands::game_coach::set_game_coach_enabled,
            commands::game_coach::set_game_coach_model,
            commands::game_coach::set_game_coach_use_vision,
            // Image generation
            commands::imagegen::generate_image,
            commands::imagegen::cancel_image_generation,
            commands::imagegen::save_generated_image,
            commands::imagegen::set_imagegen_provider,
            commands::imagegen::set_imagegen_openrouter_model,
            commands::imagegen::set_imagegen_replicate_model,
            commands::imagegen::set_imagegen_local_binary,
            commands::imagegen::set_imagegen_local_model,
            commands::imagegen::set_imagegen_device,
            commands::imagegen::set_imagegen_size,
            commands::imagegen::set_imagegen_steps,
            commands::imagegen::set_imagegen_negative_prompt,
            commands::imagegen::set_replicate_token,
            commands::imagegen::clear_replicate_token,
            // Weather
            commands::weather::get_weather,
            commands::weather::set_weather_provider,
            commands::weather::set_weather_api_key,
            commands::weather::clear_weather_api_key,
            commands::weather::set_weather_default_city,
            commands::weather::set_weather_use_ip,
            commands::weather::set_weather_units,
            // Relationship / user
            commands::relationship::get_relationship_state,
            commands::relationship::reset_relationship,
            commands::relationship::set_user_name,
            commands::relationship::set_relationship_visibility,
            commands::relationship::set_relationship_nsfw_allowed,
            commands::relationship::set_relationship_decay_enabled,
            commands::relationship::set_language,
            commands::relationship::get_resolved_language,
            // Feedback / telemetry
            commands::feedback::feedback_record,
            commands::feedback::feedback_stats,
            commands::feedback::feedback_purge,
            commands::feedback::set_telemetry_enabled,
            commands::feedback::set_telemetry_endpoint,
            // Training schedule (Phase 2 stub)
            commands::training::set_training_enabled,
            commands::training::set_training_max_cpu_pct,
            commands::training::set_training_battery_floor_pct,
            commands::training::set_training_min_examples,
            commands::training::set_training_schedule,
            // Desktop automation (separate top-level module)
            desktop_cmds::desktop_workspace_root,
            desktop_cmds::desktop_set_workspace,
            desktop_cmds::desktop_list_screens,
            desktop_cmds::desktop_screenshot,
            desktop_cmds::desktop_screenshot_region,
            desktop_cmds::desktop_click,
            desktop_cmds::desktop_move_cursor,
            desktop_cmds::desktop_type,
            desktop_cmds::desktop_key,
            desktop_cmds::desktop_scroll,
            desktop_cmds::desktop_top_processes,
            desktop_cmds::desktop_active_window,
            desktop_cmds::desktop_context_snapshot,
            desktop_cmds::desktop_write_file,
            desktop_cmds::desktop_read_file,
            desktop_cmds::desktop_list_dir,
            desktop_cmds::desktop_vd_switch_left,
            desktop_cmds::desktop_vd_switch_right,
            desktop_cmds::desktop_vd_create,
            desktop_cmds::desktop_vd_close,
            desktop_cmds::desktop_vd_task_view,
            // Generic tool dispatcher
            tools::run_tool,
            // Intent classifier
            commands::intent::intent_status,
            commands::intent::intent_load,
            commands::intent::intent_classify_debug,
        ])
        .setup(move |app| startup::run(app, toggle_input, vision_region, overlay, toggle_window))
        .build(tauri::generate_context!())
        .expect("error while building the tauri application")
        .run(|_app, event| {
            // Destroying the last window during a mode switch must not end the
            // process: the replacement window is created right afterwards
            // (see the `Destroyed` handler above). A real `app.exit(code)`
            // carries a code and is never prevented.
            if let tauri::RunEvent::ExitRequested { api, code, .. } = &event {
                if code.is_none() && crate::window::switch_in_flight() {
                    api.prevent_exit();
                }
            }
        });
}

/// Build the replacement window after a mode switch destroyed the old one.
///
/// Runs on the main thread (posted by the `Destroyed` handler). If the window
/// cannot be built, exit: without a window the app is invisible and cannot be
/// driven, and a second process would collide with the single-instance lock.
fn recreate_main_window(app: tauri::AppHandle, mode: crate::window::OverlayMode) {
    let result = crate::window::create_main_window(&app, mode);
    let rebuild_ms = crate::window::finish_switch();
    match result {
        Ok(_) => {
            tracing::info!(
                ?mode,
                rebuild_ms = rebuild_ms.unwrap_or_default(),
                "overlay: window recreated, the app stayed alive"
            );
        }
        Err(e) => {
            tracing::error!(?e, ?mode, "overlay: window recreation failed, exiting");
            app.exit(1);
        }
    }
}
