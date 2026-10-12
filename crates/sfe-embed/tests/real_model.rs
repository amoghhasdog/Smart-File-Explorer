//! Exercises the real ONNX model end to end. Needs network for the one-time
//! download (~35 MB) and is therefore ignored by default:
//! `cargo test -p sfe-embed --test real_model -- --ignored`

use std::sync::Arc;

use sfe_embed::{ModelManager, TEXT_MODEL, cosine};

#[test]
#[ignore = "downloads the model; run explicitly"]
fn downloads_loads_and_embeds_meaningfully() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = Arc::new(ModelManager::new(dir.path()));
    assert!(!mgr.is_installed(&TEXT_MODEL));

    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen2 = Arc::clone(&seen);
    let embedder = mgr
        .install_text(&TEXT_MODEL, move |p| seen2.lock().unwrap().push(p))
        .expect("install");
    assert!(mgr.is_installed(&TEXT_MODEL));
    let progress = seen.lock().unwrap();
    assert!(
        progress.last().is_some_and(|p| p.done && p.error.is_none()),
        "{progress:?}"
    );
    drop(progress);

    assert_eq!(embedder.dim(), 384);
    let v = embedder
        .embed(&[
            "The Ferrari F40 is a sports car built in Maranello.",
            "Italian supercars with V8 engines.",
            "How to bake sourdough bread at home.",
        ])
        .unwrap();
    assert_eq!(v[0].len(), 384);
    let close = cosine(&v[0], &v[1]);
    let far = cosine(&v[0], &v[2]);
    assert!(close > far + 0.1, "close={close} far={far}");

    // Query instruction path and offline reload.
    let q = embedder.embed_query("fast italian cars").unwrap();
    assert!(cosine(&q, &v[1]) > cosine(&q, &v[2]));
    let reloaded = mgr.load_text(&TEXT_MODEL).expect("offline load");
    assert_eq!(reloaded.model_id(), embedder.model_id());
}
