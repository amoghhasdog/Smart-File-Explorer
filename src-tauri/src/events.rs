//! Events pushed to the webview.

use sfe_engine::{IndexStatus, ModelProgress};
use tauri::{AppHandle, Emitter};
use tokio::sync::watch;

/// Emitted whenever the index status changes.
pub const INDEX_STATUS: &str = "index-status";
/// Emitted while a model downloads and once when it finishes.
pub const MODEL_PROGRESS: &str = "model-progress";

pub fn forward_models(app: AppHandle, mut rx: watch::Receiver<ModelProgress>) {
    tauri::async_runtime::spawn(async move {
        while rx.changed().await.is_ok() {
            let p = rx.borrow_and_update().clone();
            if let Err(e) = app.emit(MODEL_PROGRESS, &p) {
                tracing::warn!(error = %e, "emit model-progress");
            }
        }
    });
}

/// Mirror the engine's status channel onto a Tauri event.
pub fn forward_status(app: AppHandle, mut rx: watch::Receiver<IndexStatus>) {
    tauri::async_runtime::spawn(async move {
        loop {
            let st = rx.borrow_and_update().clone();
            if let Err(e) = app.emit(INDEX_STATUS, &st) {
                tracing::warn!(error = %e, "emit index-status");
            }
            if rx.changed().await.is_err() {
                break;
            }
        }
    });
}
