//! The store archive: old ledger rows nothing reads any more move out of
//! `pulse.sqlite` into a side file next to it, `pulse-archive.sqlite`.
//!
//! The store is the busiest shared resource under the daemon's 15 s tick and
//! the ledger only grows: half a million events and twenty thousand runs by
//! November 2026. Once a month the daemon's maintenance thread (and `pulse
//! store archive` by hand) moves rows older than `daemon.archive_days` (90 by
//! default; 0 turns the archive off):
//!
//! - **events** whose action is in [`COLD_ACTIONS`] - the high-volume
//!   notifications (votes, posts, run starts and finishes, resumes) that no
//!   query of the daemon, the hub or the CLI reads back by action or subject.
//!   Every other action stays hot whatever its age: leases, failures,
//!   recoveries, landings, task and schedule history all have readers that
//!   look back without a time bound.
//! - **runs** that are finished, whose job is finished too, and that no hot
//!   row names: a brief, a harness session, an evolution archive entry or
//!   baseline, or a post written in the run's session.
//! - **harness_steps** of sessions that ended before the cutoff, unless
//!   `daemon.transcript_keep_days` is 0 (the owner keeps every transcript
//!   readable).
//!
//! The retention settings set the floor: the cutoff is the oldest of
//! `daemon.archive_days`, `daemon.recover_days`, twice `evolve.keep_days` (the
//! revert scan's look-back) and, for steps, `daemon.transcript_keep_days`.
//!
//! Archive, don't federate: no read in PULSE looks in the side file on its
//! own. A report that wants old rows attaches it explicitly
//! (`ATTACH 'pulse-archive.sqlite' AS archive`); `pulse verify` does that for
//! a commit whose run key went there.
//!
//! A move is idempotent: each batch copies with `INSERT OR IGNORE` and then
//! deletes the same rows from the store in one transaction, so a run cut off
//! halfway leaves every row in at least one of the two files and the next run
//! finishes it. The events delete guard is dropped and recreated from its
//! stored definition inside each batch's transaction, as `forget` does, so no
//! other connection ever sees the ledger unguarded.

use crate::store::{Store, configure_conn, table_present};
use anyhow::{Result, anyhow};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The side file, next to `pulse.sqlite`.
pub const ARCHIVE_NAME: &str = "pulse-archive.sqlite";
/// Days a row stays in the store before the archive may move it; 0 turns
/// the archive off.
pub const ARCHIVE_DAYS: &str = "daemon.archive_days";
/// How often the daemon's maintenance thread archives.
pub const ARCHIVE_EVERY_DAYS: i64 = 30;
/// The meta key holding the last archive's report.
pub const ARCHIVE_META: &str = "archive";
/// The event an archive that moved rows writes.
pub const ARCHIVED_ACTION: &str = "store.archived";
/// Rows per batch; each batch is its own short transaction.
pub const ARCHIVE_BATCH: usize = 5_000;

/// The event actions the archive may move: written in volume, read back by
/// no query outside tests (checked with `grep` over every crate when this
/// list was written - add an action here only after the same check).
pub const COLD_ACTIONS: &[&str] = &[
    "agent.resume",
    "harness.finish",
    "harness.start",
    "job.done",
    "lesson.recorded",
    "mention",
    "post",
    "post.answer",
    "post.claim",
    "post.finding",
    "post.opinion",
    "post.question",
    "post.result",
    "post.suggestion",
    "post.vote",
    "record.created",
    "record.revised",
    "run.finish",
    "run.start",
];

/// What one archive moved (or, on a dry run, would move).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveReport {
    pub at: String,
    /// The side file.
    pub path: String,
    /// `daemon.archive_days`; 0 when the archive is off.
    pub days: i64,
    /// Rows older than this move (events and runs).
    pub cutoff: String,
    /// The cutoff for harness steps; '' when transcripts stay (see the
    /// module doc).
    pub steps_cutoff: String,
    pub events: i64,
    pub runs: i64,
    pub harness_steps: i64,
    /// False on a dry run.
    pub applied: bool,
    /// Why nothing was looked at, when the archive is off.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub skipped: String,
}

impl ArchiveReport {
    pub fn total(&self) -> i64 {
        self.events + self.runs + self.harness_steps
    }

