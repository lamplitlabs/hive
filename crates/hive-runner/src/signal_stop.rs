//! A worker ended from outside (TASK-151 item 3).
//!
//! On 2026-11-26 one agent's `pkill` sent SIGTERM to every worker on the
//! machine; four runs were judged failed and lost an attempt and their work
//! for something none of them did. A worker that exits 143 (SIGTERM) or 137
//! (SIGKILL) while no hold, cancel or timeout is on record for its run was
//! stopped by something outside the runner. Its run is not judged: the job
//! goes back to the queue without using an attempt, the clone stays, its
//! commits are kept on the job's `-failed` branch and as a patch, and a
//! [`SIGNALLED_EVENT`] names them so the next run's brief starts from them.
//! The daemon then posts one finding per burst of such exits, naming the runs
//! that ended together ([`tend_outside_signal_findings`]).

use crate::util::Row;
use serde_json::json;

/// The event each such run writes, with the job as its subject.
pub(crate) const SIGNALLED_EVENT: &str = "job.signalled";

/// How many times one job is requeued this way; past it the run is judged
/// as it stands, so a job whose own work kills its worker cannot loop.
pub(crate) const SIGNAL_REQUEUE_LIMIT: i32 = 2;

/// Runs whose exits lie closer together than this ended together.
pub(crate) const ENDED_TOGETHER_SECONDS: i64 = 60;

/// Where the daemon keeps the last [`SIGNALLED_EVENT`] a finding named.
const FINDING_CURSOR: &str = "runner.signalled.cursor";

/// The signal behind a worker's exit code, when the exit is a kill from
/// outside: SIGTERM (143) or SIGKILL (137).
pub(crate) fn outside_signal(timeout: bool, exit_code: i32) -> Option<&'static str> {
    if timeout {
        return None;
    }
    match exit_code {
        143 => Some("SIGTERM"),
        137 => Some("SIGKILL"),
        _ => None,
    }
}

/// Why a run's stop is not an outside one: a hold or cancel the store holds
/// for one of its harness sessions, or the runner's own yield. Empty when
/// nothing is on record.
pub(crate) fn stop_on_record(s: &hive_store::Store, job_id: &str, run_id: &str) -> String {
    let Ok(conn) = s.conn() else {
        return "the store did not answer".into();
    };
    let r = crate::util::must_one(
        &conn,
        "SELECT COALESCE(SUM(cancel_requested_at != ''), 0) AS cancels, COALESCE(SUM(hold_requested_at != ''), 0) AS holds FROM harness_sessions WHERE run_id = ?",
        &[&run_id],
    );
    if r.int("cancels") > 0 {
        return "a cancel is on record for its harness session".into();
    }
    if r.int("holds") > 0 {
        return "a hold is on record for its harness session".into();
    }
    // `hive job stop` (run.stop): the watcher reads it every few seconds,
    // so a stop and the signal it led to can land between two reads. The
    // stop is the owner's, not an outside one (TASK-169 item 8).
    match crate::run::owner_stop(&*conn, run_id) {
        Some(true) => return "the owner stopped it to run again (hive job stop --requeue)".into(),
        Some(false) => return "the owner stopped it (hive job stop)".into(),
        None => {}
    }
    if crate::yield_lane::yielded(s, job_id, run_id) {
        return "the runner held it so the owner's work could start first".into();
    }
    String::new()
}

/// How many earlier runs of `job_id` were requeued after an outside signal.
pub(crate) fn earlier_requeues(q: &(impl crate::util::Querier + ?Sized), job_id: &str) -> i32 {
    crate::util::must_one(
        q,
        &format!(
            "SELECT COUNT(*) AS n FROM events WHERE action = '{SIGNALLED_EVENT}' AND subject = ?"
        ),
        &[&job_id],
    )
    .int("n")
}

/// What one run's requeue records: the event's detail, read back by the
/// brief (`why`, `patch`) and by the finding.
#[derive(Debug, Clone, Default)]
pub(crate) struct Signalled {
    pub run: String,
    pub job: String,
    pub project: String,
    pub agent: String,
    pub executor: String,
    pub exit: i32,
    pub signal: String,
    pub commits: i32,
    pub branch: String,
    pub patch: String,
}

impl Signalled {
    pub(crate) fn why(&self) -> String {
        format!(
            "its worker was ended by an outside {} (exit {}) mid-run, with no hold, cancel or timeout on record",
            self.signal, self.exit
        )
    }

