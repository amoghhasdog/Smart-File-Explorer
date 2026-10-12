//! Stage 1 — metadata: sniff the mime type from the file head and make the
//! name searchable. Cheap (one small read per file) and runs across the whole
//! corpus before any content extraction.

use std::path::Path;

use sfe_core::{FileKind, FileRecord, IndexState, Job, Stage};
use sfe_db::{Result, files, fts, jobs, rusqlite::Connection};
use sfe_extract::{FileProbe, Registry};

#[derive(Debug)]
pub(crate) enum Outcome {
    Ok {
        mime: Option<String>,
        /// An extractor exists for this file: queue the text stage.
        extract: bool,
    },
    /// File vanished between discovery and now.
    Gone,
    /// Transient or permanent failure; the job queue decides on retry.
    Failed(String),
}

/// Pure IO part, run outside any database lock.
pub(crate) fn run(rec: &FileRecord, head_bytes: usize, registry: &Registry) -> Outcome {
    if rec.kind == FileKind::Dir {
        return Outcome::Ok {
            mime: None,
            extract: false,
        };
    }
    match sfe_fs::read_head(Path::new(&rec.path), head_bytes) {
        Ok(head) => {
            let mime = sniff_mime(&head, &rec.ext);
            let extract = rec.size > 0
                && registry.supports(&FileProbe {
                    ext: &rec.ext,
                    mime: mime.as_deref(),
                    size: rec.size,
                });
            Outcome::Ok { mime, extract }
        }
        Err(e) if e.is_not_found() => Outcome::Gone,
        Err(e) => Outcome::Failed(e.to_string()),
    }
}

/// Magic-byte sniff with an extension fallback for text-like formats that
/// have no signature.
pub(crate) fn sniff_mime(head: &[u8], ext: &str) -> Option<String> {
    if let Some(t) = infer::get(head) {
        return Some(t.mime_type().to_string());
    }
    let by_ext = match ext {
        "txt" | "log" => "text/plain",
        "md" | "markdown" => "text/markdown",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        "json" | "ipynb" => "application/json",
        "xml" => "application/xml",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "text/javascript",
        "ts" | "tsx" | "jsx" | "rs" | "py" | "java" | "c" | "h" | "cpp" | "cc" | "hpp" | "cs"
        | "go" | "rb" | "php" | "swift" | "kt" | "scala" | "sh" | "ps1" | "bat" | "sql"
        | "toml" | "yaml" | "yml" | "ini" | "cfg" | "lua" | "r" | "dart" | "zig" | "vue"
        | "svelte" | "tex" => "text/plain",
        "svg" => "image/svg+xml",
        _ => return None,
    };
    Some(by_ext.to_string())
}

/// Persist the outcome and settle the job. Runs inside the writer transaction.
pub(crate) fn apply(c: &Connection, job: &Job, out: Outcome) -> Result<()> {
    match out {
        Outcome::Ok { mime, extract } => {
            if let Some(rec) = files::get(c, job.file_id)? {
                files::set_mime(c, rec.id, mime.as_deref())?;
                fts::set_name(c, rec.id, &rec.name)?;
                files::set_state(c, rec.id, IndexState::Metadata, None)?;
                if extract {
                    jobs::enqueue(c, rec.id, Stage::Text, job.priority)?;
                }
            }
            jobs::complete(c, job.id)?;
        }
        Outcome::Gone => {
            if let Some(rec) = files::get(c, job.file_id)? {
                files::tombstone_path(c, &rec.path)?;
                fts::delete(c, rec.id)?;
            }
            jobs::complete(c, job.id)?;
        }
        Outcome::Failed(msg) => {
            let retrying = jobs::fail(c, job.id)?;
            if !retrying {
                files::set_state(c, job.file_id, IndexState::Failed, Some(&msg))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_magic_bytes_and_falls_back_to_extension() {
        assert_eq!(
            sniff_mime(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR", "png").as_deref(),
            Some("image/png")
        );
        assert_eq!(
            sniff_mime(b"%PDF-1.7\n", "txt").as_deref(),
            Some("application/pdf")
        );
        assert_eq!(sniff_mime(b"hello", "md").as_deref(), Some("text/markdown"));
        assert_eq!(
            sniff_mime(b"fn main() {}", "rs").as_deref(),
            Some("text/plain")
        );
        assert_eq!(sniff_mime(b"\0\x01\x02", "bin"), None);
    }
}
