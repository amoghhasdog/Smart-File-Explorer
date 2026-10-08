//! Approximate nearest-neighbour vector index.
//!
//! SQLite (`chunks.vec`) is the source of truth; the HNSW index here is a
//! rebuildable, memory-mapped cache keyed by `ChunkId`. Implemented in Phase 5.
