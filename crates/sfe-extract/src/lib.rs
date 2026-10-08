//! Content extraction via a plugin/adapter architecture.
//!
//! An `Extractor` receives an abstract seekable input (never a path it opens
//! itself) plus limits, and returns text + a metadata patch. Adapters are
//! registered in a registry ordered by specificity. Implemented in Phase 3.
