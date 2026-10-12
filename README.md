# Smart File Explorer

Find files by what you remember about them — not by filename or folder.

> "images with a lot of blue" · "documents containing the word Ferrari" · "PDFs about machine
> learning I downloaded last year" · "videos longer than 10 minutes created before January 2026"

Windows-first desktop app. **Everything runs locally**: indexing, text extraction, embeddings and
search never leave your machine. The only network access is a one-time, consented download of the
embedding models.

## Features (MVP)

- Index any folders you choose; incremental rescans and live updates via file-system watching.
- Reads documents (PDF, DOCX, XLSX, PPTX, text/code), images (dimensions, EXIF, dominant colours),
  and media containers (duration, resolution).
- Full-text search with snippets, plus **semantic search** by meaning for text (BGE-small) and
  images (CLIP) — optional models, downloaded once.
- Natural-language queries: types, extensions, sizes, durations, dates, colours, folders, quoted
  phrases and topics are understood and shown back as chips.
- Fast at scale: tiered indexing (names searchable within minutes, content and vectors fill in
  later), SQLite + FTS5, HNSW vector index.

## Build

Prerequisites: Rust (stable, MSVC toolchain), Node 20+, Visual Studio Build Tools with the C++
workload, WebView2 (ships with Windows 11).

```bash
npm install
npm run tauri dev        # development
npm run tauri build      # NSIS installer in target/release/bundle/nsis
```

## Test

```bash
cargo test --workspace                       # Rust (unit + integration)
npm test -- --run                            # frontend
cargo clippy --workspace --all-targets -- -D warnings
```

Opt-in slow tests:

```bash
cargo test -p sfe-engine --test scale --release -- --ignored --nocapture   # 500k files, latency budget
cargo test -p sfe-embed --test real_model -- --ignored                     # downloads the text model
cargo run -p sfe-query --example explain -- "pdfs about rust from last year"  # inspect a query
```

## Layout

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). In short: a Cargo workspace of `sfe-*` crates
(`fs` is the only one touching the filesystem; `db` the only one issuing SQL; `extract`, `embed`,
`vector`, `index`, `query`, `search`, `engine`), a thin Tauri shell in `src-tauri`, and a React UI in
`src` whose only Tauri import is `src/lib/ipc.ts`.

## Data

Index, thumbnails, vector indexes and models live under `%LOCALAPPDATA%\com.smartfileexplorer.app`.
"Purge index" in the app removes everything except installed models.