    /// Writes the [`SIGNALLED_EVENT`].
    pub(crate) fn record(&self, s: &hive_store::Store, at: &str, actor: &str) {
        let Ok(conn) = s.conn() else {
            return;
        };
        crate::util::event_at(
            &conn,
            at,
            actor,
            SIGNALLED_EVENT,
            &self.job,
            json!({
                "run": self.run,
                "project": self.project,
                "agent": self.agent,
                "executor": self.executor,
                "exit": self.exit,
                "signal": self.signal,
                "commits": self.commits,
                "branch": self.branch,
                "patch": self.patch,
                "why": self.why(),
            }),
        );
    }

    /// The run's summary: not judged, requeued, where its work is.
    pub(crate) fn summary(&self) -> String {
        let mut s = format!(
            "ended by an outside {} (exit {}) with no hold, cancel or timeout on record: not judged; the job returns to the queue without using an attempt and the clone is kept",
            self.signal, self.exit
        );
        if self.commits > 0 {
            if self.branch.is_empty() {
                s.push_str(&format!(
                    "; its {} commit(s) stay in the clone only (the fetch failed)",
                    self.commits
                ));
            } else if self.patch.is_empty() {
                s.push_str(&format!(
                    "; its {} commit(s) are kept on branch {}",
                    self.commits, self.branch
                ));
            } else {
                s.push_str(&format!(
                    "; its {} commit(s) are kept on branch {} and in {}",
                    self.commits, self.branch, self.patch
                ));
            }
        }
        s
    }

    fn from_event(e: &Row) -> Self {
        let d = crate::util::json_map(&e.str("detail"));
        let text = |k: &str| {
            d.get(k)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };
        let int = |k: &str| d.get(k).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        Self {
            run: text("run"),
            job: e.str("subject"),
            project: text("project"),
            agent: text("agent"),
            executor: text("executor"),
            exit: int("exit"),
            signal: text("signal"),
            commits: int("commits"),
            branch: text("branch"),
            patch: text("patch"),
        }
    }

    fn line(&self) -> String {
        let mut l = format!(
            "- run `{}` of job `{}` ({}, {} on `{}`): exit {} ({})",
            self.run, self.job, self.project, self.agent, self.executor, self.exit, self.signal
        );
        if self.commits > 0 && !self.branch.is_empty() {
            l.push_str(&format!(
                "; {} commit(s) kept on `{}`",
                self.commits, self.branch
            ));
        } else if self.commits > 0 {
            l.push_str(&format!("; {} commit(s) left in its clone", self.commits));
        }
        l
    }
}

