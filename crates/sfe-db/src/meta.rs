//! `file_meta` and `file_text` repositories.

use rusqlite::{Connection, OptionalExtension, Row, params};
use sfe_core::{FileId, FileMeta};

use crate::{Result, now};

fn row_to_meta(r: &Row<'_>) -> rusqlite::Result<FileMeta> {
    Ok(FileMeta {
        width: r.get::<_, Option<i64>>("width")?.map(|v| v as u32),
        height: r.get::<_, Option<i64>>("height")?.map(|v| v as u32),
        duration_ms: r
            .get::<_, Option<i64>>("duration_ms")?
            .map(|v| v.max(0) as u64),
        page_count: r.get::<_, Option<i64>>("page_count")?.map(|v| v as u32),
        word_count: r.get::<_, Option<i64>>("word_count")?.map(|v| v as u32),
        camera_make: r.get("camera_make")?,
        taken_at: r.get("taken_at")?,
        gps_lat: r.get("gps_lat")?,
        gps_lon: r.get("gps_lon")?,
        dominant_colors: r.get("dominant_colors")?,
        hue_hist: r.get("hue_hist")?,
        language: r.get("language")?,
        title: r.get("title")?,
        author: r.get("author")?,
        extra: r.get("extra")?,
    })
}

/// Merge a metadata patch: only `Some` fields overwrite existing values, so
/// several extractors can each contribute their part.
pub fn upsert(conn: &Connection, id: FileId, m: &FileMeta) -> Result<()> {
    conn.execute(
        "INSERT INTO file_meta (file_id, width, height, duration_ms, page_count, word_count, camera_make,
                                taken_at, gps_lat, gps_lon, dominant_colors, hue_hist, language, title,
                                author, extra)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
         ON CONFLICT(file_id) DO UPDATE SET
            width           = coalesce(excluded.width, file_meta.width),
            height          = coalesce(excluded.height, file_meta.height),
            duration_ms     = coalesce(excluded.duration_ms, file_meta.duration_ms),
            page_count      = coalesce(excluded.page_count, file_meta.page_count),
            word_count      = coalesce(excluded.word_count, file_meta.word_count),
            camera_make     = coalesce(excluded.camera_make, file_meta.camera_make),
            taken_at        = coalesce(excluded.taken_at, file_meta.taken_at),
            gps_lat         = coalesce(excluded.gps_lat, file_meta.gps_lat),
            gps_lon         = coalesce(excluded.gps_lon, file_meta.gps_lon),
            dominant_colors = coalesce(excluded.dominant_colors, file_meta.dominant_colors),
            hue_hist        = coalesce(excluded.hue_hist, file_meta.hue_hist),
            language        = coalesce(excluded.language, file_meta.language),
            title           = coalesce(excluded.title, file_meta.title),
            author          = coalesce(excluded.author, file_meta.author),
            extra           = coalesce(excluded.extra, file_meta.extra)",
        params![
            id.0,
            m.width.map(i64::from),
            m.height.map(i64::from),
            m.duration_ms.map(|v| v as i64),
            m.page_count.map(i64::from),
            m.word_count.map(i64::from),
            m.camera_make,
            m.taken_at,
            m.gps_lat,
            m.gps_lon,
            m.dominant_colors,
            m.hue_hist,
            m.language,
            m.title,
            m.author,
            m.extra,
        ],
    )?;
    Ok(())
}

pub fn get(conn: &Connection, id: FileId) -> Result<Option<FileMeta>> {
    Ok(conn
        .query_row(
            "SELECT * FROM file_meta WHERE file_id = ?1",
            [id.0],
            row_to_meta,
        )
        .optional()?)
}

/// Replace all metadata for a file (used when a file changes and is re-extracted).
pub fn clear(conn: &Connection, id: FileId) -> Result<()> {
    conn.execute("DELETE FROM file_meta WHERE file_id = ?1", [id.0])?;
    Ok(())
}

/// Bookkeeping about a text extraction run (the text itself is in `fts`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextInfo {
    pub extractor: String,
    pub extractor_version: u32,
    pub char_count: u32,
    pub truncated: bool,
    pub extracted_at: i64,
}

pub fn set_text_info(
    conn: &Connection,
    id: FileId,
    extractor: &str,
    version: u32,
    char_count: u32,
    truncated: bool,
) -> Result<()> {
    conn.execute(
        "INSERT INTO file_text (file_id, extractor, extractor_version, char_count, truncated, extracted_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(file_id) DO UPDATE SET
            extractor = excluded.extractor, extractor_version = excluded.extractor_version,
            char_count = excluded.char_count, truncated = excluded.truncated,
            extracted_at = excluded.extracted_at",
        params![id.0, extractor, version, char_count, truncated as i64, now()],
    )?;
    Ok(())
}

pub fn get_text_info(conn: &Connection, id: FileId) -> Result<Option<TextInfo>> {
    Ok(conn
        .query_row(
            "SELECT extractor, extractor_version, char_count, truncated, extracted_at
             FROM file_text WHERE file_id = ?1",
            [id.0],
            |r| {
                Ok(TextInfo {
                    extractor: r.get(0)?,
                    extractor_version: r.get::<_, i64>(1)? as u32,
                    char_count: r.get::<_, i64>(2)? as u32,
                    truncated: r.get::<_, i64>(3)? != 0,
                    extracted_at: r.get(4)?,
                })
            },
        )
        .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::tests::disc;
    use crate::testutil::temp_db;
    use crate::{files, roots};

    #[test]
    fn upsert_merges_partial_patches() {
        let (_d, db) = temp_db();
        let c = db.writer();
        let root = roots::add(&c, "C:/r").unwrap();
        let id = files::upsert_discovered(&c, &[disc(root.id, "C:/r/a.jpg")]).unwrap()[0].id;

        upsert(
            &c,
            id,
            &FileMeta {
                width: Some(640),
                height: Some(480),
                ..Default::default()
            },
        )
        .unwrap();
        upsert(
            &c,
            id,
            &FileMeta {
                camera_make: Some("Canon".into()),
                hue_hist: Some(vec![1; 12]),
                ..Default::default()
            },
        )
        .unwrap();
        let m = get(&c, id).unwrap().unwrap();
        assert_eq!(m.width, Some(640));
        assert_eq!(m.height, Some(480));
        assert_eq!(m.camera_make.as_deref(), Some("Canon"));
        assert_eq!(m.hue_hist.as_deref(), Some(&[1u8; 12][..]));

        clear(&c, id).unwrap();
        assert_eq!(get(&c, id).unwrap(), None);
    }

    #[test]
    fn text_info_round_trip() {
        let (_d, db) = temp_db();
        let c = db.writer();
        let root = roots::add(&c, "C:/r").unwrap();
        let id = files::upsert_discovered(&c, &[disc(root.id, "C:/r/a.txt")]).unwrap()[0].id;
        assert_eq!(get_text_info(&c, id).unwrap(), None);
        set_text_info(&c, id, "plain", 1, 123, false).unwrap();
        set_text_info(&c, id, "plain", 2, 456, true).unwrap();
        let t = get_text_info(&c, id).unwrap().unwrap();
        assert_eq!(t.extractor, "plain");
        assert_eq!(t.extractor_version, 2);
        assert_eq!(t.char_count, 456);
        assert!(t.truncated);
    }
}
