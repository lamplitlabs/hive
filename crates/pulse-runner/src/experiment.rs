#![allow(dead_code, reason = "not every item has a caller in the crate")]

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

pub(crate) const EXPERIMENT_MEASURE_BUDGET: Duration = Duration::from_secs(90 * 60);
pub(crate) const DEFAULT_EVOLVE_TOLERANCE: f64 = 0.02;

pub(crate) fn brief_fields(body: String) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in body.lines() {
        let line = line.trim().trim_start_matches(['-', '*', ' ']);
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim_matches(['*', ' ', '\t']).to_lowercase();
        let value = value.trim().trim_matches('`').to_string();
        out.entry(key)
            .and_modify(|v| *v = String::new())
            .or_insert(value);
    }
    out
}

/// Whether a Keep verdict, or an archive note that ends in one (`baseline:
/// cached …; the discard hinged on …; neutral (…)`), names a tie rather than
/// a regression: the last `; `-separated part that starts with a verdict
/// word decides.
fn note_is_neutral(reason_or_note: &str) -> bool {
    reason_or_note
        .rsplit("; ")
        .find(|part| {
            pulse_evolve::is_neutral_verdict(part)
                || part.starts_with("improved")
                || part.starts_with("discarded")
        })
        .is_some_and(pulse_evolve::is_neutral_verdict)
}

pub(crate) fn evolve_tolerance(s: &pulse_store::Store) -> f64 {
    s.setting("evolve.tolerance", "")
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|v| *v >= 0.0)
        .unwrap_or(DEFAULT_EVOLVE_TOLERANCE)
}

pub(crate) fn experiment_suggestion(job: crate::util::Row) -> (String, String) {
    let f = brief_fields(job.str("brief"));
    let mut suggestion = f.get("suggestion").cloned().unwrap_or_default();
    let hypothesis = f.get("hypothesis").cloned().unwrap_or_default();
    if suggestion.is_empty()
        && let Some(rest) = job.str("source").strip_prefix("suggestion:")
    {
        suggestion = rest.trim().to_string();
    }
    (suggestion, hypothesis)
}

pub(crate) fn experiment_parent(job: crate::util::Row) -> String {
    brief_fields(job.str("brief"))
        .get("parent")
        .and_then(|v| v.split_whitespace().next())
        .unwrap_or("")
        .trim()
        .to_string()
}

pub(crate) fn experiment_thread(
    s: &pulse_store::Store,
    slug: String,
    suggestion: String,
) -> (String, String, String) {
    if !suggestion.is_empty()
        && let Ok(conn) = s.conn()
    {
        let p = crate::util::must_one(
            &conn,
            "SELECT area, thread FROM posts WHERE id = ? AND project = ?",
            &[&suggestion, &slug],
        );
        if !p.0.is_empty() {
            return (p.str("area"), p.str("thread"), suggestion);
        }
    }
    let mut thread = crate::util::slugify(&suggestion, 40);
    if thread.is_empty() || thread == "x" {
        thread = "experiments".to_string();
    }
    ("evolution".into(), thread, String::new())
}

pub(crate) fn last_run_id(s: &pulse_store::Store, job_id: String) -> String {
    let Ok(conn) = s.conn() else {
        return String::new();
    };
    crate::util::must_one(
        &conn,
        "SELECT id FROM runs WHERE job_id = ? ORDER BY started_at DESC, id DESC LIMIT 1",
        &[&job_id],
    )
    .str("id")
}

pub(crate) fn score_line(sc: &pulse_evolve::api::Scores) -> String {
    format!(
        "{:.3} over {} fixture(s), {} held out",
        sc.total,
        sc.fixtures.len(),
        sc.heldout
    )
}

/// The base recorded for a run (`runs.base`): the commit its clone was made from.
/// Empty when the run is unknown or recorded none.
pub(crate) fn run_base(s: &pulse_store::Store, run_id: &str) -> String {
    if run_id.is_empty() {
        return String::new();
    }
    let Ok(conn) = s.conn() else {
        return String::new();
    };
    crate::util::must_one(&conn, "SELECT base FROM runs WHERE id = ?", &[&run_id])
        .str("base")
        .trim()
        .to_string()
}

/// The run's recorded base as a stand-in merge base for `branch`, used when `def` and the
/// branch share no commit at all - `def` was squashed to a new root commit after the
/// branch forked, so `git merge-base` finds nothing and a two-way merge conflicts on
/// every file. The run base qualifies only while it is still a commit in the repository
/// and an ancestor of the branch; otherwise this is empty and the caller keeps its
/// old behaviour.
pub(crate) fn run_base_fork(root: &str, branch: &str, run_base: &str) -> String {
    let run_base = run_base.trim();
    if run_base.is_empty() {
        return String::new();
    }
    let short = Duration::from_secs(30);
    let commit = crate::util::git_timeout(
        root,
        short,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{run_base}^{{commit}}"),
        ],
    );
    let sha = commit.stdout.trim().to_string();
    if !commit.ok() || sha.len() < 40 {
        return String::new();
    }
    if !crate::util::git_timeout(root, short, &["merge-base", "--is-ancestor", &sha, branch]).ok() {
        return String::new();
    }
    sha
}

/// Where `branch` forked from `def`: their merge base, or - when they share no history
/// (`def` squashed after the branch forked) - the run's base as `run_base_fork` accepts
/// it. Empty when neither is known.
pub(crate) fn fork_point(root: &str, def: &str, branch: &str, run_base: &str) -> String {
    let base = merge_base(root, def, branch);
    if base.is_empty() {
        run_base_fork(root, branch, run_base)
    } else {
        base
    }
}

/// The revision the lane measures for `branch`: the branch merged onto `def` the way
/// landing merges it, built from objects alone (`git merge-tree --write-tree` and
/// `commit-tree`) so neither the checkout nor the branch moves. A variant measured on its
/// own stale base would be compared against a baseline it never contained and would miss
/// every fix `def` gained since the branch forked. A branch that already contains `def`
/// is measured as it is; a conflicting one too, so that landing reports the conflict
/// itself. A branch that shares no commit with `def` (it forked before `def` was squashed
/// to a new root) is merged three-way from `run_base`, the commit its run's clone was made
/// from, so the measured tree is `def` plus the card's change rather than the branch's
/// whole stale tree. Returns the revision and a note for the log (empty when the branch
/// is measured as it is).
pub(crate) fn measured_variant(
    root: &str,
    def: &str,
    branch: &str,
    run_base: &str,
) -> (String, String) {
    let short = Duration::from_secs(30);
    if crate::util::git_timeout(root, short, &["merge-base", "--is-ancestor", def, branch]).ok() {
        return (branch.to_string(), String::new());
    }

    let fork = if merge_base(root, def, branch).is_empty() {
        run_base_fork(root, branch, run_base)
    } else {
        String::new()
    };
    let merge_base_flag = format!("--merge-base={fork}");
    let mut args = vec!["merge-tree", "--write-tree"];
    if !fork.is_empty() {
        args.push(&merge_base_flag);
    }
    args.extend([def, branch]);
    let tree = crate::util::git_timeout(root, Duration::from_secs(120), &args);
    if !tree.ok() {
        return (branch.to_string(), String::new());
    }
    let tree_id = tree.stdout.lines().next().unwrap_or("").trim().to_string();
    if tree_id.len() < 40 {
        return (branch.to_string(), String::new());
    }
    let def_sha = crate::util::git_timeout(root, short, &["rev-parse", "--verify", def]);
    let branch_sha = crate::util::git_timeout(root, short, &["rev-parse", "--verify", branch]);
    if !def_sha.ok() || !branch_sha.ok() {
        return (branch.to_string(), String::new());
    }
    let message = format!("Measure {branch} on {def}");
    let commit = crate::util::git_timeout_env(
        root,
        short,
        &[
            "GIT_AUTHOR_NAME=pulse".into(),
            "GIT_AUTHOR_EMAIL=pulse@pulse.local".into(),
            "GIT_COMMITTER_NAME=pulse".into(),
            "GIT_COMMITTER_EMAIL=pulse@pulse.local".into(),
        ],
        &[
            "commit-tree",
            &tree_id,
            "-p",
            def_sha.stdout.trim(),
            "-p",
            branch_sha.stdout.trim(),
            "-m",
            &message,
        ],
    );
    let sha = commit.stdout.trim().to_string();
    if !commit.ok() || sha.len() < 40 {
        return (branch.to_string(), String::new());
    }
    let from = if fork.is_empty() {
        String::new()
    } else {
        format!(
            " from its run base {} ({def} shares no history with it)",
            &fork[..12]
        )
    };
    (
        sha.clone(),
        format!(
            "measured as {branch} merged onto {def}{from} ({})",
            &sha[..12]
        ),
    )
}

pub(crate) fn default_commit(root: &str, def: &str) -> String {
    let rev = crate::util::git_timeout(
        root,
        Duration::from_secs(30),
        &["rev-parse", "--verify", &format!("{def}^{{commit}}")],
    );
    if rev.ok() {
        rev.stdout.trim().to_string()
    } else {
        String::new()
    }
}

pub(crate) fn archive_patch_path(home: &str, slug: &str, archive_id: &str) -> String {
    Path::new(home)
        .join("patches")
        .join("evolve")
        .join(crate::util::slugify(slug, 80))
        .join(format!("{archive_id}.patch"))
        .display()
        .to_string()
}

fn next_archive_id(s: &pulse_store::Store, project: &str) -> String {
    s.conn()
        .and_then(|conn| {
            conn.query_row(
                "SELECT COALESCE(MAX(CAST(SUBSTR(id, 3) AS INTEGER)), 0) + 1 FROM evolution_archive WHERE project = ?",
                [project],
                |r| r.get::<_, i64>(0),
            )
            .map_err(Into::into)
        })
        .map(|next| format!("B-{next:03}"))
        .unwrap_or_default()
}

fn merge_base(root: &str, def: &str, branch: &str) -> String {
    let base =
        crate::util::git_timeout(root, Duration::from_secs(30), &["merge-base", def, branch]);
    if base.ok() {
        base.stdout.trim().to_string()
    } else {
        String::new()
    }
}

