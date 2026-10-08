//! Indexing pipeline.
//!
//! A durable job queue drives files through cheap stages first (discover,
//! metadata) across the whole corpus before expensive ones (text, embed), so
//! name/metadata search is available within minutes even for 1M files.
//! Implemented from Phase 2 onward.
