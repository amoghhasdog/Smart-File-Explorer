//! Model lifecycle: status, consented install, loading, and wiring the
//! embedder into the indexer and searcher.

use std::sync::Arc;

use sfe_embed::{ModelManager, ModelProgress, ModelStatus, TEXT_MODEL, TextEmbedder};
use sfe_search::SemanticText;
use sfe_vector::{Hnsw, VectorIndex};
use tokio::sync::watch;

use crate::{Engine, EngineError, Result};

pub(crate) struct Models {
    pub manager: Arc<ModelManager>,
    pub progress: watch::Sender<ModelProgress>,
    pub embedder: std::sync::RwLock<Option<Arc<dyn TextEmbedder>>>,
}

impl Models {
    pub fn new(data_dir: &std::path::Path) -> Self {
        Self {
            manager: Arc::new(ModelManager::new(data_dir)),
            progress: watch::channel(ModelProgress::default()).0,
            embedder: std::sync::RwLock::new(None),
        }
    }
}

impl Engine {
    pub fn models_status(&self) -> ModelStatus {
        let ready = self.inner.models.embedder.read().unwrap().is_some();
        self.inner.models.manager.status(&TEXT_MODEL, ready)
    }

    pub fn subscribe_models(&self) -> watch::Receiver<ModelProgress> {
        self.inner.models.progress.subscribe()
    }

    /// Download (user consented) and activate the text model, in the
    /// background. Progress arrives on [`Engine::subscribe_models`].
    pub fn models_install(&self) -> Result<()> {
        let st = self.models_status();
        if st.installing {
            return Ok(());
        }
        let engine = self.clone();
        self.inner.rt.spawn_blocking(move || {
            let tx = engine.inner.models.progress.clone();
            let mgr = Arc::clone(&engine.inner.models.manager);
            let res = mgr.install_text(&TEXT_MODEL, move |p| {
                let _ = tx.send(p);
            });
            match res {
                Ok(embedder) => {
                    if let Err(e) = engine.activate_embedder(embedder) {
                        tracing::error!(error = %e, "activating model");
                    }
                }
                Err(e) => tracing::error!(error = %e, "model install failed"),
            }
        });
        Ok(())
    }

    /// Load an installed model at startup (offline). Silently no-op if none.
    pub(crate) fn load_installed_models(&self) {
        if !self.inner.models.manager.is_installed(&TEXT_MODEL) {
            return;
        }
        let engine = self.clone();
        self.inner.rt.spawn_blocking(move || {
            match engine.inner.models.manager.load_text(&TEXT_MODEL) {
                Ok(e) => {
                    if let Err(err) = engine.activate_embedder(e) {
                        tracing::error!(error = %err, "activating model");
                    }
                }
                Err(e) => tracing::error!(error = %e, "loading installed model"),
            }
        });
    }

    /// Wire an embedder (real or test) into indexing and search. Opens or
    /// rebuilds the ANN index for that model.
    pub fn activate_embedder(&self, embedder: Arc<dyn TextEmbedder>) -> Result<()> {
        let dim = embedder.dim();
        let path = self
            .inner
            .data_dir
            .join("vectors")
            .join(format!("text-{}.usearch", embedder.model_id()));
        let index: Arc<dyn VectorIndex> = Arc::new(Hnsw::open(&path, dim)?);
        // Rebuild from SQLite if the on-disk index lags the database.
        let stored = sfe_db::chunks::count(&self.inner.db.reader()?, embedder.model_id(), "text")?;
        if (index.len() as u64) < stored {
            tracing::info!(
                stored,
                have = index.len(),
                "rebuilding vector index from database"
            );
            let conn = self.inner.db.reader()?;
            let mut batch = Vec::with_capacity(1024);
            sfe_db::chunks::for_each_vector(&conn, embedder.model_id(), "text", |id, v| {
                batch.push((id.0 as u64, v));
            })?;
            index.add_batch(&batch)?;
            index.save()?;
        }
        *self.inner.models.embedder.write().unwrap() = Some(Arc::clone(&embedder));
        self.inner.searcher.add_strategy(Box::new(SemanticText::new(
            Arc::clone(&embedder),
            Arc::clone(&index),
        )));
        self.inner.indexer.set_embedder(embedder, index);
        tracing::info!("semantic search active");
        Ok(())
    }
}

impl From<sfe_embed::EmbedError> for EngineError {
    fn from(e: sfe_embed::EmbedError) -> Self {
        EngineError::Embed(e)
    }
}
