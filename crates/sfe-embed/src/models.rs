//! Model installation with explicit consent and progress reporting.
//!
//! Models live under `<data_dir>/models`. Nothing is fetched until
//! [`ModelManager::install`] is called; afterwards the app is fully offline
//! (fastembed/hf-hub serve cached files without touching the network).
//! A marker file records a completed install so status checks are cheap.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;

use crate::{EmbedError, Result, TextEmbedder};

/// The text model this build uses.
pub const TEXT_MODEL: ModelSpec = ModelSpec {
    id: "bge-small-en-v1.5-q",
    display: "BGE small (English, quantised)",
    approx_bytes: 35 * 1024 * 1024,
    dim: 384,
};

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ModelSpec {
    pub id: &'static str,
    pub display: &'static str,
    pub approx_bytes: u64,
    pub dim: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub id: String,
    pub display: String,
    pub approx_bytes: u64,
    pub installed: bool,
    /// Loaded into memory and usable for search/indexing.
    pub ready: bool,
    pub installing: bool,
    /// Build has no real model support (feature off) — only the fake embedder.
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

    fn marker(&self, spec: &ModelSpec) -> PathBuf {
        self.dir.join(format!("{}.installed", spec.id))
    }

    pub fn is_installed(&self, spec: &ModelSpec) -> bool {
        self.marker(spec).exists()
    }

    pub fn status(&self, spec: &ModelSpec, ready: bool) -> ModelStatus {
        ModelStatus {
            id: spec.id.to_string(),
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
        if self
            .installing
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(EmbedError::Model("an install is already running".into()));
        }
        std::fs::create_dir_all(&self.dir)?;
        let result = self.install_inner(spec, &progress);
        self.installing.store(false, Ordering::SeqCst);
        match &result {
            Ok(_) => progress(ModelProgress {
                id: spec.id.into(),
                bytes: self.cache_bytes(),
                approx_total: spec.approx_bytes,
                done: true,
                error: None,
            }),
            Err(e) => progress(ModelProgress {
                id: spec.id.into(),
                bytes: self.cache_bytes(),
                approx_total: spec.approx_bytes,
                done: true,
                error: Some(e.to_string()),
            }),
        }
        result
    }

    #[cfg(feature = "fastembed")]
    fn install_inner(
        self: &Arc<Self>,
        spec: &ModelSpec,
        progress: &(impl Fn(ModelProgress) + Send + 'static),
    ) -> Result<Arc<dyn TextEmbedder>> {
        let stop = Arc::new(AtomicBool::new(false));
        let poller = {
            let mgr = Arc::clone(self);
            let stop = Arc::clone(&stop);
            let id = spec.id.to_string();
            let total = spec.approx_bytes;
            let (tx, rx) = std::sync::mpsc::channel::<ModelProgress>();
            let handle = std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    let _ = tx.send(ModelProgress {
                        id: id.clone(),
                        bytes: mgr.cache_bytes(),
                        approx_total: total,
                        done: false,
                        error: None,
                    });
                    std::thread::sleep(Duration::from_millis(500));
                }
            });
            (handle, rx)
        };
        let dir = self.dir.clone();
        let loaded = std::thread::scope(|s| {
            let h = s.spawn(|| crate::fastembed_impl::FastEmbedText::load(&dir));
            // Forward poller progress while the load runs.
            while !h.is_finished() {
                if let Ok(p) = poller.1.recv_timeout(Duration::from_millis(600)) {
                    progress(p);
                }
            }
            h.join()
                .unwrap_or_else(|_| Err(EmbedError::Model("model load panicked".into())))
        });
        stop.store(true, Ordering::SeqCst);
        let _ = poller.0.join();
        let model = loaded?;
        std::fs::write(self.marker(spec), b"ok")?;
        Ok(Arc::new(model))
    }

    #[cfg(not(feature = "fastembed"))]
    fn install_inner(
        self: &Arc<Self>,
        _spec: &ModelSpec,
        _progress: &(impl Fn(ModelProgress) + Send + 'static),
    ) -> Result<Arc<dyn TextEmbedder>> {
        Err(EmbedError::Model(
            "this build has no model support (feature `fastembed` disabled)".into(),
        ))
    }

    /// Load an already-installed model without any network access.
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
            Err(EmbedError::Model("no model support in this build".into()))
        }
    }
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
        assert!(matches!(
            m.load_text(&TEXT_MODEL),
            Err(EmbedError::NotInstalled(_))
        ));

        std::fs::create_dir_all(m.cache_dir()).unwrap();
        std::fs::write(m.cache_dir().join("blob.bin"), vec![0u8; 1000]).unwrap();
        std::fs::write(m.marker(&TEXT_MODEL), "ok").unwrap();
        assert!(m.status(&TEXT_MODEL, true).installed);
        assert!(m.cache_bytes() >= 1000);
    }
}
