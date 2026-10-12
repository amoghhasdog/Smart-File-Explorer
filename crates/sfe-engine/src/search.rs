//! Interim keyword search (Phase 2–3). Replaced by `sfe-search` in Phase 4.

use serde::Serialize;
use sfe_core::{Category, FileId};
use sfe_db::{Database, files, fts};

use crate::Result;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub id: FileId,
    pub path: String,
    pub name: String,
    pub ext: String,
    pub category: Category,
    pub size: u64,
    pub mtime: i64,
    pub mime: Option<String>,
    pub snippet: String,
}

/// Build an FTS5 query where every whitespace-separated word must match as a
/// prefix (so "ferr" finds "Ferrari"). Quoted phrases are kept exact.
pub(crate) fn build_query(input: &str) -> Option<String> {
    let mut parts = Vec::new();
    let mut rest = input.trim();
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('"') {
            let end = after.find('"').unwrap_or(after.len());
            let phrase = after[..end].trim();
            if !phrase.is_empty() {
                parts.push(fts::phrase(phrase));
            }
            rest = after.get(end + 1..).unwrap_or("").trim_start();
        } else {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            let word = &rest[..end];
            if !word.is_empty() {
                parts.push(format!("{}*", fts::escape_term(word)));
            }
            rest = rest[end..].trim_start();
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

pub(crate) fn keyword(
    db: &Database,
    input: &str,
    limit: usize,
    offset: usize,
) -> Result<Vec<SearchHit>> {
    let Some(q) = build_query(input) else {
        return Ok(vec![]);
    };
    let conn = db.reader()?;
    let hits = fts::search(&conn, &q, limit, offset)?;
    let ids: Vec<FileId> = hits.iter().map(|h| h.id).collect();
    let recs = files::get_many(&conn, &ids)?;
    Ok(hits
        .into_iter()
        .filter_map(|h| {
            let r = recs.iter().find(|r| r.id == h.id)?;
            Some(SearchHit {
                id: r.id,
                path: r.path.clone(),
                name: r.name.clone(),
                ext: r.ext.clone(),
                category: r.category,
                size: r.size,
                mtime: r.mtime,
                mime: r.mime.clone(),
                snippet: h.snippet,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_prefix_and_phrase_queries() {
        assert_eq!(build_query("  "), None);
        assert_eq!(build_query("ferrari").unwrap(), "\"ferrari\"*");
        assert_eq!(build_query("athena road").unwrap(), "\"athena\"* \"road\"*");
        assert_eq!(
            build_query("\"exact phrase\" more").unwrap(),
            "\"exact phrase\" \"more\"*"
        );
        assert_eq!(build_query("\"unterminated").unwrap(), "\"unterminated\"");
        assert_eq!(build_query("c++ \"\"").unwrap(), "\"c++\"*");
    }
}
