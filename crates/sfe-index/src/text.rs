//! Stage 2 — text: run the matching extractor, store text in FTS and the
//! metadata patch in `file_meta`, hash the content.

use std::io::Read;
use std::path::Path;

use sfe_core::{FileMeta, FileRecord, IndexState, Job};
use sfe_db::{Result, files, fts, jobs, meta, rusqlite::Connection};
use sfe_extract::{ExtractError, FileInput, FileProbe, Limits, Registry};

#[derive(Debug)]
#[allow(clippy::large_enum_variant)] // transient, one per job
pub(crate) enum Outcome {
    Ok {
        text: Option<String>,
        meta: FileMeta,
        truncated: bool,
        extractor: &'static str,
        version: u32,
        hash: Option<[u8; 32]>,
    },
    /// No extractor applies after all (mime changed, etc.).
    Unsupported,
    Gone,
    /// `permanent` failures (parse errors, panics, size caps) are not retried.
    Failed {
        message: String,
        permanent: bool,
    },
}

pub(crate) fn run(rec: &FileRecord, registry: &Registry, limits: &Limits) -> Outcome {
    let path = Path::new(&rec.path);
    let probe = FileProbe {
        ext: &rec.ext,
        mime: rec.mime.as_deref(),
        size: rec.size,
    };
    let Some(extractor) = registry.find(&probe) else {
        return Outcome::Unsupported;
    };
    let file = match sfe_fs::open_read(path) {
        Ok(f) => f,
        Err(e) if e.is_not_found() => return Outcome::Gone,
        Err(e) => {
            return Outcome::Failed {
                message: e.to_string(),
                permanent: false,
            };
        }
    };
    let input = FileInput::new(file, rec.size, path.to_path_buf());
    match sfe_extract::run_guarded(extractor, Box::new(input), limits) {
        Ok(ex) => Outcome::Ok {
            text: ex.text,
            meta: ex.meta,
            truncated: ex.truncated,
            extractor: extractor.id(),
            version: extractor.version(),
            hash: hash_file(path, limits.max_bytes),
        },
        Err(e) => {
            let permanent = !matches!(e, ExtractError::Io(_));
            tracing::debug!(path = %rec.path, extractor = extractor.id(), error = %e, "extraction failed");
            Outcome::Failed {
                message: format!("{}: {e}", extractor.id()),
                permanent,
            }
        }
    }
}

fn hash_file(path: &Path, max_bytes: u64) -> Option<[u8; 32]> {
    let mut f = sfe_fs::open_read(path).ok()?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut total = 0u64;
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > max_bytes {
            return None;
        }
        hasher.update(&buf[..n]);
    }
    Some(*hasher.finalize().as_bytes())
}

pub(crate) fn apply(c: &Connection, job: &Job, out: Outcome) -> Result<()> {
    match out {
        Outcome::Ok {
            text,
            meta: patch,
            truncated,
            extractor,
            version,
            hash,
        } => {
            if let Some(rec) = files::get(c, job.file_id)? {
                let text = text.unwrap_or_default();
                fts::set(c, rec.id, &rec.name, &text)?;
                meta::upsert(c, rec.id, &patch)?;
                meta::set_text_info(
                    c,
                    rec.id,
                    extractor,
                    version,
                    text.chars().count().min(u32::MAX as usize) as u32,
                    truncated,
                )?;
                if let Some(h) = hash {
                    files::set_content_hash(c, rec.id, &h)?;
                }
                files::set_state(c, rec.id, IndexState::Text, None)?;
            }
            jobs::complete(c, job.id)?;
        }
        Outcome::Unsupported => {
            files::set_state(c, job.file_id, IndexState::Text, None)?;
            jobs::complete(c, job.id)?;
        }
        Outcome::Gone => {
            if let Some(rec) = files::get(c, job.file_id)? {
                files::tombstone_path(c, &rec.path)?;
                fts::delete(c, rec.id)?;
            }
            jobs::complete(c, job.id)?;
        }
        Outcome::Failed { message, permanent } => {
            if permanent {
                jobs::complete(c, job.id)?;
                files::set_state(c, job.file_id, IndexState::Failed, Some(&message))?;
            } else if !jobs::fail(c, job.id)? {
                files::set_state(c, job.file_id, IndexState::Failed, Some(&message))?;
            }
        }
    }
    Ok(())
}
