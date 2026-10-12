//! Approximate nearest-neighbour vector index.
//!
//! SQLite (`chunks.vec`) is the source of truth; the HNSW index here is a
//! rebuildable, persisted cache keyed by `chunks.id`. [`Hnsw`] wraps
//! `usearch` (cosine, f16 storage); [`Flat`] is an exact in-memory index for
//! tests and tiny corpora.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

#[derive(Debug, thiserror::Error)]
pub enum VectorError {
    #[error("vector index: {0}")]
    Index(String),
    #[error("dimension mismatch: index has {expected}, got {got}")]
    Dim { expected: usize, got: usize },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, VectorError>;

/// A nearest neighbour hit: key and cosine similarity (1 = identical).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Neighbor {
    pub key: u64,
    pub score: f32,
}

pub trait VectorIndex: Send + Sync {
    fn dim(&self) -> usize;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn add(&self, key: u64, vector: &[f32]) -> Result<()>;
    fn add_batch(&self, items: &[(u64, Vec<f32>)]) -> Result<()> {
        for (k, v) in items {
            self.add(*k, v)?;
        }
        Ok(())
    }
    fn remove(&self, key: u64) -> Result<bool>;
    fn contains(&self, key: u64) -> bool;
    /// Top-`k` by cosine similarity, best first.
    fn search(&self, query: &[f32], k: usize) -> Result<Vec<Neighbor>>;
    /// Persist to disk (no-op for in-memory indexes).
    fn save(&self) -> Result<()>;
}

// ---------------------------------------------------------------------------

/// `usearch` HNSW index persisted at `path`.
pub struct Hnsw {
    index: RwLock<usearch::Index>,
    path: PathBuf,
    dim: usize,
}

impl Hnsw {
    /// Open the index at `path` (loaded if the file exists, else empty).
    pub fn open(path: impl AsRef<Path>, dim: usize) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let opts = usearch::IndexOptions {
            dimensions: dim,
            metric: usearch::MetricKind::Cos,
            quantization: usearch::ScalarKind::F16,
            connectivity: 0,
            expansion_add: 0,
            expansion_search: 0,
            multi: false,
        };
        let index = usearch::Index::new(&opts).map_err(|e| VectorError::Index(e.to_string()))?;
        if path.exists() {
            match index.load(path.to_str().unwrap_or_default()) {
                Ok(()) => {
                    tracing::info!(path = %path.display(), n = index.size(), "vector index loaded")
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "vector index unreadable; starting empty");
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        if index.dimensions() != dim {
            return Err(VectorError::Dim {
                expected: dim,
                got: index.dimensions(),
            });
        }
        index
            .reserve(index.size().max(1024))
            .map_err(|e| VectorError::Index(e.to_string()))?;
        Ok(Self {
            index: RwLock::new(index),
            path,
            dim,
        })
    }

    fn check(&self, v: &[f32]) -> Result<()> {
        if v.len() != self.dim {
            return Err(VectorError::Dim {
                expected: self.dim,
                got: v.len(),
            });
        }
        Ok(())
    }
}

// `usearch` mutates through `&self`; the write lock exists to serialise
// `reserve` with concurrent adds/searches, so clippy's "only read" is moot.
#[allow(clippy::readonly_write_lock)]
impl VectorIndex for Hnsw {
    fn dim(&self) -> usize {
        self.dim
    }

    fn len(&self) -> usize {
        self.index.read().unwrap().size()
    }

    fn add(&self, key: u64, vector: &[f32]) -> Result<()> {
        self.check(vector)?;
        let idx = self.index.write().unwrap();
        if idx.size() + 1 > idx.capacity() {
            idx.reserve((idx.capacity() * 2).max(1024))
                .map_err(|e| VectorError::Index(e.to_string()))?;
        }
        if idx.contains(key) {
            let _ = idx.remove(key);
        }
        idx.add(key, vector)
            .map_err(|e| VectorError::Index(e.to_string()))
    }

    fn add_batch(&self, items: &[(u64, Vec<f32>)]) -> Result<()> {
        let idx = self.index.write().unwrap();
        let need = idx.size() + items.len();
        if need > idx.capacity() {
            idx.reserve(need.max(idx.capacity() * 2).max(1024))
                .map_err(|e| VectorError::Index(e.to_string()))?;
        }
        for (k, v) in items {
            if v.len() != self.dim {
                return Err(VectorError::Dim {
                    expected: self.dim,
                    got: v.len(),
                });
            }
            if idx.contains(*k) {
                let _ = idx.remove(*k);
            }
            idx.add(*k, v)
                .map_err(|e| VectorError::Index(e.to_string()))?;
        }
        Ok(())
    }

    fn remove(&self, key: u64) -> Result<bool> {
        let idx = self.index.write().unwrap();
        idx.remove(key)
            .map(|n| n > 0)
            .map_err(|e| VectorError::Index(e.to_string()))
    }

    fn contains(&self, key: u64) -> bool {
        self.index.read().unwrap().contains(key)
    }

