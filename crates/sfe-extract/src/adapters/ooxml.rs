//! Office Open XML (docx / xlsx / pptx): zipped XML. Text is gathered from
//! the relevant parts; title/author from `docProps/core.xml`.

use std::io::{Read, Seek};

use quick_xml::events::Event;
use sfe_core::FileMeta;

use crate::{
    ExtractError, ExtractInput, Extracted, Extractor, FileProbe, Limits, Result, Support,
    normalise_text, word_count,
};

pub struct Docx;
pub struct Xlsx;
pub struct Pptx;

const OOXML_MIME_PREFIX: &str = "application/vnd.openxmlformats-officedocument";

fn support(probe: &FileProbe<'_>, ext: &str, mime_suffix: &str) -> Support {
    match probe.mime {
        Some(m) if m.starts_with(OOXML_MIME_PREFIX) && m.ends_with(mime_suffix) => Support::ByMagic,
        // `infer` reports a generic zip/docx guess for all OOXML; trust the extension then.
        Some(m)
            if (m == "application/zip" || m.starts_with(OOXML_MIME_PREFIX)) && probe.ext == ext =>
        {
            Support::ByExt
        }
        Some(_) => Support::No,
        None if probe.ext == ext => Support::ByExt,
        None => Support::No,
    }
}

impl Extractor for Docx {
    fn id(&self) -> &'static str {
        "docx"
    }
    fn version(&self) -> u32 {
        1
    }
    fn supports(&self, probe: &FileProbe<'_>) -> Support {
        support(probe, "docx", "wordprocessingml.document")
    }
    fn extract(&self, input: &mut dyn ExtractInput, limits: &Limits) -> Result<Extracted> {
        extract_parts(input, limits, &["word/document.xml"], &["t"], &["p"])
    }
}

impl Extractor for Xlsx {
    fn id(&self) -> &'static str {
        "xlsx"
    }
    fn version(&self) -> u32 {
        1
    }
    fn supports(&self, probe: &FileProbe<'_>) -> Support {
        support(probe, "xlsx", "spreadsheetml.sheet")
    }
    fn extract(&self, input: &mut dyn ExtractInput, limits: &Limits) -> Result<Extracted> {
        // Shared strings hold most cell text; inline strings live in sheets.
        extract_parts(
            input,
            limits,
            &["xl/sharedStrings.xml", "xl/worksheets/"],
            &["t"],
            &["si", "row"],
        )
    }
}

impl Extractor for Pptx {
    fn id(&self) -> &'static str {
        "pptx"
    }
    fn version(&self) -> u32 {
        1
    }
    fn supports(&self, probe: &FileProbe<'_>) -> Support {
        support(probe, "pptx", "presentationml.presentation")
    }
    fn extract(&self, input: &mut dyn ExtractInput, limits: &Limits) -> Result<Extracted> {
        extract_parts(input, limits, &["ppt/slides/slide"], &["t"], &["p"])
    }
}

