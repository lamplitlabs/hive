//! Clones kept on rented and paired machines (TASK-133) find their way home
//! (TASK-138). A run whose commits could neither be fetched nor sent as a
//! bundle leaves its clone on the machine and marks the machine: it takes no
//! run that changes code until a fetch from it works again. Before this, no
//! run that could fetch ever went to such a machine, so the mark stayed until
//! a hand cleared it, and the kept clones stayed until the machine went.
//!
//! Each maintenance pass, for every machine holding kept clones:
//!
//! - a clone whose job is closed (done: its work landed another way;
//!   cancelled) is removed from the machine;
//! - every `KEPT_FETCH_MINUTES` the daemon fetches the other clones again,
//!   the fetch probe: their commits go to the failed-work branch in the
//!   project's checkout (`hive/<agent>/<job>-failed`, the branch a retry
//!   starts from) and a patch beside it, the clone is removed, and the
//!   machine's mark clears ([`crate::executor::note_fetch_ok`]), so code work
//!   goes to it again. A fetch that fails is written down as
//!   `executor.fetch-probe` and `hive executor list` shows it;
//! - a marked machine left with no kept clone has its mark lifted
//!   (`executor.fetch-retry`): the next code run there tests the fetch.

use serde_json::json;

use crate::executor::{FETCH_PROBE_EVENT, KEPT_FETCH_MINUTES, KEPT_GONE_EVENT, KeptClone};
use crate::util::{Row, event_at, is_nil, must_one, plus_minutes};

/// How long the clean verb may take on the machine.
const CLEAN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// One maintenance pass over the machines that keep clones; returns the
/// daemon's lines.
pub(crate) fn tend_kept_work(
    s: &hive_store::Store,
    cancel: &hive_bus::CancellationToken,
    now: &str,
    actor: &str,
) -> Vec<String> {
    let mut lines = Vec::new();
    for r in crate::executor::executor_rows(s, false) {
        if cancel.is_cancelled() {
            break;
        }
        let name = r.str("name");
        let left = crate::executor::kept_left(s, &name);
        if left.is_empty() {
            lift_mark(s, now, actor, &name, &mut lines);
            continue;
        }
        // A machine the probe found unreachable answers no clean and no
        // fetch: each try would only cost the connection's retries (up to
        // about 150 seconds a call on a paired machine that is off). The
        // executor probe brings it back to active, and the next pass then
        // tends it.
        if r.str("status") == "unreachable" {
            continue;
        }
        // One spacing for every call to the machine, cleans included: a
        // clean that failed is written down as a try like a failed fetch.
        if !fetch_due(s, &name, now) {
            continue;
        }
        let (open, closed): (Vec<KeptClone>, Vec<KeptClone>) = left
            .into_iter()
            .partition(|k| !matches!(k.job_status.as_str(), "done" | "cancelled"));
        let mut answered = true;
        for k in closed {
            if cancel.is_cancelled() {
                break;
            }
            if running_here(s, &name, &k.job) {
                continue;
            }
            let why = format!("its job is {}", k.job_status);
            match clean(s, cancel, &r, &k, "closed", now, actor) {
                Ok(()) => lines.push(format!(
                    "kept clone of {} removed from {name}: {why}",
                    k.job
                )),
                Err(e) => {
                    let note = crate::util::trunc(&format!("{}: not removed: {e}", k.job), 300);
                    probe_event(s, now, actor, &name, false, &note);
                    lines.push(format!(
                        "kept clone of {} on {name} not removed ({why}): {e}; the next try after {}",
                        k.job,
                        plus_minutes(now, KEPT_FETCH_MINUTES as f64)
                    ));
                    // The machine is the likely cause: the rest waits for
                    // the next try.
                    answered = false;
                    break;
                }
            }
        }
        if crate::executor::kept_left(s, &name).is_empty() {
            lift_mark(s, now, actor, &name, &mut lines);
            continue;
        }
        if !answered || open.is_empty() {
            continue;
        }
        for k in open {
            if cancel.is_cancelled() {
                break;
            }
            if running_here(s, &name, &k.job) {
                continue;
            }
            match fetch_kept(s, cancel, &r, &k) {
                Ok(kept) => {
                    probe_event(s, now, actor, &name, true, &format!("{}: {kept}", k.job));
                    let removed = match clean(s, cancel, &r, &k, "fetched", now, actor) {
                        Ok(()) => "the clone is removed from the machine".to_string(),
                        Err(e) => format!("the clone stays on the machine: {e}"),
                    };
                    lines.push(format!(
                        "kept work of {} fetched from {name}: {kept}; {removed}",
                        k.job
                    ));
                    post_fetched(s, now, actor, &name, &k, &kept, &removed);
                }
                Err(e) if e.downcast_ref::<CloneMissing>().is_some() => {
                    // Nothing left to fetch: the machine lost the clone (it
                    // was re-imaged, or a hand removed it). Its other clones
                    // are still tried.
                    let note = crate::util::trunc(&format!("{}: {e}", k.job), 300);
                    probe_event(s, now, actor, &name, false, &note);
                    if let Ok(conn) = s.conn() {
                        event_at(
                            &conn,
                            now,
                            actor,
                            KEPT_GONE_EVENT,
                            &name,
                            json!({"job": k.job, "run": k.run, "why": "missing"}),
                        );
                    }
                    lines.push(format!(
                        "kept clone of {} is gone from {name}; its commits are lost ({note})",
                        k.job
                    ));
                }
                Err(e) => {
                    let note = crate::util::trunc(&format!("{}: {e}", k.job), 300);
                    probe_event(s, now, actor, &name, false, &note);
                    lines.push(format!(
                        "fetch probe of {name} failed ({note}); the next after {}",
                        plus_minutes(now, KEPT_FETCH_MINUTES as f64)
                    ));
                    // The machine is the likely cause: its other clones wait
                    // for the next probe.
                    break;
                }
            }
        }
    }
    lines
}

