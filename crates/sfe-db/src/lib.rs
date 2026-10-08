//! SQLite persistence: schema migrations, FTS5 table, and typed repositories.
//!
//! Only this crate issues SQL. Writers go through a single writer connection;
//! readers use WAL-mode read connections. Schema is defined in
//! `docs/ARCHITECTURE.md` §4 and implemented in Phase 1.