/// Posts one finding for each burst of [`SIGNALLED_EVENT`]s the board has not
/// heard of: runs whose exits lie within [`ENDED_TOGETHER_SECONDS`] of each
/// other ended together, most likely from one `pkill`/`killall` or an
/// out-of-memory kill. A burst is posted once it has been quiet that long,
/// so a run that ends a few seconds after the others is in the same post.
pub(crate) fn tend_outside_signal_findings(
    s: &hive_store::Store,
    now: &str,
    actor: &str,
) -> Vec<String> {
    let Ok(conn) = s.conn() else {
        return Vec::new();
    };
    let cursor = s
        .meta(FINDING_CURSOR)
        .ok()
        .and_then(|(v, _)| v.parse::<i64>().ok())
        .unwrap_or(0);
    let events = crate::util::query_rows(
        &conn,
        &format!(
            "SELECT seq, at, subject, detail FROM events WHERE action = '{SIGNALLED_EVENT}' AND seq > ? ORDER BY seq"
        ),
        &[&cursor],
    )
    .unwrap_or_default();
    let mut bursts: Vec<Vec<Row>> = Vec::new();
    for e in events {
        let at = crate::util::must_iso(&e.str("at"));
        match bursts.last_mut() {
            Some(b)
                if (at
                    - crate::util::must_iso(
                        &b.last().map(|l| l.str("at")).unwrap_or_default(),
                    ))
                .num_seconds()
                .abs()
                    <= ENDED_TOGETHER_SECONDS =>
            {
                b.push(e)
            }
            _ => bursts.push(vec![e]),
        }
    }
    let clock = crate::util::must_iso(now);
    let mut lines = Vec::new();
    for burst in bursts {
        let Some(last) = burst.last() else {
            continue;
        };
        if (clock - crate::util::must_iso(&last.str("at"))).num_seconds() < ENDED_TOGETHER_SECONDS {
            break;
        }
        let runs: Vec<Signalled> = burst.iter().map(Signalled::from_event).collect();
        let first_at = burst[0].str("at");
        let brain = crate::board::brain_project(&conn);
        let project = if !crate::util::is_nil(&brain) {
            brain.str("slug")
        } else {
            runs[0].project.clone()
        };
        let summary = crate::util::trunc(
            &if runs.len() == 1 {
                format!(
                    "run {} ended by an outside {} at {first_at}; requeued without using an attempt",
                    runs[0].run, runs[0].signal
                )
            } else {
                format!(
                    "{} runs ended together by an outside signal at {first_at}; requeued without using an attempt",
                    runs.len()
                )
            },
            230,
        );
        let mut body = vec![
            format!(
                "Between {first_at} and {} these workers exited 143 (SIGTERM) or 137 (SIGKILL) while no hold, cancel or timeout of theirs was on record: something outside the runner stopped them - a `pkill` or `killall` by pattern, an out-of-memory kill, or a person.",
                last.str("at")
            ),
            String::new(),
        ];
        body.extend(runs.iter().map(Signalled::line));
        body.push(String::new());
        body.push("None of them was judged: each job went back to the queue without using an attempt, its clone stays, and its commits are kept on its `-failed` branch and as a patch the next run's brief starts from. When this repeats, find what sends the signal; a worker stops only what it started, by its own process id.".into());
        let o = crate::board::PostOpts {
            kind: "finding".into(),
            project,
            area: "runner".into(),
            thread: "outside-signal".into(),
            summary: summary.clone(),
            body: body.join("\n") + "\n",
            agent: crate::board::actor_agent(s, actor.to_string()),
            date: now.to_string(),
            ..Default::default()
        };
        crate::board::create_finding_or_warn(s, now.to_string(), "outside signal", o);
        let _ = s.set_meta(FINDING_CURSOR, &last.int("seq").to_string());
        lines.push(format!("outside signal: {summary}"));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_owner_stop_is_a_stop_on_record() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        assert_eq!(stop_on_record(&s, "job-1", "run-1"), "");
        crate::util::event_at(
            &*s.conn().unwrap(),
            crate::test_helpers::TEST_NOW,
            hive_store::HUMAN_AGENT,
            hive_board::RUN_STOP_ACTION,
            "run-1",
            json!({"job": "job-1", "executor": "", "requeue": false}),
        );
        assert_eq!(
            stop_on_record(&s, "job-1", "run-1"),
            "the owner stopped it (hive job stop)"
        );
        assert_eq!(stop_on_record(&s, "job-2", "run-2"), "");
        crate::util::event_at(
            &*s.conn().unwrap(),
            crate::test_helpers::TEST_NOW,
            hive_store::HUMAN_AGENT,
            hive_board::RUN_STOP_ACTION,
            "run-2",
            json!({"job": "job-2", "executor": "", "requeue": true}),
        );
        assert!(
            stop_on_record(&s, "job-2", "run-2").contains("--requeue"),
            "{}",
            stop_on_record(&s, "job-2", "run-2")
        );
    }

    #[test]
    fn only_a_kill_from_outside_is_an_outside_signal() {
        assert_eq!(outside_signal(false, 143), Some("SIGTERM"));
        assert_eq!(outside_signal(false, 137), Some("SIGKILL"));
        assert_eq!(outside_signal(true, 143), None, "a timeout is the runner's");
        for code in [0, 1, 5, 75, 130] {
            assert_eq!(outside_signal(false, code), None, "exit {code}");
        }
    }

    /// TASK-170: what is on record for a run decides whether its SIGTERM was
    /// the runner's own: a cancel or a hold asked of its harness session,
    /// or the runner yielding its lane to the owner's work. Another run's
    /// record, or a session with neither asked, is not.
    #[test]
    fn stop_on_record_names_the_runners_own_stop() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        let conn = s.conn().unwrap();
        let session = |id: &str, run: &str, cancel: &str, hold: &str| {
            conn.execute(
                "INSERT INTO harness_sessions (id, run_id, job_id, agent, project, status, started_at, updated_at, cancel_requested_at, hold_requested_at) VALUES (?1, ?2, 'job-1', 'aurora', 'proj', 'running', ?3, ?3, ?4, ?5)",
                rusqlite::params![id, run, crate::test_helpers::TEST_NOW, cancel, hold],
            )
            .unwrap();
        };
        // Nothing on record: the signal came from outside.
        assert_eq!(stop_on_record(&s, "job-1", "run-a"), "");
        session("hs-a", "run-a", "", "");
        assert_eq!(stop_on_record(&s, "job-1", "run-a"), "");
        // Another run's cancel and hold are not this run's.
        session(
            "hs-other",
            "run-b",
            crate::test_helpers::TEST_NOW,
            crate::test_helpers::TEST_NOW,
        );
        assert_eq!(stop_on_record(&s, "job-1", "run-a"), "");
        session("hs-c", "run-c", crate::test_helpers::TEST_NOW, "");
        assert_eq!(
            stop_on_record(&s, "job-1", "run-c"),
            "a cancel is on record for its harness session"
        );
        session("hs-h", "run-h", "", crate::test_helpers::TEST_NOW);
        assert_eq!(
            stop_on_record(&s, "job-1", "run-h"),
            "a hold is on record for its harness session"
        );
        // A cancel wins over a hold of the same run.
        session(
            "hs-ch",
            "run-ch",
            crate::test_helpers::TEST_NOW,
            crate::test_helpers::TEST_NOW,
        );
        assert_eq!(
            stop_on_record(&s, "job-1", "run-ch"),
            "a cancel is on record for its harness session"
        );
        // The runner yielded this run's lane to the owner's work.
        crate::util::event_at(
            &conn,
            crate::test_helpers::TEST_NOW,
            "daemon",
            crate::yield_lane::YIELD_ACTION,
            "job-1",
            json!({"run": "run-y"}),
        );
        assert_eq!(
            stop_on_record(&s, "job-1", "run-y"),
            "the runner held it so the owner's work could start first"
        );
        assert_eq!(stop_on_record(&s, "job-2", "run-y"), "", "another job");
        assert_eq!(stop_on_record(&s, "job-1", "run-a"), "", "another run");
    }

    #[test]
    fn runs_that_ended_together_are_named_in_one_finding() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        let at = |secs: i64| {
            crate::util::format_iso(
                crate::util::must_iso(crate::test_helpers::TEST_NOW)
                    + chrono::Duration::seconds(secs),
            )
        };
        let signalled = |run: &str, job: &str, secs: i64| {
            Signalled {
                run: run.into(),
                job: job.into(),
                project: "proj".into(),
                agent: "aurora".into(),
                executor: "local".into(),
                exit: 143,
                signal: "SIGTERM".into(),
                commits: 2,
                branch: format!("hive/aurora/{job}-failed"),
                patch: String::new(),
            }
            .record(&s, &at(secs), "runner")
        };
        signalled("run-a", "job-a", 0);
        signalled("run-b", "job-b", 5);
        signalled("run-c", "job-c", 300);
        let findings = || {
            crate::util::query_rows(
                &s.conn().unwrap(),
                "SELECT summary, body FROM posts WHERE kind = 'finding' AND area = 'runner' AND thread = 'outside-signal' ORDER BY rowid",
                &[],
            )
            .unwrap()
        };
        // The first burst is still settling 30s after its last exit.
        assert!(tend_outside_signal_findings(&s, &at(35), "daemon").is_empty());
        assert!(findings().is_empty());
        // Once quiet, runs a and b are one finding; c is still settling.
        let lines = tend_outside_signal_findings(&s, &at(120), "daemon");
        assert_eq!(lines.len(), 1, "{lines:?}");
        let posted = findings();
        assert_eq!(posted.len(), 1, "{posted:?}");
        assert!(
            posted[0]
                .str("summary")
                .starts_with("2 runs ended together by an outside signal"),
            "{posted:?}"
        );
        for want in [
            "run `run-a` of job `job-a`",
            "run `run-b` of job `job-b`",
            "hive/aurora/job-a-failed",
        ] {
            assert!(posted[0].str("body").contains(want), "{want}: {posted:?}");
        }
        assert!(!posted[0].str("body").contains("run-c"), "{posted:?}");
        // Later, c is a finding of its own, and nothing is posted twice.
        tend_outside_signal_findings(&s, &at(400), "daemon");
        tend_outside_signal_findings(&s, &at(500), "daemon");
        let posted = findings();
        assert_eq!(posted.len(), 2, "{posted:?}");
        assert!(
            posted[1]
                .str("summary")
                .starts_with("run run-c ended by an outside SIGTERM"),
            "{posted:?}"
        );
    }
}
