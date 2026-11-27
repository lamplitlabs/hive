//! The pull request reviewer: every finished branch becomes a pull request
//! on the project's upstream (`task_pr.rs` and the job flow emit `land.pr`),
//! and a review agent, not the daemon, decides it. The maintenance pass
//! queues one `review` job per open Hive pull request, the agent works it
//! with the `hive review` commands (`review_cli.rs`), and ends with exactly
//! one verdict: merge, ask for changes, or hand it to the owner.
//!
//! The complexity rule ([`reasons_for`]) is computed here, in code, before
//! the job is queued: a pull request the owner asked to see (`review.merge`
//! `ask`), one touching a guarded path, or one not from a Hive branch is
//! reviewed all the same, but the agent may only hand it to the owner with
//! its findings. The rule lives in code rather than in the brief so the
//! agent cannot talk itself past it, and so the hub can show the owner why
//! a pull request waits for them. Size is not a reason ([`care_for`]): a
//! pull request over `review.max_files` / `review.max_lines`, or one that
//! mostly deletes, is read file by file and merged by the reviewer when
//! the whole diff is what the title and body say. On 2026-11-04 the two
//! pull requests size alone handed to the owner (a blog post with its
//! data, 81 dictionary entries) carried the reviewer's own "fine to merge"
//! and then waited a day for a click.
//!
//! One review job per project at a time: a review clones the project and
//! runs its check, and two reviews of pull requests that may conflict with
//! each other waste that work, so the next pull request waits for the
//! verdict on the first. A review job that ends without a verdict is a
//! `review.failed`; the pass queues one more attempt for the same head and
//! after a second failure hands the pull request to the owner.

use serde_json::{Value, json};
use std::time::Duration;

use crate::util::Row;

/// The actor the daemon's own review events carry.
pub(crate) const ACTOR: &str = "daemon";
/// The job kind the reviewer queues (`hive_store::kinds`).
pub(crate) const KIND: &str = "review";
/// The source prefix of a review job: `review:<slug>:<number>`.
pub(crate) const SOURCE_PREFIX: &str = "review:";
/// The source prefix of the follow-up an `improve` job gets after a
/// `changes` verdict on a job or experiment pull request.
pub(crate) const FIX_SOURCE_PREFIX: &str = "pr-fix:";
/// Review attempts per pull request head before the owner is asked.
pub(crate) const MAX_ATTEMPTS: i64 = 2;
/// The reason a pull request goes to the owner after two failed reviews.
pub(crate) const FAILED_TWICE: &str = "the reviewer failed twice";
/// Minutes a review may take, written into the brief (no per-kind time
/// budget exists in the runner; `minutes_per_job` is global).
pub(crate) const REVIEW_MINUTES: i32 = 20;
/// A review is the owner's work ([`crate::jobs::OWNER_DRIVEN_SQL`]); the
/// number only orders it among the project's other owner-driven jobs.
const REVIEW_PRIORITY: i32 = 60;
const FOLLOW_UP_PRIORITY: i32 = 60;
const GH_TIMEOUT: Duration = Duration::from_secs(60);
const MERGE_TIMEOUT: Duration = Duration::from_secs(180);
/// The branch `hive review checkout` puts a pull request's head on.
pub(crate) fn review_branch(number: i64) -> String {
    format!("review/pr-{number}")
}

/// The events that end a pull request's review: once the newest of these
/// for a number is not `land.pr`, nothing is queued for it. `review.failed`
/// is not among them: a failed review is tried once more (`MAX_ATTEMPTS`).
/// `review.merge-wait` is: the reviewer said merge and the merge waits for
/// its checks or the forge ([`retry_merges`]), with no second review.
/// `review.again` (a waiting merge whose head moved on the forge) opens
/// the pull request for review again, as a `land.pr` does.
const STATE_ACTIONS: &str = "('land.pr', 'land.merged', 'land.pr-closed', 'review.merged', 'review.owner', 'review.changes', 'review.merge-wait', 'review.again')";

/// Whether the newest state event `action` leaves the pull request open
/// for review: a `land.pr`, or a `review.again`.
fn opens_review(action: &str) -> bool {
    action == "land.pr" || action == AGAIN_ACTION
}

/// An open Hive pull request as the newest `land.pr` for its number
/// describes it.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub(crate) struct OpenPr {
    pub number: i64,
    pub url: String,
    pub title: String,
    /// `task`, `job`, `experiment` or `proposed`; a `land.pr` without a
    /// kind and with a task is a task pull request (the first flow emitted
    /// no kind).
    pub kind: String,
    pub task: String,
    pub job: String,
    pub head: String,
    pub base: String,
    /// The pull request `base` is the head of, when the pull request stacks
    /// on one (`delivery.rs`); 0 when it targets the default branch.
    pub stack_on: i64,
    /// The head commit the pull request was last pushed at.
    pub sha: String,
    /// The event's sequence number: later verdicts are the ones after it.
    pub seq: i64,
}

impl OpenPr {
    fn from_event(seq: i64, detail: &Value) -> Self {
        let s = |k: &str| detail[k].as_str().unwrap_or_default().to_string();
        let task = s("task");
        let mut kind = s("kind");
        if kind.is_empty() {
            kind = if task.is_empty() { "job" } else { "task" }.to_string();
        }
        OpenPr {
            number: detail["number"].as_i64().unwrap_or(0),
            url: s("url"),
            title: s("title"),
            kind,
            task,
            job: s("job"),
            head: s("head"),
            base: s("base"),
            stack_on: detail["stack_on"].as_i64().unwrap_or(0),
            sha: s("to"),
            seq,
        }
    }
}

/// What a pull request stacks on, when it does not target the default
/// branch `def`: `#N` for the pull request below, or the base branch when
/// the `land.pr` did not name one. Such a pull request is reviewed and
/// merged after the one below, never before: merging it would merge it
/// into the branch below, not land it.
pub(crate) fn stacked_on(pr: &OpenPr, def: &str) -> Option<String> {
    if pr.base.is_empty() || pr.base == def {
        None
    } else if pr.stack_on != 0 {
        Some(format!("#{}", pr.stack_on))
    } else {
        Some(pr.base.clone())
    }
}

/// The open Hive pull requests of a project, oldest number first: every
/// number whose newest state event is a `land.pr`.
pub(crate) fn open_prs(q: &(impl crate::util::Querier + ?Sized), slug: &str) -> Vec<OpenPr> {
    pr_states(q, slug)
        .into_iter()
        .filter(|(action, _)| opens_review(action))
        .map(|(_, pr)| pr)
        .collect()
}

/// Every Hive pull request of a project with a `land.pr`, oldest number
/// first, with the newest state event that stands for it.
fn pr_states(q: &(impl crate::util::Querier + ?Sized), slug: &str) -> Vec<(String, OpenPr)> {
    read_pr_states(q, slug).unwrap_or_default()
}

#[cfg(test)]
thread_local! {
    /// Tests: every read through `read_pr_states` fails while set and the
    /// store's other reads go on, as when one read fails and the next one
    /// succeeds.
    static PR_STATES_UNREAD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// [`pr_states`], or why the store could not be read.
fn read_pr_states(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
) -> Result<Vec<(String, OpenPr)>, String> {
    #[cfg(test)]
    if PR_STATES_UNREAD.get() {
        return Err(format!(
            "the pull requests of {slug} could not be read: disk I/O error"
        ));
    }
    let rows = crate::util::query_rows(
        q,
        &format!(
            "SELECT seq, action, detail FROM events WHERE subject = ? AND action IN {STATE_ACTIONS} ORDER BY seq"
        ),
        &[&slug],
    )
    .map_err(|e| format!("the pull requests of {slug} could not be read: {e}"))?;
    let mut latest: std::collections::BTreeMap<i64, (String, Option<OpenPr>)> = Default::default();
    for r in rows {
        let detail: Value = serde_json::from_str(&r.str("detail")).unwrap_or(Value::Null);
        let Some(number) = detail["number"].as_i64() else {
            continue;
        };
        let action = r.str("action");
        let entry = latest.entry(number).or_insert((String::new(), None));
        entry.1 = opened_by(entry.1.take(), &action, r.int("seq") as i64, &detail);
        entry.0 = action;
    }
    Ok(latest
        .into_values()
        .filter_map(|(action, pr)| Some((action, pr?)))
        .collect())
}

/// The Hive pull requests of a project that are still open on the forge,
/// oldest number first: every number whose newest state event neither
/// merged nor closed it. Unlike [`open_prs`] (the ones open for review),
/// a pull request under a "changes" verdict, a waiting merge or a hand-off
/// to the owner is here too: its branch and its base are still the
/// forge's. `Err` when the store could not be read: no answer is not "none
/// open", and a merge that took it so would delete a branch another pull
/// request is based on.
pub(crate) fn forge_open_prs(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
) -> Result<Vec<OpenPr>, String> {
    Ok(read_pr_states(q, slug)?
        .into_iter()
        .filter(|(action, _)| {
            !matches!(
                action.as_str(),
                "land.merged" | "land.pr-closed" | "review.merged"
            )
        })
        .map(|(_, pr)| pr)
        .collect())
}

/// The pull request as its state events so far describe it: a `land.pr`
/// names it afresh; a `review.again` keeps it and moves its head to the
/// one the forge held when the waiting merge found it moved (the head the
/// next review reads, and the one its attempts are counted at).
fn opened_by(pr: Option<OpenPr>, action: &str, seq: i64, detail: &Value) -> Option<OpenPr> {
    match action {
        "land.pr" => Some(OpenPr::from_event(seq, detail)),
        AGAIN_ACTION => pr.map(|mut p| {
            p.sha = detail["head_sha"].as_str().unwrap_or_default().to_string();
            p.seq = seq;
            p
        }),
        _ => pr,
    }
}

/// The newest `land.pr` of one number, open or not, with the action that
/// currently stands for it (`land.pr` when open).
pub(crate) fn pr_state(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    number: i64,
) -> Option<(OpenPr, String)> {
    let rows = crate::util::query_rows(
        q,
        &format!(
            "SELECT seq, action, detail FROM events WHERE subject = ? AND action IN {STATE_ACTIONS}
             AND json_extract(detail, '$.number') = ? ORDER BY seq"
        ),
        &[&slug, &number],
    )
    .unwrap_or_default();
    let last = rows.last()?;
    let mut pr = None;
    for r in &rows {
        let detail: Value = serde_json::from_str(&r.str("detail")).unwrap_or(Value::Null);
        pr = opened_by(pr, &r.str("action"), r.int("seq") as i64, &detail);
    }
    let action = last.str("action");
    // `review.again` stands for an open pull request, as `land.pr` does.
    let action = if action == AGAIN_ACTION {
        "land.pr".to_string()
    } else {
        action
    };
    Some((pr?, action))
}

/// What `gh pr view` says about a pull request: the fields the complexity
/// rule and the review brief read.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub(crate) struct PrView {
    pub number: i64,
    pub title: String,
    pub body: String,
    pub url: String,
    /// `OPEN`, `MERGED` or `CLOSED`.
    pub state: String,
    pub head: String,
    pub head_sha: String,
    pub base: String,
    pub author: String,
    pub additions: i64,
    pub deletions: i64,
    /// (path, additions, deletions) per changed file.
    pub files: Vec<(String, i64, i64)>,
    /// A draft (`isDraft`): the forge will not merge it.
    pub is_draft: bool,
    /// `mergeStateStatus`: `CLEAN`, `DIRTY` (conflicts), `BLOCKED` (a
    /// required check or review is missing), `BEHIND`, `UNSTABLE`,
    /// `HAS_HOOKS` or `UNKNOWN`; empty from an older gh.
    pub merge_state: String,
    /// How many checks the head reports (`statusCheckRollup` entries).
    pub checks: usize,
    /// The checks that failed, were cancelled, timed out or want action.
    pub failing_checks: Vec<String>,
    /// The checks still queued or running.
    pub pending_checks: Vec<String>,
}

