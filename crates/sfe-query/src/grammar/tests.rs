use chrono::{DateTime, Local, NaiveDate, TimeZone};
use sfe_core::Category;

use crate::ast::{ChipKind, DateField, Range};
use crate::dates::ts;
use crate::parse;

fn now() -> DateTime<Local> {
    Local
        .from_local_datetime(
            &NaiveDate::from_ymd_opt(2026, 10, 11)
                .unwrap()
                .and_hms_opt(12, 0, 0)
                .unwrap(),
        )
        .unwrap()
}

fn d(y: i32, m: u32, day: u32) -> i64 {
    ts(NaiveDate::from_ymd_opt(y, m, day).unwrap())
}

fn chips(q: &crate::Query, kind: ChipKind) -> Vec<&str> {
    q.chips
        .iter()
        .filter(|c| c.kind == kind)
        .map(|c| c.label.as_str())
        .collect()
}

#[test]
fn images_with_a_lot_of_blue() {
    let q = parse("find all images that contain a lot of blue", now());
    assert_eq!(q.filters.categories, vec![Category::Image]);
    let c = q.filters.color.clone().unwrap();
    assert_eq!(c.name, "blue");
    assert_eq!(c.bins, vec![7, 8]);
    assert_eq!(c.min_share, 102);
    assert!(q.terms.is_empty(), "{:?}", q.terms);
    assert!(q.is_filter_only());
    assert_eq!(chips(&q, ChipKind::Color), vec!["mostly blue"]);
}

#[test]
fn documents_containing_the_word() {
    let q = parse("find documents containing the word Ferrari", now());
    assert_eq!(q.filters.categories, vec![Category::Document]);
    assert_eq!(q.phrases, vec!["Ferrari"]);
    assert!(q.terms.is_empty());
    assert_eq!(chips(&q, ChipKind::Phrase), vec!["\"Ferrari\""]);
}

#[test]
fn related_to_project() {
    let q = parse("find files related to my Athena project", now());
    assert_eq!(q.semantic.as_deref(), Some("my Athena project"));
    assert!(q.filters.is_empty());
    assert_eq!(chips(&q, ChipKind::About), vec!["about my Athena project"]);
}

#[test]
fn videos_longer_than_created_before() {
    let q = parse(
        "find videos longer than 10 minutes created before January 2026",
        now(),
    );
    assert_eq!(q.filters.categories, vec![Category::Video]);
    assert_eq!(
        q.filters.duration_ms,
        Range {
            min: Some(600_000),
            max: None
        }
    );
    assert_eq!(
        q.filters.created,
        Range {
            min: None,
            max: Some(d(2026, 1, 1))
        }
    );
    assert!(q.filters.modified.is_empty());
    assert!(q.is_filter_only(), "{q:?}");
    assert_eq!(chips(&q, ChipKind::Duration), vec!["≥ 10 min"]);
}

#[test]
fn screenshots_containing_python_code() {
    let q = parse("find screenshots containing Python code", now());
    assert_eq!(q.filters.categories, vec![Category::Image, Category::Code]);
    assert_eq!(q.terms, vec!["Python"]);
    assert_eq!(q.semantic.as_deref(), Some("Python"));
}

#[test]
fn pdfs_about_ml_downloaded_last_year() {
    let q = parse(
        "find PDFs about machine learning I downloaded last year",
        now(),
    );
    assert_eq!(q.filters.exts, vec!["pdf"]);
    assert_eq!(q.filters.categories, vec![Category::Document]);
    assert_eq!(
        q.filters.created,
        Range {
            min: Some(d(2025, 1, 1)),
            max: Some(d(2026, 1, 1))
        }
    );
    assert_eq!(q.semantic.as_deref(), Some("machine learning"));
    assert!(q.terms.is_empty(), "{:?}", q.terms);
}

#[test]
fn sizes_in_many_spellings() {
    for (input, min, max) in [
        ("files over 10 mb", Some(10 << 20), None),
        ("larger than 1.5gb", Some(1_610_612_736), None),
        ("under 200 KB", None, Some(200 << 10)),
        ("at least 1 gig", Some(1 << 30), None),
        ("> 5 MB < 50 MB", Some(5 << 20), Some(50 << 20)),
        ("bigger than 1,000 bytes", Some(1000), None),
    ] {
        let q = parse(input, now());
        assert_eq!(q.filters.size, Range { min, max }, "{input}");
        assert!(q.terms.is_empty(), "{input}: {:?}", q.terms);
    }
}

#[test]
fn durations_and_ambiguous_m() {
    let q = parse("songs shorter than 3 min", now());
    assert_eq!(q.filters.duration_ms.max, Some(180_000));
    assert_eq!(q.filters.categories, vec![Category::Audio]);
    let q = parse("clips over an hour long", now());
    assert_eq!(q.filters.duration_ms.min, Some(3_600_000));
    // "over 10 m" is a size (megabytes) because "over" is a size comparator first.
    let q = parse("files over 10 m", now());
    assert_eq!(q.filters.size.min, Some(10 << 20));
    assert!(q.filters.duration_ms.is_empty());
}

