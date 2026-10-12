//! Stage 3 — embed: chunk the extracted text, embed each chunk, store the
//! vectors in `chunks` and the ANN index.

use std::sync::Arc;

use sfe_core::{FileId, IndexState, Job};
use sfe_db::{Result, chunks, files, fts, jobs, rusqlite::Connection};
use sfe_embed::TextEmbedder;
use sfe_vector::VectorIndex;

use crate::chunk::chunk_text;

#[derive(Debug)]
pub(crate) enum Outcome {
    Ok {
        texts: Vec<String>,
        vectors: Vec<Vec<f32>>,
    },
    /// Nothing to embed (no text) — still counts as embedded.
    Empty,
    Failed(String),
}

/// Read the file's text and embed it. Runs outside the write lock.
pub(crate) fn run(conn: &Connection, file: FileId, embedder: &dyn TextEmbedder) -> Outcome {
    let text = match fts::get_text(conn, file) {
        Ok(Some(t)) if !t.trim().is_empty() => t,
        Ok(_) => return Outcome::Empty,
        Err(e) => return Outcome::Failed(e.to_string()),
    };
    let texts = chunk_text(&text);
    if texts.is_empty() {
        return Outcome::Empty;
    }
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    match embedder.embed(&refs) {
        Ok(vectors) if vectors.len() == texts.len() => Outcome::Ok { texts, vectors },
        Ok(v) => Outcome::Failed(format!(
            "embedder returned {} vectors for {} chunks",
            v.len(),
            texts.len()
        )),
        Err(e) => Outcome::Failed(e.to_string()),
    }
}

pub(crate) fn apply(
    c: &Connection,
    job: &Job,
    out: Outcome,
    model: &str,
    index: &Arc<dyn VectorIndex>,
) -> Result<()> {
    match out {
        Outcome::Ok { texts, vectors } => {
            let new: Vec<chunks::NewChunk<'_>> = texts
                .iter()
                .zip(&vectors)
                .enumerate()
                .map(|(i, (t, v))| chunks::NewChunk {
                    ord: i as u32,
                    modality: "text",
                    text: Some(t.as_str()),
                    model,
                    vec: v.as_slice(),
                })
                .collect();
            let old = chunks::delete_for_file(c, job.file_id)?;
            for id in old {
                let _ = index.remove(id.0 as u64);
            }
            let ids = chunks::replace_for_file(c, job.file_id, &new)?;
            let items: Vec<(u64, Vec<f32>)> = ids
                .iter()
                .zip(vectors)
                .map(|(id, v)| (id.0 as u64, v))
                .collect();
            if let Err(e) = index.add_batch(&items) {
                tracing::error!(error = %e, "vector index add failed");
            }
            files::set_state(c, job.file_id, IndexState::Embedded, None)?;
            jobs::complete(c, job.id)?;
        }
        Outcome::Empty => {
            let old = chunks::delete_for_file(c, job.file_id)?;
            for id in old {
                let _ = index.remove(id.0 as u64);
            }
            files::set_state(c, job.file_id, IndexState::Embedded, None)?;
            jobs::complete(c, job.id)?;
        }
        Outcome::Failed(msg) => {
            if !jobs::fail(c, job.id)? {
                files::set_state(c, job.file_id, IndexState::Failed, Some(&msg))?;
            }
        }
    }
    Ok(())
}
