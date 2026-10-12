//! Local-first embeddings.
//!
//! [`TextEmbedder`] and [`ImageEmbedder`] are what the pipeline and search
//! depend on. [`FakeEmbedder`] / [`FakeImageEmbedder`] give deterministic
//! vectors for tests; [`fastembed_impl`] (feature `fastembed`, default) runs
//! real ONNX models. [`models::ModelManager`] downloads models only after the
//! user consents, into the app's own cache directory, and reports progress.

pub mod fake;
#[cfg(feature = "fastembed")]
pub mod fastembed_impl;
pub mod models;

pub use fake::{FakeEmbedder, FakeImageEmbedder};
pub use image::DynamicImage;
pub use models::{IMAGE_MODEL, ModelManager, ModelProgress, ModelSpec, ModelStatus, TEXT_MODEL};

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

/// Joint image/text space (CLIP-style): images are embedded for indexing,
/// natural-language queries are embedded into the same space for search.
pub trait ImageEmbedder: Send + Sync {
    fn model_id(&self) -> &str;
    fn dim(&self) -> usize;
    fn embed_images(&self, images: Vec<DynamicImage>) -> Result<Vec<Vec<f32>>>;
    fn embed_text(&self, query: &str) -> Result<Vec<f32>>;
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
