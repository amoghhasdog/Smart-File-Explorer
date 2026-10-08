//! Domain model: roots, files, metadata, pipeline jobs.
//!
//! Timestamps are unix seconds (`i64`). Paths are stored as `String`:
//! absolute, canonical, forward-slash-agnostic (whatever the OS gives us),
//! and unique per root. Non-UTF-8 paths are rejected at the `sfe-fs` boundary.

use serde::{Deserialize, Serialize};

use crate::{FileId, RootId};

/// A folder the user chose to index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Root {
    pub id: RootId,
    pub path: String,
    pub added_at: i64,
    pub enabled: bool,
    pub last_scan_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    File,
    Dir,
}

impl FileKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FileKind::File => "file",
            FileKind::Dir => "dir",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "file" => Some(FileKind::File),
            "dir" => Some(FileKind::Dir),
            _ => None,
        }
    }
}

/// Coarse file category, derived from extension (and later mime sniffing).
/// Drives type filters such as "images" or "documents" in queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Document,
    Image,
    Video,
    Audio,
    Code,
    Archive,
    Other,
}

impl Category {
    pub const ALL: [Category; 7] = [
        Category::Document,
        Category::Image,
        Category::Video,
        Category::Audio,
        Category::Code,
        Category::Archive,
        Category::Other,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Category::Document => "document",
            Category::Image => "image",
            Category::Video => "video",
            Category::Audio => "audio",
            Category::Code => "code",
            Category::Archive => "archive",
            Category::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Category::ALL.into_iter().find(|c| c.as_str() == s)
    }

    /// Classify by lowercase extension without a leading dot.
    pub fn from_ext(ext: &str) -> Self {
        match ext {
            "txt" | "md" | "markdown" | "rtf" | "pdf" | "doc" | "docx" | "odt" | "xls" | "xlsx"
            | "ods" | "csv" | "tsv" | "ppt" | "pptx" | "odp" | "epub" | "tex" | "log" => {
                Category::Document
            }
            "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "tif" | "tiff" | "heic" | "heif"
            | "avif" | "svg" | "ico" | "raw" | "cr2" | "nef" | "arw" | "dng" => Category::Image,
            "mp4" | "mkv" | "mov" | "avi" | "webm" | "wmv" | "m4v" | "flv" | "mpg" | "mpeg"
            | "3gp" => Category::Video,
            "mp3" | "wav" | "flac" | "aac" | "m4a" | "ogg" | "opus" | "wma" | "aiff" | "mid"
            | "midi" => Category::Audio,
            "rs" | "py" | "js" | "jsx" | "ts" | "tsx" | "java" | "kt" | "c" | "h" | "cpp"
            | "cc" | "hpp" | "cs" | "go" | "rb" | "php" | "swift" | "scala" | "sh" | "ps1"
            | "bat" | "cmd" | "sql" | "html" | "htm" | "css" | "scss" | "json" | "yaml" | "yml"
            | "toml" | "xml" | "ini" | "cfg" | "lua" | "r" | "m" | "dart" | "zig" | "vue"
            | "svelte" | "ipynb" => Category::Code,
            "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "zst" | "iso" | "cab" | "jar" => {
                Category::Archive
            }
            _ => Category::Other,
        }
    }
}

/// Progress of a file through the indexing pipeline. Stored as an integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IndexState {
    /// Seen on disk; nothing read yet.
    Discovered = 0,
    /// Mime/category/basic metadata recorded; name is searchable.
    Metadata = 1,
    /// Text extracted and in FTS.
    Text = 2,
    /// Embeddings written.
    Embedded = 3,
    /// Gave up on this file; see `index_error`.
    Failed = 9,
}

impl IndexState {
    pub fn from_i64(v: i64) -> Option<Self> {
        match v {
            0 => Some(IndexState::Discovered),
            1 => Some(IndexState::Metadata),
            2 => Some(IndexState::Text),
            3 => Some(IndexState::Embedded),
            9 => Some(IndexState::Failed),
            _ => None,
        }
    }
}

/// Pipeline stage a job belongs to. Ordered: cheap stages run for the whole
/// corpus before expensive ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Metadata = 1,
    Text = 2,
    Embed = 3,
}

