//! Stage 3 — embed. Text files: chunk the extracted text and embed each
//! chunk with the text model. Images: embed the thumbnail with the image
//! model. Vectors go to `chunks` and the matching ANN index.

use std::path::Path;
use std::sync::Arc;

use sfe_core::{FileId, IndexState, Job};
use sfe_db::{Result, chunks, files, fts, jobs, rusqlite::Connection};
use sfe_embed::{ImageEmbedder, TextEmbedder};
use sfe_vector::VectorIndex;

use crate::chunk::chunk_text;

pub(crate) const TEXT: &str = "text";
pub(crate) const IMAGE: &str = "image";

#[derive(Debug)]
pub(crate) enum Outcome {
    Ok {
        modality: &'static str,
        texts: Vec<Option<String>>,
        vectors: Vec<Vec<f32>>,
    },
    /// Nothing to embed — still counts as embedded.
    Empty,
    Failed(String),
}

/// Text path. Runs outside the write lock.
pub(crate) fn run_text(conn: &Connection, file: FileId, embedder: &dyn TextEmbedder) -> Outcome {
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
        Ok(vectors) if vectors.len() == texts.len() => Outcome::Ok {
            modality: TEXT,
            texts: texts.into_iter().map(Some).collect(),
            vectors,
        },
        Ok(v) => Outcome::Failed(format!(
            "embedder returned {} vectors for {} chunks",
            v.len(),
            texts.len()
        )),
        Err(e) => Outcome::Failed(e.to_string()),
    }
}

/// Image path: one vector per file from its thumbnail (or the original).
pub(crate) fn run_image(path: &str, thumb: Option<&Path>, embedder: &dyn ImageEmbedder) -> Outcome {
    let img = match crate::thumbs::load_for_embedding(thumb, Path::new(path)) {
        Ok(i) => i,
        Err(e) => return Outcome::Failed(format!("decode: {e}")),
    };
    match embedder.embed_images(vec![img]) {
        Ok(mut v) if v.len() == 1 => Outcome::Ok {
            modality: IMAGE,
            texts: vec![None],
            vectors: vec![v.remove(0)],
        },
        Ok(_) => Outcome::Failed("image embedder returned no vector".into()),
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
        Outcome::Ok {
            modality,
            texts,
            vectors,
        } => {
            let new: Vec<chunks::NewChunk<'_>> = texts
                .iter()
                .zip(&vectors)
                .enumerate()
                .map(|(i, (t, v))| chunks::NewChunk {
                    ord: i as u32,
                    modality,
                    text: t.as_deref(),
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
