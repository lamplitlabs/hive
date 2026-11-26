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
        let (open, closed): (Vec<KeptClone>, Vec<KeptClone>) = left
            .into_iter()
            .partition(|k| !matches!(k.job_status.as_str(), "done" | "cancelled"));
        for k in closed {
            if cancel.is_cancelled() || running_here(s, &name, &k.job) {
                continue;
            }
            let why = format!("its job is {}", k.job_status);
            match clean(s, &r, &k, "closed", now, actor) {
                Ok(()) => lines.push(format!(
                    "kept clone of {} removed from {name}: {why}",
                    k.job
                )),
                Err(e) => lines.push(format!(
                    "kept clone of {} on {name} not removed ({why}): {e}",
                    k.job
                )),
            }
        }
        if crate::executor::kept_left(s, &name).is_empty() {
            lift_mark(s, now, actor, &name, &mut lines);
            continue;
        }
        if open.is_empty() || !fetch_due(s, &name, now) {
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
                    let removed = match clean(s, &r, &k, "fetched", now, actor) {
                        Ok(()) => "the clone is removed from the machine".to_string(),
                        Err(e) => format!("the clone stays on the machine: {e}"),
                    };
                    lines.push(format!(
                        "kept work of {} fetched from {name}: {kept}; {removed}",
                        k.job
                    ));
                    post_fetched(s, now, actor, &name, &k, &kept, &removed);
                }
                Err(e) if clone_missing(&e.to_string()) => {
                    // Nothing left to fetch: the machine lost the clone (it
                    // was re-imaged, or a hand removed it).
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
/// left to fetch as a probe, so the next code run there is the test.
fn lift_mark(s: &hive_store::Store, now: &str, actor: &str, name: &str, lines: &mut Vec<String>) {
    if !crate::executor::fetch_broken(s, name) {
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

/// Whether a failed fetch says the clone is not on the machine at all (the
/// hand's answers for a job directory without a clone).
fn clone_missing(said: &str) -> bool {
    said.contains("no such clone") || said.contains("no clone for job")
}

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
    r: &Row,
    k: &KeptClone,
    why: &str,
    now: &str,
    actor: &str,
) -> anyhow::Result<()> {
    let (rc, _, err) =
        crate::executor::ssh_run(r, &["clean".into(), k.job.clone()], &[], CLEAN_TIMEOUT)?;
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
                said = o.stderr.trim().lines().last().unwrap_or("").to_string();
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