    fn search(&self, query: &[f32], k: usize) -> Result<Vec<Neighbor>> {
        self.check(query)?;
        let idx = self.index.read().unwrap();
        if idx.size() == 0 || k == 0 {
            return Ok(vec![]);
        }
        let m = idx
            .search(query, k)
            .map_err(|e| VectorError::Index(e.to_string()))?;
        Ok(m.keys
            .into_iter()
            .zip(m.distances)
            .map(|(key, d)| Neighbor {
                key,
                score: 1.0 - d,
            })
            .collect())
    }

    fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let idx = self.index.read().unwrap();
        idx.save(self.path.to_str().unwrap_or_default())
            .map_err(|e| VectorError::Index(e.to_string()))
    }
}

// ---------------------------------------------------------------------------

/// Exact brute-force index (tests / fallback).
#[derive(Default)]
pub struct Flat {
    dim: usize,
    items: RwLock<Vec<(u64, Vec<f32>)>>,
}

impl Flat {
    pub fn new(dim: usize) -> Self {
        Self {
            dim,
            items: RwLock::new(Vec::new()),
        }
    }
}

impl VectorIndex for Flat {
    fn dim(&self) -> usize {
        self.dim
    }

    fn len(&self) -> usize {
        self.items.read().unwrap().len()
    }

    fn add(&self, key: u64, vector: &[f32]) -> Result<()> {
        if vector.len() != self.dim {
            return Err(VectorError::Dim {
                expected: self.dim,
                got: vector.len(),
            });
        }
        let mut items = self.items.write().unwrap();
        items.retain(|(k, _)| *k != key);
        items.push((key, vector.to_vec()));
        Ok(())
    }

    fn remove(&self, key: u64) -> Result<bool> {
        let mut items = self.items.write().unwrap();
        let before = items.len();
        items.retain(|(k, _)| *k != key);
        Ok(items.len() != before)
    }

    fn contains(&self, key: u64) -> bool {
        self.items.read().unwrap().iter().any(|(k, _)| *k == key)
    }

    fn search(&self, query: &[f32], k: usize) -> Result<Vec<Neighbor>> {
        let items = self.items.read().unwrap();
        let mut out: Vec<Neighbor> = items
            .iter()
            .map(|(key, v)| Neighbor {
                key: *key,
                score: cosine(query, v),
            })
            .collect();
        out.sort_by(|a, b| b.score.total_cmp(&a.score));
        out.truncate(k);
        Ok(out)
    }

    fn save(&self) -> Result<()> {
        Ok(())
    }
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(i: usize, dim: usize) -> Vec<f32> {
        let mut v = vec![0.0; dim];
        v[i] = 1.0;
        v
    }

    fn exercise(idx: &dyn VectorIndex) {
        let d = idx.dim();
        idx.add(1, &unit(0, d)).unwrap();
        idx.add(2, &unit(1, d)).unwrap();
        let mut mixed = unit(0, d);
        mixed[1] = 0.5;
        idx.add(3, &mixed).unwrap();
        assert_eq!(idx.len(), 3);
        assert!(idx.contains(2));

        let hits = idx.search(&unit(0, d), 2).unwrap();
        assert_eq!(hits[0].key, 1);
        assert!((hits[0].score - 1.0).abs() < 0.01, "{hits:?}");
        assert_eq!(hits[1].key, 3);

        assert!(idx.remove(1).unwrap());
        assert!(!idx.remove(1).unwrap());
        assert_eq!(idx.search(&unit(0, d), 1).unwrap()[0].key, 3);

        // re-adding an existing key replaces it
        idx.add(3, &unit(1, d)).unwrap();
        assert_eq!(idx.len(), 2);
        assert!(matches!(idx.add(9, &[1.0]), Err(VectorError::Dim { .. })));
    }

    #[test]
    fn flat_index() {
        exercise(&Flat::new(8));
    }

    #[test]
    fn hnsw_index_persists() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("v").join("t.usearch");
        {
            let idx = Hnsw::open(&p, 8).unwrap();
            exercise(&idx);
            idx.save().unwrap();
        }
        let idx = Hnsw::open(&p, 8).unwrap();
        assert_eq!(idx.len(), 2);
        assert!(idx.contains(2));
        assert!(matches!(Hnsw::open(&p, 16), Err(VectorError::Dim { .. })));
    }

    #[test]
    fn hnsw_batch_add_grows_capacity() {
        let dir = tempfile::tempdir().unwrap();
        let idx = Hnsw::open(dir.path().join("b.usearch"), 4).unwrap();
        let items: Vec<(u64, Vec<f32>)> = (0..3000u64)
            .map(|i| (i, vec![(i % 7) as f32, 1.0, (i % 3) as f32, 0.5]))
            .collect();
        idx.add_batch(&items).unwrap();
        assert_eq!(idx.len(), 3000);
        assert_eq!(idx.search(&[6.0, 1.0, 2.0, 0.5], 1).unwrap().len(), 1);
    }
}
