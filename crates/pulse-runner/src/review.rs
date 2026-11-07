//! The pull request reviewer: every finished branch becomes a pull request
//! on the project's upstream (`task_pr.rs` and the job flow emit `land.pr`),
//! and a review agent, not the daemon, decides it. The maintenance pass
//! queues one `review` job per open PULSE pull request, the agent works it
//! with the `pulse review` commands (`review_cli.rs`), and ends with exactly
//! one verdict: merge, ask for changes, or hand it to the owner.
//!
//! The complexity rule ([`reasons_for`]) is computed here, in code, before
//! the job is queued: a pull request the owner asked to see (`review.merge`
//! `ask`), one touching a guarded path, or one not from a PULSE branch is
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
/// The job kind the reviewer queues (`pulse_store::kinds`).
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
/// The branch `pulse review checkout` puts a pull request's head on.
pub(crate) fn review_branch(number: i64) -> String {
    format!("review/pr-{number}")
}

/// The events that end a pull request's review: once the newest of these
/// for a number is not `land.pr`, nothing is queued for it. `review.failed`
/// is not among them: a failed review is tried once more (`MAX_ATTEMPTS`).
const STATE_ACTIONS: &str = "('land.pr', 'land.merged', 'land.pr-closed', 'review.merged', 'review.owner', 'review.changes')";

/// An open PULSE pull request as the newest `land.pr` for its number
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

/// The open PULSE pull requests of a project, oldest number first: every
/// number whose newest state event is a `land.pr`.
pub(crate) fn open_prs(q: &(impl crate::util::Querier + ?Sized), slug: &str) -> Vec<OpenPr> {
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
        let Some(number) = detail["number"].as_i64() else {
            continue;
        };
        latest.insert(number, (r.int("seq") as i64, r.str("action"), detail));
    }
    latest
        .into_iter()
        .filter(|(_, (_, action, _))| action == "land.pr")
        .map(|(_, (seq, _, detail))| OpenPr::from_event(seq, &detail))
        .collect()
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
    let pr = rows.iter().rev().find(|r| r.str("action") == "land.pr")?;
    let detail: Value = serde_json::from_str(&pr.str("detail")).unwrap_or(Value::Null);
    Some((
        OpenPr::from_event(pr.int("seq") as i64, &detail),
        last.str("action"),
    ))
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
}

