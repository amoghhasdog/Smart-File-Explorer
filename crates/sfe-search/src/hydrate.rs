//! Turn ranked ids into display-ready hits in one query per page.

use serde::Serialize;
use sfe_core::{Category, FileId};
use sfe_db::rusqlite::{Connection, OptionalExtension};

use crate::{Candidate, Result};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub id: FileId,
    pub path: String,
    pub name: String,
    pub ext: String,
    pub category: Category,
    pub size: u64,
    pub mtime: i64,
    pub mime: Option<String>,
    pub snippet: Option<String>,
    pub score: f64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_ms: Option<u64>,
}

pub(crate) fn hydrate(conn: &Connection, page: &[Candidate]) -> Result<Vec<Hit>> {
    let mut stmt = conn.prepare_cached(
        "SELECT f.path, f.name, f.ext, f.category, f.size, f.mtime, f.mime,
                fm.width, fm.height, fm.duration_ms
         FROM files f LEFT JOIN file_meta fm ON fm.file_id = f.id
         WHERE f.id = ?1 AND f.deleted = 0",
    )?;
    let mut out = Vec::with_capacity(page.len());
    for c in page {
        let row = stmt
            .query_row([c.id.0], |r| {
                let cat: String = r.get(3)?;
                Ok(Hit {
                    id: c.id,
                    path: r.get(0)?,
                    name: r.get(1)?,
                    ext: r.get(2)?,
                    category: Category::parse(&cat).unwrap_or(Category::Other),
                    size: r.get::<_, i64>(4)?.max(0) as u64,
                    mtime: r.get(5)?,
                    mime: r.get(6)?,
                    snippet: c.snippet.clone(),
                    score: c.score,
                    width: r.get::<_, Option<i64>>(7)?.map(|v| v as u32),
                    height: r.get::<_, Option<i64>>(8)?.map(|v| v as u32),
                    duration_ms: r.get::<_, Option<i64>>(9)?.map(|v| v.max(0) as u64),
                })
            })
            .optional()?;
        if let Some(h) = row {
            out.push(h);
        }
    }
    Ok(out)
}
