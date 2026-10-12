//! Pipeline integration tests against a real temp directory and database.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use sfe_core::{IndexState, Root};
use sfe_db::{Database, files, fts, roots};

use crate::{Indexer, IndexerConfig, ScanOutcome};

struct Fixture {
    _data: tempfile::TempDir,
    tree: tempfile::TempDir,
    db: Arc<Database>,
    indexer: Arc<Indexer>,
    root: Root,
    workers: Vec<tokio::task::JoinHandle<()>>,
}

impl Fixture {
    async fn finish(self) {
        self.indexer.shutdown();
        for h in self.workers {
            h.await.unwrap();
        }
    }
}

fn fixture() -> Fixture {
    let data = tempfile::tempdir().unwrap();
    let tree = tempfile::tempdir().unwrap();
    let db = Arc::new(Database::open(data.path().join("i.db")).unwrap());
    let cfg = IndexerConfig {
        batch_size: 3,
        workers: 2,
        ..Default::default()
    };
    let indexer = Indexer::new(Arc::clone(&db), cfg);
    let root_path = sfe_fs::canonical_root(tree.path()).unwrap();
    let root = roots::add(&db.writer(), &root_path).unwrap();
    let workers = indexer.spawn_workers();
    Fixture {
        _data: data,
        tree,
        db,
        indexer,
        root,
        workers,
    }
}

fn write(dir: &Path, rel: &str, content: &[u8]) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// Wait until no jobs are pending and no scan is running.
async fn drain(indexer: &Arc<Indexer>) {
    let mut rx = indexer.subscribe();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let st = rx.borrow_and_update().clone();
        if st.pending_jobs == 0 && st.scanning.is_empty() {
            break;
        }
        tokio::select! {
            _ = rx.changed() => {}
            _ = tokio::time::sleep_until(deadline) => panic!("pipeline did not drain: {st:?}"),
        }
    }
}

#[tokio::test]
async fn scan_then_metadata_stage_makes_names_searchable() {
    let fx = fixture();
    write(fx.tree.path(), "docs/Ferrari notes.md", b"# Ferrari\n");
    let img = image::RgbImage::from_pixel(4, 4, image::Rgb([0, 0, 255]));
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png).unwrap();
    write(fx.tree.path(), "img/blue.png", &png.into_inner());
    write(fx.tree.path(), "node_modules/skip.js", b"");
    write(fx.tree.path(), "bin.dat", b"\0\x01\x02\x03");

    let out = fx.indexer.scan(&fx.root, None).await.unwrap();
    match out {
        ScanOutcome::Done {
            stats,
            enqueued,
            tombstoned,
        } => {
            assert_eq!(stats.files, 3);
            assert_eq!(stats.dirs, 2);
            assert_eq!(enqueued, 5);
            assert_eq!(tombstoned, 0);
        }
        other => panic!("{other:?}"),
    }

    drain(&fx.indexer).await;

    let st = fx.indexer.status();
    assert_eq!(st.total_files, 3);
    assert_eq!(st.metadata, 1, "binary file stops at metadata; {st:?}");
    assert_eq!(st.text, 2, "md + png continue to the text stage; {st:?}");
    assert_eq!(st.failed, 0);

    {
        let c = fx.db.writer();
        let hits = fts::search(&c, &fts::escape_term("ferrari"), 10, 0).unwrap();
        assert_eq!(hits.len(), 1);
        let rec = files::get(&c, hits[0].id).unwrap().unwrap();
        assert!(rec.path.ends_with("Ferrari notes.md"));
        assert_eq!(rec.mime.as_deref(), Some("text/markdown"));
        assert_eq!(rec.index_state, IndexState::Text);

        let png = files::get(
            &c,
            fts::search(&c, &fts::escape_term("blue"), 10, 0).unwrap()[0].id,
        )
        .unwrap()
        .unwrap();
        assert_eq!(png.mime.as_deref(), Some("image/png"));
        assert!(
            fts::search(&c, &fts::escape_term("skip"), 10, 0)
                .unwrap()
                .is_empty()
        );
    }
    fx.finish().await;
}

#[tokio::test]
async fn rescan_is_incremental_and_tombstones_removed_files() {
    let fx = fixture();
    write(fx.tree.path(), "a.txt", b"a");
    write(fx.tree.path(), "sub/b.txt", b"b");
    fx.indexer.scan(&fx.root, None).await.unwrap();
    drain(&fx.indexer).await;

    // Nothing changed → no new jobs.
    let out = fx.indexer.scan(&fx.root, None).await.unwrap();
    assert!(
        matches!(
            out,
            ScanOutcome::Done {
                enqueued: 0,
                tombstoned: 0,
                ..
            }
        ),
        "{out:?}"
    );

    // Remove one file, change another (size), add one.
    std::fs::remove_file(fx.tree.path().join("a.txt")).unwrap();
    write(fx.tree.path(), "sub/b.txt", b"bb-changed");
    write(fx.tree.path(), "c.txt", b"c");
    // updated_at has second granularity; make sure the next scan starts strictly later.
    tokio::time::sleep(Duration::from_millis(1100)).await;

    let out = fx.indexer.scan(&fx.root, None).await.unwrap();
    match out {
        ScanOutcome::Done {
            enqueued,
            tombstoned,
            ..
        } => {
            assert_eq!(enqueued, 2, "changed + new");
            assert_eq!(tombstoned, 1);
        }
        other => panic!("{other:?}"),
    }
    {
        let c = fx.db.writer();
        let a = files::get_by_path(&c, &format!("{}\\a.txt", fx.root.path))
            .or_else(|_| files::get_by_path(&c, &format!("{}/a.txt", fx.root.path)))
            .unwrap();
        assert!(a.map(|r| r.deleted).unwrap_or(true));
        assert!(
            fts::search(&c, &fts::escape_term("a.txt"), 10, 0)
                .unwrap()
                .is_empty()
        );
    }
    fx.finish().await;
}

