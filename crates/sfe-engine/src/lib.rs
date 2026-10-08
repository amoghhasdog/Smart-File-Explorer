//! Application engine façade.
//!
//! Owns the database (and, in later phases, the indexing pipeline, embedders
//! and search orchestrator) and exposes a small API that the Tauri shell and
//! integration tests call.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sfe_db::Database;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("could not create data directory {0}: {1}")]
    DataDir(PathBuf, std::io::Error),
    #[error(transparent)]
    Db(#[from] sfe_db::DbError),
}

pub type Result<T> = std::result::Result<T, EngineError>;

#[derive(Debug, Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    data_dir: PathBuf,
    db: Database,
}

impl Engine {
    /// Open the engine with all state under `data_dir` (created if missing).
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self> {
        let data_dir = data_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&data_dir)
            .map_err(|e| EngineError::DataDir(data_dir.clone(), e))?;
        let db = Database::open(data_dir.join("index.db"))?;
        db.quick_check()?;
        Ok(Self {
            inner: Arc::new(Inner { data_dir, db }),
        })
    }

    pub fn data_dir(&self) -> &Path {
        &self.inner.data_dir
    }

    pub fn db(&self) -> &Database {
        &self.inner.db
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_creates_data_dir_and_database() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("nested").join("app");
        let engine = Engine::open(&data).unwrap();
        assert!(data.join("index.db").exists());
        assert_eq!(engine.data_dir(), data);
        // Reopen is fine.
        Engine::open(&data).unwrap();
    }
}
