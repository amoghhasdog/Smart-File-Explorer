//! Durable pipeline job queue.
//!
//! One job per `(file, stage)`. Workers *lease* jobs for a bounded time; a
//! lease that expires (crash, hang) makes the job pickable again. Jobs are
//! picked lowest stage first so cheap stages drain across the whole corpus
//! before expensive ones start, then by priority, then FIFO.

use rusqlite::{Connection, params};
use sfe_core::{FileId, Job, Stage};

use crate::{Result, now};

pub const MAX_ATTEMPTS: u32 = 3;

/// Queue a job; if one already exists for this `(file, stage)` keep it but
/// raise its priority to `max(existing, priority)` and clear any backoff.
pub fn enqueue(conn: &Connection, file: FileId, stage: Stage, priority: i32) -> Result<()> {
    conn.execute(
        "INSERT INTO jobs (file_id, stage, priority, attempts, not_before, created_at)
         VALUES (?1, ?2, ?3, 0, 0, ?4)
         ON CONFLICT(file_id, stage) DO UPDATE SET
            priority = max(jobs.priority, excluded.priority),
            not_before = 0",
        params![file.0, stage as i64, priority, now()],
    )?;
    Ok(())
}

pub fn enqueue_many(
    conn: &Connection,
    files: &[FileId],
    stage: Stage,
    priority: i32,
) -> Result<()> {
    let mut stmt = conn.prepare_cached(
        "INSERT INTO jobs (file_id, stage, priority, attempts, not_before, created_at)
         VALUES (?1, ?2, ?3, 0, 0, ?4)
         ON CONFLICT(file_id, stage) DO UPDATE SET
            priority = max(jobs.priority, excluded.priority),
            not_before = 0",
    )?;
    let ts = now();
    for f in files {
        stmt.execute(params![f.0, stage as i64, priority, ts])?;
    }
    Ok(())
}

/// Lease up to `limit` runnable jobs for `lease_secs`. Optionally restrict to
/// one stage (a worker pool dedicated to embedding, for instance).
pub fn lease(
    conn: &Connection,
    stage: Option<Stage>,
    limit: usize,
    lease_secs: i64,
) -> Result<Vec<Job>> {
    let ts = now();
    let mut stmt = conn.prepare_cached(
        "UPDATE jobs SET leased_until = ?1 + ?2
         WHERE id IN (
            SELECT jobs.id FROM jobs JOIN files ON files.id = jobs.file_id
            WHERE jobs.leased_until <= ?1 AND jobs.not_before <= ?1 AND files.deleted = 0
              AND (?3 IS NULL OR jobs.stage = ?3)
            ORDER BY jobs.stage ASC, jobs.priority DESC, jobs.id ASC
            LIMIT ?4)
         RETURNING id, file_id, stage, priority, attempts",
    )?;
    let rows = stmt.query_map(
        params![ts, lease_secs, stage.map(|s| s as i64), limit as i64],
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i32>(3)?,
                r.get::<_, i64>(4)?,
            ))
        },
    )?;
    let mut out = Vec::new();
    for row in rows {
        let (id, file_id, stage, priority, attempts) = row?;
        if let Some(stage) = Stage::from_i64(stage) {
            out.push(Job {
                id,
                file_id: FileId(file_id),
                stage,
                priority,
                attempts: attempts.max(0) as u32,
            });
        }
    }
    // RETURNING yields rows in update order, not the subquery's ORDER BY.
    out.sort_by_key(|j| (j.stage, std::cmp::Reverse(j.priority), j.id));
    Ok(out)
}

/// Job finished successfully: remove it.
pub fn complete(conn: &Connection, job_id: i64) -> Result<()> {
    conn.execute("DELETE FROM jobs WHERE id = ?1", [job_id])?;
    Ok(())
}

/// Job failed. Returns `true` if it will be retried (with exponential backoff),
/// `false` if attempts are exhausted and the job was dropped — the caller then
/// marks the file `Failed`.
pub fn fail(conn: &Connection, job_id: i64) -> Result<bool> {
    let attempts: i64 = conn.query_row(
        "UPDATE jobs SET attempts = attempts + 1, leased_until = 0 WHERE id = ?1 RETURNING attempts",
        [job_id],
        |r| r.get(0),
    )?;
    if attempts as u32 >= MAX_ATTEMPTS {
        conn.execute("DELETE FROM jobs WHERE id = ?1", [job_id])?;
        return Ok(false);
    }
    let backoff = 30i64 << (attempts - 1); // 30s, 60s, 120s …
    conn.execute(
        "UPDATE jobs SET not_before = ?2 WHERE id = ?1",
        params![job_id, now() + backoff],
    )?;
    Ok(true)
}

/// Clear all leases (startup after a crash, or pause → resume).
pub fn release_all(conn: &Connection) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE jobs SET leased_until = 0 WHERE leased_until <> 0",
        [],
    )?)
}

/// Remove every job for a file (file changed → stages will be re-enqueued).
pub fn clear_for_file(conn: &Connection, file: FileId) -> Result<()> {
    conn.execute("DELETE FROM jobs WHERE file_id = ?1", [file.0])?;
    Ok(())
}

