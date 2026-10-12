//! `#[tauri::command]` wrappers. Each command validates input, calls into
//! `sfe_engine`, and maps errors to a small serialisable [`IpcError`].
//! Commands never receive filesystem paths from the UI except the one the
//! native folder picker returns.

use serde::Serialize;
use sfe_core::{FileId, Root, RootId};
use sfe_engine::{
    Engine, EngineError, FileDetails, IndexStatus, ModelKind, ModelStatus, Problem, Query,
    SearchResponse, Stats,
};
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
            EngineError::Search(_) => "search",
            EngineError::Embed(_) => "model",
            EngineError::Vector(_) => "vector_index",
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
) -> IpcResult<SearchResponse> {
    let limit = limit.unwrap_or(50).clamp(1, 200);
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.search(&query, limit, offset.unwrap_or(0)))
        .await
        .map_err(|e| IpcError {
            code: "runtime",
            message: e.to_string(),
        })?
        .map_err(IpcError::from)
}

/// Parse only — drives the interpretation chips while the user types.
#[tauri::command]
pub async fn search_explain(engine: State<'_, Engine>, query: String) -> IpcResult<Query> {
    Ok(engine.explain(&query))
}

#[tauri::command]
pub async fn file_details(engine: State<'_, Engine>, id: FileId) -> IpcResult<Option<FileDetails>> {
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.file_details(id))
        .await
        .map_err(|e| IpcError {
            code: "runtime",
            message: e.to_string(),
        })?
        .map_err(IpcError::from)
}

#[tauri::command]
pub async fn models_status(engine: State<'_, Engine>) -> IpcResult<Vec<ModelStatus>> {
    Ok(engine.models_status())
}

/// The user consented to downloading a model (one time, then offline).
#[tauri::command]
pub async fn models_install(engine: State<'_, Engine>, kind: ModelKind) -> IpcResult<()> {
    Ok(engine.models_install(kind)?)
}

/// Files that failed to index, newest first.
#[tauri::command]
pub async fn index_problems(
    engine: State<'_, Engine>,
    limit: Option<usize>,
) -> IpcResult<Vec<Problem>> {
    Ok(engine.problems(limit.unwrap_or(200))?)
}

#[tauri::command]
pub async fn index_retry_failed(engine: State<'_, Engine>) -> IpcResult<usize> {
    Ok(engine.retry_failed()?)
}

#[tauri::command]
pub async fn app_stats(engine: State<'_, Engine>) -> IpcResult<Stats> {
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.stats())
        .await
        .map_err(|e| IpcError {
            code: "runtime",
            message: e.to_string(),
        })?
        .map_err(IpcError::from)
}

/// Remove every indexed folder and all derived data (text, vectors,
/// thumbnails). Models stay installed.
#[tauri::command]
pub async fn index_purge(engine: State<'_, Engine>) -> IpcResult<()> {
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.purge_index())
        .await
        .map_err(|e| IpcError {
            code: "runtime",
            message: e.to_string(),
        })?
        .map_err(IpcError::from)
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
