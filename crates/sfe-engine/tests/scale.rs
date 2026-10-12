//! Scale test: synthesises a large index and checks search latency.
//! Ignored by default (takes a few minutes and ~1 GB of disk):
//! `cargo test -p sfe-engine --test scale --release -- --ignored --nocapture`

use std::sync::Arc;
use std::time::{Duration, Instant};

use sfe_core::{DiscoveredFile, FileId, FileKind, RootId};
use sfe_db::{chunks, files, fts, roots};
use sfe_embed::{FakeEmbedder, TextEmbedder};
use sfe_engine::Engine;

const FILES: usize = 500_000;
const VECTORS: usize = 200_000;
const P95_BUDGET: Duration = Duration::from_millis(150);

fn p95(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    samples[(samples.len() as f64 * 0.95) as usize - 1]
}

#[test]
#[ignore = "slow; run explicitly"]
fn search_stays_fast_at_half_a_million_files() {
    let data = tempfile::tempdir().unwrap();
    let engine = Engine::open(data.path()).unwrap();
    let embedder = Arc::new(FakeEmbedder::default());
    engine.activate_embedder(embedder.clone()).unwrap();
    let db = engine.db();

    let kinds = [
        ("report", "pdf"),
        ("photo", "jpg"),
        ("notes", "md"),
        ("clip", "mp4"),
        ("sheet", "xlsx"),
    ];
    let t = Instant::now();
    let root = db.transaction(|c| roots::add(c, "C:/scale")).unwrap();
    let mut ids: Vec<FileId> = Vec::with_capacity(FILES);
    for start in (0..FILES).step_by(5_000) {
        let batch: Vec<DiscoveredFile> = (start..(start + 5_000).min(FILES))
            .map(|i| {
                let (stem, ext) = kinds[i % kinds.len()];
                let name = format!("{stem}-{i}.{ext}");
                DiscoveredFile {
                    root_id: root.id,
                    path: format!("C:/scale/d{}/{name}", i % 997),
                    parent_path: format!("C:/scale/d{}", i % 997),
                    name,
                    ext: ext.into(),
                    kind: FileKind::File,
                    size: (i as u64 * 7_919) % (50 << 20),
                    mtime: 1_600_000_000 + (i as i64 * 61) % 190_000_000,
                    ctime: None,
                }
            })
            .collect();
        let got = db
            .transaction(|c| {
                let ups = files::upsert_discovered(c, &batch)?;
                for (u, d) in ups.iter().zip(&batch) {
                    fts::set(c, u.id, &d.name, "")?;
                }
                Ok(ups.into_iter().map(|u| u.id).collect::<Vec<_>>())
            })
            .unwrap();
        ids.extend(got);
    }
    println!("inserted {FILES} files in {:?}", t.elapsed());

    // Vectors straight into chunks + the ANN index (what the embed stage does).
    let t = Instant::now();
    let dim = embedder.dim();
    let index_path = data
        .path()
        .join("vectors")
        .join("text-fake-hash-v1.usearch");
    let index = sfe_vector::Hnsw::open(&index_path, dim).unwrap();
    for start in (0..VECTORS).step_by(2_000) {
        let items = db
            .transaction(|c| {
                let mut items = Vec::new();
                for (i, file_id) in ids.iter().enumerate().skip(start).take(2_000) {
                    if i >= VECTORS {
                        break;
                    }
                    let v = embedder
                        .embed(&[&format!("document {i} about topic {}", i % 500)])
                        .unwrap()
                        .remove(0);
                    let cid = chunks::replace_for_file(
                        c,
                        *file_id,
                        &[chunks::NewChunk {
                            ord: 0,
                            modality: "text",
                            text: None,
                            model: embedder.model_id(),
                            vec: &v,
                        }],
                    )?;
                    items.push((cid[0].0 as u64, v));
                }
                Ok(items)
            })
            .unwrap();
        sfe_vector::VectorIndex::add_batch(&index, &items).unwrap();
    }
    sfe_vector::VectorIndex::save(&index).unwrap();
    drop(index);
    println!("inserted {VECTORS} vectors in {:?}", t.elapsed());

    // Fresh engine so the ANN index is loaded from disk like in production.
    drop(engine);
    let t = Instant::now();
    let engine = Engine::open(data.path()).unwrap();
    engine.activate_embedder(embedder.clone()).unwrap();
    println!("reopened engine in {:?}", t.elapsed());
    let st = engine.status();
    assert_eq!(st.total_files as usize, FILES);

    let queries = [
        "report",
        "photo-1234",
        "pdfs over 10 mb modified in 2024",
        "images",
        "notes about topic 42",
        "clips longer than 10 minutes",
        "sheet-9",
    ];
    let mut by_query = Vec::new();
    for q in queries {
        let mut samples = Vec::new();
        for _ in 0..20 {
            let t = Instant::now();
            let r = engine.search(q, 50, 0).unwrap();
            samples.push(t.elapsed());
            assert!(
                !r.hits.is_empty() || q == "clips longer than 10 minutes",
                "{q}"
            );
        }
        let p = p95(samples);
        println!("{q:40} p95 {:?}", p);
        by_query.push((q, p));
    }
    for (q, p) in by_query {
        assert!(p <= P95_BUDGET, "{q}: p95 {p:?} exceeds {P95_BUDGET:?}");
    }
    let _ = RootId(0);
}