/// Saves the experiment's diff - from `base`, where the branch forked (`fork_point`), to
/// the branch tip - as the archive's patch. Returns the patch path (empty when none was
/// written) and the base.
fn save_experiment_patch(
    s: &pulse_store::Store,
    root: &str,
    slug: &str,
    base: String,
    branch: &str,
    archive_id: &str,
    log: (&str, &str),
) -> (String, String) {
    if base.is_empty() || archive_id.is_empty() {
        return (String::new(), base);
    }
    let path = archive_patch_path(&s.home, slug, archive_id);
    let Some(parent) = Path::new(&path).parent() else {
        return (String::new(), base);
    };
    if let Err(e) = std::fs::create_dir_all(parent) {
        log_patch_skip(
            s,
            log.0,
            log.1,
            branch,
            format!("creating patch directory failed: {e}"),
        );
        return (String::new(), base);
    }
    let out = Command::new(pulse_gitenv::git_program())
        .args(["-C", root, "diff", "--binary", &base, branch])
        .output();
    match out {
        Ok(out) if out.status.success() => {
            if let Err(e) = std::fs::write(&path, out.stdout) {
                log_patch_skip(
                    s,
                    log.0,
                    log.1,
                    branch,
                    format!("writing patch failed: {e}"),
                );
                return (String::new(), base);
            }
            (path, base)
        }
        Ok(out) => {
            log_patch_skip(
                s,
                log.0,
                log.1,
                branch,
                format!(
                    "git diff failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ),
            );
            (String::new(), base)
        }
        Err(e) => {
            log_patch_skip(s, log.0, log.1, branch, format!("git diff failed: {e}"));
            (String::new(), base)
        }
    }
}

fn log_patch_skip(s: &pulse_store::Store, actor: &str, now: &str, branch: &str, reason: String) {
    if let Ok(conn) = s.conn() {
        crate::util::event_at(
            &conn,
            now,
            actor,
            "evolution.patch_skipped",
            branch,
            serde_json::json!({"branch": branch, "reason": reason}),
        );
    }
}

/// Why a measurement must wait, if the machine is saturated. A measurement is
/// two builds, so the lane obeys the daemon's hold like a leased run does,
/// instead of adding load on top of the runs the governor already refused.
pub(crate) fn measure_hold(s: &pulse_store::Store) -> Option<String> {
    crate::daemon_load::hold_reason(&measure_limits(s), &crate::daemon_load::pressure())
}

/// The governor's limits as the lane sees them (no `--max-jobs` flag here:
/// the lane measures on the daemon's and on `pulse land`'s machine alike).
fn measure_limits(s: &pulse_store::Store) -> crate::daemon_load::Limits {
    crate::daemon_load::limits(s, &crate::daemon_load::Machine::detect(), 0)
}

pub(crate) fn land_experiment(
    s: &pulse_store::Store,
    project: crate::util::Row,
    job: crate::util::Row,
    actor: String,
    now: String,
    verify: bool,
) -> (String, String) {
    let ctx = pulse_bus::CancellationToken::new();
    land_experiment_context(&ctx, s, project, job, actor, now, verify)
}

pub(crate) fn land_experiment_context(
    parent: &pulse_bus::CancellationToken,
    s: &pulse_store::Store,
    project: crate::util::Row,
    job: crate::util::Row,
    actor: String,
    now: String,
    verify: bool,
) -> (String, String) {
    let root = project.str("path");
    let def = {
        let d = project.str("default_branch");
        if d.is_empty() { "main".to_string() } else { d }
    };
    let slug = project.str("slug");
    let branch = job.str("branch");
    let (suggestion, hypothesis) = experiment_suggestion(job.clone());
    let parent_archive = experiment_parent(job.clone());
    let (area, thread, on) = experiment_thread(s, slug.clone(), suggestion.clone());
    let agent = crate::board::actor_agent(s, actor.clone());
    // A result post that fails must not undo the landing decision, but it must not
    // vanish either: owners read the board for "what landed", so the miss goes to
    // stderr, which the daemon log captures (same as land.rs).
    let post = |resolution: &str, summary: String, body: String| {
        if let Err(e) = crate::board::create_post(
            s,
            now.clone(),
            crate::board::PostOpts {
                kind: "result".into(),
                project: slug.clone(),
                area: area.clone(),
                thread: thread.clone(),
                summary: crate::util::trunc(&summary, 230),
                body,
                agent: agent.clone(),
                on: on.clone(),
                resolution: resolution.into(),
                date: now.clone(),
                ..Default::default()
            },
        ) {
            eprintln!(
                "{}",
                experiment_result_post_failed_warning(&branch, resolution, &e)
            );
        }
    };
    if let why = crate::landing_context::landing_stop(parent, s, root.clone())
        && !why.is_empty()
    {
        return ("stopped".into(), why);
    }

    let run_id = last_run_id(s, job.str("id"));
    let prior = match crate::landing_evidence::experiment_archive(
        s,
        slug.clone(),
        branch.clone(),
        run_id.clone(),
    ) {
        Ok(r) => r,
        Err(e) => return ("error".into(), crate::run::err_message(&e)),
    };
    let (exists, unreadable) = crate::land::branch_exists(root.clone(), branch.clone());
    if !unreadable.is_empty() {
        return ("unreadable".into(), format!("{branch} kept: {unreadable}"));
    }
    if !exists && (prior.0.is_empty() || prior.int("kept") != 0) {
        let archive_id = prior.str("id");
        let why = format!(
            "experiment artifact {branch} is missing; delivery was not confirmed; any measured archive is retained"
        );
        if let Err(e) = crate::landing_evidence::fail_missing_experiment(
            s,
            project.clone(),
            job.clone(),
            actor.clone(),
            run_id,
            archive_id,
            why.clone(),
        ) {
            return ("error".into(), crate::run::err_message(&e));
        }
        post(
            "dropped",
            format!("experiment {suggestion} failed: branch {branch} is missing"),
            format!(
                "Job {} has no branch artifact (`{branch}` does not exist in {root}). A measured keep is not proof of delivery. The job failed; its measurements remain unchanged. Owner review is needed.\n",
                job.str("id")
            ),
        );
        return ("missing".into(), why);
    }
    let tol = evolve_tolerance(s);
    // A branch the Keep rule did not land: `discarded` when the variant
    // regressed, `neutral` when nothing was shown to move (a tie, an
    // unconfirmed gain, an unmeasured variant). Both delete the branch and
    // record `land.dropped` with the verdict, but the board post and the
    // result line say which it was: a tie is not a regression.
    let drop_branch = |archive_id: String, reason: String, measured: String| -> (String, String) {
        let neutral = note_is_neutral(&reason);
        if let why = crate::landing_context::landing_stop(parent, s, root.clone())
            && !why.is_empty()
        {
            return ("stopped".into(), why);
        }
        if exists {
            let removed = crate::util::git_timeout(
                &root,
                Duration::from_secs(30),
                &["branch", "-D", &branch],
            );
            if !removed.ok() {
                return (
                    "deferred".into(),
                    format!(
                        "experiment {} (archive {archive_id}) awaits branch removal: {}",
                        if neutral { "neutral" } else { "discard" },
                        removed.stderr.trim()
                    ),
                );
            }
        }
        if let Err(e) = crate::landing_evidence::record_landing(
            s,
            project.clone(),
            branch.clone(),
            actor.clone(),
            "land.dropped".into(),
            format!(
                " | {}: {reason}",
                if neutral { "not landed" } else { "dropped" }
            ),
            crate::land::LandOpts {
                job: job.clone(),
                archive_id: archive_id.clone(),
                run_id: run_id.clone(),
                ..Default::default()
            },
            BTreeMap::from([
                ("why".into(), serde_json::json!(reason.clone())),
                (
                    "verdict".into(),
                    serde_json::json!(if neutral { "neutral" } else { "discarded" }),
                ),
            ]),
        ) {
            return ("error".into(), crate::run::err_message(&e));
        }
        if neutral {
            post(
                "dropped",
                format!("experiment {suggestion} not landed: {reason}"),
                format!(
                    "Branch `{branch}` was measured and scored neutral (archive {archive_id}): {measured}.\n\nNothing was shown to move, so the branch is not landed; it did not regress either. The branch is deleted; nothing changed on `{def}`.\n"
                ),
            );
            return (
                "dropped".into(),
                format!("{branch} not landed: {reason}; archive {archive_id}"),
            );
        }
        post(
            "dropped",
            format!("experiment {suggestion} dropped: {reason}"),
            format!(
                "Branch `{branch}` was measured and discarded (archive {archive_id}): {measured}.\n\nThe branch is deleted; nothing changed on `{def}`.\n"
            ),
        );
        (
            "dropped".into(),
            format!("{branch} dropped ({reason}); archive {archive_id}"),
        )
    };
    // A kept experiment goes to the forge as a pull request (`delivery.rs`,
    // kind `experiment`) and the local default branch follows the upstream
    // once the reviewer merges it: nothing merges locally any more, because
    // a local merge the upstream never saw diverges the two, and the brain
    // project's self-update reads the local default branch. A project with
    // no upstream at all lands locally as before. Returns the lane's
    // (state, note): `pr` when the pull request is open, the landing's own
    // states on the local path, `deferred` when the push or gh failed (held
    // once as `land.pr-held`; the lane retries on its schedule).
    let deliver_kept = |archive_id: String, summary: String| -> (String, String) {
        if crate::forge::upstream_of(&root, &def).is_none() {
            return match crate::land::land_branch(
                s,
                project.clone(),
                branch.clone(),
                actor.clone(),
                now.clone(),
                crate::land::LandOpts {
                    job: job.clone(),
                    verify,
                    ctx: Some(parent.clone()),
                    archive_id: archive_id.clone(),
                    run_id: run_id.clone(),
                    ..Default::default()
                },
            ) {
                Ok(v) => v,
                Err(e) => ("error".into(), crate::run::err_message(&e)),
            };
        }
        match crate::delivery::deliver_experiment(
            s,
            &project,
            &job,
            &archive_id,
            &run_id,
            &summary,
            &now,
        ) {
            Ok((number, url, _)) => (
                "pr".into(),
                format!("delivered as pull request #{number} {url}"),
            ),
            Err(note) => {
                let sha = crate::util::git_timeout(
                    &root,
                    Duration::from_secs(10),
                    &["rev-parse", "--verify", &format!("refs/heads/{branch}")],
                )
                .stdout
                .trim()
                .to_string();
                let work = format!("{branch} {}", crate::recipe::short(&sha, 12));
                if let Ok(conn) = s.conn() {
                    let held = crate::util::query_rows(
                        &conn,
                        "SELECT detail FROM events WHERE subject = ? AND action = 'land.pr-held'
                         AND json_extract(detail, '$.job') = ? ORDER BY seq DESC LIMIT 1",
                        &[&slug, &job.str("id")],
                    )
                    .unwrap_or_default()
                    .first()
                    .is_some_and(|r| {
                        serde_json::from_str::<serde_json::Value>(&r.str("detail"))
                            .ok()
                            .and_then(|d| {
                                d.get("work").and_then(|w| w.as_str()).map(str::to_string)
                            })
                            .as_deref()
                            == Some(work.as_str())
                    });
                    if !held {
                        crate::util::event_at(
                            &conn,
                            &now,
                            &actor,
                            "land.pr-held",
                            &slug,
                            serde_json::json!({"project": slug, "job": job.str("id"), "experiment": job.str("id"),
                                "archive": archive_id, "work": work, "note": note}),
                        );
                    }
                }
                ("deferred".into(), note)
            }
        }
    };
    // How the kept branch reached the default branch, for the result post.
    let kept_how = |state: &str, note: &str| -> String {
        if state == "pr" {
            format!(
                "It was {note}; the PULSE reviewer reviews and merges it, or asks the owner when it is complex."
            )
        } else {
            format!("It landed under ADR-024: {note}.")
        }
    };
    if !prior.0.is_empty() {
        if prior.int("kept") == 0 {
            return drop_branch(prior.str("id"), prior.str("note"), prior.str("note"));
        }
        let (state, note) = deliver_kept(prior.str("id"), prior.str("note"));
        match state.as_str() {
            "landed" | "merged" | "pr" => {
                post(
                    "tested",
                    format!("experiment {suggestion} kept: {}", prior.str("note")),
                    format!(
                        "Branch `{branch}` was measured and kept earlier (archive {}): {}.\n\n{}\n",
                        prior.str("id"),
                        prior.str("note"),
                        kept_how(&state, &note)
                    ),
                );
                return (
                    state,
                    format!("{branch} kept (archive {}); {note}", prior.str("id")),
                );
            }
            _ => {
                return (
                    state,
                    format!(
                        "experiment kept earlier (archive {}) but {note}",
                        prior.str("id")
                    ),
                );
            }
        }
    }
    // A deterministic archive validation refusal already held this job once:
    // the row text has not changed, so do not measure again on later passes.
    let held = crate::util::must_one(
        &s.conn().unwrap(),
        "SELECT 1 AS x FROM events WHERE action = 'land.unmeasured' AND subject = ? AND detail LIKE ? AND detail LIKE ?",
        &[
            &branch,
            &format!("%{}%", job.str("id")),
            &"%archive validation%".to_string(),
        ],
    );
    if !crate::util::is_nil(&held) {
        return (
            "unmeasured".into(),
            format!("{branch} not landed: measurement unavailable (archive validation)"),
        );
    }
    let limits = measure_limits(s);
    if let Some(why) = crate::daemon_load::hold_reason(&limits, &crate::daemon_load::pressure()) {
        return (
            "deferred".into(),
            format!("{branch} not landed: measuring waits - {why}"),
        );
    }
    // The measurement's builds obey daemon.build_jobs as a worker's do;
    // pulse-evolve has no store, so the cap is handed over here.
    pulse_evolve::process::set_build_jobs(usize::try_from(limits.build_jobs).unwrap_or(0));
    let _measuring = crate::daemon_load::MeasuringMark::new(&s.home);
    let ctx = parent.with_timeout(EXPERIMENT_MEASURE_BUDGET);
    let unavailable = |what: String, err: anyhow::Error| -> (String, String) {
        let summary = format!("experiment {suggestion} waits: measurement unavailable ({what})");
        let job_thread = crate::util::slugify(&job.str("id"), 40);
        if let Ok(conn) = s.conn() {
            let p = crate::util::must_one(
                &conn,
                "SELECT 1 AS x FROM posts WHERE project = ? AND kind = 'result' AND area = 'jobs' AND thread = ? AND summary = ?",
                &[&slug, &job_thread, &crate::util::trunc(&summary, 230)],
            );
            if p.0.is_empty()
                && let Err(e) = crate::board::create_post(
                    s,
                    now.clone(),
                    crate::board::PostOpts {
                        kind: "result".into(),
                        project: slug.clone(),
                        area: "jobs".into(),
                        thread: job_thread,
                        summary: crate::util::trunc(&summary, 230),
                        body: format!(
                            "Branch `{branch}` of experiment {suggestion} is not landed: {what} ({}). The job stays in review with its branch until the measurement is available; a human may land it with `pulse land --project {slug} --branch {branch}`.\n",
                            crate::run::err_message(&err)
                        ),
                        agent: agent.clone(),
                        date: now.clone(),
                        ..Default::default()
                    },
                )
            {
                eprintln!(
                    "{}",
                    experiment_result_post_failed_warning(&branch, "unmeasured", &e)
                );
            }
            crate::util::event_at(
                &conn,
                &now,
                &actor,
                "land.unmeasured",
                &branch,
                serde_json::json!({"project": slug, "job": job.str("id"), "what": what}),
            );
        }
        (
            "unmeasured".into(),
            format!("{branch} not landed: measurement unavailable ({what})"),
        )
    };
    let commit = default_commit(&root, &def);
    let fixture_hash = match pulse_evolve::baseline_fixture_hash(s, &slug) {
        Ok(v) => v,
        Err(e) => {
            return (
                "deferred".into(),
                format!(
                    "{branch} not landed: baseline cache key failed: {}",
                    crate::run::err_message(&e)
                ),
            );
        }
    };
    let mut baseline_cache_note = "baseline: measured".to_string();
    let cached = match pulse_evolve::cached_baseline(s, &slug, &commit, &fixture_hash) {
        Ok(v) => v,
        Err(e) => {
            return (
                "deferred".into(),
                format!(
                    "{branch} not landed: reading baseline cache failed: {}",
                    crate::run::err_message(&e)
                ),
            );
        }
    };
    let baseline = if let Some(cached) = cached {
        let short = commit.get(..7).unwrap_or(&commit);
        baseline_cache_note = format!("baseline: reused {}", cached.at);
        if let Ok(conn) = s.conn() {
            crate::util::event_at(
                &conn,
                &now,
                &actor,
                "evolution.baseline_cache",
                &commit,
                serde_json::json!({"project": slug, "commit": commit, "score": cached.score, "at": cached.at, "line": format!("baseline for {short} reused from {} (score {:.3})", cached.at, cached.score)}),
            );
        }
        cached.scores
    } else {
        let baseline = match pulse_evolve::measure(
            &ctx,
            s,
            &slug,
            &root,
            &def,
            pulse_evolve::MeasureOptions::default(),
        ) {
            Ok(v) => v,
            Err(e) if e.to_string() == pulse_evolve::err_not_implemented().to_string() => {
                return unavailable("evolve.Measure is not implemented".into(), e);
            }
            Err(e) => {
                return (
                    "deferred".into(),
                    format!(
                        "{branch} not landed: measuring {def} failed: {}",
                        crate::run::err_message(&e)
                    ),
                );
            }
        };
        if let Err(e) =
            pulse_evolve::upsert_baseline(s, &slug, &commit, &fixture_hash, &baseline, &run_id)
        {
            return (
                "deferred".into(),
                format!(
                    "{branch} not landed: writing baseline cache failed: {}",
                    crate::run::err_message(&e)
                ),
            );
        }
        baseline
    };
    if let why = crate::landing_context::landing_stop(parent, s, root.clone())
        && !why.is_empty()
    {
        return ("stopped".into(), why);
    }
    // A branch that forked before `def` was squashed shares no commit with it: the
    // commit the run's clone was made from stands in for the merge base.
    let run_base = run_base(s, &run_id);
    let (variant_rev, merged_note) = measured_variant(&root, &def, &branch, &run_base);
    // The variant and its base are measured in the card's diff scope: the
    // whole-tree baseline stays the cached one, the fixtures the card scopes
    // (and its own check) are measured again on the base and laid over it.
    let card_check = pulse_evolve::card_check_of(&job.str("brief"));
    let pair = match pulse_evolve::measure_pair(
        &ctx,
        s,
        &slug,
        &root,
        &def,
        &variant_rev,
        Some(baseline),
        &card_check,
        None,
    ) {
        Ok(v) => v,
        Err(e) if e.to_string() == pulse_evolve::err_not_implemented().to_string() => {
            return unavailable("evolve.Measure is not implemented".into(), e);
        }
        Err(e) => {
            return (
                "deferred".into(),
                format!(
                    "{branch} not landed: measuring {branch} failed: {}",
                    crate::run::err_message(&e)
                ),
            );
        }
    };
    let (keep, reason) = pulse_evolve::keep_pair(&pair, tol);
    let pulse_evolve::MeasuredPair {
        base: baseline,
        variant,
        scope,
        skipped,
    } = pair;
    let disputed = if skipped.is_some() {
        Vec::new()
    } else {
        pulse_evolve::hinges_on(&baseline, &variant, tol)
    };
    let (keep, reason, variant, dispute_note) = if disputed.is_empty() {
        (keep, reason, variant, String::new())
    } else {
        let measure_again = |r#ref: &str| {
            pulse_evolve::measure(
                &ctx,
                s,
                &slug,
                &root,
                r#ref,
                pulse_evolve::MeasureOptions {
                    fixtures: disputed.clone(),
                    exact: true,
                    scope: Some(scope.clone()),
                    card_check: card_check.clone(),
                    ..Default::default()
                },
            )
        };
        let again = measure_again(&def).and_then(|b| measure_again(&variant_rev).map(|v| (b, v)));
        let (baseline_again, variant_again) = match again {
            Ok(v) => v,
            Err(e) => {
                return (
                    "deferred".into(),
                    format!(
                        "{branch} not landed: measuring {} again failed: {}",
                        disputed.join(", "),
                        crate::run::err_message(&e)
                    ),
                );
            }
        };
        let r = pulse_evolve::reconcile(
            &baseline,
            &variant,
            &baseline_again,
            &variant_again,
            &disputed,
        );
        let (keep, reason) = pulse_evolve::keep(Some(&baseline), Some(&r.variant), tol);
        let note = format!(
            "the discard hinged on {}, measured again on both trees: {}",
            disputed.join(", "),
            r.notes.join("; ")
        );
        if let Ok(conn) = s.conn() {
            crate::util::event_at(
                &conn,
                &now,
                &actor,
                "evolution.measured_again",
                &branch,
                serde_json::json!({"project": slug, "fixtures": disputed, "noise": r.noise, "reproduced": r.reproduced, "kept": keep, "line": format!("{branch}: {note}; {reason}")}),
            );
        }
        (keep, reason, r.variant, note)
    };
    // A claimed gain must reproduce: the variant is measured a second time
    // on the same fixtures (the baseline is the cached one) and the gain
    // stands only when the second measurement also says improved. One
    // measurement over the tolerance is what a noisy judge produces by
    // itself; a gain that is not there the second time is scored neutral
    // and the branch is not landed. The second scores are archived; the
    // first delta stays in the note.
    let (keep, reason, variant, confirm_note) = if !keep
        || skipped.is_some()
        || !reason.starts_with("improved")
    {
        (keep, reason, variant, String::new())
    } else {
        let first_delta = variant.total - baseline.total;
        let again = pulse_evolve::measure(
            &ctx,
            s,
            &slug,
            &root,
            &variant_rev,
            pulse_evolve::MeasureOptions {
                scope: Some(scope.clone()),
                card_check: card_check.clone(),
                ..Default::default()
            },
        );
        let variant_again = match again {
            Ok(v) => v,
            Err(e) => {
                return (
                    "deferred".into(),
                    format!(
                        "{branch} not landed: measuring {branch} again to confirm the gain failed: {}",
                        crate::run::err_message(&e)
                    ),
                );
            }
        };
        let (keep_again, reason_again) =
            pulse_evolve::keep(Some(&baseline), Some(&variant_again), tol);
        let second_delta = variant_again.total - baseline.total;
        let confirmed = keep_again && reason_again.starts_with("improved");
        let (keep, reason, variant) = if confirmed {
            (
                true,
                format!(
                    "{reason_again} (confirmed: first measurement {first_delta:+.4}, second {second_delta:+.4})"
                ),
                variant_again,
            )
        } else {
            (
                false,
                format!(
                    "neutral (unconfirmed: first measurement {first_delta:+.4}, second {second_delta:+.4} (tolerance {tol:.4}); a gain that is not there the second time is not landed)"
                ),
                variant_again,
            )
        };
        let note = format!(
            "the gain was measured again on {branch}: first {first_delta:+.4}, second {second_delta:+.4}, {}",
            if confirmed {
                "confirmed"
            } else {
                "not confirmed"
            }
        );
        if let Ok(conn) = s.conn() {
            crate::util::event_at(
                &conn,
                &now,
                &actor,
                "evolution.measured_again",
                &branch,
                serde_json::json!({"project": slug, "confirm": true, "first": first_delta, "second": second_delta, "confirmed": confirmed, "kept": keep, "line": format!("{branch}: {note}; {reason}")}),
            );
        }
        (keep, reason, variant, note)
    };
    let mut archive_note = baseline_cache_note.clone();
    for part in [
        dispute_note.as_str(),
        confirm_note.as_str(),
        reason.as_str(),
    ] {
        if !part.is_empty() {
            archive_note.push_str("; ");
            archive_note.push_str(part);
        }
    }
    let sandbox = if variant.sandbox.is_empty() {
        baseline.sandbox.clone()
    } else {
        variant.sandbox.clone()
    };
    let expected_archive_id = next_archive_id(s, &slug);
    let (patch, base_commit) = save_experiment_patch(
        s,
        &root,
        &slug,
        fork_point(&root, &def, &branch, &run_base),
        &branch,
        &expected_archive_id,
        (&actor, &now),
    );
    let archive_id = match pulse_evolve::archive_row(
        s,
        pulse_evolve::ArchiveRow {
            project: slug.clone(),
            suggestion_post: suggestion.clone(),
            hypothesis: hypothesis.clone(),
            branch: branch.clone(),
            parent: parent_archive.clone(),
            patch,
            base_commit,
            baseline: Some(baseline.clone()),
            variant: Some(variant.clone()),
            tolerance: tol,
            kept: keep,
            run_id: run_id.clone(),
            sandbox,
            note: archive_note.clone(),
        },
    ) {
        Ok(v) => v,
        Err(e) if e.to_string() == pulse_evolve::err_not_implemented().to_string() => {
            return unavailable("evolve.Archive is not implemented".into(), e);
        }
        Err(e)
            if e.downcast_ref::<pulse_evolve::ArchiveValidation>()
                .is_some() =>
        {
            return unavailable(format!("archive validation for {suggestion}"), e);
        }
        Err(e) => {
            return (
                "deferred".into(),
                format!(
                    "{branch} not landed: archiving failed: {}",
                    crate::run::err_message(&e)
                ),
            );
        }
    };
    let mut measured = format!(
        "baseline {}; variant {}; tolerance {:.3}; {}",
        score_line(&baseline),
        score_line(&variant),
        tol,
        archive_note
    );
    if !merged_note.is_empty() {
        measured.push_str("; ");
        measured.push_str(&merged_note);
    }
    let hyp = if hypothesis.is_empty() {
        String::new()
    } else {
        format!("\n\nHypothesis: {hypothesis}")
    };
    if !keep {
        return drop_branch(archive_id, reason, measured + &hyp);
    }
    let (state, note) = deliver_kept(archive_id.clone(), format!("{measured}.{hyp}"));
    match state.as_str() {
        "landed" | "merged" | "pr" => {
            post(
                "tested",
                format!("experiment {suggestion} kept: {reason}"),
                format!(
                    "Branch `{branch}` was measured and kept (archive {archive_id}): {measured}.{hyp}\n\n{}\n",
                    kept_how(&state, &note)
                ),
            );
            (
                state,
                format!("{branch} kept ({reason}); archive {archive_id}; {note}"),
            )
        }
        "error" => (state, note),
        _ => {
            if let Ok(conn) = s.conn() {
                crate::util::event_at(
                    &conn,
                    &now,
                    &actor,
                    "land.kept-waiting",
                    &branch,
                    serde_json::json!({"project": slug, "job": job.str("id"), "archive": archive_id, "state": state}),
                );
            }
            (
                state,
                format!("experiment kept ({reason}; archive {archive_id}) but {note}"),
            )
        }
    }
}

