//! Model installation with explicit consent and progress reporting.
//!
//! Models live under `<data_dir>/models`. Nothing is fetched until an
//! `install_*` call is made; afterwards the app is fully offline
//! (fastembed/hf-hub serve cached files without touching the network).
//! A marker file per model records a completed install.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{EmbedError, ImageEmbedder, Result, TextEmbedder};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelKind {
    Text,
    Image,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ModelSpec {
    pub id: &'static str,
    pub kind: ModelKind,
    pub display: &'static str,
    pub approx_bytes: u64,
    pub dim: usize,
}

/// The text model this build uses.
pub const TEXT_MODEL: ModelSpec = ModelSpec {
    id: "bge-small-en-v1.5-q",
    kind: ModelKind::Text,
    display: "BGE small (text, English)",
    approx_bytes: 35 * 1024 * 1024,
    dim: 384,
};

/// The joint image/text model (CLIP ViT-B/32 vision + text towers).
pub const IMAGE_MODEL: ModelSpec = ModelSpec {
    id: "clip-vit-b-32",
    kind: ModelKind::Image,
    display: "CLIP ViT-B/32 (images)",
    approx_bytes: 600 * 1024 * 1024,
    dim: 512,
};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub id: String,
    pub kind: ModelKind,
    pub display: String,
    pub approx_bytes: u64,
    pub installed: bool,
    /// Loaded into memory and usable for search/indexing.
    pub ready: bool,
    pub installing: bool,
    /// Build has real model support (feature `fastembed`).
    pub available: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProgress {
    pub id: String,
    /// Bytes present in the cache so far (approximate progress).
    pub bytes: u64,
    pub approx_total: u64,
    pub done: bool,
    pub error: Option<String>,
}

pub struct ModelManager {
    dir: PathBuf,
    installing: AtomicBool,
}