/// Lifts the fetch mark of `name` when it holds no kept clone: nothing is
/// left to fetch as a probe, so the next code run there is the test. A mark
/// that run itself left ([`crate::executor::mark_follows_retry`]) stays:
/// the fault is unchanged, and lifting it again would strand one more run
/// there each time a kept job closed (review of PR 283 item 1). A fetch that
/// works or the owner activating the machine ends it.
fn lift_mark(s: &hive_store::Store, now: &str, actor: &str, name: &str, lines: &mut Vec<String>) {
    if !crate::executor::fetch_broken(s, name) || crate::executor::mark_follows_retry(s, name) {
        return;
    }
    let Ok(conn) = s.conn() else {
        return;
    };
    event_at(
        &conn,
        now,
        actor,
        crate::executor::FETCH_RETRY_EVENT,
        name,
        json!({"why": "no kept clone left"}),
    );
    lines.push(format!(
        "fetch mark lifted on {name}: it holds no kept clone, so the next code run there tests the fetch (its clone is kept again if the fetch fails)"
    ));
}

/// The machine's line in a failed fetch that says the clone is not on the
/// machine at all (the hand's answers for a job directory without a clone),
/// if there is one. `said` is the whole text: git ends a failed ssh fetch
/// with its own general lines ("and the repository exists."), after the
/// machine's, and the bundle verb's answer is only in the run log - matching
/// the last line alone never found it (review of PR 283 item 2).
fn missing_line(said: &str) -> Option<&str> {
    said.lines()
        .map(str::trim)
        .find(|l| l.contains("no such clone") || l.contains("no clone for job"))
}

/// A kept clone the machine no longer has: its commits are lost.
#[derive(Debug)]
struct CloneMissing(String);
impl std::fmt::Display for CloneMissing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the machine has no clone for it ({})", self.0)
    }
}
impl std::error::Error for CloneMissing {}

/// Whether the daemon may try the kept clones of `name` again.
fn fetch_due(s: &hive_store::Store, name: &str, now: &str) -> bool {
    let (at, _) = crate::executor::last_fetch_probe(s, name);
    at.is_empty()
        || crate::util::must_iso(now) - crate::util::must_iso(&at)
            >= chrono::Duration::minutes(KEPT_FETCH_MINUTES)
}

