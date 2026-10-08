//! `#[tauri::command]` wrappers. Each command validates input, calls into
//! `sfe_engine`, and maps errors to a small serialisable `IpcError`.
//! Real commands (roots, index, search, file, models) arrive in later phases.

/// Returns the application version. Used by the UI shell and as an IPC smoke test.
#[tauri::command]
pub fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