impl ModelManager {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            dir: data_dir.join("models"),
            installing: AtomicBool::new(false),
        }
    }

    pub fn cache_dir(&self) -> &Path {
        &self.dir
    }

    pub fn spec(kind: ModelKind) -> &'static ModelSpec {
        match kind {
            ModelKind::Text => &TEXT_MODEL,
            ModelKind::Image => &IMAGE_MODEL,
        }
    }

    fn marker(&self, spec: &ModelSpec) -> PathBuf {
        self.dir.join(format!("{}.installed", spec.id))
    }

    pub fn is_installed(&self, spec: &ModelSpec) -> bool {
        self.marker(spec).exists()
    }

    pub fn status(&self, spec: &ModelSpec, ready: bool) -> ModelStatus {
        ModelStatus {
            id: spec.id.to_string(),
            kind: spec.kind,
            display: spec.display.to_string(),
            approx_bytes: spec.approx_bytes,
            installed: self.is_installed(spec),
            ready,
            installing: self.installing.load(Ordering::SeqCst),
            available: cfg!(feature = "fastembed"),
        }
    }

    /// Bytes currently under the cache dir (for coarse progress).
    pub fn cache_bytes(&self) -> u64 {
        fn walk(p: &Path) -> u64 {
            let Ok(rd) = std::fs::read_dir(p) else {
                return 0;
            };
            rd.flatten()
                .map(|e| match e.metadata() {
                    Ok(m) if m.is_dir() => walk(&e.path()),
                    Ok(m) => m.len(),
                    Err(_) => 0,
                })
                .sum()
        }
        walk(&self.dir)
    }

    /// Download (if needed) and load the text model. Blocks; call from a
    /// background thread. `progress` is invoked periodically while the
    /// download runs and once at the end.
    pub fn install_text(
        self: &Arc<Self>,
        spec: &ModelSpec,
        progress: impl Fn(ModelProgress) + Send + 'static,
    ) -> Result<Arc<dyn TextEmbedder>> {
        self.install(spec, progress, |dir| {
            #[cfg(feature = "fastembed")]
            {
                let m = crate::fastembed_impl::FastEmbedText::load(dir)?;
                Ok(Arc::new(m) as Arc<dyn TextEmbedder>)
            }
            #[cfg(not(feature = "fastembed"))]
            {
                let _ = dir;
                Err(no_support())
            }
        })
    }

    /// Download (if needed) and load the image model.
    pub fn install_image(
        self: &Arc<Self>,
        spec: &ModelSpec,
        progress: impl Fn(ModelProgress) + Send + 'static,
    ) -> Result<Arc<dyn ImageEmbedder>> {
        self.install(spec, progress, |dir| {
            #[cfg(feature = "fastembed")]
            {
                let m = crate::fastembed_impl::FastEmbedClip::load(dir)?;
                Ok(Arc::new(m) as Arc<dyn ImageEmbedder>)
            }
            #[cfg(not(feature = "fastembed"))]
            {
                let _ = dir;
                Err(no_support())
            }
        })
    }

    /// Load an already-installed text model without any network access.
    pub fn load_text(&self, spec: &ModelSpec) -> Result<Arc<dyn TextEmbedder>> {
        if !self.is_installed(spec) {
            return Err(EmbedError::NotInstalled(spec.id.into()));
        }
        #[cfg(feature = "fastembed")]
        {
            let m = crate::fastembed_impl::FastEmbedText::load(&self.dir)?;
            Ok(Arc::new(m))
        }
        #[cfg(not(feature = "fastembed"))]
        {
            Err(no_support())
        }
    }

    pub fn load_image(&self, spec: &ModelSpec) -> Result<Arc<dyn ImageEmbedder>> {
        if !self.is_installed(spec) {
            return Err(EmbedError::NotInstalled(spec.id.into()));
        }
        #[cfg(feature = "fastembed")]
        {
            let m = crate::fastembed_impl::FastEmbedClip::load(&self.dir)?;
            Ok(Arc::new(m))
        }
        #[cfg(not(feature = "fastembed"))]
        {
            Err(no_support())
        }
    }

    fn install<T: Send + 'static>(
        self: &Arc<Self>,
        spec: &ModelSpec,
        progress: impl Fn(ModelProgress) + Send + 'static,
        loader: impl FnOnce(&Path) -> Result<T> + Send,
    ) -> Result<T> {
        if self
            .installing
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(EmbedError::Model("an install is already running".into()));
        }
        let result: Result<T> = (|| {
            std::fs::create_dir_all(&self.dir)?;
            let loaded = self.load_with_progress(spec, &progress, loader)?;
            std::fs::write(self.marker(spec), b"ok")?;
            Ok(loaded)
        })();
        self.installing.store(false, Ordering::SeqCst);
        progress(ModelProgress {
            id: spec.id.into(),
            bytes: self.cache_bytes(),
            approx_total: spec.approx_bytes,
            done: true,
            error: result.as_ref().err().map(|e| e.to_string()),
        });
        result
    }

    /// Run `loader` on a helper thread while polling the cache size so the
    /// UI gets coarse download progress.
    fn load_with_progress<T: Send + 'static>(
        self: &Arc<Self>,
        spec: &ModelSpec,
        progress: &(impl Fn(ModelProgress) + Send + 'static),
        loader: impl FnOnce(&Path) -> Result<T> + Send,
    ) -> Result<T> {
        let dir = self.dir.clone();
        let id = spec.id.to_string();
        let total = spec.approx_bytes;
        std::thread::scope(|s| {
            let h = s.spawn(move || loader(&dir));
            while !h.is_finished() {
                progress(ModelProgress {
                    id: id.clone(),
                    bytes: self.cache_bytes(),
                    approx_total: total,
                    done: false,
                    error: None,
                });
                std::thread::sleep(Duration::from_millis(500));
            }
            h.join()
                .unwrap_or_else(|_| Err(EmbedError::Model("model load panicked".into())))
        })
    }
}

#[cfg(not(feature = "fastembed"))]
fn no_support() -> EmbedError {
    EmbedError::Model("this build has no model support (feature `fastembed` disabled)".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_reflects_marker_file() {
        let dir = tempfile::tempdir().unwrap();
        let m = ModelManager::new(dir.path());
        let st = m.status(&TEXT_MODEL, false);
        assert!(!st.installed);
        assert!(!st.ready);
        assert_eq!(st.id, TEXT_MODEL.id);
        assert_eq!(st.kind, ModelKind::Text);
        assert!(matches!(
            m.load_text(&TEXT_MODEL),
            Err(EmbedError::NotInstalled(_))
        ));
        assert!(matches!(
            m.load_image(&IMAGE_MODEL),
            Err(EmbedError::NotInstalled(_))
        ));

        std::fs::create_dir_all(m.cache_dir()).unwrap();
        std::fs::write(m.cache_dir().join("blob.bin"), vec![0u8; 1000]).unwrap();
        std::fs::write(m.marker(&TEXT_MODEL), "ok").unwrap();
        assert!(m.status(&TEXT_MODEL, true).installed);
        assert!(!m.status(&IMAGE_MODEL, false).installed);
        assert!(m.cache_bytes() >= 1000);
        assert_eq!(ModelManager::spec(ModelKind::Image).id, IMAGE_MODEL.id);
    }
}
