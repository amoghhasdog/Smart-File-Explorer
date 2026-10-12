//! Search and file-detail entry points.

use serde::Serialize;
use sfe_core::{FileId, FileMeta, FileRecord};
use sfe_db::{Database, files, fts, meta};
use sfe_query::Query;
use sfe_search::{SearchResponse, Searcher};

use crate::Result;

pub use sfe_search::Hit as SearchHit;

/// Everything the preview pane shows for one file.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDetails {
    pub record: FileRecord,
    pub meta: Option<FileMeta>,
    /// First part of the extracted text, if any.
    pub text_excerpt: Option<String>,
    pub text_chars: Option<u32>,
    pub extractor: Option<String>,
}

const EXCERPT_CHARS: usize = 3000;

pub(crate) fn search(
    searcher: &Searcher,
    input: &str,
    limit: usize,
    offset: usize,
) -> Result<SearchResponse> {
    let q = sfe_query::parse_now(input);
    Ok(searcher.search(&q, limit, offset)?)
}

pub(crate) fn explain(input: &str) -> Query {
    sfe_query::parse_now(input)
}

pub(crate) fn details(db: &Database, id: FileId) -> Result<Option<FileDetails>> {
    let conn = db.reader()?;
    let Some(record) = files::get(&conn, id)? else {
        return Ok(None);
    };
    let meta = meta::get(&conn, id)?;
    let info = meta::get_text_info(&conn, id)?;
    let text_excerpt = fts::get_text(&conn, id)?
        .filter(|t| !t.is_empty())
        .map(|t| {
            let cut = t
                .char_indices()
                .nth(EXCERPT_CHARS)
                .map(|(i, _)| i)
                .unwrap_or(t.len());
            t[..cut].to_string()
        });
    Ok(Some(FileDetails {
        record,
        meta,
        text_excerpt,
        text_chars: info.as_ref().map(|i| i.char_count),
        extractor: info.map(|i| i.extractor),
    }))
}
