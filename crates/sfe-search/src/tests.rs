use std::sync::Arc;

use chrono::{Local, NaiveDate, TimeZone};
use sfe_core::{DiscoveredFile, FileId, FileKind, FileMeta, RootId};
use sfe_db::{Database, files, fts, meta, roots};

use crate::Searcher;

struct Seed {
    _dir: tempfile::TempDir,
    searcher: Searcher,
    ids: Vec<FileId>,
}

fn disc(root: RootId, path: &str, size: u64, mtime: i64) -> DiscoveredFile {
    let (parent, name) = path.rsplit_once('/').unwrap();
    DiscoveredFile {
        root_id: root,
        path: path.into(),
        parent_path: parent.into(),
        name: name.into(),
        ext: name
            .rsplit_once('.')
            .map(|(_, e)| e.to_lowercase())
            .unwrap_or_default(),
        kind: FileKind::File,
        size,
        mtime,
        ctime: Some(mtime),
    }
}

fn day(y: i32, m: u32, d: u32) -> i64 {
    Local
        .from_local_datetime(
            &NaiveDate::from_ymd_opt(y, m, d)
                .unwrap()
                .and_hms_opt(10, 0, 0)
                .unwrap(),
        )
        .unwrap()
        .timestamp()
}

fn seed() -> Seed {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path().join("s.db")).unwrap());
    let ids = db
        .transaction(|c| {
            let root = roots::add(c, "C:/r")?;
            let batch = [
                disc(root.id, "C:/r/docs/ml-intro.pdf", 3 << 20, day(2025, 6, 1)),
                disc(
                    root.id,
                    "C:/r/docs/ferrari-history.docx",
                    200 << 10,
                    day(2026, 2, 1),
                ),
                disc(root.id, "C:/r/photos/sea.png", 1 << 20, day(2025, 8, 15)),
                disc(root.id, "C:/r/photos/sunset.jpg", 2 << 20, day(2026, 9, 1)),
                disc(root.id, "C:/r/video/talk.mp4", 500 << 20, day(2026, 1, 10)),
                disc(root.id, "C:/r/code/parser.rs", 4096, day(2026, 10, 1)),
            ];
            let ids: Vec<FileId> = files::upsert_discovered(c, &batch)?
                .into_iter()
                .map(|u| u.id)
                .collect();
            fts::set(
                c,
                ids[0],
                "ml-intro.pdf",
                "An introduction to machine learning and neural networks.",
            )?;
            fts::set(
                c,
                ids[1],
                "ferrari-history.docx",
                "The Ferrari company was founded by Enzo Ferrari.",
            )?;
            fts::set(c, ids[2], "sea.png", "")?;
            fts::set(c, ids[3], "sunset.jpg", "")?;
            fts::set(c, ids[4], "talk.mp4", "")?;
            fts::set(
                c,
                ids[5],
                "parser.rs",
                "fn parse(input: &str) -> Query { machine }",
            )?;
            let mut blue = vec![0u8; 12];
            blue[7] = 120;
            blue[8] = 120;
            meta::upsert(
                c,
                ids[2],
                &FileMeta {
                    width: Some(1920),
                    height: Some(1080),
                    hue_hist: Some(blue),
                    ..Default::default()
                },
            )?;
            let mut orange = vec![0u8; 12];
            orange[1] = 200;
            orange[0] = 55;
            meta::upsert(
                c,
                ids[3],
                &FileMeta {
                    width: Some(800),
                    height: Some(600),
                    hue_hist: Some(orange),
                    taken_at: Some(day(2026, 8, 30)),
                    ..Default::default()
                },
            )?;
            meta::upsert(
                c,
                ids[4],
                &FileMeta {
                    duration_ms: Some(15 * 60 * 1000),
                    width: Some(1280),
                    height: Some(720),
                    ..Default::default()
                },
            )?;
            Ok(ids)
        })
        .unwrap();
    Seed {
        _dir: dir,
        searcher: Searcher::new(db),
        ids,
    }
}

fn now() -> chrono::DateTime<Local> {
    Local
        .from_local_datetime(
            &NaiveDate::from_ymd_opt(2026, 10, 11)
                .unwrap()
                .and_hms_opt(12, 0, 0)
                .unwrap(),
        )
        .unwrap()
}

fn run(s: &Seed, input: &str) -> Vec<FileId> {
    let q = sfe_query::parse(input, now());
    s.searcher
        .search(&q, 50, 0)
        .unwrap()
        .hits
        .into_iter()
        .map(|h| h.id)
        .collect()
}

#[test]
fn keyword_search_with_snippets_and_prefixes() {
    let s = seed();
    let q = sfe_query::parse("ferr", now());
    let r = s.searcher.search(&q, 50, 0).unwrap();
    assert_eq!(r.hits.len(), 1);
    assert_eq!(r.hits[0].name, "ferrari-history.docx");
    assert!(r.hits[0].snippet.as_deref().unwrap().contains("[Ferrari]"));
    assert!(!r.has_more);
    assert_eq!(r.query.terms, vec!["ferr"]);
}

#[test]
fn example_queries_from_the_brief() {
    let s = seed();
    assert_eq!(
        run(&s, "find all images that contain a lot of blue"),
        vec![s.ids[2]]
    );
    assert_eq!(
        run(&s, "find documents containing the word Ferrari"),
        vec![s.ids[1]]
    );
    assert_eq!(
        run(
            &s,
            "videos longer than 10 minutes created before February 2026"
        ),
        vec![s.ids[4]]
    );
    assert_eq!(
        run(&s, "PDFs about machine learning I downloaded last year"),
        vec![s.ids[0]]
    );
    // "about" words act as keywords until semantic search exists; the .rs file also says "machine".
    let mut ml = run(&s, "about machine");
    ml.sort();
    assert_eq!(ml, vec![s.ids[0], s.ids[5]]);
}

#[test]
fn filter_only_queries_are_newest_first() {
    let s = seed();
    assert_eq!(run(&s, "images"), vec![s.ids[3], s.ids[2]]);
    assert_eq!(run(&s, "files over 100 MB"), vec![s.ids[4]]);
    assert_eq!(run(&s, "photos taken in august 2026"), vec![s.ids[3]]);
    assert_eq!(run(&s, "modified this month"), vec![s.ids[5]]);
    assert_eq!(run(&s, "1080p pictures"), vec![s.ids[2]]);
    assert_eq!(run(&s, "orange photos"), vec![s.ids[3]]);
    assert!(
        run(&s, "find all files").is_empty(),
        "empty query returns nothing"
    );
}

#[test]
fn types_and_extensions_are_alternatives() {
    let s = seed();
    let mut r = run(&s, ".pdf or .rs files");
    r.sort();
    let mut expected = vec![s.ids[0], s.ids[5]];
    expected.sort();
    assert_eq!(r, expected);
}

#[test]
fn paging() {
    let s = seed();
    let q = sfe_query::parse("images", now());
    let p1 = s.searcher.search(&q, 1, 0).unwrap();
    assert_eq!(p1.hits.len(), 1);
    assert!(p1.has_more);
    let p2 = s.searcher.search(&q, 1, 1).unwrap();
    assert_eq!(p2.hits.len(), 1);
    assert!(!p2.has_more);
    assert_ne!(p1.hits[0].id, p2.hits[0].id);
    let r = s.searcher.search(&q, 1, 0).unwrap();
    assert_eq!(r.hits[0].width, Some(800));
}
