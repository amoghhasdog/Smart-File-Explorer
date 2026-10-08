//! Search execution.
//!
//! Runs the filter stage, keyword (FTS5/bm25), semantic-text and
//! semantic-image strategies concurrently and fuses them with reciprocal rank
//! fusion. Each strategy implements a common `SearchStrategy` trait so new
//! ones can be added without touching the others. Implemented in Phase 4–6.
