//! FTS5 repository. `fts.rowid == files.id`; columns `name` (weight ×3 in
//! ranking) and `text`.

use rusqlite::{Connection, OptionalExtension, params};
use sfe_core::{FileId, RootId};

use crate::Result;

/// Set (or replace) the searchable name + text of a file.
pub fn set(conn: &Connection, id: FileId, name: &str, text: &str) -> Result<()> {
    conn.execute("DELETE FROM fts WHERE rowid = ?1", [id.0])?;
    conn.execute(
        "INSERT INTO fts(rowid, name, text) VALUES (?1, ?2, ?3)",
        params![id.0, name, text],
    )?;
    Ok(())
}

/// Set only the name (metadata stage); keeps any existing text.
pub fn set_name(conn: &Connection, id: FileId, name: &str) -> Result<()> {
    let existing: Option<String> = conn
        .query_row("SELECT text FROM fts WHERE rowid = ?1", [id.0], |r| {
            r.get(0)
        })
        .optional()?;
    set(conn, id, name, existing.as_deref().unwrap_or(""))
}

pub fn delete(conn: &Connection, id: FileId) -> Result<()> {
    conn.execute("DELETE FROM fts WHERE rowid = ?1", [id.0])?;
    Ok(())
}

pub fn delete_many(conn: &Connection, ids: &[FileId]) -> Result<()> {
    let mut stmt = conn.prepare_cached("DELETE FROM fts WHERE rowid = ?1")?;
    for id in ids {
        stmt.execute([id.0])?;
    }
    Ok(())
}

/// Remove FTS rows of every file under a root (call before `roots::remove`).
pub fn delete_for_root(conn: &Connection, root: RootId) -> Result<()> {
    conn.execute(
        "DELETE FROM fts WHERE rowid IN (SELECT id FROM files WHERE root_id = ?1)",
        [root.0],
    )?;
    Ok(())
}

pub fn get_text(conn: &Connection, id: FileId) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT text FROM fts WHERE rowid = ?1", [id.0], |r| {
            r.get(0)
        })
        .optional()?)
}

#[derive(Debug, Clone, PartialEq)]
pub struct FtsHit {
    pub id: FileId,
    /// Lower is better (FTS5 `bm25()` returns negative scores).
    pub score: f64,
    pub snippet: String,
}

/// Run an FTS5 `MATCH`. `query` must already be valid FTS5 syntax — callers
/// build it with [`escape_term`] / [`phrase`]. Excludes tombstoned files.
pub fn search(conn: &Connection, query: &str, limit: usize, offset: usize) -> Result<Vec<FtsHit>> {
    let mut stmt = conn.prepare_cached(
        "SELECT fts.rowid, bm25(fts, 3.0, 1.0) AS score,
                snippet(fts, 1, '[', ']', '…', 12)
         FROM fts JOIN files ON files.id = fts.rowid
         WHERE fts MATCH ?1 AND files.deleted = 0
         ORDER BY score
         LIMIT ?2 OFFSET ?3",
    )?;
    let rows = stmt.query_map(params![query, limit as i64, offset as i64], |r| {
        Ok(FtsHit {
            id: FileId(r.get(0)?),
            score: r.get(1)?,
            snippet: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Quote a single term so FTS5 treats it literally (handles `"`, `-`, `*`, etc.).
pub fn escape_term(term: &str) -> String {
    format!("\"{}\"", term.replace('"', "\"\""))
}

/// Quote several words as one exact phrase.
pub fn phrase(words: &str) -> String {
    escape_term(words)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::tests::disc;
    use crate::testutil::temp_db;
    use crate::{files, roots};

    fn seed(c: &Connection) -> (RootId, Vec<FileId>) {
        let root = roots::add(c, "C:/r").unwrap();
        let ids: Vec<_> = files::upsert_discovered(
            c,
            &[
                disc(root.id, "C:/r/cars.txt"),
                disc(root.id, "C:/r/ferrari-notes.md"),
                disc(root.id, "C:/r/other.txt"),
            ],
        )
        .unwrap()
        .into_iter()
        .map(|u| u.id)
        .collect();
        set(
            c,
            ids[0],
            "cars.txt",
            "The Ferrari and the Porsche raced. Ferrari won.",
        )
        .unwrap();
        set(
            c,
            ids[1],
            "ferrari-notes.md",
            "Some unrelated notes about cooking.",
        )
        .unwrap();
        set(c, ids[2], "other.txt", "Nothing to see here.").unwrap();
        (root.id, ids)
    }

    #[test]
    fn search_ranks_and_snippets() {
        let (_d, db) = temp_db();
        let c = db.writer();
        let (_, ids) = seed(&c);
        let hits = search(&c, &escape_term("ferrari"), 10, 0).unwrap();
        assert_eq!(hits.len(), 2);
        let hit_ids: Vec<_> = hits.iter().map(|h| h.id).collect();
        assert!(hit_ids.contains(&ids[0]) && hit_ids.contains(&ids[1]));
        let body = hits.iter().find(|h| h.id == ids[0]).unwrap();
        assert!(
            body.snippet.contains("[Ferrari]"),
            "snippet was {:?}",
            body.snippet
        );
        assert!(hits[0].score <= hits[1].score);
    }

    #[test]
    fn phrase_and_escaping() {
        let (_d, db) = temp_db();
        let c = db.writer();
        let (_, ids) = seed(&c);
        assert_eq!(
            search(&c, &phrase("ferrari won"), 10, 0).unwrap()[0].id,
            ids[0]
        );
        assert!(
            search(&c, &phrase("won ferrari"), 10, 0)
                .unwrap()
                .is_empty()
        );
        // Characters that are FTS5 syntax must be safe when escaped.
        assert!(
            search(&c, &escape_term("c++ -x \"q\" *"), 10, 0)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn diacritics_are_folded() {
        let (_d, db) = temp_db();
        let c = db.writer();
        let (_, ids) = seed(&c);
        set(&c, ids[2], "other.txt", "Café résumé").unwrap();
        assert_eq!(
            search(&c, &escape_term("cafe"), 10, 0).unwrap()[0].id,
            ids[2]
        );
    }

    #[test]
    fn tombstoned_and_deleted_rows_are_excluded() {
        let (_d, db) = temp_db();
        let c = db.writer();
        let (root, ids) = seed(&c);
        files::tombstone_path(&c, "C:/r/cars.txt").unwrap();
        let hits = search(&c, &escape_term("ferrari"), 10, 0).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, ids[1]);

        delete(&c, ids[1]).unwrap();
        assert!(
            search(&c, &escape_term("ferrari"), 10, 0)
                .unwrap()
                .is_empty()
        );

        delete_for_root(&c, root).unwrap();
        assert_eq!(get_text(&c, ids[2]).unwrap(), None);
    }

    #[test]
    fn set_name_keeps_text_and_pagination_works() {
        let (_d, db) = temp_db();
        let c = db.writer();
        let (_, ids) = seed(&c);
        set_name(&c, ids[0], "renamed.txt").unwrap();
        assert!(get_text(&c, ids[0]).unwrap().unwrap().contains("Porsche"));
        assert_eq!(
            search(&c, &escape_term("renamed"), 10, 0).unwrap()[0].id,
            ids[0]
        );

        let page1 = search(&c, &escape_term("ferrari"), 1, 0).unwrap();
        let page2 = search(&c, &escape_term("ferrari"), 1, 1).unwrap();
        assert_eq!(page1.len(), 1);
        assert_eq!(page2.len(), 1);
        assert_ne!(page1[0].id, page2[0].id);
    }
}