/// Pull text nodes under `text_tags` from every zip entry whose name starts
/// with one of `part_prefixes`. A closing `break_tags` element emits a newline.
fn extract_parts(
    input: &mut dyn ExtractInput,
    limits: &Limits,
    part_prefixes: &[&str],
    text_tags: &[&str],
    break_tags: &[&str],
) -> Result<Extracted> {
    let mut zip = zip::ZipArchive::new(ReadSeek(input))
        .map_err(|e| ExtractError::Parse(format!("zip: {e}")))?;
    let mut names: Vec<String> = zip
        .file_names()
        .filter_map(|n| n.ok())
        .filter(|n| part_prefixes.iter().any(|p| n.starts_with(p)) && n.ends_with(".xml"))
        .map(|n| n.into_owned())
        .collect();
    if names.is_empty() {
        return Err(ExtractError::Unsupported("no document parts in package"));
    }
    names.sort_by_key(|n| natural_key(n));

    let max_bytes = limits.max_text_chars * 4;
    let mut text = String::new();
    let mut truncated = false;
    'parts: for name in &names {
        let mut part = zip
            .by_name(name)
            .map_err(|e| ExtractError::Parse(format!("zip entry {name}: {e}")))?;
        let mut xml = Vec::new();
        part.read_to_end(&mut xml)?;
        let mut reader = quick_xml::Reader::from_reader(xml.as_slice());
        let mut depth_in_text = 0usize;
        loop {
            match reader.read_event() {
                Ok(Event::Eof) => break,
                Ok(Event::Start(e)) => {
                    if tag_in(e.local_name().as_ref(), text_tags) {
                        depth_in_text += 1;
                    }
                }
                Ok(Event::End(e)) => {
                    let n = e.local_name();
                    if tag_in(n.as_ref(), text_tags) {
                        depth_in_text = depth_in_text.saturating_sub(1);
                    } else if tag_in(n.as_ref(), break_tags) {
                        text.push('\n');
                    }
                }
                Ok(Event::Text(t)) if depth_in_text > 0 => {
                    text.push_str(&t);
                    if text.len() > max_bytes {
                        truncated = true;
                        break 'parts;
                    }
                }
                Ok(Event::GeneralRef(r)) if depth_in_text > 0 => {
                    if let Some(c) = resolve_ref(&r) {
                        text.push(c);
                    }
                }
                Ok(_) => {}
                Err(e) => return Err(ExtractError::Parse(format!("xml in {name}: {e}"))),
            }
        }
        text.push('\n');
    }
    let text = normalise_text(&text);
    let mut meta = FileMeta {
        word_count: Some(word_count(&text)),
        ..Default::default()
    };
    if let Ok(mut core) = zip.by_name("docProps/core.xml") {
        let mut xml = Vec::new();
        if core.read_to_end(&mut xml).is_ok() {
            let (title, author) = core_props(&xml);
            meta.title = title;
            meta.author = author;
        }
    }
    Ok(Extracted {
        text: Some(text),
        meta,
        truncated,
    })
}

/// Resolve `&amp;`-style and numeric character references.
fn resolve_ref(r: &quick_xml::events::BytesRef<'_>) -> Option<char> {
    if let Ok(Some(c)) = r.resolve_char_ref() {
        return Some(c);
    }
    match &**r {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => None,
    }
}

fn tag_in(name: &str, tags: &[&str]) -> bool {
    tags.contains(&name)
}

/// Sort `slide2.xml` before `slide10.xml`.
fn natural_key(s: &str) -> (String, u64) {
    let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    let prefix: String = s.chars().filter(|c| !c.is_ascii_digit()).collect();
    (prefix, digits.parse().unwrap_or(0))
}

fn core_props(xml: &[u8]) -> (Option<String>, Option<String>) {
    let mut reader = quick_xml::Reader::from_reader(xml);
    let (mut title, mut author) = (None, None);
    let mut current: Option<&'static str> = None;
    loop {
        match reader.read_event() {
            Ok(Event::Eof) | Err(_) => break,
            Ok(Event::Start(e)) => {
                current = match e.local_name().as_ref() {
                    "title" => Some("title"),
                    "creator" => Some("creator"),
                    _ => None,
                }
            }
            Ok(Event::End(_)) => current = None,
            Ok(Event::Text(t)) => {
                let s = t.trim().to_string();
                if s.is_empty() {
                    continue;
                }
                match current {
                    Some("title") => title = Some(s),
                    Some("creator") => author = Some(s),
                    _ => {}
                }
            }
            Ok(_) => {}
        }
    }
    (title, author)
}

/// Adapter so `zip` (which wants `Read + Seek`) can borrow our trait object.
struct ReadSeek<'a>(&'a mut dyn ExtractInput);

