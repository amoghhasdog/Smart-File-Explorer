//! SQLite persistence: schema migrations, FTS5 table, and typed repositories.
//!
//! Only this crate issues SQL. The [`Database`] owns one **writer** connection
//! (SQLite allows a single writer; serialising writes behind a mutex avoids
//! `SQLITE_BUSY`) and hands out fresh **reader** connections that, thanks to
//! WAL mode, never block on the writer.
//!
//! Repositories are plain functions over `&Connection` in the submodules
//! ([`roots`], [`files`], [`meta`], [`fts`], [`jobs`], [`settings`]) so they
//! compose inside a single transaction and are trivially unit-testable.

pub mod chunks;
mod error;
pub mod files;
pub mod fts;
mod functions;
pub mod jobs;
pub mod meta;
mod migrations;
pub mod roots;
pub mod settings;

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OpenFlags};

pub use error::DbError;
pub use rusqlite;

pub type Result<T> = std::result::Result<T, DbError>;

/// Current unix time in seconds. Centralised so tests can reason about it.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Handle to the on-disk index database.
pub struct Database {
    path: PathBuf,
    writer: Mutex<Connection>,
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database")
            .field("path", &self.path)
            .finish()
    }
}

impl Database {
    /// Open (creating if needed) the database at `path` and bring the schema
    /// up to date.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let conn = Connection::open(&path)?;
        configure(&conn)?;
        migrations::apply(&conn)?;
        tracing::info!(path = %path.display(), "database opened");
        Ok(Self {
            path,
            writer: Mutex::new(conn),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Exclusive access to the writer connection. Keep the guard short-lived.
    pub fn writer(&self) -> MutexGuard<'_, Connection> {
        self.writer.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A new read-only connection. Cheap to open; callers may cache one per task.
    pub fn reader(&self) -> Result<Connection> {
        let conn = Connection::open_with_flags(
            &self.path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        configure(&conn)?;
        Ok(conn)
    }

    /// Run `f` inside a write transaction; commits on `Ok`, rolls back on `Err`.
    pub fn transaction<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let mut conn = self.writer();
        let tx = conn.transaction()?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }

    /// `PRAGMA quick_check`; `Ok(())` when the file is healthy.
    pub fn quick_check(&self) -> Result<()> {
        let conn = self.writer();
        let status: String = conn.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
        if status == "ok" {
            Ok(())
        } else {
            Err(DbError::Corrupt(status))
        }
    }
}

fn configure(conn: &Connection) -> Result<()> {
    functions::register(conn)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA temp_store = MEMORY;
         PRAGMA cache_size = -65536;  -- 64 MiB
         PRAGMA busy_timeout = 5000;",
    )?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod testutil {
    use super::*;

    /// A fresh database in a temp dir. The `TempDir` must outlive the `Database`.
    pub fn temp_db() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path().join("test.db")).unwrap();
        (dir, db)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_in_wal_mode_with_foreign_keys() {
        let (_d, db) = testutil::temp_db();
        let conn = db.writer();
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        let fk: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fk, 1);
    }

    #[test]
    fn reader_sees_committed_writes_and_cannot_write() {
        let (_d, db) = testutil::temp_db();
        db.transaction(|c| settings::set(c, "k", "v")).unwrap();
        let r = db.reader().unwrap();
        assert_eq!(settings::get(&r, "k").unwrap().as_deref(), Some("v"));
        assert!(settings::set(&r, "k", "w").is_err());
    }

    #[test]
    fn transaction_rolls_back_on_error() {
        let (_d, db) = testutil::temp_db();
        let res: Result<()> = db.transaction(|c| {
            settings::set(c, "k", "v")?;
            Err(DbError::Corrupt("boom".into()))
        });
        assert!(res.is_err());
        assert_eq!(settings::get(&db.writer(), "k").unwrap(), None);
    }

    #[test]
    fn quick_check_passes_on_fresh_db() {
        let (_d, db) = testutil::temp_db();
        db.quick_check().unwrap();
    }
}
