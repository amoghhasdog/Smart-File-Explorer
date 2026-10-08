//! Tauri application shell. This crate is intentionally thin: it wires the
//! [`sfe_engine::Engine`] into Tauri state, exposes `#[tauri::command]`
//! wrappers under [`commands`], and emits typed events. No filesystem,
//! database, or search logic lives here.

mod commands;

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
        .invoke_handler(tauri::generate_handler![commands::app_version])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
