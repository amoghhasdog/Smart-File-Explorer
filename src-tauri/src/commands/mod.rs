//! `#[tauri::command]` wrappers. Each command validates input, calls into
//! `sfe_engine`, and maps errors to a small serialisable [`IpcError`].
//! Real commands (roots, index, search, file, models) arrive in later phases.

use serde::Serialize;
use sfe_engine::Engine;
use tauri::State;

/// Error shape sent to the UI. `code` is stable and machine-readable;
/// `message` is for display.
#[derive(Debug, Serialize)]
#[allow(dead_code)] // first fallible commands land in Phase 2
pub struct IpcError {
    pub code: &'static str,
    pub message: String,
}

impl From<sfe_engine::EngineError> for IpcError {
    fn from(e: sfe_engine::EngineError) -> Self {
        let code = match &e {
            sfe_engine::EngineError::DataDir(..) => "data_dir",
            sfe_engine::EngineError::Db(_) => "database",
        };
        IpcError {
            code,
            message: e.to_string(),
        }
    }
}

/// Returns the application version. Used by the UI shell and as an IPC smoke test.
#[tauri::command]
pub fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Where the index lives on disk (shown in settings).
#[tauri::command]
pub fn data_dir(engine: State<'_, Engine>) -> String {
    engine.data_dir().display().to_string()
}
