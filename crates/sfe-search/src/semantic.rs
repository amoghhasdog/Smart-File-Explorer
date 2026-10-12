//! Semantic strategies: embed the query, find nearest chunks in an ANN
//! index, map to files, apply the structured filters, keep the best chunk
//! score per file. One instance per modality (text chunks, image vectors).

use std::collections::HashMap;
use std::sync::Arc;

use sfe_core::{Category, ChunkId, FileId};
use sfe_db::chunks;
use sfe_db::rusqlite::{Connection, params_from_iter, types::Value};
use sfe_embed::{ImageEmbedder, TextEmbedder};
use sfe_query::Query;
use sfe_vector::VectorIndex;

use crate::{Candidate, Result, SearchStrategy, sql};

type QueryEmbedFn = Box<dyn Fn(&str) -> sfe_embed::Result<Vec<f32>> + Send + Sync>;

pub struct Semantic {
    name: &'static str,
    embed_query: QueryEmbedFn,
    index: Arc<dyn VectorIndex>,
    /// Only consult this strategy when images are in scope.
    images_only: bool,
}

impl Semantic {
    /// Text chunks embedded with a text model.
    pub fn text(embedder: Arc<dyn TextEmbedder>, index: Arc<dyn VectorIndex>) -> Self {
        Self {
            name: "semantic-text",
            embed_query: Box::new(move |q| embedder.embed_query(q)),
            index,
            images_only: false,
        }
    }

    /// Images embedded with a joint image/text model; queries go through
    /// its text tower.
    pub fn image(embedder: Arc<dyn ImageEmbedder>, index: Arc<dyn VectorIndex>) -> Self {
        Self {
            name: "semantic-image",
            embed_query: Box::new(move |q| embedder.embed_text(q)),
            index,
            images_only: true,
        }
    }

    /// The text to embed: an explicit topic, else the loose terms, else —
    /// for images — the colour name ("a lot of blue" alone is a fine CLIP query).
    fn query_text(&self, q: &Query) -> Option<String> {
        q.semantic
            .clone()
            .or_else(|| (!q.terms.is_empty()).then(|| q.terms.join(" ")))
            .or_else(|| {
                self.images_only
                    .then(|| {
                        q.filters
                            .color
                            .as_ref()
                            .map(|c| format!("{} colour", c.name))
                    })
                    .flatten()
            })
            .filter(|s| !s.trim().is_empty())
    }
}

/// Backwards-compatible aliases.
pub type SemanticText = Semantic;

impl SearchStrategy for Semantic {
    fn name(&self) -> &'static str {
        self.name
    }

    fn applies(&self, q: &Query) -> bool {
        if self.index.is_empty() || self.query_text(q).is_none() {
            return false;
        }
        if self.images_only {
            q.filters.categories.is_empty() || q.filters.categories.contains(&Category::Image)
        } else {
            true
        }
    }

    fn run(&self, conn: &Connection, q: &Query, k: usize) -> Result<Vec<Candidate>> {
        let Some(text) = self.query_text(q) else {
            return Ok(vec![]);
        };
        let qv = match (self.embed_query)(&text) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(strategy = self.name, error = %e, "query embedding failed");
                return Ok(vec![]);
            }
        };
        // Several chunks per file: over-fetch so k distinct files survive.
        let neighbours = match self.index.search(&qv, (k * 4).max(50)) {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!(strategy = self.name, error = %e, "vector search failed");
                return Ok(vec![]);
            }
        };
        if neighbours.is_empty() {
            return Ok(vec![]);
        }
        let ids: Vec<ChunkId> = neighbours.iter().map(|n| ChunkId(n.key as i64)).collect();
        let chunk_to_file: HashMap<ChunkId, FileId> =
            chunks::file_ids(conn, &ids)?.into_iter().collect();

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
