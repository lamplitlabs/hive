//! The daily run cap (`jobs_per_day`): one definition of what it counts,
//! read by the runner that enforces it (hive-runner `daily_cap.rs`), the
//! status report (hive-inspect) and the hub that show "N of M today".
//!
//! The cap counts the distinct jobs that ran today, not the runs: a run the
//! daemon asked to hold (for the owner's work or its own restart) carries
//! on later as the same job under the same agent, and a run lost to a
//! restart never finished its step - on 2026-11-03 sixty-six held runs of a
//! handful of jobs counted toward the cap and stopped the fleet by noon.
//! Dry runs and runs that failed before the worker started (`error`) were
//! never counted.

// One filter for the list and the count, so a change to what the cap
// counts cannot reach one and miss the other.
macro_rules! daily_cap_jobs_sql {
    () => {
        "SELECT DISTINCT job_id FROM runs WHERE mode != 'dry-run' AND status NOT IN ('error', 'held', 'lost') AND substr(started_at, 1, 10) = ?"
    };
}

/// The jobs the cap counts for one UTC day, one `job_id` a row; the one
/// parameter is the day as `YYYY-MM-DD`. The runner's lease reads this
/// list rather than the count: the jobs it leased this tick may or may
/// not have their run rows in yet (the run's thread inserts them), and a
/// job must count once either way.
pub const DAILY_CAP_JOBS_SQL: &str = daily_cap_jobs_sql!();

/// Counts what the cap counts for one UTC day; the one parameter is the
/// day as `YYYY-MM-DD`.
pub const DAILY_CAP_COUNT_SQL: &str = concat!("SELECT COUNT(*) FROM (", daily_cap_jobs_sql!(), ")");

/// The cap a `jobs_per_day` value means: 0 for no cap (empty, `0`, `none`,
/// `off`, `unlimited`), the number otherwise. `fallback` is what an
/// unparsable value means (the runner reads the default, 2).
pub fn daily_cap_of(raw: &str, fallback: i64) -> i64 {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "0" | "none" | "off" | "unlimited" => 0,
        word => word.parse::<i64>().map(|n| n.max(0)).unwrap_or(fallback),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cap_words_mean_no_cap_and_numbers_mean_themselves() {
        for word in ["", "0", " none ", "OFF", "unlimited"] {
            assert_eq!(daily_cap_of(word, 2), 0, "{word:?}");
        }
        assert_eq!(daily_cap_of("10", 2), 10);
        assert_eq!(daily_cap_of("-3", 2), 0);
        assert_eq!(daily_cap_of("two", 2), 2);
    }

    #[test]
    fn the_count_skips_held_lost_error_and_dry_runs_and_counts_a_job_once() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE runs (id TEXT, job_id TEXT, started_at TEXT, status TEXT, mode TEXT DEFAULT 'real')",
        )
        .unwrap();
        for (id, job, status, mode, at) in [
            ("a1", "job-a", "done", "real", "2026-11-03T01:00:00Z"),
            ("a2", "job-a", "failed", "real", "2026-11-03T02:00:00Z"),
            ("b1", "job-b", "held", "real", "2026-11-03T03:00:00Z"),
            ("b2", "job-b", "held", "real", "2026-11-03T04:00:00Z"),
            ("b3", "job-b", "running", "real", "2026-11-03T05:00:00Z"),
            ("c1", "job-c", "lost", "real", "2026-11-03T06:00:00Z"),
            ("d1", "job-d", "error", "real", "2026-11-03T07:00:00Z"),
            ("e1", "job-e", "done", "dry-run", "2026-11-03T08:00:00Z"),
            ("f1", "job-f", "done", "real", "2026-11-02T23:59:59Z"),
        ] {
            conn.execute(
                "INSERT INTO runs (id, job_id, status, mode, started_at) VALUES (?, ?, ?, ?, ?)",
                (id, job, status, mode, at),
            )
            .unwrap();
        }
        let n: i64 = conn
            .query_row(DAILY_CAP_COUNT_SQL, ["2026-11-03"], |r| r.get(0))
            .unwrap();
        // job-a once (two runs), job-b once (its running run; the held ones
        // are the same work), nothing for lost, error, dry-run or yesterday.
        assert_eq!(n, 2);
    }
}
