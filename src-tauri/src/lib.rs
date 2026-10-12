//! Tauri application shell. This crate is intentionally thin: it wires the
//! [`sfe_engine::Engine`] into Tauri state, exposes `#[tauri::command]`
//! wrappers under [`commands`], and emits typed events. No filesystem,
//! database, or search logic lives here.

mod commands;
mod events;
mod thumbs;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,sfe=debug".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .register_uri_scheme_protocol("thumb", thumbs::handle)
        .setup(|app| {
            let data_dir = app.path().app_local_data_dir()?;
            let engine = sfe_engine::Engine::open(&data_dir)?;
            tracing::info!(dir = %data_dir.display(), "engine ready");
            events::forward_status(app.handle().clone(), engine.subscribe_status());
            events::forward_models(app.handle().clone(), engine.subscribe_models());
            app.manage(engine);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_version,
            commands::data_dir,
            commands::roots_list,
            commands::roots_add,
            commands::roots_pick_and_add,
            commands::roots_remove,
            commands::index_status,
            commands::index_pause,
            commands::index_resume,
            commands::index_rescan,
            commands::search,
            commands::search_explain,
            commands::file_details,
            commands::models_status,
            commands::models_install,
            commands::index_problems,
            commands::index_retry_failed,
            commands::app_stats,
            commands::index_purge,
            commands::file_open,
            commands::file_reveal,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
