//! `files` repository.

use rusqlite::{Connection, OptionalExtension, Row, params};
use sfe_core::{Category, DiscoveredFile, FileId, FileKind, FileRecord, IndexState, RootId};

use crate::{DbError, Result, now};

const COLS: &str = "id, root_id, path, parent_path, name, ext, kind, category, size, mtime, ctime, \
                    content_hash, mime, index_state, index_error, indexed_at, updated_at, deleted";

fn row_to_file(r: &Row<'_>) -> Result<FileRecord> {
    let kind: String = r.get("kind")?;
    let category: String = r.get("category")?;
    let state: i64 = r.get("index_state")?;
    Ok(FileRecord {
        id: FileId(r.get("id")?),
        root_id: RootId(r.get("root_id")?),
        path: r.get("path")?,
        parent_path: r.get("parent_path")?,
        name: r.get("name")?,
        ext: r.get("ext")?,
        kind: FileKind::parse(&kind).ok_or_else(|| DbError::invalid("kind", &kind))?,
        category: Category::parse(&category)
            .ok_or_else(|| DbError::invalid("category", &category))?,
        size: r.get::<_, i64>("size")?.max(0) as u64,
        mtime: r.get("mtime")?,
        ctime: r.get("ctime")?,
        content_hash: r.get("content_hash")?,
        mime: r.get("mime")?,
        index_state: IndexState::from_i64(state)
            .ok_or_else(|| DbError::invalid("index_state", state))?,
        index_error: r.get("index_error")?,
        indexed_at: r.get("indexed_at")?,
        updated_at: r.get("updated_at")?,
        deleted: r.get::<_, i64>("deleted")? != 0,
    })
}

/// Outcome of upserting one discovered entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Upserted {
    pub id: FileId,
    /// `true` when the row is new or its `(size, mtime)` changed, i.e. the
    /// pipeline must (re)process it. Unchanged rows that were already fully
    /// indexed report `false`.
    pub needs_index: bool,
}

/// Insert or refresh rows from a directory listing. A changed `(size, mtime)`
/// resets `index_state` to `Discovered`; an unchanged row only has its
/// `updated_at` (= "last seen") bumped so [`tombstone_unseen`] can find
/// files that vanished. Also revives tombstoned rows that reappear.
pub fn upsert_discovered(conn: &Connection, batch: &[DiscoveredFile]) -> Result<Vec<Upserted>> {
    let mut stmt = conn.prepare_cached(
        "INSERT INTO files (root_id, path, parent_path, name, ext, kind, category, size, mtime, ctime,
                            index_state, updated_at, deleted)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, ?11, 0)
         ON CONFLICT(path) DO UPDATE SET
            root_id     = excluded.root_id,
            parent_path = excluded.parent_path,
            name        = excluded.name,
            ext         = excluded.ext,
            kind        = excluded.kind,
            category    = excluded.category,
            size        = excluded.size,
            mtime       = excluded.mtime,
            ctime       = excluded.ctime,
            updated_at  = excluded.updated_at,
            deleted     = 0,
            index_state = CASE
                WHEN files.deleted = 1 OR files.size <> excluded.size OR files.mtime <> excluded.mtime
                THEN 0 ELSE files.index_state END,
            index_error = CASE
                WHEN files.deleted = 1 OR files.size <> excluded.size OR files.mtime <> excluded.mtime
                THEN NULL ELSE files.index_error END,
            content_hash = CASE
                WHEN files.size <> excluded.size OR files.mtime <> excluded.mtime
                THEN NULL ELSE files.content_hash END
         RETURNING id, index_state",
    )?;
    let ts = now();
    let mut out = Vec::with_capacity(batch.len());
    for f in batch {
        let (id, state): (i64, i64) = stmt.query_row(
            params![
                f.root_id.0,
                f.path,
                f.parent_path,
                f.name,
                f.ext,
                f.kind.as_str(),
                f.category().as_str(),
                f.size as i64,
                f.mtime,
                f.ctime,
                ts,
            ],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        out.push(Upserted {
            id: FileId(id),
            needs_index: state == IndexState::Discovered as i64,
        });
    }
    Ok(out)
}

/// Tombstone every live file under `root` whose `updated_at` predates
/// `scan_started_at` — i.e. it was not seen by a scan that began then.
/// Returns the tombstoned ids so callers can drop FTS/vector entries.
pub fn tombstone_unseen(
    conn: &Connection,
    root: RootId,
    scan_started_at: i64,
) -> Result<Vec<FileId>> {
    let mut stmt = conn.prepare(
        "UPDATE files SET deleted = 1
         WHERE root_id = ?1 AND deleted = 0 AND updated_at < ?2
         RETURNING id",
    )?;
    let ids = stmt
        .query_map(params![root.0, scan_started_at], |r| r.get::<_, i64>(0))?
        .map(|r| r.map(FileId))
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids)
}

/// Tombstone a single path (watcher delete event). Returns the id if it existed.
pub fn tombstone_path(conn: &Connection, path: &str) -> Result<Option<FileId>> {
    Ok(conn
        .query_row(
            "UPDATE files SET deleted = 1 WHERE path = ?1 AND deleted = 0 RETURNING id",
            [path],
            |r| r.get::<_, i64>(0).map(FileId),
        )
        .optional()?)
}

