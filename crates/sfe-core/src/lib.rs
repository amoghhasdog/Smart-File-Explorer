//! Core domain types shared by every other `sfe-*` crate.
//!
//! This crate performs **no IO**: no filesystem, no database, no network.
//! It defines ids, the file/metadata model, configuration, and the traits
//! that the pipeline and search layers are built on.

pub mod ids;

pub use ids::{ChunkId, FileId, RootId};
