//! Forward-only schema migrations keyed by `PRAGMA user_version`.
//!
//! Append a new entry to [`MIGRATIONS`] for every schema change; never edit a
//! shipped one. Each entry runs inside a transaction and bumps `user_version`.

use rusqlite::Connection;

use crate::{DbError, Result};

const MIGRATIONS: &[&str] = &[
    include_str!("sql/v1_initial.sql"),
    include_str!("sql/v2_chunks_autoincrement.sql"),
];

pub fn supported_version() -> i64 {
    MIGRATIONS.len() as i64
}

pub fn current_version(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

pub fn apply(conn: &Connection) -> Result<()> {
    let current = current_version(conn)?;
    let supported = supported_version();
    if current > supported {
        return Err(DbError::SchemaTooNew {
            found: current,
            supported,
        });
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let target = i as i64 + 1;
        tracing::info!(from = i, to = target, "applying migration");
        conn.execute_batch("BEGIN")?;
        let result = conn
            .execute_batch(sql)
            .and_then(|_| conn.pragma_update(None, "user_version", target));
        match result {
            Ok(()) => conn.execute_batch("COMMIT")?,
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(e.into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::temp_db;

    #[test]
    fn fresh_db_is_at_latest_version_and_has_all_tables() {
        let (_d, db) = temp_db();
        let conn = db.writer();
        assert_eq!(current_version(&conn).unwrap(), supported_version());
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type IN ('table','view') ORDER BY name")
            .unwrap();
        let names: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        for required in [
            "roots",
            "files",
            "file_meta",
            "file_text",
            "fts",
            "chunks",
            "jobs",
            "models",
            "settings",
        ] {
            assert!(
                names.contains(&required.to_string()),
                "missing table {required}"
            );
        }
    }

    #[test]
    fn reopening_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.db");
        crate::Database::open(&p).unwrap();
        let db = crate::Database::open(&p).unwrap();
        assert_eq!(current_version(&db.writer()).unwrap(), supported_version());
    }

    #[test]
    fn refuses_database_from_the_future() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.db");
        {
            let c = Connection::open(&p).unwrap();
            c.pragma_update(None, "user_version", 999).unwrap();
        }
        let err = crate::Database::open(&p).unwrap_err();
        assert!(matches!(err, DbError::SchemaTooNew { found: 999, .. }));
    }
}