/// Whether a run of `job` is in flight on `name`: its clone is in use.
fn running_here(s: &hive_store::Store, name: &str, job: &str) -> bool {
    let Ok(conn) = s.conn() else {
        return true;
    };
    !is_nil(&must_one(
        &conn,
        "SELECT 1 FROM runs WHERE job_id = ? AND executor = ? AND status = 'running'",
        &[&job, &name],
    ))
}

fn probe_event(s: &hive_store::Store, now: &str, actor: &str, name: &str, ok: bool, note: &str) {
    if let Ok(conn) = s.conn() {
        event_at(
            &conn,
            now,
            actor,
            FETCH_PROBE_EVENT,
            name,
            json!({"ok": ok, "note": note}),
        );
    }
}

/// Removes the kept clone of `k` from the machine (the hand's `clean` verb:
/// the job directory, the processes left under it) and writes it down.
fn clean(
    s: &hive_store::Store,
    cancel: &hive_bus::CancellationToken,
    r: &Row,
    k: &KeptClone,
    why: &str,
    now: &str,
    actor: &str,
) -> anyhow::Result<()> {
    let (rc, _, err) = crate::executor::ssh_run_context(
        cancel,
        r,
        &["clean".into(), k.job.clone()],
        &[],
        CLEAN_TIMEOUT,
    )?;
    // A clone already gone (the machine was re-imaged, a hand removed it)
    // is gone all the same.
    if rc != 0 && !err.contains("no such file") && !err.contains("No such file") {
        anyhow::bail!("clean exit {rc}: {}", crate::util::trunc(err.trim(), 200));
    }
    event_at(
        &s.conn()?,
        now,
        actor,
        KEPT_GONE_EVENT,
        &r.str("name"),
        json!({"job": k.job, "run": k.run, "why": why}),
    );
    Ok(())
}

/// Fetches the commits of the kept clone `k` into the project's checkout as
/// the failed-work branch; returns where they are kept.
fn fetch_kept(
    s: &hive_store::Store,
    cancel: &hive_bus::CancellationToken,
    r: &Row,
    k: &KeptClone,
) -> anyhow::Result<String> {
    let conn = s.conn()?;
    let run = must_one(&conn, "SELECT * FROM runs WHERE id = ?", &[&k.run]);
    if is_nil(&run) {
        anyhow::bail!("its run {} is not in the store", k.run);
    }
    let job = must_one(&conn, "SELECT * FROM jobs WHERE id = ?", &[&k.job]);
    if is_nil(&job) {
        anyhow::bail!("the job is not in the store");
    }
    let project = crate::util::project_row(s, &run.str("project"));
    let root = project.str("path");
    if root.is_empty() {
        anyhow::bail!("project {} has no checkout", run.str("project"));
    }
    let agent = run.str("agent");
    let mut log = crate::exec::RunLog::default();
    let mut ex = crate::exec::new_ssh_exec(
        r.clone(),
        s,
        &s.home,
        job,
        project.clone(),
        &agent,
        &mut log,
    );
    ex.base = run.str("base");
    // A branch of that name already in the checkout (an earlier run of the
    // same job) is not this clone's work: fetch beside it, never skip the
    // fetch, so a clone is only removed once its own commits are here.
    let mut branch = crate::land::failed_work_branch(&agent, &k.job);
    if crate::util::git(
        &root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .ok()
    {
        branch = format!("{branch}-{}", r.str("name"));
        if crate::util::git(
            &root,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ],
        )
        .ok()
        {
            anyhow::bail!(
                "branches {} and {branch} both exist in {root}",
                crate::land::failed_work_branch(&agent, &k.job)
            );
        }
    }
    let mut said = String::new();
    let kept = crate::land::keep_failed_work(
        &root,
        |dst| match crate::run::fetch_run_work(s, cancel, &ex, &k.job, &root, dst, &log) {
            Ok(o) if o.exit_code == 0 => true,
            Ok(o) => {
                said = o.stderr.trim().to_string();
                false
            }
            Err(e) => {
                said = crate::run::err_message(&e);
                false
            }
        },
        &run.str("base"),
        &branch,
        &s.home,
        &project.str("slug"),
    );
    // The bundle fallback writes its answer next to this store's work
    // directory; the probe has no run whose cleanup would remove it.
    let _ = std::fs::remove_file(format!("{}.result", ex.bundle));
    let _ = std::fs::remove_file(&ex.bundle);
    if kept.is_empty() {
        // The whole of what git said and what the run log holds (the
        // bundle verb's answer goes only there).
        if let Some(line) = missing_line(&said).or(missing_line(&log.text())) {
            return Err(CloneMissing(line.to_string()).into());
        }
        // The note shows git's last line, the one that names the failure.
        said = said.lines().last().unwrap_or("").trim().to_string();
        if said.is_empty() {
            said = "the fetch and the bundle both failed".into();
        }
        anyhow::bail!("{said}");
    }
    // fetch_run_work cleared the machine's mark (executor.fetch-ok): code
    // work goes to it again.
    Ok(kept)
}