    /// One line for the daemon log and the CLI.
    pub fn line(&self) -> String {
        if !self.skipped.is_empty() {
            return format!("store archive: {}", self.skipped);
        }
        let verb = if self.applied { "moved" } else { "would move" };
        format!(
            "store archive: {verb} {} events, {} runs and {} harness steps older than {} to {}",
            self.events, self.runs, self.harness_steps, self.cutoff, self.path
        )
    }
}

struct Plan {
    table: &'static str,
    key: &'static str,
    cutoff: String,
    clause: String,
}

/// The side file of the store at `db_path`.
pub fn archive_path(db_path: &str) -> PathBuf {
    Path::new(db_path)
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(ARCHIVE_NAME)
}

/// Opens the side file read-only for a report that wants old rows; `None`
/// while there is no archive yet.
pub fn open_archive(s: &Store) -> Result<Option<Connection>> {
    let path = archive_path(s.db_path());
    if !path.exists() {
        return Ok(None);
    }
    let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    configure_conn(&conn, true)?;
    Ok(Some(conn))
}

/// Whether the side file opened by [`open_archive`] holds `table` yet.
pub fn archive_has_table(conn: &Connection, table: &str) -> Result<bool> {
    table_present(conn, table)
}

fn days_before(now: &str, days: i64) -> Result<String> {
    let at = chrono::DateTime::parse_from_rfc3339(now)
        .map_err(|e| anyhow!("archive: now {now:?} is not a time: {e}"))?
        .with_timezone(&chrono::Utc);
    Ok((at - chrono::Duration::days(days))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string())
}

