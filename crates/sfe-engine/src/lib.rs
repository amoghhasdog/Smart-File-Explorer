//! Application engine façade.
//!
//! Owns the database, the indexing pipeline, the filesystem watcher and its
//! own small tokio runtime, so background work never competes with the UI
//! runtime. The Tauri shell (and integration tests) talk only to [`Engine`].

mod models;
mod search;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sfe_core::{FileId, FileRecord, Root, RootId};
use sfe_db::{Database, files, fts, roots};
use sfe_fs::{FsChange, Watcher};
pub use sfe_index::IndexStatus;
use sfe_index::{Indexer, IndexerConfig};
use sfe_search::Searcher;
use tokio::sync::watch;

pub use search::{FileDetails, SearchHit};
pub use sfe_embed::{ModelProgress, ModelStatus, TextEmbedder};
pub use sfe_query::Query;
pub use sfe_search::SearchResponse;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("could not create data directory {0}: {1}")]
    DataDir(PathBuf, std::io::Error),
    #[error(transparent)]
    Db(#[from] sfe_db::DbError),
    #[error(transparent)]
    Fs(#[from] sfe_fs::FsError),
    #[error(transparent)]
    Index(#[from] sfe_index::IndexError),
    #[error(transparent)]
    Search(#[from] sfe_search::SearchError),
    #[error(transparent)]
    Embed(sfe_embed::EmbedError),
    #[error(transparent)]
    Vector(#[from] sfe_vector::VectorError),
    #[error("could not start background runtime: {0}")]
    Runtime(std::io::Error),
    #[error("{0} is already covered by indexed folder {1}")]
    NestedRoot(String, String),
    #[error("no such folder id {0}")]
    NoSuchRoot(RootId),
}

pub type Result<T> = std::result::Result<T, EngineError>;

#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

struct Inner {
    data_dir: PathBuf,
    db: Arc<Database>,
    indexer: Arc<Indexer>,
    searcher: Searcher,
    models: models::Models,
    rt: tokio::runtime::Runtime,
    watcher: Mutex<Watcher>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("data_dir", &self.inner.data_dir)
            .finish()
    }
}

impl Engine {
    /// Open the engine with all state under `data_dir` (created if missing),
    /// start the stage workers, watch every enabled root and kick off an
    /// incremental scan of each.
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(data_dir, IndexerConfig::default())
    }

    pub fn open_with(data_dir: impl AsRef<Path>, cfg: IndexerConfig) -> Result<Self> {
        let data_dir = data_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&data_dir)
            .map_err(|e| EngineError::DataDir(data_dir.clone(), e))?;
        let db = Arc::new(Database::open(data_dir.join("index.db"))?);
        db.quick_check()?;
        // Leases from a previous (crashed) run are stale.
        db.transaction(|c| sfe_db::jobs::release_all(c).map(|_| ()))?;

        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("sfe-engine")
            .enable_all()
            .build()
            .map_err(EngineError::Runtime)?;
        let indexer = Indexer::new(Arc::clone(&db), cfg);
        let searcher = Searcher::new(Arc::clone(&db));
        let models = models::Models::new(&data_dir);
        let (watcher, changes) = Watcher::new(Duration::from_secs(2))?;

        let engine = Self {
            inner: Arc::new(Inner {
                data_dir,
                db,
                indexer,
                searcher,
                models,
                rt,
                watcher: Mutex::new(watcher),
            }),
        };
        engine.inner.rt.block_on(async {
            engine.inner.indexer.spawn_workers();
        });
        engine.spawn_change_pump(changes);
        engine.load_installed_models();
        for root in engine.list_roots()? {
            if root.enabled {
                engine.watch_root(&root);
                engine.spawn_scan(root);
            }
        }
        Ok(engine)
    }

    pub fn data_dir(&self) -> &Path {
        &self.inner.data_dir
    }

    pub fn db(&self) -> &Database {
        &self.inner.db
    }

    // ----- roots -----------------------------------------------------------

    pub fn list_roots(&self) -> Result<Vec<Root>> {
        Ok(roots::list(&self.inner.db.writer())?)
    }

    /// Register a folder, start watching it and scan it in the background.
    /// Rejects folders nested inside (or containing) an existing root.
    pub fn add_root(&self, path: impl AsRef<Path>) -> Result<Root> {
        let canon = sfe_fs::canonical_root(path)?;
        let existing = self.list_roots()?;
        for r in &existing {
            if r.path == canon {
                return Ok(r.clone());
            }
            if is_under(&canon, &r.path) {
                return Err(EngineError::NestedRoot(canon, r.path.clone()));
            }
            if is_under(&r.path, &canon) {
                return Err(EngineError::NestedRoot(r.path.clone(), canon));
            }
        }
        let root = self.inner.db.transaction(|c| roots::add(c, &canon))?;
        tracing::info!(path = %root.path, id = %root.id, "root added");
        self.watch_root(&root);
        self.spawn_scan(root.clone());
        Ok(root)
    }

    /// Stop watching and delete the root with everything indexed under it.
    pub fn remove_root(&self, id: RootId) -> Result<()> {
        let root = roots::get(&self.inner.db.writer(), id)?.ok_or(EngineError::NoSuchRoot(id))?;
        if let Ok(mut w) = self.inner.watcher.lock() {
            let _ = w.unwatch(Path::new(&root.path));
        }
        self.inner.db.transaction(|c| {
            fts::delete_for_root(c, id)?;
            roots::remove(c, id)?;
            Ok(())
        })?;
        self.inner.indexer.refresh_status();
        tracing::info!(path = %root.path, "root removed");
        Ok(())
    }

    pub fn rescan(&self, id: RootId) -> Result<()> {
        let root = roots::get(&self.inner.db.writer(), id)?.ok_or(EngineError::NoSuchRoot(id))?;
        self.spawn_scan(root);
        Ok(())
    }

    // ----- indexing --------------------------------------------------------

    pub fn status(&self) -> IndexStatus {
        self.inner.indexer.status()
    }

    pub fn subscribe_status(&self) -> watch::Receiver<IndexStatus> {
        self.inner.indexer.subscribe()
    }

    pub fn pause(&self) {
        self.inner.indexer.pause();
    }

    pub fn resume(&self) {
        self.inner.indexer.resume();
    }

    /// Block until no scan is running and no jobs are pending (tests / CLI).
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let mut rx = self.subscribe_status();
        self.inner.rt.block_on(async {
            tokio::time::timeout(timeout, async {
                loop {
                    if !rx.borrow_and_update().busy() {
                        return;
                    }
                    if rx.changed().await.is_err() {
                        return;
                    }
                }
            })
            .await
            .is_ok()
        })
    }

    // ----- files & search --------------------------------------------------

    pub fn file(&self, id: FileId) -> Result<Option<FileRecord>> {
        Ok(files::get(&self.inner.db.reader()?, id)?)
    }

    /// Natural-language search: parse, run strategies, fuse, page.
    pub fn search(&self, input: &str, limit: usize, offset: usize) -> Result<SearchResponse> {
        search::search(&self.inner.searcher, input, limit, offset)
    }

    /// How a query would be interpreted (for live chips while typing).
    pub fn explain(&self, input: &str) -> Query {
        search::explain(input)
    }

    pub fn file_details(&self, id: FileId) -> Result<Option<FileDetails>> {
        search::details(&self.inner.db, id)
    }

    // ----- internals -------------------------------------------------------

    fn watch_root(&self, root: &Root) {
        match self.inner.watcher.lock() {
            Ok(mut w) => {
                if let Err(e) = w.watch(Path::new(&root.path)) {
                    tracing::warn!(path = %root.path, error = %e, "could not watch root");
                }
            }
            Err(_) => tracing::error!("watcher mutex poisoned"),
        }
    }

    fn spawn_scan(&self, root: Root) {
        self.inner
            .indexer
            .request_scan(self.inner.rt.handle(), root);
    }

    /// Forward watcher batches to the indexer, grouped by root.
    fn spawn_change_pump(&self, changes: std::sync::mpsc::Receiver<FsChange>) {
        let weak = Arc::downgrade(&self.inner);
        std::thread::Builder::new()
            .name("sfe-fs-changes".into())
            .spawn(move || {
                while let Ok(change) = changes.recv() {
                    let Some(inner) = weak.upgrade() else { break };
                    let roots = match roots::list(&inner.db.writer()) {
                        Ok(r) => r,
                        Err(e) => {
                            tracing::error!(error = %e, "listing roots for change pump");
                            continue;
                        }
                    };
                    if change.rescan {
                        for root in roots {
                            inner.indexer.request_scan(inner.rt.handle(), root);
                        }
                        continue;
                    }
                    for root in roots {
                        let mine: Vec<PathBuf> = change
                            .paths
                            .iter()
                            .filter(|p| p.starts_with(&root.path))
                            .cloned()
                            .collect();
                        if mine.is_empty() {
                            continue;
                        }
                        let indexer = Arc::clone(&inner.indexer);
                        inner.rt.spawn(async move {
                            if let Err(e) = indexer.reconcile(&root, mine).await {
                                tracing::warn!(error = %e, "reconcile failed");
                            }
                        });
                    }
                }
                tracing::debug!("change pump stopped");
            })
            .expect("spawn change pump thread");
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.indexer.shutdown();
        self.indexer.save_vectors();
    }
}