/// Says once, on the job's thread, that the work came back.
fn post_fetched(
    s: &hive_store::Store,
    now: &str,
    actor: &str,
    name: &str,
    k: &KeptClone,
    kept: &str,
    removed: &str,
) {
    let project = must_one(
        &match s.conn() {
            Ok(c) => c,
            Err(_) => return,
        },
        "SELECT project FROM jobs WHERE id = ?",
        &[&k.job],
    )
    .str("project");
    if project.is_empty() {
        return;
    }
    let _ = crate::board::create_post(
        s,
        now.to_string(),
        crate::board::PostOpts {
            kind: "finding".into(),
            project,
            area: "jobs".into(),
            thread: k.job.clone(),
            summary: crate::util::trunc(
                &format!("the commits of {} came back from {name}: {kept}", k.job),
                230,
            ),
            body: format!(
                "The run {} committed on {name}, but neither the fetch nor the bundle brought its commits back, so its clone was kept there (since {}). The daemon's fetch probe fetched them now: {kept}; {removed}. {name} takes code work again.\n",
                k.run, k.at
            ),
            agent: crate::board::actor_agent(s, actor.to_string()),
            ..Default::default()
        },
    );
}

#[cfg(test)]
mod tests {
    use crate::executor::{
        FETCH_FAILED_EVENT, FETCH_PROBE_EVENT, KEPT_FETCH_MINUTES, KEPT_GONE_EVENT,
    };
    use crate::test_helpers::TEST_NOW;
    use crate::util::{event_at, exec1, plus_minutes};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    /// A store with the ssh machine `m1` and the job `job` in project
    /// `proj`, whose status is `status`.
    fn machine_with_job(s: &hive_store::Store, job: &str, status: &str) {
        crate::executor::add_executor(
            s,
            TEST_NOW,
            "m1",
            "hive@m1.invalid",
            "/srv/hive",
            1,
            "*",
            crate::util::HUMAN_AGENT,
        )
        .unwrap();
        add_job(s, job, status);
    }

    fn add_job(s: &hive_store::Store, job: &str, status: &str) {
        crate::test_helpers_daemon::queued_job(s, job, "proj", "task", "owner");
        exec1(
            &*s.conn().unwrap(),
            "UPDATE jobs SET status = ? WHERE id = ?",
            &[&status, &job],
        );
    }

    /// The run of `job` on m1 could not fetch: its clone is kept there.
    fn fetch_failed(s: &hive_store::Store, at: &str, job: &str) {
        event_at(
            &s.conn().unwrap(),
            at,
            "runner",
            FETCH_FAILED_EVENT,
            "m1",
            json!({"job": job, "run": format!("run-{job}"), "clone": format!("/srv/hive/{job}/clone")}),
        );
    }

    /// The kept clone of `job` is gone from m1.
    fn kept_gone(s: &hive_store::Store, at: &str, job: &str) {
        event_at(
            &s.conn().unwrap(),
            at,
            "daemon",
            KEPT_GONE_EVENT,
            "m1",
            json!({"job": job, "run": format!("run-{job}"), "why": "closed"}),
        );
    }

    fn lifted(lines: &[String]) -> bool {
        lines
            .iter()
            .any(|l| l.starts_with("fetch mark lifted on m1"))
    }

