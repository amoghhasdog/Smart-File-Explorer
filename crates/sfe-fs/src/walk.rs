//! Recursive directory walk producing [`DiscoveredFile`] batches.

use std::path::Path;

use sfe_core::{DiscoveredFile, FileKind, RootId};
use walkdir::WalkDir;

use crate::policy::IgnoreRules;
use crate::probe::{attributes, discovered};
use crate::{FsError, Result};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WalkStats {
    pub files: u64,
    pub dirs: u64,
    pub skipped: u64,
    pub errors: u64,
}

/// Walk `dir` (inside root `root_id`), calling `sink` with batches of at most
/// `batch_size` entries. The directory itself is not emitted; its descendants
/// are. Symlinks and junctions are never followed. Returning `Err` from `sink`
/// aborts the walk; `should_stop` is polled between entries for cooperative
/// cancellation (the walk then ends early with the stats so far).
pub fn walk<E>(
    root_id: RootId,
    dir: &Path,
    rules: &IgnoreRules,
    batch_size: usize,
    mut should_stop: impl FnMut() -> bool,
    mut sink: impl FnMut(Vec<DiscoveredFile>) -> std::result::Result<(), E>,
) -> Result<std::result::Result<WalkStats, E>> {
    if !dir.is_dir() {
        return Err(FsError::NotADirectory(dir.to_path_buf()));
    }
    let mut stats = WalkStats::default();
    let mut batch = Vec::with_capacity(batch_size);
    let walker = WalkDir::new(dir)
        .follow_links(false)
        .min_depth(1)
        .into_iter()
        .filter_entry(|e| {
            let attrs = e.metadata().ok().as_ref().and_then(attributes);
            !rules.skip_path(e.path(), e.file_type().is_dir(), attrs)
        });

    for entry in walker {
        if should_stop() {
            break;
        }
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                stats.errors += 1;
                tracing::debug!(error = %e, "walk error");
                continue;
            }
        };
        let ft = entry.file_type();
        let kind = if ft.is_dir() {
            FileKind::Dir
        } else if ft.is_file() {
            FileKind::File
        } else {
            stats.skipped += 1;
            continue;
        };
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => {
                stats.errors += 1;
                continue;
            }
        };
        match discovered(root_id, entry.path(), kind, &meta) {
            Ok(d) => {
                match kind {
                    FileKind::Dir => stats.dirs += 1,
                    FileKind::File => stats.files += 1,
                }
                batch.push(d);
            }
            Err(FsError::NonUtf8(p)) => {
                stats.skipped += 1;
                tracing::warn!(path = %p.display(), "skipping non-UTF-8 path");
            }
            Err(e) => return Err(e),
        }
        if batch.len() >= batch_size {
            let full = std::mem::replace(&mut batch, Vec::with_capacity(batch_size));
            if let Err(e) = sink(full) {
                return Ok(Err(e));
            }
        }
    }
    if !batch.is_empty()
        && let Err(e) = sink(batch)
    {
        return Ok(Err(e));
    }
    Ok(Ok(stats))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(dir: &Path, rules: &IgnoreRules, batch: usize) -> (WalkStats, Vec<DiscoveredFile>) {
        let mut all = Vec::new();
        let stats = walk(
            RootId(1),
            dir,
            rules,
            batch,
            || false,
            |b| {
                all.extend(b);
                Ok::<(), ()>(())
            },
        )
        .unwrap()
        .unwrap();
        all.sort_by(|a, b| a.path.cmp(&b.path));
        (stats, all)
    }

    #[test]
    fn walks_recursively_in_batches_and_applies_rules() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::create_dir_all(p.join("sub/deeper")).unwrap();
        std::fs::create_dir_all(p.join("node_modules/x")).unwrap();
        std::fs::write(p.join("a.txt"), "a").unwrap();
        std::fs::write(p.join("sub/b.md"), "bb").unwrap();
        std::fs::write(p.join("sub/deeper/c.PNG"), "ccc").unwrap();
        std::fs::write(p.join("node_modules/x/ignored.js"), "").unwrap();
        std::fs::write(p.join(".secret"), "").unwrap();

        let (stats, all) = collect(p, &IgnoreRules::default(), 2);
        let names: Vec<_> = all.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["a.txt", "sub", "b.md", "deeper", "c.PNG"]);
        assert_eq!(stats.files, 3);
        assert_eq!(stats.dirs, 2);
        let c = all.iter().find(|d| d.name == "c.PNG").unwrap();
        assert_eq!(c.ext, "png");
        assert_eq!(c.size, 3);
        assert_eq!(c.kind, FileKind::File);
    }

    #[test]
    fn stop_flag_aborts_and_sink_error_propagates() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..10 {
            std::fs::write(dir.path().join(format!("{i}.txt")), "x").unwrap();
        }
        let rules = IgnoreRules::default();
        let mut n = 0;
        let r = walk(
            RootId(1),
            dir.path(),
            &rules,
            3,
            || {
                n += 1;
                n > 4
            },
            |_| Ok::<(), ()>(()),
        )
        .unwrap()
        .unwrap();
        assert!(r.files < 10);

        let r = walk(
            RootId(1),
            dir.path(),
            &rules,
            3,
            || false,
            |_| Err("sink failed"),
        )
        .unwrap();
        assert_eq!(r, Err("sink failed"));

        let missing = dir.path().join("nope");
        assert!(
            walk(
                RootId(1),
                &missing,
                &rules,
                3,
                || false,
                |_| Ok::<(), ()>(())
            )
            .is_err()
        );
    }
}
