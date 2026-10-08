# Smart File Explorer — Technical Architecture & Implementation Plan

## Context

Empty repo (`smart file explorer`, origin `amoghhasdog/Smart-File-Explorer`). Goal: a Windows-first,
local-only desktop app that lets users find files by what they *remember* about them (content,
meaning, visual properties, metadata) instead of filename/folder. The MVP is search only:
index user-selected folders, extract text + metadata, full-text + semantic (text and image)
search, metadata filters, hybrid natural-language queries, polished UI. No destructive ops.

Decisions already made with the user:
- Models are **downloaded on first run** (explicit consent, pinned versions, checksums), then fully offline.
- Target corpus: **100k–1M files** → needs a real ANN index and a tiered, prioritised indexing pipeline.
- Query parsing: **rule-based** parser for MVP; LLM parser is a later plugin.

Environment facts: Node 24 / npm 11 present. **Rust toolchain is NOT installed** (`rustup` needed,
plus MSVC Build Tools and WebView2 — WebView2 ships with Windows 11). This is step 0.

---

## 1. Technology review (your proposals + verdicts)

| Proposal | Verdict | Notes |
|---|---|---|
| Tauri 2 | ✅ Keep | Small binary, Rust backend in-process, WebView2 on Windows. Perfect fit. |
| React + TypeScript | ✅ Keep | Add Vite, TanStack Query (async state), Zustand (UI state), TanStack Virtual (lists of 100k results). |
| Rust backend | ✅ Keep | All filesystem/extraction/search lives here. UI never touches the FS. |
| SQLite + FTS5 | ✅ Keep | `rusqlite` with `bundled` feature (ships FTS5 + JSON1). WAL mode. One DB for metadata + FTS. |
| FastEmbed (`fastembed-rs`) | ✅ Keep, with caveats | Wraps ONNX Runtime; supports text models **and CLIP image embeddings** (`ImageEmbedding`) so one runtime covers both. Caveats: ORT DLL must be bundled (~30 MB); use `ort` with `download-binaries` off and the `load-dynamic` strategy so we control the DLL. Alternative `candle` is pure-Rust but slower on CPU and lacks a drop-in CLIP pipeline — not worth it for MVP. |
| Vector search in SQLite (`sqlite-vec`) | ⚠️ **Not recommended as primary** at 1M scale | `sqlite-vec` is brute-force (no ANN). Fine ≤100k, too slow at 1M×384-d. **Use `usearch`** (HNSW, Rust bindings, memory-mapped on-disk index, supports f16/i8 quantisation). SQLite remains the source of truth for vectors (BLOB column); the HNSW index is a rebuildable cache. |
| PDF text extraction | ⚠️ Rust options are weak | `pdf-extract` (pure Rust) handles most text PDFs but fails on complex layouts/fonts. Plan: start with `pdf-extract` behind the extractor trait; upgrade to `pdfium-render` (bundles Google's pdfium DLL, excellent) if failure rate is high in tests. No OCR in MVP. |

Additional library choices:
- File walking: `jwalk` (parallel) or `walkdir`; watching: `notify` + `notify-debouncer-full`.
- Hashing: `blake3` (content hash for change detection + future dedupe).
- Images: `image` crate (decode/thumbnail/colour stats), `kamadak-exif` (EXIF).
- DOCX/XLSX/PPTX: `zip` + `quick-xml` (they're zipped XML) — simple custom extractors, no heavy deps.
- Video/audio metadata only: `symphonia` (audio) and `mp4`/`matroska` crates for duration/resolution. No ffmpeg in MVP.
- Date parsing in queries: `chrono` + custom grammar; tokeniser via `logos` or `nom`.
- Async: `tokio` (Tauri's runtime), `rayon` for CPU-bound extraction.
- Errors: `thiserror` (library crates), `anyhow` (app boundary). Logging: `tracing` + rolling file appender.

---

## 2. Directory structure (Cargo workspace + Tauri app)

```
smart-file-explorer/
├── .github/workflows/ci.yml          # fmt, clippy, cargo test, vitest, tauri build
├── Cargo.toml                        # workspace: members = ["crates/*", "src-tauri"]
├── package.json / pnpm-workspace     # frontend
├── src-tauri/                        # Tauri app shell (thin!)
│   ├── tauri.conf.json
│   ├── capabilities/                 # Tauri 2 permission scopes (minimal)
│   └── src/
│       ├── main.rs
│       ├── commands/                 # #[tauri::command] wrappers only — validate, call core, map errors
│       ├── events.rs                 # typed event names emitted to UI (index progress, etc.)
│       └── state.rs                  # AppState: Arc<Engine>
├── crates/
│   ├── sfe-core/                     # domain types, config, error types, ids, traits (no IO)
│   ├── sfe-db/                       # SQLite schema, migrations, repositories (typed queries)
│   ├── sfe-fs/                       # ONLY crate allowed to touch the filesystem: walker, watcher, path policy
│   ├── sfe-extract/                  # Extractor trait + registry + adapters (text, pdf, docx, image, media)
│   ├── sfe-embed/                    # Embedder trait, fastembed text + CLIP adapters, model manager (download/verify)
│   ├── sfe-vector/                   # VectorIndex trait, usearch impl, persistence
│   ├── sfe-index/                    # indexing pipeline: scheduler, job queue, stages, progress
│   ├── sfe-query/                    # NL query parser → structured Query AST
│   ├── sfe-search/                   # search orchestrator: FTS + vector + filters → fused ranked results
│   └── sfe-engine/                   # façade wiring everything; what src-tauri calls
├── src/                              # React app
│   ├── app/                          # routes/shell, providers
│   ├── features/
│   │   ├── search/                   # search bar, parsed-query chips, results, preview
│   │   ├── folders/                  # manage indexed roots
│   │   ├── indexing/                 # progress, status, model download consent
│   │   └── settings/
│   ├── lib/ipc/                      # typed invoke() wrappers, generated TS types (ts-rs / specta)
│   ├── components/ui/                # design-system primitives
│   └── test/                         # vitest setup, mocks for ipc
├── models/                           # NOT committed: model manifest (JSON) lives in sfe-embed
├── tests/                            # cross-crate integration tests + fixture corpus
│   └── fixtures/                     # small sample files per type (txt, md, pdf, docx, png, jpg, mp4 stub)
└── docs/ARCHITECTURE.md              # this document, maintained
```

Dependency rule (enforced via crate boundaries + a `cargo-deny`/review check):
`sfe-core` ← everything; `sfe-fs` is the only crate importing `std::fs`/`notify`/`walkdir`;
`src-tauri` depends only on `sfe-engine`. The frontend has **no** `@tauri-apps/plugin-fs` —
all file access goes through named commands.

---

## 3. Frontend / backend separation

- **Backend (Rust)** owns: config, DB, indexing, extraction, embeddings, search, file metadata, thumbnails. Exposes a small typed command API and emits events.
- **Frontend (React)** owns: presentation, query input, result virtualisation, preview rendering. Receives *data only*; never receives raw file bytes except thumbnails (served via a Tauri custom `thumb://` protocol backed by a disk cache in app-data).
- **IPC contract** (generated TS types via `specta` + `tauri-specta` so the contract is checked at compile time):
  - `roots.list / roots.add(path) / roots.remove(id)` — `add` uses Tauri dialog plugin so the user picks the folder; backend validates it's a directory, canonicalises, rejects nested duplicates.
  - `index.status()` → `{ phase, queued, done, failed, rate, eta }`; `index.pause() / index.resume() / index.rescan(rootId)`
  - `search.query(q: string, opts: {limit, cursor, filters?})` → `{ parsed: QueryAst, results: Hit[], nextCursor }`
  - `search.explain(q)` → parsed AST only (drives "chips" in UI as user types)
  - `file.get(id)` → full metadata + extracted snippet; `file.reveal(id)` (open in Explorer) ; `file.open(id)` (shell open) — the only two "actions" in MVP, both non-destructive.
  - `models.status() / models.download(consent: true)` with `model-progress` events
  - Events: `index-progress`, `index-error`, `model-progress`, `roots-changed`
- Long results are **paged** (cursor) — never ship 100k hits over IPC.

---

## 4. Database schema (SQLite, WAL, `sfe-db` migrations via `rusqlite_migration`)

```sql
-- indexed roots chosen by the user
roots(id INTEGER PK, path TEXT UNIQUE, added_at, enabled INT, last_scan_at)

-- one row per file (and dirs, kind='dir', for path browsing)
files(
  id INTEGER PK,
  root_id INT REFERENCES roots ON DELETE CASCADE,
  path TEXT NOT NULL UNIQUE,            -- absolute, canonical, NFC-normalised
  parent_path TEXT, name TEXT, ext TEXT, -- ext lowercased, no dot
  kind TEXT,                             -- 'file' | 'dir'
  category TEXT,                         -- document|image|video|audio|code|archive|other (derived)
  size INT, mtime INT, ctime INT,        -- unix seconds
  content_hash BLOB,                     -- blake3, NULL until hashed (lazy for large files)
  mime TEXT,
  index_state INT,                       -- 0 discovered,1 meta,2 text,3 embedded,9 failed
  index_error TEXT, indexed_at INT, updated_at INT,
  deleted INT DEFAULT 0                  -- tombstone; purged by GC
)
CREATE INDEX files_root, files_ext, files_category, files_mtime, files_size, files_state, files_parent;

-- type-specific metadata, one row per file (sparse; NULLs cheap)
file_meta(
  file_id INT PK REFERENCES files ON DELETE CASCADE,
  width INT, height INT,                 -- images / video
  duration_ms INT,                       -- audio / video
  page_count INT, word_count INT,        -- documents
  camera_make TEXT, taken_at INT, gps_lat REAL, gps_lon REAL,   -- EXIF
  dominant_colors TEXT,                  -- JSON [[r,g,b,share],...] top 5
  hue_hist BLOB,                         -- 12-bin hue histogram (u8) → "lot of blue"
  language TEXT, title TEXT, author TEXT,
  extra JSON                             -- extractor-specific overflow
)

-- extraction bookkeeping only; the text itself lives in `fts`
file_text(file_id INT PK, extractor TEXT, extractor_version INT, char_count INT, truncated INT, extracted_at INT)

-- full-text index. A *regular* FTS5 table (not external-content/contentless):
-- external-content tables make deletes fragile (old values must be replayed
-- exactly) and contentless ones cannot produce snippet(). Owning the text here
-- avoids duplicating it. rowid = file_id; snippet()/bm25() used in search.
CREATE VIRTUAL TABLE fts USING fts5(name, text, tokenize='unicode61 remove_diacritics 2');

-- text chunks for embeddings (documents are chunked; images have 1 "chunk")
chunks(
  id INTEGER PK, file_id INT REFERENCES files ON DELETE CASCADE,
  ord INT, modality TEXT,                -- 'text' | 'image'
  text TEXT,                             -- chunk text (NULL for image)
  model TEXT, dim INT, vec BLOB          -- f32 LE; source of truth for HNSW rebuilds
)
CREATE INDEX chunks_file ON chunks(file_id);

-- durable job queue for the pipeline
-- one job per (file, stage); workers lease jobs for a bounded time so a crash
-- or hang makes them pickable again
jobs(id INTEGER PK, file_id INT, stage INT, priority INT, attempts INT, not_before INT,
     leased_until INT, created_at INT, UNIQUE(file_id, stage))
CREATE INDEX jobs_pick ON jobs(stage, priority DESC, not_before, id);

-- model registry (what's downloaded + verified)
models(name TEXT PK, version TEXT, sha256 TEXT, path TEXT, downloaded_at INT)

settings(key TEXT PK, value TEXT)
schema_version via rusqlite_migration
```

Sizing at 1M files: `files` ~300–400 MB, `fts` depends on text volume (cap extracted text at 1 MB/file, 200 k chars default), `chunks.vec` 384-d f32 = 1.5 KB/chunk. Expect several GB for a 1M mixed corpus — acceptable, user sees DB size in settings. Store the DB in `%LOCALAPPDATA%\SmartFileExplorer\`.

---

## 5. Indexing pipeline (`sfe-index`)

**Tiered, resumable, prioritised.** Every file passes through stages; cheap stages run first across the *whole* corpus so the user gets name/metadata search within minutes even on 1M files.

```
Stage 0  Discover   walk root (jwalk, parallel) → upsert files rows (batched, 1 txn / 2k rows),
                    diff against DB by (path, size, mtime) → enqueue jobs for new/changed, tombstone missing.
Stage 1  Metadata   mime sniff (magic bytes), category, EXIF/image dims/duration (header-only reads, no full decode).
                    Also insert `name` into FTS. Fast: ~thousands/s.
Stage 2  Text       run matching Extractor → file_text + FTS text column; compute blake3 (streamed) for files < 256 MB.
Stage 3  Embed      chunk text (≈ 256 tokens, 32 overlap, max 64 chunks/file) → text embedder (batched 32);
                    images → thumbnail (max 512 px, cached) → CLIP image embedder (batched 16); hue histogram.
                    Write vectors to chunks + add to HNSW.
```

- **Scheduler**: one `tokio` task owns the queue; pulls jobs ordered `(stage ASC, priority DESC)` so stage 1 for all files precedes stage 2 for any, unless priority overrides (recently modified files, files in the folder the user is currently viewing, smaller files first within a stage).
- **Workers**: CPU-bound stages run on a dedicated `rayon` pool sized `max(1, cores-2)`; embedding runs on **one** dedicated thread (ORT is internally multi-threaded; two sessions fight). Blocking IO via `tokio::task::spawn_blocking`.
- **Backpressure**: bounded channels (cap 1k) between stages; DB writes batched by a single writer task (SQLite = single writer; this avoids `SQLITE_BUSY`).
- **Incremental**: change detection = `(size, mtime)` then hash on stage 2; unchanged → skip. `notify` watcher on each root (debounced 2 s) enqueues jobs live; `ReadDirectoryChangesW` overflow → full rescan of that root.
- **Resume**: jobs table is durable; on startup, in-flight jobs are re-queued. Pipeline can be paused (UI, or auto when on battery / when user is actively searching — "search gets priority" by lowering worker count to 1 while a search is in flight).
- **Throughput budget** (CPU-only, 8-core): stage 1 ~3–5k files/s, stage 2 ~50–500 files/s (PDF slowest), text embed ~100–300 chunks/s, CLIP ~15–40 img/s. 1M files with 200k images ≈ 2–4 h for images → acceptable as background work; UI shows ETA per stage. Optional later: DirectML execution provider for GPU.
- **GC**: tombstoned files purged (and vectors removed from HNSW) in a periodic sweep.

---

## 6. File extraction architecture (`sfe-extract`)

```rust
pub trait Extractor: Send + Sync {
    fn id(&self) -> &'static str;              // "pdf", "docx", "plain", "image"
    fn version(&self) -> u32;                   // bump → re-extract affected files
    fn supports(&self, probe: &FileProbe) -> Support; // No | ByExt | ByMagic (priority)
    fn extract(&self, input: &mut dyn ExtractInput, limits: &Limits) -> Result<Extracted, ExtractError>;
}
pub struct Extracted { text: Option<String>, meta: MetaPatch, image: Option<ImageHandle>, truncated: bool }
```
- `ExtractInput` abstracts the file (seekable reader + size + path hint) so tests feed in-memory bytes and the extractor crate never opens files itself (`sfe-fs` opens, hands over the reader).
- **Registry** orders adapters by specificity; first `Support::ByMagic` wins over `ByExt`. New types = add one module + register. Future external plugins (e.g. a WASM extractor host) can slot in behind the same trait — not MVP.
- MVP adapters: `plain` (txt/md/csv/json/xml/yaml/toml + ~40 code extensions, UTF-8/UTF-16 BOM detection, binary-sniff bail), `pdf` (pdf-extract), `docx`, `xlsx` (cell text), `pptx`, `image` (dims, EXIF, thumbnail, dominant colours + hue histogram), `media` (duration/resolution only via container parsing).
- **Limits** passed in: max bytes read, max text chars, wall-clock timeout (extraction runs under `std::thread` + timeout watchdog; a hung PDF parser must not stall the pool — enforced by running extractors in a **separate process pool**? No: for MVP use `catch_unwind` + timeout via a cancellation flag; see Risks).

---

## 7. Search architecture (`sfe-search` + `sfe-query`)

**Query AST** (output of rule-based parser):
```
Query { text_terms: Vec<String>, phrases: Vec<String>, semantic_text: Option<String>,
        filters: Filters { categories, exts, size: Range, mtime: Range, taken: Range,
                           duration: Range, dims, color: Option<ColorPred>, root, path_contains },
        mode_hint: Keyword | Semantic | Auto }
```
Parser grammar (deterministic, unit-tested): type words (`images, photos, pdfs, videos, docs, spreadsheets, code, screenshots`→image+name/dims heuristic), extensions, size (`over 10mb`, `< 2 GB`), dates (`before Jan 2026`, `last year`, `in 2024`, `yesterday`, `last week`, `downloaded last year`→mtime/ctime), durations (`longer than 10 minutes`), colours (`a lot of blue`, `mostly red` → hue-bin predicate), `containing the word X` / `"quoted"` → exact FTS phrase, `about X` / `related to X` → semantic. Leftover tokens → both FTS and semantic. UI shows the parsed chips so users see/correct the interpretation.

**Execution plan** (`SearchStrategy` trait, modular):
1. `FilterStage` → SQL `WHERE` over `files`/`file_meta` producing a candidate set (bitmap/ID list; skipped if no filters).
2. `KeywordStrategy` → FTS5 `MATCH` with `bm25()` weighting (name ×3, text ×1), restricted to candidates via `rowid IN` (or post-filter when candidate set is large).
3. `SemanticTextStrategy` → embed query with text model → HNSW top-K (K=200) over text chunks → group by file (max chunk score).
4. `SemanticImageStrategy` → embed query with CLIP **text** tower → HNSW top-K over image vectors (separate index, different dim/model).
5. `Fusion` → Reciprocal Rank Fusion across strategies (weights tunable; keyword gets a boost when exact phrase present), then apply filters that couldn't be pushed down, then page.
6. Result enrichment: snippet from FTS `snippet()` or best chunk; thumbnail URL; matched-because explanation (which strategy hit).

Latency target: < 150 ms for 1M files (filters via indexes, HNSW ~ms, FTS ~tens of ms). Strategies run concurrently with `tokio::join!`.

---

## 8. Embedding architecture (`sfe-embed` + `sfe-vector`)

```rust
pub trait TextEmbedder: Send + Sync { fn model(&self) -> ModelId; fn dim(&self) -> usize; fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>; }
pub trait ImageEmbedder: Send + Sync { fn embed_images(&self, imgs: &[DynamicImage]) -> ...; fn embed_text(&self, q: &str) -> ...; }
pub trait VectorIndex: Send + Sync { fn add(&self, id: u64, v: &[f32]); fn remove(&self, id); fn search(&self, v: &[f32], k: usize, filter: Option<&dyn Fn(u64)->bool>) -> Vec<(u64,f32)>; fn save/load }
```
- Models (pinned in a manifest with SHA-256): text = `BAAI/bge-small-en-v1.5` (384-d, fast, good) via fastembed; image = `CLIP ViT-B/32` (512-d) via fastembed `ImageEmbedding` + its text tower for queries. Multilingual option later (`multilingual-e5-small`).
- **Model manager**: first-run consent screen → downloads to app-data, verifies hash, writes `models` row. App works without models (keyword + filters only) and shows "semantic search unavailable" until installed.
- Two HNSW indexes (`text.usearch`, `image.usearch`), keyed by `chunks.id`, cosine metric, f16 storage (halves RAM: 1M text chunks ≈ 0.8 GB; realistic). Loaded via mmap at startup; rebuilt from `chunks.vec` if missing/corrupt. Writes batched; index saved every N adds and on shutdown.
- Determinism: model + version stored per chunk; changing model → background re-embed job.

---

## 9. Concurrency strategy (summary)

| Concern | Mechanism |
|---|---|
| UI never freezes | All commands `async`; heavy work on rayon / blocking pool; events for progress; results paged. |
| SQLite single writer | One writer task with batched transactions; readers use a pool of read-only connections (WAL). |
| ORT sessions | One text session, one CLIP session, each behind a `Mutex`, used by a single embed worker. |
| Search vs indexing | Indexer throttles (workers→1, `SQLITE` writes yield) while a search is active; search reads never block on writer (WAL). |
| Cancellation | `CancellationToken` plumbed to every stage; pause/stop/rescan cancels cooperatively. |
| Shutdown | Flush queue state, save HNSW, checkpoint WAL. |

---

## 10. Failure handling

- Per-file failures are **isolated**: extractor error → `index_state=9` + `index_error`; retry with backoff up to 3 attempts for IO errors; permanent for parse errors until `extractor.version` bumps. Visible in a "problems" list in UI.
- Panics inside extractors caught (`catch_unwind`); timeouts enforced; oversized inputs truncated with `truncated=1`.
- Locked/unreadable files (permissions, in-use) → skipped, logged, retried on next change event.
- Root removed/offline (external drive) → root marked `unavailable`, files kept, not tombstoned until user confirms.
- DB corruption → detected on open (`PRAGMA quick_check`), offer rebuild. HNSW corrupt → silent rebuild from `chunks`.
- Model download failure → app continues in keyword mode; retry button.
- All errors structured (`thiserror`) and mapped to a small `IpcError { code, message }` at the command boundary — no raw paths/Rust errors leak into UI strings unexpectedly.

---

## 11. Security & privacy

- **No network** except the model download (user-initiated, consented, to pinned URLs, hash-verified). CI test asserts no other crate opens sockets (`cargo-deny` ban list + review).
- Tauri 2 capabilities minimal: `dialog:open`, `shell:open` (allow-list only `file://` reveal/open), `core:event`. No `fs` plugin exposed to the webview. CSP strict; no remote content.
- Custom `thumb://` protocol validates IDs, serves only from the thumbnail cache dir.
- Path policy in `sfe-fs`: canonicalise, reject reparse-point loops, skip `$Recycle.Bin`, `System Volume Information`, `node_modules`/`.git` by default (configurable ignore list, `.sfeignore` support later), symlink/junction cycles guarded by visited-set of file IDs.
- Index stores extracted text — that's sensitive by nature. Store DB under per-user AppData with default NTFS ACLs; offer "exclude folder" and "purge index" actions; document plainly in UI. (At-rest encryption via SQLCipher is a possible later option, not MVP.)
- No telemetry.

---

## 12. Testing strategy

- **Unit (Rust, per crate)**: query parser (table-driven, ~100 cases), extractors against `tests/fixtures` (in-memory input, assert text/meta), chunker, RRF fusion, path policy, change detection, colour histogram.
- **DB tests**: migrations up from empty; repository round-trips on `:memory:` or temp file; FTS snippet/bm25 behaviour.
- **Pipeline integration**: temp dir with generated corpus (`tempfile`) → run engine with a *fake embedder* (deterministic hash-based vectors so tests don't need models) → assert states, incremental re-index on touch/delete/rename, watcher events (with debounce tolerance).
- **Search integration**: seeded index → NL queries → expected files ranked top-N.
- **Scale test (ignored by default, `cargo test -- --ignored`)**: synthesise 500k file rows + 1M random vectors → assert discover throughput, search p95 < 150 ms, memory ceiling.
- **Model smoke test** (opt-in, needs downloaded models): real embedder sanity (similar texts closer than dissimilar; CLIP "a red car" ranks red-car fixture first).
- **Frontend**: Vitest + Testing Library with mocked `invoke`; Playwright-style E2E via `tauri-driver` (WebDriver) on the built app for the core flow (add folder → wait → search → open result) — run in CI on Windows runner.
- **CI**: GitHub Actions (windows-latest): `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace`, `npm test`, `tauri build` artifact.

---

## 13. Technical risks (ranked)

1. **PDF extraction quality** in pure Rust — mitigated by adapter trait; fallback to `pdfium-render` (+DLL). OCR for scanned PDFs out of scope.
2. **ONNX Runtime packaging on Windows** (DLL discovery, CRT, antivirus false positives on first run). Mitigation: bundle `onnxruntime.dll` as a Tauri resource, load dynamically, pin ORT version matching fastembed.
3. **Embedding throughput at 1M scale on CPU** — tiered pipeline makes keyword search available early; user can cap "semantic indexing" to selected roots/categories; GPU (DirectML) EP as follow-up.
4. **Memory**: 1M HNSW vectors + SQLite caches. Mitigated with f16 storage, chunk caps per file, mmap index, configurable limits; measured by scale test.
5. **Hung/crashing third-party parsers** inside the app process. `catch_unwind` doesn't catch stack overflows/aborts. If fixtures reveal this, move extraction to a sidecar process (`sfe-extract-worker`) — the trait design makes this a transport change only.
6. **Filesystem watcher gaps** (overflow, network drives, OneDrive placeholder files that trigger cloud download on read). Mitigation: skip files with `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`; periodic reconciliation scan.
7. **Rule-based NL parser coverage** — chips UI makes misinterpretation visible; grammar grows via test cases.
8. **Long paths / Unicode / case-insensitivity on NTFS** — use `\\?\` prefix, NFC normalise, case-insensitive uniqueness.
9. **Rust toolchain not installed** — step 0; `rustup`, `Visual Studio Build Tools (C++)`, confirm `cargo tauri` builds the template before anything else.

---

## 14. Explicitly NOT in MVP

Destructive/organising ops (move/rename/delete, undo), smart organisation, collections, duplicate detection UI (hash is stored, nothing built on it), cleanup recommendations, NL file operations, video frame understanding, audio transcription, OCR, project auto-grouping, LLM query parsing, cloud anything, multi-user, macOS/Linux builds (keep code portable, don't test), index encryption, external plugin loading, GPU acceleration, browsing-the-filesystem-as-Explorer-replacement (we show results + reveal in Explorer).

---

## 15. Implementation phases (each ends with green tests)

0. **Toolchain + skeleton**: install Rust/MSVC; `create-tauri-app` (React-TS, Vite); workspace crates with empty traits; CI; `docs/ARCHITECTURE.md`. Verify `tauri dev` and `tauri build`.
1. **sfe-core + sfe-db**: types, migrations, repositories, FTS table; tests.
2. **sfe-fs + Discover/Metadata stages**: walker, change detection, watcher, scheduler, writer task; UI: add roots, progress. *Milestone: name search over 1M files.*
3. **sfe-extract**: plain/pdf/docx/xlsx/pptx/image/media adapters + fixtures; Text stage; FTS search with snippets.
4. **sfe-query + sfe-search (keyword + filters)**: parser, chips UI, filter pushdown, paging, result list with virtualisation, preview pane, open/reveal.
5. **sfe-embed + sfe-vector**: model manager + consent UI, text embedder, chunker, usearch index, semantic text strategy, RRF fusion.
6. **Image semantics**: thumbnails, CLIP, hue histogram + colour predicates, image strategy.
7. **Hardening**: scale test, pause/throttle, problems list, settings, installer (NSIS via Tauri), polish pass on UI.

## Verification (end-to-end)

- `cargo test --workspace` and `npm test` green; `cargo test -- --ignored scale_` passes thresholds.
- Manual: add a folder with mixed fixtures → within seconds name search works → after extraction "documents containing the word Ferrari" returns the fixture → after models installed "images with a lot of blue", "screenshots containing Python code", "PDFs about machine learning downloaded last year" return expected fixtures with sensible chips; UI stays responsive (type in search while indexing a large folder).
- Network monitor shows zero connections outside the model download.