#[test]
fn date_expressions() {
    let n = now();
    let cases: Vec<(&str, DateField, Range<i64>)> = vec![
        (
            "modified yesterday",
            DateField::Modified,
            Range {
                min: Some(d(2026, 10, 10)),
                max: Some(d(2026, 10, 11)),
            },
        ),
        (
            "edited today",
            DateField::Modified,
            Range {
                min: Some(d(2026, 10, 11)),
                max: Some(d(2026, 10, 12)),
            },
        ),
        (
            "from last week",
            DateField::Modified,
            Range {
                min: Some(d(2026, 9, 28)),
                max: Some(d(2026, 10, 5)),
            },
        ),
        (
            "this month",
            DateField::Modified,
            Range {
                min: Some(d(2026, 10, 1)),
                max: Some(d(2026, 11, 1)),
            },
        ),
        (
            "in 2024",
            DateField::Modified,
            Range {
                min: Some(d(2024, 1, 1)),
                max: Some(d(2025, 1, 1)),
            },
        ),
        (
            "after 2024",
            DateField::Modified,
            Range {
                min: Some(d(2025, 1, 1)),
                max: None,
            },
        ),
        (
            "since 2024",
            DateField::Modified,
            Range {
                min: Some(d(2024, 1, 1)),
                max: None,
            },
        ),
        (
            "before jan 2026",
            DateField::Modified,
            Range {
                min: None,
                max: Some(d(2026, 1, 1)),
            },
        ),
        (
            "until jan 2026",
            DateField::Modified,
            Range {
                min: None,
                max: Some(d(2026, 2, 1)),
            },
        ),
        (
            "in february",
            DateField::Modified,
            Range {
                min: Some(d(2026, 2, 1)),
                max: Some(d(2026, 3, 1)),
            },
        ),
        (
            "in november",
            DateField::Modified,
            Range {
                min: Some(d(2025, 11, 1)),
                max: Some(d(2025, 12, 1)),
            },
        ),
        (
            "taken on 2025-07-04",
            DateField::Taken,
            Range {
                min: Some(d(2025, 7, 4)),
                max: Some(d(2025, 7, 5)),
            },
        ),
        (
            "created on march 5 2024",
            DateField::Created,
            Range {
                min: Some(d(2024, 3, 5)),
                max: Some(d(2024, 3, 6)),
            },
        ),
        (
            "between 2020 and 2022",
            DateField::Modified,
            Range {
                min: Some(d(2020, 1, 1)),
                max: Some(d(2023, 1, 1)),
            },
        ),
        (
            "older than 2 years",
            DateField::Modified,
            Range {
                min: None,
                max: Some(d(2024, 10, 11)),
            },
        ),
        (
            "within the last 30 days",
            DateField::Modified,
            Range {
                min: Some(d(2026, 9, 11)),
                max: None,
            },
        ),
        (
            "last 3 months",
            DateField::Modified,
            Range {
                min: Some(d(2026, 7, 11)),
                max: Some(d(2026, 10, 12)),
            },
        ),
        (
            "2 weeks ago",
            DateField::Modified,
            Range {
                min: Some(d(2026, 9, 27)),
                max: Some(d(2026, 10, 12)),
            },
        ),
        (
            "photos shot last year",
            DateField::Taken,
            Range {
                min: Some(d(2025, 1, 1)),
                max: Some(d(2026, 1, 1)),
            },
        ),
    ];
    for (input, field, expected) in cases {
        let q = parse(input, n);
        let got = match field {
            DateField::Modified => q.filters.modified,
            DateField::Created => q.filters.created,
            DateField::Taken => q.filters.taken,
        };
        assert_eq!(got, expected, "{input}: {q:?}");
        assert!(q.terms.is_empty(), "{input}: leftover {:?}", q.terms);
    }
}

#[test]
fn ambiguous_words_stay_terms() {
    let q = parse("notes from the march meeting", now());
    assert!(q.filters.modified.is_empty(), "{q:?}");
    assert_eq!(q.terms, vec!["march", "meeting"]);
    let q = parse("may invoice", now());
    assert!(q.filters.modified.is_empty());
    assert_eq!(q.terms, vec!["may", "invoice"]);
    // A bare colour outside image context is just a word.
    let q = parse("red car brochure", now());
    assert!(q.filters.color.is_none());
    assert_eq!(q.terms, vec!["red", "car", "brochure"]);
    // ...but with an image type it is a colour filter.
    let q = parse("red photos", now());
    assert_eq!(q.filters.color.unwrap().name, "red");
    assert!(q.terms.is_empty());
}

#[test]
fn extensions_and_type_words() {
    let q = parse("png or jpg screenshots in the Downloads folder", now());
    assert_eq!(q.filters.exts, vec!["png", "jpg"]);
    assert_eq!(q.filters.categories, vec![Category::Image]);
    assert_eq!(q.filters.path_contains, vec!["Downloads"]);
    let q = parse("rust code about parsing", now());
    assert_eq!(q.filters.categories, vec![Category::Code]);
    assert_eq!(q.semantic.as_deref(), Some("parsing"));
    assert_eq!(q.terms, vec!["rust"]);
    let q = parse("spreadsheets .rs files", now());
    assert_eq!(q.filters.exts, vec!["xlsx", "rs"]);
    // a dotted extension is only an extension, not a type word
    let q = parse(".pdf files", now());
    assert_eq!(q.filters.exts, vec!["pdf"]);
    assert!(q.filters.categories.is_empty());
    // bare "rs" without a dot is not an extension
    let q = parse("rs", now());
    assert!(q.filters.exts.is_empty());
}

#[test]
fn quoted_phrases_and_dimensions() {
    let q = parse("\"quarterly report\" 4k wallpapers 1920x1080", now());
    assert_eq!(q.phrases, vec!["quarterly report"]);
    assert_eq!(q.filters.min_width, Some(1920));
    assert_eq!(q.filters.categories, vec![Category::Image]);
    assert_eq!(chips(&q, ChipKind::Dimensions).len(), 2);
}

#[test]
fn empty_and_plain_queries() {
    assert!(parse("", now()).is_empty());
    assert!(parse("find all the files", now()).is_empty());
    let q = parse("ferrari", now());
    assert_eq!(q.terms, vec!["ferrari"]);
    assert_eq!(q.semantic.as_deref(), Some("ferrari"));
    assert!(!q.is_filter_only());
    assert_eq!(q.raw, "ferrari");
}