impl PrView {
    pub(crate) fn from_json(v: &Value) -> Self {
        let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();
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
    let low = err.to_ascii_lowercase();
    if low.contains("branch not found") {
        return false;
    }
    low.contains("http 401")
        || low.contains("http 403")
        || low.contains("http 404")
        || low.contains("could not resolve to a repository")
        || low.contains("gh auth token for")
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
    s: &pulse_store::Store,
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
    let run_as = |user: &str| -> Result<String, String> {
        let env = if user.is_empty() {
            crate::proc::environ()
        } else {
            crate::gh_auth::env_as(user)?
        };
        let out = crate::proc::run_group(
            &pulse_bus::CancellationToken::new(),
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

/// `gh pr view` of one pull request, the fields the rule and brief read.
pub(crate) fn pr_view(
    s: &pulse_store::Store,
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
            "number,title,body,url,state,headRefName,headRefOid,baseRefName,author,additions,deletions,files",
        ],
        GH_TIMEOUT,
    )?;
    let v: Value = serde_json::from_str(out.trim())
        .map_err(|e| format!("gh pr view {number} answered no JSON: {e}"))?;
    Ok(PrView::from_json(&v))
}

/// `gh pr diff` of one pull request.
pub(crate) fn pr_diff(
    s: &pulse_store::Store,
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
/// and the owner's paths for a `pulse/` branch), plus environment files,
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
pub(crate) fn reasons_for(s: &pulse_store::Store, pr: &PrView) -> Vec<String> {
    let mut reasons = Vec::new();
    if s.setting("review.merge", "auto").trim() == "ask" {
        reasons.push("the owner asked to merge every pull request".to_string());
    }
    let paths: Vec<String> = pr.files.iter().map(|(p, _, _)| p.clone()).collect();
    for p in guarded_paths(&paths, &pr.head) {
        reasons.push(format!("touches {p}"));
    }
    // A change that has a face shows it before and after (`pulse report`,
    // `landing.ui_report = require`); without that page it is the owner's.
    if let Some(why) = crate::ui_evidence::pr_report_reason(s, &pr.head, &paths) {
        reasons.push(why);
    }
    // PULSE's own heads are `pulse/...` (jobs, tasks, conversations) and
    // `evolve/<id>` (a kept experiment, already measured against its card);
    // anything else was pushed by a person or another tool and gets the owner.
    if !pulse_head(&pr.head) {
        reasons.push(format!(
            "opened by a human/other tool (branch {})",
            if pr.head.is_empty() {
                "unknown"
            } else {
                &pr.head
            }
        ));
    }
    reasons
}

/// What the reviewer must read with extra care: the shape of the change,
/// not a bar to merging it. More than `review.max_files` files or
/// `review.max_lines` lines (generated paths left out), or a change that
/// mostly deletes, is still the reviewer's to merge once every file of it
/// is explained by the title and body; what it cannot account for goes to
/// the owner through the verdict, as for any other pull request.
pub(crate) fn care_for(s: &pulse_store::Store, pr: &PrView) -> Vec<String> {
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

/// True for a branch PULSE itself pushes: `pulse/<agent>/<job>`,
/// `pulse/task-<id>`, `pulse/chat-<id>` or an experiment's `evolve/<id>`.
pub(crate) fn pulse_head(head: &str) -> bool {
    head.starts_with("pulse/") || head.starts_with("evolve/")
}

/// The project's check command when the store knows one.
fn verify_cmd_of(s: &pulse_store::Store, slug: &str) -> String {
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
        "proposed" => "a branch proposed with pulse land --propose".to_string(),
        other => format!("a {other} branch"),
    };
    b.push(format!(
        "This is a PULSE pull request ({what}): head `{}` into `{}`.",
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
                "Read it with extra care: {}. Size is not a reason to hand it over: go file by file, and merge only when every file is what the title and body say and the check is green; a file the body does not account for, or anything you cannot judge, goes to the owner with `pulse review owner {n} --note`.",
                care.join("; ")
            ));
        }
    } else {
        b.push("You may NOT merge this pull request. Hand it to the owner because:".into());
        for r in reasons {
            b.push(format!("- {r}"));
        }
        b.push(format!(
            "Review it all the same and end with `pulse review owner {n} --note \"<your findings>\"`: the note is what the owner reads before deciding."
        ));
    }
    b.push(String::new());
    if verify_cmd.is_empty() {
        b.push(format!(
            "The project's check: not recorded; `pulse env show --project {slug}` or the README names it."
        ));
    } else {
        b.push(format!("The project's check: `{verify_cmd}`"));
    }
    b.push(String::new());
    b.push("Procedure, in this order, in this clone:".into());
    b.push(format!(
        "1. `pulse review show {n}` - the title, body, changed files and the reasons above."
    ));
    b.push(format!(
        "2. `pulse review checkout {n}` - puts this clone on `{}`, the pull request's head.",
        review_branch(n)
    ));
    b.push("3. Run the project's check. A red check is never merged.".into());
    b.push(format!(
        "4. `pulse review diff {n}` and read the whole diff as a reviewer: does it do what the title and body say, is it correct, is it tested, does it touch anything the body does not explain (secrets, deletions, paths outside its scope), does it break a convention the README or an ADR states."
    ));
    b.push("5. Exactly ONE verdict command:".into());
    if reasons.is_empty() {
        b.push(format!(
            "   - `pulse review merge {n}` - correct, safe, in scope, check green: it merges on GitHub and records review.merged."
        ));
        b.push(format!(
            "   - `pulse review changes {n} --note \"<the exact fixes>\"` - fixable: GitHub gets the request, and one follow-up job makes the fixes on the pull request's head; your note is its brief, so name files and what to change."
        ));
        b.push(format!(
            "   - `pulse review owner {n} --note \"<your findings>\"` - a real decision (money, auth, data, dependencies, deploy, a surface no ADR covers), a conflict with the base, or anything you are not sure of."
        ));
    } else {
        b.push(format!(
            "   - `pulse review owner {n} --note \"<your findings>\"` - the only verdict allowed for this pull request; `pulse review merge {n}` refuses it."
        ));
    }
    b.push(format!(
        "6. `pulse job finish $PULSE_JOB done --result 'SUMMARY: reviewed #{n}: <verdict> - <one line>'` (no `--branch`: a review leaves no branch)."
    ));
    b.push(String::new());
    b.push(format!(
        "Do not fix the code yourself: a fix is a `changes` note, and the follow-up job does it. Do not call gh or git push yourself: `pulse review` talks to GitHub for you. Never merge a red check or a diff you did not read. A review takes at most {REVIEW_MINUTES} minutes; a job that ends without a verdict counts as a failed review."
    ));
    b.join("\n")
}

fn source_of(slug: &str, number: i64) -> String {
    format!("{SOURCE_PREFIX}{slug}:{number}")
}

/// Whether a job is a pull request review. A review's clone holds the
/// pull request's own commits once the agent runs `pulse review checkout`,
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
/// count when the head is unknown).
fn attempts_at(q: &(impl crate::util::Querier + ?Sized), slug: &str, pr: &OpenPr) -> i64 {
    let rows = crate::util::query_rows(
        q,
        "SELECT COALESCE(json_extract(detail, '$.sha'), '') AS sha FROM events WHERE subject = ? AND action = 'review.queued' AND json_extract(detail, '$.number') = ?",
        &[&slug, &pr.number],
    )
    .unwrap_or_default();
    rows.iter()
        .filter(|r| pr.sha.is_empty() || r.str("sha").is_empty() || r.str("sha") == pr.sha)
        .count() as i64
}

fn event(s: &pulse_store::Store, now: &str, actor: &str, action: &str, slug: &str, detail: Value) {
    if let Ok(conn) = s.conn() {
        crate::util::event_at(&conn, now, actor, action, slug, detail);
    }
}

/// Queues the review job of one pull request and records `review.queued`.
pub(crate) fn queue_review(
    s: &pulse_store::Store,
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
/// owner's Merge clicks (`review.ask`), records review jobs that ended
/// without a verdict, and queues one review job per project.
pub(crate) fn tend_reviews(s: &pulse_store::Store, projects: &[Row], now: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for p in projects {
        if p.str("path").is_empty() {
            continue;
        }
        lines.extend(honour_asks(s, p, now));
        lines.extend(tend_project(s, p, now));
    }
    lines
}

/// The owner clicked Merge in the hub: the newest `review.ask` of a number
/// with no `review.merged` or `review.failed` after it is merged as the
/// owner, whatever the complexity rule says.
fn honour_asks(s: &pulse_store::Store, project: &Row, now: &str) -> Vec<String> {
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
    let mut newest: std::collections::BTreeMap<i64, i64> = Default::default();
    for a in asks {
        let detail: Value = serde_json::from_str(&a.str("detail")).unwrap_or(Value::Null);
        if detail["action"].as_str().unwrap_or("merge") != "merge" {
            continue;
        }
        if let Some(n) = detail["number"].as_i64() {
            newest.insert(n, a.int("seq") as i64);
        }
    }
    let mut lines = Vec::new();
    for (number, seq) in newest {
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
        match merge_as_owner(s, project, number, now) {
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

/// Merges a pull request as the owner (`review.merged` with `by: owner`),
/// bypassing the complexity rule: the hub's Merge button and `pulse review
/// merge N --by owner` come here. A failure is a `review.failed`.
pub(crate) fn merge_as_owner(
    s: &pulse_store::Store,
    project: &Row,
    number: i64,
    now: &str,
) -> Result<String, String> {
    merge(s, project, number, "owner", ACTOR, now)
}

/// `gh pr merge --merge --delete-branch`, then `review.merged`; on a gh
/// failure `review.failed` with the error. Returns the pull request's url.
/// A pull request stacked on another is refused before gh is asked, and
/// without a `review.failed`: it is not a failure of the merge, only its
/// turn has not come, and an owner's ask stands until it does.
pub(crate) fn merge(
    s: &pulse_store::Store,
    project: &Row,
    number: i64,
    by: &str,
    actor: &str,
    now: &str,
) -> Result<String, String> {
    let slug = project.str("slug");
    let known = s
        .conn()
        .ok()
        .and_then(|c| pr_state(&c, &slug, number))
        .map(|(pr, _)| pr);
    let def = crate::baseline::default_branch(project.clone());
    if let Some(below) = known.as_ref().and_then(|p| stacked_on(p, &def)) {
        return Err(format!(
            "pull request #{number} is stacked on {below}; {below} must merge first"
        ));
    }
    let url = known.as_ref().map(|p| p.url.clone()).unwrap_or_default();
    let repo = repo_of(project, &url);
    let result = gh(
        s,
        &slug,
        &repo,
        &[
            "pr",
            "merge",
            &number.to_string(),
            "--merge",
            "--delete-branch",
        ],
        MERGE_TIMEOUT,
    );
    let job = std::env::var("PULSE_JOB").unwrap_or_default();
    match result {
        Ok(_) => {
            let mut detail = json!({"project": slug, "number": number, "url": url, "by": by});
            if !job.is_empty() {
                detail["job"] = json!(job);
            }
            event(s, now, actor, "review.merged", &slug, detail);
            Ok(url)
        }
        Err(e) => {
            let e = pulse_cred::redact(&e, &[]);
            event(
                s,
                now,
                actor,
                "review.failed",
                &slug,
                json!({"project": slug, "number": number, "url": url, "error": e}),
            );
            Err(e)
        }
    }
}

/// One project's pass: review jobs that ended without a verdict become
/// `review.failed`; then, when no review job is queued or running, the
/// oldest open pull request gets one (or, after `MAX_ATTEMPTS` failed
/// reviews of its head, goes to the owner). A pull request stacked on
/// another waits: the stack is reviewed bottom up, and delivery moves it
/// onto the default branch when the one below merges.
fn tend_project(s: &pulse_store::Store, project: &Row, now: &str) -> Vec<String> {
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
    s: &pulse_store::Store,
    conn: &pulse_store::PooledConn,
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
        "('review.merged', 'review.changes', 'review.owner', 'review.failed')",
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
    s: &pulse_store::Store,
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
        "Merge it from the task's sheet or the pull request's card in the hub, or with `pulse review merge {n} --by owner --project {slug}`; close it on GitHub to drop it."
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
        let _ = pulse_tasks::send_message(s, &slug, &pr.task, actor, &msg);
    }
    post
}

/// The follow-up job after a `changes` verdict: a task pull request gets a
/// `task` job on the task's source (the delivery folds its branch into the
/// task's pull request); any other kind gets an `improve` job on
/// `pr-fix:<slug>:<number>` (the delivery pushes its branch onto the pull
/// request's head). Both briefs start by putting the clone on the head.
pub(crate) fn queue_follow_up(
    s: &pulse_store::Store,
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
        "Run `pulse review checkout {n}` first so your work sits on the pull request's head.\n\n\
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
/// own author, and PULSE opens its pull requests as the same gh account
/// that reviews them, so a refused `--request-changes` becomes a comment.
pub(crate) fn request_changes(
    s: &pulse_store::Store,
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
    let body = format!(
        "Changes requested by the PULSE reviewer:\n\n{}",
        note.trim()
    );
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
        let _ = pulse_tasks::send_message(
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
    s: &pulse_store::Store,
    project: &Row,
    pr: &OpenPr,
    reasons: &[String],
    note: &str,
    actor: &str,
    now: &str,
) -> Result<String, String> {
    let slug = project.str("slug");
    let repo = repo_of(project, &pr.url);
    let mut body = String::from("The PULSE reviewer handed this pull request to the owner.");
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

fn fail(s: &pulse_store::Store, slug: &str, pr: &OpenPr, error: &str, actor: &str, now: &str) {
    event(
        s,
        now,
        actor,
        "review.failed",
        slug,
        json!({"project": slug, "number": pr.number, "url": pr.url, "error": pulse_cred::redact(error, &[])}),
    );
}

/// Puts the pull request's head on `review/pr-N` in `dir` (a job clone)
/// and checks it out. The head is fetched from the project's local checkout
/// first - the branch the daemon pushed is there and the clone has no
/// remote of its own - and from the upstream's `refs/pull/N/head` when the
/// local branch is not at the pull request's head commit (the daemon's
/// checkout may be behind a push made elsewhere).
pub(crate) fn checkout_into(
    s: &pulse_store::Store,
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
    if have.is_empty() || (!view.head_sha.is_empty() && have != view.head_sha) {
        let url = upstream_url(project, &view.url);
        if url.is_empty() {
            return Err(format!(
                "the local checkout does not hold {} at {} and no upstream url is known",
                view.head,
                short(&view.head_sha)
            ));
        }
        let (env, cleanup, why) = crate::forge::push_credential_env(s, root.clone());
        if !why.is_empty() {
            return Err(why);
        }
        let pull_ref = format!("+refs/pull/{number}/head:refs/heads/{branch}");
        let fetched = crate::util::git_timeout_env(
            dir,
            Duration::from_secs(300),
            &env,
            &["fetch", "--quiet", "--no-tags", &url, &pull_ref],
        );
        cleanup("", &[]);
        if !fetched.ok() {
            return Err(pulse_cred::redact(
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
    /// logged.
    fn fake_gh() -> (
        tempfile::TempDir,
        pulse_testutil::SetenvGuard,
        String,
        std::path::PathBuf,
    ) {
        let bin = tempfile::tempdir().unwrap();
        let log = bin.path().join("gh.log").display().to_string();
        let view = bin.path().join("view");
        std::fs::write(
            &view,
            view_json(7, "pulse/task/TASK-001", &[("src/a.rs", 5, 2)]),
        )
        .unwrap();
        let fail = bin.path().join("fail");
        std::fs::write(&fail, "").unwrap();
        let body = format!(
            "#!/bin/sh\necho \"$@\" >> '{log}'\nif grep -q \"^$2$\" '{fail}'; then echo \"$(cat '{failmsg}')\" >&2; exit 1; fi\ncase \"$1 $2\" in\n\"pr view\") cat '{view}' ;;\n\"pr diff\") echo 'diff --git a/src/a.rs b/src/a.rs' ;;\n\"pr merge\") echo merged ;;\n\"pr review\") echo reviewed ;;\n\"pr comment\") echo commented ;;\n\"auth status\") exit 1 ;;\n*) echo \"unexpected gh $*\" >&2; exit 2 ;;\nesac\n",
            view = view.display(),
            fail = fail.display(),
            failmsg = bin.path().join("failmsg").display(),
        );
        std::fs::write(bin.path().join("failmsg"), "HTTP 422: boom").unwrap();
        let path = bin.path().join("gh");
        std::fs::write(&path, body).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let guard = pulse_testutil::setenv(
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
        let adds: i64 = files.iter().map(|f| f.1).sum();
        let dels: i64 = files.iter().map(|f| f.2).sum();
        json!({
            "number": number, "title": format!("PR {number} title"), "body": "what it does",
            "url": format!("https://github.com/o/r/pull/{number}"), "state": "OPEN",
            "headRefName": head, "headRefOid": "abc123", "baseRefName": "main",
            "author": {"login": "pulse-bot"}, "additions": adds, "deletions": dels,
            "files": files.iter().map(|(p, a, d)| json!({"path": p, "additions": a, "deletions": d})).collect::<Vec<_>>(),
        })
        .to_string()
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

    fn land_pr(s: &pulse_store::Store, number: i64, kind: &str, task: &str, job: &str) {
        let head = if kind == "task" {
            format!("pulse/task/{task}")
        } else {
            format!("pulse/worker/{job}")
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

    fn events(s: &pulse_store::Store, action: &str) -> Vec<Value> {
        crate::util::must_rows(
            &*s.conn().unwrap(),
            "SELECT detail FROM events WHERE action = ? ORDER BY seq",
            &[&action],
        )
        .iter()
        .map(|r| serde_json::from_str(&r.str("detail")).unwrap())
        .collect()
    }

    fn jobs_of(s: &pulse_store::Store, kind: &str) -> Vec<Row> {
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
            "pulse/task/TASK-001",
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
        let big = view_of(7, "pulse/x/y", &files);
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
            "pulse/x/y",
            &[("src/a.rs", 400, 201), ("web/hub/out/x.js", 5000, 5000)],
        );
        assert!(reasons_for(&s, &long).is_empty());
        assert_eq!(
            care_for(&s, &long),
            ["1 files / 601 lines changed (the usual reading is 20 files / 600 lines)"]
        );
        set_setting(&s, "review.max_lines", "700");
        assert!(care_for(&s, &view_of(7, "pulse/x/y", &[("src/a.rs", 400, 201)])).is_empty());

        let r = reasons_for(
            &s,
            &view_of(
                7,
                "pulse/x/y",
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
            &view_of(7, "pulse/x/y", &[("docs/decisions/adr-1.md", 1, 0)]),
        );
        assert_eq!(
            r,
            ["touches docs/decisions/adr-1.md"],
            "the owner's paths are guarded on a pulse/ branch"
        );

        let mostly_deletes = view_of(7, "pulse/x/y", &[("src/a.rs", 10, 250)]);
        assert!(reasons_for(&s, &mostly_deletes).is_empty());
        assert_eq!(
            care_for(&s, &mostly_deletes),
            ["removes more than it adds (250 lines removed, 10 added)"]
        );
        assert!(care_for(&s, &view_of(7, "pulse/x/y", &[("src/a.rs", 100, 150)])).is_empty());

        let r = reasons_for(&s, &view_of(7, "feature/by-hand", &[("src/a.rs", 1, 0)]));
        assert_eq!(r, ["opened by a human/other tool (branch feature/by-hand)"]);
        assert!(
            reasons_for(&s, &view_of(7, "evolve/B-013", &[("src/a.rs", 1, 0)])).is_empty(),
            "a kept experiment's branch is PULSE's own"
        );

        set_setting(&s, "review.merge", "ask");
        let r = reasons_for(&s, &view_of(7, "pulse/x/y", &[("src/a.rs", 1, 0)]));
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
            head: "pulse/x/y".into(),
            base: "main".into(),
            ..Default::default()
        };
        let view = view_of(7, "pulse/x/y", &[("data/words.json", 900, 100)]);
        let care = care_for(&s, &view);
        let brief = review_brief("proj", &pr, &view, &[], &care, "npm test");
        assert!(brief.contains("You may merge it yourself"), "{brief}");
        assert!(
            brief.contains("Read it with extra care: 1 files / 1000 lines changed"),
            "{brief}"
        );
        assert!(brief.contains("`pulse review merge 7`"), "{brief}");
        assert!(!brief.contains("You may NOT merge"), "{brief}");
    }

    #[test]
    fn one_review_job_per_project_and_none_while_one_runs() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
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
        assert!(brief.contains("pulse review show 7"), "{brief}");
        assert!(brief.contains("pulse review checkout 7"), "{brief}");
        assert!(brief.contains("pulse review diff 7"), "{brief}");
        assert!(brief.contains("pulse review merge 7"), "{brief}");
        assert!(brief.contains("pulse review changes 7 --note"), "{brief}");
        assert!(brief.contains("pulse review owner 7 --note"), "{brief}");
        assert!(
            brief.contains("pulse job finish $PULSE_JOB done --result"),
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
        let _env = pulse_testutil::env_lock();
        let (_bin, _path, _log, view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "experiment", "", "JOB-a");
        std::fs::write(
            &view,
            view_json(
                7,
                "pulse/worker/JOB-a",
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
        assert!(!brief.contains("`pulse review changes 7"), "{brief}");
        assert_eq!(
            events(&s, "review.queued")[0]["reasons"],
            json!(["touches .github/workflows/ci.yml"])
        );
    }

    #[test]
    fn the_owners_merge_click_merges_as_owner_once() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
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
        assert_eq!(calls(&log), ["pr merge 7"]);
        assert!(
            jobs_of(&s, "review").is_empty(),
            "a merged pull request needs no review"
        );

        // Settled: the ask is not honoured twice.
        let lines = tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(calls(&log), ["pr merge 7"]);
    }

    #[test]
    fn a_failed_owner_merge_is_recorded_and_not_retried() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
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

    #[test]
    fn a_review_without_a_verdict_fails_is_retried_once_then_goes_to_the_owner() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let (_bin, _path, _log, _view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        let task = pulse_tasks::new(
            &s,
            "proj",
            "Add the thing",
            "b",
            0,
            pulse_store::HUMAN_AGENT,
        )
        .unwrap();
        land_pr(&s, 7, "task", &task.id, "JOB-a");
        let tend = |s: &pulse_store::Store| tend_reviews(s, std::slice::from_ref(&proj), TEST_NOW);

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
        let msgs = pulse_tasks::messages(&s, "proj", &task.id, "", "").unwrap();
        assert!(
            msgs.iter().any(|m| m.body.contains("waits for your merge")),
            "{msgs:?}"
        );
        // Settled: nothing more.
        assert!(tend(&s).is_empty());
        assert_eq!(events(&s, "review.owner").len(), 1);
    }

    #[test]
    fn a_new_head_while_the_review_runs_waits_for_the_verdict() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
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
        let mut d = json!({"project": "proj", "kind": "job", "job": "JOB-a", "number": 7, "url": "https://github.com/o/r/pull/7", "head": "pulse/worker/JOB-a", "base": "main", "to": "def456", "opened": false});
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
        assert!(!generated_path("crates/pulse-runner/src/review.rs"));
    }

    #[test]
    fn checkout_fetches_the_head_from_the_local_checkout_then_the_upstream() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let job = push_branch(&s, &proj, &repo, "pulse/worker/a");
        let _ = job;
        let sha = ref_of(&repo, "refs/heads/pulse/worker/a");
        let clone = std::path::Path::new(&s.home)
            .join("tmp")
            .join("clone-review");
        let clone = clone.display().to_string();
        run(".", &["git", "clone", "-q", &repo, &clone]);
        run(&clone, &["git", "remote", "remove", "origin"]);

        let mut view = view_of(7, "pulse/worker/a", &[("x", 1, 0)]);
        view.head_sha = sha.clone();
        let got = checkout_into(&s, &proj, &view, &clone).unwrap();
        assert_eq!(got, sha);
        assert_eq!(crate::util::git_branch(&clone), "review/pr-7");
        assert_eq!(ref_of(&clone, "HEAD"), sha);

        // The local branch is behind the pull request (pushed elsewhere):
        // the head comes from the upstream's refs/pull/N/head.
        run(&repo, &["git", "checkout", "-q", "pulse/worker/a"]);
        std::fs::write(std::path::Path::new(&repo).join("more.txt"), "more\n").unwrap();
        run(&repo, &["git", "add", "-A"]);
        run(&repo, &["git", "commit", "-q", "-m", "more"]);
        let newer = ref_of(&repo, "refs/heads/pulse/worker/a");
        run(
            &repo,
            &[
                "git",
                "push",
                "-q",
                "origin",
                "pulse/worker/a:refs/pull/7/head",
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
        let _env = pulse_testutil::env_lock();
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
                "url": "https://github.com/o/r/pull/8", "head": "pulse/worker/JOB-b",
                "base": "pulse/task/TASK-001", "stack_on": 7, "to": "abc123", "opened": true,
                "title": "PR 8 title"}),
        );

        let lines = tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert_eq!(
            lines,
            ["review proj: #8 stacked on #7: reviewed after #7 merges"]
        );
        assert!(jobs_of(&s, "review").is_empty());
        assert_eq!(
            merge_as_owner(&s, &proj, 8, TEST_NOW).unwrap_err(),
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
                "url": "https://github.com/o/r/pull/8", "head": "pulse/worker/JOB-b",
                "base": "main", "to": "abc123", "opened": false,
                "retargeted_from": "pulse/task/TASK-001", "title": "PR 8 title"}),
        );
        let lines = tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains("#8 queued for review")),
            "{lines:?}"
        );
        assert_eq!(jobs_of(&s, "review").len(), 1);
    }
}
