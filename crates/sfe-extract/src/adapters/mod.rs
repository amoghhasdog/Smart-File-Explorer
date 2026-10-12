//! Built-in extractor adapters. Each is a zero-sized type with a `static`
//! instance so the registry can hold `&'static dyn Extractor`.

pub mod image;
pub mod media;
pub mod ooxml;
pub mod pdf;
pub mod plain;
