//! Real embeddings via `fastembed` (ONNX Runtime, CPU).

use std::path::Path;
use std::sync::Mutex;

use fastembed::{
    EmbeddingModel, ImageEmbedding, ImageEmbeddingModel, ImageInitOptions, TextEmbedding,
    TextInitOptions,
};
use image::DynamicImage;

use crate::{EmbedError, ImageEmbedder, Result, TextEmbedder};

/// BGE-small v1.5 (quantised): 384 dims, ~33 MB, good English retrieval
/// quality at CPU-friendly speed.
pub const TEXT_MODEL: EmbeddingModel = EmbeddingModel::BGESmallENV15Q;
pub const TEXT_MODEL_ID: &str = "bge-small-en-v1.5-q";
pub const TEXT_DIM: usize = 384;
/// Recommended query instruction for BGE v1.5 retrieval.
const QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";

/// CLIP ViT-B/32: 512-d joint image/text space.
pub const CLIP_IMAGE_MODEL: ImageEmbeddingModel = ImageEmbeddingModel::ClipVitB32;
pub const CLIP_TEXT_MODEL: EmbeddingModel = EmbeddingModel::ClipVitB32;
pub const CLIP_MODEL_ID: &str = "clip-vit-b-32";
pub const CLIP_DIM: usize = 512;

pub struct FastEmbedText {
    inner: Mutex<TextEmbedding>,
}

impl FastEmbedText {
    /// Load from `cache_dir`. Downloads if the files are missing — callers
    /// gate this behind [`crate::ModelManager`] so it only happens with consent.
    pub fn load(cache_dir: &Path) -> Result<Self> {
        let opts = TextInitOptions::new(TEXT_MODEL)
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
        TEXT_MODEL_ID
    }

    fn dim(&self) -> usize {
        TEXT_DIM
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

/// CLIP vision + text towers.
pub struct FastEmbedClip {
    image: Mutex<ImageEmbedding>,
    text: Mutex<TextEmbedding>,
}

impl FastEmbedClip {
    pub fn load(cache_dir: &Path) -> Result<Self> {
        let image = ImageEmbedding::try_new(
            ImageInitOptions::new(CLIP_IMAGE_MODEL)
                .with_cache_dir(cache_dir.to_path_buf())
                .with_show_download_progress(false),
        )
        .map_err(|e| EmbedError::Model(e.to_string()))?;
        let text = TextEmbedding::try_new(
            TextInitOptions::new(CLIP_TEXT_MODEL)
                .with_cache_dir(cache_dir.to_path_buf())
                .with_show_download_progress(false),
        )
        .map_err(|e| EmbedError::Model(e.to_string()))?;
        Ok(Self {
            image: Mutex::new(image),
            text: Mutex::new(text),
        })
    }
}

impl ImageEmbedder for FastEmbedClip {
    fn model_id(&self) -> &str {
        CLIP_MODEL_ID
    }

    fn dim(&self) -> usize {
        CLIP_DIM
    }

    fn embed_images(&self, images: Vec<DynamicImage>) -> Result<Vec<Vec<f32>>> {
        if images.is_empty() {
            return Ok(vec![]);
        }
        let mut m = self.image.lock().unwrap_or_else(|e| e.into_inner());
        m.embed_images(images)
            .map_err(|e| EmbedError::Model(e.to_string()))
    }

    fn embed_text(&self, query: &str) -> Result<Vec<f32>> {
        let mut m = self.text.lock().unwrap_or_else(|e| e.into_inner());
        let mut v = m
            .embed([query], None)
            .map_err(|e| EmbedError::Model(e.to_string()))?;
        Ok(v.pop().unwrap_or_default())
    }
}
