//! Plain text and source code. Handles UTF-8/UTF-16 BOMs, falls back to
//! Windows-1252 for legacy files, and refuses binary data.

use std::io::Read;

use sfe_core::FileMeta;

use crate::{
    ExtractError, ExtractInput, Extracted, Extractor, FileProbe, Limits, Result, Support,
    normalise_text, word_count,
};

pub struct Plain;

const TEXT_EXTS: &[&str] = &[
    "txt",
    "text",
    "md",
    "markdown",
    "rst",
    "log",
    "csv",
    "tsv",
    "json",
    "jsonl",
    "xml",
    "yaml",
    "yml",
    "toml",
    "ini",
    "cfg",
    "conf",
    "env",
    "html",
    "htm",
    "css",
    "scss",
    "less",
    "js",
    "mjs",
    "cjs",
    "jsx",
    "ts",
    "tsx",
    "rs",
    "py",
    "java",
    "kt",
    "kts",
    "c",
    "h",
    "cpp",
    "cc",
    "cxx",
    "hpp",
    "hh",
    "cs",
    "go",
    "rb",
    "php",
    "swift",
    "scala",
    "sh",
    "bash",
    "zsh",
    "ps1",
    "psm1",
    "bat",
    "cmd",
    "sql",
    "lua",
    "r",
    "m",
    "dart",
    "zig",
    "vue",
    "svelte",
    "tex",
    "bib",
    "ipynb",
    "gradle",
    "properties",
    "srt",
    "vtt",
    "sub",
    "nfo",
    "diff",
    "patch",
    "makefile",
    "dockerfile",
    "gitignore",
    "editorconfig",
    "graphql",
    "proto",
    "cmake",
    "pl",
    "pm",
    "ex",
    "exs",
    "erl",
    "hs",
    "ml",
    "clj",
    "nim",
    "v",
    "sv",
    "vhd",
    "asm",
    "s",
];

impl Extractor for Plain {
    fn id(&self) -> &'static str {
        "plain"
    }

    fn version(&self) -> u32 {
        1
    }

    fn supports(&self, probe: &FileProbe<'_>) -> Support {
        if probe
            .mime
            .is_some_and(|m| m.starts_with("text/") || m == "application/json")
        {
            return Support::ByMagic;
        }
        if TEXT_EXTS.contains(&probe.ext) {
            Support::ByExt
        } else {
            Support::No
        }
    }

    fn extract(&self, input: &mut dyn ExtractInput, limits: &Limits) -> Result<Extracted> {
        // Read at most what we could keep: 4 bytes/char worst case, plus slack.
        let cap = (limits.max_text_chars as u64)
            .saturating_mul(4)
            .saturating_add(1024)
            .min(input.size());
        let mut bytes = Vec::with_capacity(cap as usize);
        input.take(cap).read_to_end(&mut bytes)?;
        let text = decode(&bytes).ok_or(ExtractError::Unsupported("binary data"))?;
        let text = normalise_text(&text);
        Ok(Extracted {
            meta: FileMeta {
                word_count: Some(word_count(&text)),
                ..Default::default()
            },
            truncated: cap < input.size(),
            text: Some(text),
        })
    }
}

/// Decode bytes as text. Returns `None` for binary content.
pub(crate) fn decode(bytes: &[u8]) -> Option<String> {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return Some(String::from_utf8_lossy(rest).into_owned());
    }
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        let (s, _, _) = if bytes.starts_with(&[0xFF, 0xFE]) {
            encoding_rs::UTF_16LE.decode(&bytes[2..])
        } else {
            encoding_rs::UTF_16BE.decode(&bytes[2..])
        };
        return Some(s.into_owned());
    }
    let sample = &bytes[..bytes.len().min(8192)];
    if sample.contains(&0) {
        return None;
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => Some(s.to_string()),
        Err(_) => {
            // Could be a truncated multi-byte sequence at the end, or a legacy
            // single-byte encoding. Lossy UTF-8 first, else Windows-1252.
            let lossy = String::from_utf8_lossy(bytes);
            let bad = lossy.chars().filter(|&c| c == '\u{FFFD}').count();
            if bad * 100 < lossy.chars().count().max(1) {
                Some(lossy.into_owned())
            } else {
                let (s, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
                Some(s.into_owned())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemInput;

    fn run(bytes: &[u8], limits: &Limits) -> Result<Extracted> {
        Plain.extract(&mut MemInput::new(bytes.to_vec()), limits)
    }

    #[test]
    fn decodes_utf8_bom_utf16_and_cp1252() {
        let l = Limits::default();
        assert_eq!(
            run("plain ascii".as_bytes(), &l).unwrap().text.as_deref(),
            Some("plain ascii")
        );
        assert_eq!(
            run(b"\xEF\xBB\xBFbom", &l).unwrap().text.as_deref(),
            Some("bom")
        );
        let mut utf16 = vec![0xFF, 0xFE];
        for u in "héllo".encode_utf16() {
            utf16.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(run(&utf16, &l).unwrap().text.as_deref(), Some("héllo"));
        assert_eq!(run(b"caf\xE9", &l).unwrap().text.as_deref(), Some("café"));
    }

    #[test]
    fn rejects_binary_and_counts_words() {
        let l = Limits::default();
        assert!(matches!(
            run(b"abc\0def", &l),
            Err(ExtractError::Unsupported(_))
        ));
        let e = run(b"one two\n\nthree", &l).unwrap();
        assert_eq!(e.meta.word_count, Some(3));
        assert_eq!(e.text.as_deref(), Some("one two\nthree"));
    }

    #[test]
    fn reads_only_what_the_limit_allows() {
        let l = Limits {
            max_text_chars: 4,
            ..Default::default()
        };
        let big = "x".repeat(10_000);
        let e = run(big.as_bytes(), &l).unwrap();
        assert!(e.truncated);
        assert!(e.text.unwrap().len() <= 4 * 4 + 1024);
    }

    #[test]
    fn support_levels() {
        assert_eq!(
            Plain.supports(&FileProbe {
                ext: "rs",
                mime: None,
                size: 1
            }),
            Support::ByExt
        );
        assert_eq!(
            Plain.supports(&FileProbe {
                ext: "weird",
                mime: Some("text/x-thing"),
                size: 1
            }),
            Support::ByMagic
        );
        assert_eq!(
            Plain.supports(&FileProbe {
                ext: "exe",
                mime: None,
                size: 1
            }),
            Support::No
        );
    }
}
