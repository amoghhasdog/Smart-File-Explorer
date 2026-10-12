//! Search execution.
//!
//! A parsed [`Query`] runs through one or more [`SearchStrategy`]s whose
//! ranked candidate lists are fused with reciprocal rank fusion
//! ([`fusion::rrf`]). Structured [`sfe_query::Filters`] are pushed down into
//! SQL ([`sql`]) so every strategy only ever sees matching files.
//!
//! Phase 4 ships the keyword strategy (FTS5/bm25) and the filter-only path;
//! Phase 5 adds semantic text, Phase 6 semantic image.

pub mod fusion;
mod hydrate;
mod keyword;
pub mod sql;

use std::sync::Arc;

use serde::Serialize;
use sfe_core::{Category, FileId};
use sfe_db::Database;
use sfe_query::Query;

pub use hydrate::Hit;

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error(transparent)]
    Db(#[from] sfe_db::DbError),
    #[error("sqlite: {0}")]
    Sqlite(#[from] sfe_db::rusqlite::Error),
}

pub type Result<T> = std::result::Result<T, SearchError>;

/// One ranked candidate from a strategy, before fusion.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub id: FileId,
    /// Higher is better (strategies normalise their own scale).
    pub score: f64,
    pub snippet: Option<String>,
}

/// A source of ranked candidates for a query.
pub trait SearchStrategy: Send + Sync {
    fn name(&self) -> &'static str;
    /// Whether this strategy has anything to contribute for `q`.
    fn applies(&self, q: &Query) -> bool;
    /// Top `k` candidates satisfying the query's filters.
    fn run(
        &self,
        conn: &sfe_db::rusqlite::Connection,
        q: &Query,
        k: usize,
    ) -> Result<Vec<Candidate>>;
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    pub query: Query,
    pub hits: Vec<Hit>,
    pub took_ms: u64,
    /// `true` when more results may exist beyond `offset + hits.len()`.
    pub has_more: bool,
}

pub struct Searcher {
    db: Arc<Database>,
    strategies: Vec<Box<dyn SearchStrategy>>,
}

impl Searcher {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            db,
            strategies: vec![Box::new(keyword::Keyword)],
        }
    }

    pub fn add_strategy(&mut self, s: Box<dyn SearchStrategy>) {
        self.strategies.push(s);
    }

    /// Run `q` and return a page of hydrated hits.
    pub fn search(&self, q: &Query, limit: usize, offset: usize) -> Result<SearchResponse> {
        let started = std::time::Instant::now();
        let conn = self.db.reader()?;
        let limit = limit.clamp(1, 500);
        // Fetch one extra to know whether another page exists.
        let want = offset + limit + 1;

        let ranked: Vec<Candidate> = if q.is_filter_only() {
            keyword::filter_only(&conn, q, want)?
        } else {
            let mut lists = Vec::new();
            let mut snippets = std::collections::HashMap::new();
            for s in &self.strategies {
                if !s.applies(q) {
                    continue;
                }
                let cands = s.run(&conn, q, want.max(200))?;
                for c in &cands {
                    if let Some(sn) = &c.snippet {
                        snippets.entry(c.id).or_insert_with(|| sn.clone());
                    }
                }
                lists.push(cands.into_iter().map(|c| c.id).collect::<Vec<_>>());
            }
            fusion::rrf(&lists, fusion::DEFAULT_K)
                .into_iter()
                .map(|(id, score)| Candidate {
                    id,
                    score,
                    snippet: snippets.remove(&id),
                })
                .collect()
        };

        let has_more = ranked.len() > offset + limit;
        let page: Vec<Candidate> = ranked.into_iter().skip(offset).take(limit).collect();
        let hits = hydrate::hydrate(&conn, &page)?;
        Ok(SearchResponse {
            query: q.clone(),
            hits,
            took_ms: started.elapsed().as_millis() as u64,
            has_more,
        })
    }
}

/// Convenience for callers that only need categories as strings.
pub fn category_names(cats: &[Category]) -> Vec<&'static str> {
    cats.iter().map(|c| c.as_str()).collect()
}

#[cfg(test)]
mod tests;
