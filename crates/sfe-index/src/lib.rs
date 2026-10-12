//! Indexing pipeline.
//!
//! A durable job queue (in SQLite) drives files through cheap stages first
//! across the whole corpus before expensive ones, so name/metadata search is
//! available within minutes even for very large trees.
//!
//! * [`Indexer::scan`] — the *discover* stage: walk a directory, upsert rows
//!   in batches, enqueue `Metadata` jobs for new/changed files, tombstone
//!   what vanished.
//! * [`Indexer::reconcile`] — the watcher path: re-stat a handful of paths.
//! * [`Indexer::spawn_workers`] — lease jobs and run their stage. Phase 2
//!   implements the `Metadata` stage (mime sniff, name into FTS).
//!
//! All blocking IO/DB work runs on `spawn_blocking`; the async tasks only
//! coordinate.

mod metadata;
mod status;
mod text;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sfe_core::{FileId, FileKind, Root, RootId, Stage};
use sfe_db::{Database, files, fts, jobs, roots};
use sfe_extract::{Limits, Registry};
use sfe_fs::{IgnoreRules, WalkStats};
use tokio::sync::{Notify, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub use status::IndexStatus;

#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error(transparent)]
    Db(#[from] sfe_db::DbError),
    #[error(transparent)]
    Fs(#[from] sfe_fs::FsError),
    #[error("background task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
}

pub type Result<T> = std::result::Result<T, IndexError>;

#[derive(Debug, Clone)]
pub struct IndexerConfig {
    /// Rows per write transaction during discovery.
    pub batch_size: usize,
    /// Concurrent stage workers.
    pub workers: usize,
    /// Jobs leased per worker iteration.
    pub job_batch: usize,
    pub lease_secs: i64,
    /// Bytes read from the head of each file for mime sniffing.
    pub head_bytes: usize,
    pub ignore: IgnoreRules,
    /// Caps for content extraction.
    pub limits: Limits,
}

impl Default for IndexerConfig {
    fn default() -> Self {
        Self {
            batch_size: 2000,
            workers: std::thread::available_parallelism()
                .map(|n| n.get().saturating_sub(1))
                .unwrap_or(2)
                .clamp(1, 4),
            job_batch: 64,
            lease_secs: 120,
            head_bytes: 8192,
            ignore: IgnoreRules::default(),
            limits: Limits::default(),
        }
    }
}

/// Result of a discover pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanOutcome {
    Done {
        stats: WalkStats,
        tombstoned: usize,
        enqueued: usize,
    },
    /// Another scan of this root is in progress; request ignored.
    AlreadyRunning,
    Cancelled,
}

pub struct Indexer {
    db: Arc<Database>,
    cfg: IndexerConfig,
    registry: Registry,
    status: status::StatusHub,
    paused: AtomicBool,
    cancel: CancellationToken,
    /// Wakes idle workers when new jobs land.
    wake: Notify,
    scanning: Mutex<HashSet<RootId>>,
}

impl std::fmt::Debug for Indexer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Indexer")
            .field("paused", &self.paused.load(Ordering::Relaxed))
            .finish()
    }
}

impl Indexer {
    pub fn new(db: Arc<Database>, cfg: IndexerConfig) -> Arc<Self> {
        let status = status::StatusHub::new(&db);
        Arc::new(Self {
            db,
            cfg,
            registry: Registry::with_defaults(),
            status,
            paused: AtomicBool::new(false),
            cancel: CancellationToken::new(),
            wake: Notify::new(),
            scanning: Mutex::new(HashSet::new()),
        })
    }

    pub fn config(&self) -> &IndexerConfig {
        &self.cfg
    }

    pub fn subscribe(&self) -> watch::Receiver<IndexStatus> {
        self.status.subscribe()
    }

    pub fn status(&self) -> IndexStatus {
        self.status.current()
    }

    /// Recount from the database and publish.
    pub fn refresh_status(&self) {
        self.status
            .refresh(&self.db, self.is_paused(), &self.scanning_paths());
    }

    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
        self.refresh_status();
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
        self.wake.notify_waiters();
        self.refresh_status();
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// Stop workers and abort in-flight scans at the next checkpoint.
    pub fn shutdown(&self) {
        self.cancel.cancel();
        self.wake.notify_waiters();
    }

    fn scanning_paths(&self) -> Vec<RootId> {
        self.scanning.lock().unwrap().iter().copied().collect()
    }

