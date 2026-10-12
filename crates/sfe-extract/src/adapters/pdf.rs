//! PDF text via `pdf-extract` (pure Rust). Scanned PDFs yield no text (no
//! OCR in MVP). Swappable for `pdfium-render` behind the same trait if the
//! failure rate on real corpora turns out too high.

use sfe_core::FileMeta;

use crate::{
    ExtractError, ExtractInput, Extracted, Extractor, FileProbe, Limits, Result, Support,
    normalise_text, word_count,
};

pub struct Pdf;

impl Extractor for Pdf {
    fn id(&self) -> &'static str {
        "pdf"
    }

    fn version(&self) -> u32 {
        1
    }

    fn supports(&self, probe: &FileProbe<'_>) -> Support {
        match (probe.mime, probe.ext) {
            (Some("application/pdf"), _) => Support::ByMagic,
            (_, "pdf") => Support::ByExt,
            _ => Support::No,
        }
    }

    fn extract(&self, input: &mut dyn ExtractInput, _limits: &Limits) -> Result<Extracted> {
        let bytes = input.read_all()?;
        if !bytes.starts_with(b"%PDF") {
            return Err(ExtractError::Unsupported("not a PDF"));
        }
        let text = pdf_extract::extract_text_from_mem(&bytes)
            .map_err(|e| ExtractError::Parse(e.to_string()))?;
        let text = normalise_text(&text);
        Ok(Extracted {
            meta: FileMeta {
                page_count: Some(count_pages(&bytes)),
                word_count: Some(word_count(&text)),
                ..Default::default()
            },
            text: Some(text),
            truncated: false,
        })
    }
}

/// Count page objects by scanning for `/Type /Page` (not `/Pages`). Good
/// enough for metadata; object streams in compressed PDFs may hide some.
fn count_pages(bytes: &[u8]) -> u32 {
    let mut n = 0u32;
    let mut i = 0;
    while let Some(pos) = find(&bytes[i..], b"/Type") {
        let mut j = i + pos + 5;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if bytes[j..].starts_with(b"/Page") && !bytes[j..].starts_with(b"/Pages") {
            n += 1;
        }
        i = j.max(i + pos + 1);
    }
    n
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::MemInput;

    /// A minimal single-page, uncompressed PDF with Helvetica text.
    pub(crate) fn minimal_pdf(text: &str) -> Vec<u8> {
        let content = format!("BT /F1 24 Tf 72 700 Td ({text}) Tj ET");
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes(),
        );
        for off in offsets {
            out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objs.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    #[test]
    fn extracts_text_and_page_count() {
        let pdf = minimal_pdf("Machine learning for Ferrari engines");
        let e = Pdf
            .extract(&mut MemInput::new(pdf), &Limits::default())
            .unwrap();
        let t = e.text.unwrap();
        assert!(t.contains("Ferrari"), "got {t:?}");
        assert_eq!(e.meta.page_count, Some(1));
        assert!(e.meta.word_count.unwrap() >= 5);
    }

    #[test]
    fn rejects_non_pdf() {
        let err = Pdf
            .extract(&mut MemInput::new(b"hello".to_vec()), &Limits::default())
            .unwrap_err();
        assert!(matches!(err, ExtractError::Unsupported(_)));
    }
}
