//! `chunks` repository: embedded text/image chunks and their vectors.
//! Vectors are stored little-endian f32 so the HNSW index can be rebuilt.

use rusqlite::{Connection, params};
use sfe_core::{ChunkId, FileId};

use crate::Result;

#[derive(Debug, Clone, PartialEq)]
pub struct NewChunk<'a> {
    pub ord: u32,
    pub modality: &'a str,
    pub text: Option<&'a str>,
    pub model: &'a str,
    pub vec: &'a [f32],
}

pub fn encode(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

pub fn decode(b: &[u8]) -> Vec<f32> {
    b.as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect()
}

/// Replace all chunks of a file for `model`. Returns ids in `ord` order.
pub fn replace_for_file(
    conn: &Connection,
    file: FileId,
    chunks: &[NewChunk<'_>],
) -> Result<Vec<ChunkId>> {
    delete_for_file(conn, file)?;
    let mut stmt = conn.prepare_cached(
        "INSERT INTO chunks (file_id, ord, modality, text, model, dim, vec)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) RETURNING id",
    )?;
    let mut ids = Vec::with_capacity(chunks.len());
    for c in chunks {
        let id: i64 = stmt.query_row(
            params![
                file.0,
                c.ord,
                c.modality,
                c.text,
                c.model,
                c.vec.len() as i64,
                encode(c.vec)
            ],
            |r| r.get(0),
        )?;
        ids.push(ChunkId(id));
    }
    Ok(ids)
}

/// Remove a file's chunks; returns the removed ids (to drop from the index).
pub fn delete_for_file(conn: &Connection, file: FileId) -> Result<Vec<ChunkId>> {
    let mut stmt = conn.prepare_cached("DELETE FROM chunks WHERE file_id = ?1 RETURNING id")?;
    let ids = stmt
        .query_map([file.0], |r| r.get::<_, i64>(0).map(ChunkId))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids)
}

/// Map chunk ids to their file ids (search joins through this).
pub fn file_ids(conn: &Connection, chunks: &[ChunkId]) -> Result<Vec<(ChunkId, FileId)>> {
    let mut stmt = conn.prepare_cached("SELECT file_id FROM chunks WHERE id = ?1")?;
    let mut out = Vec::with_capacity(chunks.len());
    for c in chunks {
        let mut rows = stmt.query([c.0])?;
        if let Some(r) = rows.next()? {
            out.push((*c, FileId(r.get(0)?)));
        }
    }
    Ok(out)
}

/// Every chunk id under a root (to drop from vector indexes before the root goes).
pub fn ids_for_root(conn: &Connection, root: sfe_core::RootId) -> Result<Vec<ChunkId>> {
    let mut stmt = conn.prepare(
        "SELECT c.id FROM chunks c JOIN files f ON f.id = c.file_id WHERE f.root_id = ?1",
    )?;
    let ids = stmt
        .query_map([root.0], |r| r.get::<_, i64>(0).map(ChunkId))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids)
}

pub fn count_all(conn: &Connection) -> Result<(u64, u64)> {
    let (t, i): (i64, i64) = conn.query_row(
        "SELECT coalesce(sum(modality = 'text'), 0), coalesce(sum(modality = 'image'), 0) FROM chunks",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok((t.max(0) as u64, i.max(0) as u64))
}

pub fn count(conn: &Connection, model: &str, modality: &str) -> Result<u64> {
    let n: i64 = conn.query_row(
        "SELECT count(*) FROM chunks WHERE model = ?1 AND modality = ?2",
        params![model, modality],
        |r| r.get(0),
    )?;
    Ok(n.max(0) as u64)
}

/// Stream every stored vector of a model (for rebuilding the ANN index).
pub fn for_each_vector(
    conn: &Connection,
    model: &str,
    modality: &str,
    mut f: impl FnMut(ChunkId, Vec<f32>),
) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.vec FROM chunks c JOIN files fl ON fl.id = c.file_id
         WHERE c.model = ?1 AND c.modality = ?2 AND fl.deleted = 0",
    )?;
    let rows = stmt.query_map(params![model, modality], |r| {
        Ok((ChunkId(r.get(0)?), r.get::<_, Vec<u8>>(1)?))
    })?;
    for row in rows {
        let (id, bytes) = row?;
        f(id, decode(&bytes));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::tests::disc;
    use crate::testutil::temp_db;
    use crate::{files, roots};

    #[test]
    fn round_trip_replace_and_rebuild() {
        let (_d, db) = temp_db();
        let c = db.writer();
        let root = roots::add(&c, "C:/r").unwrap();
        let id = files::upsert_discovered(&c, &[disc(root.id, "C:/r/a.txt")]).unwrap()[0].id;

        let v1 = vec![0.1f32, 0.2, 0.3];
        let v2 = vec![1.0f32, 0.0, -1.0];
        let ids = replace_for_file(
            &c,
            id,
            &[
                NewChunk {
                    ord: 0,
                    modality: "text",
                    text: Some("one"),
                    model: "m",
                    vec: &v1,
                },
                NewChunk {
                    ord: 1,
                    modality: "text",
                    text: Some("two"),
                    model: "m",
                    vec: &v2,
                },
            ],
        )
        .unwrap();
        assert_eq!(ids.len(), 2);
        assert_eq!(count(&c, "m", "text").unwrap(), 2);
        assert_eq!(
            file_ids(&c, &ids).unwrap(),
            vec![(ids[0], id), (ids[1], id)]
        );

        let mut seen = Vec::new();
        for_each_vector(&c, "m", "text", |cid, v| seen.push((cid, v))).unwrap();
        assert_eq!(seen, vec![(ids[0], v1.clone()), (ids[1], v2.clone())]);

        // replace drops the old rows
        let ids2 = replace_for_file(
            &c,
            id,
            &[NewChunk {
                ord: 0,
                modality: "text",
                text: None,
                model: "m",
                vec: &v1,
            }],
        )
        .unwrap();
        assert_eq!(count(&c, "m", "text").unwrap(), 1);
        assert_ne!(ids2[0], ids[0]);
        assert_eq!(delete_for_file(&c, id).unwrap(), ids2);
        assert_eq!(decode(&encode(&v2)), v2);
    }
}
