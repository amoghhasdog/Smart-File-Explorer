//! Local-first embeddings.
//!
//! [`TextEmbedder`] is the only thing the pipeline and search depend on.
//! [`FakeEmbedder`] gives deterministic, hash-based vectors for tests;
//! [`fastembed_impl::FastEmbedText`] (feature `fastembed`, default) runs a
//! real ONNX model. [`models::ModelManager`] downloads models only after the
//! user consents, into the app's own cache directory, and reports progress.

pub mod fake;
#[cfg(feature = "fastembed")]
pub mod fastembed_impl;
pub mod models;

pub use fake::FakeEmbedder;
pub use models::{ModelManager, ModelProgress, ModelStatus, TEXT_MODEL};

#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    #[error("model not installed: {0}")]
    NotInstalled(String),
    #[error("model error: {0}")]
    Model(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, EmbedError>;

/// Produces unit-length vectors for text. Implementations are `Sync`; batch
/// calls are preferred (ONNX amortises per-call overhead).
pub trait TextEmbedder: Send + Sync {
    /// Stable identifier stored with every chunk (`chunks.model`).
    fn model_id(&self) -> &str;
    fn dim(&self) -> usize;
    /// Embed documents for indexing.
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;
    /// Embed a search query (some models want a different prefix).
    fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        let mut v = self.embed(&[text])?;
        Ok(v.pop().unwrap_or_default())
    }
}

/// Cosine similarity of two vectors (not assumed normalised).
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}
