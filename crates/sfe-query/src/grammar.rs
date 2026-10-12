//! Pattern passes over the token list. Each pass marks the tokens it
//! consumes; leftovers become search terms.

use chrono::{DateTime, Local};
use sfe_core::Category;

use crate::ast::{Chip, ChipKind, ColorPred, DateField, Query, Range};
use crate::dates::{self, Qual};
use crate::tokens::{Tok, tokenize};

const KNOWN_EXTS: &[&str] = &[
    "pdf", "doc", "docx", "xls", "xlsx", "csv", "ppt", "pptx", "txt", "md", "rtf", "odt", "epub",
    "jpg", "jpeg", "png", "gif", "bmp", "webp", "tif", "tiff", "heic", "svg", "raw", "psd", "mp4",
    "mkv", "mov", "avi", "webm", "wmv", "m4v", "mp3", "wav", "flac", "aac", "m4a", "ogg", "zip",
    "rar", "7z", "tar", "gz", "iso", "exe", "msi", "dll", "json", "xml", "yaml", "yml", "toml",
    "html", "css", "js", "ts", "tsx", "jsx", "py", "rs", "java", "cs", "cpp", "c", "h", "go", "rb",
    "php", "sql", "sh", "ps1", "ipynb", "log", "ini", "cfg",
];

const STOPWORDS: &[&str] = &[
    "find",
    "search",
    "show",
    "get",
    "list",
    "give",
    "me",
    "all",
    "any",
    "every",
    "some",
    "the",
    "a",
    "an",
    "file",
    "files",
    "that",
    "which",
    "who",
    "with",
    "of",
    "my",
    "i",
    "i'm",
    "have",
    "has",
    "had",
    "contain",
    "contains",
    "containing",
    "and",
    "or",
    "in",
    "on",
    "for",
    "to",
    "where",
    "are",
    "is",
    "were",
    "was",
    "be",
    "please",
    "look",
    "up",
    "stuff",
    "things",
    "something",
    "anything",
    "everything",
    "this",
    "those",
    "these",
    "it",
    "its",
    "there",
    "want",
    "need",
    "like",
    "from",
    "at",
    "by",
    "about",
    "into",
    "than",
    "then",
    "type",
    "kind",
    "sort",
    "called",
    "named",
    "titled",
    "whose",
    "name",
    "names",
    "saved",
    "stored",
];

struct P<'a> {
    toks: Vec<Tok>,
    now: DateTime<Local>,
    q: Query,
    _phantom: std::marker::PhantomData<&'a ()>,
}

pub(crate) fn parse(input: &str, now: DateTime<Local>) -> Query {
    let mut p = P {
        toks: tokenize(input),
        now,
        q: Query {
            raw: input.to_string(),
            ..Default::default()
        },
        _phantom: std::marker::PhantomData,
    };
    p.quoted_phrases();
    p.sizes();
    p.durations();
    p.dimensions();
    p.dates();
    p.types_and_exts();
    p.colors();
    p.paths();
    p.word_phrases();
    p.about();
    p.leftovers();
    p.q
}

