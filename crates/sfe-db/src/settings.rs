//! Key/value settings.

use rusqlite::{Connection, OptionalExtension, params};

use crate::Result;

pub fn get(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
            r.get(0)
        })
        .optional()?)
}

pub fn set(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO settings(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, key: &str) -> Result<()> {
    conn.execute("DELETE FROM settings WHERE key = ?1", [key])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::temp_db;

    #[test]
    fn set_get_overwrite_delete() {
        let (_d, db) = temp_db();
        let c = db.writer();
        assert_eq!(get(&c, "a").unwrap(), None);
        set(&c, "a", "1").unwrap();
        set(&c, "a", "2").unwrap();
        assert_eq!(get(&c, "a").unwrap().as_deref(), Some("2"));
        delete(&c, "a").unwrap();
        assert_eq!(get(&c, "a").unwrap(), None);
    }
}
