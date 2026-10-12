//! Semantic text strategy: embed the query, find nearest chunks in the ANN
//! index, map to files, apply the structured filters, keep the best chunk
//! score per file.

use std::collections::HashMap;
use std::sync::Arc;

use sfe_core::{ChunkId, FileId};
use sfe_db::chunks;
use sfe_db::rusqlite::{Connection, params_from_iter, types::Value};
use sfe_embed::TextEmbedder;
use sfe_query::Query;
use sfe_vector::VectorIndex;

use crate::{Candidate, Result, SearchStrategy, sql};

pub struct SemanticText {
    embedder: Arc<dyn TextEmbedder>,
    index: Arc<dyn VectorIndex>,
}

impl SemanticText {
    pub fn new(embedder: Arc<dyn TextEmbedder>, index: Arc<dyn VectorIndex>) -> Self {
        Self { embedder, index }
    }

    /// The text to embed: an explicit topic, else the loose terms.
    fn query_text(q: &Query) -> Option<String> {
        q.semantic
            .clone()
            .or_else(|| (!q.terms.is_empty()).then(|| q.terms.join(" ")))
            .filter(|s| !s.trim().is_empty())
    }
}

impl SearchStrategy for SemanticText {
    fn name(&self) -> &'static str {
        "semantic-text"
    }

    fn applies(&self, q: &Query) -> bool {
        Self::query_text(q).is_some() && !self.index.is_empty()
    }

    fn run(&self, conn: &Connection, q: &Query, k: usize) -> Result<Vec<Candidate>> {
        let Some(text) = Self::query_text(q) else {
            return Ok(vec![]);
        };
        let qv = match self.embedder.embed_query(&text) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "query embedding failed");
                return Ok(vec![]);
            }
        };
        // Several chunks per file: over-fetch so k distinct files survive.
        let neighbours = match self.index.search(&qv, (k * 4).max(50)) {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!(error = %e, "vector search failed");
                return Ok(vec![]);
            }
        };
        if neighbours.is_empty() {
            return Ok(vec![]);
        }
        let ids: Vec<ChunkId> = neighbours.iter().map(|n| ChunkId(n.key as i64)).collect();
        let mapping = chunks::file_ids(conn, &ids)?;
        let chunk_to_file: HashMap<ChunkId, FileId> = mapping.into_iter().collect();

        // Best score per file.
        let mut best: HashMap<FileId, f64> = HashMap::new();
        for n in &neighbours {
            if let Some(f) = chunk_to_file.get(&ChunkId(n.key as i64)) {
                let e = best.entry(*f).or_insert(f64::MIN);
                *e = e.max(n.score as f64);
            }
        }
        if best.is_empty() {
            return Ok(vec![]);
        }

        // Keep only files that satisfy the filters (and are live).
        let file_ids: Vec<FileId> = best.keys().copied().collect();
        let (filter_sql, mut params) = sql::where_clause(&q.filters);
        let marks = vec!["?"; file_ids.len()].join(", ");
        let stmt_sql = format!(
            "SELECT f.id FROM {from} WHERE {base}{filter_sql} AND f.id IN ({marks})",
            from = sql::FROM,
            base = sql::BASE,
        );
        params.extend(file_ids.iter().map(|f| Value::from(f.0)));
        let mut stmt = conn.prepare_cached(&stmt_sql)?;
        let allowed: Vec<FileId> = stmt
            .query_map(params_from_iter(params), |r| r.get::<_, i64>(0).map(FileId))?
            .collect::<std::result::Result<_, _>>()?;

        let mut out: Vec<Candidate> = allowed
            .into_iter()
            .map(|id| Candidate {
                id,
                score: best[&id],
                snippet: None,
            })
            .collect();
        out.sort_by(|a, b| b.score.total_cmp(&a.score));
        out.truncate(k);
        Ok(out)
    }
}