    /// Review of PR 283 item 1: the daemon lifts a machine's mark once it
    /// holds nothing kept, so the next code run tests the fetch. When that
    /// run's fetch fails too and its job closes (it lands elsewhere after a
    /// requeue, or is cancelled), the mark is not lifted again: before, each
    /// closed kept job let one more run be stranded there, without a bound.
    /// A fetch that works or the owner activating the machine ends it.
    #[test]
    fn a_mark_that_followed_a_retry_is_not_lifted_again() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        machine_with_job(&s, "job-a", "done");
        let cancel = hive_bus::CancellationToken::new();
        let at = |m: f64| plus_minutes(TEST_NOW, m);
        // The first mark: its clone went with its closed job; nothing is
        // kept, so the mark is lifted and the next code run is the test.
        fetch_failed(&s, &at(1.0), "job-a");
        kept_gone(&s, &at(2.0), "job-a");
        assert!(crate::executor::fetch_broken(&s, "m1"));
        let lines = super::tend_kept_work(&s, &cancel, &at(3.0), "daemon");
        assert!(lifted(&lines), "{lines:?}");
        assert!(!crate::executor::fetch_broken(&s, "m1"));
        // That run failed to fetch the same way and its job closed too.
        add_job(&s, "job-b", "done");
        fetch_failed(&s, &at(4.0), "job-b");
        kept_gone(&s, &at(5.0), "job-b");
        assert!(crate::executor::mark_follows_retry(&s, "m1"));
        let lines = super::tend_kept_work(&s, &cancel, &at(6.0), "daemon");
        assert!(!lifted(&lines), "the mark was lifted again: {lines:?}");
        assert!(crate::executor::fetch_broken(&s, "m1"));
        let lines = super::tend_kept_work(&s, &cancel, &at(60.0), "daemon");
        assert!(!lifted(&lines), "{lines:?}");
        assert!(crate::executor::fetch_broken(&s, "m1"));
        // The owner activating the machine says it is fixed; a later mark
        // is a first one again and may be lifted.
        crate::executor::set_executor_status(
            &s,
            &at(61.0),
            "m1",
            "active",
            crate::util::HUMAN_AGENT,
            None,
        )
        .unwrap();
        assert!(!crate::executor::fetch_broken(&s, "m1"));
        add_job(&s, "job-c", "done");
        fetch_failed(&s, &at(62.0), "job-c");
        kept_gone(&s, &at(63.0), "job-c");
        assert!(!crate::executor::mark_follows_retry(&s, "m1"));
        let lines = super::tend_kept_work(&s, &cancel, &at(64.0), "daemon");
        assert!(lifted(&lines), "{lines:?}");
        // So does a fetch from it that worked.
        add_job(&s, "job-d", "done");
        fetch_failed(&s, &at(65.0), "job-d");
        crate::executor::note_fetch_ok(&s, "m1", &at(66.0), "job-e");
        add_job(&s, "job-f", "done");
        fetch_failed(&s, &at(67.0), "job-f");
        kept_gone(&s, &at(68.0), "job-d");
        kept_gone(&s, &at(68.0), "job-f");
        let lines = super::tend_kept_work(&s, &cancel, &at(69.0), "daemon");
        assert!(lifted(&lines), "{lines:?}");
    }

    /// Records each hands verb the pass sends to a machine and the token it
    /// was sent with; every call fails as a machine that is off does.
    struct Calls {
        verbs: Arc<Mutex<Vec<String>>>,
        tokens: Arc<Mutex<Vec<hive_bus::CancellationToken>>>,
        _guard: crate::test_helpers_hands::RunSshGuard,
    }
    impl Calls {
        fn install() -> Self {
            let verbs = Arc::new(Mutex::new(Vec::new()));
            let tokens = Arc::new(Mutex::new(Vec::new()));
            let (v, t) = (verbs.clone(), tokens.clone());
            let old = crate::executor::set_run_ssh(move |c, argv, _, _| {
                let verb = argv
                    .last()
                    .and_then(|w| w.strip_prefix("b64 "))
                    .and_then(|b| {
                        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b).ok()
                    })
                    .map(|raw| {
                        String::from_utf8_lossy(&raw)
                            .split('\0')
                            .next()
                            .unwrap_or("")
                            .to_string()
                    })
                    .unwrap_or_default();
                v.lock().unwrap().push(verb);
                t.lock().unwrap().push(c.clone());
                Ok((
                    255,
                    String::new(),
                    "ssh: connect to host m1.invalid port 22: Connection refused".into(),
                ))
            });
            Self {
                verbs,
                tokens,
                _guard: crate::test_helpers_hands::RunSshGuard::new(old),
            }
        }
        fn verbs(&self) -> Vec<String> {
            self.verbs.lock().unwrap().clone()
        }
    }

    /// Review of PR 283 item 3: a pass does not call a machine the probe
    /// found unreachable; a clean that failed waits the probe's spacing like
    /// a failed fetch (before, a paired machine that was off cost every pass
    /// the connection's retries for each closed clone); and the clean follows
    /// the pass's cancel (before, it ran on a token of its own).
    #[test]
    fn a_pass_spares_machines_that_do_not_answer() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = crate::test_helpers::new_store();
        machine_with_job(&s, "job-closed", "cancelled");
        fetch_failed(&s, TEST_NOW, "job-closed");
        let calls = Calls::install();
        let cancel = hive_bus::CancellationToken::new();
        // Unreachable: not called at all.
        exec1(
            &*s.conn().unwrap(),
            "UPDATE executors SET status = 'unreachable' WHERE name = 'm1'",
            &[],
        );
        let now = plus_minutes(TEST_NOW, 1.0);
        assert_eq!(
            super::tend_kept_work(&s, &cancel, &now, "daemon"),
            Vec::<String>::new()
        );
        assert!(calls.verbs().is_empty(), "{:?}", calls.verbs());
        // Back to active (the executor probe's doing): the clean is tried,
        // fails, and is written down as a try.
        exec1(
            &*s.conn().unwrap(),
            "UPDATE executors SET status = 'active' WHERE name = 'm1'",
            &[],
        );
        let lines = super::tend_kept_work(&s, &cancel, &now, "daemon");
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("kept clone of job-closed on m1 not removed")),
            "{lines:?}"
        );
        let tried = calls.verbs().len();
        assert!(
            tried > 0 && calls.verbs().iter().all(|v| v == "clean"),
            "{:?}",
            calls.verbs()
        );
        assert_eq!(
            crate::test_helpers::count(
                &s,
                "SELECT COUNT(*) AS n FROM events WHERE action = ? AND subject = 'm1'",
                &[&FETCH_PROBE_EVENT]
            ),
            1
        );
        // The clean ran on the pass's token: cancelling the pass ends it.
        assert!(
            calls
                .tokens
                .lock()
                .unwrap()
                .iter()
                .all(|t| !t.is_cancelled())
        );
        // Not again inside the spacing.
        let soon = plus_minutes(&now, (KEPT_FETCH_MINUTES - 1) as f64);
        assert_eq!(
            super::tend_kept_work(&s, &cancel, &soon, "daemon"),
            Vec::<String>::new()
        );
        assert_eq!(calls.verbs().len(), tried, "{:?}", calls.verbs());
        // Once the spacing is over it is tried again.
        let later = plus_minutes(&now, (KEPT_FETCH_MINUTES + 1) as f64);
        let _ = super::tend_kept_work(&s, &cancel, &later, "daemon");
        assert!(calls.verbs().len() > tried, "{:?}", calls.verbs());
        cancel.cancel();
        assert!(
            calls
                .tokens
                .lock()
                .unwrap()
                .iter()
                .all(|t| t.is_cancelled()),
            "the clean did not follow the pass's cancel"
        );
        // The clone is still kept: nothing said it is gone.
        assert_eq!(crate::executor::kept_left(&s, "m1").len(), 1);
        assert!(crate::executor::fetch_broken(&s, "m1"));
    }

    #[test]
    fn a_missing_clone_is_read_anywhere_in_the_text() {
        let said = "hands: no such clone\nfatal: Could not read from remote repository.\n\nPlease make sure you have the correct access rights\nand the repository exists.";
        assert_eq!(super::missing_line(said), Some("hands: no such clone"));
        let log = "$ ssh hive@m1 bundle job-x\nbundle on m1 failed (exit 1): hands: no clone for job job-x";
        assert_eq!(
            super::missing_line(log),
            Some("bundle on m1 failed (exit 1): hands: no clone for job job-x")
        );
        assert_eq!(
            super::missing_line("fatal: Could not read from remote repository."),
            None
        );
    }
}