impl PrView {
    pub(crate) fn from_json(v: &Value) -> Self {
        let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();
        let forge = crate::landing_pr::pr_state(v);
        PrView {
            number: v["number"].as_i64().unwrap_or(0),
            title: s("title"),
            body: s("body"),
            url: s("url"),
            state: s("state"),
            head: s("headRefName"),
            head_sha: s("headRefOid"),
            base: s("baseRefName"),
            author: v["author"]["login"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            additions: v["additions"].as_i64().unwrap_or(0),
            deletions: v["deletions"].as_i64().unwrap_or(0),
            files: v["files"]
                .as_array()
                .map(|fs| {
                    fs.iter()
                        .map(|f| {
                            (
                                f["path"].as_str().unwrap_or_default().to_string(),
                                f["additions"].as_i64().unwrap_or(0),
                                f["deletions"].as_i64().unwrap_or(0),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
            is_draft: forge.is_draft,
            merge_state: forge.merge_state,
            checks: v["statusCheckRollup"].as_array().map(Vec::len).unwrap_or(0),
            failing_checks: forge.failing_checks,
            pending_checks: forge.pending_checks,
        }
    }
}

/// The repository `gh --repo` wants for a project: the one the pull
/// request's url names (`OWNER/REPO`, or `HOST/OWNER/REPO` off github.com),
/// else the one the default branch's upstream points at, else the project's
/// recorded remote.
pub(crate) fn repo_of(project: &Row, pr_url: &str) -> String {
    if let Some(repo) = repo_from_pr_url(pr_url) {
        return repo;
    }
    let root = project.str("path");
    let def = crate::baseline::default_branch(project.clone());
    if !root.is_empty()
        && let Some((remote, _, _)) = crate::forge::tracking_of(&root, &def)
        && let Some(repo) = crate::landing_pr::repo_slug(&crate::forge::push_url(&root, &remote))
    {
        return repo;
    }
    crate::landing_pr::repo_slug(&project.str("remote")).unwrap_or_default()
}

/// `https://github.com/o/r/pull/7` -> `o/r`; another host keeps its name
/// in front, as `gh --repo` reads it.
pub(crate) fn repo_from_pr_url(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let mut segs = rest.split('/');
    let host = segs.next()?;
    let owner = segs.next()?;
    let repo = segs.next()?;
    if host.is_empty() || owner.is_empty() || repo.is_empty() || segs.next() != Some("pull") {
        return None;
    }
    Some(if host == "github.com" {
        format!("{owner}/{repo}")
    } else {
        format!("{host}/{owner}/{repo}")
    })
}

/// Whether gh's error says the account it ran as cannot see the repository
/// (the same reading as `pipeline.rs`): 404 for a repository the account
/// has no access to, 403 for one it may not write, 401 for a token that
/// is no longer good, and an account whose token cannot be read at all.
fn account_failed(err: &str) -> bool {
    crate::pipeline::account_failed(err)
}

fn gh_meta_key(slug: &str) -> String {
    format!("review-gh:{slug}")
}

/// Runs `gh <args> --repo <repo>` as the account that can see the
/// repository. The account that last worked for the project goes first
/// (the active login when none is remembered); when it cannot see the
/// repository every other logged-in account is tried once and the one that
/// answers is remembered for the project. gh runs in the store's home, not
/// in a checkout, so `pr merge --delete-branch` never touches a local
/// branch of the owner's checkout. Returns stdout, or the failure in words
/// (never the environment: a token is never put in words).
pub(crate) fn gh(
    s: &hive_store::Store,
    slug: &str,
    repo: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<String, String> {
    let mut argv: Vec<String> = std::iter::once("gh".to_string())
        .chain(args.iter().map(|a| a.to_string()))
        .collect();
    if !repo.is_empty() {
        argv.push("--repo".into());
        argv.push(repo.to_string());
    }
    let what = format!(
        "gh {}",
        args.iter().take(2).copied().collect::<Vec<_>>().join(" ")
    );
    let remembered = s.meta(&gh_meta_key(slug)).map(|m| m.0).unwrap_or_default();
    let run_env = |env: Vec<String>| -> Result<String, String> {
        let out = crate::proc::run_group(
            &hive_bus::CancellationToken::new(),
            &argv,
            &s.home,
            &env,
            &[],
            timeout,
        )
        .map_err(|e| format!("{what} could not start: {e}"))?;
        if out.timeout {
            return Err(format!(
                "{what} did not finish within {}s",
                timeout.as_secs()
            ));
        }
        if out.exit_code != 0 {
            return Err(format!(
                "{what} failed: {}",
                crate::run::squeeze(&format!("{} {}", out.stderr, out.stdout), 300)
            ));
        }
        Ok(out.stdout)
    };
    let run_as = |user: &str| -> Result<String, String> {
        let env = if user.is_empty() {
            crate::proc::environ()
        } else {
            crate::gh_auth::env_as(user)?
        };
        run_env(env)
    };
    // The GitHub App's token for this repository first (ADR-070 step 3);
    // a call it cannot make falls back to the machine's logins.
    if let Some(Ok(env)) = crate::gitapp::gh_env(s, repo) {
        match run_env(env) {
            Ok(out) => return Ok(out),
            Err(e) if !account_failed(&e) => return Err(e),
            Err(_) => {}
        }
    }
    let first = match run_as(&remembered) {
        Ok(out) => return Ok(out),
        Err(e) if !account_failed(&e) => return Err(e),
        Err(e) => e,
    };
    let logins = crate::gh_auth::logins();
    let mut candidates: Vec<String> = Vec::new();
    if !remembered.is_empty() && remembered != logins.active {
        candidates.push(String::new());
    }
    candidates.extend(logins.others(&remembered));
    for user in candidates {
        match run_as(&user) {
            Ok(out) => {
                let _ = s.set_meta(&gh_meta_key(slug), &user);
                return Ok(out);
            }
            Err(e) if account_failed(&e) => continue,
            Err(e) => return Err(e),
        }
    }
    Err(first)
}

/// `gh pr view` of one pull request: the fields the rule and brief read,
/// and what the forge says stands between it and its merge.
pub(crate) fn pr_view(
    s: &hive_store::Store,
    slug: &str,
    repo: &str,
    number: i64,
) -> Result<PrView, String> {
    let out = gh(
        s,
        slug,
        repo,
        &[
            "pr",
            "view",
            &number.to_string(),
            "--json",
            "number,title,body,url,state,headRefName,headRefOid,baseRefName,author,additions,deletions,files,isDraft,mergeStateStatus,statusCheckRollup",
        ],
        GH_TIMEOUT,
    )?;
    let v: Value = serde_json::from_str(out.trim())
        .map_err(|e| format!("gh pr view {number} answered no JSON: {e}"))?;
    Ok(PrView::from_json(&v))
}

/// `gh pr diff` of one pull request.
pub(crate) fn pr_diff(
    s: &hive_store::Store,
    slug: &str,
    repo: &str,
    number: i64,
) -> Result<String, String> {
    gh(
        s,
        slug,
        repo,
        &["pr", "diff", &number.to_string()],
        GH_TIMEOUT,
    )
}

/// Paths whose size says nothing about the review: build output and
/// lockfiles are regenerated, not written, so they do not count toward
/// `review.max_lines` (a hub rebuild alone is thousands of lines).
pub(crate) fn generated_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.ends_with(".lock")
        || name == "Cargo.lock"
        || name == "package-lock.json"
        || name.contains(".min.")
        || path
            .split('/')
            .any(|seg| matches!(seg, "dist" | "out" | "target"))
}

/// Paths a review agent may never merge a change to: the landing's refused
/// paths (`land.rs`: the constitution, the evaluators, STOP, the workflows,
/// and the owner's paths for a `hive/` branch), plus environment files,
/// anything named like a secret or a credential, and the CI configuration
/// of the other forges.
pub(crate) fn guarded_paths(files: &[String], head: &str) -> Vec<String> {
    let mut out = crate::land::refused_paths(files.to_vec(), head.to_string());
    for f in files {
        let name = f.rsplit('/').next().unwrap_or(f);
        let low = name.to_ascii_lowercase();
        let env_file =
            low.starts_with(".env") && !low.ends_with(".example") && !low.ends_with(".sample");
        let secret = [
            "secret",
            "credential",
            "password",
            ".pem",
            "id_rsa",
            "id_ed25519",
        ]
        .iter()
        .any(|w| low.contains(w));
        let ci = matches!(
            name,
            ".gitlab-ci.yml"
                | ".travis.yml"
                | "azure-pipelines.yml"
                | "Jenkinsfile"
                | "bitbucket-pipelines.yml"
        ) || f.starts_with(".circleci/");
        if (env_file || secret || ci) && !out.contains(f) {
            out.push(f.clone());
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Why the review agent may not merge this pull request itself; empty when
/// it may. Computed from `gh pr view` before the job is queued and again
/// by every verdict command, so the brief and the command agree.
pub(crate) fn reasons_for(s: &hive_store::Store, pr: &PrView) -> Vec<String> {
    let mut reasons = Vec::new();
    if s.setting("review.merge", "auto").trim() == "ask" {
        reasons.push("the owner asked to merge every pull request".to_string());
    }
    let paths: Vec<String> = pr.files.iter().map(|(p, _, _)| p.clone()).collect();
    for p in guarded_paths(&paths, &pr.head) {
        reasons.push(format!("touches {p}"));
    }
    // A change that has a face shows it before and after (`hive report`,
    // `landing.ui_report = require`); without that page it is the owner's.
    if let Some(why) = crate::ui_evidence::pr_report_reason(s, &pr.head, &paths) {
        reasons.push(why);
    }
    // Hive's own heads are `hive/...` (jobs, tasks, conversations; `pulse/...`
    // on a pull request opened before the rename) and `evolve/<id>` (a kept
    // experiment, already measured against its card); anything else was
    // pushed by a person or another tool and gets the owner.
    if !hive_head(&pr.head) {
        reasons.push(format!(
            "opened by a human/other tool (branch {})",
            if pr.head.is_empty() {
                "unknown"
            } else {
                &pr.head
            }
        ));
    }
    // A task that builds a Tier 2 decision record the owner has not
    // accepted is the owner's to merge: PR 280 built ADR-073 while it was
    // Proposed and the reviewer merged it by itself (TASK-147).
    reasons.extend(crate::record_gate::reasons(s, pr));
    reasons
}

/// What the reviewer must read with extra care: the shape of the change,
/// not a bar to merging it. More than `review.max_files` files or
/// `review.max_lines` lines (generated paths left out), or a change that
/// mostly deletes, is still the reviewer's to merge once every file of it
/// is explained by the title and body; what it cannot account for goes to
/// the owner through the verdict, as for any other pull request.
pub(crate) fn care_for(s: &hive_store::Store, pr: &PrView) -> Vec<String> {
    let mut care = Vec::new();
    let max_files = crate::util::setting_int(s, "review.max_files").max(1) as i64;
    let max_lines = crate::util::setting_int(s, "review.max_lines").max(1) as i64;
    let counted: Vec<&(String, i64, i64)> = pr
        .files
        .iter()
        .filter(|(p, _, _)| !generated_path(p))
        .collect();
    let files = counted.len() as i64;
    let added: i64 = counted.iter().map(|(_, a, _)| a).sum();
    let deleted: i64 = counted.iter().map(|(_, _, d)| d).sum();
    let lines = added + deleted;
    if files > max_files || lines > max_lines {
        care.push(format!(
            "{files} files / {lines} lines changed (the usual reading is {max_files} files / {max_lines} lines)"
        ));
    }
    if deleted > 200 && deleted > 2 * added {
        care.push(format!(
            "removes more than it adds ({deleted} lines removed, {added} added)"
        ));
    }
    care
}

/// True for a branch Hive itself pushes: `hive/<agent>/<job>`,
/// `hive/task-<id>`, `hive/chat-<id>` or an experiment's `evolve/<id>`. A
/// pull request opened before the rename has its head under `pulse/`; it
/// was Hive's then and is now, and calling it a person's would hand every
/// one of them to the owner.
pub(crate) fn hive_head(head: &str) -> bool {
    hive_store::agent_branch(head) || head.starts_with("evolve/")
}

/// The project's check command when the store knows one.
fn verify_cmd_of(s: &hive_store::Store, slug: &str) -> String {
    s.project(slug)
        .ok()
        .flatten()
        .map(|p| p.verify_cmd.trim().to_string())
        .unwrap_or_default()
}

/// The review job's brief: what the pull request is, whether it may be
/// merged by the agent, and the exact procedure ending in one verdict.
pub(crate) fn review_brief(
    slug: &str,
    pr: &OpenPr,
    view: &PrView,
    reasons: &[String],
    care: &[String],
    verify_cmd: &str,
) -> String {
    let n = pr.number;
    let title = if view.title.is_empty() {
        &pr.title
    } else {
        &view.title
    };
    let mut b = Vec::new();
    b.push(format!("Review pull request #{n} of {slug}: {title}"));
    b.push(pr.url.clone());
    let what = match pr.kind.as_str() {
        "task" => format!("the work of task {}", pr.task),
        "job" => format!("the branch of job {}", pr.job),
        "experiment" => "an experiment's branch".to_string(),
        "proposed" => "a branch proposed with hive land --propose".to_string(),
        other => format!("a {other} branch"),
    };
    b.push(format!(
        "This is a Hive pull request ({what}): head `{}` into `{}`.",
        view.head, view.base
    ));
    b.push(String::new());
    if reasons.is_empty() {
        b.push(
            "You may merge it yourself when it is correct, safe, in scope and its check is green."
                .into(),
        );
        if !care.is_empty() {
            b.push(format!(
                "Read it with extra care: {}. Size is not a reason to hand it over: go file by file, and merge only when every file is what the title and body say and the check is green; a file the body does not account for, or anything you cannot judge, goes to the owner with `hive review owner {n} --note`.",
                care.join("; ")
            ));
        }
    } else {
        b.push("You may NOT merge this pull request. Hand it to the owner because:".into());
        for r in reasons {
            b.push(format!("- {r}"));
        }
        b.push(format!(
            "Review it all the same and end with `hive review owner {n} --note \"<your findings>\"`: the note is what the owner reads before deciding."
        ));
    }
    b.push(String::new());
    if verify_cmd.is_empty() {
        b.push(format!(
            "The project's check: not recorded; `hive env show --project {slug}` or the README names it."
        ));
    } else {
        b.push(format!("The project's check: `{verify_cmd}`"));
    }
    b.push(String::new());
    b.push("Procedure, in this order, in this clone:".into());
    b.push(format!(
        "1. `hive review show {n}` - the title, body, changed files and the reasons above."
    ));
    b.push(format!(
        "2. `hive review checkout {n}` - puts this clone on `{}`, the pull request's head. When it cannot, it records this review's verdict itself (`review.again`: a fresh review is queued, the owner is not asked); finish the job with what it said and give no other verdict.",
        review_branch(n)
    ));
    b.push(format!(
        "3. `hive review check {n}` - runs the project's check here, on the pull request's head, and records the head, the exit and the log. Do not run the check by hand instead: `hive review merge {n}` refuses without a green check of this head recorded for this review. A red check is never merged; when its failing tests pass re-run alone they are flaky, and the command queues their fix as your verdict."
    ));
    b.push(format!(
        "4. `hive review diff {n}` and read the whole diff as a reviewer: does it do what the title and body say, is it correct, is it tested, does it touch anything the body does not explain (secrets, deletions, paths outside its scope), does it break a convention the README or an ADR states."
    ));
    b.push("5. Exactly ONE verdict command:".into());
    if reasons.is_empty() {
        b.push(format!(
            "   - `hive review merge {n}` - correct, safe, in scope, check green: it merges on GitHub and records review.merged. The command reads the pull request on GitHub first and refuses, without a verdict, a head with no green `hive review check {n}` recorded (it names the check and its log), a head that moved since this review was queued, a draft, a failing or still running check, or a merge GitHub blocks; a refusal is not a failure, so read the reason and finish the job with it. Checks still running, or GitHub refusing the merge with a 5xx or an internal error, keep your verdict: the command says the merge waits, and the daemon merges once the checks pass or GitHub answers - finish the job, do not review again."
        ));
        b.push(format!(
            "   - `hive review changes {n} --note \"<the exact fixes>\"` - fixable: GitHub gets the request, and one follow-up job makes the fixes on the pull request's head; your note is its brief, so name files and what to change."
        ));
        b.push(format!(
            "   - `hive review owner {n} --note \"<your findings>\"` - a real decision (money, auth, data, dependencies, deploy, a surface no ADR covers), a conflict with the base, or anything you are not sure of."
        ));
    } else {
        b.push(format!(
            "   - `hive review owner {n} --note \"<your findings>\"` - the only verdict allowed for this pull request; `hive review merge {n}` refuses it."
        ));
    }
    b.push(format!(
        "6. `hive job finish $HIVE_JOB done --result 'SUMMARY: reviewed #{n}: <verdict> - <one line>'` (no `--branch`: a review leaves no branch)."
    ));
    b.push(String::new());
    b.push(format!(
        "Do not fix the code yourself: a fix is a `changes` note, and the follow-up job does it. Do not call gh or git push yourself: `hive review` talks to GitHub for you. Never merge a red check or a diff you did not read. A review takes at most {REVIEW_MINUTES} minutes; a job that ends without a verdict counts as a failed review."
    ));
    b.join("\n")
}

fn source_of(slug: &str, number: i64) -> String {
    format!("{SOURCE_PREFIX}{slug}:{number}")
}

/// Whether a job is a pull request review. A review's clone holds the
/// pull request's own commits once the agent runs `hive review checkout`,
/// so the run that carries it lands nothing: its outcome is the verdict.
pub(crate) fn is_review_job(job: &Row) -> bool {
    job.str("kind") == "review" && job.str("source").starts_with(SOURCE_PREFIX)
}

/// A queued or running review job of the project, when there is one.
fn active_review_job(q: &(impl crate::util::Querier + ?Sized), slug: &str) -> Option<Row> {
    crate::util::query_rows(
        q,
        "SELECT * FROM jobs WHERE project = ? AND kind = 'review' AND status IN ('queued', 'leased') ORDER BY created_at LIMIT 1",
        &[&slug],
    )
    .unwrap_or_default()
    .into_iter()
    .next()
}

/// The newest review job queued for one pull request, with the sequence
/// number of its `review.queued`: the event log orders attempts, where
/// `created_at` (to the second) cannot tell two apart.
pub(crate) fn newest_review_job(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    number: i64,
) -> Option<(Row, i64)> {
    let queued = crate::util::query_rows(
        q,
        "SELECT seq, json_extract(detail, '$.job') AS job FROM events WHERE subject = ? AND action = 'review.queued' AND json_extract(detail, '$.number') = ? ORDER BY seq DESC LIMIT 1",
        &[&slug, &number],
    )
    .unwrap_or_default()
    .into_iter()
    .next()?;
    let job = crate::util::query_rows(
        q,
        "SELECT * FROM jobs WHERE id = ? AND kind = 'review'",
        &[&queued.str("job")],
    )
    .unwrap_or_default()
    .into_iter()
    .next()?;
    Some((job, queued.int("seq") as i64))
}

/// The events of one number after a sequence number, actions only.
fn actions_after(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    number: i64,
    seq: i64,
    actions: &str,
) -> Vec<String> {
    crate::util::query_rows(
        q,
        &format!(
            "SELECT action FROM events WHERE subject = ? AND seq > ? AND action IN {actions} AND json_extract(detail, '$.number') = ? ORDER BY seq"
        ),
        &[&slug, &seq, &number],
    )
    .unwrap_or_default()
    .iter()
    .map(|r| r.str("action"))
    .collect()
}

/// How many review jobs were queued for the pull request at its current
/// head (`review.queued` events carrying that sha; those without a sha
/// count when the head is unknown). A review whose job could not check the
/// head out ended with its own `review.again` (`checkout`): that is no
/// failed reading of the pull request, and it never counts towards the
/// hand-over to the owner.
fn attempts_at(q: &(impl crate::util::Querier + ?Sized), slug: &str, pr: &OpenPr) -> i64 {
    let rows = crate::util::query_rows(
        q,
        "SELECT COALESCE(json_extract(detail, '$.sha'), '') AS sha, COALESCE(json_extract(detail, '$.job'), '') AS job FROM events WHERE subject = ? AND action = 'review.queued' AND json_extract(detail, '$.number') = ?",
        &[&slug, &pr.number],
    )
    .unwrap_or_default();
    let requeued: std::collections::BTreeSet<String> = crate::util::query_rows(
        q,
        "SELECT COALESCE(json_extract(detail, '$.job'), '') AS job FROM events WHERE subject = ? AND action = 'review.again' AND json_extract(detail, '$.number') = ? AND json_extract(detail, '$.checkout') = 1",
        &[&slug, &pr.number],
    )
    .unwrap_or_default()
    .iter()
    .map(|r| r.str("job"))
    .filter(|j| !j.is_empty())
    .collect();
    rows.iter()
        .filter(|r| pr.sha.is_empty() || r.str("sha").is_empty() || r.str("sha") == pr.sha)
        .filter(|r| !requeued.contains(&r.str("job")))
        .count() as i64
}

/// The review job running this command (`HIVE_JOB`) when it is the one
/// queued for pull request `number` of `slug`.
pub(crate) fn running_review_job(s: &hive_store::Store, slug: &str, number: i64) -> Option<Row> {
    let id = std::env::var("HIVE_JOB").unwrap_or_default();
    if id.trim().is_empty() {
        return None;
    }
    let conn = s.conn().ok()?;
    let job = crate::jobs::job_row(&conn, id.trim().to_string());
    (is_review_job(&job) && job.str("source") == source_of(slug, number)).then_some(job)
}

/// A review job that could not put the pull request's head in its clone
/// (`hive review checkout`) ends with this verdict of its own:
/// `review.again` with `checkout: true`, the job and the error. The pull
/// request is open for review again at the same head, the next pass queues
/// a fresh review, and the attempt is not counted as a failed review
/// ([`attempts_at`]): a clone that cannot fetch is the harness's trouble,
/// never a reason to hand the pull request to the owner. Only the pull
/// request's own review job (`HIVE_JOB`) records it, and only while the
/// pull request is open for review; returns whether it did.
pub(crate) fn requeue_after_checkout(
    s: &hive_store::Store,
    slug: &str,
    view: &PrView,
    error: &str,
    actor: &str,
    now: &str,
) -> bool {
    let Some(job) = running_review_job(s, slug, view.number) else {
        return false;
    };
    let Some((pr, action)) = s.conn().ok().and_then(|q| pr_state(&q, slug, view.number)) else {
        return false;
    };
    if action != "land.pr" {
        return false;
    }
    let head = if view.head_sha.is_empty() {
        pr.sha.clone()
    } else {
        view.head_sha.clone()
    };
    event(
        s,
        now,
        actor,
        AGAIN_ACTION,
        slug,
        json!({"project": slug, "number": pr.number, "url": pr.url, "head_sha": head,
            "was": pr.sha, "checkout": true, "job": job.str("id"),
            "note": format!("the review job could not check out the pull request's head: {}", hive_cred::redact(error, &[]))}),
    );
    true
}

/// Whether the newest state event of pull request `number` is a
/// `review.again` the review job `job` recorded because it could not check
/// the head out: that job has given its verdict.
pub(crate) fn requeued_by(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    number: i64,
    job: &str,
) -> bool {
    crate::util::query_rows(
        q,
        &format!(
            "SELECT action, detail FROM events WHERE subject = ? AND action IN {STATE_ACTIONS} AND json_extract(detail, '$.number') = ? ORDER BY seq DESC LIMIT 1"
        ),
        &[&slug, &number],
    )
    .unwrap_or_default()
    .first()
    .is_some_and(|r| {
        let d: Value = serde_json::from_str(&r.str("detail")).unwrap_or(Value::Null);
        r.str("action") == AGAIN_ACTION
            && d["checkout"].as_bool() == Some(true)
            && d["job"].as_str() == Some(job)
    })
}

fn event(s: &hive_store::Store, now: &str, actor: &str, action: &str, slug: &str, detail: Value) {
    if let Ok(conn) = s.conn() {
        crate::util::event_at(&conn, now, actor, action, slug, detail);
    }
}

/// Queues the review job of one pull request and records `review.queued`.
pub(crate) fn queue_review(
    s: &hive_store::Store,
    project: &Row,
    pr: &OpenPr,
    view: &PrView,
    reasons: &[String],
    now: &str,
) -> Result<Row, String> {
    let slug = project.str("slug");
    let title = if view.title.is_empty() {
        &pr.title
    } else {
        &view.title
    };
    let brief = review_brief(
        &slug,
        pr,
        view,
        reasons,
        &care_for(s, view),
        &verify_cmd_of(s, &slug),
    );
    let (job, _) = crate::jobs::add_job(
        s,
        slug.clone(),
        KIND.into(),
        format!("Review PR #{}: {title}", pr.number),
        brief,
        0,
        REVIEW_PRIORITY,
        source_of(&slug, pr.number),
        ACTOR.into(),
        true,
    )
    .map_err(|e| format!("could not queue the review job: {e}"))?;
    let mut detail = json!({
        "project": slug, "number": pr.number, "url": pr.url, "job": job.str("id"),
        "kind": pr.kind, "reasons": reasons, "sha": view.head_sha,
    });
    if !pr.task.is_empty() {
        detail["task"] = json!(pr.task);
    }
    event(s, now, ACTOR, "review.queued", &slug, detail);
    Ok(job)
}

/// The maintenance pass over the projects' pull requests: honours the
/// owner's Merge clicks (`review.ask`), merges the approved pull requests
/// whose merge waited (`review.merge-wait`), records review jobs that
/// ended without a verdict, and queues one review job per project.
pub(crate) fn tend_reviews(s: &hive_store::Store, projects: &[Row], now: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for p in projects {
        if p.str("path").is_empty() {
            continue;
        }
        lines.extend(answer_lapses(s, p, now));
        lines.extend(honour_asks(s, p, now));
        lines.extend(retry_merges(s, p, now));
        lines.extend(tend_project(s, p, now));
    }
    lines
}

/// The owner clicked Merge in the hub: the newest `review.ask` of a number
/// with no `review.merged` or `review.failed` after it is merged as the
/// owner, whatever the complexity rule says - at the head Hive held when
/// it was pressed: an ask a newer head outran ([`ask_outrun`]) lapses.
fn honour_asks(s: &hive_store::Store, project: &Row, now: &str) -> Vec<String> {
    let slug = project.str("slug");
    let Ok(conn) = s.conn() else {
        return Vec::new();
    };
    let asks = crate::util::query_rows(
        &conn,
        "SELECT seq, detail FROM events WHERE subject = ? AND action = 'review.ask' ORDER BY seq",
        &[&slug],
    )
    .unwrap_or_default();
    // Per number, the newest ask: its seq and the head it was pressed for.
    let mut newest: std::collections::BTreeMap<i64, (i64, String)> = Default::default();
    for a in asks {
        let detail: Value = serde_json::from_str(&a.str("detail")).unwrap_or(Value::Null);
        if detail["action"].as_str().unwrap_or("merge") != "merge" {
            continue;
        }
        if let Some(n) = detail["number"].as_i64() {
            let sha = detail["sha"].as_str().unwrap_or_default().to_string();
            newest.insert(n, (a.int("seq") as i64, sha));
        }
    }
    let mut lines = Vec::new();
    for (number, (seq, asked)) in newest {
        if !actions_after(
            &conn,
            &slug,
            number,
            seq,
            "('review.merged', 'review.failed')",
        )
        .is_empty()
        {
            continue;
        }
        // A click on a pull request somebody merged or closed on GitHub in
        // the meantime (the landing poll recorded it) asks for nothing.
        if matches!(
            pr_state(&conn, &slug, number).map(|(_, a)| a).as_deref(),
            Some("land.merged" | "land.pr-closed")
        ) {
            continue;
        }
        // The click was for the head the hub showed then. A newer head
        // Hive recorded since (pushed, its base merged in, found moved
        // while a merge waited, queued for review) is reviewed on its own;
        // the click does not merge it, even when its checks are green
        // before its verdict. The owner is told once, where they pressed.
        // A lapse is for good: an ask once told as lapsed is never honoured
        // again, even when the head comes back to the commit it was pressed
        // for (a force-push back, a review queued at a stale commit). The
        // owner's answer to the lapse ([`answer_lapses`]) or a new press is
        // a new ask.
        if ask_lapsed(s, &conn, &slug, number, seq) {
            continue;
        }
        if let Some(outrun) = ask_outrun(&conn, &slug, number, seq, &asked) {
            let _ = s.set_meta(&lapsed_key(&slug, number), &seq.to_string());
            let told = tell_lapsed(s, &conn, &slug, number, seq, &outrun, now);
            lines.push(format!(
                "review {slug}: #{number} not merged for the owner: Merge was pressed at {}, the pull request is at {} now ({}); it is reviewed again{told}",
                crate::recipe::short(&outrun.was, 12),
                crate::recipe::short(&outrun.head, 12),
                outrun.why,
            ));
            continue;
        }
        match merge_as_owner(s, project, number, &asked, now) {
            Ok(url) => lines.push(format!(
                "review {slug}: #{number} merged for the owner {url}"
            )),
            Err(e) => lines.push(format!(
                "review {slug}: #{number} could not be merged for the owner: {e}"
            )),
        }
    }
    lines
}

/// The meta key that held the seq of the newest lapsed ask of a number
/// before `review.ask-lapsed` was recorded; still read for those.
fn lapsed_key(slug: &str, number: i64) -> String {
    format!("review-ask-lapsed:{slug}:{number}")
}

/// The event that says an owner's Merge lapsed: `ask` is the seq of the
/// `review.ask`, `was` the commit it was pressed for, `head` the one Hive
/// holds now, `post` the owner's question with the two answers.
pub(crate) const LAPSED_ACTION: &str = "review.ask-lapsed";

/// Whether the `review.ask` at `seq` lapsed already: a `review.ask-lapsed`
/// names it, or (before that event) the lapse mark does.
fn ask_lapsed(
    s: &hive_store::Store,
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    number: i64,
    seq: i64,
) -> bool {
    if s.meta(&lapsed_key(slug, number))
        .map(|m| m.0)
        .unwrap_or_default()
        == seq.to_string()
    {
        return true;
    }
    !crate::util::query_rows(
        q,
        "SELECT 1 AS one FROM events WHERE subject = ? AND action = 'review.ask-lapsed'
         AND json_extract(detail, '$.number') = ? AND json_extract(detail, '$.ask') = ? LIMIT 1",
        &[&slug, &number, &seq],
    )
    .unwrap_or_default()
    .is_empty()
}

/// The two answers of a lapse, as the owner picks them.
pub(crate) const LAPSE_MERGE: &str = "Merge this commit";
pub(crate) const LAPSE_WAIT: &str = "Wait for the review of the new commit";

/// The owner answered a lapse ([`tell_lapsed`]'s question): "Merge this
/// commit" asks for the merge of the commit the question named - a new
/// `review.ask` pinned to it, honoured on this pass like a press of Merge,
/// and lapsing in turn if the head moved again; "Wait for the review of the
/// new commit" asks nothing, the review goes on. Each answer is acted on
/// once.
fn answer_lapses(s: &hive_store::Store, project: &Row, now: &str) -> Vec<String> {
    let slug = project.str("slug");
    let Ok(conn) = s.conn() else {
        return Vec::new();
    };
    let rows = crate::util::query_rows(
        &conn,
        "SELECT e.detail, d.answer, d.decided_by FROM events e JOIN decisions d ON d.post_id = json_extract(e.detail, '$.post')
         WHERE e.subject = ? AND e.action = 'review.ask-lapsed' ORDER BY e.seq",
        &[&slug],
    )
    .unwrap_or_default();
    let mut lines = Vec::new();
    for r in rows {
        let detail: Value = serde_json::from_str(&r.str("detail")).unwrap_or(Value::Null);
        let post = detail["post"].as_str().unwrap_or_default().to_string();
        let key = format!("review-ask-lapsed-answered:{post}");
        if post.is_empty() || s.meta(&key).is_ok_and(|m| !m.0.is_empty()) {
            continue;
        }
        let _ = s.set_meta(&key, now);
        let number = detail["number"].as_i64().unwrap_or(0);
        let head = detail["head"].as_str().unwrap_or_default().to_string();
        // A merge is the owner's alone: only the owner's own pick asks it.
        if r.str("answer") != LAPSE_MERGE || r.str("decided_by") != hive_store::HUMAN_AGENT {
            lines.push(format!(
                "review {slug}: #{number} the owner waits for the review of {}",
                crate::recipe::short(&head, 12)
            ));
            continue;
        }
        let Some((pr, action)) = pr_state(&conn, &slug, number) else {
            continue;
        };
        if matches!(
            action.as_str(),
            "land.merged" | "land.pr-closed" | "review.merged"
        ) {
            continue;
        }
        match hive_tasks::ask_merge(s, &slug, number, &pr.task, &head, hive_store::HUMAN_AGENT) {
            Ok(()) => lines.push(format!(
                "review {slug}: #{number} the owner picked {LAPSE_MERGE:?}: Merge asked for {}",
                crate::recipe::short(&head, 12)
            )),
            Err(e) => lines.push(format!(
                "review {slug}: #{number} the owner's {LAPSE_MERGE:?} was not asked: {e}"
            )),
        }
    }
    lines
}

/// Merges a pull request as the owner (`review.merged` with `by: owner`),
/// bypassing the complexity rule: the hub's Merge button and `hive review
/// merge N --by owner` come here. The forge's own gate ([`refusal_of`])
/// is not bypassed: a red or running check, a draft, a blocked pull
/// request or a head that moved since the hub showed it is refused, and
/// the ask stands for the next pass. A failure of gh is a `review.failed`.
pub(crate) fn merge_as_owner(
    s: &hive_store::Store,
    project: &Row,
    number: i64,
    pressed: &str,
    now: &str,
) -> Result<String, String> {
    merge_pinned(s, project, number, None, pressed, "owner", ACTOR, now)
}

/// The head an event of a pull request records: the commit `land.pr`
/// pushed (`to`), the one a waiting merge found on the forge
/// (`review.again`, `head_sha`), or the one a review was queued at
/// (`review.queued`, `sha`).
const HEAD_ACTIONS: &str = "('land.pr', 'review.again', 'review.queued')";

fn head_of_event(action: &str, detail: &Value) -> String {
    let key = match action {
        "land.pr" => "to",
        AGAIN_ACTION => "head_sha",
        _ => "sha",
    };
    detail[key].as_str().unwrap_or_default().to_string()
}

/// An owner's Merge a newer head outran: the head it was pressed for, the
/// head Hive holds now, and why the head moved, in the owner's words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Outrun {
    pub was: String,
    pub head: String,
    pub why: String,
    /// The new head is Hive's own refresh (its base merged in): nothing
    /// new to review in the work itself.
    pub refresh: bool,
}

/// Whether the owner's Merge (the `review.ask` at `seq`, pressed for
/// `asked`) was outrun by a newer head. The head it was for is the one the
/// ask stores (`sha`, written by the hub from the state it judged the press
/// on); an older ask without one is read as the newest head an event
/// recorded before it. It is outrun when the newest head Hive recorded is
/// another one - whether that event was written before the ask (a press
/// on a page that was a moment behind) or after it. None when nothing
/// moved, or when no head was known at the click (an older ask is
/// honoured as before, behind the forge's gate).
fn ask_outrun(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    number: i64,
    seq: i64,
    asked: &str,
) -> Option<Outrun> {
    let rows = crate::util::query_rows(
        q,
        &format!(
            "SELECT seq, action, detail FROM events WHERE subject = ? AND action IN {HEAD_ACTIONS}
             AND json_extract(detail, '$.number') = ? ORDER BY seq"
        ),
        &[&slug, &number],
    )
    .unwrap_or_default();
    let mut was = asked.to_string();
    // The head Hive holds now, and the event that first recorded it (a
    // review queued at a head a refresh pushed names the refresh).
    let mut current: Option<(String, String, Value)> = None;
    for r in rows {
        let detail: Value = serde_json::from_str(&r.str("detail")).unwrap_or(Value::Null);
        let action = r.str("action");
        let head = head_of_event(&action, &detail);
        if head.is_empty() {
            continue;
        }
        if asked.is_empty() && (r.int("seq") as i64) < seq {
            was = head.clone();
        }
        if current.as_ref().is_none_or(|(h, _, _)| *h != head) {
            current = Some((head, action, detail));
        }
    }
    let (head, action, detail) = current?;
    if was.is_empty() || head == was {
        return None;
    }
    Some(Outrun {
        was,
        head,
        why: moved_because(&action, &detail),
        refresh: action == "land.pr" && detail["refreshed"].as_bool() == Some(true),
    })
}

/// Why a pull request's head moved, read from the event that recorded the
/// new one, in the owner's words.
fn moved_because(action: &str, detail: &Value) -> String {
    match action {
        "land.pr" if detail["refreshed"].as_bool() == Some(true) => {
            "Hive merged its base into it so that it can merge".to_string()
        }
        "land.pr" if !detail["fix"].as_str().unwrap_or_default().is_empty() => format!(
            "job {} pushed a fix onto it",
            detail["fix"].as_str().unwrap_or_default()
        ),
        "land.pr" => "new commits were pushed to it".to_string(),
        AGAIN_ACTION => "a new commit reached it on GitHub while its merge waited".to_string(),
        _ => "a newer commit was queued for review".to_string(),
    }
}

/// Says once that the owner's press lapsed, and hands the pull request
/// back with answers to pick (`review.ask-lapsed` names the question): a
/// question for the owner - on the Sessions strip, whatever the pull
/// request carries - with "Merge this commit" (the commit it is at now)
/// and "Wait for the review of the new commit", the second recommended
/// unless the new commit is Hive's own refresh; plus a message on the
/// task's conversation for a task's pull request, or a note in the chat's
/// transcript for a conversation's. Returns the log line's tail.
fn tell_lapsed(
    s: &hive_store::Store,
    conn: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    number: i64,
    seq: i64,
    outrun: &Outrun,
    now: &str,
) -> String {
    let pr = pr_state(conn, slug, number).map(|(p, _)| p);
    let url = pr
        .as_ref()
        .map(|p| p.url.clone())
        .filter(|u| !u.is_empty())
        .map(|u| format!(" ({u})"))
        .unwrap_or_default();
    let was = crate::recipe::short(&outrun.was, 12);
    let head = crate::recipe::short(&outrun.head, 12);
    let text = format!(
        "Pull request #{number}{url} was not merged: your Merge was for commit {was}, and {} - it is at {head} now. \
         The new commit is reviewed on its own; pick an answer on the Sessions strip to merge it now, or press Merge again if it comes back to you.",
        outrun.why,
    );
    let merge =
        format!("{LAPSE_MERGE} - merges commit {head} now, as you, without waiting for its review");
    let wait = format!(
        "{LAPSE_WAIT} - the reviewer reads commit {head}; it is merged, or comes back to you if it is still yours"
    );
    let answers = hive_board::option_lines(
        &[&merge, &wait],
        if outrun.refresh { 0 } else { 1 },
        if outrun.refresh {
            "the new commit only merges the base in; the work you approved is unchanged"
        } else {
            "the new commit changes what you approved; let it be reviewed first"
        },
    );
    let title = pr
        .as_ref()
        .map(|p| p.title.clone())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| format!("commit {head}"));
    let body = format!(
        "{text}\n\nWas: commit {was} (your Merge)\nNow: commit {head} ({})\n\n{answers}",
        outrun.why
    );
    let post = crate::board::create_post(
        s,
        now.to_string(),
        crate::board::PostOpts {
            kind: "question".into(),
            project: slug.to_string(),
            area: "runner".into(),
            thread: "review".into(),
            summary: format!("Merge pull request #{number}: {title}? Your Merge lapsed"),
            body,
            agent: crate::board::actor_agent(s, ACTOR.to_string()),
            date: now.to_string(),
            for_owner: true,
            ..Default::default()
        },
    )
    .map(|p| p.str("id"))
    .unwrap_or_default();
    event(
        s,
        now,
        ACTOR,
        LAPSED_ACTION,
        slug,
        json!({"project": slug, "number": number, "ask": seq, "was": outrun.was,
            "head": outrun.head, "why": outrun.why, "refresh": outrun.refresh, "post": post}),
    );
    let mut told = if post.is_empty() {
        String::new()
    } else {
        format!("; asked the owner ({post})")
    };
    let task = pr.as_ref().map(|p| p.task.clone()).unwrap_or_default();
    if !task.is_empty() {
        told.push_str(
            &match hive_tasks::send_message(s, slug, &task, ACTOR, &text) {
                Ok(_) => format!("; task {task} told"),
                Err(e) => format!("; task {task} not told: {e}"),
            },
        );
        return told;
    }
    let conversation = land_pr_field(conn, slug, number, "conversation");
    if conversation.is_empty() {
        return told;
    }
    match s.chat_get(&conversation) {
        Ok(Some(c)) if c.status != "archived" => {}
        _ => return told,
    }
    told.push_str(&match s.chat_add_message(
        &conversation,
        "note",
        &text,
        &json!({"pr": {"number": number, "url": pr.map(|p| p.url).unwrap_or_default(), "state": "merge_lapsed", "post": post}}),
    ) {
        Ok(_) => format!("; conversation {conversation} told"),
        Err(e) => format!("; conversation {conversation} not told: {e}"),
    });
    told
}

/// A field of the newest `land.pr` of one pull request.
fn land_pr_field(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    number: i64,
    key: &str,
) -> String {
    crate::util::query_rows(
        q,
        "SELECT detail FROM events WHERE subject = ? AND action = 'land.pr'
         AND json_extract(detail, '$.number') = ? ORDER BY seq DESC LIMIT 1",
        &[&slug, &number],
    )
    .unwrap_or_default()
    .first()
    .and_then(|r| serde_json::from_str::<Value>(&r.str("detail")).ok())
    .and_then(|d| d[key].as_str().map(ToString::to_string))
    .unwrap_or_default()
}

/// The head of a pull request as Hive last saw it: the sha its newest
/// review was queued at (`review.queued`: what the reviewer read), or,
/// when no review was queued yet, the one its newest `land.pr` pushed
/// (what the hub shows the owner).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SeenHead {
    pub sha: String,
    /// The sha is the one a review was queued at.
    pub reviewed: bool,
}

fn seen_head(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    number: i64,
) -> Option<SeenHead> {
    let reviewed = crate::util::query_rows(
        q,
        "SELECT COALESCE(json_extract(detail, '$.sha'), '') AS sha FROM events WHERE subject = ? AND action = 'review.queued'
         AND json_extract(detail, '$.number') = ? AND COALESCE(json_extract(detail, '$.sha'), '') != '' ORDER BY seq DESC LIMIT 1",
        &[&slug, &number],
    )
    .unwrap_or_default()
    .into_iter()
    .next()
    .map(|r| r.str("sha"));
    if let Some(sha) = reviewed {
        return Some(SeenHead {
            sha,
            reviewed: true,
        });
    }
    pr_state(q, slug, number)
        .map(|(pr, _)| pr.sha)
        .filter(|sha| !sha.is_empty())
        .map(|sha| SeenHead {
            sha,
            reviewed: false,
        })
}

/// What kind of thing stands between a pull request and its merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gate {
    /// Merged or closed on the forge already: the landing poll records it.
    Settled,
    /// Only checks still queued or running: nothing failed yet, so a
    /// reviewer's merge verdict waits for them ([`merge_or_wait`]).
    Pending,
    /// The head moved since it was reviewed (or pushed): what the verdict
    /// approved is not what the forge would merge. Never merged; a
    /// waiting merge is reviewed again ([`retry_merges`]).
    Moved,
    /// A red check, a conflict, a rule of the repository, a draft: a person
    /// decides.
    Stop,
}

/// Why `gh pr merge` must not be asked for this pull request as the forge
/// shows it now, or None when nothing stands in the way: the head moved
/// since Hive reviewed (or pushed) it, it is a draft, a check failed or
/// still runs, or `mergeStateStatus` says the forge would refuse (`DIRTY`:
/// conflicts, `BLOCKED`: a rule of the repository, `BEHIND`: the base must
/// be merged in first). A pull request with no checks at all passes: the
/// project may have no CI, and the caller records that none reported.
/// The [`Gate`] says what kind of refusal it is.
fn refusal_of(view: &PrView, seen: Option<&SeenHead>) -> Option<(Gate, String)> {
    let n = view.number;
    let other = |why: String| Some((Gate::Stop, why));
    if !view.state.is_empty() && view.state != "OPEN" {
        return Some((
            Gate::Settled,
            format!(
                "pull request #{n} is {} on GitHub; nothing to merge",
                view.state.to_ascii_lowercase()
            ),
        ));
    }
    if let Some(seen) = seen
        && !view.head_sha.is_empty()
        && seen.sha != view.head_sha
    {
        let was = crate::recipe::short(&seen.sha, 12);
        let now = crate::recipe::short(&view.head_sha, 12);
        return Some((
            Gate::Moved,
            if seen.reviewed {
                format!(
                    "the pull request moved since it was reviewed: reviewed {was}, now {now}; it is reviewed again"
                )
            } else {
                format!(
                    "the pull request moved since Hive pushed it: pushed {was}, now {now}; it is reviewed again"
                )
            },
        ));
    }
    if view.is_draft {
        return other(format!(
            "pull request #{n} is a draft; GitHub does not merge a draft"
        ));
    }
    if !view.failing_checks.is_empty() {
        return other(format!(
            "pull request #{n} has failing checks: {}; a red check is never merged",
            view.failing_checks.join(", ")
        ));
    }
    if !view.pending_checks.is_empty() {
        return Some((
            Gate::Pending,
            format!(
                "pull request #{n} has checks pending: {}; it is merged when they are green",
                view.pending_checks.join(", ")
            ),
        ));
    }
    match view.merge_state.as_str() {
        "DIRTY" => other(format!(
            "pull request #{n} conflicts with its base (mergeStateStatus DIRTY); delivery merges the base in when that is clean"
        )),
        "BLOCKED" => other(format!(
            "pull request #{n} is blocked on GitHub (mergeStateStatus BLOCKED: a required check, review or rule of the repository is not met)"
        )),
        "BEHIND" => other(format!(
            "pull request #{n} is behind its base (mergeStateStatus BEHIND); the base must be merged in before GitHub takes it"
        )),
        _ => None,
    }
}

/// Whether the project expects checks on a pull request: `pipeline.watch`
/// is on and the checkout has a workflow under `.github/workflows`. A merge
/// with no check reported is fine without; with, it is recorded as such.
fn ci_expected(s: &hive_store::Store, project: &Row) -> bool {
    if s.setting(crate::pipeline::WATCH, "fix").trim() == "off" {
        return false;
    }
    let dir = std::path::Path::new(&project.str("path")).join(".github/workflows");
    std::fs::read_dir(dir)
        .map(|d| {
            d.flatten().any(|e| {
                let name = e.file_name().to_string_lossy().to_ascii_lowercase();
                name.ends_with(".yml") || name.ends_with(".yaml")
            })
        })
        .unwrap_or(false)
}

/// `gh pr view`, then `gh pr merge --merge --delete-branch
/// --match-head-commit <the head read>` (without `--delete-branch` while
/// an open pull request is based on its head: `branch_kept_for` names
/// them), then `review.merged`; on a gh failure of the merge `review.failed` with the
/// error. Returns the pull request's url. Refused before gh is asked to
/// merge, and without a `review.failed`: a pull request stacked on another
/// (its turn has not come), and whatever [`refusal_of`] names from the
/// forge's view - the head moved since it was reviewed, a draft, a red or
/// running check, a blocked or dirty merge state. None of these is a
/// failure of the merge; an owner's ask stands until the next pass finds
/// the pull request ready. The `review.merged` records the head sha merged
/// and how many checks reported (`no_checks` when none did although the
/// project has workflows). The daemon's callers go through
/// [`merge_viewed`] or [`merge_as_owner`]; the tests call this.
#[cfg(test)]
pub(crate) fn merge(
    s: &hive_store::Store,
    project: &Row,
    number: i64,
    by: &str,
    actor: &str,
    now: &str,
) -> Result<String, String> {
    merge_viewed(s, project, number, None, by, actor, now)
}

/// [`merge`] with the `gh pr view` a caller has just read (`hive review
/// merge` reads it for its own preconditions): the view is not read twice,
/// and the gate judges the one given.
pub(crate) fn merge_viewed(
    s: &hive_store::Store,
    project: &Row,
    number: i64,
    viewed: Option<&PrView>,
    by: &str,
    actor: &str,
    now: &str,
) -> Result<String, String> {
    merge_pinned(s, project, number, viewed, "", by, actor, now)
}

/// [`merge_viewed`] pinned to `pin`, the commit the owner pressed Merge
/// for (empty: the head the forge is read at).
#[allow(clippy::too_many_arguments)]
fn merge_pinned(
    s: &hive_store::Store,
    project: &Row,
    number: i64,
    viewed: Option<&PrView>,
    pin: &str,
    by: &str,
    actor: &str,
    now: &str,
) -> Result<String, String> {
    match try_merge(s, project, number, viewed, pin, by, actor, now) {
        Tried::Merged(url) => Ok(url),
        Tried::Refused(_, why) | Tried::Unread(why) | Tried::Outrun(why) => Err(why),
        Tried::Failed { url, error } => {
            let slug = project.str("slug");
            event(
                s,
                now,
                actor,
                "review.failed",
                &slug,
                json!({"project": slug, "number": number, "url": url, "error": error}),
            );
            Err(error)
        }
    }
}

/// What one merge attempt came to.
enum Tried {
    /// Merged: `review.merged` is recorded; the pull request's url.
    Merged(String),
    /// Not asked of gh: the gate ([`refusal_of`]) or the stack said no.
    Refused(Gate, String),
    /// `gh pr view` could not be read.
    Unread(String),
    /// `gh pr merge` failed; nothing is recorded yet.
    Failed { url: String, error: String },
    /// The forge refused `gh pr merge --match-head-commit`: a commit
    /// reached the pull request between the read and the merge. Nothing is
    /// merged; the new head is recorded (`review.again`) and reviewed on
    /// its own. The words say what moved.
    Outrun(String),
}

/// Whether a failed `gh pr merge --match-head-commit` says the head is no
/// longer the commit it was pinned to (GitHub: "Head branch was modified.
/// Review and try the merge again."). Not a failure that passes: the
/// commit that was approved is not the one the forge would merge.
pub(crate) fn forge_head_moved(error: &str) -> bool {
    let low = error.to_ascii_lowercase();
    low.contains("head branch was modified")
        || low.contains("expectedheadoid")
        || low.contains("expected head")
}

/// The merge itself: the gate, `gh pr merge --match-head-commit`, and
/// `review.merged` when it went through. The merge is pinned to `pin` (the
/// commit the owner pressed Merge for) or, when it is empty, to the head
/// the forge was read at: a forge holding another commit than `pin` is
/// refused before gh is asked, and a commit pushed between the read and
/// the merge makes the forge refuse it ([`Tried::Outrun`]). A failure is
/// returned, not recorded: [`merge_viewed`] records it as `review.failed`,
/// [`merge_or_wait`] may wait instead.
#[allow(clippy::too_many_arguments)]
fn try_merge(
    s: &hive_store::Store,
    project: &Row,
    number: i64,
    viewed: Option<&PrView>,
    pin: &str,
    by: &str,
    actor: &str,
    now: &str,
) -> Tried {
    let slug = project.str("slug");
    let (known, seen) = match s.conn().ok() {
        Some(c) => (
            pr_state(&c, &slug, number).map(|(pr, _)| pr),
            seen_head(&c, &slug, number),
        ),
        None => (None, None),
    };
    let def = crate::baseline::default_branch(project.clone());
    if let Some(below) = known.as_ref().and_then(|p| stacked_on(p, &def)) {
        return Tried::Refused(
            Gate::Stop,
            format!("pull request #{number} is stacked on {below}; {below} must merge first"),
        );
    }
    let url = known.as_ref().map(|p| p.url.clone()).unwrap_or_default();
    let repo = repo_of(project, &url);
    let view = match viewed {
        Some(v) => v.clone(),
        None => match pr_view(s, &slug, &repo, number) {
            Ok(v) => v,
            Err(e) => return Tried::Unread(e),
        },
    };
    // The owner pressed Merge (or picked "Merge this commit") for the very
    // commit the forge holds: that it moved since the last review does not
    // stop it - the owner approved this commit - but every other gate does.
    let pressed_for_head = !pin.is_empty() && pin == view.head_sha;
    let refusal = match refusal_of(&view, seen.as_ref()) {
        Some((Gate::Moved, _)) if pressed_for_head => refusal_of(&view, None),
        other => other,
    };
    if let Some((gate, why)) = refusal {
        return Tried::Refused(gate, why);
    }
    if !pin.is_empty() && !view.head_sha.is_empty() && view.head_sha != pin {
        let why = format!(
            "the pull request moved since Merge was pressed: pressed {}, now {}; it is reviewed again",
            crate::recipe::short(pin, 12),
            crate::recipe::short(&view.head_sha, 12)
        );
        record_forge_head(s, &slug, number, &url, &view.head_sha, pin, &why, now);
        return Tried::Refused(Gate::Moved, why);
    }
    let pinned = if pin.is_empty() {
        view.head_sha.clone()
    } else {
        pin.to_string()
    };
    // A branch another open pull request is based on stays: GitHub closes
    // the pull requests whose base branch `--delete-branch` deletes, and a
    // closed one never reopens (#170 closed so when #168 merged). Delivery
    // moves them onto this one's base and deletes the branch after
    // (`delivery::retarget_stacked`).
    let head = if view.head.is_empty() {
        known.as_ref().map(|p| p.head.clone()).unwrap_or_default()
    } else {
        view.head.clone()
    };
    // Every pull request the forge still has open counts, whatever its
    // review said: one under a "changes" verdict (its fix job is running),
    // one whose approved merge waits, one handed to the owner (#267 closed
    // when #266 merged while its fix job ran).
    // A store that cannot say which are open keeps the branch: the merge
    // goes through, and delivery deletes the branch once a pass reads that
    // nothing is based on it (`delivery::delete_kept_head`).
    let (stacked_above, unread): (Vec<i64>, Option<String>) = if head.is_empty() {
        (Vec::new(), None)
    } else {
        match s
            .conn()
            .map_err(|e| format!("the store could not be opened: {e}"))
            .and_then(|c| forge_open_prs(&c, &slug))
        {
            Ok(open) => (
                open.into_iter()
                    .filter(|p| p.number != number && p.base == head)
                    .map(|p| p.number)
                    .collect(),
                None,
            ),
            Err(e) => (Vec::new(), Some(e)),
        }
    };
    let number_arg = number.to_string();
    let mut args = vec!["pr", "merge", number_arg.as_str(), "--merge"];
    if stacked_above.is_empty() && unread.is_none() {
        args.push("--delete-branch");
    }
    // Only the commit that was read (or pressed for) merges: a push onto
    // the pull request after the read makes the forge refuse it.
    if !pinned.is_empty() {
        args.extend(["--match-head-commit", pinned.as_str()]);
    }
    let result = gh(s, &slug, &repo, &args, MERGE_TIMEOUT);
    let job = std::env::var("HIVE_JOB").unwrap_or_default();
    match result {
        Ok(_) => {
            let mut detail = json!({"project": slug, "number": number, "url": url, "by": by,
                "head_sha": view.head_sha, "checks": view.checks});
            if !job.is_empty() {
                detail["job"] = json!(job);
            }
            if !stacked_above.is_empty() {
                detail["branch_kept_for"] = json!(stacked_above);
            }
            if let Some(why) = &unread {
                detail["branch_kept"] = json!(why);
            }
            if view.checks == 0 && ci_expected(s, project) {
                detail["no_checks"] = json!(true);
                detail["note"] = json!(
                    "merged with no checks reported, although the project has workflows under .github/workflows"
                );
            }
            event(s, now, actor, "review.merged", &slug, detail);
            Tried::Merged(url)
        }
        Err(e) if !pinned.is_empty() && forge_head_moved(&e) => {
            // Read the head the forge holds now, so Hive reviews that one.
            let head = pr_view(s, &slug, &repo, number)
                .map(|v| v.head_sha)
                .unwrap_or_default();
            let why = format!(
                "a commit reached the pull request after it was read: GitHub refused the merge at {}{}; it is reviewed again",
                crate::recipe::short(&pinned, 12),
                if head.is_empty() || head == pinned {
                    String::new()
                } else {
                    format!(", it is at {} now", crate::recipe::short(&head, 12))
                }
            );
            if !head.is_empty() && head != pinned {
                record_forge_head(s, &slug, number, &url, &head, &pinned, &why, now);
            }
            Tried::Outrun(why)
        }
        Err(e) => Tried::Failed {
            url,
            error: hive_cred::redact(&e, &[]),
        },
    }
}

/// The forge holds `head`, not the commit `was` a merge was pinned to:
/// `review.again` records it, so the pull request is open for review at
/// the new head and an owner's Merge pressed for `was` lapses. Written
/// once per head.
#[allow(clippy::too_many_arguments)]
fn record_forge_head(
    s: &hive_store::Store,
    slug: &str,
    number: i64,
    url: &str,
    head: &str,
    was: &str,
    why: &str,
    now: &str,
) {
    let known = s
        .conn()
        .ok()
        .and_then(|c| pr_state(&c, slug, number))
        .map(|(p, _)| p.sha)
        .unwrap_or_default();
    if known == head {
        return;
    }
    event(
        s,
        now,
        ACTOR,
        AGAIN_ACTION,
        slug,
        json!({"project": slug, "number": number, "url": url,
            "head_sha": head, "was": was, "note": why}),
    );
}

/// What a reviewer's merge verdict came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Merged on the forge; the pull request's url.
    Merged(String),
    /// The verdict stands and the merge waits (`review.merge-wait`): for
    /// checks still running, or for the forge after it refused the merge
    /// for a passing reason. The words say what is waited for.
    Waits(String),
}

/// Whether a failed `gh pr merge` (or `gh pr view`) failed for a reason
/// that passes on its own: GitHub answered 5xx or an internal error, a
/// rate limit (429, or a 403 that says so), the call timed out or lost its
/// connection (reset, refused, the network unreachable, no route to the
/// host, a broken pipe), or the base moved under the merge. A conflict, a red check,
/// a rule of the repository, a refused credential (401, 403) or a pull
/// request the forge does not know (404) does not.
pub(crate) fn passing_forge_error(error: &str) -> bool {
    static FIVE_XX: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"(?i)\b(http|status|error)[ :]*(5\d\d|429)\b|\b5\d\d (internal|bad gateway|service|gateway)",
        )
        .expect("5xx pattern")
    });
    // Go's net/http says a dropped connection as `EOF` / `unexpected EOF`.
    static EOF: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\bEOF\b").expect("EOF pattern"));
    let low = error.to_ascii_lowercase();
    FIVE_XX.is_match(error)
        || EOF.is_match(error)
        || [
            "internal server error",
            "bad gateway",
            "service unavailable",
            "gateway timeout",
            "something went wrong",
            "an internal error",
            "did not finish within",
            "base branch was modified",
            "connection reset",
            "connection refused",
            "error connecting",
            "network is unreachable",
            "no route to host",
            "broken pipe",
            "i/o timeout",
            "tls handshake timeout",
            "rate limit",
            "too many requests",
        ]
        .iter()
        .any(|m| low.contains(m))
}

/// The reviewer's merge verdict: [`merge_viewed`], except that a verdict
/// the forge cannot honour yet for a passing reason is kept rather than
/// failed. Checks still running record `review.merge-wait` (`waiting:
/// checks`) and the pass merges once they pass; a `gh pr merge` (or `gh pr
/// view`) that failed for a passing reason ([`passing_forge_error`])
/// records it (`waiting: forge`) and the pass tries again on the recovery
/// back-off ([`retry_merges`]). Neither queues a new review nor asks the
/// owner. Everything else - a conflict, a red check, a rule of the
/// repository, a moved head - is refused or failed as before.
pub(crate) fn merge_or_wait(
    s: &hive_store::Store,
    project: &Row,
    number: i64,
    viewed: Option<&PrView>,
    by: &str,
    actor: &str,
    now: &str,
) -> Result<Verdict, String> {
    let slug = project.str("slug");
    let head = viewed.map(|v| v.head_sha.as_str()).unwrap_or_default();
    let wait = |waiting: &str, error: &str, attempt: i64| {
        Verdict::Waits(start_wait(
            s, &slug, number, head, waiting, error, attempt, by, actor, now,
        ))
    };
    match try_merge(s, project, number, viewed, "", by, actor, now) {
        Tried::Merged(url) => Ok(Verdict::Merged(url)),
        Tried::Refused(Gate::Pending, why) => Ok(wait("checks", &why, 0)),
        Tried::Unread(e) if passing_forge_error(&e) => Ok(wait("forge", &e, 1)),
        Tried::Refused(_, why) | Tried::Unread(why) | Tried::Outrun(why) => Err(why),
        Tried::Failed { error, .. } if passing_forge_error(&error) => Ok(wait("forge", &error, 1)),
        Tried::Failed { url, error } => {
            event(
                s,
                now,
                actor,
                "review.failed",
                &slug,
                json!({"project": slug, "number": number, "url": url, "error": error}),
            );
            Err(error)
        }
    }
}

/// The reviewer's merge verdict when `gh pr view` itself failed for a
/// passing reason before anything could be judged (`hive review merge`
/// reads the pull request first): the verdict stands as a forge wait, its
/// first try spent, and [`retry_merges`] reads the pull request again -
/// the owner's rule ([`reasons_for`]) and the forge's gate included -
/// before anything is merged. Returns the words of what is waited for.
pub(crate) fn wait_unread(
    s: &hive_store::Store,
    project: &Row,
    number: i64,
    error: &str,
    by: &str,
    actor: &str,
    now: &str,
) -> String {
    start_wait(
        s,
        &project.str("slug"),
        number,
        "",
        "forge",
        error,
        1,
        by,
        actor,
        now,
    )
}

/// Records the first `review.merge-wait` of a verdict: the wait starts now.
#[allow(clippy::too_many_arguments)]
fn start_wait(
    s: &hive_store::Store,
    slug: &str,
    number: i64,
    head_sha: &str,
    waiting: &str,
    error: &str,
    attempt: i64,
    by: &str,
    actor: &str,
    now: &str,
) -> String {
    let url = s
        .conn()
        .ok()
        .and_then(|c| pr_state(&c, slug, number))
        .map(|(p, _)| p.url)
        .unwrap_or_default();
    let w = MergeWait {
        number,
        url,
        by: by.to_string(),
        reviewer: actor.to_string(),
        head_sha: head_sha.to_string(),
        waiting: waiting.to_string(),
        error: error.to_string(),
        attempt,
        due: String::new(),
        since: now.to_string(),
        last: false,
    };
    record_wait(s, slug, w, actor, now)
}

/// A merge verdict that waits, as its `review.merge-wait` event carries it.
#[derive(Debug, Clone, Default, PartialEq)]
struct MergeWait {
    number: i64,
    url: String,
    /// Who the merge is for (`review.merged`'s `by`).
    by: String,
    /// The reviewer whose verdict it is.
    reviewer: String,
    head_sha: String,
    /// `checks` (a check still runs) or `forge` (GitHub refused the merge,
    /// or could not be read, for a passing reason).
    waiting: String,
    /// The refusal or gh's error: the forge's last answer.
    error: String,
    /// How many times the forge failed the merge so far - refused it or
    /// could not be read (0 while checks run and nothing failed).
    attempt: i64,
    /// When the next merge is tried; empty: on every pass.
    due: String,
    /// When the verdict started to wait: the time bound counts from it.
    since: String,
    /// The wait is past its hours and its last try is still to come: the
    /// read before it failed for a passing reason, so the try waits for
    /// `due` like any other instead of being made at once.
    last: bool,
}

impl MergeWait {
    fn from_detail(d: &Value) -> Self {
        let s = |k: &str| d[k].as_str().unwrap_or_default().to_string();
        MergeWait {
            number: d["number"].as_i64().unwrap_or(0),
            url: s("url"),
            by: s("by"),
            reviewer: s("reviewer"),
            head_sha: s("head_sha"),
            waiting: s("waiting"),
            error: s("error"),
            attempt: d["attempt"].as_i64().unwrap_or(0),
            due: s("due"),
            since: s("since"),
            last: d["last"].as_bool().unwrap_or(false),
        }
    }
}

/// The forge's answer without gh's prefix, as the one line shows it.
fn forge_answer(error: &str) -> String {
    crate::run::squeeze(
        error
            .strip_prefix("gh pr merge failed: ")
            .or_else(|| error.strip_prefix("gh pr view failed: "))
            .unwrap_or(error),
        120,
    )
}

/// The one line that says what a waiting merge waits for: the Sessions
/// page shows it under the pull request (the event's `note`) and the
/// daemon log prints it.
fn wait_words(w: &MergeWait) -> String {
    if w.waiting == "checks" {
        let names = w
            .error
            .split_once("checks pending: ")
            .map(|(_, rest)| rest.split(';').next().unwrap_or(rest).trim().to_string())
            .unwrap_or_default();
        if names.is_empty() {
            "approved; the merge waits for the checks still running".to_string()
        } else {
            format!("approved; the merge waits for the checks still running ({names})")
        }
    } else {
        format!(
            "approved; GitHub refused the merge ({}); retried at {} (attempt {})",
            forge_answer(&w.error),
            w.due,
            w.attempt + 1
        )
    }
}

/// Records `review.merge-wait` for `w`, its `due` set from the recovery
/// back-off when the forge refused (5, 15, 45, 135 minutes, then 6 hours);
/// returns the words of what is waited for.
fn record_wait(
    s: &hive_store::Store,
    slug: &str,
    mut w: MergeWait,
    actor: &str,
    now: &str,
) -> String {
    w.due = if w.waiting == "forge" {
        crate::util::format_iso(
            crate::util::must_iso(now)
                + chrono::Duration::from_std(crate::failure::recover_backoff(
                    (w.attempt - 1).max(0) as i32,
                ))
                .unwrap_or_else(|_| chrono::Duration::zero()),
        )
    } else {
        String::new()
    };
    if w.since.is_empty() {
        w.since = now.to_string();
    }
    let words = wait_words(&w);
    event(
        s,
        now,
        actor,
        "review.merge-wait",
        slug,
        json!({"project": slug, "number": w.number, "url": w.url, "by": w.by,
            "reviewer": w.reviewer, "head_sha": w.head_sha, "waiting": w.waiting,
            "error": w.error, "attempt": w.attempt, "due": w.due, "since": w.since,
            "last": w.last, "note": words}),
    );
    words
}

/// The pull requests whose reviewer's merge waits: the newest state event
/// of the number is a `review.merge-wait`. (seq, wait) oldest number first.
fn waiting_merges(q: &(impl crate::util::Querier + ?Sized), slug: &str) -> Vec<(i64, MergeWait)> {
    let rows = crate::util::query_rows(
        q,
        &format!(
            "SELECT seq, action, detail FROM events WHERE subject = ? AND action IN {STATE_ACTIONS} ORDER BY seq"
        ),
        &[&slug],
    )
    .unwrap_or_default();
    let mut latest: std::collections::BTreeMap<i64, (i64, String, Value)> = Default::default();
    for r in rows {
        let detail: Value = serde_json::from_str(&r.str("detail")).unwrap_or(Value::Null);
        if let Some(n) = detail["number"].as_i64() {
            latest.insert(n, (r.int("seq") as i64, r.str("action"), detail));
        }
    }
    latest
        .into_values()
        .filter(|(_, action, _)| action == WAIT_ACTION)
        .map(|(seq, _, d)| (seq, MergeWait::from_detail(&d)))
        .collect()
}

/// The event a reviewer's merge verdict that waits records.
pub(crate) const WAIT_ACTION: &str = "review.merge-wait";

/// The event that ends a merge wait whose head moved on the forge: the
/// pull request is open for review again at the new head, as after a
/// `land.pr`, with no merge.
pub(crate) const AGAIN_ACTION: &str = "review.again";

/// How many times the forge may fail an approved merge (refuse it, or not
/// be read) before the wait ends.
pub(crate) const WAIT_TRIES: &str = "review.merge_wait_tries";
/// How many hours an approved merge may wait, for its checks or for the
/// forge, before the wait ends.
pub(crate) const WAIT_HOURS: &str = "review.merge_wait_hours";

/// The bounds of a merge wait: (tries, hours), each at least 1.
fn wait_bounds(s: &hive_store::Store) -> (i64, i64) {
    (
        s.setting_int(WAIT_TRIES).max(1),
        s.setting_int(WAIT_HOURS).max(1),
    )
}

/// Why the wait `w` ends at `now`, when it reached one of its bounds:
/// `tries` failures of the forge, or `hours` since it began. `failures`
/// counts the one just met.
fn wait_spent(w: &MergeWait, failures: i64, tries: i64, hours: i64, now: &str) -> Option<String> {
    if failures >= tries {
        return Some(format!(
            "the forge failed the approved merge {failures} time(s) (review.merge_wait_tries = {tries})"
        ));
    }
    if !w.since.is_empty()
        && crate::util::must_iso(now)
            >= crate::util::must_iso(&w.since) + chrono::Duration::hours(hours)
    {
        return Some(format!(
            "the approved merge waited since {} (review.merge_wait_hours = {hours})",
            w.since
        ));
    }
    None
}

fn wait_logged_key(slug: &str, number: i64) -> String {
    format!("review-wait-logged:{slug}:{number}")
}

/// The pass over merge verdicts that wait (`review.merge-wait`): once due,
/// the pull request is read again, the owner's rule ([`reasons_for`]) is
/// applied again (a reviewer's merge only), and it is merged when the gate
/// is clear - no new review run, no owner. Checks still running keep it
/// waiting (read on every pass); a passing forge error - of the merge or
/// of the read - waits for the next step of the recovery back-off. A wait
/// is bounded: after `review.merge_wait_tries` failures of the forge, or
/// `review.merge_wait_hours` since it began, it ends ([`give_up_wait`]):
/// the pull request goes to the owner and the project's lead is told once,
/// with the forge's last answer; a forge wait first found past its hours
/// with tries left makes one last try before it ends. A head that moved on the forge ends the
/// wait with `review.again` and a new review, no merge. A gate that turned
/// (a failed check, a conflict, a rule of the repository), a merge or read
/// error that does not pass, or a rule of the owner hands the pull request
/// to the owner as a refused merge always did. A pull request merged or
/// closed on the forge meanwhile is left to the landing poll. Each wait is
/// said once in the daemon log.
fn retry_merges(s: &hive_store::Store, project: &Row, now: &str) -> Vec<String> {
    let slug = project.str("slug");
    let Ok(conn) = s.conn() else {
        return Vec::new();
    };
    let (tries, hours) = wait_bounds(s);
    let mut lines = Vec::new();
    for (_, w) in waiting_merges(&conn, &slug) {
        let n = w.number;
        // A forge wait with tries left is read when due; one already past
        // its bounds is read now. Out of tries (the verdict spent the last
        // one, or the bound was lowered meanwhile) it ends without a merge.
        // Past only its hours (the machine slept, the daemon was down) it
        // makes one last try first, as a checks wait at its bound merges
        // when the checks passed: a failure then ends it ([`wait_spent`]
        // still says so). A checks wait is read on every pass and ends at
        // its bound while a check still runs.
        let spent = if w.waiting == "forge" {
            wait_spent(&w, w.attempt, tries, hours, now)
        } else {
            None
        };
        let last_try = spent.is_some() && w.attempt < tries;
        let spent_already = if last_try { None } else { spent };
        // A last try whose read failed waits for its due like any try.
        if spent_already.is_none()
            && (!last_try || w.last)
            && !w.due.is_empty()
            && crate::util::must_iso(&w.due) > crate::util::must_iso(now)
        {
            continue;
        }
        let by = if w.by.is_empty() {
            ACTOR
        } else {
            w.by.as_str()
        };
        let again = |waiting: &str, error: &str, attempt: i64| MergeWait {
            waiting: waiting.to_string(),
            error: error.to_string(),
            attempt,
            last: false,
            ..w.clone()
        };
        // A failure of the forge (the read or the merge): wait for the next
        // try, or end the wait at its bound. The last try past the hours is
        // the merge's: a read that fails for a passing reason before it
        // counts as a failure of the forge but does not spend it - the
        // next due pass makes the last try. That grace is given once: a
        // read that fails again on the deferred last try (`w.last`) ends
        // the wait on its hours, so a large `review.merge_wait_tries` does
        // not go on in 6-hour steps past `review.merge_wait_hours`.
        let forge_failed = |error: &str, merge_asked: bool, lines: &mut Vec<String>| {
            let failures = w.attempt + 1;
            let bound = if last_try && !merge_asked && !w.last {
                wait_spent(&w, failures, tries, hours, now).filter(|_| failures >= tries)
            } else {
                wait_spent(&w, failures, tries, hours, now)
            };
            match bound {
                Some(bound) => lines.push(give_up_wait(s, project, &w, &bound, error, now)),
                None => {
                    let next = MergeWait {
                        last: last_try && !merge_asked,
                        ..again("forge", error, failures)
                    };
                    record_wait(s, &slug, next, ACTOR, now);
                }
            }
        };
        let url = if w.url.is_empty() {
            pr_state(&conn, &slug, n)
                .map(|(p, _)| p.url)
                .unwrap_or_default()
        } else {
            w.url.clone()
        };
        let view = match pr_view(s, &slug, &repo_of(project, &url), n) {
            Ok(v) => v,
            Err(e) if passing_forge_error(&e) => {
                forge_failed(&e, false, &mut lines);
                continue;
            }
            Err(e) => {
                hand_waiting_to_owner(s, project, &w, std::slice::from_ref(&e), now);
                lines.push(format!(
                    "review {slug}: #{n} handed to the owner: the approved merge cannot be read: {e}"
                ));
                continue;
            }
        };
        // Merged or closed on the forge meanwhile: the landing poll
        // records it, and that ends the wait.
        if !view.state.is_empty() && view.state != "OPEN" {
            continue;
        }
        // The owner's rule, read again: `review.merge = ask` set while the
        // merge waited, a guarded path, a missing report.
        if by != "owner" {
            let reasons = reasons_for(s, &view);
            if !reasons.is_empty() {
                hand_waiting_to_owner(s, project, &w, &reasons, now);
                lines.push(format!(
                    "review {slug}: #{n} handed to the owner: the approved merge is the owner's now: {}",
                    reasons.join("; ")
                ));
                continue;
            }
        }
        if let Some(bound) = &spent_already
            && !matches!(
                refusal_of(&view, seen_head(&conn, &slug, n).as_ref()),
                Some((Gate::Moved, _))
            )
        {
            lines.push(give_up_wait(s, project, &w, bound, &w.error, now));
            continue;
        }
        match try_merge(s, project, n, Some(&view), "", by, ACTOR, now) {
            Tried::Merged(url) => {
                let _ = s.set_meta(&wait_logged_key(&slug, n), "");
                lines.push(format!(
                    "review {slug}: #{n} merged after waiting ({}) {url}",
                    if w.waiting == "checks" {
                        "for its checks".to_string()
                    } else {
                        format!("{} refused merges", w.attempt)
                    }
                ));
            }
            Tried::Refused(Gate::Settled, _) => {}
            Tried::Refused(Gate::Pending, why) => {
                if let Some(bound) = wait_spent(&w, 0, tries, hours, now) {
                    lines.push(give_up_wait(s, project, &w, &bound, &why, now));
                } else if w.waiting != "checks" || w.error != why {
                    record_wait(s, &slug, again("checks", &why, w.attempt), ACTOR, now);
                }
            }
            Tried::Refused(Gate::Moved, why) => {
                review_again(s, &slug, &w, &view, &why, now);
                lines.push(format!("review {slug}: #{n} not merged: {why}"));
            }
            // The forge refused the pinned merge: the head moved after the
            // read. `try_merge` recorded the new head (`review.again`) when
            // it could read it; otherwise the next pass finds it moved.
            Tried::Outrun(why) => {
                let _ = s.set_meta(&wait_logged_key(&slug, n), "");
                lines.push(format!("review {slug}: #{n} not merged: {why}"));
            }
            Tried::Refused(Gate::Stop, why) => {
                hand_waiting_to_owner(s, project, &w, std::slice::from_ref(&why), now);
                lines.push(format!(
                    "review {slug}: #{n} handed to the owner: the approved merge cannot go through: {why}"
                ));
            }
            Tried::Unread(e) if passing_forge_error(&e) => {
                forge_failed(&e, false, &mut lines);
            }
            Tried::Failed { error: e, .. } if passing_forge_error(&e) => {
                forge_failed(&e, true, &mut lines);
            }
            Tried::Unread(error) | Tried::Failed { error, .. } => {
                hand_waiting_to_owner(s, project, &w, std::slice::from_ref(&error), now);
                lines.push(format!(
                    "review {slug}: #{n} handed to the owner: the approved merge failed: {error}"
                ));
            }
        }
    }
    // One line per wait, when it starts or changes: the newest
    // review.merge-wait not said yet.
    for (seq, w) in waiting_merges(&conn, &slug) {
        let key = wait_logged_key(&slug, w.number);
        let said = s.meta(&key).map(|m| m.0).unwrap_or_default();
        if said == seq.to_string() {
            continue;
        }
        let _ = s.set_meta(&key, &seq.to_string());
        lines.push(format!("review {slug}: #{} {}", w.number, wait_words(&w)));
    }
    lines
}

/// A head that moved on the forge while the merge waited: nothing is
/// merged, and `review.again` opens the pull request for a new review at
/// the head the forge holds now.
fn review_again(
    s: &hive_store::Store,
    slug: &str,
    w: &MergeWait,
    view: &PrView,
    why: &str,
    now: &str,
) {
    let _ = s.set_meta(&wait_logged_key(slug, w.number), "");
    event(
        s,
        now,
        ACTOR,
        AGAIN_ACTION,
        slug,
        json!({"project": slug, "number": w.number, "url": w.url,
            "head_sha": view.head_sha, "was": w.head_sha, "note": why}),
    );
}

/// A merge wait that reached its bound ends: the pull request goes to the
/// owner (`review.owner`, the question, the Merge button) and the
/// project's lead is told once on the same thread, with the forge's last
/// answer. Returns the daemon log line.
fn give_up_wait(
    s: &hive_store::Store,
    project: &Row,
    w: &MergeWait,
    bound: &str,
    last: &str,
    now: &str,
) -> String {
    let slug = project.str("slug");
    let n = w.number;
    let answer = forge_answer(last);
    let why = format!("{bound}; the forge's last answer: {answer}");
    hand_waiting_to_owner(s, project, w, std::slice::from_ref(&why), now);
    let lead = crate::lead::lead_name(slug.clone());
    let url = if w.url.is_empty() {
        String::new()
    } else {
        format!(" ({})", w.url)
    };
    let body = format!(
        "@{lead}: the reviewer's approved merge of pull request #{n}{url} stopped waiting: {bound}.\n\n\
         The forge's last answer: {}\n\n\
         The pull request is with the owner now (review.owner, its Merge button and the question on this thread); \
         a pull request stacked on it waits until it merges. Decide whether it merges as it is, needs a fix, or is closed.\n",
        crate::run::squeeze(last, 600)
    );
    let _ = crate::board::create_post(
        s,
        now.to_string(),
        crate::board::PostOpts {
            kind: "finding".into(),
            project: slug.clone(),
            area: "runner".into(),
            thread: "review".into(),
            summary: format!("The approved merge of #{n} stopped waiting: {answer}"),
            body,
            agent: crate::board::actor_agent(s, ACTOR.to_string()),
            date: now.to_string(),
            ..Default::default()
        },
    );
    format!("review {slug}: #{n} handed to the owner, the lead told: {why}")
}

/// A merge that waited and can no longer go through goes to the owner the
/// way a refused merge does: `review.owner` with the refusal as its reason.
fn hand_waiting_to_owner(
    s: &hive_store::Store,
    project: &Row,
    w: &MergeWait,
    reasons: &[String],
    now: &str,
) {
    let slug = project.str("slug");
    let _ = s.set_meta(&wait_logged_key(&slug, w.number), "");
    let pr = s
        .conn()
        .ok()
        .and_then(|c| pr_state(&c, &slug, w.number))
        .map(|(p, _)| p)
        .unwrap_or_else(|| OpenPr {
            number: w.number,
            url: w.url.clone(),
            ..Default::default()
        });
    let reviewer = if w.reviewer.is_empty() {
        "The reviewer".to_string()
    } else {
        w.reviewer.clone()
    };
    let note = format!(
        "{reviewer} approved pull request #{} and its merge waited; it cannot go through now: {}",
        w.number,
        reasons.join("; ")
    );
    hand_to_owner(s, project, &pr, reasons, &note, ACTOR, now);
}

/// One project's pass: review jobs that ended without a verdict become
/// `review.failed`; then, when no review job is queued or running, the
/// oldest open pull request gets one (or, after `MAX_ATTEMPTS` failed
/// reviews of its head, goes to the owner). A pull request stacked on
/// another waits: the stack is reviewed bottom up, and delivery moves it
/// onto the default branch when the one below merges.
fn tend_project(s: &hive_store::Store, project: &Row, now: &str) -> Vec<String> {
    let slug = project.str("slug");
    let Ok(conn) = s.conn() else {
        return Vec::new();
    };
    let open = open_prs(&conn, &slug);
    if open.is_empty() {
        return Vec::new();
    }
    let def = crate::baseline::default_branch(project.clone());
    let mut lines = Vec::new();
    let mut settled_now = std::collections::BTreeSet::new();
    for pr in &open {
        if let Some(line) = settle_finished(s, &conn, &slug, pr, now) {
            lines.push(line);
            settled_now.insert(pr.number);
        }
    }
    if active_review_job(&conn, &slug).is_some() {
        return lines;
    }
    for pr in &open {
        if settled_now.contains(&pr.number) {
            continue;
        }
        if let Some(below) = stacked_on(pr, &def) {
            lines.push(format!(
                "review {slug}: #{} stacked on {below}: reviewed after {below} merges",
                pr.number
            ));
            continue;
        }
        if attempts_at(&conn, &slug, pr) >= MAX_ATTEMPTS {
            let reasons = vec![FAILED_TWICE.to_string()];
            let note = format!(
                "Two review jobs of pull request #{} ended without a verdict; the pull request waits for you.",
                pr.number
            );
            hand_to_owner(s, project, pr, &reasons, &note, ACTOR, now);
            lines.push(format!(
                "review {slug}: #{} handed to the owner: {FAILED_TWICE}",
                pr.number
            ));
            continue;
        }
        let repo = repo_of(project, &pr.url);
        let view = match pr_view(s, &slug, &repo, pr.number) {
            Ok(v) => v,
            Err(e) => {
                lines.push(format!("review {slug}: #{} not queued: {e}", pr.number));
                continue;
            }
        };
        if view.state != "OPEN" && !view.state.is_empty() {
            // Merged or closed on the forge and the landing poll has not
            // recorded it yet: that poll owns the bookkeeping.
            continue;
        }
        let reasons = reasons_for(s, &view);
        match queue_review(s, project, pr, &view, &reasons, now) {
            Ok(job) => lines.push(format!(
                "review {slug}: #{} queued for review ({}){}",
                pr.number,
                job.str("id"),
                if reasons.is_empty() {
                    String::new()
                } else {
                    format!("; for the owner: {}", reasons.join("; "))
                }
            )),
            Err(e) => lines.push(format!("review {slug}: #{}: {e}", pr.number)),
        }
        // One review job per project at a time.
        return lines;
    }
    lines
}

/// The newest review job of the pull request ended and no verdict (or
/// failure) was recorded after it was queued: records `review.failed`
/// with the job's result so the next pass may queue one more attempt.
fn settle_finished(
    s: &hive_store::Store,
    conn: &hive_store::PooledConn,
    slug: &str,
    pr: &OpenPr,
    now: &str,
) -> Option<String> {
    let (job, queued_seq) = newest_review_job(conn, slug, pr.number)?;
    let status = job.str("status");
    if matches!(status.as_str(), "queued" | "leased") {
        return None;
    }
    if !actions_after(
        conn,
        slug,
        pr.number,
        queued_seq,
        "('review.merged', 'review.changes', 'review.owner', 'review.failed', 'review.merge-wait', 'review.again')",
    )
    .is_empty()
    {
        return None;
    }
    let result = job.str("result");
    let error = if result.trim().is_empty() {
        format!(
            "review job {} ended {status} without a verdict",
            job.str("id")
        )
    } else {
        format!(
            "review job {} ended {status} without a verdict: {}",
            job.str("id"),
            crate::util::trunc(result.trim(), 400)
        )
    };
    event(
        s,
        now,
        ACTOR,
        "review.failed",
        slug,
        json!({"project": slug, "number": pr.number, "url": pr.url, "error": error, "job": job.str("id")}),
    );
    Some(format!(
        "review {slug}: #{} review {} ended {status} without a verdict",
        pr.number,
        job.str("id")
    ))
}

/// Hands a pull request to the owner: `review.owner`, a question on the
/// project's board marked for the owner, and a message on the task's
/// conversation when it is a task pull request.
pub(crate) fn hand_to_owner(
    s: &hive_store::Store,
    project: &Row,
    pr: &OpenPr,
    reasons: &[String],
    note: &str,
    actor: &str,
    now: &str,
) -> String {
    let slug = project.str("slug");
    let n = pr.number;
    event(
        s,
        now,
        actor,
        "review.owner",
        &slug,
        json!({"project": slug, "number": n, "url": pr.url, "reasons": reasons, "note": note}),
    );
    let title = if pr.title.is_empty() {
        String::new()
    } else {
        format!(": {}", pr.title)
    };
    let mut body = format!("Pull request #{n}{title}\n{}\n\n", pr.url);
    if !reasons.is_empty() {
        body.push_str("Why it waits for you:\n");
        for r in reasons {
            body.push_str(&format!("- {r}\n"));
        }
        body.push('\n');
    }
    if !note.trim().is_empty() {
        body.push_str(&format!("The review:\n\n{}\n\n", note.trim()));
    }
    body.push_str(&format!(
        "Merge it from the task's sheet or the pull request's card in the hub, or with `hive review merge {n} --by owner --project {slug}`; close it on GitHub to drop it.\n\n{}",
        hive_board::yes_no_lines(
            ("Merge it", "your go-ahead is recorded; the merge itself is the Merge button on the pull request's card"),
            ("Not yet", "it stays open; close it on GitHub to drop it"),
            false,
            "read the reasons above first; a merge is yours alone and cannot be undone by Hive",
        )
    ));
    let post = crate::board::create_post(
        s,
        now.to_string(),
        crate::board::PostOpts {
            kind: "question".into(),
            project: slug.clone(),
            area: "runner".into(),
            thread: "review".into(),
            summary: format!("Merge pull request #{n}{title}?"),
            body,
            agent: crate::board::actor_agent(s, actor.to_string()),
            date: now.to_string(),
            for_owner: true,
            ..Default::default()
        },
    )
    .map(|p| p.str("id"))
    .unwrap_or_default();
    if !pr.task.is_empty() {
        let mut msg = format!("Pull request #{n} ({}) waits for your merge.", pr.url);
        if !reasons.is_empty() {
            msg.push_str(&format!(" Why: {}.", reasons.join("; ")));
        }
        if !note.trim().is_empty() {
            msg.push_str(&format!("\n\nThe review:\n\n{}", note.trim()));
        }
        let _ = hive_tasks::send_message(s, &slug, &pr.task, actor, &msg);
    }
    post
}

/// The follow-up job after a `changes` verdict: a task pull request gets a
/// `task` job on the task's source (the delivery folds its branch into the
/// task's pull request); any other kind gets an `improve` job on
/// `pr-fix:<slug>:<number>` (the delivery pushes its branch onto the pull
/// request's head). Both briefs start by putting the clone on the head.
pub(crate) fn queue_follow_up(
    s: &hive_store::Store,
    project: &Row,
    pr: &OpenPr,
    view: &PrView,
    note: &str,
    actor: &str,
) -> Result<Row, String> {
    let slug = project.str("slug");
    let n = pr.number;
    let title = if view.title.is_empty() {
        &pr.title
    } else {
        &view.title
    };
    let (kind, source, dedupe) = if pr.kind == "task" && !pr.task.is_empty() {
        ("task", format!("task:{}", pr.task), false)
    } else {
        ("improve", format!("{FIX_SOURCE_PREFIX}{slug}:{n}"), true)
    };
    let brief = format!(
        "Run `hive review checkout {n}` first so your work sits on the pull request's head.\n\n\
         Pull request #{n} ({}) - {title} - was reviewed and the reviewer asked for these changes:\n\n{}\n\n\
         Make exactly these fixes on top of the pull request's head (`{}` after the checkout), run the project's check, and commit. \
         Do not rewrite the pull request's history and do not widen its scope; the reviewer reads the new commits next.",
        pr.url,
        note.trim(),
        review_branch(n)
    );
    let (job, _) = crate::jobs::add_job(
        s,
        slug,
        kind.into(),
        format!("Address the review of PR #{n}: {title}"),
        brief,
        1,
        FOLLOW_UP_PRIORITY,
        source,
        actor.into(),
        dedupe,
    )
    .map_err(|e| format!("could not queue the follow-up job: {e}"))?;
    Ok(job)
}

/// Asks for changes on GitHub and records `review.changes` with the
/// follow-up job. GitHub refuses a review verdict from the pull request's
/// own author, and Hive opens its pull requests as the same gh account
/// that reviews them, so a refused `--request-changes` becomes a comment.
pub(crate) fn request_changes(
    s: &hive_store::Store,
    project: &Row,
    pr: &OpenPr,
    view: &PrView,
    note: &str,
    actor: &str,
    now: &str,
) -> Result<Row, String> {
    let slug = project.str("slug");
    let repo = repo_of(project, &pr.url);
    let n = pr.number.to_string();
    let body = format!("Changes requested by the Hive reviewer:\n\n{}", note.trim());
    let posted = gh(
        s,
        &slug,
        &repo,
        &["pr", "review", &n, "--request-changes", "--body", &body],
        GH_TIMEOUT,
    );
    if let Err(e) = posted {
        if !e.to_ascii_lowercase().contains("own pull request") {
            fail(s, &slug, pr, &e, actor, now);
            return Err(e);
        }
        if let Err(e) = gh(
            s,
            &slug,
            &repo,
            &["pr", "comment", &n, "--body", &body],
            GH_TIMEOUT,
        ) {
            fail(s, &slug, pr, &e, actor, now);
            return Err(e);
        }
    }
    let job = queue_follow_up(s, project, pr, view, note, actor)?;
    event(
        s,
        now,
        actor,
        "review.changes",
        &slug,
        json!({"project": slug, "number": pr.number, "url": pr.url, "note": note.trim(), "follow_up": job.str("id")}),
    );
    if !pr.task.is_empty() {
        let _ = hive_tasks::send_message(
            s,
            &slug,
            &pr.task,
            actor,
            &format!(
                "The review of pull request #{} ({}) asked for changes; job {} makes them on the pull request's head.\n\n{}",
                pr.number,
                pr.url,
                job.str("id"),
                note.trim()
            ),
        );
    }
    Ok(job)
}

/// Comments the review on GitHub and hands the pull request to the owner.
pub(crate) fn escalate(
    s: &hive_store::Store,
    project: &Row,
    pr: &OpenPr,
    reasons: &[String],
    note: &str,
    actor: &str,
    now: &str,
) -> Result<String, String> {
    let slug = project.str("slug");
    let repo = repo_of(project, &pr.url);
    let mut body = String::from("The Hive reviewer handed this pull request to the owner.");
    if !reasons.is_empty() {
        body.push_str("\n\nWhy:\n");
        for r in reasons {
            body.push_str(&format!("- {r}\n"));
        }
    }
    if !note.trim().is_empty() {
        body.push_str(&format!("\n\nThe review:\n\n{}", note.trim()));
    }
    if let Err(e) = gh(
        s,
        &slug,
        &repo,
        &["pr", "comment", &pr.number.to_string(), "--body", &body],
        GH_TIMEOUT,
    ) {
        fail(s, &slug, pr, &e, actor, now);
        return Err(e);
    }
    Ok(hand_to_owner(s, project, pr, reasons, note, actor, now))
}

fn fail(s: &hive_store::Store, slug: &str, pr: &OpenPr, error: &str, actor: &str, now: &str) {
    event(
        s,
        now,
        actor,
        "review.failed",
        slug,
        json!({"project": slug, "number": pr.number, "url": pr.url, "error": hive_cred::redact(error, &[])}),
    );
}

/// Puts the pull request's head on `review/pr-N` in `dir` (a job clone)
/// and checks it out. The head is fetched from the project's local checkout
/// first - the branch the daemon pushed is there and the clone has no
/// remote of its own - and from the upstream's `refs/pull/N/head` when the
/// local branch is not at the pull request's head commit (the daemon's
/// checkout may be behind a push made elsewhere).
pub(crate) fn checkout_into(
    s: &hive_store::Store,
    project: &Row,
    view: &PrView,
    dir: &str,
) -> Result<String, String> {
    let root = project.str("path");
    let number = view.number;
    let branch = review_branch(number);
    let inside = crate::util::git_timeout(
        dir,
        Duration::from_secs(10),
        &["rev-parse", "--show-toplevel"],
    );
    if !inside.ok() {
        return Err(format!("{dir} is not a git checkout"));
    }
    let top = std::fs::canonicalize(inside.stdout.trim()).unwrap_or_default();
    let root_canon = std::fs::canonicalize(&root).unwrap_or_default();
    if !root.is_empty() && top == root_canon {
        return Err(format!(
            "{dir} is the project's own checkout; run the review inside a job clone"
        ));
    }
    let refspec = format!("+refs/heads/{}:refs/heads/{branch}", view.head);
    let mut have = String::new();
    if !root.is_empty() && !view.head.is_empty() {
        let fetched = crate::util::git_timeout(
            dir,
            Duration::from_secs(120),
            &["fetch", "--quiet", "--no-tags", &root, &refspec],
        );
        if fetched.ok() {
            have = crate::util::git_timeout(
                dir,
                Duration::from_secs(10),
                &["rev-parse", &format!("refs/heads/{branch}")],
            )
            .stdout
            .trim()
            .to_string();
        }
    }
    if !root.is_empty()
        && !view.head.is_empty()
        && !view.head_sha.is_empty()
        && have != view.head_sha
        && let Some(tracking) = tracking_at(&root, &view.head, &view.head_sha)
    {
        // The local branch is behind a push the daemon made by url (a
        // refresh that merged the base in): the checkout's remote-tracking
        // ref holds the head, and the job needs no upstream for it.
        let fetched = crate::util::git_timeout(
            dir,
            Duration::from_secs(120),
            &[
                "fetch",
                "--quiet",
                "--no-tags",
                &root,
                &format!("+{tracking}:refs/heads/{branch}"),
            ],
        );
        if fetched.ok() {
            have = crate::util::git_timeout(
                dir,
                Duration::from_secs(10),
                &["rev-parse", &format!("refs/heads/{branch}")],
            )
            .stdout
            .trim()
            .to_string();
        }
    }
    if have.is_empty() || (!view.head_sha.is_empty() && have != view.head_sha) {
        let url = upstream_url(project, &view.url);
        if url.is_empty() {
            return Err(format!(
                "the local checkout does not hold {} at {} and no upstream url is known",
                view.head,
                short(&view.head_sha)
            ));
        }
        let (env, cleanup, why) = crate::forge::remote_credential_env(s, root.clone(), &url);
        if !why.is_empty() {
            return Err(why);
        }
        let pull_ref = format!("+refs/pull/{number}/head:refs/heads/{branch}");
        let fetched = crate::forge::git_upstream(
            dir,
            Duration::from_secs(300),
            &env,
            &url,
            &["fetch", "--quiet", "--no-tags", &url, &pull_ref],
        );
        cleanup("", &[]);
        if !fetched.ok() {
            return Err(hive_cred::redact(
                &format!(
                    "fetch of pull/{number}/head failed: {}",
                    crate::run::squeeze(&fetched.stderr, 200)
                ),
                &[],
            ));
        }
        have = crate::util::git_timeout(
            dir,
            Duration::from_secs(10),
            &["rev-parse", &format!("refs/heads/{branch}")],
        )
        .stdout
        .trim()
        .to_string();
    }
    let co = crate::util::git_timeout(
        dir,
        Duration::from_secs(60),
        &["checkout", "--quiet", &branch],
    );
    if !co.ok() {
        return Err(format!(
            "checkout of {branch} failed: {}",
            crate::run::squeeze(&co.stderr, 200)
        ));
    }
    Ok(have)
}

/// The checkout's remote-tracking ref of `head` (`refs/remotes/<remote>/
/// <head>`, any remote) that is at `sha`, when one is.
fn tracking_at(root: &str, head: &str, sha: &str) -> Option<String> {
    crate::util::git_timeout(
        root,
        Duration::from_secs(10),
        &[
            "for-each-ref",
            "--format=%(objectname) %(refname)",
            "refs/remotes/",
        ],
    )
    .stdout
    .lines()
    .filter_map(|l| l.trim().split_once(' '))
    .find(|(oid, name)| {
        *oid == sha
            && name
                .strip_prefix("refs/remotes/")
                .and_then(|r| r.split_once('/'))
                .is_some_and(|(_, b)| b == head)
    })
    .map(|(_, name)| name.to_string())
}

/// The url the pull request's upstream is fetched from: the default
/// branch's tracking remote, else the project's recorded remote.
fn upstream_url(project: &Row, _pr_url: &str) -> String {
    let root = project.str("path");
    let def = crate::baseline::default_branch(project.clone());
    if !root.is_empty()
        && let Some((remote, _, _)) = crate::forge::tracking_of(&root, &def)
    {
        return crate::forge::push_url(&root, &remote);
    }
    project.str("remote")
}

fn short(sha: &str) -> &str {
    if sha.len() > 12 { &sha[..12] } else { sha }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::*;
    use crate::test_helpers_daemon::*;

    /// A `gh` on PATH that answers like GitHub: `pr view` prints the JSON in
    /// the `view` file (the test writes it), `pr diff` a one-line diff,
    /// `pr merge`/`pr review`/`pr comment` succeed unless the `fail` file
    /// names the verb, and `auth status` knows no accounts. Every call is
    /// logged. A `pushed` file holding a sha is a push that lands while
    /// `pr merge` runs: the view becomes `view.next`, and a merge pinned
    /// with `--match-head-commit` to another commit is refused as GitHub
    /// refuses it; an unpinned one merges whatever the head is.
    fn fake_gh() -> (
        tempfile::TempDir,
        hive_testutil::SetenvGuard,
        String,
        std::path::PathBuf,
    ) {
        let bin = tempfile::tempdir().unwrap();
        let log = bin.path().join("gh.log").display().to_string();
        let view = bin.path().join("view");
        std::fs::write(
            &view,
            view_json(7, "hive/task/TASK-001", &[("src/a.rs", 5, 2)]),
        )
        .unwrap();
        let fail = bin.path().join("fail");
        std::fs::write(&fail, "").unwrap();
        let body = format!(
            "#!/bin/sh\necho \"$@\" >> '{log}'\nif grep -q \"^$2$\" '{fail}'; then echo \"$(cat '{failmsg}')\" >&2; exit 1; fi\ncase \"$1 $2\" in\n\"pr view\") cat '{view}' ;;\n\"pr diff\") echo 'diff --git a/src/a.rs b/src/a.rs' ;;\n\"pr merge\") if [ -s '{pushed}' ]; then cp '{next}' '{view}'; want=$(cat '{pushed}'); pin=''; prev=''; for a in \"$@\"; do [ \"$prev\" = --match-head-commit ] && pin=\"$a\"; prev=\"$a\"; done; if [ -n \"$pin\" ] && [ \"$pin\" != \"$want\" ]; then echo 'GraphQL: Head branch was modified. Review and try the merge again. (mergePullRequest)' >&2; exit 1; fi; fi; echo merged ;;\n\"pr review\") echo reviewed ;;\n\"pr comment\") echo commented ;;\n\"auth status\") exit 1 ;;\n*) echo \"unexpected gh $*\" >&2; exit 2 ;;\nesac\n",
            view = view.display(),
            fail = fail.display(),
            failmsg = bin.path().join("failmsg").display(),
            pushed = bin.path().join("pushed").display(),
            next = bin.path().join("view.next").display(),
        );
        std::fs::write(bin.path().join("failmsg"), "HTTP 422: boom").unwrap();
        let path = bin.path().join("gh");
        std::fs::write(&path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let guard = hive_testutil::setenv(
            "PATH",
            format!(
                "{}:{}",
                bin.path().display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        );
        (bin, guard, log, view)
    }

    fn view_json(number: i64, head: &str, files: &[(&str, i64, i64)]) -> String {
        view_value(number, head, files).to_string()
    }

    /// The `gh pr view` answer for an open pull request at `abc123` with
    /// one green check; a test edits the fields it wants otherwise.
    fn view_value(number: i64, head: &str, files: &[(&str, i64, i64)]) -> Value {
        let adds: i64 = files.iter().map(|f| f.1).sum();
        let dels: i64 = files.iter().map(|f| f.2).sum();
        json!({
            "number": number, "title": format!("PR {number} title"), "body": "what it does",
            "url": format!("https://github.com/o/r/pull/{number}"), "state": "OPEN",
            "headRefName": head, "headRefOid": "abc123", "baseRefName": "main",
            "author": {"login": "hive-bot"}, "additions": adds, "deletions": dels,
            "files": files.iter().map(|(p, a, d)| json!({"path": p, "additions": a, "deletions": d})).collect::<Vec<_>>(),
            "isDraft": false, "mergeStateStatus": "CLEAN",
            "statusCheckRollup": [{"__typename": "CheckRun", "name": "check", "status": "COMPLETED", "conclusion": "SUCCESS"}],
        })
    }

    /// Rewrites the fake gh's `pr view` answer for #7 with `edit` applied.
    fn view_with(view: &std::path::Path, edit: impl FnOnce(&mut Value)) {
        let mut v = view_value(7, "hive/task/TASK-001", &[("src/a.rs", 5, 2)]);
        edit(&mut v);
        std::fs::write(view, v.to_string()).unwrap();
    }

    fn view_of(number: i64, head: &str, files: &[(&str, i64, i64)]) -> PrView {
        PrView::from_json(&serde_json::from_str(&view_json(number, head, files)).unwrap())
    }

    fn calls(log: &str) -> Vec<String> {
        std::fs::read_to_string(log)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with("pr "))
            .map(|l| l.split_whitespace().take(3).collect::<Vec<_>>().join(" "))
            .collect()
    }

    fn land_pr(s: &hive_store::Store, number: i64, kind: &str, task: &str, job: &str) {
        let head = if kind == "task" {
            format!("hive/task/{task}")
        } else {
            format!("hive/worker/{job}")
        };
        let mut d = json!({
            "project": "proj", "kind": kind, "number": number, "url": format!("https://github.com/o/r/pull/{number}"),
            "head": head, "base": "main", "remote": "origin", "from": "f00", "to": "abc123",
            "jobs": [job], "commits": 1, "opened": true, "title": format!("PR {number} title"),
        });
        if !task.is_empty() {
            d["task"] = json!(task);
        }
        if !job.is_empty() {
            d["job"] = json!(job);
        }
        event(s, TEST_NOW, "daemon", "land.pr", "proj", d);
    }

    fn events(s: &hive_store::Store, action: &str) -> Vec<Value> {
        crate::util::must_rows(
            &*s.conn().unwrap(),
            "SELECT detail FROM events WHERE action = ? ORDER BY seq",
            &[&action],
        )
        .iter()
        .map(|r| serde_json::from_str(&r.str("detail")).unwrap())
        .collect()
    }

    fn jobs_of(s: &hive_store::Store, kind: &str) -> Vec<Row> {
        crate::util::must_rows(
            &*s.conn().unwrap(),
            "SELECT * FROM jobs WHERE kind = ? ORDER BY created_at, id",
            &[&kind],
        )
    }

    #[test]
    fn the_complexity_rule_names_every_reason() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        let small = view_of(
            7,
            "hive/task/TASK-001",
            &[("src/a.rs", 5, 2), ("Cargo.lock", 900, 900)],
        );
        assert!(
            reasons_for(&s, &small).is_empty(),
            "lockfile lines do not count"
        );

        let many: Vec<(String, i64, i64)> =
            (0..21).map(|i| (format!("src/f{i}.rs"), 1, 0)).collect();
        let files: Vec<(&str, i64, i64)> =
            many.iter().map(|(p, a, d)| (p.as_str(), *a, *d)).collect();
        let big = view_of(7, "hive/x/y", &files);
        assert!(
            reasons_for(&s, &big).is_empty(),
            "size alone never hands a pull request to the owner"
        );
        assert_eq!(
            care_for(&s, &big),
            ["21 files / 21 lines changed (the usual reading is 20 files / 600 lines)"]
        );

        let long = view_of(
            7,
            "hive/x/y",
            &[("src/a.rs", 400, 201), ("web/hub/out/x.js", 5000, 5000)],
        );
        assert!(reasons_for(&s, &long).is_empty());
        assert_eq!(
            care_for(&s, &long),
            ["1 files / 601 lines changed (the usual reading is 20 files / 600 lines)"]
        );
        set_setting(&s, "review.max_lines", "700");
        assert!(care_for(&s, &view_of(7, "hive/x/y", &[("src/a.rs", 400, 201)])).is_empty());

        let r = reasons_for(
            &s,
            &view_of(
                7,
                "hive/x/y",
                &[
                    (".github/workflows/ci.yml", 1, 1),
                    (".env.local", 1, 0),
                    ("ops/db_password.txt", 1, 0),
                    ("src/a.rs", 1, 0),
                ],
            ),
        );
        assert_eq!(
            r,
            [
                "touches .env.local",
                "touches .github/workflows/ci.yml",
                "touches ops/db_password.txt"
            ]
        );
        let r = reasons_for(
            &s,
            &view_of(7, "hive/x/y", &[("docs/decisions/adr-1.md", 1, 0)]),
        );
        assert_eq!(
            r,
            ["touches docs/decisions/adr-1.md"],
            "the owner's paths are guarded on a hive/ branch"
        );

        let mostly_deletes = view_of(7, "hive/x/y", &[("src/a.rs", 10, 250)]);
        assert!(reasons_for(&s, &mostly_deletes).is_empty());
        assert_eq!(
            care_for(&s, &mostly_deletes),
            ["removes more than it adds (250 lines removed, 10 added)"]
        );
        assert!(care_for(&s, &view_of(7, "hive/x/y", &[("src/a.rs", 100, 150)])).is_empty());

        let r = reasons_for(&s, &view_of(7, "feature/by-hand", &[("src/a.rs", 1, 0)]));
        assert_eq!(r, ["opened by a human/other tool (branch feature/by-hand)"]);
        assert!(
            reasons_for(&s, &view_of(7, "evolve/B-013", &[("src/a.rs", 1, 0)])).is_empty(),
            "a kept experiment's branch is Hive's own"
        );
        // A pull request opened before the rename has its head under
        // `pulse/`. It is Hive's own, with the owner's paths closed to it
        // like any agent's branch.
        for head in [
            "pulse/fjord-55/job-20261115T175900Z-cef603",
            "pulse/task-TASK-116",
        ] {
            assert!(
                reasons_for(&s, &view_of(7, head, &[("src/a.rs", 1, 0)])).is_empty(),
                "{head}"
            );
            assert_eq!(
                reasons_for(&s, &view_of(7, head, &[(".github/dependabot.yml", 1, 0)])),
                ["touches .github/dependabot.yml"],
                "{head}"
            );
        }
        let r = reasons_for(&s, &view_of(7, "pulsed/x", &[("src/a.rs", 1, 0)]));
        assert_eq!(r, ["opened by a human/other tool (branch pulsed/x)"]);

        set_setting(&s, "review.merge", "ask");
        let r = reasons_for(&s, &view_of(7, "hive/x/y", &[("src/a.rs", 1, 0)]));
        assert_eq!(r, ["the owner asked to merge every pull request"]);
    }

    #[test]
    fn a_large_pull_request_gets_a_brief_that_reads_with_care_and_may_merge() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        let pr = OpenPr {
            kind: "job".into(),
            number: 7,
            url: "https://github.com/o/r/pull/7".into(),
            title: "PR 7 title".into(),
            head: "hive/x/y".into(),
            base: "main".into(),
            ..Default::default()
        };
        let view = view_of(7, "hive/x/y", &[("data/words.json", 900, 100)]);
        let care = care_for(&s, &view);
        let brief = review_brief("proj", &pr, &view, &[], &care, "npm test");
        assert!(brief.contains("You may merge it yourself"), "{brief}");
        assert!(
            brief.contains("Read it with extra care: 1 files / 1000 lines changed"),
            "{brief}"
        );
        assert!(brief.contains("`hive review merge 7`"), "{brief}");
        assert!(!brief.contains("You may NOT merge"), "{brief}");
    }

