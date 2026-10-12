//! Keyword strategy: FTS5 `MATCH` with bm25 (name ×3), filters pushed down.
//! Also hosts the filter-only path (no text at all → newest first).

use sfe_db::fts;
use sfe_db::rusqlite::{Connection, params_from_iter, types::Value};
use sfe_query::Query;

use crate::{Candidate, Result, SearchStrategy, sql};

pub struct Keyword;

/// FTS5 expression: phrases exact, terms as prefixes, all required. When the
/// user only gave a topic ("about X"), its words act as terms.
pub fn fts_expression(q: &Query) -> Option<String> {
    let mut parts: Vec<String> = q.phrases.iter().map(|p| fts::phrase(p)).collect();
    let terms: Vec<&str> = if q.terms.is_empty() {
        q.semantic
            .as_deref()
            .map(|s| s.split_whitespace().collect())
            .unwrap_or_default()
    } else {
        q.terms.iter().map(String::as_str).collect()
    };
    parts.extend(terms.iter().map(|t| format!("{}*", fts::escape_term(t))));
    (!parts.is_empty()).then(|| parts.join(" "))
}

impl SearchStrategy for Keyword {
    fn name(&self) -> &'static str {
        "keyword"
    }

    fn applies(&self, q: &Query) -> bool {
        fts_expression(q).is_some()
    }

    fn run(&self, conn: &Connection, q: &Query, k: usize) -> Result<Vec<Candidate>> {
        let Some(expr) = fts_expression(q) else {
            return Ok(vec![]);
        };
        let (filter_sql, mut params) = sql::where_clause(&q.filters);
        let stmt_sql = format!(
            "SELECT f.id, bm25(fts, 3.0, 1.0) AS score, snippet(fts, 1, '[', ']', '…', 12)
             FROM fts JOIN files f ON f.id = fts.rowid
             LEFT JOIN file_meta fm ON fm.file_id = f.id
             WHERE {base}{filter_sql} AND fts MATCH ?
             ORDER BY score LIMIT ?",
            base = sql::BASE,
        );
        params.push(Value::from(expr));
        params.push(Value::from(k as i64));
        let mut stmt = conn.prepare_cached(&stmt_sql)?;
        let rows = stmt.query_map(params_from_iter(params), |r| {
            let bm25: f64 = r.get(1)?;
            Ok(Candidate {
                id: sfe_core::FileId(r.get(0)?),
                score: -bm25, // bm25 is negative-is-better
                snippet: r.get::<_, Option<String>>(2)?.filter(|s| !s.is_empty()),
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
}

/// No text to match: return files satisfying the filters, newest first.
pub fn filter_only(conn: &Connection, q: &Query, k: usize) -> Result<Vec<Candidate>> {
    if q.filters.is_empty() {
        return Ok(vec![]);
    }
    let (filter_sql, mut params) = sql::where_clause(&q.filters);
    let stmt_sql = format!(
        "SELECT f.id FROM {from} WHERE {base}{filter_sql} ORDER BY f.mtime DESC, f.id DESC LIMIT ?",
        from = sql::FROM,
        base = sql::BASE,
    );
    params.push(Value::from(k as i64));
    let mut stmt = conn.prepare_cached(&stmt_sql)?;
    let rows = stmt.query_map(params_from_iter(params), |r| {
        Ok(Candidate {
            id: sfe_core::FileId(r.get(0)?),
            score: 0.0,
            snippet: None,
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}