fn quoted(list: &[&str]) -> String {
    list.iter()
        .map(|a| format!("'{}'", a.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The runs clause: finished, job finished, and named by no hot row. Tables
/// a store does not have yet are left out. `NOT IN` rather than a correlated
/// `NOT EXISTS`: the referring columns are `NOT NULL` and mostly unindexed,
/// and SQLite builds each list once (70 ms on the measured store of 22,000
/// runs, against 35 s probing every run against every table).
fn runs_clause(conn: &Connection) -> Result<String> {
    let mut clause = String::from(
        "t.status != 'running' AND t.started_at < ?1 AND (t.finished_at = '' OR t.finished_at < ?1) \
         AND t.job_id NOT IN (SELECT j.id FROM main.jobs j \
           WHERE j.status IN ('queued', 'leased') OR j.finished_at = '' OR j.finished_at >= ?1)",
    );
    for (table, column) in [
        ("briefs", "run_id"),
        ("harness_sessions", "run_id"),
        ("evolution_archive", "run_id"),
        ("evolution_baselines", "run_id"),
        ("posts", "session"),
    ] {
        if table_present(conn, table)? {
            clause.push_str(&format!(
                " AND t.id NOT IN (SELECT x.{column} FROM main.{table} x WHERE x.{column} IS NOT NULL)"
            ));
        }
    }
    Ok(clause)
}

fn plans(s: &Store, conn: &Connection, rep: &mut ArchiveReport) -> Result<Vec<Plan>> {
    let floor = [
        rep.days,
        s.setting_int("daemon.recover_days"),
        2 * s.setting_int("evolve.keep_days"),
    ]
    .into_iter()
    .max()
    .unwrap_or(rep.days);
    rep.cutoff = days_before(&rep.at, floor)?;
    let mut out = vec![
        Plan {
            table: "events",
            key: "seq",
            cutoff: rep.cutoff.clone(),
            clause: format!("t.at < ?1 AND t.action IN ({})", quoted(COLD_ACTIONS)),
        },
        Plan {
            table: "runs",
            key: "id",
            cutoff: rep.cutoff.clone(),
            clause: runs_clause(conn)?,
        },
    ];
    let transcripts = s.setting_int("daemon.transcript_keep_days");
    if transcripts > 0
        && table_present(conn, "harness_steps")?
        && table_present(conn, "harness_sessions")?
    {
        rep.steps_cutoff = days_before(&rep.at, floor.max(transcripts))?;
        out.push(Plan {
            table: "harness_steps",
            key: "rowid",
            cutoff: rep.steps_cutoff.clone(),
            clause: "t.session_id IN (SELECT h.id FROM main.harness_sessions h \
                     WHERE h.status != 'running' AND h.ended_at != '' AND h.ended_at < ?1)"
                .into(),
        });
    }
    Ok(out)
}

/// The `CREATE TABLE` of `table` in the side file, from the store's own
/// definition, plus any column the store gained since the side file's table
/// was made.
fn ensure_archive_table(conn: &Connection, table: &str) -> Result<()> {
    let sql: String = conn
        .query_row(
            "SELECT sql FROM main.sqlite_master WHERE type = 'table' AND name = ?",
            [table],
            |r| r.get(0),
        )
        .map_err(|e| anyhow!("archive: no definition for {table}: {e}"))?;
    let head = format!("CREATE TABLE {table}");
    let Some(rest) = sql.strip_prefix(&head) else {
        anyhow::bail!("archive: unexpected definition for {table}: {sql}");
    };
    conn.execute_batch(&format!("CREATE TABLE IF NOT EXISTS archive.{table}{rest}"))?;
    let have = columns(conn, "archive", table)?;
    let mut stmt = conn.prepare(&format!(
        "SELECT name, type, \"notnull\", dflt_value FROM main.pragma_table_info('{table}')"
    ))?;
    let main_cols = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (name, kind, not_null, default) in main_cols {
        if have.contains(&name) {
            continue;
        }
        let mut col = format!("\"{name}\" {kind}");
        if let Some(d) = default {
            if not_null != 0 {
                col.push_str(" NOT NULL");
            }
            col.push_str(&format!(" DEFAULT {d}"));
        }
        conn.execute_batch(&format!("ALTER TABLE archive.{table} ADD COLUMN {col}"))?;
    }
    Ok(())
}

fn columns(conn: &Connection, schema: &str, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT name FROM {schema}.pragma_table_info('{table}')"
    ))?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The BEFORE DELETE guards on `table`, by name and stored definition.
fn delete_guards(conn: &Connection, table: &str) -> Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT name, sql FROM main.sqlite_master WHERE type = 'trigger' AND tbl_name = ? ORDER BY name",
    )?;
    let rows = stmt
        .query_map([table], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::new();
    for (name, sql) in rows {
        let Some(sql) = sql.filter(|s| !s.trim().is_empty()) else {
            anyhow::bail!("archive: trigger {name} has no stored definition");
        };
        if sql.to_uppercase().contains("BEFORE DELETE") {
            out.push((name, sql));
        }
    }
    Ok(out)
}

fn count(conn: &Connection, plan: &Plan) -> Result<i64> {
    Ok(conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM main.{} AS t WHERE {}",
            plan.table, plan.clause
        ),
        params![plan.cutoff],
        |r| r.get(0),
    )?)
}

/// Moves `plan`'s rows in batches; each batch copies, deletes and checks in
/// one transaction.
fn move_rows(conn: &Connection, plan: &Plan) -> Result<i64> {
    ensure_archive_table(conn, plan.table)?;
    let cols = columns(conn, "main", plan.table)?
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let picked = format!(
        "SELECT t.{key} FROM main.{table} AS t WHERE {clause} ORDER BY t.{key} LIMIT ?2",
        key = plan.key,
        table = plan.table,
        clause = plan.clause
    );
    let copy = format!(
        "INSERT OR IGNORE INTO archive.{table} ({cols}) SELECT {cols} FROM main.{table} WHERE {key} IN ({picked})",
        table = plan.table,
        key = plan.key
    );
    let delete = format!(
        "DELETE FROM main.{table} WHERE {key} IN ({picked})",
        table = plan.table,
        key = plan.key
    );
    let mut moved = 0i64;
    loop {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let step = (|| -> Result<usize> {
            let guards = delete_guards(conn, plan.table)?;
            for (name, _) in &guards {
                conn.execute_batch(&format!("DROP TRIGGER main.{name}"))?;
            }
            conn.execute(&copy, params![plan.cutoff, ARCHIVE_BATCH as i64])?;
            let n = conn.execute(&delete, params![plan.cutoff, ARCHIVE_BATCH as i64])?;
            for (_, sql) in &guards {
                conn.execute_batch(sql)?;
            }
            Ok(n)
        })();
        match step {
            Ok(n) => {
                conn.execute_batch("COMMIT")?;
                moved += n as i64;
                if n < ARCHIVE_BATCH {
                    return Ok(moved);
                }
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(e);
            }
        }
    }
}

