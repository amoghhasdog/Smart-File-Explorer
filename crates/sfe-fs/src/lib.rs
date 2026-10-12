//! Filesystem access boundary.
//!
//! Every `std::fs` / directory-walk / change-watch call in the workspace lives
//! here. Other crates receive paths, readers and change events from this crate
//! and never open files themselves.

pub mod policy;
pub mod probe;
pub mod walk;
pub mod watch;

use std::path::{Path, PathBuf};

pub use policy::IgnoreRules;
pub use probe::{open_read, read_head, stat};
pub use walk::{WalkStats, walk};
pub use watch::{FsChange, Watcher};

#[derive(Debug, thiserror::Error)]
pub enum FsError {
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("path is not valid UTF-8: {0}")]
    NonUtf8(PathBuf),
    #[error("not a directory: {0}")]
    NotADirectory(PathBuf),
    #[error("watch error: {0}")]
    Watch(#[from] notify::Error),
}

impl FsError {
    /// `true` when the underlying cause is "file does not exist".
    pub fn is_not_found(&self) -> bool {
        matches!(self, FsError::Io(_, e) if e.kind() == std::io::ErrorKind::NotFound)
    }
}

pub type Result<T> = std::result::Result<T, FsError>;

/// Resolve a user-chosen folder into the canonical string form stored in the
/// database: absolute, symlinks resolved, no `\\?\` prefix, NFC-normalised.
pub fn canonical_root(path: impl AsRef<Path>) -> Result<String> {
    let path = path.as_ref();
    let canon = dunce::canonicalize(path).map_err(|e| FsError::Io(path.to_path_buf(), e))?;
    if !canon.is_dir() {
        return Err(FsError::NotADirectory(canon));
    }
    path_to_string(&canon)
}

/// Convert a path to its stored string form. Rejects non-UTF-8 paths.
pub fn path_to_string(path: &Path) -> Result<String> {
    use unicode_normalization::UnicodeNormalization;
    let s = path
        .to_str()
        .ok_or_else(|| FsError::NonUtf8(path.to_path_buf()))?;
    Ok(s.nfc().collect())
}

/// Seconds since the unix epoch for a filesystem timestamp.
pub(crate) fn to_unix(t: std::io::Result<std::time::SystemTime>) -> Option<i64> {
    t.ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_root_resolves_and_rejects_files() {
        let dir = tempfile::tempdir().unwrap();
        let s = canonical_root(dir.path()).unwrap();
        assert!(!s.starts_with(r"\\?\"));
        assert!(Path::new(&s).is_absolute());
        let f = dir.path().join("f.txt");
        std::fs::write(&f, "x").unwrap();
        assert!(matches!(canonical_root(&f), Err(FsError::NotADirectory(_))));
        let missing = canonical_root(dir.path().join("nope")).unwrap_err();
        assert!(missing.is_not_found());
    }

    #[test]
    fn path_to_string_normalises_nfc() {
        let decomposed = "e\u{301}";
        let s = path_to_string(Path::new(decomposed)).unwrap();
        assert_eq!(s, "\u{e9}");
    }
}
