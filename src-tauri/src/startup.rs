//! One-time post-builder initialization: register hotkeys, install tray,
//! open the RAG index, kick off the proactive/coach loops, etc.

use std::sync::Arc;
use tauri::Manager;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

use crate::feedback;
use crate::tray;

/// Runs every startup task that needs the fully-built `App` (state,
/// handles, paths, …). Errors that aren't fatal to the app are logged
/// and swallowed; only truly unrecoverable ones are propagated.
pub(crate) fn run(
    app: &mut tauri::App,
    toggle_input: Shortcut,
    vision_region: Shortcut,
    overlay: Shortcut,
    toggle_window: Shortcut,
) -> Result<(), Box<dyn std::error::Error>> {
    app.global_shortcut().register(toggle_input)?;
    if let Err(e) = app.global_shortcut().register(vision_region) {
        tracing::warn!(?e, "failed to register Alt+V hotkey");
    }
    if let Err(e) = app.global_shortcut().register(overlay) {
        tracing::warn!(?e, "failed to register Alt+O overlay hotkey");
    }
    if let Err(e) = app.global_shortcut().register(toggle_window) {
        tracing::warn!(?e, "failed to register F11 window toggle hotkey");
    }

    // The main window is created with `visible: false` so that the overlay's
    // layer surface can be attached while the widget is still unmapped
    // (gtk-layer-shell re-creates the GdkWindow and refuses a mapped widget).
    // It is shown below unless this is a headless run.
    //
    // The layer surface is attached *before* the forced mode is applied:
    // `apply_overlay` needs to know whether the window already is a layer
    // surface, otherwise it warns about tiling and tries to fill a monitor
    // that does not exist yet (the window is still unmapped at this point).
    //
    // `KOMOREBI_START_MODE` (bug repro / QA) pins the mode over the remembered
    // choice; the frontend uses the same value to tell a pinned mode apart.
    if let Some(w) = app.get_webview_window("main") {
        crate::window::init_layer_shell(&w);
        if let Some(m) = crate::window::start_mode() {
            let effective = crate::window::apply_overlay(&w, m);
            tracing::info!(
                ?effective,
                "startup: forced window mode from KOMOREBI_START_MODE"
            );
        }
    }

    // Phase 1: spawn the feedback-telemetry uploader. Runs even when
    // the user hasn't opted in — it's a no-op until the toggle is
    // flipped, then drains the local queue periodically.
    feedback::spawn_uploader(app.handle().clone());

    tray::install(app)?;

    // Initialize the RAG index in the app's data dir and stash it on
    // the Tauri state map.
    match app.path().app_data_dir() {
        Ok(dir) => {
            let db = dir.join("rag.db");
            match komorebi_storage::RagIndex::open(&db) {
                Ok(idx) => {
                    app.manage(Arc::new(idx));
                    tracing::info!(?db, "RAG index opened");
                }
                Err(e) => tracing::warn!(?e, "failed to open RAG index"),
            }
        }
        Err(e) => tracing::warn!(?e, "app_data_dir unavailable; RAG disabled"),
    }

    // Apply persisted TTS config to the shared handle.
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        crate::commands::reload_tts(&handle).await;
    });

    // Proactive agent — polls active window/processes and nudges the
    // user when appropriate (only if enabled in settings).
    crate::proactive::spawn(app.handle().clone());
    crate::coach::spawn(app.handle().clone());

    if let Some(w) = app.get_webview_window("main") {
        // The window starts hidden (see `visible: false` above); a normal run
        // shows it once the overlay surface is set up.
        if let Err(e) = w.show() {
            tracing::warn!(?e, "failed to show the main window");
        }
    }

    // Local intent classifier: if the user has previously downloaded
    // the embedding model (cache dir exists), warm it up in the
    // background. Otherwise stay dormant — the model is opt-in and
    // only fetched when the user explicitly invokes `intent_load`.
    if let Ok(data_dir) = app.path().app_data_dir() {
        let intent_cache = data_dir.join("intent");
        if intent_cache.exists()
            && std::fs::read_dir(&intent_cache)
                .map(|mut d| d.next().is_some())
                .unwrap_or(false)
        {
            if let Some(state) = app.try_state::<std::sync::Arc<crate::intent::IntentState>>() {
                let state = state.inner().clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = state.load(intent_cache).await {
                        tracing::warn!(?e, "intent classifier auto-load failed");
                    } else {
                        tracing::info!("intent classifier ready");
                    }
                });
            }
        }
    }

    tracing::info!("Komorebi started");
    Ok(())
}