impl Store {
    /// Moves (or with `dry_run` counts) the rows the module doc describes into
    /// the side file and, when it moved any, writes a `store.archived` event.
    /// The report is kept in meta either way, so the daemon knows when the
    /// next one is due.
    pub fn archive_old_rows(&self, dry_run: bool, actor: &str) -> Result<ArchiveReport> {
        let mut rep = ArchiveReport {
            at: self.now(),
            path: archive_path(self.db_path()).display().to_string(),
            days: self.setting_int(ARCHIVE_DAYS),
            applied: !dry_run,
            ..Default::default()
        };
        if rep.days <= 0 {
            rep.applied = false;
            rep.skipped = format!("{ARCHIVE_DAYS} is 0, so nothing is archived");
            return Ok(rep);
        }
        let conn = Connection::open_with_flags(
            self.db_path(),
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        configure_conn(&conn, false)?;
        let plans = plans(self, &conn, &mut rep)?;
        if dry_run {
            for plan in &plans {
                let n = count(&conn, plan)?;
                set(&mut rep, plan.table, n);
            }
            return Ok(rep);
        }
        conn.execute("ATTACH DATABASE ?1 AS archive", [&rep.path])?;
        conn.pragma_update(Some("archive"), "journal_mode", "WAL")?;
        let moved = (|| -> Result<()> {
            for plan in &plans {
                let n = move_rows(&conn, plan)?;
                set(&mut rep, plan.table, n);
            }
            Ok(())
        })();
        let _ = conn.execute_batch("DETACH DATABASE archive");
        moved?;
        let detail = crate::store::Detail::json(&rep)?;
        if rep.total() > 0 {
            self.event(actor, ARCHIVED_ACTION, ARCHIVE_NAME, detail.clone())?;
        }
        self.set_meta(ARCHIVE_META, &detail.text()?)?;
        Ok(rep)
    }

    /// The last archive the store kept a report of.
    pub fn last_archive(&self) -> Option<ArchiveReport> {
        let conn = self.conn().ok()?;
        let raw: String = conn
            .query_row(
                "SELECT value FROM meta WHERE key = ?",
                [ARCHIVE_META],
                |r| r.get(0),
            )
            .optional()
            .ok()??;
        serde_json::from_str(&raw).ok()
    }

    /// Whether the daemon's monthly archive is due at `now`: the archive is
    /// on and the last one is `ARCHIVE_EVERY_DAYS` old or there is none.
    pub fn archive_due(&self, now: &str) -> bool {
        if self.setting_int(ARCHIVE_DAYS) <= 0 {
            return false;
        }
        match self.last_archive() {
            None => true,
            Some(last) => days_before(now, ARCHIVE_EVERY_DAYS)
                .map(|due| last.at.as_str() <= due.as_str())
                .unwrap_or(false),
        }
    }

    /// The maintenance thread's stage: archives when due; one line when it
    /// did, none otherwise.
    pub fn tend_archive(&self, now: &str, actor: &str) -> Vec<String> {
        if !self.archive_due(now) {
            return Vec::new();
        }
        match self.archive_old_rows(false, actor) {
            Ok(rep) => vec![rep.line()],
            Err(e) => vec![format!("store archive: {e}")],
        }
    }

    /// Deletes a forgotten project's rows from the side file: its runs, the
    /// steps of its harness sessions and the events about it, its lead and
    /// the ids of its rows. Called by `forget_project` once it has passed its
    /// refusals and before the store's own rows go, while their ids can still
    /// be read. Returns the rows deleted.
    pub(crate) fn forget_archived(&self, slug: &str, lead: &str) -> Result<i64> {
        let path = archive_path(self.db_path());
        if !path.exists() {
            return Ok(0);
        }
        let conn = Connection::open_with_flags(
            self.db_path(),
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        configure_conn(&conn, false)?;
        conn.execute(
            "ATTACH DATABASE ?1 AS archive",
            [path.display().to_string()],
        )?;
        let present = |t: &str| -> Result<bool> {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM archive.sqlite_master WHERE type = 'table' AND name = ?",
                [t],
                |r| r.get::<_, i64>(0),
            )? > 0)
        };
        let result = (|| -> Result<i64> {
            let mut n = 0i64;
            // Deferred: the caller's transaction holds the store's write lock;
            // this one only reads the store and writes the side file.
            conn.execute_batch("BEGIN")?;
            if present("events")? {
                let mut ids = vec![
                    "SELECT ?1".to_string(),
                    "SELECT ?2 WHERE ?2 != ''".to_string(),
                ];
                if present("runs")? {
                    ids.push("SELECT id FROM archive.runs WHERE project = ?1".into());
                }
                for table in [
                    "posts",
                    "jobs",
                    "runs",
                    "records",
                    "harness_sessions",
                    "tasks",
                    "task_suggestions",
                    "messages",
                    "schedules",
                    "leads",
                    "compute_requests",
                    "evolution_fixtures",
                    "evolution_fleets",
                    "evolution_archive",
                ] {
                    if table_present(&conn, table)? {
                        ids.push(format!("SELECT id FROM main.{table} WHERE project = ?1"));
                    }
                }
                n += conn.execute(
                    &format!(
                        "DELETE FROM archive.events WHERE subject IN ({})",
                        ids.join(" UNION ALL ")
                    ),
                    params![slug, lead],
                )? as i64;
            }
            if present("harness_steps")? && table_present(&conn, "harness_sessions")? {
                n += conn.execute(
                    "DELETE FROM archive.harness_steps WHERE session_id IN \
                     (SELECT id FROM main.harness_sessions WHERE project = ?1)",
                    [slug],
                )? as i64;
            }
            if present("runs")? {
                n += conn.execute("DELETE FROM archive.runs WHERE project = ?1", [slug])? as i64;
            }
            conn.execute_batch("COMMIT")?;
            Ok(n)
        })();
        if result.is_err() {
            let _ = conn.execute_batch("ROLLBACK");
        }
        let _ = conn.execute_batch("DETACH DATABASE archive");
        result
    }
}

