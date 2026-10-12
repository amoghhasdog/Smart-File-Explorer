//! Custom SQL functions registered on every connection.

use rusqlite::Connection;
use rusqlite::functions::FunctionFlags;

use crate::Result;

/// `hue_share(hue_hist BLOB, bins TEXT) -> INTEGER`
///
/// Sums the given comma-separated bins of a 12-byte hue histogram, so a
/// colour predicate becomes `hue_share(fm.hue_hist, '7,8') >= 102`.
/// NULL histogram → 0.
pub(crate) fn register(conn: &Connection) -> Result<()> {
    conn.create_scalar_function(
        "hue_share",
        2,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let hist: Option<Vec<u8>> = ctx.get(0)?;
            let bins: String = ctx.get(1)?;
            let Some(hist) = hist else { return Ok(0i64) };
            let sum: i64 = bins
                .split(',')
                .filter_map(|b| b.trim().parse::<usize>().ok())
                .filter_map(|b| hist.get(b))
                .map(|&v| v as i64)
                .sum();
            Ok(sum)
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::testutil::temp_db;

    #[test]
    fn hue_share_sums_requested_bins() {
        let (_d, db) = temp_db();
        let c = db.writer();
        let hist: Vec<u8> = (0..12).map(|i| i * 10).collect();
        let v: i64 = c
            .query_row("SELECT hue_share(?1, '7,8')", [&hist], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 70 + 80);
        let v: i64 = c
            .query_row("SELECT hue_share(?1, '0,11,99')", [&hist], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 110);
        let v: i64 = c
            .query_row("SELECT hue_share(NULL, '1')", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 0);
        // Reader connections have it too.
        let r = db.reader().unwrap();
        let v: i64 = r
            .query_row("SELECT hue_share(?1, '2')", [&hist], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 20);
    }
}
