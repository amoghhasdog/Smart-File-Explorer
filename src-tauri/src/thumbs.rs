//! `thumb://localhost/<file_id>` — serves cached thumbnails to the webview.
//! The id is validated and mapped to a path by the engine; the webview never
//! supplies a filesystem path.

use std::borrow::Cow;

use sfe_core::FileId;
use sfe_engine::Engine;
use tauri::{Manager, UriSchemeContext, http};

pub fn handle(
    ctx: UriSchemeContext<'_, tauri::Wry>,
    request: http::Request<Vec<u8>>,
) -> http::Response<Cow<'static, [u8]>> {
    let id = request
        .uri()
        .path()
        .trim_start_matches('/')
        .parse::<i64>()
        .ok()
        .map(FileId);
    let path = id.and_then(|id| ctx.app_handle().state::<Engine>().thumbnail_path(id));
    match path.and_then(|p| std::fs::read(p).ok()) {
        Some(bytes) => http::Response::builder()
            .status(200)
            .header("Content-Type", "image/jpeg")
            .header("Cache-Control", "max-age=3600")
            .body(Cow::Owned(bytes))
            .unwrap_or_else(|_| not_found()),
        None => not_found(),
    }
}

fn not_found() -> http::Response<Cow<'static, [u8]>> {
    http::Response::builder()
        .status(404)
        .body(Cow::Borrowed(&b""[..]))
        .expect("static response")
}