#[tokio::test]
async fn reconcile_handles_create_modify_delete_and_new_dirs() {
    let fx = fixture();
    write(fx.tree.path(), "keep.txt", b"k");
    fx.indexer.scan(&fx.root, None).await.unwrap();
    drain(&fx.indexer).await;

    // Create a file and a whole new directory tree, then reconcile only the top paths.
    write(fx.tree.path(), "fresh.txt", b"f");
    write(fx.tree.path(), "newdir/inner/deep.txt", b"d");
    let paths = vec![
        fx.tree.path().join("fresh.txt"),
        fx.tree.path().join("newdir"),
    ];
    let paths: Vec<_> = paths
        .into_iter()
        .map(|p| dunce::canonicalize(p).unwrap())
        .collect();
    fx.indexer.reconcile(&fx.root, paths).await.unwrap();
    drain(&fx.indexer).await;
    {
        let c = fx.db.writer();
        assert_eq!(
            fts::search(&c, &fts::escape_term("deep"), 10, 0)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            fts::search(&c, &fts::escape_term("fresh"), 10, 0)
                .unwrap()
                .len(),
            1
        );
    }

    // Delete the directory → everything under it is tombstoned.
    std::fs::remove_dir_all(fx.tree.path().join("newdir")).unwrap();
    let gone = dunce::canonicalize(fx.tree.path()).unwrap().join("newdir");
    fx.indexer.reconcile(&fx.root, vec![gone]).await.unwrap();
    {
        let c = fx.db.writer();
        assert!(
            fts::search(&c, &fts::escape_term("deep"), 10, 0)
                .unwrap()
                .is_empty()
        );
        assert_eq!(files::count_live(&c).unwrap(), 2);
    }
    fx.finish().await;
}

#[tokio::test]
async fn pause_stops_workers_and_resume_continues() {
    let fx = fixture();
    for i in 0..5 {
        write(fx.tree.path(), &format!("f{i}.txt"), b"x");
    }
    fx.indexer.pause();
    fx.indexer.scan(&fx.root, None).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(fx.indexer.status().pending_jobs, 5);
    assert!(fx.indexer.status().paused);

    fx.indexer.resume();
    let mut rx = fx.indexer.subscribe();
    tokio::time::timeout(Duration::from_secs(10), async {
        while rx.borrow_and_update().pending_jobs > 0 {
            rx.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert_eq!(fx.indexer.status().text, 5);
    fx.finish().await;
}

#[tokio::test]
async fn text_stage_indexes_content_metadata_and_hash() {
    let fx = fixture();
    write(
        fx.tree.path(),
        "car.md",
        b"# Notes\n\nThe Ferrari F40 is red.\n",
    );
    write(fx.tree.path(), "data.bin", b"\0\x01\x02\x03binary");
    let img = image::RgbImage::from_pixel(16, 16, image::Rgb([0, 0, 255]));
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png).unwrap();
    write(fx.tree.path(), "sky.png", &png.into_inner());

    fx.indexer.scan(&fx.root, None).await.unwrap();
    drain(&fx.indexer).await;

    let st = fx.indexer.status();
    assert_eq!(st.failed, 0, "{st:?}");
    assert_eq!(st.text, 2, "md + png reach the text stage; {st:?}");
    assert_eq!(st.metadata, 1, "binary stays at metadata; {st:?}");

    {
        let c = fx.db.writer();
        // Body text is searchable even though the name has no "ferrari".
        let hits = fts::search(&c, &fts::escape_term("ferrari"), 10, 0).unwrap();
        assert_eq!(hits.len(), 1);
        let rec = files::get(&c, hits[0].id).unwrap().unwrap();
        assert_eq!(rec.name, "car.md");
        assert_eq!(rec.index_state, IndexState::Text);
        assert_eq!(rec.content_hash.as_ref().map(|h| h.len()), Some(32));
        let info = sfe_db::meta::get_text_info(&c, rec.id).unwrap().unwrap();
        assert_eq!(info.extractor, "plain");
        assert!(!info.truncated);
        assert_eq!(
            sfe_db::meta::get(&c, rec.id).unwrap().unwrap().word_count,
            Some(7)
        );

        let png = files::get(
            &c,
            fts::search(&c, &fts::escape_term("sky"), 10, 0).unwrap()[0].id,
        )
        .unwrap()
        .unwrap();
        let m = sfe_db::meta::get(&c, png.id).unwrap().unwrap();
        assert_eq!((m.width, m.height), (Some(16), Some(16)));
        assert_eq!(m.hue_hist.unwrap()[8], 255);
    }
    fx.finish().await;
}