    #[test]
    fn one_review_job_per_project_and_none_while_one_runs() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, log, _view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        land_pr(&s, 8, "job", "", "JOB-b");

        let lines = tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains("#7 queued for review")),
            "{lines:?}"
        );
        let jobs = jobs_of(&s, "review");
        assert_eq!(jobs.len(), 1, "one review job per project at a time");
        let job = &jobs[0];
        assert_eq!(job.str("source"), "review:proj:7");
        assert_eq!(job.str("title"), "Review PR #7: PR 7 title");
        assert_eq!(job.str("status"), "queued");
        assert!(crate::jobs::is_owner_driven(job));
        let brief = job.str("brief");
        assert!(brief.contains("hive review show 7"), "{brief}");
        assert!(brief.contains("hive review checkout 7"), "{brief}");
        assert!(brief.contains("hive review diff 7"), "{brief}");
        assert!(brief.contains("hive review merge 7"), "{brief}");
        assert!(brief.contains("hive review changes 7 --note"), "{brief}");
        assert!(brief.contains("hive review owner 7 --note"), "{brief}");
        assert!(
            brief.contains("hive job finish $HIVE_JOB done --result"),
            "{brief}"
        );
        assert!(brief.contains("the work of task TASK-001"), "{brief}");
        assert!(!brief.contains("You may NOT merge"), "{brief}");
        let queued = events(&s, "review.queued");
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0]["number"], 7);
        assert_eq!(queued[0]["job"], job.str("id"));
        assert_eq!(queued[0]["kind"], "task");
        assert_eq!(queued[0]["task"], "TASK-001");
        assert_eq!(queued[0]["reasons"], json!([]));
        assert_eq!(calls(&log), ["pr view 7"]);

        // Queued: nothing more is queued, nothing is asked of GitHub.
        let lines = tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert!(lines.is_empty(), "{lines:?}");
        exec1(
            &s,
            "UPDATE jobs SET status = 'leased' WHERE id = ?",
            &[&job.str("id")],
        );
        let lines = tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(jobs_of(&s, "review").len(), 1);
        assert_eq!(calls(&log), ["pr view 7"]);

        // The verdict came (merged by the agent): the next pass reviews #8.
        event(
            &s,
            TEST_NOW,
            "reviewer",
            "review.merged",
            "proj",
            json!({"project": "proj", "number": 7, "url": "u", "by": "reviewer"}),
        );
        exec1(
            &s,
            "UPDATE jobs SET status = 'done' WHERE id = ?",
            &[&job.str("id")],
        );
        let lines = tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains("#8 queued for review")),
            "{lines:?}"
        );
        let jobs = jobs_of(&s, "review");
        assert_eq!(jobs.len(), 2);
        let eight = jobs
            .iter()
            .find(|j| j.str("source") == "review:proj:8")
            .expect("a review job of #8");
        assert!(
            eight.str("brief").contains("the branch of job JOB-b"),
            "{}",
            eight.str("brief")
        );
        assert_eq!(events(&s, "review.failed").len(), 0);
    }

    #[test]
    fn a_complex_pull_request_is_reviewed_for_the_owner() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, _log, view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "experiment", "", "JOB-a");
        std::fs::write(
            &view,
            view_json(
                7,
                "hive/worker/JOB-a",
                &[("src/a.rs", 1, 0), (".github/workflows/ci.yml", 1, 0)],
            ),
        )
        .unwrap();
        let lines = tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert!(
            lines
                .iter()
                .any(|l| l.contains("for the owner: touches .github/workflows/ci.yml")),
            "{lines:?}"
        );
        let job = &jobs_of(&s, "review")[0];
        let brief = job.str("brief");
        assert!(
            brief.contains("You may NOT merge this pull request"),
            "{brief}"
        );
        assert!(
            brief.contains("- touches .github/workflows/ci.yml"),
            "{brief}"
        );
        assert!(brief.contains("the only verdict allowed"), "{brief}");
        assert!(!brief.contains("`hive review changes 7"), "{brief}");
        assert_eq!(
            events(&s, "review.queued")[0]["reasons"],
            json!(["touches .github/workflows/ci.yml"])
        );
    }

    #[test]
    fn the_owners_merge_click_merges_as_owner_once() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, log, _view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        event(
            &s,
            TEST_NOW,
            "human",
            "review.ask",
            "proj",
            json!({"project": "proj", "number": 7, "action": "merge", "task": "TASK-001"}),
        );

        let lines = tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains("#7 merged for the owner")),
            "{lines:?}"
        );
        let merged = events(&s, "review.merged");
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0]["by"], "owner");
        assert_eq!(merged[0]["number"], 7);
        assert_eq!(merged[0]["url"], "https://github.com/o/r/pull/7");
        assert_eq!(merged[0]["head_sha"], "abc123");
        assert_eq!(merged[0]["checks"], 1);
        assert!(merged[0].get("no_checks").is_none(), "{}", merged[0]);
        assert_eq!(calls(&log), ["pr view 7", "pr merge 7"]);
        assert!(
            jobs_of(&s, "review").is_empty(),
            "a merged pull request needs no review"
        );

        // Settled: the ask is not honoured twice.
        let lines = tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(calls(&log), ["pr view 7", "pr merge 7"]);
    }

    /// A merge whose head another open pull request is based on keeps the
    /// branch: GitHub closes the pull requests whose base `--delete-branch`
    /// deletes (#170 was lost so when #168 merged), so delivery moves them
    /// first and deletes the branch after. With nothing on it, the branch
    /// goes with the merge.
    #[test]
    fn a_merge_keeps_its_branch_while_a_pull_request_is_based_on_it() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, log, view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        event(
            &s,
            TEST_NOW,
            "daemon",
            "land.pr",
            "proj",
            json!({"project": "proj", "kind": "task", "task": "TASK-002", "number": 8,
                "url": "https://github.com/o/r/pull/8", "head": "hive/task/TASK-002",
                "base": "hive/task/TASK-001", "stack_on": 7, "to": "def456", "opened": true,
                "title": "PR 8 title"}),
        );
        let merge_lines = |log: &str| -> Vec<String> {
            std::fs::read_to_string(log)
                .unwrap_or_default()
                .lines()
                .filter(|l| l.starts_with("pr merge"))
                .map(str::to_string)
                .collect()
        };

        merge(&s, &proj, 7, "reviewer-1", "reviewer-1", TEST_NOW).unwrap();
        let merges = merge_lines(&log);
        assert_eq!(merges.len(), 1, "{merges:?}");
        assert!(merges[0].contains("--merge"), "{merges:?}");
        assert!(!merges[0].contains("--delete-branch"), "{merges:?}");
        let merged = events(&s, "review.merged");
        assert_eq!(merged[0]["branch_kept_for"], json!([8]), "{}", merged[0]);

        // Nothing is based on #9's head: its branch goes with the merge.
        land_pr(&s, 9, "task", "TASK-003", "JOB-c");
        view_with(&view, |v| {
            v["number"] = json!(9);
            v["url"] = json!("https://github.com/o/r/pull/9");
            v["headRefName"] = json!("hive/task/TASK-003");
        });
        merge(&s, &proj, 9, "reviewer-1", "reviewer-1", TEST_NOW).unwrap();
        let merges = merge_lines(&log);
        assert_eq!(merges.len(), 2, "{merges:?}");
        assert!(merges[1].contains("--delete-branch"), "{merges:?}");
        let merged = events(&s, "review.merged");
        assert!(merged[1].get("branch_kept_for").is_none(), "{}", merged[1]);
    }

    /// A merge whose store cannot say which pull requests are open keeps
    /// its branch: an empty answer read as "nothing is based on it" passed
    /// `--delete-branch`, and the forge closes for good whatever is stacked
    /// on that branch. The merge itself goes through and says why the
    /// branch stayed.
    #[test]
    fn a_merge_keeps_its_branch_when_the_store_cannot_say_what_is_open() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, log, _view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        PR_STATES_UNREAD.set(true);
        let merged = merge(&s, &proj, 7, "reviewer-1", "reviewer-1", TEST_NOW);
        PR_STATES_UNREAD.set(false);
        merged.unwrap();
        let merges: Vec<String> = std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with("pr merge"))
            .map(str::to_string)
            .collect();
        assert_eq!(merges.len(), 1, "{merges:?}");
        assert!(!merges[0].contains("--delete-branch"), "{merges:?}");
        let merged = events(&s, "review.merged");
        assert!(
            merged[0]["branch_kept"]
                .as_str()
                .is_some_and(|w| w.contains("could not be read")),
            "{}",
            merged[0]
        );
    }

    /// A pull request stacked on the one that merges keeps that branch
    /// whatever its own review said: under a "changes" verdict (its fix job
    /// running) or handed to the owner it is out of the review queue but
    /// still open on the forge, and `--delete-branch` would close it for
    /// good (#267 closed so when #266 merged on 26 November).
    #[test]
    fn a_merge_keeps_its_branch_for_a_stacked_pull_request_under_changes_or_with_the_owner() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, log, view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        let merge_lines = |log: &str| -> Vec<String> {
            std::fs::read_to_string(log)
                .unwrap_or_default()
                .lines()
                .filter(|l| l.starts_with("pr merge"))
                .map(str::to_string)
                .collect()
        };
        for (i, verdict) in ["review.changes", "review.owner"].into_iter().enumerate() {
            let below = 7 + 10 * i as i64;
            let above = below + 1;
            land_pr(&s, below, "task", "TASK-001", "JOB-a");
            view_with(&view, |v| {
                v["number"] = json!(below);
                v["url"] = json!(format!("https://github.com/o/r/pull/{below}"));
            });
            event(
                &s,
                TEST_NOW,
                "daemon",
                "land.pr",
                "proj",
                json!({"project": "proj", "kind": "task", "task": "TASK-002", "number": above,
                    "url": format!("https://github.com/o/r/pull/{above}"), "head": "hive/task/TASK-002",
                    "base": "hive/task/TASK-001", "stack_on": below, "to": "def456", "opened": true,
                    "title": "stacked"}),
            );
            event(
                &s,
                TEST_NOW,
                "reviewer-1",
                verdict,
                "proj",
                json!({"project": "proj", "number": above, "note": "fix it"}),
            );
            assert!(
                open_prs(&s.conn().unwrap(), "proj")
                    .iter()
                    .all(|p| p.number != above),
                "out of the review queue under {verdict}"
            );

            merge(&s, &proj, below, "owner", "owner", TEST_NOW).unwrap();
            let merges = merge_lines(&log);
            assert_eq!(merges.len(), i + 1, "{merges:?}");
            assert!(
                !merges[i].contains("--delete-branch"),
                "{verdict}: {merges:?}"
            );
            let merged = events(&s, "review.merged");
            assert_eq!(merged[i]["branch_kept_for"], json!([above]), "{verdict}");
            // The one below is merged now; the next round starts afresh.
            event(
                &s,
                TEST_NOW,
                "daemon",
                "land.merged",
                "proj",
                json!({"project": "proj", "number": above}),
            );
        }
    }

    /// The owner's click, like the reviewer's verdict, goes through the
    /// forge's gate: a red or running check, a draft, a blocked merge state
    /// or a head that moved is refused without a `review.failed`, and the
    /// ask stands for the pass that finds the pull request ready.
    #[test]
    fn the_owners_merge_click_waits_for_green_checks_and_the_shown_head() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, log, view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        event(
            &s,
            TEST_NOW,
            "human",
            "review.ask",
            "proj",
            json!({"project": "proj", "number": 7, "action": "merge", "task": "TASK-001"}),
        );
        // Before any review was queued, the head the hub showed is the one
        // the newest land.pr pushed; the forge holds another commit now.
        view_with(&view, |v| v["headRefOid"] = json!("def456def456def456"));
        assert_eq!(
            merge_as_owner(&s, &proj, 7, "", TEST_NOW).unwrap_err(),
            "the pull request moved since Hive pushed it: pushed abc123, now def456def456; it is reviewed again"
        );
        assert!(events(&s, "review.failed").is_empty());
        let refused = |why: &str| {
            let lines = tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW);
            let line = lines
                .iter()
                .find(|l| l.contains("#7 could not be merged for the owner"))
                .unwrap_or_else(|| panic!("{lines:?}"));
            assert!(line.contains(why), "{line}");
            assert!(events(&s, "review.merged").is_empty());
            assert!(
                events(&s, "review.failed").is_empty(),
                "a refusal is no failure"
            );
            assert!(
                !calls(&log).iter().any(|c| c.starts_with("pr merge")),
                "{:?}",
                calls(&log)
            );
        };

        view_with(&view, |v| {
            v["statusCheckRollup"] = json!([
                {"__typename": "CheckRun", "name": "check", "status": "COMPLETED", "conclusion": "FAILURE"},
                {"__typename": "CheckRun", "name": "lint", "status": "IN_PROGRESS", "conclusion": null}
            ]);
        });
        refused("pull request #7 has failing checks: check; a red check is never merged");

        view_with(&view, |v| {
            v["statusCheckRollup"] = json!([
                {"__typename": "CheckRun", "name": "check", "status": "QUEUED", "conclusion": null},
                {"__typename": "StatusContext", "context": "ci/slow", "state": "PENDING"}
            ]);
        });
        refused(
            "pull request #7 has checks pending: check, ci/slow; it is merged when they are green",
        );

        view_with(&view, |v| v["isDraft"] = json!(true));
        refused("pull request #7 is a draft");

        view_with(&view, |v| v["mergeStateStatus"] = json!("BLOCKED"));
        refused("pull request #7 is blocked on GitHub (mergeStateStatus BLOCKED");

        view_with(&view, |v| v["mergeStateStatus"] = json!("DIRTY"));
        refused("pull request #7 conflicts with its base (mergeStateStatus DIRTY)");

        view_with(&view, |v| v["mergeStateStatus"] = json!("BEHIND"));
        refused("pull request #7 is behind its base (mergeStateStatus BEHIND)");

        // The pass queued a review at abc123; the forge holds another commit
        // now: the refusal names the review.
        assert_eq!(events(&s, "review.queued")[0]["sha"], "abc123");
        view_with(&view, |v| v["headRefOid"] = json!("def456def456def456"));
        refused(
            "the pull request moved since it was reviewed: reviewed abc123, now def456def456; it is reviewed again",
        );

        // Green, clean and at the head the owner saw: merged, the ask honoured.
        view_with(&view, |_| {});
        let lines = tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains("#7 merged for the owner")),
            "{lines:?}"
        );
        assert_eq!(events(&s, "review.merged").len(), 1);
        assert_eq!(
            calls(&log)
                .iter()
                .filter(|c| c.starts_with("pr merge"))
                .count(),
            1
        );
    }

    /// A pull request with no check at all merges: the project may have no
    /// CI. When it has workflows, the `review.merged` says none reported.
    #[test]
    fn a_merge_with_no_checks_reported_is_allowed_and_noted_when_the_project_has_workflows() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, _log, view) = fake_gh();
        let s = new_store();
        let (proj, repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "job", "", "JOB-a");
        view_with(&view, |v| v["statusCheckRollup"] = json!([]));
        merge(&s, &proj, 7, "reviewer", "reviewer", TEST_NOW).unwrap();
        let merged = events(&s, "review.merged");
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0]["checks"], 0);
        assert!(
            merged[0].get("no_checks").is_none(),
            "no workflows, no note: {}",
            merged[0]
        );

        land_pr(&s, 8, "job", "", "JOB-b");
        let wf = std::path::Path::new(&repo).join(".github/workflows");
        std::fs::create_dir_all(&wf).unwrap();
        std::fs::write(wf.join("ci.yml"), "name: ci\n").unwrap();
        view_with(&view, |v| {
            v["number"] = json!(8);
            v["statusCheckRollup"] = json!([]);
        });
        merge(&s, &proj, 8, "reviewer", "reviewer", TEST_NOW).unwrap();
        let merged = events(&s, "review.merged");
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[1]["no_checks"], true);
        assert!(
            merged[1]["note"]
                .as_str()
                .unwrap()
                .contains("merged with no checks reported"),
            "{}",
            merged[1]
        );

        // pipeline.watch off: the project watches no CI, no note.
        land_pr(&s, 9, "job", "", "JOB-c");
        s.set_setting("owner", crate::pipeline::WATCH, "off")
            .unwrap();
        view_with(&view, |v| {
            v["number"] = json!(9);
            v["statusCheckRollup"] = json!([]);
        });
        merge(&s, &proj, 9, "reviewer", "reviewer", TEST_NOW).unwrap();
        let merged = events(&s, "review.merged");
        assert_eq!(merged.len(), 3);
        assert!(merged[2].get("no_checks").is_none(), "{}", merged[2]);
    }

    #[test]
    fn a_failed_owner_merge_is_recorded_and_not_retried() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (bin, _path, log, _view) = fake_gh();
        std::fs::write(bin.path().join("fail"), "merge\n").unwrap();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "job", "", "JOB-a");
        event(
            &s,
            TEST_NOW,
            "human",
            "review.ask",
            "proj",
            json!({"project": "proj", "number": 7, "action": "merge"}),
        );
        let lines = tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains(
                "#7 could not be merged for the owner: gh pr merge failed: HTTP 422: boom"
            )),
            "{lines:?}"
        );
        let failed = events(&s, "review.failed");
        assert_eq!(failed.len(), 1);
        assert!(failed[0]["error"].as_str().unwrap().contains("HTTP 422"));
        // The pull request is still open, so the pass queues its review.
        assert_eq!(jobs_of(&s, "review").len(), 1);
        let lines = tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(
            calls(&log)
                .iter()
                .filter(|c| c.starts_with("pr merge"))
                .count(),
            1
        );
    }

    /// TASK-147: a review job that cannot check the pull request out ends
    /// with its own verdict, `review.again` (`checkout`): the next pass
    /// queues a fresh review of the same head, and however often that
    /// happens the pull request is never handed to the owner for it.
    #[test]
    fn a_review_that_cannot_check_out_queues_the_review_again_never_the_owner() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, _log, _view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "job", "", "JOB-a");
        let tend = |s: &hive_store::Store| tend_reviews(s, std::slice::from_ref(&proj), TEST_NOW);
        let view = view_of(7, "hive/worker/JOB-a", &[("src/a.rs", 5, 2)]);

        // Outside the pull request's review job nothing is recorded.
        assert!(!requeue_after_checkout(
            &s, "proj", &view, "boom", "xenon", TEST_NOW
        ));
        for round in 1..=3 {
            tend(&s);
            let jobs = jobs_of(&s, "review");
            assert_eq!(jobs.len(), round, "one fresh review per round");
            let job = jobs
                .iter()
                .find(|j| j.str("status") == "queued")
                .expect("a queued review")
                .str("id");
            {
                let _job = hive_testutil::setenv("HIVE_JOB", &job);
                assert!(requeue_after_checkout(
                    &s,
                    "proj",
                    &view,
                    "fetch of pull/7/head failed: refused (ADR-041 D4)",
                    "xenon",
                    TEST_NOW
                ));
                let conn = s.conn().unwrap();
                assert!(requeued_by(&conn, "proj", 7, &job));
            }
            exec1(
                &s,
                "UPDATE jobs SET status = 'done', result = 'SUMMARY: could not check out' WHERE id = ?",
                &[&job],
            );
            let lines = tend(&s);
            assert!(
                lines.iter().any(|l| l.contains("#7 queued for review")),
                "round {round}: {lines:?}"
            );
        }
        let again = events(&s, AGAIN_ACTION);
        assert_eq!(again.len(), 3);
        assert_eq!(again[0]["checkout"], true);
        assert_eq!(again[0]["head_sha"], "abc123");
        assert!(
            again[0]["note"]
                .as_str()
                .unwrap()
                .contains("could not check out"),
            "{again:?}"
        );
        assert!(events(&s, "review.failed").is_empty());
        assert!(events(&s, "review.owner").is_empty());
        assert_eq!(jobs_of(&s, "review").len(), 4, "a fourth review is queued");
    }

    #[test]
    fn a_review_without_a_verdict_fails_is_retried_once_then_goes_to_the_owner() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, _log, _view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        let task =
            hive_tasks::new(&s, "proj", "Add the thing", "b", 0, hive_store::HUMAN_AGENT).unwrap();
        land_pr(&s, 7, "task", &task.id, "JOB-a");
        let tend = |s: &hive_store::Store| tend_reviews(s, std::slice::from_ref(&proj), TEST_NOW);

        tend(&s);
        let first = jobs_of(&s, "review")[0].str("id");
        exec1(
            &s,
            "UPDATE jobs SET status = 'done', result = 'SUMMARY: ran out of steps' WHERE id = ?",
            &[&first],
        );
        let lines = tend(&s);
        assert!(
            lines
                .iter()
                .any(|l| l.contains(&format!("#7 review {first} ended done without a verdict"))),
            "{lines:?}"
        );
        let failed = events(&s, "review.failed");
        assert_eq!(failed.len(), 1);
        assert!(
            failed[0]["error"]
                .as_str()
                .unwrap()
                .contains("ran out of steps"),
            "{failed:?}"
        );
        assert_eq!(
            jobs_of(&s, "review").len(),
            1,
            "the next pass re-queues, not this one"
        );

        let lines = tend(&s);
        assert!(
            lines.iter().any(|l| l.contains("#7 queued for review")),
            "{lines:?}"
        );
        let jobs = jobs_of(&s, "review");
        assert_eq!(jobs.len(), 2);
        // Both jobs carry the test clock, so their order is the ids' and
        // the new one is whichever is not the first.
        let second = jobs
            .iter()
            .map(|j| j.str("id"))
            .find(|id| *id != first)
            .expect("a second review job");
        assert_eq!(events(&s, "review.queued").len(), 2);
        // Still running: the failed event is not repeated.
        exec1(
            &s,
            "UPDATE jobs SET status = 'leased' WHERE id = ?",
            &[&second],
        );
        assert!(tend(&s).is_empty());
        assert_eq!(events(&s, "review.failed").len(), 1);

        exec1(
            &s,
            "UPDATE jobs SET status = 'failed' WHERE id = ?",
            &[&second],
        );
        let lines = tend(&s);
        assert!(
            lines
                .iter()
                .any(|l| l.contains("ended failed without a verdict")),
            "{lines:?}"
        );
        assert_eq!(events(&s, "review.failed").len(), 2);
        let lines = tend(&s);
        assert!(
            lines
                .iter()
                .any(|l| l.contains(&format!("#7 handed to the owner: {FAILED_TWICE}"))),
            "{lines:?}"
        );
        let owner = events(&s, "review.owner");
        assert_eq!(owner.len(), 1);
        assert_eq!(owner[0]["reasons"], json!([FAILED_TWICE]));
        assert_eq!(owner[0]["number"], 7);
        assert_eq!(jobs_of(&s, "review").len(), 2, "no third attempt");
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM posts WHERE kind = 'question' AND json_extract(extra, '$.for_owner') = 1 AND summary LIKE 'Merge pull request #7%'",
                &[]
            ),
            1
        );
        let msgs = hive_tasks::messages(&s, "proj", &task.id, "", "").unwrap();
        assert!(
            msgs.iter().any(|m| m.body.contains("waits for your merge")),
            "{msgs:?}"
        );
        // Settled: nothing more.
        assert!(tend(&s).is_empty());
        assert_eq!(events(&s, "review.owner").len(), 1);
    }

    /// Queues the review of #7 and ends its job, as a reviewer that gave
    /// its verdict and finished does; returns the job's id.
    fn reviewed(s: &hive_store::Store, proj: &Row) -> String {
        tend_reviews(s, std::slice::from_ref(proj), TEST_NOW);
        let job = jobs_of(s, "review")[0].str("id");
        exec1(
            s,
            "UPDATE jobs SET status = 'done', result = 'SUMMARY: reviewed #7: merge' WHERE id = ?",
            &[&job],
        );
        job
    }

    fn merges(log: &str) -> usize {
        calls(log)
            .iter()
            .filter(|c| c.starts_with("pr merge"))
            .count()
    }

    #[test]
    fn an_approved_merge_the_forge_refuses_with_5xx_is_retried_on_the_back_off() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (bin, _path, log, _view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        let task =
            hive_tasks::new(&s, "proj", "Add the thing", "b", 0, hive_store::HUMAN_AGENT).unwrap();
        land_pr(&s, 7, "task", &task.id, "JOB-a");
        reviewed(&s, &proj);
        std::fs::write(bin.path().join("fail"), "merge\n").unwrap();
        std::fs::write(
            bin.path().join("failmsg"),
            "HTTP 502: Bad Gateway (https://api.github.com/graphql)",
        )
        .unwrap();
        let tend = |at: &str| tend_reviews(&s, std::slice::from_ref(&proj), at);

        // The reviewer's verdict: GitHub answers 502, the verdict stands.
        let v = merge_or_wait(&s, &proj, 7, None, "reviewer-1", "reviewer-1", TEST_NOW).unwrap();
        let Verdict::Waits(words) = v else {
            panic!("{v:?}")
        };
        assert!(
            words.contains("GitHub refused the merge (HTTP 502"),
            "{words}"
        );
        assert!(
            words.contains("retried at 2026-09-06T10:05:00Z (attempt 2)"),
            "{words}"
        );
        assert!(
            events(&s, "review.failed").is_empty(),
            "a passing refusal is no failure"
        );
        let wait = events(&s, WAIT_ACTION);
        assert_eq!(wait.len(), 1);
        assert_eq!(wait[0]["waiting"], "forge");
        assert_eq!(wait[0]["due"], "2026-09-06T10:05:00Z");
        assert_eq!(merges(&log), 1);

        // Inside the back-off: no merge, no review, no owner; the daemon log
        // says once what is waited for.
        let lines = tend(TEST_NOW);
        assert_eq!(
            lines,
            [format!("review proj: #7 {words}")],
            "one line for the wait"
        );
        assert!(
            tend(&crate::util::plus_minutes(TEST_NOW, 2)).is_empty(),
            "said once"
        );
        assert_eq!(merges(&log), 1);

        // Due: the second 502 waits for the next step, 15 minutes on.
        let at = crate::util::plus_minutes(TEST_NOW, 5);
        let lines = tend(&at);
        assert_eq!(merges(&log), 2);
        assert!(
            lines
                .iter()
                .any(|l| l.contains("retried at 2026-09-06T10:20:00Z (attempt 3)")),
            "{lines:?}"
        );
        assert!(tend(&crate::util::plus_minutes(TEST_NOW, 19)).is_empty());
        assert_eq!(merges(&log), 2);

        // GitHub answers again: merged, with the one review run it had.
        std::fs::write(bin.path().join("fail"), "").unwrap();
        let lines = tend(&crate::util::plus_minutes(TEST_NOW, 20));
        assert!(
            lines
                .iter()
                .any(|l| l.contains("#7 merged after waiting (2 refused merges)")),
            "{lines:?}"
        );
        assert_eq!(merges(&log), 3);
        let merged = events(&s, "review.merged");
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0]["by"], "reviewer-1");
        assert_eq!(jobs_of(&s, "review").len(), 1, "no second review run");
        assert!(
            events(&s, "review.owner").is_empty(),
            "the owner was not asked"
        );
        assert!(events(&s, "review.failed").is_empty());
        let msgs = hive_tasks::messages(&s, "proj", &task.id, "", "").unwrap();
        assert!(
            !msgs.iter().any(|m| m.body.contains("waits for your merge")),
            "{msgs:?}"
        );
        let pr = hive_tasks::pr_states_for(&s, "proj")
            .unwrap()
            .into_iter()
            .find(|p| p.number == 7)
            .unwrap();
        assert_eq!(pr.state, "merged");
        assert!(tend(&crate::util::plus_minutes(TEST_NOW, 400)).is_empty());
        assert_eq!(merges(&log), 3);
    }

    #[test]
    fn an_approved_merge_waits_for_a_pending_check_and_merges_when_it_passes() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, log, view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        reviewed(&s, &proj);
        view_with(&view, |v| {
            v["statusCheckRollup"] = json!([
                {"__typename": "CheckRun", "name": "check", "status": "COMPLETED", "conclusion": "SUCCESS"},
                {"__typename": "CheckRun", "name": "ci/build", "status": "IN_PROGRESS", "conclusion": null}
            ]);
        });
        let tend = |at: &str| tend_reviews(&s, std::slice::from_ref(&proj), at);

        let v = merge_or_wait(&s, &proj, 7, None, "reviewer-1", "reviewer-1", TEST_NOW).unwrap();
        assert_eq!(
            v,
            Verdict::Waits(
                "approved; the merge waits for the checks still running (ci/build)".into()
            )
        );
        assert_eq!(merges(&log), 0, "a running check is not merged");
        let wait = events(&s, WAIT_ACTION);
        assert_eq!(wait[0]["waiting"], "checks");
        assert_eq!(wait[0]["due"], "", "checks are read on every pass");
        let pr = hive_tasks::pr_states_for(&s, "proj")
            .unwrap()
            .into_iter()
            .find(|p| p.number == 7)
            .unwrap();
        assert_eq!(pr.state, "merge_waiting");
        assert_eq!(
            pr.note,
            "approved; the merge waits for the checks still running (ci/build)"
        );
        assert!(pr.can_merge, "the owner may merge it now");

        // Still running: one log line, then quiet; nothing new recorded.
        let lines = tend(TEST_NOW);
        assert_eq!(
            lines,
            ["review proj: #7 approved; the merge waits for the checks still running (ci/build)"]
        );
        assert!(tend(&crate::util::plus_minutes(TEST_NOW, 1)).is_empty());
        assert_eq!(events(&s, WAIT_ACTION).len(), 1);
        assert_eq!(merges(&log), 0);

        // The check passed: merged on the next pass, without a review.
        view_with(&view, |_| {});
        let lines = tend(&crate::util::plus_minutes(TEST_NOW, 2));
        assert!(
            lines
                .iter()
                .any(|l| l.contains("#7 merged after waiting (for its checks)")),
            "{lines:?}"
        );
        assert_eq!(merges(&log), 1);
        assert_eq!(events(&s, "review.merged").len(), 1);
        assert_eq!(jobs_of(&s, "review").len(), 1);
        assert!(events(&s, "review.owner").is_empty());
    }

    #[test]
    fn an_approved_merge_that_meets_a_conflict_goes_to_the_owner() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (bin, _path, log, view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        reviewed(&s, &proj);
        let tend = |at: &str| tend_reviews(&s, std::slice::from_ref(&proj), at);

        // A conflict at the verdict is refused as it always was: no wait.
        view_with(&view, |v| v["mergeStateStatus"] = json!("DIRTY"));
        let e =
            merge_or_wait(&s, &proj, 7, None, "reviewer-1", "reviewer-1", TEST_NOW).unwrap_err();
        assert!(e.contains("conflicts with its base"), "{e}");
        assert!(events(&s, WAIT_ACTION).is_empty());
        // A merge gh refuses for a reason that does not pass fails as before.
        view_with(&view, |_| {});
        std::fs::write(bin.path().join("fail"), "merge\n").unwrap();
        std::fs::write(
            bin.path().join("failmsg"),
            "GraphQL: Pull Request is not mergeable (mergePullRequest)",
        )
        .unwrap();
        let e =
            merge_or_wait(&s, &proj, 7, None, "reviewer-1", "reviewer-1", TEST_NOW).unwrap_err();
        assert!(e.contains("not mergeable"), "{e}");
        assert_eq!(events(&s, "review.failed").len(), 1);
        assert!(events(&s, WAIT_ACTION).is_empty());

        // A 503 waits; by the time it is due the base moved into a conflict:
        // the pull request goes to the owner with the reason.
        std::fs::write(bin.path().join("failmsg"), "HTTP 503: Service Unavailable").unwrap();
        let v = merge_or_wait(&s, &proj, 7, None, "reviewer-1", "reviewer-1", TEST_NOW).unwrap();
        assert!(matches!(v, Verdict::Waits(_)), "{v:?}");
        std::fs::write(bin.path().join("fail"), "").unwrap();
        view_with(&view, |v| v["mergeStateStatus"] = json!("DIRTY"));
        let lines = tend(&crate::util::plus_minutes(TEST_NOW, 5));
        assert!(
            lines.iter().any(|l| l.contains(
                "#7 handed to the owner: the approved merge cannot go through: pull request #7 conflicts with its base"
            )),
            "{lines:?}"
        );
        let owner = events(&s, "review.owner");
        assert_eq!(owner.len(), 1);
        assert!(
            owner[0]["reasons"][0]
                .as_str()
                .unwrap()
                .contains("conflicts with its base"),
            "{owner:?}"
        );
        assert!(
            owner[0]["note"]
                .as_str()
                .unwrap()
                .contains("reviewer-1 approved pull request #7"),
            "{owner:?}"
        );
        assert_eq!(merges(&log), 2, "the conflict was not sent to gh");
        // Settled with the owner: no retry, no new review.
        assert!(tend(&crate::util::plus_minutes(TEST_NOW, 500)).is_empty());
        assert_eq!(jobs_of(&s, "review").len(), 1);
    }

    /// A reviewer's verdict on #7 that waits for a running check.
    fn waiting_for_checks(s: &hive_store::Store, proj: &Row, view: &std::path::Path) {
        view_with(view, |v| {
            v["statusCheckRollup"] = json!([
                {"__typename": "CheckRun", "name": "ci/build", "status": "IN_PROGRESS", "conclusion": ""},
            ]);
        });
        let v = merge_or_wait(s, proj, 7, None, "reviewer-1", "reviewer-1", TEST_NOW).unwrap();
        assert!(matches!(v, Verdict::Waits(_)), "{v:?}");
    }

    /// A reviewer's verdict on #7 that waits because GitHub answered 502.
    fn waiting_for_forge(s: &hive_store::Store, proj: &Row, bin: &std::path::Path) {
        std::fs::write(bin.join("fail"), "merge\n").unwrap();
        std::fs::write(bin.join("failmsg"), "HTTP 502: Bad Gateway").unwrap();
        let v = merge_or_wait(s, proj, 7, None, "reviewer-1", "reviewer-1", TEST_NOW).unwrap();
        assert!(matches!(v, Verdict::Waits(_)), "{v:?}");
        std::fs::write(bin.join("fail"), "").unwrap();
    }

    fn lead_posts(s: &hive_store::Store) -> Vec<Row> {
        crate::util::query_rows(
            &s.conn().unwrap(),
            "SELECT * FROM posts WHERE project = 'proj' AND area = 'runner' AND thread = 'review' AND kind = 'finding'",
            &[],
        )
        .unwrap()
    }

    #[test]
    fn a_pending_check_that_then_fails_is_not_merged_and_goes_to_the_owner() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, log, view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        reviewed(&s, &proj);
        waiting_for_checks(&s, &proj, &view);
        view_with(&view, |v| {
            v["statusCheckRollup"] = json!([
                {"__typename": "CheckRun", "name": "ci/build", "status": "COMPLETED", "conclusion": "FAILURE"},
            ]);
        });
        let lines = tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert!(
            lines
                .iter()
                .any(|l| l.contains("#7 handed to the owner: the approved merge cannot go through")),
            "{lines:?}"
        );
        assert_eq!(merges(&log), 0, "a failed check is never merged");
        assert_eq!(events(&s, "review.owner").len(), 1);
        assert!(
            tend_reviews(
                &s,
                std::slice::from_ref(&proj),
                &crate::util::plus_minutes(TEST_NOW, 600)
            )
            .is_empty()
        );
        assert_eq!(merges(&log), 0);
    }

    #[test]
    fn a_head_that_moved_while_the_merge_waited_is_reviewed_again() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, log, view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        reviewed(&s, &proj);
        waiting_for_checks(&s, &proj, &view);
        // Someone pushed to the branch on GitHub; its checks are green.
        view_with(&view, |v| v["headRefOid"] = json!("def456"));
        let lines = tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains("#7 not merged")),
            "{lines:?}"
        );
        assert_eq!(merges(&log), 0, "a head nobody reviewed is never merged");
        let again = events(&s, AGAIN_ACTION);
        assert_eq!(again.len(), 1);
        assert_eq!(again[0]["head_sha"], "def456");
        assert!(events(&s, "review.owner").is_empty());
        let pr = hive_tasks::pr_states_for(&s, "proj")
            .unwrap()
            .into_iter()
            .find(|p| p.number == 7)
            .unwrap();
        // Open for review again, and the same pass queues the review of
        // the new head (the old head's review job is done).
        assert_eq!(pr.state, "in_review");
        assert!(
            lines.iter().any(|l| l.contains("#7 queued for review")),
            "{lines:?}"
        );
        let reviews = jobs_of(&s, "review");
        assert_eq!(reviews.len(), 2, "a new review of the new head");
        assert!(events(&s, WAIT_ACTION).len() == 1, "the wait ended");
        assert!(
            tend_reviews(
                &s,
                std::slice::from_ref(&proj),
                &crate::util::plus_minutes(TEST_NOW, 5)
            )
            .iter()
            .all(|l| !l.contains("merged"))
        );
        assert_eq!(merges(&log), 0);
    }

    #[test]
    fn a_new_land_pr_while_the_merge_waits_ends_the_wait() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (bin, _path, log, _view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        reviewed(&s, &proj);
        waiting_for_forge(&s, &proj, bin.path());
        assert_eq!(merges(&log), 1);
        // Delivery pushed more work onto the pull request.
        land_pr(&s, 7, "task", "TASK-001", "JOB-b");
        let lines = tend_reviews(
            &s,
            std::slice::from_ref(&proj),
            &crate::util::plus_minutes(TEST_NOW, 10),
        );
        assert_eq!(merges(&log), 1, "the old verdict merges nothing: {lines:?}");
        assert!(events(&s, "review.owner").is_empty());
        assert!(lead_posts(&s).is_empty());
        assert!(
            lines.iter().any(|l| l.contains("#7 queued for review")),
            "{lines:?}"
        );
        assert_eq!(jobs_of(&s, "review").len(), 2);
    }

    #[test]
    fn a_failed_read_at_a_retry_waits_only_when_it_passes() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (bin, _path, log, _view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        reviewed(&s, &proj);
        waiting_for_forge(&s, &proj, bin.path());
        let tend = |at: &str| tend_reviews(&s, std::slice::from_ref(&proj), at);

        // Due at +5: the read answers 503, the wait goes on to its next step.
        std::fs::write(bin.path().join("fail"), "view\n").unwrap();
        std::fs::write(bin.path().join("failmsg"), "HTTP 503: Service Unavailable").unwrap();
        let lines = tend(&crate::util::plus_minutes(TEST_NOW, 5));
        assert!(lines.iter().any(|l| l.contains("(attempt 3)")), "{lines:?}");
        assert_eq!(events(&s, WAIT_ACTION).len(), 2);
        assert!(events(&s, "review.owner").is_empty());

        // Due again: a 404 does not pass - the owner has it, with the error.
        std::fs::write(
            bin.path().join("failmsg"),
            "GraphQL: Could not resolve to a PullRequest with the number of 7. (HTTP 404)",
        )
        .unwrap();
        let lines = tend(&crate::util::plus_minutes(TEST_NOW, 20));
        assert!(
            lines
                .iter()
                .any(|l| l.contains("#7 handed to the owner: the approved merge cannot be read")),
            "{lines:?}"
        );
        let owner = events(&s, "review.owner");
        assert_eq!(owner.len(), 1);
        assert!(
            owner[0]["reasons"][0]
                .as_str()
                .unwrap()
                .contains("Could not resolve"),
            "{owner:?}"
        );
        assert_eq!(merges(&log), 1, "only the first, refused, merge");
        assert!(tend(&crate::util::plus_minutes(TEST_NOW, 600)).is_empty());
    }

    #[test]
    fn a_pull_request_merged_or_closed_on_the_forge_ends_the_wait() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        for (state, ended) in [("MERGED", "land.merged"), ("CLOSED", "land.pr-closed")] {
            let (bin, _path, log, view) = fake_gh();
            let s = new_store();
            let (proj, _repo, _bare) = push_fixture(&s);
            land_pr(&s, 7, "task", "TASK-001", "JOB-a");
            reviewed(&s, &proj);
            waiting_for_forge(&s, &proj, bin.path());
            view_with(&view, |v| v["state"] = json!(state));
            let lines = tend_reviews(
                &s,
                std::slice::from_ref(&proj),
                &crate::util::plus_minutes(TEST_NOW, 5),
            );
            assert!(
                !lines
                    .iter()
                    .any(|l| l.contains("merged after") || l.contains("handed to the owner")),
                "{state}: {lines:?}"
            );
            assert_eq!(merges(&log), 1, "{state}: no second merge");
            assert!(events(&s, "review.owner").is_empty(), "{state}");
            // The landing poll records it; the wait is over for good.
            event(
                &s,
                TEST_NOW,
                "daemon",
                ended,
                "proj",
                json!({"project": "proj", "kind": "task", "task": "TASK-001", "number": 7}),
            );
            assert!(
                tend_reviews(
                    &s,
                    std::slice::from_ref(&proj),
                    &crate::util::plus_minutes(TEST_NOW, 600)
                )
                .is_empty(),
                "{state}"
            );
            assert_eq!(merges(&log), 1, "{state}");
            assert!(lead_posts(&s).is_empty(), "{state}");
        }
    }

    #[test]
    fn a_pull_request_stacked_on_a_waiting_one_waits_and_follows_its_bound() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (bin, _path, log, _view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        set_setting(&s, WAIT_TRIES, "2");
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        reviewed(&s, &proj);
        waiting_for_forge(&s, &proj, bin.path());
        event(
            &s,
            TEST_NOW,
            "daemon",
            "land.pr",
            "proj",
            json!({"project": "proj", "kind": "job", "job": "JOB-b", "number": 8,
                "url": "https://github.com/o/r/pull/8", "head": "hive/worker/JOB-b",
                "base": "hive/task/TASK-001", "stack_on": 7, "to": "abc123", "opened": true,
                "title": "PR 8 title"}),
        );
        let lines = tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert!(
            lines
                .iter()
                .any(|l| l == "review proj: #8 stacked on #7: reviewed after #7 merges"),
            "{lines:?}"
        );
        assert_eq!(jobs_of(&s, "review").len(), 1, "#8 is not reviewed yet");
        // #7's second refusal is its bound: #7 is the owner's, the lead is
        // told, and #8 still waits below it - never merged around it.
        std::fs::write(bin.path().join("fail"), "merge\n").unwrap();
        let lines = tend_reviews(
            &s,
            std::slice::from_ref(&proj),
            &crate::util::plus_minutes(TEST_NOW, 5),
        );
        assert!(
            lines
                .iter()
                .any(|l| l.contains("#7 handed to the owner, the lead told")),
            "{lines:?}"
        );
        assert_eq!(lead_posts(&s).len(), 1);
        assert!(
            lead_posts(&s)[0]
                .str("body")
                .contains("a pull request stacked on it waits until it merges"),
        );
        assert!(
            !calls(&log).iter().any(|c| c == "pr merge 8"),
            "{:?}",
            calls(&log)
        );
        assert_eq!(jobs_of(&s, "review").len(), 1);
    }

    #[test]
    fn a_bound_of_one_ends_the_wait_at_the_next_pass_and_tells_the_lead_once() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (bin, _path, log, _view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        set_setting(&s, WAIT_TRIES, "1");
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        reviewed(&s, &proj);
        waiting_for_forge(&s, &proj, bin.path());
        // Not even due yet: one failure is the whole bound.
        let lines = tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains(
                "#7 handed to the owner, the lead told: the forge failed the approved merge 1 time(s) (review.merge_wait_tries = 1); the forge's last answer: HTTP 502: Bad Gateway"
            )),
            "{lines:?}"
        );
        assert_eq!(merges(&log), 1, "no second try");
        let owner = events(&s, "review.owner");
        assert_eq!(owner.len(), 1);
        let posts = lead_posts(&s);
        assert_eq!(posts.len(), 1);
        assert!(posts[0].str("body").starts_with("@proj-lead:"), "{posts:?}");
        assert!(
            posts[0].str("body").contains("HTTP 502: Bad Gateway"),
            "{posts:?}"
        );
        let pr = hive_tasks::pr_states_for(&s, "proj")
            .unwrap()
            .into_iter()
            .find(|p| p.number == 7)
            .unwrap();
        assert_eq!(pr.state, "needs_owner");
        assert!(pr.can_merge);
        // Told once.
        assert!(
            tend_reviews(
                &s,
                std::slice::from_ref(&proj),
                &crate::util::plus_minutes(TEST_NOW, 600)
            )
            .is_empty()
        );
        assert_eq!(lead_posts(&s).len(), 1);
        assert_eq!(merges(&log), 1);
    }

    /// A forge wait found past its hour bound with tries left (the machine
    /// slept, the daemon was down) tries the merge once more before it
    /// goes to the owner: merged when the forge takes it, handed over with
    /// the hour bound and the forge's answer when it fails again.
    #[test]
    fn a_forge_wait_past_its_hours_tries_the_merge_once_more() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let late = crate::util::plus_minutes(TEST_NOW, 7 * 60);
        for forge_fails in [false, true] {
            let (bin, _path, log, _view) = fake_gh();
            let s = new_store();
            let (proj, _repo, _bare) = push_fixture(&s);
            land_pr(&s, 7, "task", "TASK-001", "JOB-a");
            reviewed(&s, &proj);
            waiting_for_forge(&s, &proj, bin.path());
            if forge_fails {
                std::fs::write(bin.path().join("fail"), "merge\n").unwrap();
            }
            let lines = tend_reviews(&s, std::slice::from_ref(&proj), &late);
            assert_eq!(merges(&log), 2, "one last try ({forge_fails}): {lines:?}");
            if forge_fails {
                assert!(
                    lines.iter().any(|l| l.contains(
                        "#7 handed to the owner, the lead told: the approved merge waited since"
                    ) && l.contains("(review.merge_wait_hours = 6)")
                        && l.contains("HTTP 502: Bad Gateway")),
                    "{lines:?}"
                );
                assert_eq!(events(&s, "review.owner").len(), 1);
                assert!(events(&s, "review.merged").is_empty());
            } else {
                assert!(
                    lines
                        .iter()
                        .any(|l| l.contains("#7 merged after waiting (1 refused merges)")),
                    "{lines:?}"
                );
                assert_eq!(events(&s, "review.merged").len(), 1);
                assert!(events(&s, "review.owner").is_empty());
            }
            // Settled either way: no third try.
            assert!(
                tend_reviews(
                    &s,
                    std::slice::from_ref(&proj),
                    &crate::util::plus_minutes(&late, 600)
                )
                .is_empty()
            );
            assert_eq!(merges(&log), 2);
        }
    }

    /// The owner's Merge is for the head the hub showed: pressed while the
    /// approved merge waits for its checks, then a new commit lands on the
    /// forge. The wait is reviewed again, and the click does not merge the
    /// new head when its checks go green before its review gives a verdict.
    #[test]
    fn the_owners_merge_beside_a_wait_does_not_merge_a_head_that_moved() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, log, view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        reviewed(&s, &proj);
        waiting_for_checks(&s, &proj, &view);
        event(
            &s,
            TEST_NOW,
            "human",
            "review.ask",
            "proj",
            json!({"project": "proj", "number": 7, "action": "merge", "task": "TASK-001"}),
        );
        // A new commit on the forge, its checks green.
        view_with(&view, |v| v["headRefOid"] = json!("def456def456def456"));
        let tend = |minutes: i32| {
            tend_reviews(
                &s,
                std::slice::from_ref(&proj),
                &crate::util::plus_minutes(TEST_NOW, minutes),
            )
        };
        let lines = tend(1);
        assert!(
            lines.iter().any(|l| l.contains("#7 not merged")),
            "{lines:?}"
        );
        assert_eq!(events(&s, "review.again").len(), 1);
        // The next passes queue the new head's review and leave the click
        // alone: it was for abc123.
        let mut said = Vec::new();
        for minutes in [2, 3, 4] {
            said.extend(tend(minutes));
        }
        assert_eq!(merges(&log), 0, "{said:?}");
        assert!(events(&s, "review.merged").is_empty());
        assert!(events(&s, "review.failed").is_empty());
        let queued = events(&s, "review.queued");
        assert_eq!(queued.last().unwrap()["sha"], "def456def456def456");
        assert_eq!(
            said.iter()
                .filter(|l| l.contains(
                    "#7 not merged for the owner: Merge was pressed at abc123, the pull request is at def456def456 now"
                ))
                .count(),
            1,
            "said once: {said:?}"
        );
        let pr = hive_tasks::pr_states_for(&s, "proj")
            .unwrap()
            .into_iter()
            .find(|p| p.number == 7)
            .unwrap();
        assert_ne!(pr.state, "merge_asked");

        // Pressed again at the new head, it merges.
        event(
            &s,
            TEST_NOW,
            "human",
            "review.ask",
            "proj",
            json!({"project": "proj", "number": 7, "action": "merge", "task": "TASK-001"}),
        );
        let lines = tend(5);
        assert!(
            lines.iter().any(|l| l.contains("#7 merged for the owner")),
            "{lines:?}"
        );
        assert_eq!(merges(&log), 1);
    }

    #[test]
    fn a_check_that_stays_queued_ends_the_wait_at_the_time_bound() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, log, view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        set_setting(&s, WAIT_HOURS, "1");
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        reviewed(&s, &proj);
        waiting_for_checks(&s, &proj, &view);
        let tend = |at: &str| tend_reviews(&s, std::slice::from_ref(&proj), at);
        assert!(
            !tend(&crate::util::plus_minutes(TEST_NOW, 59))
                .iter()
                .any(|l| l.contains("handed to the owner"))
        );
        let lines = tend(&crate::util::plus_minutes(TEST_NOW, 60));
        assert!(
            lines.iter().any(|l| l.contains(
                "#7 handed to the owner, the lead told: the approved merge waited since 2026-09-06T10:00:00Z (review.merge_wait_hours = 1)"
            )),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.contains("ci/build")),
            "the forge's last answer names the check: {lines:?}"
        );
        assert_eq!(merges(&log), 0);
        assert_eq!(lead_posts(&s).len(), 1);
        assert!(tend(&crate::util::plus_minutes(TEST_NOW, 600)).is_empty());
        assert_eq!(lead_posts(&s).len(), 1);
    }

    #[test]
    fn the_owners_rule_is_read_again_at_a_retry() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (bin, _path, log, _view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        reviewed(&s, &proj);
        waiting_for_forge(&s, &proj, bin.path());
        set_setting(&s, "review.merge", "ask");
        let lines = tend_reviews(
            &s,
            std::slice::from_ref(&proj),
            &crate::util::plus_minutes(TEST_NOW, 5),
        );
        assert!(
            lines.iter().any(|l| l.contains(
                "#7 handed to the owner: the approved merge is the owner's now: the owner asked to merge every pull request"
            )),
            "{lines:?}"
        );
        assert_eq!(merges(&log), 1, "the daemon does not merge it");
        assert_eq!(events(&s, "review.owner").len(), 1);
    }

    /// The owner's Merge on a pull request that needs a cure first: the
    /// forge refuses it (a conflict), delivery merges the base in and
    /// records the new head, and the ask lapses. The owner is told once,
    /// where they pressed - the task's conversation, or the chat's
    /// transcript - with why the head moved, not only the daemon log.
    #[test]
    fn a_merge_that_lapses_on_a_cure_is_said_to_the_owner_once() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        for chat in [false, true] {
            let (_bin, _path, log, view) = fake_gh();
            let s = new_store();
            let (proj, repo, _bare) = push_fixture(&s);
            let task = if chat {
                s.chat_create(&hive_store::Conversation {
                    id: "chat-1".into(),
                    title: "a chat".into(),
                    project: "proj".into(),
                    cwd: repo.clone(),
                    branch: "hive/chat-chat-1".into(),
                    status: "idle".into(),
                    ..Default::default()
                })
                .unwrap();
                event(
                    &s,
                    TEST_NOW,
                    "daemon",
                    "land.pr",
                    "proj",
                    json!({"project": "proj", "kind": "proposed", "conversation": "chat-1", "number": 7,
                        "url": "https://github.com/o/r/pull/7", "head": "hive/chat-chat-1", "base": "main",
                        "remote": "origin", "to": "abc123", "opened": true, "title": "PR 7 title"}),
                );
                String::new()
            } else {
                let task =
                    hive_tasks::new(&s, "proj", "Add the thing", "b", 0, hive_store::HUMAN_AGENT)
                        .unwrap();
                land_pr(&s, 7, "task", &task.id, "JOB-a");
                task.id
            };
            event(
                &s,
                TEST_NOW,
                "reviewer-1",
                "review.owner",
                "proj",
                json!({"project": "proj", "number": 7, "url": "https://github.com/o/r/pull/7", "reasons": ["touches 40 files"]}),
            );
            hive_tasks::ask_merge(&s, "proj", 7, &task, "", hive_store::HUMAN_AGENT).unwrap();
            // The forge refuses it: it conflicts with its base.
            view_with(&view, |v| v["mergeStateStatus"] = json!("DIRTY"));
            let tend = |minutes: i32| {
                tend_reviews(
                    &s,
                    std::slice::from_ref(&proj),
                    &crate::util::plus_minutes(TEST_NOW, minutes),
                )
            };
            let lines = tend(1);
            assert!(
                lines
                    .iter()
                    .any(|l| l.contains("#7 could not be merged for the owner")
                        && l.contains("DIRTY")),
                "{lines:?}"
            );
            // Delivery cures it: the base merged in and pushed.
            let mut cured = crate::util::must_rows(
                &*s.conn().unwrap(),
                "SELECT detail FROM events WHERE action = 'land.pr' ORDER BY seq DESC LIMIT 1",
                &[],
            )[0]
            .str("detail");
            let mut d: Value = serde_json::from_str(&cured).unwrap();
            d["from"] = json!("abc123");
            d["to"] = json!("def456def456def456");
            d["opened"] = json!(false);
            d["refreshed"] = json!(true);
            cured = d.to_string();
            event(
                &s,
                TEST_NOW,
                "daemon",
                "land.pr",
                "proj",
                serde_json::from_str(&cured).unwrap(),
            );
            view_with(&view, |v| v["headRefOid"] = json!("def456def456def456"));
            let mut said = Vec::new();
            for minutes in [2, 3, 4] {
                said.extend(tend(minutes));
            }
            assert_eq!(merges(&log), 0, "{said:?}");
            assert_eq!(
                said.iter()
                    .filter(|l| l.contains(
                        "#7 not merged for the owner: Merge was pressed at abc123, the pull request is at def456def456 now (Hive merged its base into it so that it can merge)"
                    ))
                    .count(),
                1,
                "said once in the log: {said:?}"
            );
            let words = "Pull request #7 (https://github.com/o/r/pull/7) was not merged: your Merge was for commit abc123, and Hive merged its base into it so that it can merge - it is at def456def456 now.";
            let told: Vec<String> = if chat {
                s.chat_messages("chat-1", 0)
                    .unwrap()
                    .into_iter()
                    .filter(|m| m.role == "note")
                    .map(|m| m.content)
                    .collect()
            } else {
                hive_tasks::messages(&s, "proj", &task, "", "")
                    .unwrap()
                    .into_iter()
                    .map(|m| m.body)
                    .collect()
            };
            assert_eq!(
                told.iter().filter(|m| m.contains(words)).count(),
                1,
                "told once where Merge was pressed (chat: {chat}): {told:?} / {said:?}"
            );
            assert!(
                told.iter().any(|m| m.contains("press Merge again")),
                "{told:?}"
            );
        }
    }

    /// The ask stores the head the owner pressed Merge for, and that is
    /// the head it merges: a press the hub judged on the old state but
    /// wrote just after the daemon recorded a new head lapses, and a head
    /// recorded again (a review queued at the same commit) keeps it.
    #[test]
    fn the_owners_merge_is_for_the_head_it_stores() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        for moved in [true, false] {
            let (_bin, _path, log, view) = fake_gh();
            let s = new_store();
            let (proj, _repo, _bare) = push_fixture(&s);
            land_pr(&s, 7, "task", "TASK-001", "JOB-a");
            event(
                &s,
                TEST_NOW,
                "reviewer-1",
                "review.owner",
                "proj",
                json!({"project": "proj", "number": 7, "url": "https://github.com/o/r/pull/7", "reasons": ["touches 40 files"]}),
            );
            // The hub judged the press on #7 at abc123 ...
            let shown = hive_tasks::pr_states_for(&s, "proj")
                .unwrap()
                .into_iter()
                .find(|p| p.number == 7)
                .unwrap();
            assert_eq!((shown.sha.as_str(), shown.can_merge), ("abc123", true));
            if moved {
                // ... and the daemon recorded a new head before the ask was
                // written.
                event(
                    &s,
                    TEST_NOW,
                    "daemon",
                    "land.pr",
                    "proj",
                    json!({"project": "proj", "kind": "task", "task": "TASK-001", "number": 7,
                        "url": "https://github.com/o/r/pull/7", "head": "hive/task/TASK-001", "base": "main",
                        "remote": "origin", "from": "abc123", "to": "def456def456def456", "opened": false}),
                );
                view_with(&view, |v| v["headRefOid"] = json!("def456def456def456"));
            } else {
                // ... and a review was queued at the same commit meanwhile.
                event(
                    &s,
                    TEST_NOW,
                    "daemon",
                    "review.queued",
                    "proj",
                    json!({"project": "proj", "number": 7, "sha": "abc123", "reasons": ["touches 40 files"]}),
                );
            }
            hive_tasks::ask_merge(
                &s,
                "proj",
                7,
                "TASK-001",
                &shown.sha,
                hive_store::HUMAN_AGENT,
            )
            .unwrap();
            assert_eq!(events(&s, "review.ask")[0]["sha"], "abc123");
            let lines = tend_reviews(
                &s,
                std::slice::from_ref(&proj),
                &crate::util::plus_minutes(TEST_NOW, 1),
            );
            if moved {
                assert_eq!(merges(&log), 0, "{lines:?}");
                assert!(
                    lines.iter().any(|l| l.contains(
                        "#7 not merged for the owner: Merge was pressed at abc123, the pull request is at def456def456 now (new commits were pushed to it)"
                    )),
                    "{lines:?}"
                );
            } else {
                assert_eq!(merges(&log), 1, "{lines:?}");
                assert!(
                    lines.iter().any(|l| l.contains("#7 merged for the owner")),
                    "{lines:?}"
                );
            }
        }
    }

    /// The last try after a sleep is the merge's: a read of the pull
    /// request that fails for a passing reason on that pass does not end
    /// the wait with no merge tried - the try waits for its back-off and
    /// is made then, merged or handed over on its own answer. Out of tries,
    /// the failed read still ends it.
    #[test]
    fn a_failed_read_does_not_spend_the_last_try_after_the_hours() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let late = crate::util::plus_minutes(TEST_NOW, 7 * 60);
        for (tries, merge_fails) in [(5, false), (5, true), (2, false)] {
            let (bin, _path, log, _view) = fake_gh();
            let s = new_store();
            let (proj, _repo, _bare) = push_fixture(&s);
            set_setting(&s, WAIT_TRIES, &tries.to_string());
            land_pr(&s, 7, "task", "TASK-001", "JOB-a");
            reviewed(&s, &proj);
            waiting_for_forge(&s, &proj, bin.path());
            assert_eq!(merges(&log), 1);
            // The machine slept past the hours; on waking the read fails.
            std::fs::write(bin.path().join("fail"), "view\n").unwrap();
            std::fs::write(bin.path().join("failmsg"), "HTTP 502: Bad Gateway").unwrap();
            let tend = |at: &str| tend_reviews(&s, std::slice::from_ref(&proj), at);
            let lines = tend(&late);
            assert_eq!(merges(&log), 1, "no merge could be tried: {lines:?}");
            if tries == 2 {
                assert!(
                    lines.iter().any(|l| l.contains(
                        "#7 handed to the owner, the lead told: the forge failed the approved merge 2 time(s)"
                    )),
                    "out of tries, the read ends it: {lines:?}"
                );
                assert_eq!(events(&s, "review.owner").len(), 1);
                continue;
            }
            assert!(
                events(&s, "review.owner").is_empty(),
                "the last try is not spent on the read: {lines:?}"
            );
            assert!(lead_posts(&s).is_empty(), "{lines:?}");
            let waits = events(&s, "review.merge-wait");
            assert_eq!(waits.last().unwrap()["attempt"], 2, "{waits:?}");
            assert_eq!(waits.last().unwrap()["last"], true, "{waits:?}");
            // Before its back-off, nothing; then the last try is made.
            std::fs::write(
                bin.path().join("fail"),
                if merge_fails { "merge\n" } else { "" },
            )
            .unwrap();
            assert!(tend(&crate::util::plus_minutes(&late, 1)).is_empty());
            assert_eq!(merges(&log), 1);
            let lines = tend(&crate::util::plus_minutes(&late, 20));
            assert_eq!(merges(&log), 2, "the last try ({merge_fails}): {lines:?}");
            if merge_fails {
                assert!(
                    lines.iter().any(|l| l.contains(
                        "#7 handed to the owner, the lead told: the approved merge waited since"
                    )),
                    "{lines:?}"
                );
                assert_eq!(events(&s, "review.owner").len(), 1);
                assert!(events(&s, "review.merged").is_empty());
            } else {
                assert!(
                    lines
                        .iter()
                        .any(|l| l.contains("#7 merged after waiting (2 refused merges)")),
                    "{lines:?}"
                );
                assert_eq!(events(&s, "review.merged").len(), 1);
            }
            // Settled: no third try.
            assert!(tend(&crate::util::plus_minutes(&late, 600)).is_empty());
            assert_eq!(merges(&log), 2);
        }
    }

    #[test]
    fn passing_forge_errors_are_5xx_and_internal_errors_only() {
        for e in [
            "gh pr merge failed: HTTP 502: Bad Gateway (https://api.github.com/graphql)",
            "gh pr merge failed: HTTP 500: Internal Server Error",
            "gh pr merge failed: GraphQL: Something went wrong while executing your query. This may be the result of a timeout",
            "gh pr merge failed: GraphQL: Base branch was modified. Review and try the merge again. (mergePullRequest)",
            "gh pr merge did not finish within 180s",
            "gh pr merge failed: HTTP 503",
            "gh pr view failed: HTTP 429: Too Many Requests",
            "gh pr view failed: HTTP 403: API rate limit exceeded for installation ID 1",
            "gh pr view failed: HTTP 403: You have exceeded a secondary rate limit",
            "gh pr view failed: error connecting to api.github.com",
            "gh pr view failed: Post \"https://api.github.com/graphql\": dial tcp: connect: connection refused",
            "gh pr view failed: Post \"https://api.github.com/graphql\": EOF",
            "gh pr view failed: Post \"https://api.github.com/graphql\": unexpected EOF",
            "gh pr view failed: Post \"https://api.github.com/graphql\": dial tcp 140.82.121.6:443: connect: network is unreachable",
            "gh pr merge failed: Post \"https://api.github.com/graphql\": dial tcp 140.82.121.6:443: connect: no route to host",
            "gh pr merge failed: Post \"https://api.github.com/graphql\": write tcp 192.168.1.5:50122->140.82.121.6:443: write: broken pipe",
        ] {
            assert!(passing_forge_error(e), "{e}");
        }
        for e in [
            "gh pr merge failed: GraphQL: Pull Request is not mergeable (mergePullRequest)",
            "gh pr merge failed: HTTP 405: Required status check \"ci\" is failing",
            "gh pr merge failed: HTTP 422: Changes must be made through a pull request",
            "gh pr merge failed: HTTP 403: Resource not accessible by integration",
            "gh pr merge failed: merge conflict between base and head",
            "gh pr merge failed: Pull request #502 is not mergeable",
            "gh pr view failed: HTTP 401: Bad credentials",
            "gh pr view failed: GraphQL: Could not resolve to a PullRequest with the number of 7. (HTTP 404)",
        ] {
            assert!(!passing_forge_error(e), "{e}");
        }
    }

    #[test]
    fn a_new_head_while_the_review_runs_waits_for_the_verdict() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, log, _view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "job", "", "JOB-a");
        tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW);
        let job = jobs_of(&s, "review")[0].str("id");
        exec1(
            &s,
            "UPDATE jobs SET status = 'leased' WHERE id = ?",
            &[&job],
        );
        // New commits on the pull request: a second land.pr, opened:false.
        let mut d = json!({"project": "proj", "kind": "job", "job": "JOB-a", "number": 7, "url": "https://github.com/o/r/pull/7", "head": "hive/worker/JOB-a", "base": "main", "to": "def456", "opened": false});
        d["title"] = json!("PR 7 title");
        event(&s, TEST_NOW, "daemon", "land.pr", "proj", d);
        let lines = tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(jobs_of(&s, "review").len(), 1);
        assert_eq!(calls(&log), ["pr view 7"]);
    }

    #[test]
    fn repo_names_read_from_pull_request_urls() {
        assert_eq!(
            repo_from_pr_url("https://github.com/o/r/pull/7").as_deref(),
            Some("o/r")
        );
        assert_eq!(
            repo_from_pr_url("https://ghe.corp/o/r/pull/7").as_deref(),
            Some("ghe.corp/o/r")
        );
        assert_eq!(repo_from_pr_url("https://github.com/o/r"), None);
        assert_eq!(repo_from_pr_url(""), None);
        assert!(generated_path("web/hub/out/_next/x.js"));
        assert!(generated_path("Cargo.lock"));
        assert!(generated_path("web/hub/package-lock.json"));
        assert!(generated_path("vendor/lib.min.js"));
        assert!(generated_path("target/debug/x"));
        assert!(!generated_path("src/outer.rs"));
        assert!(!generated_path("crates/hive-runner/src/review.rs"));
    }

    #[test]
    fn checkout_fetches_the_head_from_the_local_checkout_then_the_upstream() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let job = push_branch(&s, &proj, &repo, "hive/worker/a");
        let _ = job;
        let sha = ref_of(&repo, "refs/heads/hive/worker/a");
        let clone = std::path::Path::new(&s.home)
            .join("tmp")
            .join("clone-review");
        let clone = clone.display().to_string();
        run(".", &["git", "clone", "-q", &repo, &clone]);
        run(&clone, &["git", "remote", "remove", "origin"]);

        let mut view = view_of(7, "hive/worker/a", &[("x", 1, 0)]);
        view.head_sha = sha.clone();
        let got = checkout_into(&s, &proj, &view, &clone).unwrap();
        assert_eq!(got, sha);
        assert_eq!(crate::util::git_branch(&clone), "review/pr-7");
        assert_eq!(ref_of(&clone, "HEAD"), sha);

        // The local branch is behind the pull request (pushed elsewhere):
        // the head comes from the upstream's refs/pull/N/head.
        run(&repo, &["git", "checkout", "-q", "hive/worker/a"]);
        std::fs::write(std::path::Path::new(&repo).join("more.txt"), "more\n").unwrap();
        run(&repo, &["git", "add", "-A"]);
        run(&repo, &["git", "commit", "-q", "-m", "more"]);
        let newer = ref_of(&repo, "refs/heads/hive/worker/a");
        run(
            &repo,
            &[
                "git",
                "push",
                "-q",
                "origin",
                "hive/worker/a:refs/pull/7/head",
            ],
        );
        run(&repo, &["git", "reset", "-q", "--hard", &sha]);
        run(&repo, &["git", "checkout", "-q", "main"]);
        assert_eq!(bare_ref(&bare, "refs/pull/7/head"), newer);
        view.head_sha = newer.clone();
        run(&clone, &["git", "checkout", "-q", "main"]);
        let got = checkout_into(&s, &proj, &view, &clone).unwrap();
        assert_eq!(got, newer);
        assert_eq!(ref_of(&clone, "HEAD"), newer);

        // The project's own checkout is refused.
        let err = checkout_into(&s, &proj, &view, &repo).unwrap_err();
        assert!(err.contains("project's own checkout"), "{err}");
    }

    /// A pull request stacked on another waits for it: no review while it
    /// targets the branch below, no merge either; once delivery moves it
    /// onto main (a fresh `land.pr`), it is reviewed like any other.
    #[test]
    fn a_stacked_pull_request_is_reviewed_and_merged_after_the_one_below() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, log, _view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        event(
            &s,
            TEST_NOW,
            "reviewer",
            "review.owner",
            "proj",
            json!({"project": "proj", "number": 7, "url": "u", "reasons": ["big"]}),
        );
        event(
            &s,
            TEST_NOW,
            "daemon",
            "land.pr",
            "proj",
            json!({"project": "proj", "kind": "job", "job": "JOB-b", "number": 8,
                "url": "https://github.com/o/r/pull/8", "head": "hive/worker/JOB-b",
                "base": "hive/task/TASK-001", "stack_on": 7, "to": "abc123", "opened": true,
                "title": "PR 8 title"}),
        );

        let lines = tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert_eq!(
            lines,
            ["review proj: #8 stacked on #7: reviewed after #7 merges"]
        );
        assert!(jobs_of(&s, "review").is_empty());
        assert_eq!(
            merge_as_owner(&s, &proj, 8, "", TEST_NOW).unwrap_err(),
            "pull request #8 is stacked on #7; #7 must merge first"
        );
        assert!(calls(&log).is_empty(), "{:?}", calls(&log));
        assert!(events(&s, "review.failed").is_empty());

        // Delivery moved #8 onto main.
        event(
            &s,
            TEST_NOW,
            "daemon",
            "land.pr",
            "proj",
            json!({"project": "proj", "kind": "job", "job": "JOB-b", "number": 8,
                "url": "https://github.com/o/r/pull/8", "head": "hive/worker/JOB-b",
                "base": "main", "to": "abc123", "opened": false,
                "retargeted_from": "hive/task/TASK-001", "title": "PR 8 title"}),
        );
        let lines = tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains("#8 queued for review")),
            "{lines:?}"
        );
        assert_eq!(jobs_of(&s, "review").len(), 1);
    }

    /// The owner's #7, handed over at abc123 and asked to merge there.
    fn owner_asked(s: &hive_store::Store) {
        land_pr(s, 7, "task", "TASK-001", "JOB-a");
        event(
            s,
            TEST_NOW,
            "reviewer-1",
            "review.owner",
            "proj",
            json!({"project": "proj", "number": 7, "url": "https://github.com/o/r/pull/7", "reasons": ["touches 40 files"]}),
        );
        hive_tasks::ask_merge(s, "proj", 7, "TASK-001", "", hive_store::HUMAN_AGENT).unwrap();
    }

    /// A commit the daemon records for #7 (a fix job's push, a refresh).
    fn new_head(s: &hive_store::Store, to: &str, refreshed: bool) {
        event(
            s,
            TEST_NOW,
            "daemon",
            "land.pr",
            "proj",
            json!({"project": "proj", "kind": "task", "task": "TASK-001", "number": 7,
                "url": "https://github.com/o/r/pull/7", "head": "hive/task/TASK-001", "base": "main",
                "remote": "origin", "to": to, "opened": false, "refreshed": refreshed, "title": "PR 7 title"}),
        );
    }

    fn merge_args(log: &str) -> Vec<String> {
        std::fs::read_to_string(log)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with("pr merge"))
            .map(str::to_string)
            .collect()
    }

    /// TASK-147 item 1: every merge Hive makes is pinned to the commit it
    /// read (`--match-head-commit`), the owner's and the reviewer's; a
    /// commit pushed between the read and the merge is not merged. The
    /// forge refuses it, the new head is recorded (`review.again`) and the
    /// owner's ask lapses as for any moved head.
    #[test]
    fn a_head_that_moves_between_the_read_and_the_merge_is_not_merged() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        for owner in [true, false] {
            let (bin, _path, log, view) = fake_gh();
            let s = new_store();
            let (proj, _repo, _bare) = push_fixture(&s);
            // A push of def456 lands while `gh pr merge` runs.
            std::fs::write(bin.path().join("pushed"), "def456def456def456").unwrap();
            let mut next = view_value(7, "hive/task/TASK-001", &[("src/a.rs", 5, 2)]);
            next["headRefOid"] = json!("def456def456def456");
            std::fs::write(bin.path().join("view.next"), next.to_string()).unwrap();
            let lines = if owner {
                owner_asked(&s);
                tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW)
            } else {
                land_pr(&s, 7, "task", "TASK-001", "JOB-a");
                reviewed(&s, &proj);
                let err = merge_or_wait(&s, &proj, 7, None, "reviewer-1", "reviewer-1", TEST_NOW)
                    .unwrap_err();
                vec![err]
            };
            let args = merge_args(&log);
            assert_eq!(args.len(), 1, "{args:?}");
            assert!(
                args[0].contains("--match-head-commit abc123"),
                "pinned to the commit read: {args:?}"
            );
            assert!(events(&s, "review.merged").is_empty(), "{lines:?}");
            assert!(
                events(&s, "review.failed").is_empty(),
                "no failure: {lines:?}"
            );
            let again = events(&s, "review.again");
            assert_eq!(again.len(), 1, "{lines:?}");
            assert_eq!(again[0]["head_sha"], "def456def456def456");
            assert_eq!(again[0]["was"], "abc123");
            assert!(
                lines
                    .iter()
                    .any(|l| l.contains("a commit reached the pull request after it was read")),
                "{lines:?}"
            );
            let _ = view;
            if owner {
                // The next pass: the ask was for abc123; it lapses, once.
                let lines = tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW);
                assert!(
                    lines
                        .iter()
                        .any(|l| l.contains("#7 not merged for the owner")),
                    "{lines:?}"
                );
                assert_eq!(events(&s, LAPSED_ACTION).len(), 1);
                assert_eq!(merge_args(&log).len(), 1, "no second merge");
            }
        }
    }

    /// TASK-147 item 2: a lapse is for good. When the head comes back to
    /// the commit Merge was pressed for (a force-push back, a review queued
    /// at a stale commit), the ask already told as lapsed is not honoured
    /// again: nothing is merged and nothing is said twice.
    #[test]
    fn a_lapsed_ask_whose_head_returns_stays_lapsed() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, log, view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        owner_asked(&s);
        new_head(&s, "def456def456def456", false);
        view_with(&view, |v| v["headRefOid"] = json!("def456def456def456"));
        let tend = || tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW);
        let lines = tend();
        assert!(
            lines
                .iter()
                .any(|l| l.contains("#7 not merged for the owner")),
            "{lines:?}"
        );
        // The head returns to abc123, green and clean on the forge.
        new_head(&s, "abc123", false);
        view_with(&view, |_| {});
        let lines = tend();
        assert_eq!(merges(&log), 0, "the lapsed ask is not honoured: {lines:?}");
        assert!(events(&s, "review.merged").is_empty());
        assert_eq!(events(&s, LAPSED_ACTION).len(), 1, "said once: {lines:?}");
        assert!(
            !lines.iter().any(|l| l.contains("for the owner")),
            "{lines:?}"
        );
    }

    /// TASK-147 item 3: the hours end a forge wait whose reads keep
    /// failing, whatever `review.merge_wait_tries` says. Past the hours the
    /// failed read gives the last try one more back-off; when that read
    /// fails too, the wait ends then - not 6-hour steps later.
    #[test]
    fn reads_that_keep_failing_end_the_wait_on_its_hours() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (bin, _path, log, _view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        set_setting(&s, WAIT_TRIES, "50");
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        reviewed(&s, &proj);
        waiting_for_forge(&s, &proj, bin.path());
        std::fs::write(bin.path().join("fail"), "view\n").unwrap();
        std::fs::write(bin.path().join("failmsg"), "HTTP 502: Bad Gateway").unwrap();
        let late = crate::util::plus_minutes(TEST_NOW, 7 * 60);
        let tend = |at: &str| tend_reviews(&s, std::slice::from_ref(&proj), at);
        let lines = tend(&late);
        assert!(events(&s, "review.owner").is_empty(), "{lines:?}");
        assert_eq!(
            events(&s, "review.merge-wait").last().unwrap()["last"],
            true
        );
        // The deferred last try: its read fails too, and the hours end it.
        let lines = tend(&crate::util::plus_minutes(&late, 20));
        assert!(
            lines.iter().any(|l| l.contains(
                "#7 handed to the owner, the lead told: the approved merge waited since"
            )),
            "{lines:?}"
        );
        assert_eq!(events(&s, "review.owner").len(), 1);
        assert_eq!(merges(&log), 1, "no merge was tried past the hours");
        assert!(tend(&crate::util::plus_minutes(&late, 600)).is_empty());
    }

    /// TASK-147 item 6: reads that fail on every try end the wait when the
    /// tries are spent, with the forge's last answer.
    #[test]
    fn reads_that_fail_until_the_tries_end_hand_it_to_the_owner() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (bin, _path, log, _view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        set_setting(&s, WAIT_TRIES, "3");
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        reviewed(&s, &proj);
        waiting_for_forge(&s, &proj, bin.path());
        std::fs::write(bin.path().join("fail"), "view\n").unwrap();
        std::fs::write(bin.path().join("failmsg"), "HTTP 503: Service Unavailable").unwrap();
        let mut said = Vec::new();
        for minutes in [6, 25] {
            said.extend(tend_reviews(
                &s,
                std::slice::from_ref(&proj),
                &crate::util::plus_minutes(TEST_NOW, minutes),
            ));
        }
        assert!(
            said.iter().any(|l| l.contains(
                "#7 handed to the owner, the lead told: the forge failed the approved merge 3 time(s)"
            )),
            "{said:?}"
        );
        assert_eq!(events(&s, "review.owner").len(), 1);
        assert_eq!(merges(&log), 1, "only the first merge was tried");
    }

    /// TASK-147 item 4: a lapse hands the pull request back with answers
    /// to pick, one recommended, as a question for the owner - also for a
    /// pull request with neither a task nor a chat. "Wait for the review of
    /// the new commit" is recommended when the new commit is not Hive's own
    /// refresh; "Merge this commit" when it is. The owner's "Merge this
    /// commit" merges the commit the question named and nothing else; a
    /// pick of "Wait ..." merges nothing; neither queues a job.
    #[test]
    fn a_lapse_comes_back_with_answers_and_the_owners_pick_is_acted_on() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        for (refresh, pick) in [(false, LAPSE_MERGE), (true, LAPSE_WAIT)] {
            let (_bin, _path, log, view) = fake_gh();
            let s = new_store();
            let (proj, _repo, _bare) = push_fixture(&s);
            // A job's pull request: no task, no chat.
            land_pr(&s, 7, "job", "", "JOB-a");
            event(
                &s,
                TEST_NOW,
                "reviewer-1",
                "review.owner",
                "proj",
                json!({"project": "proj", "number": 7, "url": "https://github.com/o/r/pull/7", "reasons": ["touches 40 files"]}),
            );
            hive_tasks::ask_merge(&s, "proj", 7, "", "", hive_store::HUMAN_AGENT).unwrap();
            let mut d = json!({"project": "proj", "kind": "job", "job": "JOB-a", "number": 7,
                "url": "https://github.com/o/r/pull/7", "head": "hive/worker/JOB-a", "base": "main",
                "remote": "origin", "to": "def456def456def456", "opened": false, "title": "PR 7 title"});
            if refresh {
                d["refreshed"] = json!(true);
            }
            event(&s, TEST_NOW, "daemon", "land.pr", "proj", d);
            view_with(&view, |v| v["headRefOid"] = json!("def456def456def456"));
            let tend = || tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW);
            let lines = tend();
            let lapsed = events(&s, LAPSED_ACTION);
            assert_eq!(lapsed.len(), 1, "{lines:?}");
            let post = lapsed[0]["post"].as_str().unwrap().to_string();
            assert!(!post.is_empty(), "{lines:?}");
            let q = crate::board::post_row(&*s.conn().unwrap(), post.clone());
            assert_eq!(q.str("kind"), "question");
            assert!(
                q.str("summary")
                    .starts_with("Merge pull request #7: PR 7 title?"),
                "{}",
                q.str("summary")
            );
            let body = q.str("body");
            let options = hive_board::ask_options(&body);
            let mut labels: Vec<&str> = options.options.iter().map(|o| o.label.as_str()).collect();
            labels.sort_unstable();
            assert_eq!(labels, [LAPSE_MERGE, LAPSE_WAIT], "{body}");
            assert!(options.ready(), "{body}");
            let recommended = if refresh { LAPSE_MERGE } else { LAPSE_WAIT };
            assert_eq!(
                options
                    .options
                    .iter()
                    .find(|o| o.recommended)
                    .unwrap()
                    .label,
                recommended
            );
            assert!(
                body.contains(&format!("Recommended: {recommended} - ")),
                "{body}"
            );
            assert!(body.contains("Was: commit abc123"), "{body}");
            assert!(body.contains("Now: commit def456def456"), "{body}");
            // The owner picks.
            let decision =
                hive_board::decide_option(&s, &post, pick, "", hive_store::HUMAN_AGENT).unwrap();
            assert!(decision.job.is_none(), "the daemon acts on the pick itself");
            let lines = tend();
            if pick == LAPSE_MERGE {
                assert!(
                    lines.iter().any(|l| l.contains("#7 merged for the owner")),
                    "{lines:?}"
                );
                let args = merge_args(&log);
                assert_eq!(args.len(), 1, "{args:?}");
                assert!(
                    args[0].contains("--match-head-commit def456def456def456"),
                    "{args:?}"
                );
            } else {
                assert_eq!(merges(&log), 0, "{lines:?}");
                assert!(
                    lines
                        .iter()
                        .any(|l| l.contains("the owner waits for the review")),
                    "{lines:?}"
                );
            }
            // Acted on once.
            assert!(
                !tend()
                    .iter()
                    .any(|l| l.contains("the owner picked") || l.contains("the owner waits")),
            );
        }
    }
}
