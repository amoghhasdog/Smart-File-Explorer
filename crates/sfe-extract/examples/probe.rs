//! `cargo run -p sfe-extract --example probe -- <file>` — run the registry on a file.
use sfe_extract::{FileInput, FileProbe, Limits, Registry};
fn main() {
    let path = std::path::PathBuf::from(std::env::args().nth(1).expect("path"));
    let meta = std::fs::metadata(&path).unwrap();
    let head = std::fs::read(&path).unwrap();
    println!("head: {:02x?}", &head[..16.min(head.len())]);
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let mime = infer::get(&head).map(|t| t.mime_type().to_string());
    println!("ext={ext} mime={mime:?} size={}", meta.len());
    let reg = Registry::with_defaults();
    let probe = FileProbe {
        ext: &ext,
        mime: mime.as_deref(),
        size: meta.len(),
    };
    let ex = reg.find(&probe).expect("no extractor");
    println!("extractor={}", ex.id());
    let input = FileInput::new(
        std::fs::File::open(&path).unwrap(),
        meta.len(),
        path.clone(),
    );
    match sfe_extract::run_guarded(ex, Box::new(input), &Limits::default()) {
        Ok(e) => println!("ok: {:?}", e.meta),
        Err(e) => println!("ERR: {e}"),
    }
}