/// Queue an embed job for every live file that finished the text stage but
/// has no chunks yet (model installed after indexing).
pub fn enqueue_missing_embed(conn: &Connection) -> Result<usize> {
    let n = conn.execute(
        "INSERT OR IGNORE INTO jobs (file_id, stage, priority, attempts, not_before, created_at)
         SELECT f.id, 3, -1, 0, 0, ?1 FROM files f
         WHERE f.deleted = 0 AND f.kind = 'file' AND f.index_state = 2
           AND NOT EXISTS (SELECT 1 FROM chunks c WHERE c.file_id = f.id)",
        [now()],
    )?;
    Ok(n)
}

pub fn clear_for_files(conn: &Connection, files: &[FileId]) -> Result<()> {
    let mut stmt = conn.prepare_cached("DELETE FROM jobs WHERE file_id = ?1")?;
    for f in files {
        stmt.execute([f.0])?;
    }
    Ok(())
}

pub fn count_pending(conn: &Connection) -> Result<u64> {
    let n: i64 = conn.query_row("SELECT count(*) FROM jobs", [], |r| r.get(0))?;
    Ok(n.max(0) as u64)
}

/// Pending jobs per stage, for progress reporting.
pub fn count_by_stage(conn: &Connection) -> Result<Vec<(Stage, u64)>> {
    let mut stmt =
        conn.prepare("SELECT stage, count(*) FROM jobs GROUP BY stage ORDER BY stage")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
    let mut out = Vec::new();
    for row in rows {
        let (s, n) = row?;
        if let Some(stage) = Stage::from_i64(s) {
            out.push((stage, n.max(0) as u64));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::tests::disc;
    use crate::testutil::temp_db;
    use crate::{files, roots};

    fn seed(c: &Connection, n: usize) -> Vec<FileId> {
        let root = roots::add(c, "C:/r").unwrap();
        let batch: Vec<_> = (0..n)
            .map(|i| disc(root.id, &format!("C:/r/f{i}.txt")))
            .collect();
        files::upsert_discovered(c, &batch)
            .unwrap()
            .into_iter()
            .map(|u| u.id)
            .collect()
    }

    #[test]
    fn lease_orders_by_stage_then_priority_then_fifo() {
        let (_d, db) = temp_db();
        let c = db.writer();
        let ids = seed(&c, 4);
        enqueue(&c, ids[0], Stage::Text, 0).unwrap();
        enqueue(&c, ids[1], Stage::Metadata, 0).unwrap();
        enqueue(&c, ids[2], Stage::Metadata, 5).unwrap();
        enqueue(&c, ids[3], Stage::Embed, 100).unwrap();

        let got: Vec<_> = lease(&c, None, 10, 60)
            .unwrap()
            .into_iter()
            .map(|j| j.file_id)
            .collect();
        assert_eq!(got, vec![ids[2], ids[1], ids[0], ids[3]]);

        // Everything is leased now.
        assert!(lease(&c, None, 10, 60).unwrap().is_empty());
        assert_eq!(release_all(&c).unwrap(), 4);
        assert_eq!(lease(&c, Some(Stage::Embed), 10, 60).unwrap().len(), 1);
    }

    #[test]
    fn enqueue_is_idempotent_and_raises_priority() {
        let (_d, db) = temp_db();
        let c = db.writer();
        let ids = seed(&c, 1);
        enqueue(&c, ids[0], Stage::Metadata, 1).unwrap();
        enqueue(&c, ids[0], Stage::Metadata, 9).unwrap();
        enqueue(&c, ids[0], Stage::Metadata, 3).unwrap();
        let jobs = lease(&c, None, 10, 60).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].priority, 9);
        assert_eq!(count_by_stage(&c).unwrap(), vec![(Stage::Metadata, 1)]);
    }

    #[test]
    fn fail_backs_off_then_drops_after_max_attempts() {
        let (_d, db) = temp_db();
        let c = db.writer();
        let ids = seed(&c, 1);
        enqueue(&c, ids[0], Stage::Text, 0).unwrap();
        let job = lease(&c, None, 1, 60).unwrap().remove(0);

        assert!(fail(&c, job.id).unwrap());
        // backed off → not leasable right now
        assert!(lease(&c, None, 1, 60).unwrap().is_empty());
        c.execute("UPDATE jobs SET not_before = 0", []).unwrap();
        let job = lease(&c, None, 1, 60).unwrap().remove(0);
        assert_eq!(job.attempts, 1);

        assert!(fail(&c, job.id).unwrap());
        c.execute("UPDATE jobs SET not_before = 0", []).unwrap();
        let job = lease(&c, None, 1, 60).unwrap().remove(0);
        assert!(!fail(&c, job.id).unwrap());
        assert!(count_by_stage(&c).unwrap().is_empty());
    }

    #[test]
    fn complete_removes_and_tombstoned_files_are_skipped() {
        let (_d, db) = temp_db();
        let c = db.writer();
        let ids = seed(&c, 2);
        enqueue_many(&c, &ids, Stage::Metadata, 0).unwrap();
        files::tombstone_path(&c, "C:/r/f0.txt").unwrap();
        let jobs = lease(&c, None, 10, 60).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].file_id, ids[1]);
        complete(&c, jobs[0].id).unwrap();
        clear_for_file(&c, ids[0]).unwrap();
        assert!(count_by_stage(&c).unwrap().is_empty());
    }
}