/// Is `path` strictly inside `dir`? Compares on separator boundaries,
/// case-insensitively (NTFS).
fn is_under(path: &str, dir: &str) -> bool {
    let (p, d) = (path.to_lowercase(), dir.to_lowercase());
    p.len() > d.len() && p.starts_with(&d) && matches!(p.as_bytes()[d.len()], b'\\' | b'/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_under_checks_boundaries() {
        assert!(is_under(r"C:\a\b", r"C:\a"));
        assert!(is_under(r"C:\A\b\c", r"c:\a"));
        assert!(!is_under(r"C:\ab", r"C:\a"));
        assert!(!is_under(r"C:\a", r"C:\a"));
        assert!(!is_under(r"C:\a", r"C:\a\b"));
    }

    #[test]
    fn open_creates_data_dir_and_database() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("nested").join("app");
        let engine = Engine::open(&data).unwrap();
        assert!(data.join("index.db").exists());
        assert_eq!(engine.data_dir(), data);
        drop(engine);
        Engine::open(&data).unwrap();
    }

    #[test]
    fn add_root_scans_and_name_search_works_end_to_end() {
        let data = tempfile::tempdir().unwrap();
        let tree = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tree.path().join("projects/athena")).unwrap();
        std::fs::write(tree.path().join("projects/athena/Athena roadmap.md"), "x").unwrap();
        std::fs::write(tree.path().join("holiday.bin"), b"\0\x01\x02 not text").unwrap();

        let engine = Engine::open(data.path()).unwrap();
        let root = engine.add_root(tree.path()).unwrap();
        assert!(engine.wait_idle(Duration::from_secs(15)));

        let st = engine.status();
        assert_eq!(st.total_files, 2);
        assert_eq!(st.metadata, 1, "binary stops at metadata; {st:?}");
        assert_eq!(st.text, 1, "markdown reaches the text stage; {st:?}");

        let hits = engine.search("athena road", 10, 0).unwrap().hits;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "Athena roadmap.md");
        assert_eq!(hits[0].category, sfe_core::Category::Document);

        // nested root rejected, duplicate returns same root
        assert!(matches!(
            engine.add_root(tree.path().join("projects")),
            Err(EngineError::NestedRoot(..))
        ));
        assert_eq!(engine.add_root(tree.path()).unwrap().id, root.id);

        engine.remove_root(root.id).unwrap();
        assert!(engine.list_roots().unwrap().is_empty());
        assert!(engine.search("athena", 10, 0).unwrap().hits.is_empty());
        assert!(matches!(
            engine.remove_root(root.id),
            Err(EngineError::NoSuchRoot(_))
        ));
    }

    #[test]
    fn watcher_picks_up_new_files() {
        let data = tempfile::tempdir().unwrap();
        let tree = tempfile::tempdir().unwrap();
        let engine = Engine::open(data.path()).unwrap();
        engine.add_root(tree.path()).unwrap();
        assert!(engine.wait_idle(Duration::from_secs(15)));
        std::thread::sleep(Duration::from_millis(300));

        std::fs::write(tree.path().join("Ferrari spec.txt"), "v12").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            if !engine.search("ferrari", 10, 0).unwrap().hits.is_empty() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "watcher never indexed the file"
            );
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    #[test]
    fn semantic_search_finds_related_files_by_meaning() {
        let data = tempfile::tempdir().unwrap();
        let tree = tempfile::tempdir().unwrap();
        std::fs::write(
            tree.path().join("planning.md"),
            "Athena project roadmap: milestones for the athena launch and planning.",
        )
        .unwrap();
        std::fs::write(
            tree.path().join("bread.txt"),
            "Recipe for banana bread with walnuts.",
        )
        .unwrap();

        let engine = Engine::open(data.path()).unwrap();
        engine
            .activate_embedder(Arc::new(sfe_embed::FakeEmbedder::default()))
            .unwrap();
        engine.add_root(tree.path()).unwrap();
        assert!(engine.wait_idle(Duration::from_secs(20)));
        let st = engine.status();
        assert_eq!(st.embedded, 2, "{st:?}");

        // No keyword overlap with "planning.md"'s *name*; meaning (shared words in
        // the body) ranks it first via the semantic strategy.
        let r = engine.search("about the athena launch", 10, 0).unwrap();
        assert!(!r.hits.is_empty());
        assert_eq!(r.hits[0].name, "planning.md");

        // Index persists and reloads: a second engine over the same data dir
        // rebuilds nothing and still searches.
        drop(engine);
        let engine = Engine::open(data.path()).unwrap();
        engine
            .activate_embedder(Arc::new(sfe_embed::FakeEmbedder::default()))
            .unwrap();
        let r = engine.search("athena launch planning", 10, 0).unwrap();
        assert_eq!(r.hits[0].name, "planning.md");
        assert!(!engine.models_status().installed);
    }
}
