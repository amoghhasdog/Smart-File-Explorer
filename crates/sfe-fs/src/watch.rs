//! Debounced recursive change watching.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};

use crate::Result;

/// A batch of paths whose state may have changed. The receiver re-stats each
/// path and reconciles the index; it does not trust the event kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsChange {
    pub paths: Vec<PathBuf>,
    /// The OS buffer overflowed or the watch was reset: do a full rescan of
    /// the affected roots instead of trusting `paths`.
    pub rescan: bool,
}

/// Watches directories recursively, coalescing bursts of events.
pub struct Watcher {
    debouncer: Debouncer<notify::RecommendedWatcher, RecommendedCache>,
}

impl Watcher {
    /// Events are delivered on the returned channel after `debounce` of quiet.
    pub fn new(debounce: Duration) -> Result<(Self, mpsc::Receiver<FsChange>)> {
        let (tx, rx) = mpsc::channel();
        let debouncer = new_debouncer(debounce, None, move |res: DebounceEventResult| {
            let change = match res {
                Ok(events) => {
                    let mut paths: Vec<PathBuf> = Vec::new();
                    let mut rescan = false;
                    for ev in events {
                        if ev.need_rescan() {
                            rescan = true;
                        }
                        for p in &ev.paths {
                            if !paths.contains(p) {
                                paths.push(p.clone());
                            }
                        }
                    }
                    FsChange { paths, rescan }
                }
                Err(errors) => {
                    tracing::warn!(?errors, "watch errors; requesting rescan");
                    FsChange {
                        paths: vec![],
                        rescan: true,
                    }
                }
            };
            let _ = tx.send(change);
        })?;
        Ok((Self { debouncer }, rx))
    }

    pub fn watch(&mut self, dir: &Path) -> Result<()> {
        self.debouncer.watch(dir, RecursiveMode::Recursive)?;
        Ok(())
    }

    pub fn unwatch(&mut self, dir: &Path) -> Result<()> {
        self.debouncer.unwatch(dir)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_created_paths() {
        let dir = tempfile::tempdir().unwrap();
        let (mut w, rx) = Watcher::new(Duration::from_millis(200)).unwrap();
        w.watch(dir.path()).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let f = dir.path().join("new.txt");
        std::fs::write(&f, "hi").unwrap();
        let mut seen = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if let Ok(ch) = rx.recv_timeout(Duration::from_millis(500)) {
                seen.extend(ch.paths);
                if seen.iter().any(|p| p.ends_with("new.txt")) {
                    break;
                }
            }
        }
        assert!(seen.iter().any(|p| p.ends_with("new.txt")), "saw {seen:?}");
        w.unwatch(dir.path()).unwrap();
    }
}
