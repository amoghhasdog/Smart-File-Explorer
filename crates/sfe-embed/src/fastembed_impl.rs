//! Real text embeddings via `fastembed` (ONNX Runtime, CPU).

use std::path::Path;
use std::sync::Mutex;

use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};

use crate::{EmbedError, Result, TextEmbedder};

/// BGE-small v1.5 (quantised): 384 dims, ~33 MB, good English retrieval
/// quality at CPU-friendly speed.
pub const MODEL: EmbeddingModel = EmbeddingModel::BGESmallENV15Q;
pub const MODEL_ID: &str = "bge-small-en-v1.5-q";
pub const DIM: usize = 384;
/// Recommended query instruction for BGE v1.5 retrieval.
const QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";

pub struct FastEmbedText {
    inner: Mutex<TextEmbedding>,
}

impl FastEmbedText {
    /// Load from `cache_dir`. Downloads if the files are missing — callers
    /// gate this behind [`crate::ModelManager`] so it only happens with consent.
    pub fn load(cache_dir: &Path) -> Result<Self> {
        let opts = TextInitOptions::new(MODEL)
            .with_cache_dir(cache_dir.to_path_buf())
            .with_show_download_progress(false);
        let model = TextEmbedding::try_new(opts).map_err(|e| EmbedError::Model(e.to_string()))?;
        Ok(Self {
            inner: Mutex::new(model),
        })
    }
}

impl TextEmbedder for FastEmbedText {
    fn model_id(&self) -> &str {
        MODEL_ID
    }

    fn dim(&self) -> usize {
        DIM
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }
        let mut m = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        m.embed(texts, Some(32))
            .map_err(|e| EmbedError::Model(e.to_string()))
    }

    fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        let q = format!("{QUERY_PREFIX}{text}");
        let mut v = self.embed(&[q.as_str()])?;
        Ok(v.pop().unwrap_or_default())
    }
}
