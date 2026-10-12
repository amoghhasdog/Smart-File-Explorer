//! Reading small amounts of a file: head bytes for mime sniffing, and stat.

use std::io::Read;
use std::path::Path;

use sfe_core::{DiscoveredFile, FileKind, RootId};

use crate::{FsError, Result, path_to_string, to_unix};

/// Read up to `n` bytes from the start of a file.
pub fn read_head(path: &Path, n: usize) -> Result<Vec<u8>> {
    let mut f = std::fs::File::open(path).map_err(|e| FsError::Io(path.to_path_buf(), e))?;
    let mut buf = vec![0u8; n];
    let mut filled = 0;
    while filled < n {
        match f.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(k) => filled += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(FsError::Io(path.to_path_buf(), e)),
        }
    }
    buf.truncate(filled);
    Ok(buf)
}

/// Open a file for sequential reading (handed to extractors).
pub fn open_read(path: &Path) -> Result<std::fs::File> {
    std::fs::File::open(path).map_err(|e| FsError::Io(path.to_path_buf(), e))
}

/// Stat a single path into a [`DiscoveredFile`] (used by the watcher).
/// Returns `Ok(None)` if the path no longer exists or is not a plain
/// file/directory.
pub fn stat(root_id: RootId, path: &Path) -> Result<Option<DiscoveredFile>> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(FsError::Io(path.to_path_buf(), e)),
    };
    let kind = if meta.is_dir() {
        FileKind::Dir
    } else if meta.is_file() {
        FileKind::File
    } else {
        return Ok(None);
    };
    Ok(Some(discovered(root_id, path, kind, &meta)?))
}

pub(crate) fn discovered(
    root_id: RootId,
    path: &Path,
    kind: FileKind,
    meta: &std::fs::Metadata,
) -> Result<DiscoveredFile> {
    let full = path_to_string(path)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    let parent_path = path
        .parent()
        .map(path_to_string)
        .transpose()?
        .unwrap_or_default();
    let ext = match kind {
        FileKind::File => path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default(),
        FileKind::Dir => String::new(),
    };
    Ok(DiscoveredFile {
        root_id,
        path: full,
        parent_path,
        name,
        ext,
        kind,
        size: if kind == FileKind::File {
            meta.len()
        } else {
            0
        },
        mtime: to_unix(meta.modified()).unwrap_or(0),
        ctime: to_unix(meta.created()),
    })
}

/// Raw Windows attributes, `None` elsewhere.
pub(crate) fn attributes(meta: &std::fs::Metadata) -> Option<u32> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        Some(meta.file_attributes())
    }
    #[cfg(not(windows))]
    {
        let _ = meta;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_head_truncates_and_handles_short_files() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.bin");
        std::fs::write(&p, b"hello world").unwrap();
        assert_eq!(read_head(&p, 5).unwrap(), b"hello");
        assert_eq!(read_head(&p, 100).unwrap(), b"hello world");
        assert!(
            read_head(&dir.path().join("missing"), 5)
                .unwrap_err()
                .is_not_found()
        );
    }

    #[test]
    fn stat_file_dir_and_missing() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("Report.PDF");
        std::fs::write(&p, b"12345").unwrap();
        let f = stat(RootId(1), &p).unwrap().unwrap();
        assert_eq!(f.kind, FileKind::File);
        assert_eq!(f.ext, "pdf");
        assert_eq!(f.name, "Report.PDF");
        assert_eq!(f.size, 5);
        assert!(f.mtime > 0);
        assert_eq!(f.parent_path, path_to_string(dir.path()).unwrap());

        let d = stat(RootId(1), dir.path()).unwrap().unwrap();
        assert_eq!(d.kind, FileKind::Dir);
        assert_eq!(d.ext, "");
        assert_eq!(d.size, 0);

        assert!(stat(RootId(1), &dir.path().join("nope")).unwrap().is_none());
    }
}
