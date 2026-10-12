//! Model lifecycle: status, consented install, loading, and wiring the
//! embedders into the indexer and searcher.

use std::sync::{Arc, RwLock};

use sfe_embed::models::ModelKind;
use sfe_embed::{
    IMAGE_MODEL, ImageEmbedder, ModelManager, ModelProgress, ModelStatus, TEXT_MODEL, TextEmbedder,
};
use sfe_search::Semantic;
use sfe_vector::{Hnsw, VectorIndex};
use tokio::sync::watch;

use crate::{Engine, EngineError, Result};

pub(crate) struct Models {
    pub manager: Arc<ModelManager>,
    pub progress: watch::Sender<ModelProgress>,
    pub text: RwLock<Option<Arc<dyn TextEmbedder>>>,
    pub image: RwLock<Option<Arc<dyn ImageEmbedder>>>,
}

impl Models {
    pub fn new(data_dir: &std::path::Path) -> Self {
        Self {
            manager: Arc::new(ModelManager::new(data_dir)),
            progress: watch::channel(ModelProgress::default()).0,
            text: RwLock::new(None),
            image: RwLock::new(None),
        }
    }
}

impl Engine {
    /// Status of every model this build knows about.
    pub fn models_status(&self) -> Vec<ModelStatus> {
        let m = &self.inner.models;
        vec![
            m.manager
                .status(&TEXT_MODEL, m.text.read().unwrap().is_some()),
            m.manager
                .status(&IMAGE_MODEL, m.image.read().unwrap().is_some()),
        ]
    }

    pub fn subscribe_models(&self) -> watch::Receiver<ModelProgress> {
        self.inner.models.progress.subscribe()
    }

    /// Download (user consented) and activate a model, in the background.
    /// Progress arrives on [`Engine::subscribe_models`].
    pub fn models_install(&self, kind: ModelKind) -> Result<()> {
        if self.models_status().iter().any(|s| s.installing) {
            return Ok(());
        }
        let engine = self.clone();
        self.inner.rt.spawn_blocking(move || {
            let tx = engine.inner.models.progress.clone();
            let mgr = Arc::clone(&engine.inner.models.manager);
            let report = move |p: ModelProgress| {
                let _ = tx.send(p);
            };
            let res = match kind {
                ModelKind::Text => mgr
                    .install_text(&TEXT_MODEL, report)
                    .and_then(|e| engine.activate_embedder(e).map_err(to_embed_err)),
                ModelKind::Image => mgr
                    .install_image(&IMAGE_MODEL, report)
                    .and_then(|e| engine.activate_image_embedder(e).map_err(to_embed_err)),
            };
            if let Err(e) = res {
                tracing::error!(?kind, error = %e, "model install failed");
            }
        });
        Ok(())
    }

    /// Load installed models at startup (offline). No-op for missing ones.
    pub(crate) fn load_installed_models(&self) {
        let mgr = &self.inner.models.manager;
        if mgr.is_installed(&TEXT_MODEL) {
            let engine = self.clone();
            self.inner.rt.spawn_blocking(move || {
                match engine.inner.models.manager.load_text(&TEXT_MODEL) {
                    Ok(e) => {
                        if let Err(err) = engine.activate_embedder(e) {
                            tracing::error!(error = %err, "activating text model");
                        }
                    }
                    Err(e) => tracing::error!(error = %e, "loading installed text model"),
                }
            });
        }
        if mgr.is_installed(&IMAGE_MODEL) {
            let engine = self.clone();
            self.inner.rt.spawn_blocking(move || {
                match engine.inner.models.manager.load_image(&IMAGE_MODEL) {
                    Ok(e) => {
                        if let Err(err) = engine.activate_image_embedder(e) {
                            tracing::error!(error = %err, "activating image model");
                        }
                    }
                    Err(e) => tracing::error!(error = %e, "loading installed image model"),
                }
            });
        }
    }

    /// Wire a text embedder (real or test) into indexing and search.
    pub fn activate_embedder(&self, embedder: Arc<dyn TextEmbedder>) -> Result<()> {
        let index = self.open_vector_index("text", embedder.model_id(), embedder.dim())?;
        *self.inner.models.text.write().unwrap() = Some(Arc::clone(&embedder));
        self.inner.searcher.add_strategy(Box::new(Semantic::text(
            Arc::clone(&embedder),
            Arc::clone(&index),
        )));
        self.inner.indexer.set_embedder(embedder, index);
        tracing::info!("semantic text search active");
        Ok(())
    }

    /// Wire an image embedder (real or test) into indexing and search.
    pub fn activate_image_embedder(&self, embedder: Arc<dyn ImageEmbedder>) -> Result<()> {
        let index = self.open_vector_index("image", embedder.model_id(), embedder.dim())?;
        *self.inner.models.image.write().unwrap() = Some(Arc::clone(&embedder));
        self.inner.searcher.add_strategy(Box::new(Semantic::image(
            Arc::clone(&embedder),
            Arc::clone(&index),
        )));
        self.inner.indexer.set_image_embedder(embedder, index);
        tracing::info!("semantic image search active");
        Ok(())
    }

    /// Open the on-disk ANN index for a model, rebuilding from SQLite when
    /// it lags the database.
    fn open_vector_index(
        &self,
        modality: &str,
        model_id: &str,
        dim: usize,
    ) -> Result<Arc<dyn VectorIndex>> {
        let path = self
            .inner
            .data_dir
            .join("vectors")
            .join(format!("{modality}-{model_id}.usearch"));
        let index: Arc<dyn VectorIndex> = Arc::new(Hnsw::open(&path, dim)?);
        let conn = self.inner.db.reader()?;
        let stored = sfe_db::chunks::count(&conn, model_id, modality)?;
        if (index.len() as u64) < stored {
            tracing::info!(
                modality,
                stored,
                have = index.len(),
                "rebuilding vector index from database"
            );
            let mut batch = Vec::with_capacity(1024);
            sfe_db::chunks::for_each_vector(&conn, model_id, modality, |id, v| {
                batch.push((id.0 as u64, v));
            })?;
            index.add_batch(&batch)?;
            index.save()?;
        }
        Ok(index)
    }
}

fn to_embed_err(e: EngineError) -> sfe_embed::EmbedError {
    sfe_embed::EmbedError::Model(e.to_string())
}

impl From<sfe_embed::EmbedError> for EngineError {
    fn from(e: sfe_embed::EmbedError) -> Self {
        EngineError::Embed(e)
    }
}
