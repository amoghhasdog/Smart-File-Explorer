//! Housekeeping: storage statistics, failed-file list, purge.

use std::path::Path;

use serde::Serialize;
use sfe_core::FileId;
use sfe_db::{chunks, files};

use crate::{Engine, Result};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub version: String,
    pub data_dir: String,
    pub db_bytes: u64,
    pub vectors_bytes: u64,
    pub thumbs_bytes: u64,
    pub models_bytes: u64,
    pub files: u64,
    pub text_chunks: u64,
    pub image_chunks: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    pub id: FileId,
    pub path: String,
    pub name: String,
    pub error: String,
    pub at: Option<i64>,
}

fn dir_bytes(p: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(p) else {
        return 0;
    };
    rd.flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_bytes(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

fn file_bytes(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

impl Engine {
    pub fn stats(&self) -> Result<Stats> {
        let d = &self.inner.data_dir;
        let db_path = self.inner.db.path().to_path_buf();
        let conn = self.inner.db.reader()?;
        let (text_chunks, image_chunks) = chunks::count_all(&conn)?;
        Ok(Stats {
            version: env!("CARGO_PKG_VERSION").to_string(),
            data_dir: d.display().to_string(),
            db_bytes: file_bytes(&db_path)
                + file_bytes(&db_path.with_extension("db-wal"))
                + file_bytes(&db_path.with_extension("db-shm")),
            vectors_bytes: dir_bytes(&d.join("vectors")),
            thumbs_bytes: dir_bytes(&d.join("thumbs")),
            models_bytes: dir_bytes(&d.join("models")),
            files: files::count_live(&conn)?,
            text_chunks,
            image_chunks,
        })
    }

    pub fn problems(&self, limit: usize) -> Result<Vec<Problem>> {
        let conn = self.inner.db.reader()?;
        Ok(files::list_failed(&conn, limit)?
            .into_iter()
            .map(|r| Problem {
                id: r.id,
                path: r.path,
                name: r.name,
                error: r.index_error.unwrap_or_default(),
                at: r.indexed_at,
            })
            .collect())
    }

    pub fn retry_failed(&self) -> Result<usize> {
        Ok(self.inner.indexer.retry_failed()?)
    }

    /// Remove every root and all derived data; installed models are kept.
    pub fn purge_index(&self) -> Result<()> {
        for root in self.list_roots()? {
            self.remove_root(root.id)?;
        }
        self.inner.indexer.gc()?;
        self.inner.indexer.clear_vectors();
        let _ = std::fs::remove_dir_all(self.inner.data_dir.join("thumbs"));
        {
            let conn = self.inner.db.writer();
            conn.execute_batch("DELETE FROM chunks; DELETE FROM jobs; VACUUM;")
                .map_err(sfe_db::DbError::from)?;
        }
        self.inner.indexer.refresh_status();
        tracing::info!("index purged");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use crate::Engine;

    #[test]
    fn stats_problems_retry_and_purge() {
        let data = tempfile::tempdir().unwrap();
        let tree = tempfile::tempdir().unwrap();
        std::fs::write(tree.path().join("ok.md"), "fine text").unwrap();
        std::fs::write(tree.path().join("bad.pdf"), "%PDF-1.4 definitely not a pdf").unwrap();

        let engine = Engine::open(data.path()).unwrap();
        engine
            .activate_embedder(Arc::new(sfe_embed::FakeEmbedder::default()))
            .unwrap();
        let root = engine.add_root(tree.path()).unwrap();
        assert!(engine.wait_idle(Duration::from_secs(20)));

        let st = engine.stats().unwrap();
        assert_eq!(st.files, 2);
        assert!(st.db_bytes > 0);
        assert_eq!(st.text_chunks, 1);

        let problems = engine.problems(10).unwrap();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert_eq!(problems[0].name, "bad.pdf");
        assert!(
            problems[0].error.starts_with("pdf:"),
            "{}",
            problems[0].error
        );

        assert_eq!(engine.retry_failed().unwrap(), 1);
        assert!(engine.wait_idle(Duration::from_secs(20)));
        assert_eq!(
            engine.problems(10).unwrap().len(),
            1,
            "still broken after retry"
        );

        engine.purge_index().unwrap();
        assert!(engine.list_roots().unwrap().is_empty());
        let st = engine.stats().unwrap();
        assert_eq!(st.files, 0);
        assert_eq!(st.text_chunks, 0);
        assert!(engine.search("fine", 10, 0).unwrap().hits.is_empty());
        let _ = root;
    }
}