/// Permanently remove tombstoned rows (cascades meta/text/chunks/jobs).
pub fn purge_tombstoned(conn: &Connection, limit: usize) -> Result<Vec<FileId>> {
    let mut stmt = conn.prepare(
        "DELETE FROM files WHERE id IN (SELECT id FROM files WHERE deleted = 1 LIMIT ?1) RETURNING id",
    )?;
    let ids = stmt
        .query_map([limit as i64], |r| r.get::<_, i64>(0))?
        .map(|r| r.map(FileId))
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids)
}

pub fn get(conn: &Connection, id: FileId) -> Result<Option<FileRecord>> {
    let mut stmt = conn.prepare_cached(&format!("SELECT {COLS} FROM files WHERE id = ?1"))?;
    let mut rows = stmt.query([id.0])?;
    match rows.next()? {
        Some(r) => Ok(Some(row_to_file(r)?)),
        None => Ok(None),
    }
}

pub fn get_by_path(conn: &Connection, path: &str) -> Result<Option<FileRecord>> {
    let mut stmt = conn.prepare_cached(&format!("SELECT {COLS} FROM files WHERE path = ?1"))?;
    let mut rows = stmt.query([path])?;
    match rows.next()? {
        Some(r) => Ok(Some(row_to_file(r)?)),
        None => Ok(None),
    }
}

pub fn get_many(conn: &Connection, ids: &[FileId]) -> Result<Vec<FileRecord>> {
    let mut stmt = conn.prepare_cached(&format!("SELECT {COLS} FROM files WHERE id = ?1"))?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let mut rows = stmt.query([id.0])?;
        if let Some(r) = rows.next()? {
            out.push(row_to_file(r)?);
        }
    }
    Ok(out)
}

pub fn set_state(
    conn: &Connection,
    id: FileId,
    state: IndexState,
    error: Option<&str>,
) -> Result<()> {
    conn.execute(
        "UPDATE files SET index_state = ?2, index_error = ?3, indexed_at = ?4 WHERE id = ?1",
        params![id.0, state as i64, error, now()],
    )?;
    Ok(())
}

pub fn set_mime(conn: &Connection, id: FileId, mime: Option<&str>) -> Result<()> {
    conn.execute(
        "UPDATE files SET mime = ?2 WHERE id = ?1",
        params![id.0, mime],
    )?;
    Ok(())
}

pub fn set_content_hash(conn: &Connection, id: FileId, hash: &[u8]) -> Result<()> {
    conn.execute(
        "UPDATE files SET content_hash = ?2 WHERE id = ?1",
        params![id.0, hash],
    )?;
    Ok(())
}

