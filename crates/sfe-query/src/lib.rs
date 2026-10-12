//! Natural-language query parsing.
//!
//! A deterministic grammar turns free text such as
//! `"PDFs about machine learning downloaded last year over 2 MB"` into a
//! [`Query`]: structured [`Filters`] plus the words that remain for keyword
//! and semantic search. Every recognised fragment also becomes a [`Chip`] so
//! the UI can show users how their query was understood.
//!
//! The parser never fails: anything it does not understand is kept as a
//! search term.

mod ast;
mod dates;
mod grammar;
mod tokens;

pub use ast::{Chip, ChipKind, ColorPred, DateField, Filters, Query, Range};

use chrono::{DateTime, Local};

/// Parse `input` relative to `now` (for "yesterday", "last year", …).
pub fn parse(input: &str, now: DateTime<Local>) -> Query {
    grammar::parse(input, now)
}

/// Parse relative to the current local time.
pub fn parse_now(input: &str) -> Query {
    parse(input, Local::now())
}
