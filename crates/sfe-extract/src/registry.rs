use crate::adapters;
use crate::{Extractor, FileProbe, Support};

/// Ordered set of adapters. [`Registry::find`] returns the adapter with the
/// strongest [`Support`]; ties go to registration order, so register more
/// specific adapters first.
pub struct Registry {
    extractors: Vec<&'static dyn Extractor>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::with_defaults()
    }
}

impl Registry {
    pub fn empty() -> Self {
        Self { extractors: vec![] }
    }

    /// All built-in adapters.
    pub fn with_defaults() -> Self {
        let mut r = Self::empty();
        r.register(&adapters::pdf::Pdf);
        r.register(&adapters::ooxml::Docx);
        r.register(&adapters::ooxml::Xlsx);
        r.register(&adapters::ooxml::Pptx);
        r.register(&adapters::image::Image);
        r.register(&adapters::media::Media);
        r.register(&adapters::plain::Plain);
        r
    }

    pub fn register(&mut self, e: &'static dyn Extractor) {
        self.extractors.push(e);
    }

    pub fn find(&self, probe: &FileProbe<'_>) -> Option<&'static dyn Extractor> {
        let mut best: Option<(Support, &'static dyn Extractor)> = None;
        for e in &self.extractors {
            let s = e.supports(probe);
            if s == Support::No {
                continue;
            }
            if best.as_ref().is_none_or(|(bs, _)| s > *bs) {
                best = Some((s, *e));
            }
        }
        best.map(|(_, e)| e)
    }

    pub fn supports(&self, probe: &FileProbe<'_>) -> bool {
        self.find(probe).is_some()
    }

    pub fn ids(&self) -> Vec<&'static str> {
        self.extractors.iter().map(|e| e.id()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe<'a>(ext: &'a str, mime: Option<&'a str>) -> FileProbe<'a> {
        FileProbe {
            ext,
            mime,
            size: 10,
        }
    }

    #[test]
    fn picks_by_magic_over_extension() {
        let r = Registry::with_defaults();
        // Claims .txt but the sniffed mime says PDF → pdf wins.
        assert_eq!(
            r.find(&probe("txt", Some("application/pdf"))).unwrap().id(),
            "pdf"
        );
        assert_eq!(r.find(&probe("md", None)).unwrap().id(), "plain");
        assert_eq!(r.find(&probe("docx", None)).unwrap().id(), "docx");
        assert_eq!(r.find(&probe("xlsx", None)).unwrap().id(), "xlsx");
        assert_eq!(r.find(&probe("pptx", None)).unwrap().id(), "pptx");
        assert_eq!(
            r.find(&probe("png", Some("image/png"))).unwrap().id(),
            "image"
        );
        assert_eq!(
            r.find(&probe("mp4", Some("video/mp4"))).unwrap().id(),
            "media"
        );
        assert!(
            r.find(&probe(
                "exe",
                Some("application/vnd.microsoft.portable-executable")
            ))
            .is_none()
        );
        assert!(r.find(&probe("", None)).is_none());
    }
}