fn set(rep: &mut ArchiveReport, table: &str, n: i64) {
    match table {
        "events" => rep.events = n,
        "runs" => rep.runs = n,
        _ => rep.harness_steps = n,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Options, Store};

    const NOW: &str = "2027-01-10T04:00:00Z";
    /// Older than 90 days before NOW.
    const OLD: &str = "2026-09-01T10:00:00Z";
    /// Younger than 90 days before NOW.
    const YOUNG: &str = "2026-12-20T10:00:00Z";

    fn open() -> (tempfile::TempDir, Store) {
        unsafe { std::env::set_var("PULSE_JOB", "") };
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(Options {
            home: dir.path().display().to_string(),
            create: true,
            now: NOW.into(),
            ..Options::default()
        })
        .unwrap();
        (dir, s)
    }

    fn x(s: &Store, sql: &str, args: &[&dyn rusqlite::ToSql]) {
        s.conn()
            .unwrap()
            .execute(sql, args)
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    fn n(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0))
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    fn event(s: &Store, at: &str, action: &str, subject: &str) {
        x(
            s,
            "INSERT INTO events (at, actor, action, subject, detail) VALUES (?, 'w', ?, ?, '{}')",
            &[&at, &action, &subject],
        );
    }

    fn job(s: &Store, id: &str, project: &str, status: &str, finished: &str) {
        x(
            s,
            "INSERT INTO jobs (id, project, kind, title, status, created_at, finished_at) VALUES (?, ?, 'task', 't', ?, ?, ?)",
            &[&id, &project, &status, &OLD, &finished],
        );
    }

    fn run(s: &Store, id: &str, job: &str, project: &str, status: &str, at: &str) {
        let finished = if status == "running" { "" } else { at };
        x(
            s,
            "INSERT INTO runs (id, job_id, agent, project, started_at, finished_at, status, signing_pubkey) VALUES (?, ?, 'w', ?, ?, ?, ?, 'ssh-ed25519 AAAA')",
            &[&id, &job, &project, &at, &finished, &status],
        );
    }

    fn session(s: &Store, id: &str, run: &str, status: &str, ended: &str) {
        x(
            s,
            "INSERT INTO harness_sessions (id, run_id, job_id, agent, project, status, started_at, updated_at, ended_at) VALUES (?, ?, 'job-old', 'w', 'demo', ?, ?, ?, ?)",
            &[&id, &run, &status, &OLD, &OLD, &ended],
        );
        for step in 1..=3 {
            x(
                s,
                "INSERT INTO harness_steps (session_id, n, at, kind, content) VALUES (?, ?, ?, 'assistant', 'hi')",
                &[&id, &step, &OLD],
            );
        }
    }

    /// Old and young rows of every kind the archive weighs.
    fn seed(s: &Store) {
        for (at, action, subject) in [
            (OLD, "post.vote", "post-1"),
            (OLD, "run.start", "run-old"),
            (OLD, "run.finish", "run-old"),
            (OLD, "agent.resume", "lead"),
            // Hot whatever their age: readers look back without a bound.
            (OLD, "job.lease", "job-old"),
            (OLD, "job.failed", "job-old"),
            (OLD, "land.landed", "pulse/w/job-old"),
            (OLD, "task.status", "TASK-1"),
            (YOUNG, "post.vote", "post-2"),
            (YOUNG, "run.start", "run-young"),
        ] {
            event(s, at, action, subject);
        }
        job(s, "job-old", "demo", "done", OLD);
        job(s, "job-young", "demo", "done", YOUNG);
        job(s, "job-open", "demo", "queued", "");
        run(s, "run-old", "job-old", "demo", "done", OLD);
        run(s, "run-young", "job-young", "demo", "done", YOUNG);
        // Old, but its job is still queued: a requeue may read it.
        run(s, "run-open-job", "job-open", "demo", "error", OLD);
        // Old, but a brief / harness session / post names it.
        run(s, "run-briefed", "job-old", "demo", "done", OLD);
        x(
            s,
            "INSERT INTO briefs (run_id, job_id, agent, text, at) VALUES ('run-briefed', 'job-old', 'w', 'b', ?)",
            &[&OLD],
        );
        run(s, "run-session", "job-old", "demo", "done", OLD);
        session(s, "hs-ended", "run-session", "submitted", OLD);
        session(s, "hs-running", "run-session", "running", "");
        run(s, "run-posted", "job-old", "demo", "done", OLD);
        x(
            s,
            "INSERT INTO posts (id, project, area, thread, kind, agent, date, created_at, session) VALUES ('post-1', 'demo', 'a', 't', 'finding', 'w', ?, ?, 'run-posted')",
            &[&OLD, &OLD],
        );
    }

    fn ids(conn: &Connection, sql: &str) -> Vec<String> {
        let mut stmt = conn.prepare(sql).unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn old_rows_move_young_and_hot_rows_stay_and_a_second_run_moves_nothing() {
        let _serial = crate::test_helpers::serial();
        let (_dir, s) = open();
        seed(&s);
        let events_before = n(&s.conn().unwrap(), "SELECT COUNT(*) FROM events");

        let dry = s.archive_old_rows(true, "human").unwrap();
        assert!(!dry.applied);
        assert_eq!(dry.days, 90);
        assert_eq!(dry.cutoff, "2026-10-12T04:00:00Z");
        assert_eq!(
            (dry.events, dry.runs, dry.harness_steps),
            (4, 1, 3),
            "{dry:?}"
        );
        assert!(
            !archive_path(s.db_path()).exists(),
            "a dry run creates nothing"
        );
        assert_eq!(
            n(&s.conn().unwrap(), "SELECT COUNT(*) FROM events"),
            events_before
        );

        let rep = s.archive_old_rows(false, "human").unwrap();
        assert!(rep.applied);
        assert_eq!((rep.events, rep.runs, rep.harness_steps), (4, 1, 3));
        assert!(
            rep.line()
                .starts_with("store archive: moved 4 events, 1 runs and 3 harness steps")
        );

        let conn = s.conn().unwrap();
        assert_eq!(
            ids(
                &conn,
                "SELECT action FROM events WHERE at < '2026-10-12' ORDER BY seq"
            ),
            ["job.lease", "job.failed", "land.landed", "task.status"]
        );
        assert_eq!(
            ids(
                &conn,
                "SELECT action FROM events WHERE at >= '2026-10-12' AND at < '2027' ORDER BY seq"
            ),
            ["post.vote", "run.start"]
        );
        assert_eq!(
            ids(&conn, "SELECT id FROM runs ORDER BY id"),
            [
                "run-briefed",
                "run-open-job",
                "run-posted",
                "run-session",
                "run-young"
            ]
        );
        assert_eq!(
            ids(&conn, "SELECT DISTINCT session_id FROM harness_steps"),
            ["hs-running"]
        );
        // The ledger is guarded again: the moved kinds cannot be deleted by hand.
        assert!(
            conn.execute("DELETE FROM events WHERE action = 'post.vote'", [])
                .is_err()
        );
        let detail: String = conn
            .query_row(
                "SELECT detail FROM events WHERE action = ?",
                [ARCHIVED_ACTION],
                |r| r.get(0),
            )
            .unwrap();
        let logged: ArchiveReport = serde_json::from_str(&detail).unwrap();
        assert_eq!(logged.total(), 8);
        drop(conn);

        let archive = open_archive(&s).unwrap().expect("the side file");
        assert_eq!(
            ids(&archive, "SELECT action FROM events ORDER BY seq"),
            ["post.vote", "run.start", "run.finish", "agent.resume"]
        );
        assert_eq!(ids(&archive, "SELECT id FROM runs"), ["run-old"]);
        assert_eq!(n(&archive, "SELECT COUNT(*) FROM harness_steps"), 3);
        // Same columns as the store's, so a report can join them.
        assert_eq!(
            columns(&archive, "main", "runs").unwrap(),
            columns(&s.conn().unwrap(), "main", "runs").unwrap()
        );

        let again = s.archive_old_rows(false, "human").unwrap();
        assert_eq!(again.total(), 0, "{again:?}");
        assert_eq!(
            n(
                &s.conn().unwrap(),
                &format!("SELECT COUNT(*) FROM events WHERE action = '{ARCHIVED_ACTION}'")
            ),
            1,
            "an archive that moved nothing writes no event"
        );
        assert_eq!(n(&archive, "SELECT COUNT(*) FROM events"), 4);
    }

    #[test]
    fn retention_settings_set_the_floor_and_zero_turns_the_archive_off() {
        let _serial = crate::test_helpers::serial();
        let (_dir, s) = open();
        seed(&s);
        // A recovery window longer than the archive's keeps everything.
        s.set_setting("human", "daemon.recover_days", "200")
            .unwrap();
        let rep = s.archive_old_rows(true, "human").unwrap();
        assert_eq!(rep.cutoff, "2026-06-24T04:00:00Z");
        assert_eq!(rep.total(), 0, "{rep:?}");
        s.set_setting("human", "daemon.recover_days", "7").unwrap();
        // Transcripts kept for good stay in the store.
        s.set_setting("human", "daemon.transcript_keep_days", "0")
            .unwrap();
        let rep = s.archive_old_rows(true, "human").unwrap();
        assert_eq!((rep.events, rep.runs, rep.harness_steps), (4, 1, 0));
        assert!(rep.steps_cutoff.is_empty());

        s.set_setting("human", ARCHIVE_DAYS, "0").unwrap();
        let rep = s.archive_old_rows(false, "human").unwrap();
        assert!(!rep.applied);
        assert!(rep.line().contains("is 0"), "{}", rep.line());
        assert!(!archive_path(s.db_path()).exists());
        assert!(!s.archive_due(NOW));
        assert!(s.tend_archive(NOW, "pulse-daemon").is_empty());
    }

    #[test]
    fn the_daemon_archives_once_a_month() {
        let _serial = crate::test_helpers::serial();
        let (_dir, s) = open();
        seed(&s);
        assert!(s.archive_due(NOW), "no archive yet");
        let lines = s.tend_archive(NOW, "pulse-daemon");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("moved 4 events"), "{lines:?}");
        assert_eq!(s.last_archive().unwrap().at, NOW);
        assert!(!s.archive_due("2027-02-08T04:00:00Z"));
        assert!(
            s.tend_archive("2027-02-08T04:00:00Z", "pulse-daemon")
                .is_empty()
        );
        assert!(s.archive_due("2027-02-09T04:00:00Z"));
    }

    #[test]
    fn forgetting_a_project_clears_its_archived_rows() {
        let _serial = crate::test_helpers::serial();
        let (_dir, s) = open();
        seed(&s);
        x(
            &s,
            "INSERT INTO projects (slug, name, path, status, added_at) VALUES ('demo', 'demo', '/nonexistent/demo', 'inactive', ?)",
            &[&OLD],
        );
        x(
            &s,
            "UPDATE harness_sessions SET status = 'submitted', ended_at = ?",
            &[&OLD],
        );
        x(
            &s,
            "UPDATE jobs SET status = 'done', finished_at = ?",
            &[&OLD],
        );
        x(&s, "UPDATE runs SET status = 'done'", &[]);
        let rep = s.archive_old_rows(false, "human").unwrap();
        assert!(rep.total() > 0, "{rep:?}");
        let archive = open_archive(&s).unwrap().unwrap();
        assert!(n(&archive, "SELECT COUNT(*) FROM runs") > 0);
        s.forget_project("demo", "human").unwrap();
        assert_eq!(n(&archive, "SELECT COUNT(*) FROM runs"), 0);
        assert_eq!(n(&archive, "SELECT COUNT(*) FROM harness_steps"), 0);
        // The run's and the post's events went; the lead's resume names no
        // project row and is not the project's.
        assert_eq!(
            ids(&archive, "SELECT subject FROM events ORDER BY seq"),
            ["lead"]
        );
    }

    /// The hot queries the new indexes are for read them, not the whole
    /// ledger.
    #[test]
    fn hot_queries_use_the_json_and_actor_indexes() {
        let _serial = crate::test_helpers::serial();
        let (_dir, s) = open();
        let conn = s.conn().unwrap();
        // A ledger shaped like a working one, with the statistics the store
        // gathers on open (`optimize`): a few kinds, many projects, jobs,
        // roots and workers.
        conn.execute_batch("BEGIN").unwrap();
        for i in 0..4_000 {
            let action = [
                "land.landed",
                "land.deferred",
                "job.recover",
                "job.lease",
                "post.vote",
                "run.start",
                "run.finish",
                "post",
            ][i % 8];
            let detail = serde_json::json!({
                "project": format!("p{}", i % 40),
                "job": format!("job-{i}"),
                "root": format!("job-{}", i % 400),
            });
            conn.execute(
                "INSERT INTO events (at, actor, action, subject, detail) VALUES (?, ?, ?, ?, ?)",
                params![
                    NOW,
                    format!("w{}", i % 30),
                    action,
                    format!("s{i}"),
                    detail.to_string()
                ],
            )
            .unwrap();
        }
        conn.execute_batch("COMMIT; ANALYZE").unwrap();
        let plan = |sql: &str| -> String {
            let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            stmt.query_map([], |r| r.get::<_, String>(3))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
                .join(" | ")
        };
        for (sql, index) in [
            // landing_outcome.rs / land.rs: a project's landings and holds.
            (
                "SELECT seq FROM events WHERE action = 'land.landed' AND json_valid(detail) AND json_extract(detail, '$.project') = 'demo'",
                "events_detail_project",
            ),
            // daemon_maintenance.rs first_own_deferral, delivery.rs landings_of.
            (
                "SELECT at FROM events WHERE action = 'land.deferred' AND json_valid(detail) AND json_extract(detail, '$.job') = 'job-1' ORDER BY seq LIMIT 1",
                "events_detail_job",
            ),
            // pulse-board jobs.rs: the recoveries of a root job.
            (
                "SELECT COUNT(*) FROM events WHERE action = 'job.recover' AND json_valid(detail) AND json_extract(detail, '$.root') = 'job-1'",
                "events_detail_root",
            ),
            // run.rs pick_worker_on: each worker's last lease.
            (
                "SELECT actor, MAX(seq) FROM events WHERE action = 'job.lease' GROUP BY actor",
                "events_action_actor",
            ),
            (
                "SELECT MAX(seq) FROM events WHERE action = 'job.lease' AND actor = 'w'",
                "events_action_actor",
            ),
        ] {
            let got = plan(sql);
            assert!(got.contains(index), "{sql}\n  plan: {got}");
            assert!(!got.contains("SCAN events"), "{sql}\n  plan: {got}");
        }
    }
}
