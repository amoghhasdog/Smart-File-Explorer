//! `#[tauri::command]` wrappers. Each command validates input, calls into
//! `sfe_engine`, and maps errors to a small serialisable [`IpcError`].
//! Commands never receive filesystem paths from the UI except the one the
//! native folder picker returns.

use serde::Serialize;
use sfe_core::{FileId, Root, RootId};
use sfe_engine::{Engine, EngineError, IndexStatus, SearchHit};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

/// Error shape sent to the UI. `code` is stable and machine-readable;
/// `message` is for display.
#[derive(Debug, Serialize)]
pub struct IpcError {
    pub code: &'static str,
    pub message: String,
}

impl From<EngineError> for IpcError {
    fn from(e: EngineError) -> Self {
        let code = match &e {
            EngineError::DataDir(..) => "data_dir",
            EngineError::Db(_) => "database",
            EngineError::Fs(f) if f.is_not_found() => "not_found",
            EngineError::Fs(_) => "filesystem",
            EngineError::Index(_) => "index",
            EngineError::Runtime(_) => "runtime",
            EngineError::NestedRoot(..) => "nested_root",
            EngineError::NoSuchRoot(_) => "no_such_root",
        };
        IpcError {
            code,
            message: e.to_string(),
        }
    }
}

impl From<tauri_plugin_opener::Error> for IpcError {
    fn from(e: tauri_plugin_opener::Error) -> Self {
        IpcError {
            code: "opener",
            message: e.to_string(),
        }
    }
}

type IpcResult<T> = Result<T, IpcError>;

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

#[tauri::command]
pub async fn roots_list(engine: State<'_, Engine>) -> IpcResult<Vec<Root>> {
    Ok(engine.list_roots()?)
}

#[tauri::command]
pub async fn roots_add(engine: State<'_, Engine>, path: String) -> IpcResult<Root> {
    Ok(engine.add_root(path)?)
}

/// Open the native folder picker; on a choice, register it as a root.
#[tauri::command]
pub async fn roots_pick_and_add(
    app: AppHandle,
    engine: State<'_, Engine>,
) -> IpcResult<Option<Root>> {
    let picked = app.dialog().file().blocking_pick_folder();
    match picked.and_then(|p| p.into_path().ok()) {
        Some(path) => Ok(Some(engine.add_root(path)?)),
        None => Ok(None),
    }
}

#[tauri::command]
pub async fn roots_remove(engine: State<'_, Engine>, id: RootId) -> IpcResult<()> {
    Ok(engine.remove_root(id)?)
}

#[tauri::command]
pub async fn index_status(engine: State<'_, Engine>) -> IpcResult<IndexStatus> {
    Ok(engine.status())
}

#[tauri::command]
pub async fn index_pause(engine: State<'_, Engine>) -> IpcResult<()> {
    engine.pause();
    Ok(())
}

#[tauri::command]
pub async fn index_resume(engine: State<'_, Engine>) -> IpcResult<()> {
    engine.resume();
    Ok(())
}

#[tauri::command]
pub async fn index_rescan(engine: State<'_, Engine>, id: RootId) -> IpcResult<()> {
    Ok(engine.rescan(id)?)
}

#[tauri::command]
pub async fn search(
    engine: State<'_, Engine>,
    query: String,
    limit: Option<usize>,
    offset: Option<usize>,
) -> IpcResult<Vec<SearchHit>> {
    let limit = limit.unwrap_or(50).clamp(1, 200);
    Ok(engine.search(&query, limit, offset.unwrap_or(0))?)
}

/// Open a file with its default application. The path comes from the index,
/// never from the UI.
#[tauri::command]
pub async fn file_open(app: AppHandle, engine: State<'_, Engine>, id: FileId) -> IpcResult<()> {
    let Some(rec) = engine.file(id)? else {
        return Err(IpcError {
            code: "not_found",
            message: format!("no file {id}"),
        });
    };
    app.opener().open_path(rec.path, None::<&str>)?;
    Ok(())
}

/// Show the file in Windows Explorer.
#[tauri::command]
pub async fn file_reveal(app: AppHandle, engine: State<'_, Engine>, id: FileId) -> IpcResult<()> {
    let Some(rec) = engine.file(id)? else {
        return Err(IpcError {
            code: "not_found",
            message: format!("no file {id}"),
        });
    };
    app.opener().reveal_item_in_dir(rec.path)?;
    Ok(())
}
