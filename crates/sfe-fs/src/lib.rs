//! Filesystem access boundary.
//!
//! Every `std::fs` / directory-walk / change-watch call in the workspace lives
//! here. Other crates receive paths, readers and change events from this crate
//! and never open files themselves. Implemented in Phase 2.