/// The stderr line written when an experiment-landing result post could not be
/// created: the landing outcome itself stands, so the owner needs the branch and the
/// resolution to find it without the board entry.
pub(crate) fn experiment_result_post_failed_warning(
    branch: &str,
    resolution: &str,
    err: &anyhow::Error,
) -> String {
    format!(
        "experiment landing: `{branch}` resolved as {resolution} but the result post could not be written to the board: {err:#}; the outcome is recorded in the landing evidence"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::*;
    use crate::test_helpers_daemon::*;

    #[test]
    fn experiment_result_post_failure_warning_names_branch_and_cause() {
        let err = anyhow::anyhow!("board unavailable").context("create_post");
        let line = experiment_result_post_failed_warning("evolve/B-013", "landed", &err);
        assert!(line.contains("evolve/B-013"), "{line}");
        assert!(line.contains("landed"), "{line}");
        assert!(line.contains("create_post: board unavailable"), "{line}");
        assert!(line.contains("result post could not be written"), "{line}");
    }

    #[test]
    fn archive_patch_path_uses_evolve_project_archive_folder() {
        assert_eq!(
            archive_patch_path("/home/me/.pulse", "proj name", "B-017"),
            "/home/me/.pulse/patches/evolve/proj-name/B-017.patch"
        );
    }

    #[test]
    fn experiment_parent_reads_parent_line_from_brief() {
        let mut job = crate::util::Row::new();
        job.insert(
            "brief",
            serde_json::Value::String(
                "Suggestion: S-1\nParent: B-017 (patch ~/.pulse/patches/evolve/proj/B-017.patch, forked from abc, cmp 0.67)\n"
                    .into(),
            ),
        );
        assert_eq!(experiment_parent(job), "B-017");
    }

    #[test]
    fn measurement_waits_while_the_machine_is_saturated() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let (s, project, job, _root, calls) = measured_keep_fixture();
        set_setting(&s, "daemon.build_jobs", "3");
        let marks: std::sync::Arc<std::sync::Mutex<Vec<(usize, usize)>>> = Default::default();
        let seen = marks.clone();
        let home = s.home.clone();
        let _old =
            pulse_evolve::set_measure(Box::new(move |_ctx, _s, slug, _repo, r#ref, _opts| {
                seen.lock().unwrap().push((
                    crate::daemon_load::measuring(&home),
                    pulse_evolve::process::build_jobs(),
                ));
                let score = if r#ref == "main" { 0.8 } else { 0.9 };
                Ok(pulse_evolve::Scores {
                    project: slug.into(),
                    total: score,
                    ..Default::default()
                })
            }));
        let saturated = crate::daemon_load::pin_pressure(crate::daemon_load::Pressure {
            load1: Some(40.0),
            ..crate::daemon_load::CALM
        });
        let (state, note) = land_experiment(
            &s,
            project.clone(),
            job.clone(),
            "daemon".into(),
            TEST_NOW.into(),
            true,
        );
        assert_eq!(state, "deferred", "{note}");
        assert!(
            note.contains("not landed: measuring waits - machine saturated: load 40.0 on"),
            "{note}"
        );
        assert_eq!(*calls.lock().unwrap(), 0, "measured under a hold");
        assert_eq!(
            crate::daemon_maintenance::experiment_retry(&s, &state, &note),
            Duration::from_secs(120),
            "a held measurement waits the short retry"
        );
        assert_eq!(job_row(&s, &job.str("id")).str("branch"), job.str("branch"));
        drop(saturated);
        let (state, note) = land_experiment(
            &s,
            project,
            job.clone(),
            "daemon".into(),
            TEST_NOW.into(),
            true,
        );
        assert_eq!(state, "landed", "{note}");
        // All three measurements (main, the variant, and the variant again to
        // confirm the gain) ran under the lane's mark with the daemon's build
        // cap handed to pulse-evolve, and the mark went with them.
        assert_eq!(marks.lock().unwrap().clone(), vec![(1, 3), (1, 3), (1, 3)]);
        assert_eq!(crate::daemon_load::measuring(&s.home), 0);
        pulse_evolve::process::set_build_jobs(0);
    }

    #[test]
    fn variant_is_measured_merged_onto_main_when_main_moved() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let (s, project, job, root, _calls) = measured_keep_fixture();
        let branch = job.str("branch");
        // main gains a fix after the branch forked; the measured tree must carry both.
        std::fs::write(
            std::path::Path::new(&root).join("fix.txt"),
            "fixed on main\n",
        )
        .unwrap();
        run(&root, &["git", "add", "fix.txt"]);
        run(&root, &["git", "commit", "-q", "-m", "fix on main"]);
        let main_before = ref_of(&root, "main");
        let branch_before = ref_of(&root, &branch);
        let refs: std::sync::Arc<std::sync::Mutex<Vec<String>>> = Default::default();
        let seen = refs.clone();
        let _old =
            pulse_evolve::set_measure(Box::new(move |_ctx, _s, slug, _repo, r#ref, _opts| {
                seen.lock().unwrap().push(r#ref.to_string());
                let score = if r#ref == "main" { 0.8 } else { 0.9 };
                Ok(pulse_evolve::Scores {
                    project: slug.into(),
                    total: score,
                    ..Default::default()
                })
            }));
        let (state, note) = land_experiment(
            &s,
            project,
            job.clone(),
            "daemon".into(),
            TEST_NOW.into(),
            true,
        );
        assert_eq!(state, "landed", "{note}");
        let refs = refs.lock().unwrap().clone();
        // main, the merged variant, and the merged variant again to confirm.
        assert_eq!(refs.len(), 3, "{refs:?}");
        assert_eq!(refs[0], "main");
        assert_eq!(refs[1], refs[2], "the confirmation measures the same tree");
        let measured = &refs[1];
        assert!(
            measured != &branch && measured != &branch_before && measured.len() >= 40,
            "variant measured on its stale base: {measured}"
        );
        let parents = run(
            &root,
            &["git", "rev-list", "--parents", "-n", "1", measured],
        );
        assert!(
            parents.contains(&main_before) && parents.contains(&branch_before),
            "measured commit does not join main and the branch: {parents}"
        );
        assert_eq!(
            run(&root, &["git", "show", &format!("{measured}:README.md")]),
            "# measured variant\n"
        );
        assert_eq!(
            run(&root, &["git", "show", &format!("{measured}:fix.txt")]),
            "fixed on main\n"
        );
        // Landing merged the branch commit itself (and removed the branch), not the
        // measurement commit, which stays an unreferenced object.
        run(
            &root,
            &["git", "merge-base", "--is-ancestor", &branch_before, "main"],
        );
        assert!(
            !run(&root, &["git", "rev-list", "--parents", "-n", "1", "main"]).contains(measured),
            "the measurement commit landed"
        );
    }

    /// Rewrites `main` in the checkout at `root` as a new root commit: main's tree plus
    /// `squash.txt`, with no parent, the way an owner's "Squash project history" leaves it.
    /// Returns the new main.
    fn squash_main_to_a_new_root(root: &str) -> String {
        std::fs::write(
            std::path::Path::new(root).join("squash.txt"),
            "added with the squash\n",
        )
        .unwrap();
        run(root, &["git", "add", "squash.txt"]);
        let tree = run(root, &["git", "write-tree"]).trim().to_string();
        let squashed = run(
            root,
            &["git", "commit-tree", &tree, "-m", "Squash project history"],
        )
        .trim()
        .to_string();
        run(root, &["git", "reset", "-q", "--hard", &squashed]);
        squashed
    }

    fn changed_files(root: &str, from: &str, to: &str) -> Vec<String> {
        run(root, &["git", "diff", "--name-only", from, to])
            .lines()
            .map(ToOwned::to_owned)
            .collect()
    }

    #[test]
    fn a_branch_forked_before_main_was_squashed_is_measured_and_saved_from_its_run_base() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let (s, _project, job, root, _calls) = measured_keep_fixture();
        let branch = job.str("branch");
        let fork = ref_of(&root, "main");
        let branch_tip = ref_of(&root, &branch);
        let main = squash_main_to_a_new_root(&root);
        assert_eq!(
            merge_base(&root, "main", &branch),
            "",
            "histories still meet"
        );
        let run_id = last_run_id(&s, job.str("id"));
        exec1(
            &s,
            "UPDATE runs SET base = ? WHERE id = ?",
            &[&fork, &run_id],
        );
        assert_eq!(run_base(&s, &run_id), fork);

        // No run base, or one the repository no longer has: today's behaviour, the
        // bare branch is measured and no patch base is known.
        let gone = "0123456789abcdef0123456789abcdef01234567";
        for base in ["", gone] {
            let (rev, note) = measured_variant(&root, "main", &branch, base);
            assert_eq!(
                (rev.as_str(), note.as_str()),
                (branch.as_str(), ""),
                "{base}"
            );
            assert_eq!(fork_point(&root, "main", &branch, base), "", "{base}");
        }
        // A commit that is not on the branch is no fork point either.
        assert_eq!(run_base_fork(&root, &branch, &main), "");

        // With the run base: the branch's one change merged onto the squashed main.
        let (rev, note) = measured_variant(&root, "main", &branch, &fork);
        assert!(rev.len() >= 40 && rev != branch_tip, "{rev}: {note}");
        assert!(note.contains("from its run base"), "{note}");
        assert_eq!(changed_files(&root, &main, &rev), ["README.md"]);
        assert_eq!(
            run(&root, &["git", "show", &format!("{rev}:README.md")]),
            "# measured variant\n"
        );
        assert_eq!(
            run(&root, &["git", "show", &format!("{rev}:squash.txt")]),
            "added with the squash\n"
        );
        assert_eq!(
            run(&root, &["git", "status", "--porcelain"]).trim(),
            "",
            "merge-tree touched the checkout"
        );

        // The archive's patch holds the card's change alone, diffed from the run base.
        let base = fork_point(&root, "main", &branch, &fork);
        assert_eq!(base, fork);
        let (patch, base_commit) = save_experiment_patch(
            &s,
            &root,
            "proj",
            base,
            &branch,
            "B-900",
            ("daemon", TEST_NOW),
        );
        assert_eq!(base_commit, fork);
        let text = std::fs::read_to_string(&patch).expect("patch written");
        let touched = text
            .lines()
            .filter(|l| l.starts_with("diff --git "))
            .collect::<Vec<_>>();
        assert_eq!(touched, ["diff --git a/README.md b/README.md"], "{text}");
    }

    #[test]
    fn a_branch_forked_before_main_was_squashed_lands_its_own_commits() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let (s, project, job, root, _calls) = measured_keep_fixture();
        let branch = job.str("branch");
        let fork = ref_of(&root, "main");
        let branch_tip = ref_of(&root, &branch);
        let main = squash_main_to_a_new_root(&root);
        exec1(
            &s,
            "UPDATE runs SET base = ? WHERE id = ?",
            &[&fork, &last_run_id(&s, job.str("id"))],
        );
        let (state, note) = land_experiment(
            &s,
            project,
            job.clone(),
            "daemon".into(),
            TEST_NOW.into(),
            true,
        );
        assert_eq!(state, "landed", "{note}");
        assert!(note.contains("replayed from its run base"), "{note}");
        let landed = ref_of(&root, "main");
        assert_eq!(changed_files(&root, &main, &landed), ["README.md"]);
        assert_eq!(
            run(&root, &["git", "show", "main:README.md"]),
            "# measured variant\n"
        );
        // The squashed-away history stays out of main: neither the fork nor the
        // branch's original commit is reachable, only the copy of the latter.
        for gone in [&fork, &branch_tip] {
            let reachable = std::process::Command::new("git")
                .args(["-C", &root, "merge-base", "--is-ancestor", gone, "main"])
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .status()
                .unwrap()
                .success();
            assert!(!reachable, "{gone} came back into main");
        }
        let log = run(&root, &["git", "log", "--format=%an %s%n%b", "main"]);
        assert!(
            log.contains("t fake experiment variant")
                && log.contains(&format!("(cherry picked from commit {branch_tip})")),
            "{log}"
        );
        assert!(
            run(&root, &["git", "branch", "--list", &branch])
                .trim()
                .is_empty(),
            "the landed branch was kept"
        );
    }

    #[test]
    fn measured_variant_keeps_the_branch_when_it_contains_main_or_conflicts() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let (_s, _project, job, root, _calls) = measured_keep_fixture();
        let branch = job.str("branch");
        // The branch forked from the current main: nothing to merge.
        let (rev, note) = measured_variant(&root, "main", &branch, "");
        assert_eq!((rev.as_str(), note.as_str()), (branch.as_str(), ""));
        // main edits the same file the branch edited: the merge conflicts; landing reports it.
        std::fs::write(
            std::path::Path::new(&root).join("README.md"),
            "# conflicting main\n",
        )
        .unwrap();
        run(&root, &["git", "commit", "-q", "-am", "conflict on main"]);
        let (rev, note) = measured_variant(&root, "main", &branch, "");
        assert_eq!((rev.as_str(), note.as_str()), (branch.as_str(), ""));
        assert_eq!(
            run(&root, &["git", "status", "--porcelain"]).trim(),
            "",
            "merge-tree touched the checkout"
        );
    }

    #[test]
    fn a_kept_experiment_with_an_upstream_is_a_pull_request_until_merged() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let (s, project, job, root, calls) = measured_keep_fixture();
        let bare = Path::new(&s.home)
            .join("tmp")
            .join(format!("origin-{}.git", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(bare.parent().unwrap()).unwrap();
        let bare = bare.display().to_string();
        run(".", &["git", "init", "--bare", "-q", "-b", "main", &bare]);
        run(&root, &["git", "remote", "add", "origin", &bare]);
        run(&root, &["git", "push", "-q", "-u", "origin", "main"]);
        let branch = job.str("branch");
        let tip = run(&root, &["git", "rev-parse", &branch])
            .trim()
            .to_string();

        let (state, note) = land_experiment(
            &s,
            project.clone(),
            job.clone(),
            "daemon".into(),
            TEST_NOW.into(),
            true,
        );
        assert_eq!(
            state, "pr",
            "kept experiment with an upstream: {state}: {note}"
        );
        assert!(
            note.contains("delivered as pull request #7"),
            "note: {note}"
        );
        assert_eq!(
            *calls.lock().unwrap(),
            3,
            "main, variant, and the variant again to confirm the gain"
        );
        assert_eq!(bare_ref(&bare, &format!("refs/heads/{branch}")), tip);
        assert_eq!(gh.head_of(7), branch);
        assert!(
            !std::fs::read_to_string(Path::new(&root).join("README.md"))
                .unwrap()
                .contains("measured variant"),
            "the variant merged locally"
        );
        let pr = last_detail(&s, "land.pr");
        assert_eq!(pr["kind"], "experiment");
        assert_eq!(pr["experiment"], job.str("id"));
        assert_eq!(pr["job"], job.str("id"));
        assert_eq!(pr["archive"], "B-002");
        assert_eq!(pr["run_id"], last_run_id(&s, job.str("id")));
        assert_eq!(pr["opened"], true);
        let current = job_row(&s, &job.str("id"));
        assert_eq!(
            current.str("branch"),
            branch,
            "the job keeps its branch until the merge"
        );
        assert!(
            pulse_evolve::plan_feedback(&s, "proj", "daemon")
                .unwrap()
                .is_none(),
            "evolve saw the experiment as finalized before the merge"
        );
        // A second pass changes nothing: the pull request is open and the
        // branch has not moved.
        let again = land_experiment(
            &s,
            project.clone(),
            job.clone(),
            "daemon".into(),
            TEST_NOW.into(),
            true,
        );
        assert_eq!(again.0, "pr", "second pass: {again:?}");
        assert_eq!(
            gh.calls()
                .iter()
                .filter(|c| c.starts_with("pr create"))
                .count(),
            1
        );

        merge_on_forge(&gh, &bare, 7);
        let lines = crate::delivery::tend(&s, std::slice::from_ref(&project), TEST_NOW);
        assert!(
            lines
                .iter()
                .any(|l| l.contains("#7 merged; experiment finalized")),
            "{lines:?}"
        );
        let merged = last_detail(&s, "land.merged");
        assert_eq!(merged["kind"], "experiment");
        assert_eq!(merged["job"], job.str("id"));
        assert_eq!(merged["archive"], "B-002");
        assert_eq!(merged["run_id"], last_run_id(&s, job.str("id")));
        assert_eq!(merged["number"], 7);
        let current = job_row(&s, &job.str("id"));
        assert_eq!(current.str("branch"), "");
        assert!(
            current
                .str("result")
                .contains("delivered as pull request #7"),
            "{}",
            current.str("result")
        );
        assert!(
            run(&root, &["git", "branch", "--list", &branch])
                .trim()
                .is_empty(),
            "delivered branch kept locally"
        );
        assert!(
            std::fs::read_to_string(Path::new(&root).join("README.md"))
                .unwrap()
                .contains("measured variant"),
            "local main did not follow the upstream"
        );
        assert!(
            pulse_evolve::plan_feedback(&s, "proj", "daemon")
                .unwrap()
                .is_some(),
            "the merge did not finalize the experiment for evolve"
        );
    }

    #[test]
    fn missing_measured_keep_is_not_delivered() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let (s, project, job, root, calls) = measured_keep_fixture();
        defer_measured_keep(&s, project.clone(), job.clone(), &root);
        run(&root, &["git", "branch", "-D", &job.str("branch")]);
        let (state, note) = land_experiment(
            &s,
            project,
            job.clone(),
            "daemon".into(),
            TEST_NOW.into(),
            true,
        );
        assert_eq!(state, "missing", "missing kept artifact: {state}: {note}");
        let current = job_row(&s, &job.str("id"));
        assert_eq!(
            current.str("status"),
            "failed",
            "missing kept artifact was not terminal non-success: {current:?}"
        );
        assert_eq!(current.str("branch"), "");
        assert!(
            pulse_evolve::plan_feedback(&s, "proj", "daemon")
                .unwrap()
                .is_none()
        );
        let archives = pulse_evolve::list_archive(&s, "proj").unwrap();
        assert_eq!(archives.len(), 2, "archives: {archives:?}");
        assert!(archives[0].kept);
        assert_eq!(archives[0].variant_score, 0.9);
        assert_eq!(
            archives[0].outcome(),
            pulse_evolve::ArchiveOutcome::Improved
        );
        assert!(
            archives[0]
                .note
                .contains("the gain was measured again on evolve/")
                && archives[0]
                    .note
                    .contains(": first +0.1000, second +0.1000, confirmed; improved"),
            "the archive note keeps both deltas: {}",
            archives[0].note
        );
        assert_eq!(
            *calls.lock().unwrap(),
            3,
            "main, variant, and the variant again to confirm the gain"
        );
        assert_eq!(pulse_evolve::status(&s, "proj").unwrap().cycles, 1);
    }

    #[test]
    fn measured_landing_records_current_archive() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        for path in ["landed", "resumed", "merged"] {
            let (s, project, job, root, calls) = measured_keep_fixture();
            if path != "landed" {
                defer_measured_keep(&s, project.clone(), job.clone(), &root);
                std::fs::write(std::path::Path::new(&root).join("README.md"), "# proj\n").unwrap();
                if path == "merged" {
                    run(&root, &["git", "merge", "--ff-only", &job.str("branch")]);
                }
            }
            let (state, note) = land_experiment(
                &s,
                project,
                job.clone(),
                "daemon".into(),
                TEST_NOW.into(),
                true,
            );
            let want_state = if path == "merged" { "merged" } else { "landed" };
            assert_eq!(
                state, want_state,
                "successful fake landing: {state}: {note}"
            );
            assert_eq!(
                count(
                    &s,
                    "SELECT COUNT(*) AS n FROM events WHERE action = ? AND subject = ?
                    AND json_extract(detail, '$.project') = 'proj' AND json_extract(detail, '$.job') = ?
                    AND json_extract(detail, '$.archive') = 'B-002' AND json_extract(detail, '$.run_id') = ?",
                    &[&format!("land.{want_state}"), &job.str("branch"), &job.str("id"), &last_run_id(&s, job.str("id"))],
                ),
                1,
                "{path} left no durable current-run/current-archive landing proof"
            );
            assert!(
                pulse_evolve::plan_feedback(&s, "proj", "daemon")
                    .unwrap()
                    .is_some()
            );
            assert!(
                pulse_evolve::plan_feedback(&s, "proj", "daemon")
                    .unwrap()
                    .is_none()
            );
            assert_eq!(
                *calls.lock().unwrap(),
                3,
                "{path}: measured once, confirmed once"
            );
            assert_eq!(
                count(&s, "SELECT COUNT(*) AS n FROM evolution_archive", &[]),
                2
            );
        }
    }

    #[test]
    fn landing_receipt_failure_retries_finalization() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let (s, project, job, _root, calls) = measured_keep_fixture();
        exec1(
            &s,
            "CREATE TRIGGER reject_finalization BEFORE INSERT ON events WHEN NEW.action IN ('land.landed', 'land.merged', 'land.dropped') BEGIN SELECT RAISE(ABORT, 'fake finalization refused'); END",
            &[],
        );
        let (state, note) = land_experiment(
            &s,
            project.clone(),
            job.clone(),
            "daemon".into(),
            TEST_NOW.into(),
            true,
        );
        assert_eq!(
            state, "error",
            "receipt failure was hidden: {state}: {note}"
        );
        assert!(
            note.contains("fake finalization refused"),
            "receipt failure was hidden: {state}: {note}"
        );
        let current = job_row(&s, &job.str("id"));
        assert_eq!(current.str("status"), "done");
        assert_eq!(current.str("branch"), job.str("branch"));
        assert!(
            pulse_evolve::plan_feedback(&s, "proj", "daemon")
                .unwrap()
                .is_none()
        );
        exec1(&s, "DROP TRIGGER reject_finalization", &[]);
        let (state, note) = land_experiment(
            &s,
            project,
            job_row(&s, &job.str("id")),
            "daemon".into(),
            TEST_NOW.into(),
            true,
        );
        assert_eq!(
            state, "merged",
            "receipt retry did not finalize: {state}: {note}"
        );
        assert_eq!(*calls.lock().unwrap(), 3, "measured once, confirmed once");
        assert_eq!(
            count(&s, "SELECT COUNT(*) AS n FROM evolution_archive", &[]),
            2
        );
        assert!(
            pulse_evolve::plan_feedback(&s, "proj", "daemon")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn landing_receipt_failure_after_evidence_keeps_carrier() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        let (project, root) = new_repo(&s);
        join(&s, "aurora");
        let branch = "pulse/aurora/receipt-failure";
        run(&root, &["git", "switch", "-q", "-c", branch]);
        crate::test_helpers_landing::write_file(&root, "internal/api.go", "package internal\n");
        run(&root, &["git", "add", "-A"]);
        run(&root, &["git", "commit", "-q", "-m", "source change"]);
        run(&root, &["git", "switch", "-q", "main"]);
        let (job, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "Deliver source".into(),
            "do it".into(),
            1,
            0,
            "manual".into(),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .unwrap_or_else(|err| panic!("{err}"));
        exec1(
            &s,
            "UPDATE jobs SET status = 'done', branch = ?, lease_agent = ? WHERE id = ?",
            &[&branch, &"aurora", &job.str("id")],
        );
        exec1(
            &s,
            "CREATE TRIGGER reject_code_delivery_receipt BEFORE INSERT ON events\n\t\tWHEN NEW.action = 'land.landed' BEGIN SELECT RAISE(ABORT, 'receipt refused'); END",
            &[],
        );

        let result = crate::land::land_branch(
            &s,
            project,
            branch.into(),
            "daemon".into(),
            TEST_NOW.into(),
            crate::land::LandOpts {
                job: job_row(&s, &job.str("id")),
                verify: false,
                ..Default::default()
            },
        );
        match &result {
            Err(err) if err.to_string().contains("receipt refused") => {}
            Ok((state, note)) => {
                panic!("receipt failure hidden: state={state:?} note={note:?} err=<nil>")
            }
            Err(err) => panic!("receipt failure hidden: state=\"\" note=\"\" err={err}"),
        }
        let current = job_row(&s, &job.str("id"));
        if current.str("branch") != branch || current.str("status") != "done" {
            panic!("failed receipt cleared carrier: {current:?}");
        }
        let (exists, why) = crate::land::branch_exists(root.clone(), branch.into());
        if !exists || !why.is_empty() {
            panic!("failed receipt removed branch: exists={exists} why={why:?}");
        }
        let n = count(
            &s,
            "SELECT COUNT(*) AS n FROM events WHERE action = 'land.landed'",
            &[],
        );
        if n != 0 {
            panic!("failed receipt left success event: {n}");
        }
    }

    #[test]
    fn discard_finalization_retries_without_another_measurement() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let (s, project, job, root, calls) = measured_keep_fixture();
        let before = ref_of(&root, "HEAD");
        let _old =
            pulse_evolve::set_measure(Box::new(move |_ctx, _s, slug, _repo, r#ref, _opts| {
                *calls.lock().unwrap() += 1;
                let score = if r#ref == "main" { 0.8 } else { 0.7 };
                Ok(pulse_evolve::Scores {
                    project: slug.into(),
                    total: score,
                    ..Default::default()
                })
            }));
        let lock = std::path::Path::new(&root)
            .join(".git")
            .join("refs")
            .join("heads")
            .join(format!("{}.lock", job.str("branch")));
        std::fs::create_dir_all(lock.parent().unwrap()).unwrap();
        std::fs::write(&lock, "").unwrap();
        let (state, note) = land_experiment(
            &s,
            project.clone(),
            job.clone(),
            "daemon".into(),
            TEST_NOW.into(),
            true,
        );
        assert_eq!(
            state, "deferred",
            "failed discard cleanup did not stay pending: {state}: {note}"
        );
        assert!(note.contains("archive B-002"));
        assert_eq!(job_row(&s, &job.str("id")).str("branch"), job.str("branch"));
        assert!(
            pulse_evolve::plan_feedback(&s, "proj", "daemon")
                .unwrap()
                .is_none()
        );
        std::fs::remove_file(lock).unwrap();
        let (state, note) = land_experiment(
            &s,
            project,
            job.clone(),
            "daemon".into(),
            TEST_NOW.into(),
            true,
        );
        assert_eq!(
            state, "dropped",
            "discard did not finish after its cleanup gate opened: {state}: {note}"
        );
        assert_eq!(
            count(&s, "SELECT COUNT(*) AS n FROM evolution_archive", &[]),
            2
        );
        assert_eq!(ref_of(&root, "HEAD"), before);
        let (exists, why) = crate::land::branch_exists(root, job.str("branch"));
        assert!(
            !exists && why.is_empty(),
            "finalized discard retained the branch: {exists}, {why}"
        );
    }

    #[test]
    fn archive_validation_error_holds_once_without_remeasuring() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let (s, project, job, root, calls) = measured_keep_fixture();
        let before = ref_of(&root, "HEAD");
        let measure_calls = calls.clone();
        let _old_measure =
            pulse_evolve::set_measure(Box::new(move |_ctx, _s, slug, _repo, r#ref, _opts| {
                *measure_calls.lock().unwrap() += 1;
                let score = if r#ref == "main" { 0.8 } else { 0.7 };
                Ok(pulse_evolve::Scores {
                    project: slug.into(),
                    total: score,
                    ..Default::default()
                })
            }));
        // The same refusal the credential filter raises on every pass: a
        // deterministic ArchiveValidation error, never a transient store error.
        // The seam is process-wide, so the stub refuses only this job's branch
        // and delegates every other row to the prior hook, held in an Arc.
        let archive_calls = std::sync::Arc::new(std::sync::Mutex::new(0usize));
        let count_archive = archive_calls.clone();
        let branch = job.str("branch");
        let prior: std::sync::Arc<
            std::sync::Mutex<Option<std::sync::Arc<pulse_evolve::ArchiveFn>>>,
        > = std::sync::Arc::new(std::sync::Mutex::new(None));
        let delegate = prior.clone();
        let old = pulse_evolve::set_archive(Box::new(move |s, row| {
            if row.branch == branch {
                *count_archive.lock().unwrap() += 1;
                return Err(anyhow::Error::new(pulse_evolve::ArchiveValidation(
                    anyhow::anyhow!("archive text must not contain credentials"),
                )));
            }
            let hook = delegate.lock().unwrap().clone();
            match hook {
                Some(h) => h(s, row),
                // The delegate is set before land_experiment runs; this fallback is
                // unreachable for non-matching rows but never recurses.
                None => unreachable!("archive delegate not installed"),
            }
        }));
        *prior.lock().unwrap() = Some(std::sync::Arc::from(old));
        let (state, note) = land_experiment(
            &s,
            project.clone(),
            job.clone(),
            "daemon".into(),
            TEST_NOW.into(),
            true,
        );
        assert_eq!(
            state, "unmeasured",
            "a deterministic archive validation error must not defer for re-measurement: {state}: {note}"
        );
        assert_eq!(*calls.lock().unwrap(), 2, "main + variant measured once");
        assert_eq!(*archive_calls.lock().unwrap(), 1, "archive attempted once");
        assert!(
            note.contains("not landed: measurement unavailable"),
            "note: {note}"
        );
        let posted = count(
            &s,
            "SELECT COUNT(*) AS n FROM posts WHERE kind = 'result' AND summary LIKE '%measurement unavailable%'",
            &[],
        );
        assert_eq!(posted, 1, "one result post names the refused field");
        let (state, note) = land_experiment(
            &s,
            project,
            job.clone(),
            "daemon".into(),
            TEST_NOW.into(),
            true,
        );
        assert_eq!(state, "unmeasured", "second pass: {state}: {note}");
        assert_eq!(
            *calls.lock().unwrap(),
            2,
            "a second lane pass does not measure again"
        );
        assert_eq!(
            *archive_calls.lock().unwrap(),
            1,
            "no second archive attempt"
        );
        let posted = count(
            &s,
            "SELECT COUNT(*) AS n FROM posts WHERE kind = 'result' AND summary LIKE '%measurement unavailable%'",
            &[],
        );
        assert_eq!(posted, 1, "no duplicate result post");
        assert_eq!(ref_of(&root, "HEAD"), before, "the branch is untouched");
        // Restore the real archive hook so the rest of the process sees it.
        let real = prior.lock().unwrap().clone();
        if let Some(real) = real {
            let _ = pulse_evolve::set_archive(Box::new(move |s, row| real(s, row)));
        }
    }

    /// One metric fixture at weight 2 and one request fixture at weight 1:
    /// the request fixture flipping from 1 to 0 moves the total by a third,
    /// far beyond the tolerance.
    fn two_fixture_scores(slug: &str, request: f64, only: &[String]) -> pulse_evolve::Scores {
        let mut fixtures = vec![
            pulse_evolve::FixtureScore {
                name: "metric-size".into(),
                kind: "public".into(),
                score: 0.5,
                weight: 2.0,
                metric: true,
                note: String::new(),
            },
            pulse_evolve::FixtureScore {
                name: "starter-proposes-first".into(),
                kind: "public".into(),
                score: request,
                weight: 1.0,
                metric: false,
                note: String::new(),
            },
        ];
        if !only.is_empty() {
            fixtures.retain(|f| only.contains(&f.name));
        }
        pulse_evolve::Scores {
            project: slug.into(),
            total: pulse_evolve::weighted_total(&fixtures),
            fixtures,
            ..Default::default()
        }
    }

    type Seen = std::sync::Arc<std::sync::Mutex<Vec<(String, Vec<String>, bool)>>>;

    /// A measurement whose request fixture scores `first` on the baseline
    /// and 0 on the variant the first time, then `again` on both trees when
    /// measured again.
    fn flaky_measure(first: f64, again: f64) -> Seen {
        let seen: Seen = Default::default();
        let log = seen.clone();
        let _old =
            pulse_evolve::set_measure(Box::new(move |_ctx, _s, slug, _repo, r#ref, opts| {
                let mut log = log.lock().unwrap();
                log.push((r#ref.to_string(), opts.fixtures.clone(), opts.exact));
                let request = match (log.len(), r#ref == "main") {
                    (1, true) => first,
                    (2, false) => 0.0,
                    (3, true) => again,
                    _ => 0.0,
                };
                Ok(two_fixture_scores(slug, request, &opts.fixtures))
            }));
        seen
    }

    #[test]
    fn a_discard_on_a_request_fixture_that_does_not_reproduce_is_a_tie_not_a_discard() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let (s, project, job, root, _calls) = measured_keep_fixture();
        let before = ref_of(&root, "HEAD");
        let seen = flaky_measure(1.0, 0.0);
        let (state, note) = land_experiment(
            &s,
            project,
            job.clone(),
            "daemon".into(),
            TEST_NOW.into(),
            true,
        );
        // The flip was noise, so the variant ties its baseline: a tie does
        // not land, but it is reported as neutral, not as a regression.
        assert_eq!(state, "dropped", "{note}");
        assert!(
            note.contains("not landed: neutral (a tie:") && !note.contains("discarded"),
            "a tie is not called a discard: {note}"
        );
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 4, "both trees measured again: {seen:?}");
        assert_eq!(seen[0].0, "main");
        assert!(seen[0].1.is_empty() && seen[1].1.is_empty());
        assert_eq!(seen[2].0, "main");
        assert_eq!(
            seen[3].0, seen[1].0,
            "the same variant commit is measured again"
        );
        assert_eq!(seen[2].1, ["starter-proposes-first"]);
        assert_eq!(seen[3].1, ["starter-proposes-first"]);
        assert!(seen[2].2 && seen[3].2, "the second look is exact");
        let row = pulse_evolve::list_archive(&s, "proj").unwrap().remove(0);
        assert!(!row.kept, "{row:?}");
        assert_eq!(row.outcome(), pulse_evolve::ArchiveOutcome::Neutral);
        assert_eq!(row.baseline_score, row.variant_score);
        assert!(
            row.note.contains(
                "the discard hinged on starter-proposes-first, measured again on both trees: \
                 starter-proposes-first: baseline 1.0000 then 0.0000, variant 0.0000 then 0.0000; \
                 a difference that does not reproduce scores as a tie; neutral (a tie: the hypothesis went unmeasured"
            ),
            "{}",
            row.note
        );
        let variant = row.fixtures.variant.as_ref().unwrap();
        let fixture = variant
            .fixtures
            .iter()
            .find(|f| f.name == "starter-proposes-first")
            .unwrap();
        assert_eq!(fixture.score, 1.0);
        assert_eq!(
            fixture.note,
            "baseline 1.0000 then 0.0000, variant 0.0000 then 0.0000; scored as a tie"
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM events WHERE action = 'evolution.measured_again'",
                &[]
            ),
            1
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM events WHERE action = 'land.dropped' AND detail LIKE '%\"verdict\": \"neutral\"%'",
                &[]
            ),
            1,
            "the landing record names the verdict"
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM posts WHERE kind = 'result' AND summary LIKE '%not landed: neutral%'",
                &[]
            ),
            1,
            "the board post says the tie was not landed"
        );
        let (exists, _) = crate::land::branch_exists(root.clone(), job.str("branch"));
        assert!(!exists, "a tie's branch goes like a discard's");
        assert_eq!(ref_of(&root, "HEAD"), before, "nothing landed");
    }

    #[test]
    fn a_tie_is_not_landed_and_is_archived_neutral() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let (s, project, job, root, calls) = measured_keep_fixture();
        let before = ref_of(&root, "HEAD");
        let seen = calls.clone();
        let _old =
            pulse_evolve::set_measure(Box::new(move |_ctx, _s, slug, _repo, r#ref, _opts| {
                *seen.lock().unwrap() += 1;
                let score = if r#ref == "main" { 0.8 } else { 0.81 };
                Ok(pulse_evolve::Scores {
                    project: slug.into(),
                    total: score,
                    ..Default::default()
                })
            }));
        let (state, note) = land_experiment(
            &s,
            project,
            job.clone(),
            "daemon".into(),
            TEST_NOW.into(),
            true,
        );
        assert_eq!(state, "dropped", "{note}");
        assert!(
            note.contains("not landed: neutral (a tie: the hypothesis went unmeasured, no fixture moved, so the branch is not landed): variant +0.0100 vs baseline (tolerance 0.0200)"),
            "{note}"
        );
        assert_eq!(
            *calls.lock().unwrap(),
            2,
            "a tie is not measured again: there is no gain to confirm"
        );
        let row = pulse_evolve::list_archive(&s, "proj").unwrap().remove(0);
        assert!(!row.kept);
        assert_eq!(row.outcome(), pulse_evolve::ArchiveOutcome::Neutral);
        let st = pulse_evolve::status(&s, "proj").unwrap();
        assert_eq!((st.improved, st.neutral, st.kept), (0, 1, 0), "{st:?}");
        let (exists, _) = crate::land::branch_exists(root.clone(), job.str("branch"));
        assert!(!exists, "the branch is deleted");
        assert_eq!(ref_of(&root, "HEAD"), before, "nothing landed");
    }

    #[test]
    fn a_gain_the_second_measurement_does_not_confirm_is_neutral() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let (s, project, job, root, calls) = measured_keep_fixture();
        let before = ref_of(&root, "HEAD");
        let seen = calls.clone();
        let _old =
            pulse_evolve::set_measure(Box::new(move |_ctx, _s, slug, _repo, r#ref, opts| {
                let n = {
                    let mut n = seen.lock().unwrap();
                    *n += 1;
                    *n
                };
                // main 0.8; the variant 0.831 the first time, 0.804 the second.
                let score = match (r#ref == "main", n) {
                    (true, _) => 0.8,
                    (false, 2) => 0.831,
                    _ => 0.804,
                };
                assert!(
                    opts.fixtures.is_empty() && !opts.exact,
                    "the confirmation measures the same fixtures as the first look: {:?} exact={}",
                    opts.fixtures,
                    opts.exact
                );
                Ok(pulse_evolve::Scores {
                    project: slug.into(),
                    total: score,
                    ..Default::default()
                })
            }));
        let (state, note) = land_experiment(
            &s,
            project,
            job.clone(),
            "daemon".into(),
            TEST_NOW.into(),
            true,
        );
        assert_eq!(state, "dropped", "{note}");
        assert!(
            note.contains("not landed: neutral (unconfirmed: first measurement +0.0310, second +0.0040 (tolerance 0.0200); a gain that is not there the second time is not landed)"),
            "{note}"
        );
        assert_eq!(*calls.lock().unwrap(), 3, "main, variant, variant again");
        let row = pulse_evolve::list_archive(&s, "proj").unwrap().remove(0);
        assert!(!row.kept);
        assert_eq!(row.outcome(), pulse_evolve::ArchiveOutcome::Neutral);
        assert!(
            (row.variant_score - 0.804).abs() < 1e-9,
            "the second measurement is the archived one: {row:?}"
        );
        assert!(
            row.note.contains("the gain was measured again on evolve/")
                && row.note.contains(
                    ": first +0.0310, second +0.0040, not confirmed; neutral (unconfirmed"
                ),
            "both deltas stay in the note: {}",
            row.note
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM events WHERE action = 'evolution.measured_again' AND detail LIKE '%\"confirm\": true%' AND detail LIKE '%\"confirmed\": false%'",
                &[]
            ),
            1
        );
        let (exists, _) = crate::land::branch_exists(root.clone(), job.str("branch"));
        assert!(!exists, "the branch is deleted");
        assert_eq!(ref_of(&root, "HEAD"), before, "nothing landed");
    }

    #[test]
    fn a_discard_on_a_request_fixture_that_reproduces_stands() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let (s, project, job, root, _calls) = measured_keep_fixture();
        let before = ref_of(&root, "HEAD");
        let seen = flaky_measure(1.0, 1.0);
        let (state, note) = land_experiment(
            &s,
            project,
            job.clone(),
            "daemon".into(),
            TEST_NOW.into(),
            true,
        );
        assert_eq!(state, "dropped", "{note}");
        assert_eq!(seen.lock().unwrap().len(), 4);
        let row = pulse_evolve::list_archive(&s, "proj").unwrap().remove(0);
        assert!(!row.kept);
        assert!(
            row.note.contains(
                "starter-proposes-first: baseline 1.0000 then 1.0000, variant 0.0000 then 0.0000; \
                 the difference reproduces; discarded"
            ),
            "{}",
            row.note
        );
        assert_eq!(ref_of(&root, "HEAD"), before, "nothing landed");
    }

    #[test]
    fn a_discard_on_a_metric_fixture_is_not_measured_again() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let (s, project, job, _root, _calls) = measured_keep_fixture();
        let seen: Seen = Default::default();
        let log = seen.clone();
        let _old =
            pulse_evolve::set_measure(Box::new(move |_ctx, _s, slug, _repo, r#ref, opts| {
                log.lock()
                    .unwrap()
                    .push((r#ref.to_string(), opts.fixtures.clone(), opts.exact));
                let mut scores = two_fixture_scores(slug, 1.0, &[]);
                if r#ref != "main" {
                    scores.fixtures[0].score = 0.1;
                    scores.total = pulse_evolve::weighted_total(&scores.fixtures);
                }
                Ok(scores)
            }));
        let (state, note) =
            land_experiment(&s, project, job, "daemon".into(), TEST_NOW.into(), true);
        assert_eq!(state, "dropped", "{note}");
        assert_eq!(
            seen.lock().unwrap().len(),
            2,
            "a metric regression needs no second look"
        );
    }

    fn finished_experiment_job(
        s: &pulse_store::Store,
        root: &str,
        source: String,
        branch: &str,
        body: &str,
    ) -> crate::util::Row {
        run(root, &["git", "switch", "-q", "-c", branch]);
        std::fs::write(std::path::Path::new(root).join("README.md"), body).unwrap();
        run(root, &["git", "commit", "-q", "-am", branch]);
        run(root, &["git", "switch", "-q", "main"]);
        let (job, _) = crate::jobs::add_job(
            s,
            "proj".into(),
            "experiment".into(),
            "Experiment".into(),
            "Suggestion: cached\nHypothesis: cache reuse\n".into(),
            1,
            0,
            source,
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        exec1(
            s,
            "UPDATE jobs SET status = 'done', branch = ?, finished_at = ? WHERE id = ?",
            &[&branch, &TEST_NOW, &job.str("id")],
        );
        exec1(
            s,
            "INSERT INTO runs (id, job_id, project, agent, started_at, status) VALUES (?, ?, 'proj', 'proposer', ?, 'done')",
            &[&format!("run-{}", job.str("id")), &job.str("id"), &TEST_NOW],
        );
        job_row(s, &job.str("id"))
    }

    #[test]
    fn experiment_reuses_cached_baseline_for_same_default_commit() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let (s, project, job, root, calls) = measured_keep_fixture();
        let seen = calls.clone();
        let _old =
            pulse_evolve::set_measure(Box::new(move |_ctx, _s, slug, _repo, r#ref, _opts| {
                *seen.lock().unwrap() += 1;
                let score = if r#ref == "main" { 0.8 } else { 0.7 };
                Ok(pulse_evolve::Scores {
                    project: slug.into(),
                    total: score,
                    ..Default::default()
                })
            }));
        let (state, note) = land_experiment(
            &s,
            project.clone(),
            job.clone(),
            "daemon".into(),
            TEST_NOW.into(),
            true,
        );
        assert_eq!(state, "dropped", "first discard: {note}");
        assert_eq!(*calls.lock().unwrap(), 2);
        let first_note: String = s
            .conn()
            .unwrap()
            .query_row(
                "SELECT note FROM evolution_archive WHERE project = 'proj' ORDER BY at DESC, id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            first_note.contains("baseline: measured"),
            "first archive note: {first_note}"
        );

        let second = finished_experiment_job(
            &s,
            &root,
            job.str("source"),
            "pulse/cache-second",
            "# cached variant\n",
        );
        let (state, note) = land_experiment(
            &s,
            project.clone(),
            second.clone(),
            "daemon".into(),
            TEST_NOW.into(),
            true,
        );
        assert_eq!(state, "dropped", "second discard: {note}");
        assert_eq!(
            *calls.lock().unwrap(),
            3,
            "second experiment should measure only the variant"
        );
        let second_note: String = s
            .conn()
            .unwrap()
            .query_row(
                "SELECT note FROM evolution_archive WHERE branch = 'pulse/cache-second'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            second_note.contains("baseline: reused"),
            "second archive note: {second_note}"
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM events WHERE action = 'evolution.baseline_cache'",
                &[],
            ),
            1
        );

        std::fs::write(std::path::Path::new(&root).join("new-main.txt"), "new\n").unwrap();
        run(&root, &["git", "add", "new-main.txt"]);
        run(&root, &["git", "commit", "-q", "-m", "move main"]);
        let third = finished_experiment_job(
            &s,
            &root,
            second.str("source"),
            "pulse/cache-third",
            "# cache miss variant\n",
        );
        let (state, note) =
            land_experiment(&s, project, third, "daemon".into(), TEST_NOW.into(), true);
        assert_eq!(state, "dropped", "third discard: {note}");
        assert_eq!(
            *calls.lock().unwrap(),
            5,
            "new default commit should miss the baseline cache"
        );
    }
}
