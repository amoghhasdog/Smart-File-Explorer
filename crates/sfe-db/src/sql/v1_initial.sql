-- v1: initial schema. See docs/ARCHITECTURE.md §4.

CREATE TABLE roots (
    id           INTEGER PRIMARY KEY,
    path         TEXT NOT NULL UNIQUE,
    added_at     INTEGER NOT NULL,
    enabled      INTEGER NOT NULL DEFAULT 1,
    last_scan_at INTEGER
);

CREATE TABLE files (
    id           INTEGER PRIMARY KEY,
    root_id      INTEGER NOT NULL REFERENCES roots(id) ON DELETE CASCADE,
    path         TEXT NOT NULL UNIQUE,
    parent_path  TEXT NOT NULL,
    name         TEXT NOT NULL,
    ext          TEXT NOT NULL DEFAULT '',
    kind         TEXT NOT NULL,            -- 'file' | 'dir'
    category     TEXT NOT NULL,            -- document|image|video|audio|code|archive|other
    size         INTEGER NOT NULL DEFAULT 0,
    mtime        INTEGER NOT NULL DEFAULT 0,
    ctime        INTEGER,
    content_hash BLOB,
    mime         TEXT,
    index_state  INTEGER NOT NULL DEFAULT 0,
    index_error  TEXT,
    indexed_at   INTEGER,
    updated_at   INTEGER NOT NULL,
    deleted      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX files_root     ON files(root_id);
CREATE INDEX files_parent   ON files(parent_path);
CREATE INDEX files_ext      ON files(ext);
CREATE INDEX files_category ON files(category);
CREATE INDEX files_mtime    ON files(mtime);
CREATE INDEX files_size     ON files(size);
CREATE INDEX files_state    ON files(index_state) WHERE deleted = 0;
CREATE INDEX files_deleted  ON files(deleted) WHERE deleted = 1;

CREATE TABLE file_meta (
    file_id         INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    width           INTEGER,
    height          INTEGER,
    duration_ms     INTEGER,
    page_count      INTEGER,
    word_count      INTEGER,
    camera_make     TEXT,
    taken_at        INTEGER,
    gps_lat         REAL,
    gps_lon         REAL,
    dominant_colors TEXT,
    hue_hist        BLOB,
    language        TEXT,
    title           TEXT,
    author          TEXT,
    extra           TEXT
);
CREATE INDEX file_meta_taken_at ON file_meta(taken_at);
CREATE INDEX file_meta_duration ON file_meta(duration_ms);

-- Extraction bookkeeping. The text itself lives in `fts` (rowid = file_id).
CREATE TABLE file_text (
    file_id           INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    extractor         TEXT NOT NULL,
    extractor_version INTEGER NOT NULL,
    char_count        INTEGER NOT NULL,
    truncated         INTEGER NOT NULL DEFAULT 0,
    extracted_at      INTEGER NOT NULL
);

-- Full-text index. Regular (not external-content) FTS5 so snippet()/bm25()
-- work and deletes are safe. rowid = files.id.
CREATE VIRTUAL TABLE fts USING fts5(
    name,
    text,
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TABLE chunks (
    id       INTEGER PRIMARY KEY,
    file_id  INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    ord      INTEGER NOT NULL,
    modality TEXT NOT NULL,                -- 'text' | 'image'
    text     TEXT,
    model    TEXT NOT NULL,
    dim      INTEGER NOT NULL,
    vec      BLOB NOT NULL
);
CREATE INDEX chunks_file ON chunks(file_id);

CREATE TABLE jobs (
    id          INTEGER PRIMARY KEY,
    file_id     INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    stage       INTEGER NOT NULL,
    priority    INTEGER NOT NULL DEFAULT 0,
    attempts    INTEGER NOT NULL DEFAULT 0,
    not_before  INTEGER NOT NULL DEFAULT 0,
    leased_until INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    UNIQUE (file_id, stage)
);
CREATE INDEX jobs_pick ON jobs(stage, priority DESC, not_before, id);

CREATE TABLE models (
    name          TEXT PRIMARY KEY,
    version       TEXT NOT NULL,
    sha256        TEXT NOT NULL,
    path          TEXT NOT NULL,
    downloaded_at INTEGER NOT NULL
);

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
