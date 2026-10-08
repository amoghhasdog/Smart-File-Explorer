//! Local-first embeddings.
//!
//! `TextEmbedder` / `ImageEmbedder` traits with fastembed (ONNX Runtime)
//! implementations, plus a model manager that downloads pinned, checksummed
//! models only after explicit user consent. Implemented in Phase 5.