/// Live files per index state, for progress reporting.
pub fn count_by_state(conn: &Connection) -> Result<Vec<(IndexState, u64)>> {
    let mut stmt = conn.prepare(
        "SELECT index_state, count(*) FROM files WHERE deleted = 0 AND kind = 'file' GROUP BY index_state",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
    let mut out = Vec::new();
    for row in rows {
        let (s, n) = row?;
        if let Some(state) = IndexState::from_i64(s) {
            out.push((state, n.max(0) as u64));
        }
    }
    Ok(out)
}

pub fn count_live(conn: &Connection) -> Result<u64> {
    let n: i64 = conn.query_row(
        "SELECT count(*) FROM files WHERE deleted = 0 AND kind = 'file'",
        [],
        |r| r.get(0),
    )?;
    Ok(n.max(0) as u64)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::roots;
    use crate::testutil::temp_db;

    pub(crate) fn disc(root_id: RootId, path: &str) -> DiscoveredFile {
        disc_with(root_id, path, 10, 1_000)
    }

    pub(crate) fn disc_with(root_id: RootId, path: &str, size: u64, mtime: i64) -> DiscoveredFile {
        let (parent, name) = path.rsplit_once('/').unwrap();
        let ext = name
            .rsplit_once('.')
            .map(|(_, e)| e.to_lowercase())
            .unwrap_or_default();
        DiscoveredFile {
            root_id,
            path: path.to_string(),
            parent_path: parent.to_string(),
            name: name.to_string(),
            ext,
            kind: FileKind::File,
            size,
            mtime,
            ctime: None,
        }
    }

    #[test]
    fn upsert_inserts_then_detects_change_and_no_change() {
        let (_d, db) = temp_db();
        let c = db.writer();
        let root = roots::add(&c, "C:/r").unwrap();

        let first = upsert_discovered(&c, &[disc_with(root.id, "C:/r/a.pdf", 10, 100)]).unwrap();
        assert!(first[0].needs_index);
        let id = first[0].id;
        set_state(&c, id, IndexState::Embedded, None).unwrap();

        // unchanged → keeps Embedded, needs_index false
        let same = upsert_discovered(&c, &[disc_with(root.id, "C:/r/a.pdf", 10, 100)]).unwrap();
        assert_eq!(same[0].id, id);
        assert!(!same[0].needs_index);
        assert_eq!(
            get(&c, id).unwrap().unwrap().index_state,
            IndexState::Embedded
        );

        // mtime changed → reset to Discovered
        let changed = upsert_discovered(&c, &[disc_with(root.id, "C:/r/a.pdf", 10, 200)]).unwrap();
        assert!(changed[0].needs_index);
        let rec = get(&c, id).unwrap().unwrap();
        assert_eq!(rec.index_state, IndexState::Discovered);
        assert_eq!(rec.category, Category::Document);
        assert_eq!(rec.ext, "pdf");
        assert_eq!(rec.mtime, 200);
    }

    #[test]
    fn tombstone_unseen_only_affects_stale_rows_of_that_root() {
        let (_d, db) = temp_db();
        let c = db.writer();
        let r1 = roots::add(&c, "C:/r1").unwrap();
        let r2 = roots::add(&c, "C:/r2").unwrap();
        upsert_discovered(
            &c,
            &[disc(r1.id, "C:/r1/old.txt"), disc(r2.id, "C:/r2/other.txt")],
        )
        .unwrap();
        // Simulate rows seen by a previous scan.
        c.execute("UPDATE files SET updated_at = 5", []).unwrap();
        let scan_start = 10;
        let kept = upsert_discovered(&c, &[disc(r1.id, "C:/r1/new.txt")]).unwrap();
        let gone = tombstone_unseen(&c, r1.id, scan_start).unwrap();
        assert_eq!(gone.len(), 1);
        assert!(get(&c, gone[0]).unwrap().unwrap().deleted);
        assert!(!get(&c, kept[0].id).unwrap().unwrap().deleted);
        assert!(!get_by_path(&c, "C:/r2/other.txt").unwrap().unwrap().deleted);
        assert_eq!(count_live(&c).unwrap(), 2);
    }

    #[test]
    fn reappearing_tombstoned_file_is_revived_and_reindexed() {
        let (_d, db) = temp_db();
        let c = db.writer();
        let root = roots::add(&c, "C:/r").unwrap();
        let id = upsert_discovered(&c, &[disc(root.id, "C:/r/a.txt")]).unwrap()[0].id;
        set_state(&c, id, IndexState::Text, None).unwrap();
        assert_eq!(tombstone_path(&c, "C:/r/a.txt").unwrap(), Some(id));
        assert_eq!(tombstone_path(&c, "C:/r/a.txt").unwrap(), None);
        let back = upsert_discovered(&c, &[disc(root.id, "C:/r/a.txt")]).unwrap()[0];
        assert_eq!(back.id, id);
        assert!(back.needs_index);
        let rec = get(&c, id).unwrap().unwrap();
        assert!(!rec.deleted);
        assert_eq!(rec.index_state, IndexState::Discovered);
    }

    #[test]
    fn purge_removes_tombstoned_rows_and_cascades() {
        let (_d, db) = temp_db();
        let c = db.writer();
        let root = roots::add(&c, "C:/r").unwrap();
        let id = upsert_discovered(&c, &[disc(root.id, "C:/r/a.txt")]).unwrap()[0].id;
        crate::meta::upsert(
            &c,
            id,
            &sfe_core::FileMeta {
                width: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        tombstone_path(&c, "C:/r/a.txt").unwrap();
        assert_eq!(purge_tombstoned(&c, 100).unwrap(), vec![id]);
        assert_eq!(get(&c, id).unwrap(), None);
        assert_eq!(crate::meta::get(&c, id).unwrap(), None);
    }

    #[test]
    fn state_mime_hash_and_counts() {
        let (_d, db) = temp_db();
        let c = db.writer();
        let root = roots::add(&c, "C:/r").unwrap();
        let ids: Vec<_> = upsert_discovered(
            &c,
            &[
                disc(root.id, "C:/r/a.txt"),
                disc(root.id, "C:/r/b.txt"),
                disc(root.id, "C:/r/c.txt"),
            ],
        )
        .unwrap()
        .into_iter()
        .map(|u| u.id)
        .collect();
        set_state(&c, ids[0], IndexState::Failed, Some("boom")).unwrap();
        set_state(&c, ids[1], IndexState::Metadata, None).unwrap();
        set_mime(&c, ids[1], Some("text/plain")).unwrap();
        set_content_hash(&c, ids[1], &[1, 2, 3]).unwrap();

        let b = get(&c, ids[1]).unwrap().unwrap();
        assert_eq!(b.mime.as_deref(), Some("text/plain"));
        assert_eq!(b.content_hash, Some(vec![1, 2, 3]));
        assert!(b.indexed_at.is_some());
        assert_eq!(
            get(&c, ids[0]).unwrap().unwrap().index_error.as_deref(),
            Some("boom")
        );

        let mut counts = count_by_state(&c).unwrap();
        counts.sort();
        assert_eq!(
            counts,
            vec![
                (IndexState::Discovered, 1),
                (IndexState::Metadata, 1),
                (IndexState::Failed, 1)
            ]
        );
        assert_eq!(get_many(&c, &ids).unwrap().len(), 3);
    }
}