impl Read for ReadSeek<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Seek for ReadSeek<'_> {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::{Cursor, Write};

    use super::*;
    use crate::MemInput;

    pub(crate) fn package(parts: &[(&str, &str)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        for (name, body) in parts {
            w.start_file(*name, opts).unwrap();
            w.write_all(body.as_bytes()).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    const CORE: &str = r#"<?xml version="1.0"?><cp:coreProperties xmlns:cp="x" xmlns:dc="y"><dc:title>Athena Roadmap</dc:title><dc:creator>Amogh</dc:creator></cp:coreProperties>"#;

    #[test]
    fn docx_text_and_core_props() {
        let doc = package(&[
            (
                "word/document.xml",
                r#"<w:document xmlns:w="w"><w:body><w:p><w:r><w:t>Hello</w:t></w:r><w:r><w:t xml:space="preserve"> Ferrari &amp; co</w:t></w:r></w:p><w:p><w:r><w:t>Second para</w:t></w:r></w:p></w:body></w:document>"#,
            ),
            ("docProps/core.xml", CORE),
        ]);
        let e = Docx
            .extract(&mut MemInput::new(doc), &Limits::default())
            .unwrap();
        assert_eq!(e.text.as_deref(), Some("Hello Ferrari & co\nSecond para"));
        assert_eq!(e.meta.title.as_deref(), Some("Athena Roadmap"));
        assert_eq!(e.meta.author.as_deref(), Some("Amogh"));
        assert_eq!(e.meta.word_count, Some(6));
    }

    #[test]
    fn xlsx_shared_and_inline_strings() {
        let doc = package(&[
            (
                "xl/sharedStrings.xml",
                r#"<sst><si><t>Revenue</t></si><si><r><t>Q1</t></r><r><t> 2026</t></r></si></sst>"#,
            ),
            (
                "xl/worksheets/sheet1.xml",
                r#"<worksheet><sheetData><row><c t="inlineStr"><is><t>Inline cell</t></is></c><c><v>42</v></c></row></sheetData></worksheet>"#,
            ),
        ]);
        let e = Xlsx
            .extract(&mut MemInput::new(doc), &Limits::default())
            .unwrap();
        assert_eq!(e.text.as_deref(), Some("Revenue\nQ1 2026\nInline cell"));
    }

    #[test]
    fn pptx_slides_in_natural_order() {
        let doc = package(&[
            (
                "ppt/slides/slide10.xml",
                r#"<p:sld><a:p><a:r><a:t>Ten</a:t></a:r></a:p></p:sld>"#,
            ),
            (
                "ppt/slides/slide2.xml",
                r#"<p:sld><a:p><a:r><a:t>Two</a:t></a:r></a:p></p:sld>"#,
            ),
            ("ppt/slides/_rels/slide2.xml.rels", "<r/>"),
        ]);
        let e = Pptx
            .extract(&mut MemInput::new(doc), &Limits::default())
            .unwrap();
        assert_eq!(e.text.as_deref(), Some("Two\nTen"));
    }

    #[test]
    fn rejects_zip_without_parts_and_non_zip() {
        let doc = package(&[("other.xml", "<x/>")]);
        assert!(matches!(
            Docx.extract(&mut MemInput::new(doc), &Limits::default()),
            Err(ExtractError::Unsupported(_))
        ));
        assert!(matches!(
            Docx.extract(&mut MemInput::new(b"nope".to_vec()), &Limits::default()),
            Err(ExtractError::Parse(_))
        ));
    }

    #[test]
    fn support_resolution() {
        let p = |ext, mime| FileProbe { ext, mime, size: 1 };
        assert_eq!(
            Docx.supports(&p("docx", Some("application/zip"))),
            Support::ByExt
        );
        assert_eq!(
            Docx.supports(&p(
                "bin",
                Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document")
            )),
            Support::ByMagic
        );
        assert_eq!(
            Docx.supports(&p("xlsx", Some("application/zip"))),
            Support::No
        );
        assert_eq!(Xlsx.supports(&p("xlsx", None)), Support::ByExt);
    }
}
