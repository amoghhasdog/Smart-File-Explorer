//! Content extraction via a plugin/adapter architecture.
//!
//! An [`Extractor`] receives an abstract seekable [`ExtractInput`] (never a
//! path it opens itself) plus [`Limits`], and returns text and a metadata
//! patch. Adapters live in [`adapters`] and are registered in a [`Registry`]
//! that picks the most specific one for a file. [`run_guarded`] wraps an
//! extraction with panic isolation and a wall-clock timeout.

pub mod adapters;
mod input;
mod registry;

use std::time::Duration;

use sfe_core::FileMeta;

pub use input::{ExtractInput, FileInput, MemInput};
pub use registry::Registry;

#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported or malformed {0}")]
    Unsupported(&'static str),
    #[error("file larger than limit ({size} > {max} bytes)")]
    TooLarge { size: u64, max: u64 },
    #[error("parse error: {0}")]
    Parse(String),
    #[error("extractor panicked: {0}")]
    Panic(String),
    #[error("extractor timed out after {0:?}")]
    Timeout(Duration),
}

pub type Result<T> = std::result::Result<T, ExtractError>;

/// Caps every extractor must respect.
#[derive(Debug, Clone)]
pub struct Limits {
    /// Files larger than this are not read at all.
    pub max_bytes: u64,
    /// Extracted text is cut here (and `truncated` set).
    pub max_text_chars: usize,
    /// Images larger than this (in bytes) get dimensions/EXIF only, no
    /// pixel decode for colour statistics.
    pub max_decode_bytes: u64,
    pub timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 256 * 1024 * 1024,
            max_text_chars: 200_000,
            max_decode_bytes: 48 * 1024 * 1024,
            timeout: Duration::from_secs(60),
        }
    }
}

/// What is known about a file before any extractor runs.
#[derive(Debug, Clone, Copy)]
pub struct FileProbe<'a> {
    /// Lowercase, no dot.
    pub ext: &'a str,
    /// Sniffed mime, if any.
    pub mime: Option<&'a str>,
    pub size: u64,
}

/// How confident an adapter is that it handles a file. `ByMagic` beats
/// `ByExt` when several adapters apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Support {
    No,
    ByExt,
    ByMagic,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Extracted {
    pub text: Option<String>,
    pub meta: FileMeta,
    pub truncated: bool,
}

pub trait Extractor: Send + Sync {
    /// Stable identifier stored with each extraction (`file_text.extractor`).
    fn id(&self) -> &'static str;
    /// Bump to force re-extraction of files done with an older version.
    fn version(&self) -> u32;
    fn supports(&self, probe: &FileProbe<'_>) -> Support;
    fn extract(&self, input: &mut dyn ExtractInput, limits: &Limits) -> Result<Extracted>;
}

/// Run `extractor` on `input` on a helper thread, converting panics into
/// errors and giving up after `limits.timeout`. A timed-out thread is
/// detached (it cannot be killed); the caller must treat the file as failed.
pub fn run_guarded(
    extractor: &'static dyn Extractor,
    mut input: Box<dyn ExtractInput>,
    limits: &Limits,
) -> Result<Extracted> {
    if input.size() > limits.max_bytes {
        return Err(ExtractError::TooLarge {
            size: input.size(),
            max: limits.max_bytes,
        });
    }
    let timeout = limits.timeout;
    let max_text_chars = limits.max_text_chars;
    let limits = limits.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name(format!("sfe-extract-{}", extractor.id()))
        .spawn(move || {
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                extractor.extract(input.as_mut(), &limits)
            }));
            let out = match res {
                Ok(r) => r,
                Err(p) => Err(ExtractError::Panic(panic_message(&p))),
            };
            let _ = tx.send(out);
        })?;
    match rx.recv_timeout(timeout) {
        Ok(r) => r.map(|e| truncate(e, max_text_chars)),
        Err(_) => Err(ExtractError::Timeout(timeout)),
    }
}

fn panic_message(p: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".into()
    }
}

/// Cut text to `max_chars` on a char boundary and flag it.
pub fn truncate(mut e: Extracted, max_chars: usize) -> Extracted {
    if let Some(t) = e.text.as_mut()
        && t.chars().count() > max_chars
    {
        let cut = t
            .char_indices()
            .nth(max_chars)
            .map(|(i, _)| i)
            .unwrap_or(t.len());
        t.truncate(cut);
        e.truncated = true;
    }
    e
}

/// Collapse runs of whitespace and drop control characters; keeps newlines
/// as single `\n` so paragraph structure survives for chunking later.
pub(crate) fn normalise_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    let mut pending_newline = false;
    for ch in s.chars() {
        match ch {
            '\n' | '\r' => pending_newline = true,
            c if c.is_whitespace() => pending_space = true,
            c if c.is_control() => {}
            c => {
                if pending_newline {
                    out.push('\n');
                } else if pending_space && !out.is_empty() {
                    out.push(' ');
                }
                pending_newline = false;
                pending_space = false;
                out.push(c);
            }
        }
    }
    out
}

pub(crate) fn word_count(s: &str) -> u32 {
    s.split_whitespace().count().min(u32::MAX as usize) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Panicky;
    impl Extractor for Panicky {
        fn id(&self) -> &'static str {
            "panicky"
        }
        fn version(&self) -> u32 {
            1
        }
        fn supports(&self, _: &FileProbe<'_>) -> Support {
            Support::ByExt
        }
        fn extract(&self, _: &mut dyn ExtractInput, _: &Limits) -> Result<Extracted> {
            panic!("boom");
        }
    }

    struct Slow;
    impl Extractor for Slow {
        fn id(&self) -> &'static str {
            "slow"
        }
        fn version(&self) -> u32 {
            1
        }
        fn supports(&self, _: &FileProbe<'_>) -> Support {
            Support::ByExt
        }
        fn extract(&self, _: &mut dyn ExtractInput, _: &Limits) -> Result<Extracted> {
            std::thread::sleep(Duration::from_secs(5));
            Ok(Extracted::default())
        }
    }

    static PANICKY: Panicky = Panicky;
    static SLOW: Slow = Slow;

    #[test]
    fn guard_converts_panics_and_timeouts() {
        let limits = Limits {
            timeout: Duration::from_millis(200),
            ..Default::default()
        };
        let err =
            run_guarded(&PANICKY, Box::new(MemInput::new(b"x".to_vec())), &limits).unwrap_err();
        assert!(matches!(err, ExtractError::Panic(m) if m == "boom"));
        let err = run_guarded(&SLOW, Box::new(MemInput::new(b"x".to_vec())), &limits).unwrap_err();
        assert!(matches!(err, ExtractError::Timeout(_)));
    }

    #[test]
    fn guard_rejects_oversized_input() {
        let limits = Limits {
            max_bytes: 2,
            ..Default::default()
        };
        let err =
            run_guarded(&SLOW, Box::new(MemInput::new(b"abc".to_vec())), &limits).unwrap_err();
        assert!(matches!(err, ExtractError::TooLarge { size: 3, max: 2 }));
    }

    #[test]
    fn truncate_and_normalise() {
        let e = truncate(
            Extracted {
                text: Some("héllo wörld".into()),
                ..Default::default()
            },
            5,
        );
        assert_eq!(e.text.as_deref(), Some("héllo"));
        assert!(e.truncated);

        assert_eq!(normalise_text("  a \t b\r\n\r\n c\u{0}d  "), "a b\ncd");
        assert_eq!(word_count("one  two\nthree"), 3);
    }
}