impl P<'_> {
    fn t(&self, i: usize) -> Option<&str> {
        self.toks
            .get(i)
            .filter(|t| !t.used && !t.quoted)
            .map(|t| t.text.as_str())
    }

    fn consume(&mut self, i: usize, n: usize) {
        for t in self.toks.iter_mut().skip(i).take(n) {
            t.used = true;
        }
    }

    fn chip(&mut self, kind: ChipKind, label: impl Into<String>) {
        self.q.chips.push(Chip {
            kind,
            label: label.into(),
        });
    }

    // ---------------------------------------------------------------------

    fn quoted_phrases(&mut self) {
        for i in 0..self.toks.len() {
            if self.toks[i].quoted {
                self.toks[i].used = true;
                let p = self.toks[i].orig.clone();
                self.chip(ChipKind::Phrase, format!("\"{p}\""));
                self.q.phrases.push(p);
            }
        }
    }

    fn sizes(&mut self) {
        let mut i = 0;
        while i < self.toks.len() {
            if let Some((cmp, skip)) = self.comparator(i, Dim::Size)
                && let Some((bytes, label, n)) = self.size_at(i + skip)
            {
                let mut r = Range::default();
                match cmp {
                    Cmp::Gt => r.min = Some(bytes),
                    Cmp::Lt => r.max = Some(bytes),
                }
                self.q.filters.size.merge(r);
                self.chip(ChipKind::Size, format!("{} {label}", cmp.sym()));
                self.consume(i, skip + n);
                i += skip + n;
                continue;
            }
            i += 1;
        }
    }

    fn durations(&mut self) {
        let mut i = 0;
        while i < self.toks.len() {
            if let Some((cmp, skip)) = self.comparator(i, Dim::Duration)
                && let Some((ms, label, n)) = self.duration_at(i + skip)
            {
                let mut r = Range::default();
                match cmp {
                    Cmp::Gt => r.min = Some(ms),
                    Cmp::Lt => r.max = Some(ms),
                }
                self.q.filters.duration_ms.merge(r);
                self.chip(ChipKind::Duration, format!("{} {label}", cmp.sym()));
                self.consume(i, skip + n);
                i += skip + n;
                continue;
            }
            i += 1;
        }
    }

    fn dimensions(&mut self) {
        for i in 0..self.toks.len() {
            let Some(w) = self.t(i) else { continue };
            let (w_px, h_px) = match w {
                "4k" | "uhd" => (3840, 2160),
                "1080p" | "fullhd" | "fhd" => (1920, 1080),
                "720p" | "hd" => (1280, 720),
                _ => {
                    let Some((a, b)) = w.split_once(['x', '×']) else {
                        continue;
                    };
                    match (a.parse::<u32>(), b.parse::<u32>()) {
                        (Ok(a), Ok(b)) if a > 0 && b > 0 => (a, b),
                        _ => continue,
                    }
                }
            };
            self.q.filters.min_width = Some(w_px);
            self.q.filters.min_height = Some(h_px);
            self.chip(ChipKind::Dimensions, format!("≥ {w_px}×{h_px}"));
            self.consume(i, 1);
        }
    }

    fn dates(&mut self) {
        let mut i = 0;
        while i < self.toks.len() {
            let start = i;
            let mut field = None;
            let mut j = i;
            if let Some(f) = self.t(j).and_then(date_field) {
                field = Some(f);
                j += 1;
            }
            // "older than 3 months", "within the last 30 days"
            if let Some((r, n)) = dates::relative_bound(&self.toks, j, self.now) {
                self.apply_date(field.unwrap_or(DateField::Modified), r);
                self.consume(start, j - start + n);
                i = j + n;
                continue;
            }
            // "between A and B"
            if self.t(j) == Some("between")
                && let Some((a, na)) = dates::period_at(&self.toks, j + 1, self.now)
                && self.t(j + 1 + na) == Some("and")
                && let Some((b, nb)) = dates::period_at(&self.toks, j + 2 + na, self.now)
            {
                let r = Range {
                    min: Some(a.start),
                    max: Some(b.end),
                };
                self.apply_date(field.unwrap_or(DateField::Modified), r);
                let n = j + 2 + na + nb - start;
                self.consume(start, n);
                i = start + n;
                continue;
            }
            let (qual, nq) = dates::qualifier(&self.toks, j).unwrap_or((Qual::In, 0));
            let k = j + nq;
            // field word may also follow the qualifier: "before the last modified"? no;
            // but "in 2024 modified" is unusual — field after period is handled below.
            if let Some((p, np)) = dates::period_at(&self.toks, k, self.now) {
                // A bare period with no qualifier and no field word must look
                // like a date to count: years, months and relative phrases do;
                // a lone "may" or "march" would otherwise eat real words.
                let bare = nq == 0 && field.is_none();
                let first = self.t(k).unwrap_or("");
                if bare && looks_like_plain_word(first) {
                    i += 1;
                    continue;
                }
                let mut end = k + np;
                if field.is_none()
                    && let Some(f) = self.t(end).and_then(date_field)
                {
                    field = Some(f);
                    end += 1;
                }
                self.apply_date(field.unwrap_or(DateField::Modified), dates::apply(qual, p));
                self.consume(start, end - start);
                i = end;
                continue;
            }
            i += 1;
        }
    }

    fn apply_date(&mut self, field: DateField, r: Range<i64>) {
        let (slot, name) = match field {
            DateField::Modified => (&mut self.q.filters.modified, "modified"),
            DateField::Created => (&mut self.q.filters.created, "created"),
            DateField::Taken => (&mut self.q.filters.taken, "taken"),
        };
        slot.merge(r);
        let label = format!("{name} {}", dates::label(&r));
        self.chip(ChipKind::Date, label);
    }

    fn types_and_exts(&mut self) {
        for i in 0..self.toks.len() {
            let Some(raw) = self.t(i).map(str::to_string) else {
                continue;
            };
            let dotted = raw.starts_with('.');
            let w = raw.trim_start_matches('.');
            // ".pdf" is an explicit extension, never a type word.
            if !dotted && let Some((cat, ext)) = type_word(w) {
                if let Some(cat) = cat
                    && !self.q.filters.categories.contains(&cat)
                {
                    self.q.filters.categories.push(cat);
                    self.chip(ChipKind::Type, cat.as_str());
                }
                if let Some(ext) = ext
                    && !self.q.filters.exts.contains(&ext.to_string())
                {
                    self.q.filters.exts.push(ext.to_string());
                    self.chip(ChipKind::Ext, format!(".{ext}"));
                }
                self.consume(i, 1);
                continue;
            }
            let singular = w.strip_suffix('s').unwrap_or(w);
            let ext = if KNOWN_EXTS.contains(&w) {
                Some(w)
            } else if KNOWN_EXTS.contains(&singular) && w.len() > 2 {
                Some(singular)
            } else {
                None
            };
            if let Some(ext) = ext
                && !self.q.filters.exts.contains(&ext.to_string())
            {
                // Ambiguous short words ("c", "h", "go", "rs") only count with a dot.
                if ext.len() <= 2 && !dotted {
                    continue;
                }
                self.q.filters.exts.push(ext.to_string());
                self.chip(ChipKind::Ext, format!(".{ext}"));
                self.consume(i, 1);
            }
        }
    }

    fn colors(&mut self) {
        let image_ctx = self.q.filters.categories.contains(&Category::Image);
        let mut i = 0;
        while i < self.toks.len() {
            let mut j = i;
            let mut intens = false;
            // "a lot of", "lots of", "mostly", "mainly", "very", "predominantly", "dominant"
            if self.t(j) == Some("a") && self.t(j + 1) == Some("lot") && self.t(j + 2) == Some("of")
            {
                intens = true;
                j += 3;
            } else if self.t(j) == Some("lots") && self.t(j + 1) == Some("of") {
                intens = true;
                j += 2;
            } else if matches!(
                self.t(j),
                Some("mostly" | "mainly" | "very" | "predominantly" | "dominant" | "dominated")
            ) {
                intens = true;
                j += 1;
                if self.t(j) == Some("by") {
                    j += 1;
                }
            }
            let Some(w) = self.t(j) else {
                i += 1;
                continue;
            };
            let Some((name, bins)) = color_word(w) else {
                i += 1;
                continue;
            };
            let explicit = self.t(j + 1).is_some_and(|n| {
                matches!(
                    n,
                    "color" | "colour" | "colors" | "colours" | "tones" | "tint" | "hue"
                )
            });
            if !(intens || image_ctx || explicit) {
                i += 1;
                continue;
            }
            let mut n = j - i + 1;
            if explicit {
                n += 1;
            }
            self.q.filters.color = Some(ColorPred {
                name: name.to_string(),
                bins: bins.to_vec(),
                min_share: if intens { 102 } else { 51 }, // 40% / 20%
            });
            if !self.q.filters.categories.contains(&Category::Image) {
                self.q.filters.categories.push(Category::Image);
                self.chip(ChipKind::Type, "image");
            }
            self.chip(
                ChipKind::Color,
                if intens {
                    format!("mostly {name}")
                } else {
                    name.to_string()
                },
            );
            self.consume(i, n);
            i += n;
        }
    }

    fn paths(&mut self) {
        // "in folder X" / "in the X folder" / "under X" / "inside X"
        let mut i = 0;
        while i < self.toks.len() {
            let mut j = i;
            let mut hit = false;
            if matches!(self.t(j), Some("in" | "under" | "inside" | "within")) {
                j += 1;
                if self.t(j) == Some("the") {
                    j += 1;
                }
                if matches!(self.t(j), Some("folder" | "directory" | "dir" | "path")) {
                    j += 1;
                    hit = true;
                } else if matches!(self.t(j + 1), Some("folder" | "directory" | "dir")) {
                    let name = self.toks[j].orig.clone();
                    self.q.filters.path_contains.push(name.clone());
                    self.chip(ChipKind::Path, format!("in {name}"));
                    self.consume(i, j + 2 - i);
                    i = j + 2;
                    continue;
                }
            }
            if hit && let Some(_name) = self.t(j) {
                let name = self.toks[j].orig.clone();
                self.q.filters.path_contains.push(name.clone());
                self.chip(ChipKind::Path, format!("in {name}"));
                self.consume(i, j + 1 - i);
                i = j + 1;
                continue;
            }
            i += 1;
        }
    }

    fn word_phrases(&mut self) {
        // "containing the word X" · "with the words X Y" · "mentioning X" · "that say X"
        let mut i = 0;
        while i < self.toks.len() {
            let mut j = i;
            let mut multi = false;
            let lead = match self.t(j) {
                Some(
                    "containing" | "contains" | "contain" | "with" | "mentioning" | "mentions"
                    | "mention" | "including" | "includes" | "saying" | "says" | "say" | "having",
                ) => true,
                Some("that")
                    if matches!(
                        self.t(j + 1),
                        Some(
                            "contain"
                                | "contains"
                                | "mention"
                                | "mentions"
                                | "say"
                                | "says"
                                | "include"
                                | "includes"
                                | "have"
                                | "has"
                        )
                    ) =>
                {
                    j += 1;
                    true
                }
                _ => false,
            };
            if !lead {
                i += 1;
                continue;
            }
            j += 1;
            if self.t(j) == Some("the") {
                j += 1;
            }
            match self.t(j) {
                Some("word" | "term" | "text" | "string") => j += 1,
                Some("words" | "terms" | "phrase") => {
                    multi = true;
                    j += 1;
                }
                _ => {
                    // "mentioning X" without "the word": still a phrase of one token
                    if !matches!(
                        self.t(i),
                        Some("mentioning" | "mentions" | "mention" | "saying" | "says" | "say")
                    ) {
                        i += 1;
                        continue;
                    }
                }
            }
            let mut words = Vec::new();
            let mut k = j;
            while let Some(_w) = self.t(k) {
                let w = &self.toks[k];
                if w.text == "and" && multi {
                    k += 1;
                    continue;
                }
                words.push(w.orig.clone());
                k += 1;
                if !multi {
                    break;
                }
            }
            if words.is_empty() {
                i += 1;
                continue;
            }
            let phrase = words.join(" ");
            self.chip(ChipKind::Phrase, format!("\"{phrase}\""));
            self.q.phrases.push(phrase);
            self.consume(i, k - i);
            i = k;
        }
    }

    fn about(&mut self) {
        for i in 0..self.toks.len() {
            let (lead, n) = match (self.t(i), self.t(i + 1), self.t(i + 2)) {
                (Some("about" | "regarding" | "concerning" | "re"), ..) => (true, 1),
                (Some("related"), Some("to"), _) | (Some("relating"), Some("to"), _) => (true, 2),
                (Some("on"), Some("the"), Some("topic")) => (true, 4),
                _ => (false, 0),
            };
            if !lead {
                continue;
            }
            let mut words: Vec<(usize, String)> = Vec::new();
            let mut k = i + n;
            while self.t(k).is_some() {
                words.push((k, self.toks[k].orig.clone()));
                k += 1;
            }
            // Trailing filler ("...machine learning I") is not part of the topic.
            while words
                .last()
                .is_some_and(|(idx, _)| STOPWORDS.contains(&self.toks[*idx].text.as_str()))
            {
                words.pop();
            }
            if words.is_empty() {
                continue;
            }
            let topic = words
                .iter()
                .map(|(_, w)| w.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            self.chip(ChipKind::About, format!("about {topic}"));
            self.q.semantic = Some(topic);
            self.consume(i, k - i);
            return;
        }
    }

    fn leftovers(&mut self) {
        let mut terms = Vec::new();
        for t in &self.toks {
            if t.used || t.quoted || STOPWORDS.contains(&t.text.as_str()) {
                continue;
            }
            terms.push(t.orig.clone());
        }
        if !terms.is_empty() {
            self.chip(ChipKind::Terms, terms.join(" "));
            if self.q.semantic.is_none() {
                self.q.semantic = Some(terms.join(" "));
            }
            self.q.terms = terms;
        }
    }

    // ---------------------------------------------------------------------

    fn comparator(&self, i: usize, dim: Dim) -> Option<(Cmp, usize)> {
        let t = |k| self.t(i + k);
        Some(match (t(0)?, t(1), t(2)) {
            (">" | "over" | "above" | "exceeding", ..) => (Cmp::Gt, 1),
            ("<" | "under" | "below", ..) => (Cmp::Lt, 1),
            ("more" | "larger" | "bigger" | "greater" | "heavier", Some("than"), _)
                if dim == Dim::Size =>
            {
                (Cmp::Gt, 2)
            }
            ("less" | "smaller" | "lighter", Some("than"), _) if dim == Dim::Size => (Cmp::Lt, 2),
            ("longer" | "more" | "greater", Some("than"), _) if dim == Dim::Duration => {
                (Cmp::Gt, 2)
            }
            ("shorter" | "less", Some("than"), _) if dim == Dim::Duration => (Cmp::Lt, 2),
            ("at", Some("least"), _) | ("minimum", ..) | ("min", ..) => {
                (Cmp::Gt, if t(0) == Some("at") { 2 } else { 1 })
            }
            ("at", Some("most"), _) | ("maximum", ..) | ("max", ..) => {
                (Cmp::Lt, if t(0) == Some("at") { 2 } else { 1 })
            }
            ("bigger" | "larger" | "big" | "large" | "huge", ..) if dim == Dim::Size => {
                (Cmp::Gt, 1)
            }
            ("smaller" | "small" | "tiny", ..) if dim == Dim::Size => (Cmp::Lt, 1),
            ("longer", ..) if dim == Dim::Duration => (Cmp::Gt, 1),
            ("shorter", ..) if dim == Dim::Duration => (Cmp::Lt, 1),
            _ => return None,
        })
    }

    /// `10 mb` · `10mb` · `1.5 gb` → bytes, label, tokens consumed.
    fn size_at(&self, i: usize) -> Option<(u64, String, usize)> {
        let (num, unit, n) = self.number_unit(i)?;
        let mult: u64 = match unit.as_str() {
            "b" | "byte" | "bytes" => 1,
            "k" | "kb" | "kib" | "kilobyte" | "kilobytes" => 1 << 10,
            "m" | "mb" | "mib" | "meg" | "megs" | "megabyte" | "megabytes" => 1 << 20,
            "g" | "gb" | "gib" | "gig" | "gigs" | "gigabyte" | "gigabytes" => 1 << 30,
            "t" | "tb" | "tib" | "terabyte" | "terabytes" => 1 << 40,
            _ => return None,
        };
        let bytes = (num * mult as f64).round() as u64;
        let label = format!("{} {}", trim_num(num), unit_label_size(mult));
        Some((bytes, label, n))
    }

    /// `10 minutes` · `1.5 h` · `90s` → milliseconds.
    fn duration_at(&self, i: usize) -> Option<(u64, String, usize)> {
        let (num, unit, n) = self.number_unit(i)?;
        let (mult, name) = match unit.as_str() {
            "s" | "sec" | "secs" | "second" | "seconds" => (1_000.0, "s"),
            "m" | "min" | "mins" | "minute" | "minutes" => (60_000.0, "min"),
            "h" | "hr" | "hrs" | "hour" | "hours" => (3_600_000.0, "h"),
            _ => return None,
        };
        let ms = (num * mult).round() as u64;
        // "10 minutes long" — swallow the trailer
        let n = if self.t(i + n) == Some("long") {
            n + 1
        } else {
            n
        };
        Some((ms, format!("{} {name}", trim_num(num)), n))
    }

    /// Number followed by a unit, either as two tokens or glued (`10mb`).
    /// `a`/`an` count as 1 ("over an hour").
    fn number_unit(&self, i: usize) -> Option<(f64, String, usize)> {
        let w = self.t(i)?;
        if w == "a" || w == "an" {
            let unit = self.t(i + 1)?;
            return Some((1.0, unit.to_string(), 2));
        }
        let w = w.replace(',', "");
        let split = w
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(w.len());
        let (num, unit) = w.split_at(split);
        let num: f64 = num.parse().ok()?;
        if unit.is_empty() {
            let unit = self.t(i + 1)?;
            Some((num, unit.to_string(), 2))
        } else {
            Some((num, unit.to_string(), 1))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dim {
    Size,
    Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cmp {
    Gt,
    Lt,
}

impl Cmp {
    fn sym(self) -> &'static str {
        match self {
            Cmp::Gt => "≥",
            Cmp::Lt => "≤",
        }
    }
}

fn trim_num(n: f64) -> String {
    if n.fract() == 0.0 {
        format!("{}", n as u64)
    } else {
        format!("{n}")
    }
}

fn unit_label_size(mult: u64) -> &'static str {
    match mult {
        1 => "B",
        1024 => "KB",
        1_048_576 => "MB",
        1_073_741_824 => "GB",
        _ => "TB",
    }
}

fn date_field(w: &str) -> Option<DateField> {
    Some(match w {
        "modified" | "edited" | "changed" | "updated" | "touched" => DateField::Modified,
        "created" | "added" | "downloaded" | "made" | "written" | "received" | "new" => {
            DateField::Created
        }
        "taken" | "shot" | "captured" | "photographed" | "recorded" | "filmed" => DateField::Taken,
        _ => return None,
    })
}

/// Words that `period_at` could read as a date but which are more likely
/// ordinary words without a qualifier or field word ("may", "march").
fn looks_like_plain_word(w: &str) -> bool {
    matches!(w, "may" | "march" | "a" | "an")
}

fn type_word(w: &str) -> Option<(Option<Category>, Option<&'static str>)> {
    Some(match w {
        "image" | "images" | "photo" | "photos" | "picture" | "pictures" | "pic" | "pics"
        | "screenshot" | "screenshots" | "wallpaper" | "wallpapers" | "scan" | "scans"
        | "graphic" | "graphics" => (Some(Category::Image), None),
        "video" | "videos" | "movie" | "movies" | "clip" | "clips" | "film" | "films"
        | "recording" | "recordings" | "footage" => (Some(Category::Video), None),
        "audio" | "song" | "songs" | "music" | "track" | "tracks" | "podcast" | "podcasts"
        | "sound" | "sounds" | "voice" | "memo" | "memos" => (Some(Category::Audio), None),
        "document" | "documents" | "doc" | "docs" | "paper" | "papers" | "report" | "reports"
        | "note" | "notes" | "text" | "texts" | "writeup" | "ebook" | "ebooks" => {
            (Some(Category::Document), None)
        }
        "pdf" | "pdfs" => (Some(Category::Document), Some("pdf")),
        "spreadsheet" | "spreadsheets" | "excel" | "sheet" | "sheets" | "workbook" => {
            (Some(Category::Document), Some("xlsx"))
        }
        "presentation" | "presentations" | "powerpoint" | "powerpoints" | "slides" | "deck"
        | "decks" => (Some(Category::Document), Some("pptx")),
        "word" => return None, // handled by "the word X"
        "code" | "source" | "script" | "scripts" | "program" | "programs" | "snippet"
        | "snippets" => (Some(Category::Code), None),
        "archive" | "archives" | "zip" | "zips" | "compressed" => (Some(Category::Archive), None),
        _ => return None,
    })
}

fn color_word(w: &str) -> Option<(&'static str, &'static [u8])> {
    Some(match w {
        "red" | "reds" | "reddish" | "crimson" | "scarlet" => ("red", &[0, 11]),
        "orange" | "amber" => ("orange", &[1]),
        "yellow" | "gold" | "golden" => ("yellow", &[2]),
        "green" | "greens" | "greenish" | "lime" => ("green", &[3, 4]),
        "cyan" | "teal" | "turquoise" | "aqua" => ("cyan", &[5, 6]),
        "blue" | "blues" | "bluish" | "navy" | "azure" => ("blue", &[7, 8]),
        "purple" | "violet" | "indigo" | "lavender" => ("purple", &[9]),
        "magenta" | "pink" | "fuchsia" => ("pink", &[10]),
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
