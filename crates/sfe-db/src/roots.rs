//! `roots` repository.

use rusqlite::{Connection, OptionalExtension, Row, params};
use sfe_core::{Root, RootId};

use crate::{Result, now};

fn row_to_root(r: &Row<'_>) -> rusqlite::Result<Root> {
    Ok(Root {
        id: RootId(r.get("id")?),
        path: r.get("path")?,
        added_at: r.get("added_at")?,
        enabled: r.get::<_, i64>("enabled")? != 0,
        last_scan_at: r.get("last_scan_at")?,
    })
}

const COLS: &str = "id, path, added_at, enabled, last_scan_at";

/// Insert a root. Returns the existing root unchanged if `path` is already registered.
pub fn add(conn: &Connection, path: &str) -> Result<Root> {
    conn.execute(
        "INSERT OR IGNORE INTO roots(path, added_at, enabled) VALUES (?1, ?2, 1)",
        params![path, now()],
    )?;
    get_by_path(conn, path).map(|r| r.expect("just inserted"))
}

pub fn get(conn: &Connection, id: RootId) -> Result<Option<Root>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM roots WHERE id = ?1"),
            [id.0],
            row_to_root,
        )
        .optional()?)
}

pub fn get_by_path(conn: &Connection, path: &str) -> Result<Option<Root>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM roots WHERE path = ?1"),
            [path],
            row_to_root,
        )
        .optional()?)
}

pub fn list(conn: &Connection) -> Result<Vec<Root>> {
    let mut stmt = conn.prepare(&format!("SELECT {COLS} FROM roots ORDER BY id"))?;
    let rows = stmt.query_map([], row_to_root)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Delete a root and (via `ON DELETE CASCADE`) all its files, metadata, jobs
/// and chunks. FTS rows are not cascaded; the caller removes them via
/// [`crate::fts::delete_for_root`] first.
pub fn remove(conn: &Connection, id: RootId) -> Result<bool> {
    Ok(conn.execute("DELETE FROM roots WHERE id = ?1", [id.0])? == 1)
}

pub fn set_enabled(conn: &Connection, id: RootId, enabled: bool) -> Result<()> {
    conn.execute(
        "UPDATE roots SET enabled = ?2 WHERE id = ?1",
        params![id.0, enabled as i64],
    )?;
    Ok(())
}

pub fn set_last_scan(conn: &Connection, id: RootId, at: i64) -> Result<()> {
    conn.execute(
        "UPDATE roots SET last_scan_at = ?2 WHERE id = ?1",
        params![id.0, at],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::temp_db;

    #[test]
    fn add_is_idempotent_and_list_is_ordered() {
        let (_d, db) = temp_db();
        let c = db.writer();
        let a = add(&c, "C:/a").unwrap();
        let b = add(&c, "C:/b").unwrap();
        let a2 = add(&c, "C:/a").unwrap();
        assert_eq!(a, a2);
        assert!(a.enabled);
        assert_eq!(list(&c).unwrap(), vec![a.clone(), b]);
        assert_eq!(get(&c, a.id).unwrap(), Some(a));
    }

    #[test]
    fn remove_cascades_to_files() {
        let (_d, db) = temp_db();
        let c = db.writer();
        let root = add(&c, "C:/a").unwrap();
        crate::files::upsert_discovered(&c, &[crate::files::tests::disc(root.id, "C:/a/x.txt")])
            .unwrap();
        assert!(remove(&c, root.id).unwrap());
        assert!(!remove(&c, root.id).unwrap());
        let n: i64 = c
            .query_row("SELECT count(*) FROM files", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn enabled_and_last_scan_update() {
        let (_d, db) = temp_db();
        let c = db.writer();
        let root = add(&c, "C:/a").unwrap();
        set_enabled(&c, root.id, false).unwrap();
        set_last_scan(&c, root.id, 123).unwrap();
        let r = get(&c, root.id).unwrap().unwrap();
        assert!(!r.enabled);
        assert_eq!(r.last_scan_at, Some(123));
    }
}