impl Stage {
    pub fn from_i64(v: i64) -> Option<Self> {
        match v {
            1 => Some(Stage::Metadata),
            2 => Some(Stage::Text),
            3 => Some(Stage::Embed),
            _ => None,
        }
    }
}

/// What the discover stage learns about an entry from a directory listing
/// alone, before opening it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredFile {
    pub root_id: RootId,
    pub path: String,
    pub parent_path: String,
    pub name: String,
    /// Lowercase, no leading dot; empty when none.
    pub ext: String,
    pub kind: FileKind,
    pub size: u64,
    pub mtime: i64,
    pub ctime: Option<i64>,
}

impl DiscoveredFile {
    pub fn category(&self) -> Category {
        match self.kind {
            FileKind::Dir => Category::Other,
            FileKind::File => Category::from_ext(&self.ext),
        }
    }
}

/// A row of the `files` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRecord {
    pub id: FileId,
    pub root_id: RootId,
    pub path: String,
    pub parent_path: String,
    pub name: String,
    pub ext: String,
    pub kind: FileKind,
    pub category: Category,
    pub size: u64,
    pub mtime: i64,
    pub ctime: Option<i64>,
    pub content_hash: Option<Vec<u8>>,
    pub mime: Option<String>,
    pub index_state: IndexState,
    pub index_error: Option<String>,
    pub indexed_at: Option<i64>,
    pub updated_at: i64,
    pub deleted: bool,
}

/// Sparse type-specific metadata (`file_meta` table). Every field optional so
/// extractors can contribute partial patches.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FileMeta {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_ms: Option<u64>,
    pub page_count: Option<u32>,
    pub word_count: Option<u32>,
    pub camera_make: Option<String>,
    pub taken_at: Option<i64>,
    pub gps_lat: Option<f64>,
    pub gps_lon: Option<f64>,
    /// JSON `[[r,g,b,share], ...]`, top dominant colours.
    pub dominant_colors: Option<String>,
    /// 12-bin hue histogram, each bin 0..=255 proportional share.
    pub hue_hist: Option<Vec<u8>>,
    pub language: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    /// Extractor-specific overflow as JSON.
    pub extra: Option<String>,
}

/// A unit of pipeline work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub id: i64,
    pub file_id: FileId,
    pub stage: Stage,
    pub priority: i32,
    pub attempts: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_from_common_extensions() {
        assert_eq!(Category::from_ext("pdf"), Category::Document);
        assert_eq!(Category::from_ext("png"), Category::Image);
        assert_eq!(Category::from_ext("mp4"), Category::Video);
        assert_eq!(Category::from_ext("flac"), Category::Audio);
        assert_eq!(Category::from_ext("rs"), Category::Code);
        assert_eq!(Category::from_ext("zip"), Category::Archive);
        assert_eq!(Category::from_ext(""), Category::Other);
        assert_eq!(Category::from_ext("xyz"), Category::Other);
    }

    #[test]
    fn category_string_round_trip() {
        for c in Category::ALL {
            assert_eq!(Category::parse(c.as_str()), Some(c));
        }
        assert_eq!(Category::parse("nope"), None);
    }

    #[test]
    fn index_state_and_stage_integer_round_trip() {
        for s in [
            IndexState::Discovered,
            IndexState::Metadata,
            IndexState::Text,
            IndexState::Embedded,
            IndexState::Failed,
        ] {
            assert_eq!(IndexState::from_i64(s as i64), Some(s));
        }
        assert_eq!(IndexState::from_i64(4), None);
        for s in [Stage::Metadata, Stage::Text, Stage::Embed] {
            assert_eq!(Stage::from_i64(s as i64), Some(s));
        }
    }

    #[test]
    fn directories_are_never_categorised_by_extension() {
        let d = DiscoveredFile {
            root_id: RootId(1),
            path: "C:/x/photos.png".into(),
            parent_path: "C:/x".into(),
            name: "photos.png".into(),
            ext: "png".into(),
            kind: FileKind::Dir,
            size: 0,
            mtime: 0,
            ctime: None,
        };
        assert_eq!(d.category(), Category::Other);
    }
}
