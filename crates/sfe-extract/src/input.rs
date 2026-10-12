use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Seekable byte source handed to extractors. Abstract so tests can feed
/// in-memory bytes and so this crate never opens files itself.
pub trait ExtractInput: Read + Seek + Send + Sync {
    fn size(&self) -> u64;
    /// Original path, if any (only used for logging / format hints).
    fn path_hint(&self) -> Option<&Path> {
        None
    }
    /// Read the whole input into memory (bounded by the caller's limits).
    fn read_all(&mut self) -> std::io::Result<Vec<u8>> {
        self.seek(SeekFrom::Start(0))?;
        let mut buf = Vec::with_capacity(self.size().min(64 * 1024 * 1024) as usize);
        self.read_to_end(&mut buf)?;
        Ok(buf)
    }
}

/// An already-open file (opened by `sfe-fs`).
pub struct FileInput {
    file: std::fs::File,
    size: u64,
    path: PathBuf,
}

impl FileInput {
    pub fn new(file: std::fs::File, size: u64, path: PathBuf) -> Self {
        Self { file, size, path }
    }
}

impl Read for FileInput {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buf)
    }
}

impl Seek for FileInput {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.file.seek(pos)
    }
}

impl ExtractInput for FileInput {
    fn size(&self) -> u64 {
        self.size
    }
    fn path_hint(&self) -> Option<&Path> {
        Some(&self.path)
    }
}

/// In-memory input for tests and small buffers.
pub struct MemInput(Cursor<Vec<u8>>);

impl MemInput {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(Cursor::new(bytes))
    }
}

impl Read for MemInput {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Seek for MemInput {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}

impl ExtractInput for MemInput {
    fn size(&self) -> u64 {
        self.0.get_ref().len() as u64
    }
}
