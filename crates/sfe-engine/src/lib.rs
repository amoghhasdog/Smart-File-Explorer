//! Application engine façade.
//!
//! Owns the database, indexing pipeline, embedders and search orchestrator and
//! exposes a small async API that the Tauri shell (and integration tests) call.
//! Implemented incrementally from Phase 1.