    /// Discover stage for `dir` (default: the whole root).
    pub async fn scan(self: &Arc<Self>, root: &Root, dir: Option<&Path>) -> Result<ScanOutcome> {
        let dir: PathBuf = dir
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(&root.path));
        let is_full = dir == Path::new(&root.path);
        if is_full && !self.mark_scanning(root.id) {
            return Ok(ScanOutcome::AlreadyRunning);
        }
        self.run_scan(root.clone(), dir, is_full).await
    }

    /// Queue a full scan on `handle` and return immediately. The root shows
    /// as scanning in the status *before* this returns, so callers polling
    /// for idleness cannot miss it. Returns `false` if already scanning.
    pub fn request_scan(self: &Arc<Self>, handle: &tokio::runtime::Handle, root: Root) -> bool {
        if !self.mark_scanning(root.id) {
            return false;
        }
        let this = Arc::clone(self);
        handle.spawn(async move {
            let path = root.path.clone();
            let dir = PathBuf::from(&root.path);
            if let Err(e) = this.run_scan(root, dir, true).await {
                tracing::error!(path = %path, error = %e, "scan failed");
            }
        });
        true
    }

    fn mark_scanning(&self, id: RootId) -> bool {
        let inserted = self.scanning.lock().unwrap().insert(id);
        if inserted {
            self.refresh_status();
        }
        inserted
    }

    async fn run_scan(
        self: &Arc<Self>,
        root: Root,
        dir: PathBuf,
        is_full: bool,
    ) -> Result<ScanOutcome> {
        let this = Arc::clone(self);
        let root_id = root.id;
        let res =
            tokio::task::spawn_blocking(move || this.scan_blocking(&root, &dir, is_full)).await;
        if is_full {
            self.scanning.lock().unwrap().remove(&root_id);
        }
        self.refresh_status();
        self.wake.notify_waiters();
        res?
    }

    fn scan_blocking(&self, root: &Root, dir: &Path, is_full: bool) -> Result<ScanOutcome> {
        let started = sfe_db::now();
        let root_id = root.id;
        let mut enqueued = 0usize;
        let mut last_status = std::time::Instant::now();
        let cancel = self.cancel.clone();

        let walked = sfe_fs::walk(
            root_id,
            dir,
            &self.cfg.ignore,
            self.cfg.batch_size,
            || cancel.is_cancelled(),
            |batch| -> Result<()> {
                let n = self.db.transaction(|c| {
                    let ups = files::upsert_discovered(c, &batch)?;
                    let todo: Vec<FileId> =
                        ups.iter().filter(|u| u.needs_index).map(|u| u.id).collect();
                    jobs::enqueue_many(c, &todo, Stage::Metadata, 0)?;
                    Ok(todo.len())
                })?;
                enqueued += n;
                if last_status.elapsed() > Duration::from_millis(500) {
                    self.refresh_status();
                    self.wake.notify_waiters();
                    last_status = std::time::Instant::now();
                }
                Ok(())
            },
        )?;
        let stats = walked?;
        if cancel.is_cancelled() {
            return Ok(ScanOutcome::Cancelled);
        }

        let dir_s = sfe_fs::path_to_string(dir)?;
        let tombstoned = self.db.transaction(|c| {
            let gone = files::tombstone_under(c, root_id, &dir_s, false, started)?;
            fts::delete_many(c, &gone)?;
            jobs::clear_for_files(c, &gone)?;
            if is_full {
                roots::set_last_scan(c, root_id, sfe_db::now())?;
            }
            Ok(gone.len())
        })?;
        tracing::info!(root = %root.path, dir = %dir_s, ?stats, tombstoned, enqueued, "scan complete");
        Ok(ScanOutcome::Done {
            stats,
            tombstoned,
            enqueued,
        })
    }

    /// Watcher path: re-stat each path and bring the index in line. New
    /// directories are scanned; vanished ones are tombstoned recursively.
    pub async fn reconcile(self: &Arc<Self>, root: &Root, paths: Vec<PathBuf>) -> Result<()> {
        let this = Arc::clone(self);
        let root_owned = root.clone();
        let new_dirs =
            tokio::task::spawn_blocking(move || this.reconcile_blocking(&root_owned, &paths))
                .await??;
        for d in new_dirs {
            self.scan(root, Some(&d)).await?;
        }
        self.refresh_status();
        self.wake.notify_waiters();
        Ok(())
    }

    fn reconcile_blocking(&self, root: &Root, paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
        let mut new_dirs = Vec::new();
        for path in paths {
            if !path.starts_with(&root.path) {
                continue;
            }
            if self.cfg.ignore.skip_path(path, path.is_dir(), None)
                || path
                    .strip_prefix(&root.path)
                    .ok()
                    .map(|rel| {
                        rel.components().any(|c| {
                            c.as_os_str()
                                .to_str()
                                .is_some_and(|n| self.cfg.ignore.skip(n, true, None))
                        })
                    })
                    .unwrap_or(false)
            {
                continue;
            }
            match sfe_fs::stat(root.id, path)? {
                Some(d) => {
                    let is_dir = d.kind == FileKind::Dir;
                    let up = self.db.transaction(|c| {
                        let up = files::upsert_discovered(c, std::slice::from_ref(&d))?[0];
                        if up.needs_index {
                            jobs::enqueue(c, up.id, Stage::Metadata, 10)?;
                        }
                        Ok(up)
                    })?;
                    if is_dir && up.needs_index {
                        new_dirs.push(path.clone());
                    }
                }
                None => {
                    let p = sfe_fs::path_to_string(path)?;
                    self.db.transaction(|c| {
                        let mut gone = files::tombstone_under(c, root.id, &p, true, i64::MAX)?;
                        if let Some(id) = files::tombstone_path(c, &p)? {
                            gone.push(id);
                        }
                        fts::delete_many(c, &gone)?;
                        jobs::clear_for_files(c, &gone)?;
                        Ok(())
                    })?;
                }
            }
        }
        Ok(new_dirs)
    }

    /// Spawn the stage workers. They run until [`Indexer::shutdown`].
    pub fn spawn_workers(self: &Arc<Self>) -> Vec<JoinHandle<()>> {
        (0..self.cfg.workers.max(1))
            .map(|i| {
                let this = Arc::clone(self);
                tokio::spawn(async move { this.worker_loop(i).await })
            })
            .collect()
    }

    async fn worker_loop(self: Arc<Self>, idx: usize) {
        tracing::debug!(worker = idx, "worker started");
        loop {
            if self.cancel.is_cancelled() {
                break;
            }
            if self.is_paused() {
                tokio::select! {
                    _ = self.wake.notified() => {}
                    _ = self.cancel.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                }
                continue;
            }
            let this = Arc::clone(&self);
            let processed = match tokio::task::spawn_blocking(move || this.work_batch()).await {
                Ok(Ok(n)) => n,
                Ok(Err(e)) => {
                    tracing::error!(error = %e, "worker batch failed");
                    self.status.set_error(e.to_string());
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
                Err(e) => {
                    tracing::error!(error = %e, "worker task panicked");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
            };
            if processed == 0 {
                tokio::select! {
                    _ = self.wake.notified() => {}
                    _ = self.cancel.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_secs(2)) => {}
                }
            }
        }
        tracing::debug!(worker = idx, "worker stopped");
    }

    /// Lease a batch and run it. Returns the number of jobs processed.
    fn work_batch(&self) -> Result<usize> {
        let leased = self
            .db
            .transaction(|c| jobs::lease(c, None, self.cfg.job_batch, self.cfg.lease_secs))?;
        if leased.is_empty() {
            return Ok(0);
        }
        // Cheap stages drain first (lease orders by stage), so a batch is
        // normally homogeneous; handle mixes anyway.
        let records = {
            let conn = self.db.writer();
            let ids: Vec<FileId> = leased.iter().map(|j| j.file_id).collect();
            files::get_many(&conn, &ids)?
        };
        let n = leased.len();
        let mut meta_results = Vec::new();
        let mut text_results = Vec::new();
        // IO outside the write lock.
        for job in &leased {
            let rec = records.iter().find(|r| r.id == job.file_id);
            match job.stage {
                Stage::Metadata => {
                    let out = match rec {
                        Some(rec) => metadata::run(rec, self.cfg.head_bytes, &self.registry),
                        None => metadata::Outcome::Gone,
                    };
                    meta_results.push((job, out));
                }
                Stage::Text => {
                    let out = match rec {
                        Some(rec) => text::run(rec, &self.registry, &self.cfg.limits),
                        None => text::Outcome::Gone,
                    };
                    text_results.push((job, out));
                }
                Stage::Embed => {
                    // Not implemented until Phase 5; drop the job so it does not spin.
                    text_results.push((job, text::Outcome::Unsupported));
                }
            }
        }
        self.db.transaction(|c| {
            for (job, out) in meta_results {
                metadata::apply(c, job, out)?;
            }
            for (job, out) in text_results {
                text::apply(c, job, out)?;
            }
            Ok(())
        })?;
        self.refresh_status();
        Ok(n)
    }
}

#[cfg(test)]
mod tests;
