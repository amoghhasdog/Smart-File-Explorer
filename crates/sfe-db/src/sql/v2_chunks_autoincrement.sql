-- v2: chunk ids are keys in the on-disk vector index, so they must never be
-- reused after a delete. Recreate `chunks` with AUTOINCREMENT.

CREATE TABLE chunks_v2 (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    file_id  INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    ord      INTEGER NOT NULL,
    modality TEXT NOT NULL,
    text     TEXT,
    model    TEXT NOT NULL,
    dim      INTEGER NOT NULL,
    vec      BLOB NOT NULL
);
INSERT INTO chunks_v2 (id, file_id, ord, modality, text, model, dim, vec)
    SELECT id, file_id, ord, modality, text, model, dim, vec FROM chunks;
DROP TABLE chunks;
ALTER TABLE chunks_v2 RENAME TO chunks;
CREATE INDEX chunks_file ON chunks(file_id);
