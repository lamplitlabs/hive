//! Every finished branch of a project delivered as a pull request on its
//! upstream, and every pull request PULSE opened followed until the forge
//! settles it. Nothing merges into the local default branch on this
//! machine: it only follows the upstream (`forge::sync_default_from_upstream`).
//!
//! What is delivered, each pass (`tend`):
//! - a task whose jobs are all finished: the jobs' branches merged onto the
//!   default branch as `pulse/task-<id>`, one pull request (`land.pr`,
//!   kind `task`);
//! - every other done job that still has a branch, one pull request each
//!   from the branch as the agent left it (kind `job`) - 92% of the 260
//!   landings of the week before this (suggestion, security, lead, schedule,
//!   research...) were not task work and the per-task flow alone would have
//!   stranded them;
//! - a `pr-fix:<slug>:<n>` job: its branch pushed onto pull request n's head
//!   (`land.pr`, `opened` false);
//! - a conversation branch (`pulse/chat-*`) with commits the default branch
//!   has not got, once its last commit is ten minutes old (kind `proposed`);
//! - a kept experiment, from the experiment lane (`experiment.rs`, kind
//!   `experiment`), and a branch named by hand (`pulse land --branch`, kind
//!   `proposed`).
//!
//! The same pass reads the state of every open pull request in one `gh pr
//! list` per repository. Merged: `land.merged`, the delivered jobs lose
//! their branch, a task is done, the local branches go and the default
//! branch is fast-forwarded from the upstream. Closed unmerged:
//! `land.pr-closed`, the branches are cleared too (the work stays on the
//! forge's closed pull request; nothing re-delivers it) and a task goes back
//! to in_progress with a note for its lead.
//!
//! A project without an upstream (two of the fifteen live ones, both on a
//! feature branch with no remote) can have no pull request: its ready
//! branches land locally through `land::land_branch` as before, with
//! `land.landed`.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{Value, json};

use crate::forge::Upstream;
use crate::util::Row;

pub(crate) const ACTOR: &str = "daemon";

/// Pull requests opened per project per pass at most: GitHub's secondary
/// rate limit counts content creation per minute, and the pass is back in
/// two.
const CREATIONS_PER_PASS: usize = 10;

/// How old a conversation branch's last commit must be before the branch
/// is proposed: chat agents commit in bursts, and a pull request opened
/// between two commits of one burst is updated a minute later.
const CONVERSATION_SETTLE_SECS: i64 = 600;

/// The branch a task's pull request comes from.
pub(crate) fn task_branch(task: &str) -> String {
    format!("pulse/task-{task}")
}

/// One pass over the projects: settle the pull requests the forge merged or
/// closed, deliver the work now owed, bring every default branch up to its
/// upstream. Returns the log lines.
pub(crate) fn tend(s: &pulse_store::Store, projects: &[Row], now: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for p in projects {
        if p.str("path").is_empty() {
            continue;
        }
        lines.extend(poll_open(s, p, now));
        lines.extend(deliver(s, p, now));
        if let Some(line) = crate::forge::sync_default_from_upstream(s, p, ACTOR, now, false) {
            lines.push(line);
        }
    }
    lines
}

/// A task whose jobs are all finished with at least one done job whose
/// branch its pull request has not got yet: no pull request open, or one
/// open that a later job (the reviewer's "changes" queue a follow-up `task:`
/// job on its head) must be pushed onto.
#[derive(Debug, Clone)]
struct ReadyTask {
    task: String,
    title: String,
    /// Every done job with a branch, as (job id, branch), in creation order.
    branches: Vec<(String, String)>,
    /// The task's pull request still open, as its `land.pr` detail.
    open: Option<Value>,
}

fn ready_tasks(s: &pulse_store::Store, slug: &str) -> Vec<ReadyTask> {
    let Ok(conn) = s.conn() else {
        return Vec::new();
    };
    let tasks = crate::util::query_rows(
        &conn,
        "SELECT id, title FROM tasks WHERE project = ? AND status NOT IN ('done', 'cancelled') ORDER BY id",
        &[&slug],
    )
    .unwrap_or_default();
    let mut out = Vec::new();
    for t in tasks {
        let id = t.str("id");
        let (exact, like) = pulse_store::task_source_params(&id);
        let jobs = crate::util::query_rows(
            &conn,
            &format!(
                "SELECT id, status, branch FROM jobs WHERE project = ? AND {} ORDER BY created_at, id",
                pulse_store::task_source_clause("source")
            ),
            &[&slug, &exact, &like],
        )
        .unwrap_or_default();
        if jobs
            .iter()
            .any(|j| matches!(j.str("status").as_str(), "queued" | "leased" | "blocked"))
        {
            continue;
        }
        let branches: Vec<(String, String)> = jobs
            .iter()
            .filter(|j| j.str("status") == "done" && !j.str("branch").is_empty())
            .map(|j| (j.str("id"), j.str("branch")))
            .collect();
        if branches.is_empty() {
            continue;
        }
        let open = open_pr(&conn, slug, &id);
        if let Some(pr) = &open {
            let delivered = jobs_of(pr);
            if branches.iter().all(|(j, _)| delivered.contains(j)) {
                continue;
            }
        }
        out.push(ReadyTask {
            task: id,
            title: t.str("title"),
            branches,
            open,
        });
    }
    out
}

/// A finished job's branch waiting to go: a job of its own, or a fix for
/// pull request `fix_of`.
#[derive(Debug, Clone)]
struct ReadyJob {
    job: Row,
    fix_of: Option<i64>,
}

/// A conversation branch with commits the default branch has not got.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ChatBranch {
    branch: String,
    sha: String,
    /// Commits the default branch has not got (by patch, `git cherry`).
    ahead: usize,
    /// Seconds since the branch's last commit, by its committer date.
    age: i64,
}

impl ChatBranch {
    fn settled(&self) -> bool {
        self.age >= CONVERSATION_SETTLE_SECS
    }
    /// The chat id a hub conversation branch is named after.
    fn conversation(&self) -> Option<String> {
        self.branch
            .strip_prefix("pulse/chat-")
            .filter(|id| !id.is_empty())
            .map(str::to_string)
    }
}

/// Everything a project has waiting to be delivered.
#[derive(Debug, Default)]
struct Ready {
    tasks: Vec<ReadyTask>,
    jobs: Vec<ReadyJob>,
    conversations: Vec<ChatBranch>,
}

/// The number in a `pr-fix:<slug>:<n>` source.
fn fix_number(source: &str) -> Option<i64> {
    source
        .strip_prefix("pr-fix:")?
        .rsplit(':')
        .next()?
        .parse::<i64>()
        .ok()
}

/// Whether a done job's branch is delivered on its own: task work goes with
/// its task, review jobs never produce a branch, experiments go through the
/// experiment lane once measured.
fn delivered_alone(job: &Row) -> bool {
    let source = job.str("source");
    !matches!(job.str("kind").as_str(), "review" | "experiment")
        && !source.starts_with("task:")
        && !source.starts_with("review:")
}

fn ready_work(s: &pulse_store::Store, project: &Row, now: &str) -> Ready {
    let slug = project.str("slug");
    let root = project.str("path");
    let def = crate::baseline::default_branch(project.clone());
    let mut ready = Ready {
        tasks: ready_tasks(s, &slug),
        ..Default::default()
    };
    if let Ok(conn) = s.conn() {
        for job in crate::land::pending_landings(&conn, &slug) {
            if !delivered_alone(&job) {
                continue;
            }
            let fix_of = fix_number(&job.str("source"));
            ready.jobs.push(ReadyJob { job, fix_of });
        }
    }
    let now_unix = crate::util::must_iso(now).timestamp();
    ready.conversations = conversation_branches(&root, &def, now_unix);
    ready
}

/// The `pulse/chat-*` branches with commits the default branch has not
/// got: `for-each-ref --no-merged` names the candidates in one call, `git
/// cherry` drops the ones whose commits are all on the default branch by
/// patch already (a conversation the owner merged by hand).
fn conversation_branches(root: &str, def: &str, now_unix: i64) -> Vec<ChatBranch> {
    let refs = crate::util::git_timeout(
        root,
        Duration::from_secs(30),
        &[
            "for-each-ref",
            "--format=%(refname:short)%09%(objectname)%09%(committerdate:unix)",
            &format!("--no-merged=refs/heads/{def}"),
            "refs/heads/pulse/chat-*",
        ],
    );
    if !refs.ok() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for line in refs.stdout.lines() {
        let mut parts = line.split('\t');
        let (Some(branch), Some(sha), Some(date)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let cherry =
            crate::util::git_timeout(root, Duration::from_secs(60), &["cherry", def, branch]);
        if !cherry.ok() {
            continue;
        }
        let ahead = cherry.stdout.lines().filter(|l| l.starts_with('+')).count();
        if ahead == 0 {
            continue;
        }
        let committed = date.trim().parse::<i64>().unwrap_or(now_unix);
        out.push(ChatBranch {
            branch: branch.to_string(),
            sha: sha.to_string(),
            ahead,
            age: now_unix - committed,
        });
    }
    out
}

/// The task's pull request still open as far as PULSE knows: the newest
/// `land.pr` of the task with no `land.merged` or `land.pr-closed` after it,
/// as its event detail.
pub(crate) fn open_pr(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    task: &str,
) -> Option<Value> {
    let (action, detail) = last_event(q, slug, "task", task, false)?;
    (action == "land.pr").then_some(detail)
}

/// The job's pull request still open as far as PULSE knows, as its `land.pr`
/// detail: the experiment lane leaves such a job alone.
pub(crate) fn open_pr_for_job(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    job: &str,
) -> Option<Value> {
    let (action, detail) = last_event(q, slug, "job", job, false)?;
    (action == "land.pr").then_some(detail)
}

/// The newest delivery event about one piece of work - the task, job,
/// experiment or head branch named by `field` - as (action, detail). A job
/// is also found in the `jobs` list of a pull request that carried it (a
/// task's or a fix's). With `holds` the `land.pr-held` notes count too.
fn last_event(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    field: &str,
    value: &str,
    holds: bool,
) -> Option<(String, Value)> {
    let path = match field {
        "task" => "$.task",
        "job" => "$.job",
        "experiment" => "$.experiment",
        "head" => "$.head",
        _ => return None,
    };
    let actions = if holds {
        "'land.pr', 'land.pr-held', 'land.merged', 'land.pr-closed'"
    } else {
        "'land.pr', 'land.merged', 'land.pr-closed'"
    };
    let in_jobs = if field == "job" {
        " OR EXISTS (SELECT 1 FROM json_each(detail, '$.jobs') WHERE value = ?2)"
    } else {
        ""
    };
    let rows = crate::util::query_rows(
        q,
        &format!(
            "SELECT action, detail FROM events WHERE subject = ?1 AND action IN ({actions})
             AND json_valid(detail) AND (json_extract(detail, '{path}') = ?2{in_jobs})
             ORDER BY seq DESC LIMIT 1"
        ),
        &[&slug, &value],
    )
    .unwrap_or_default();
    let last = rows.first()?;
    Some((
        last.str("action"),
        serde_json::from_str(&last.str("detail")).unwrap_or(Value::Null),
    ))
}

/// The newest `land.pr` of pull request `number`, as its detail.
fn pr_event(q: &(impl crate::util::Querier + ?Sized), slug: &str, number: i64) -> Option<Value> {
    let rows = crate::util::query_rows(
        q,
        "SELECT detail FROM events WHERE subject = ? AND action = 'land.pr' AND json_valid(detail)
         AND json_extract(detail, '$.number') = ? ORDER BY seq DESC LIMIT 1",
        &[&slug, &number],
    )
    .unwrap_or_default();
    rows.first()
        .map(|r| serde_json::from_str(&r.str("detail")).unwrap_or(Value::Null))
}

fn field<'a>(v: &'a Value, name: &str) -> &'a str {
    v.get(name).and_then(Value::as_str).unwrap_or_default()
}

fn number_of(v: &Value) -> i64 {
    v.get("number").and_then(Value::as_i64).unwrap_or(0)
}

/// The task's last pull request was closed unmerged and no job of the task
/// has been queued since: the lead plans again before the task can close.
pub(crate) fn closed_awaiting_work(s: &pulse_store::Store, slug: &str, task: &str) -> bool {
    let Ok(conn) = s.conn() else {
        return false;
    };
    let last = crate::util::query_rows(
        &conn,
        "SELECT action, at FROM events WHERE subject = ? AND action IN ('land.pr', 'land.merged', 'land.pr-closed')
         AND json_extract(detail, '$.task') = ? ORDER BY seq DESC LIMIT 1",
        &[&slug, &task],
    )
    .unwrap_or_default();
    let Some(last) = last.first() else {
        return false;
    };
    if last.str("action") != "land.pr-closed" {
        return false;
    }
    let (exact, like) = pulse_store::task_source_params(task);
    let at = last.str("at");
    crate::util::query_rows(
        &conn,
        &format!(
            "SELECT id FROM jobs WHERE project = ? AND created_at > ? AND {} LIMIT 1",
            pulse_store::task_source_clause("source")
        ),
        &[&slug, &at, &exact, &like],
    )
    .unwrap_or_default()
    .is_empty()
}

fn event(s: &pulse_store::Store, now: &str, action: &str, slug: &str, detail: Value) {
    if let Ok(conn) = s.conn() {
        crate::util::event_at(&conn, now, ACTOR, action, slug, detail);
    }
}

/// A hold already recorded for exactly this work (`key`), newer than any
/// pull request of it: the pass does not retry a conflict or a push failure
/// until the work changes.
fn held_already(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    field: &str,
    value: &str,
    key: &str,
) -> bool {
    last_event(q, slug, field, value, true)
        .is_some_and(|(action, d)| action == "land.pr-held" && field_of(&d, "work") == key)
}

fn field_of(v: &Value, name: &str) -> String {
    field(v, name).to_string()
}

fn hold(s: &pulse_store::Store, now: &str, slug: &str, mut detail: Value, note: &str) {
    detail["project"] = json!(slug);
    detail["note"] = json!(note);
    event(s, now, "land.pr-held", slug, detail);
}

/// What the next delivery of a piece of work is, given its newest event
/// and the branch's sha now.
#[derive(Debug, PartialEq, Eq)]
enum Next {
    /// Delivered at this sha already (or held for it), or the pull request
    /// has moved past it.
    Nothing,
    /// A pull request is open for an older sha: push the new commits to it.
    Update(Value),
    /// Nothing open: a pull request to open.
    Open,
}

fn next_for(
    q: &(impl crate::util::Querier + ?Sized),
    root: &str,
    slug: &str,
    field: &str,
    value: &str,
    sha: &str,
    held_key: &str,
) -> Next {
    if held_already(q, slug, field, value, held_key) {
        return Next::Nothing;
    }
    match last_event(q, slug, field, value, false) {
        None => Next::Open,
        Some((action, d)) if action == "land.pr" => {
            let to = field_of(&d, "to");
            // The pull request's head past the local branch is a fix pushed
            // onto it (`pr-fix`): the local branch is behind, not new work,
            // and pushing it would take the fix off the pull request.
            if to == sha || is_ancestor(root, sha, &to) {
                Next::Nothing
            } else {
                Next::Update(d)
            }
        }
        // Merged or closed at this very sha: delivered; new commits since
        // are a new pull request.
        Some((_, d)) => {
            if pr_event(q, slug, number_of(&d)).is_some_and(|pr| field_of(&pr, "to") == sha) {
                Next::Nothing
            } else {
                Next::Open
            }
        }
    }
}

/// Whether `old` is an ancestor of `new` (both known to the checkout).
fn is_ancestor(root: &str, old: &str, new: &str) -> bool {
    !old.is_empty()
        && !new.is_empty()
        && crate::util::git_timeout(
            root,
            Duration::from_secs(20),
            &["merge-base", "--is-ancestor", old, new],
        )
        .ok()
}

fn work_key(branch: &str, sha: &str) -> String {
    format!("{branch} {}", crate::recipe::short(sha, 12))
}

/// Delivers what the project has waiting: pull requests when it has an
/// upstream, local landings when it has none.
fn deliver(s: &pulse_store::Store, project: &Row, now: &str) -> Vec<String> {
    let slug = project.str("slug");
    let root = project.str("path");
    let def = crate::baseline::default_branch(project.clone());
    let Some(up) = crate::forge::upstream_of(&root, &def) else {
        return land_locally(s, project, now);
    };
    let ready = ready_work(s, project, now);
    let mut lines = Vec::new();
    let mut opened = 0usize;
    for rt in &ready.tasks {
        let task = &rt.task;
        let key = rt
            .branches
            .iter()
            .map(|(_, b)| b.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let Ok(conn) = s.conn() else { break };
        if held_already(&conn, &slug, "task", task, &key) {
            continue;
        }
        drop(conn);
        if rt.open.is_none() {
            if opened >= CREATIONS_PER_PASS {
                break;
            }
            opened += 1;
        }
        match deliver_task(s, project, &up, rt, now) {
            Ok(line) => lines.push(line),
            Err(note) => {
                hold(s, now, &slug, json!({"task": task, "work": key}), &note);
                lines.push(format!("task pr {slug} {task}: held: {note}"));
            }
        }
    }
    for rj in &ready.jobs {
        let job = &rj.job;
        let id = job.str("id");
        let branch = job.str("branch");
        let (exists, why) = crate::land::branch_exists(root.clone(), branch.clone());
        if !exists {
            if !why.is_empty() {
                let Ok(conn) = s.conn() else { break };
                if !held_already(&conn, &slug, "job", &id, &branch) {
                    drop(conn);
                    hold(s, now, &slug, json!({"job": id, "work": branch}), &why);
                    lines.push(format!("job pr {slug} {id}: held: {why}"));
                }
                continue;
            }
            // The branch is gone from the checkout (a run that never copied
            // its commits, a branch deleted by hand): the job is told and
            // forgets the branch, instead of being retried every pass.
            let note = format!("branch {branch} is not in {root}");
            hold(s, now, &slug, json!({"job": id, "work": branch}), &note);
            let said = crate::land::forget_missing_branch(s, job, ACTOR, now);
            lines.push(format!("job pr {slug} {id}: {note}; {said}"));
            continue;
        }
        let sha = rev(&root, &format!("refs/heads/{branch}"));
        let key = work_key(&branch, &sha);
        let Ok(conn) = s.conn() else { break };
        let next = next_for(&conn, &root, &slug, "job", &id, &sha, &key);
        drop(conn);
        if next == Next::Nothing {
            continue;
        }
        let res = match rj.fix_of {
            Some(n) => deliver_fix(s, project, &up, job, n, &sha, now),
            None => {
                if matches!(next, Next::Open) {
                    if opened >= CREATIONS_PER_PASS {
                        break;
                    }
                    opened += 1;
                }
                deliver_job(s, project, &up, job, &sha, next, now).map(|d| d.line)
            }
        };
        match res {
            Ok(line) => lines.push(line),
            Err(note) => {
                hold(s, now, &slug, json!({"job": id, "work": key}), &note);
                lines.push(format!("job pr {slug} {id}: held: {note}"));
            }
        }
    }
    for cb in &ready.conversations {
        if !cb.settled() {
            continue;
        }
        let key = work_key(&cb.branch, &cb.sha);
        let Ok(conn) = s.conn() else { break };
        let next = next_for(&conn, &root, &slug, "head", &cb.branch, &cb.sha, &key);
        drop(conn);
        if next == Next::Nothing {
            continue;
        }
        if matches!(next, Next::Open) {
            if opened >= CREATIONS_PER_PASS {
                break;
            }
            opened += 1;
        }
        match deliver_proposed(
            s,
            project,
            &up,
            &cb.branch,
            &cb.sha,
            cb.conversation(),
            next,
            now,
        ) {
            Ok(d) => lines.push(d.line),
            Err(note) => {
                let mut d = json!({"head": cb.branch, "work": key});
                if let Some(id) = cb.conversation() {
                    d["conversation"] = json!(id);
                }
                hold(s, now, &slug, d, &note);
                lines.push(format!("proposed {slug} {}: held: {note}", cb.branch));
            }
        }
    }
    lines
}

/// What `pulse land --dry-run` prints: the work a pass would deliver.
pub(crate) fn dry_run(s: &pulse_store::Store, project: &Row, now: &str) -> Vec<String> {
    let slug = project.str("slug");
    let root = project.str("path");
    let def = crate::baseline::default_branch(project.clone());
    let upstream = crate::forge::upstream_of(&root, &def);
    let ready = ready_work(s, project, now);
    let mut lines = Vec::new();
    match &upstream {
        Some(up) => lines.push(format!(
            "{slug}: pull requests go to {} ({})",
            up.name(),
            up.url
        )),
        None => lines.push(format!(
            "{slug}: no upstream, finished branches land locally on {def}"
        )),
    }
    for rt in &ready.tasks {
        lines.push(format!(
            "  task {} ({}): {} as {}{}",
            rt.task,
            rt.title,
            rt.branches
                .iter()
                .map(|(j, b)| format!("{j} {b}"))
                .collect::<Vec<_>>()
                .join(", "),
            task_branch(&rt.task),
            match &rt.open {
                Some(pr) => format!(" -> push onto pull request #{}", number_of(pr)),
                None => " -> one pull request".to_string(),
            }
        ));
    }
    for rj in &ready.jobs {
        let job = &rj.job;
        let what = match (&upstream, rj.fix_of) {
            (None, _) => format!("lands locally on {def}"),
            (Some(_), Some(n)) => format!("push onto pull request #{n}"),
            (Some(_), None) => "one pull request".to_string(),
        };
        lines.push(format!(
            "  job {} ({}): {} -> {what}",
            job.str("id"),
            job.str("title"),
            job.str("branch")
        ));
    }
    for cb in &ready.conversations {
        let minutes = (cb.age / 60).max(0) as i32;
        lines.push(format!(
            "  conversation {}: {} {} ahead of {def}, last commit {minutes} {} ago{}",
            cb.branch,
            cb.ahead,
            crate::util::plural(cb.ahead as i32, "commit"),
            crate::util::plural(minutes, "minute"),
            if cb.settled() {
                ""
            } else {
                " (waits until it is 10 minutes old)"
            }
        ));
    }
    if lines.len() == 1 {
        lines.push(format!("  nothing waiting for {slug}"));
    }
    lines
}

/// The old local landing for a project that has no upstream to open a
/// pull request on: `land::land_pending_pass` lands the plain branches one
/// by one (experiments stay with the experiment lane) and records
/// `land.landed`, so the hub keeps showing them merged on this machine.
fn land_locally(s: &pulse_store::Store, project: &Row, now: &str) -> Vec<String> {
    let slug = project.str("slug");
    let Ok(conn) = s.conn() else {
        return Vec::new();
    };
    let waiting = crate::land::pending_landings(&conn, &slug)
        .into_iter()
        .filter(|j| j.str("kind") != "experiment")
        .count();
    drop(conn);
    if waiting == 0 {
        return Vec::new();
    }
    let mut lines = vec![format!("{slug}: no upstream, landing locally")];
    let ctx = pulse_bus::CancellationToken::new();
    for r in crate::land::land_pending_pass(
        &ctx,
        s,
        project.clone(),
        ACTOR.to_string(),
        now.to_string(),
        true,
        0,
        false,
        false,
    ) {
        lines.push(format!("{slug}: {} {}", r.state, r.note));
    }
    lines
}

fn rev(root: &str, r: &str) -> String {
    crate::util::git_timeout(root, Duration::from_secs(10), &["rev-parse", "--verify", r])
        .stdout
        .trim()
        .to_string()
}

/// One pull request's worth of work: what its `land.pr` says and what its
/// body tells the reviewer.
struct Work<'a> {
    kind: &'a str,
    /// The ids the detail carries beside `project`: `task`, `job`,
    /// `experiment`, `conversation`, `archive`, `run_id`, `fix`.
    ids: Vec<(&'a str, String)>,
    /// The jobs the pull request delivers; merging it clears their branches.
    jobs: Vec<String>,
    title: String,
    body: String,
    /// The local branch to push, and its name on the remote.
    local: String,
    head: String,
    /// The pushed sha.
    sha: String,
    /// The pull request already open for the work, as its `land.pr` detail.
    open: Option<Value>,
    /// The tip the push expects to find on the remote, when it is known to
    /// differ from what `open` recorded; else the recorded tip is the lease.
    lease: Option<String>,
}

/// A pull request opened or updated, and the log line saying so.
#[derive(Debug)]
pub(crate) struct Delivered {
    pub(crate) number: i64,
    pub(crate) url: String,
    pub(crate) opened: bool,
    pub(crate) line: String,
}

/// Pushes the work's branch to the upstream and opens its pull request, or
/// finds the one open for it, then records `land.pr`.
fn deliver_work(
    s: &pulse_store::Store,
    project: &Row,
    up: &Upstream,
    work: Work<'_>,
    now: &str,
) -> Result<Delivered, String> {
    let slug = project.str("slug");
    let root = project.str("path");
    let def = crate::baseline::default_branch(project.clone());
    let lease = work.lease.clone().or_else(|| {
        work.open
            .as_ref()
            .map(|d| field_of(d, "to"))
            .filter(|to| !to.is_empty())
    });
    push(s, &root, up, &work.local, &work.head, lease.as_deref())?;
    let from = {
        let base = crate::util::git_timeout(
            &root,
            Duration::from_secs(20),
            &["merge-base", &format!("refs/heads/{def}"), &work.sha],
        );
        if base.ok() && !base.stdout.trim().is_empty() {
            base.stdout.trim().to_string()
        } else {
            rev(&root, &format!("refs/heads/{def}"))
        }
    };
    let commits = crate::util::git_timeout(
        &root,
        Duration::from_secs(30),
        &["rev-list", "--count", &format!("{from}..{}", work.sha)],
    )
    .stdout
    .trim()
    .parse::<i64>()
    .unwrap_or(0);
    let (number, url, opened) = match &work.open {
        Some(d) => (number_of(d), field_of(d, "url"), false),
        None => match crate::landing_pr::open_pull_request(
            &root,
            &up.repo,
            &work.head,
            &up.remote_branch,
        )? {
            Some((n, u)) => (n, u, false),
            None => {
                let (n, u) = crate::landing_pr::create_pull_request(
                    &root,
                    &up.repo,
                    &work.head,
                    &up.remote_branch,
                    &work.title,
                    &work.body,
                )?;
                (n, u, true)
            }
        },
    };
    let mut detail = json!({"project": slug, "kind": work.kind, "number": number, "url": url,
        "head": work.head, "base": up.remote_branch, "remote": up.remote, "from": from, "to": work.sha,
        "jobs": work.jobs, "commits": commits, "opened": opened, "title": work.title});
    for (k, v) in &work.ids {
        detail[*k] = json!(v);
    }
    event(s, now, "land.pr", &slug, detail);
    let what = work
        .ids
        .first()
        .map(|(k, v)| format!("{k} {v}"))
        .unwrap_or_else(|| work.head.clone());
    Ok(Delivered {
        number,
        url: url.clone(),
        opened,
        line: format!(
            "{} pr {slug} {what}: #{number} {} {url}",
            work.kind,
            if opened { "opened" } else { "updated" }
        ),
    })
}

/// Pushes `local` to `head` on the upstream. A known previous tip is the
/// lease; the first push of a branch forces, since the `pulse/` namespace
/// on the remote is the daemon's own and a bare `--force-with-lease` has no
/// tracking ref to read when the push goes to a url.
fn push(
    s: &pulse_store::Store,
    root: &str,
    up: &Upstream,
    local: &str,
    head: &str,
    lease: Option<&str>,
) -> Result<(), String> {
    let (env, cleanup, why) = crate::forge::push_credential_env(s, root.to_string());
    if !why.is_empty() {
        return Err(why);
    }
    let force = match lease {
        Some(sha) => format!("--force-with-lease=refs/heads/{head}:{sha}"),
        None => "--force".to_string(),
    };
    let pushed = crate::util::git_timeout_env(
        root,
        Duration::from_secs(300),
        &env,
        &[
            "push",
            &force,
            &up.url,
            &format!("refs/heads/{local}:refs/heads/{head}"),
        ],
    );
    cleanup("", &[]);
    if !pushed.ok() {
        return Err(pulse_cred::redact(
            &format!(
                "push of {local} to {head} on {} failed: {}",
                up.name(),
                crate::run::squeeze(&format!("{} {}", pushed.stderr, pushed.stdout), 200)
            ),
            &[],
        ));
    }
    Ok(())
}

/// Builds `pulse/task-<id>` from the default branch plus every job branch,
/// pushes it and opens (or finds) its pull request. When the task's pull
/// request is open already, the rebuilt branch also takes in whatever the
/// pull request's head holds beyond the job branches (a reviewer's or the
/// owner's commits on the forge), so a follow-up job never pushes those
/// away, and the push expects that head.
fn deliver_task(
    s: &pulse_store::Store,
    project: &Row,
    up: &Upstream,
    rt: &ReadyTask,
    now: &str,
) -> Result<String, String> {
    let root = project.str("path");
    let def = crate::baseline::default_branch(project.clone());
    let task = rt.task.as_str();
    let branches = rt.branches.as_slice();
    let head = task_branch(task);
    let remote_head = match &rt.open {
        Some(_) => {
            let sha = fetch_head(s, &root, up, &head)?;
            if sha.is_empty() {
                return Err(format!(
                    "{head} (pull request #{}) is gone from {}",
                    rt.open.as_ref().map(number_of).unwrap_or_default(),
                    up.name()
                ));
            }
            Some(sha)
        }
        None => None,
    };
    let git = |args: &[&str]| crate::util::git_timeout(&root, Duration::from_secs(120), args);
    let git_dir = git(&["rev-parse", "--absolute-git-dir"])
        .stdout
        .trim()
        .to_string();
    if git_dir.is_empty() {
        return Err(format!("{root} is not a git checkout"));
    }
    let wt_path = format!("{git_dir}/pulse-task-pr/{task}");
    let _ = git(&["worktree", "remove", "--force", &wt_path]);
    let _ = std::fs::remove_dir_all(&wt_path);
    let add = git(&["worktree", "add", "--force", "-B", &head, &wt_path, &def]);
    if !add.ok() {
        return Err(format!(
            "worktree add failed: {}",
            crate::run::squeeze(&add.stderr, 200)
        ));
    }
    let in_wt = |args: &[&str]| crate::util::git_timeout(&wt_path, Duration::from_secs(300), args);
    let mut merges: Vec<(String, String)> = branches
        .iter()
        .map(|(_, b)| (b.clone(), b.clone()))
        .collect();
    if let Some(sha) = &remote_head {
        merges.push((
            sha.clone(),
            format!(
                "{head} as pull request #{} has it",
                rt.open.as_ref().map(number_of).unwrap_or_default()
            ),
        ));
    }
    let mut result: Result<(), String> = Ok(());
    for (rev, what) in &merges {
        let contained = in_wt(&["merge-base", "--is-ancestor", rev, "HEAD"]);
        if contained.ok() {
            continue;
        }
        let m = in_wt(&[
            "-c",
            "user.name=PULSE",
            "-c",
            "user.email=pulse@localhost",
            "merge",
            "--no-ff",
            "--no-edit",
            "-m",
            &format!("Merge {what} into {head}"),
            rev,
        ]);
        if !m.ok() {
            let _ = in_wt(&["merge", "--abort"]);
            result = Err(format!(
                "{what} does not merge cleanly onto {def} with the task's other branches: {}",
                crate::run::squeeze(&format!("{} {}", m.stdout, m.stderr), 200)
            ));
            break;
        }
    }
    let tip = in_wt(&["rev-parse", "HEAD"]).stdout.trim().to_string();
    let _ = git(&["worktree", "remove", "--force", &wt_path]);
    result?;
    Ok(deliver_work(
        s,
        project,
        up,
        Work {
            kind: "task",
            ids: vec![("task", task.to_string())],
            jobs: branches.iter().map(|(j, _)| j.clone()).collect(),
            title: format!("{task}: {}", rt.title),
            body: task_body(task, &rt.title, branches),
            local: head.clone(),
            head,
            sha: tip,
            open: rt.open.clone(),
            lease: remote_head,
        },
        now,
    )?
    .line)
}

/// Fetches the upstream's `head` into `refs/remotes/<remote>/<head>` and
/// returns its sha, empty when the upstream has no such branch.
fn fetch_head(
    s: &pulse_store::Store,
    root: &str,
    up: &Upstream,
    head: &str,
) -> Result<String, String> {
    let tracking = format!("refs/remotes/{}/{head}", up.remote);
    let (env, cleanup, why) = crate::forge::push_credential_env(s, root.to_string());
    if !why.is_empty() {
        return Err(why);
    }
    let fetched = crate::util::git_timeout_env(
        root,
        Duration::from_secs(120),
        &env,
        &[
            "fetch",
            "--quiet",
            "--no-tags",
            &up.url,
            &format!("+refs/heads/{head}:{tracking}"),
        ],
    );
    cleanup("", &[]);
    if !fetched.ok() {
        // A branch the upstream has not got is a fetch error, not a
        // failure: the ref stays absent and the caller sees an empty sha.
        let text = format!("{} {}", fetched.stderr, fetched.stdout).to_ascii_lowercase();
        if text.contains("couldn't find remote ref") || text.contains("could not find remote ref") {
            let _ = crate::util::git_timeout(
                root,
                Duration::from_secs(10),
                &["update-ref", "-d", &tracking],
            );
            return Ok(String::new());
        }
        return Err(pulse_cred::redact(
            &format!(
                "fetch of {head} from {} failed: {}",
                up.name(),
                crate::run::squeeze(&fetched.stderr, 200)
            ),
            &[],
        ));
    }
    Ok(rev(root, &tracking))
}

fn task_body(task: &str, title: &str, branches: &[(String, String)]) -> String {
    let mut b = format!("The work of PULSE task {task}: {title}.\n\nJobs:\n");
    for (j, br) in branches {
        b.push_str(&format!("- {j} (`{br}`)\n"));
    }
    b.push_str("\nThe PULSE reviewer reviews this pull request. Merging it closes the task; closing it unmerged sends the task back to its lead.\n");
    b
}

/// One done job's branch as the agent left it, as one pull request.
fn deliver_job(
    s: &pulse_store::Store,
    project: &Row,
    up: &Upstream,
    job: &Row,
    sha: &str,
    next: Next,
    now: &str,
) -> Result<Delivered, String> {
    let id = job.str("id");
    let branch = job.str("branch");
    let title = {
        let t = job.str("title");
        if t.trim().is_empty() {
            format!("{id}: {branch}")
        } else {
            t
        }
    };
    deliver_work(
        s,
        project,
        up,
        Work {
            kind: "job",
            ids: vec![("job", id.clone())],
            jobs: vec![id.clone()],
            title,
            body: job_body(job),
            local: branch.clone(),
            head: branch,
            sha: sha.to_string(),
            open: match next {
                Next::Update(d) => Some(d),
                _ => None,
            },
            lease: None,
        },
        now,
    )
}

fn job_body(job: &Row) -> String {
    let result = job.str("result");
    let source = job.str("source");
    let mut b = String::new();
    if !result.trim().is_empty() {
        b.push_str(&crate::util::trunc(result.trim(), 4000));
        b.push_str("\n\n");
    }
    b.push_str(&format!(
        "Job {} ({}{}), branch `{}` as the agent left it.\n\nThe PULSE reviewer reviews this pull request; merging it closes the job.\n",
        job.str("id"),
        job.str("kind"),
        if source.is_empty() {
            String::new()
        } else {
            format!(", from {source}")
        },
        job.str("branch")
    ));
    b
}

/// A `pr-fix` job's branch pushed onto the pull request it fixes: the job
/// worked on the pull request's head (`pulse review checkout`), so its tip
/// must descend from the head as the remote has it now, and the push
/// leases on that.
fn deliver_fix(
    s: &pulse_store::Store,
    project: &Row,
    up: &Upstream,
    job: &Row,
    number: i64,
    sha: &str,
    now: &str,
) -> Result<String, String> {
    let slug = project.str("slug");
    let root = project.str("path");
    let id = job.str("id");
    let branch = job.str("branch");
    let conn = s.conn().map_err(|e| e.to_string())?;
    let orig = pr_event(&conn, &slug, number)
        .ok_or_else(|| format!("no pull request #{number} of {slug} is known to PULSE"))?;
    drop(conn);
    let head = field_of(&orig, "head");
    if head.is_empty() {
        return Err(format!(
            "pull request #{number} has no head branch on record"
        ));
    }
    let remote_sha =
        fetch_head(s, &root, up, &head).map_err(|e| format!("pull request #{number}: {e}"))?;
    if remote_sha.is_empty() {
        return Err(format!(
            "{head} (pull request #{number}) is gone from {}",
            up.name()
        ));
    }
    // The remote head already at the fix's tip is a push that was made
    // (by an earlier pass that stopped before recording it, or by hand):
    // recorded now, so the job is not looked at every pass.
    if remote_sha != sha {
        let ancestor = crate::util::git_timeout(
            &root,
            Duration::from_secs(20),
            &["merge-base", "--is-ancestor", &remote_sha, sha],
        );
        if !ancestor.ok() {
            return Err(format!(
                "{branch} is not built on the head of pull request #{number} ({}); rebase it onto {head} first",
                crate::recipe::short(&remote_sha, 12)
            ));
        }
        push(s, &root, up, &branch, &head, Some(&remote_sha))?;
    }
    let commits = crate::util::git_timeout(
        &root,
        Duration::from_secs(30),
        &["rev-list", "--count", &format!("{remote_sha}..{sha}")],
    )
    .stdout
    .trim()
    .parse::<i64>()
    .unwrap_or(0);
    let mut jobs = jobs_of(&orig);
    if !jobs.contains(&id) {
        jobs.push(id.clone());
    }
    let mut detail = orig.clone();
    for (k, v) in [
        ("project", json!(slug)),
        ("from", json!(remote_sha)),
        ("to", json!(sha)),
        ("jobs", json!(jobs)),
        ("commits", json!(commits)),
        ("opened", json!(false)),
        ("fix", json!(id)),
    ] {
        detail[k] = v;
    }
    if field(&detail, "kind").is_empty() {
        detail["kind"] = json!("task");
    }
    let url = field_of(&detail, "url");
    event(s, now, "land.pr", &slug, detail);
    Ok(format!(
        "fix pr {slug} {id}: #{number} updated with {commits} {} from {branch} {url}",
        crate::util::plural(commits as i32, "commit")
    ))
}

/// A branch proposed as it is: a conversation's, or one named by hand.
#[allow(clippy::too_many_arguments)]
fn deliver_proposed(
    s: &pulse_store::Store,
    project: &Row,
    up: &Upstream,
    branch: &str,
    sha: &str,
    conversation: Option<String>,
    next: Next,
    now: &str,
) -> Result<Delivered, String> {
    let root = project.str("path");
    let def = crate::baseline::default_branch(project.clone());
    let subject = crate::util::git_timeout(
        &root,
        Duration::from_secs(10),
        &["log", "-1", "--format=%s", sha],
    )
    .stdout
    .trim()
    .to_string();
    let title = if subject.is_empty() {
        branch.to_string()
    } else {
        subject
    };
    let log = crate::util::git_timeout(
        &root,
        Duration::from_secs(30),
        &[
            "log",
            "--format=- %h %s",
            &format!("refs/heads/{def}..{sha}"),
        ],
    )
    .stdout;
    let mut body = match &conversation {
        Some(id) => format!("Proposed from conversation {id} on branch `{branch}`.\n\n"),
        None => format!("Branch `{branch}` proposed by hand with `pulse land --branch`.\n\n"),
    };
    if !log.trim().is_empty() {
        body.push_str("Commits:\n");
        body.push_str(&crate::util::trunc(log.trim_end(), 4000));
        body.push_str("\n\n");
    }
    body.push_str("The PULSE reviewer reviews this pull request.\n");
    let mut ids = Vec::new();
    if let Some(id) = &conversation {
        ids.push(("conversation", id.clone()));
    }
    deliver_work(
        s,
        project,
        up,
        Work {
            kind: "proposed",
            ids,
            jobs: Vec::new(),
            title,
            body,
            local: branch.to_string(),
            head: branch.to_string(),
            sha: sha.to_string(),
            open: match next {
                Next::Update(d) => Some(d),
                _ => None,
            },
            lease: None,
        },
        now,
    )
}

/// `pulse land --branch B`: pushes the branch and opens its pull request
/// (or pushes the new commits to the one open). The project needs an
/// upstream; the caller lands locally when it has none.
pub(crate) fn propose_branch(
    s: &pulse_store::Store,
    project: &Row,
    branch: &str,
    now: &str,
) -> Result<Delivered, String> {
    let slug = project.str("slug");
    let root = project.str("path");
    let def = crate::baseline::default_branch(project.clone());
    let up = crate::forge::upstream_of(&root, &def)
        .ok_or_else(|| format!("{slug} has no upstream to open a pull request on"))?;
    let (exists, why) = crate::land::branch_exists(root.clone(), branch.to_string());
    if !exists {
        return Err(if why.is_empty() {
            format!("branch {branch} is not in {root}")
        } else {
            why
        });
    }
    if branch == def {
        return Err(format!(
            "{branch} is the default branch; name a branch to propose"
        ));
    }
    let sha = rev(&root, &format!("refs/heads/{branch}"));
    let conn = s.conn().map_err(|e| e.to_string())?;
    let next = next_for(
        &conn,
        &root,
        &slug,
        "head",
        branch,
        &sha,
        &work_key(branch, &sha),
    );
    drop(conn);
    if next == Next::Nothing {
        let conn = s.conn().map_err(|e| e.to_string())?;
        let d = last_event(&conn, &slug, "head", branch, false)
            .map(|(_, d)| d)
            .unwrap_or(Value::Null);
        return Ok(Delivered {
            number: number_of(&d),
            url: field_of(&d, "url"),
            opened: false,
            line: format!(
                "{slug}: {branch} is delivered at {} already",
                crate::recipe::short(&sha, 12)
            ),
        });
    }
    let conversation = branch
        .strip_prefix("pulse/chat-")
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    deliver_proposed(s, project, &up, branch, &sha, conversation, next, now)
}

/// A kept experiment's branch as a pull request, from the experiment lane:
/// `body` is the measurement summary the landing note used to carry.
/// Returns (number, url, opened).
pub(crate) fn deliver_experiment(
    s: &pulse_store::Store,
    project: &Row,
    job: &Row,
    archive_id: &str,
    run_id: &str,
    summary: &str,
    now: &str,
) -> Result<(i64, String, bool), String> {
    let slug = project.str("slug");
    let root = project.str("path");
    let def = crate::baseline::default_branch(project.clone());
    let up = crate::forge::upstream_of(&root, &def)
        .ok_or_else(|| format!("{slug} has no upstream to open a pull request on"))?;
    let id = job.str("id");
    let branch = job.str("branch");
    let sha = rev(&root, &format!("refs/heads/{branch}"));
    if sha.is_empty() {
        return Err(format!("branch {branch} is not in {root}"));
    }
    let conn = s.conn().map_err(|e| e.to_string())?;
    let next = next_for(
        &conn,
        &root,
        &slug,
        "job",
        &id,
        &sha,
        &work_key(&branch, &sha),
    );
    drop(conn);
    let open = match next {
        Next::Nothing => {
            let conn = s.conn().map_err(|e| e.to_string())?;
            let d = open_pr_for_job(&conn, &slug, &id).ok_or_else(|| {
                format!(
                    "{branch} was delivered at {} already",
                    crate::recipe::short(&sha, 12)
                )
            })?;
            return Ok((number_of(&d), field_of(&d, "url"), false));
        }
        Next::Update(d) => Some(d),
        Next::Open => None,
    };
    let title = {
        let t = job.str("title");
        if t.trim().is_empty() {
            format!("Experiment {id}")
        } else {
            t
        }
    };
    let body = format!(
        "Experiment {id} (archive {archive_id}), measured and kept.\n\n{}\n\nJob {id} ({}, from {}), branch `{branch}`.\n\nThe PULSE reviewer reviews this pull request; merging it finalizes the experiment.\n",
        crate::util::trunc(summary.trim(), 6000),
        job.str("kind"),
        job.str("source")
    );
    let d = deliver_work(
        s,
        project,
        &up,
        Work {
            kind: "experiment",
            ids: vec![
                ("experiment", id.clone()),
                ("job", id.clone()),
                ("archive", archive_id.to_string()),
                ("run_id", run_id.to_string()),
            ],
            jobs: vec![id.clone()],
            title,
            body,
            local: branch.clone(),
            head: branch,
            sha,
            open,
            lease: None,
        },
        now,
    )?;
    Ok((d.number, d.url, d.opened))
}

/// The pull requests of the project still open as far as PULSE knows: the
/// newest `land.pr` per number with no `land.merged` or `land.pr-closed`
/// after it.
fn open_prs(q: &(impl crate::util::Querier + ?Sized), slug: &str) -> Vec<Value> {
    let rows = crate::util::query_rows(
        q,
        "SELECT action, detail FROM events WHERE subject = ? AND action IN ('land.pr', 'land.merged', 'land.pr-closed')
         AND json_valid(detail) ORDER BY seq",
        &[&slug],
    )
    .unwrap_or_default();
    let mut by_number: BTreeMap<i64, (String, Value)> = BTreeMap::new();
    for r in rows {
        let d: Value = serde_json::from_str(&r.str("detail")).unwrap_or(Value::Null);
        let n = number_of(&d);
        if n == 0 {
            continue;
        }
        let action = r.str("action");
        match by_number.get(&n) {
            // A later land.pr of a settled number is a new pull request's
            // push only when the forge reused the number, which it does not;
            // it is the push of new commits recorded after the merge was
            // seen, and the merge stands.
            Some((prev, _)) if prev != "land.pr" && action == "land.pr" => {}
            _ => {
                by_number.insert(n, (action, d));
            }
        }
    }
    by_number
        .into_values()
        .filter(|(a, _)| a == "land.pr")
        .map(|(_, d)| d)
        .collect()
}

/// Reads every open pull request of the project from the forge, in one
/// `gh pr list` per repository, and settles the merged and closed ones.
fn poll_open(s: &pulse_store::Store, project: &Row, now: &str) -> Vec<String> {
    let slug = project.str("slug");
    let root = project.str("path");
    let def = crate::baseline::default_branch(project.clone());
    let Ok(conn) = s.conn() else {
        return Vec::new();
    };
    let prs = open_prs(&conn, &slug);
    drop(conn);
    if prs.is_empty() {
        return Vec::new();
    }
    let Some(up) = crate::forge::upstream_of(&root, &def) else {
        return vec![format!(
            "{slug}: {} open pull {} but no upstream to read them from",
            prs.len(),
            if prs.len() == 1 {
                "request"
            } else {
                "requests"
            }
        )];
    };
    let mut lines = Vec::new();
    let listed = match crate::landing_pr::list_prs(&root, &up.repo) {
        Ok(l) => l,
        Err(e) => {
            lines.push(format!("{slug}: {e}"));
            BTreeMap::new()
        }
    };
    for pr in prs {
        let number = number_of(&pr);
        let state = match listed.get(&number) {
            Some(st) => st.clone(),
            None => match crate::landing_pr::view_pr(&root, &up.repo, number) {
                Ok(st) => st,
                Err(e) => {
                    lines.push(format!("{slug}: pull request #{number}: {e}"));
                    continue;
                }
            },
        };
        match state.state.as_str() {
            "MERGED" => lines.push(settle_merged(s, project, &pr, &state, now)),
            "CLOSED" => lines.push(settle_closed(s, project, &pr, &state, now)),
            _ => {}
        }
    }
    lines
}

/// The jobs a pull request delivered: the `jobs` list of its `land.pr`,
/// plus its `job` when it names one.
fn jobs_of(pr: &Value) -> Vec<String> {
    let mut jobs: Vec<String> = pr
        .get("jobs")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let job = field(pr, "job");
    if !job.is_empty() && !jobs.iter().any(|j| j == job) {
        jobs.push(job.to_string());
    }
    jobs
}

/// Clears the branch of every job the pull request delivered (a task's by
/// its source too, as PR #53 did), so nothing of it waits any more.
fn clear_branches(s: &pulse_store::Store, slug: &str, pr: &Value) {
    let Ok(conn) = s.conn() else {
        return;
    };
    let task = field(pr, "task");
    if !task.is_empty() {
        let (exact, like) = pulse_store::task_source_params(task);
        let _ = conn.execute(
            &format!(
                "UPDATE jobs SET branch = '' WHERE project = ? AND status = 'done' AND {}",
                pulse_store::task_source_clause("source")
            ),
            rusqlite::params![slug, exact, like],
        );
    }
    for id in jobs_of(pr) {
        crate::util::exec1(
            &conn,
            "UPDATE jobs SET branch = '' WHERE id = ? AND project = ? AND status = 'done'",
            &[&id, &slug],
        );
    }
}

/// The detail a `land.merged` or `land.pr-closed` carries: the pull
/// request's identity and ids, from its `land.pr`.
fn settle_detail(pr: &Value, slug: &str, st: &crate::landing_pr::PrState) -> Value {
    let mut d = json!({"project": slug, "kind": kind_of(pr), "number": number_of(pr),
        "url": if st.url.is_empty() { field_of(pr, "url") } else { st.url.clone() },
        "head": field_of(pr, "head"), "base": field_of(pr, "base"), "jobs": jobs_of(pr)});
    for k in [
        "task",
        "job",
        "experiment",
        "conversation",
        "archive",
        "run_id",
    ] {
        if let Some(v) = pr.get(k).filter(|v| !v.is_null()) {
            d[k] = v.clone();
        }
    }
    d
}

/// The kind of an older `land.pr` without one: PR #53 wrote task pull
/// requests only.
fn kind_of(pr: &Value) -> String {
    let k = field(pr, "kind");
    if k.is_empty() {
        "task".to_string()
    } else {
        k.to_string()
    }
}

fn settle_merged(
    s: &pulse_store::Store,
    project: &Row,
    pr: &Value,
    st: &crate::landing_pr::PrState,
    now: &str,
) -> String {
    let slug = project.str("slug");
    let root = project.str("path");
    let number = number_of(pr);
    let kind = kind_of(pr);
    let mut detail = settle_detail(pr, &slug, st);
    detail["merged_at"] = json!(st.merged_at);
    let mut line = format!("{kind} pr {slug}: #{number} merged");
    if kind == "experiment" {
        match finalize_experiment(s, project, pr, &detail) {
            Ok(()) => line.push_str("; experiment finalized"),
            Err(e) => {
                event(s, now, "land.merged", &slug, detail);
                line.push_str(&format!("; experiment not finalized: {e}"));
            }
        }
    } else {
        event(s, now, "land.merged", &slug, detail);
    }
    // The local branches go before the jobs forget them: a task's job
    // branches are found through the jobs.
    for note in delete_delivered_branches(s, &slug, &root, pr) {
        line.push_str(&format!("; {note}"));
    }
    clear_branches(s, &slug, pr);
    let task = field(pr, "task");
    if !task.is_empty() {
        line.push_str("; task done");
        if let Err(e) = pulse_tasks::set_status(s, task, "done", ACTOR) {
            line.push_str(&format!(" (status not changed: {e})"));
        }
    }
    // The merge is on the upstream now; the checkout follows it this pass,
    // not when the two-minute fetch memo runs out.
    if let Some(ff) = crate::forge::sync_default_from_upstream(s, project, ACTOR, now, true) {
        line.push_str(&format!("; {ff}"));
    }
    line
}

/// Records the merge of an experiment's pull request the way a local
/// landing did (`landing_evidence::record_landing`): the archive must still
/// be the job's current run's, the branch is cleared and `land.merged`
/// carries the archive and run, in one transaction - evolve's finalization
/// (`pulse_evolve::finalization`) reads exactly that.
fn finalize_experiment(
    s: &pulse_store::Store,
    project: &Row,
    pr: &Value,
    detail: &Value,
) -> anyhow::Result<()> {
    let slug = project.str("slug");
    let job_id = field_of(pr, "job");
    let archive = field_of(pr, "archive");
    let run_id = field_of(pr, "run_id");
    let branch = field_of(pr, "head");
    if job_id.is_empty() || archive.is_empty() || run_id.is_empty() {
        anyhow::bail!("the pull request's land.pr names no experiment archive and run");
    }
    let tx = s.begin()?;
    let current: bool = tx.query_row(
        "SELECT EXISTS (
            SELECT 1 FROM evolution_archive a JOIN runs r ON r.id = a.run_id AND r.project = a.project
            JOIN jobs j ON j.id = r.job_id AND j.project = a.project
            WHERE a.project = ? AND a.id = ? AND a.branch = ? AND a.run_id = ? AND a.kept = 1
            AND j.id = ? AND j.kind = 'experiment' AND j.status = 'done' AND r.status = 'done'
            AND r.id = (SELECT latest.id FROM runs latest WHERE latest.job_id = j.id
                ORDER BY latest.started_at DESC, latest.id DESC LIMIT 1)
            AND NOT EXISTS (SELECT 1 FROM runs active WHERE active.job_id = j.id AND active.status = 'running'))",
        rusqlite::params![slug, archive, branch, run_id, job_id],
        |r| r.get(0),
    )?;
    if !current {
        let _ = tx.rollback();
        anyhow::bail!(
            "experiment {job_id} changed before archive {archive} could be finalized; owner review needed"
        );
    }
    tx.execute(
        "UPDATE jobs SET branch = '', result = result || ? WHERE id = ? AND project = ?",
        rusqlite::params![
            format!(" | delivered as pull request #{}", number_of(pr)),
            job_id,
            slug
        ],
    )?;
    tx.event(ACTOR, "land.merged", &slug, detail.clone())?;
    tx.commit()
}

/// Deletes the local branches a merged pull request delivered - the job
/// branches, a task's `pulse/task-<id>` - so the checkout does not fill
/// with branches whose commits are on the default branch. A branch some
/// worktree has checked out (the hub's chat worktrees) stays, with a note.
fn delete_delivered_branches(
    s: &pulse_store::Store,
    slug: &str,
    root: &str,
    pr: &Value,
) -> Vec<String> {
    let mut branches: Vec<String> = Vec::new();
    let task = field(pr, "task");
    if !task.is_empty() {
        branches.push(task_branch(task));
        if let Ok(conn) = s.conn() {
            let (exact, like) = pulse_store::task_source_params(task);
            for j in crate::util::query_rows(
                &conn,
                &format!(
                    "SELECT branch FROM jobs WHERE project = ? AND status = 'done' AND branch != '' AND {}",
                    pulse_store::task_source_clause("source")
                ),
                &[&slug, &exact, &like],
            )
            .unwrap_or_default()
            {
                branches.push(j.str("branch"));
            }
        }
    }
    let head = field(pr, "head");
    if !head.is_empty() && !branches.iter().any(|b| b == head) {
        branches.push(head.to_string());
    }
    let checked_out = worktree_branches(root);
    let mut notes = Vec::new();
    for b in branches {
        if rev(root, &format!("refs/heads/{b}")).is_empty() {
            continue;
        }
        if checked_out.contains(&b) {
            notes.push(format!("{b} kept: a worktree has it checked out"));
            continue;
        }
        let del = crate::util::git_timeout(root, Duration::from_secs(30), &["branch", "-D", &b]);
        if !del.ok() {
            notes.push(format!(
                "{b} not deleted: {}",
                crate::run::squeeze(&del.stderr, 120)
            ));
        }
    }
    notes
}

/// The branches checked out in the checkout's worktrees, the root included.
fn worktree_branches(root: &str) -> Vec<String> {
    crate::util::git_timeout(
        root,
        Duration::from_secs(30),
        &["worktree", "list", "--porcelain"],
    )
    .stdout
    .lines()
    .filter_map(|l| l.strip_prefix("branch refs/heads/"))
    .map(|b| b.trim().to_string())
    .collect()
}

fn settle_closed(
    s: &pulse_store::Store,
    project: &Row,
    pr: &Value,
    st: &crate::landing_pr::PrState,
    now: &str,
) -> String {
    let slug = project.str("slug");
    let number = number_of(pr);
    let kind = kind_of(pr);
    let detail = settle_detail(pr, &slug, st);
    let url = field_of(&detail, "url");
    event(s, now, "land.pr-closed", &slug, detail);
    clear_branches(s, &slug, pr);
    let task = field(pr, "task");
    if task.is_empty() {
        return format!(
            "{kind} pr {slug}: #{number} closed unmerged; the branch stays on the remote"
        );
    }
    let _ = pulse_tasks::set_status(s, task, "in_progress", ACTOR);
    let _ = pulse_tasks::send_message(
        s,
        &slug,
        task,
        ACTOR,
        &format!(
            "Pull request #{number} ({url}) was closed without merging. The task is back in progress: its lead reads why on the pull request and plans what comes next; the branch {} stays on the remote.",
            task_branch(task)
        ),
    );
    format!("task pr {slug} {task}: #{number} closed unmerged; task back in progress")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::*;
    use crate::test_helpers_daemon::*;

    /// A task with two done jobs on their own branches.
    fn task_with_two_branches(s: &pulse_store::Store, proj: &Row, repo: &str) -> String {
        let task =
            pulse_tasks::new(s, "proj", "Add the thing", "b", 0, pulse_store::HUMAN_AGENT).unwrap();
        for name in ["pulse/worker/a", "pulse/worker/b"] {
            let j = push_branch(s, proj, repo, name);
            exec1(
                s,
                "UPDATE jobs SET source = ? WHERE id = ?",
                &[&format!("task:{}", task.id), &j.str("id")],
            );
        }
        exec1(
            s,
            "UPDATE tasks SET status = 'in_progress' WHERE id = ?",
            &[&task.id],
        );
        task.id
    }

    fn status(s: &pulse_store::Store, id: &str) -> String {
        pulse_tasks::get(s, id).unwrap().0.status
    }

    /// One more commit on `branch`, made without touching the checkout's
    /// HEAD; returns the new tip.
    fn commit_on(repo: &str, branch: &str, file: &str) -> String {
        commit_dated(repo, branch, file, "")
    }

    /// `commit_on` with the commit's committer date set to `at` when it is
    /// not empty (`run` strips the hook variables, GIT_COMMITTER_DATE among
    /// them, so the commit itself is run directly).
    fn commit_dated(repo: &str, branch: &str, file: &str, at: &str) -> String {
        let head = run(repo, &["git", "rev-parse", "--abbrev-ref", "HEAD"])
            .trim()
            .to_string();
        run(repo, &["git", "checkout", "-q", branch]);
        std::fs::write(
            std::path::Path::new(repo).join(file),
            format!("{branch} {file}\n"),
        )
        .unwrap();
        run(repo, &["git", "add", "-A"]);
        let mut cmd = std::process::Command::new("git");
        cmd.args(["commit", "-q", "-m", &format!("more on {branch}")])
            .current_dir(repo)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@x")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@x")
            .env("GIT_CONFIG_GLOBAL", "/dev/null");
        if !at.is_empty() {
            cmd.env("GIT_COMMITTER_DATE", at).env("GIT_AUTHOR_DATE", at);
        }
        let out = cmd.output().expect("git commit");
        assert!(
            out.status.success(),
            "git commit: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        run(repo, &["git", "checkout", "-q", &head]);
        ref_of(repo, &format!("refs/heads/{branch}"))
    }

    fn events(s: &pulse_store::Store, action: &str) -> i32 {
        count(
            s,
            "SELECT COUNT(*) AS n FROM events WHERE action = ?",
            &[&action],
        )
    }

    fn tend_proj(s: &pulse_store::Store, proj: &Row, now: &str) -> Vec<String> {
        crate::forge::forget_sync_memo();
        tend(s, std::slice::from_ref(proj), now)
    }

    #[test]
    fn a_finished_task_opens_one_pull_request_and_is_done_once_merged() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let task = task_with_two_branches(&s, &proj, &repo);
        let main_before = ref_of(&repo, "refs/heads/main");

        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines
                .iter()
                .any(|l| l.contains(&format!("{task}: #7 opened"))),
            "{lines:?}"
        );
        assert_eq!(
            ref_of(&repo, "refs/heads/main"),
            main_before,
            "nothing lands locally"
        );
        let head = bare_ref(&bare, &format!("refs/heads/{}", task_branch(&task)));
        for b in ["pulse/worker/a", "pulse/worker/b"] {
            let tip = ref_of(&repo, &format!("refs/heads/{b}"));
            run(&repo, &["git", "merge-base", "--is-ancestor", &tip, &head]);
        }
        let d = pulse_tasks::delivery_for(&s, &task).unwrap();
        assert_eq!(d.state, "pr_open", "{d:?}");
        assert_eq!(d.url, "https://github.com/o/r/pull/7");
        assert_eq!(d.number, 7);
        assert_eq!(d.waiting, 2);
        let pr = last_detail(&s, "land.pr");
        assert_eq!(pr["kind"], "task");
        assert_eq!(pr["task"], task);
        assert_eq!(pr["opened"], true);
        assert_eq!(pr["to"], head);
        assert_eq!(pr["jobs"].as_array().map(Vec::len), Some(2));

        // Still open: one `pr list` for the whole repository, no second
        // pull request.
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(status(&s, &task), "in_progress");
        assert_eq!(
            gh.calls(),
            ["pr list --head", "pr create --head", "pr list --state"]
        );

        // Merged on the forge: the upstream main moves, the task is done,
        // the local branches go.
        merge_on_forge(&gh, &bare, 7);
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains("#7 merged; task done")),
            "{lines:?}"
        );
        assert_eq!(status(&s, &task), "done");
        let d = pulse_tasks::delivery_for(&s, &task).unwrap();
        assert_eq!(d.state, "merged", "{d:?}");
        assert_eq!(d.waiting, 0, "the jobs' branches are cleared");
        assert_eq!(
            ref_of(&repo, "refs/heads/main"),
            head,
            "main fast-forwarded"
        );
        for b in ["pulse/worker/a", "pulse/worker/b", &task_branch(&task)] {
            assert!(
                rev(&repo, &format!("refs/heads/{b}")).is_empty(),
                "{b} deleted locally"
            );
        }
        let merged = last_detail(&s, "land.merged");
        assert_eq!(merged["kind"], "task");
        assert_eq!(merged["task"], task);
        assert_eq!(merged["number"], 7);
        assert_eq!(merged["merged_at"], "2026-11-04T07:00:00Z");
        assert_eq!(events(&s, "land.merged"), 1);
        // Settled: no more polls.
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(lines.is_empty(), "{lines:?}");
    }

    #[test]
    fn a_follow_up_job_is_pushed_onto_the_task_pull_request_with_what_the_forge_holds() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let task = task_with_two_branches(&s, &proj, &repo);
        let head = task_branch(&task);
        tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(last_detail(&s, "land.pr")["number"], 7);
        let first_tip = bare_ref(&bare, &format!("refs/heads/{head}"));

        // The reviewer pushes a commit straight to the pull request, and its
        // "changes" follow-up job finishes on a branch cut from the pull
        // request's head as `pulse review checkout` leaves it.
        run(&repo, &["git", "branch", "hand", &head]);
        let hand = commit_on(&repo, "hand", "hand.txt");
        run(
            &repo,
            &["git", "push", "-q", &bare, &format!("hand:{head}")],
        );
        run(&repo, &["git", "branch", "-D", "hand"]);
        let fix = push_branch(&s, &proj, &repo, "pulse/worker/c");
        run(&repo, &["git", "branch", "-f", "pulse/worker/c", &head]);
        let c_tip = commit_on(&repo, "pulse/worker/c", "c.txt");
        exec1(
            &s,
            "UPDATE jobs SET source = ? WHERE id = ?",
            &[&format!("task:{task}"), &fix.str("id")],
        );
        assert!(
            dry_run(&s, &proj, TEST_NOW)
                .iter()
                .any(|l| l.contains("pulse/worker/c") && l.contains("push onto pull request #7")),
            "{:?}",
            dry_run(&s, &proj, TEST_NOW)
        );

        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines
                .iter()
                .any(|l| l.contains(&format!("{task}: #7 updated"))),
            "{lines:?}"
        );
        let pushed = bare_ref(&bare, &format!("refs/heads/{head}"));
        assert_ne!(pushed, first_tip);
        for tip in [&hand, &c_tip] {
            run(&repo, &["git", "merge-base", "--is-ancestor", tip, &pushed]);
        }
        let pr = last_detail(&s, "land.pr");
        assert_eq!(pr["number"], 7);
        assert_eq!(pr["opened"], false);
        assert_eq!(pr["to"], pushed);
        assert_eq!(pr["jobs"].as_array().map(Vec::len), Some(3));
        assert!(
            pr["jobs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|j| j == &json!(fix.str("id")))
        );
        assert_eq!(
            gh.calls()
                .iter()
                .filter(|c| c.starts_with("pr create"))
                .count(),
            1,
            "{:?}",
            gh.calls()
        );
        assert_eq!(status(&s, &task), "in_progress");
        assert_eq!(pulse_tasks::delivery_for(&s, &task).unwrap().waiting, 3);

        // Nothing new: nothing pushed.
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(bare_ref(&bare, &format!("refs/heads/{head}")), pushed);

        merge_on_forge(&gh, &bare, 7);
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains("#7 merged; task done")),
            "{lines:?}"
        );
        assert_eq!(status(&s, &task), "done");
        assert_eq!(pulse_tasks::delivery_for(&s, &task).unwrap().waiting, 0);
        assert!(rev(&repo, "refs/heads/pulse/worker/c").is_empty());
        assert_eq!(
            last_detail(&s, "land.merged")["jobs"]
                .as_array()
                .map(Vec::len),
            Some(3)
        );
    }

    #[test]
    fn a_pull_request_closed_unmerged_sends_the_task_back_to_work() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, _bare) = push_fixture(&s);
        let task = task_with_two_branches(&s, &proj, &repo);
        tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(
            pulse_tasks::delivery_for(&s, &task).unwrap().state,
            "pr_open"
        );
        // Review sets it aside first; a closed pull request still goes back.
        exec1(
            &s,
            "UPDATE tasks SET status = 'review' WHERE id = ?",
            &[&task],
        );

        gh.set_state(7, "CLOSED");
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains("closed unmerged")),
            "{lines:?}"
        );
        assert_eq!(status(&s, &task), "in_progress");
        assert!(closed_awaiting_work(&s, "proj", &task));
        let d = pulse_tasks::delivery_for(&s, &task).unwrap();
        assert_eq!(d.waiting, 0, "{d:?}");
        assert_ne!(d.state, "pr_open", "{d:?}");
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM messages WHERE task_id = ? AND body LIKE '%closed without merging%'",
                &[&task]
            ),
            1
        );
        let closed = last_detail(&s, "land.pr-closed");
        assert_eq!(closed["kind"], "task");
        assert_eq!(closed["task"], task);
        assert_eq!(closed["number"], 7);
        // No new work: nothing is opened again.
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(lines.is_empty(), "{lines:?}");
    }

    #[test]
    fn a_done_job_of_any_source_is_its_own_pull_request_until_merged() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let job = push_branch(&s, &proj, &repo, "pulse/sec/x");
        let id = job.str("id");
        exec1(
            &s,
            "UPDATE jobs SET kind = 'security', source = 'security:proj:2026-09-06', result = 'Tightened the thing.' WHERE id = ?",
            &[&id],
        );
        let tip = ref_of(&repo, "refs/heads/pulse/sec/x");

        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(
            lines,
            [format!(
                "job pr proj job {id}: #7 opened https://github.com/o/r/pull/7"
            )]
        );
        assert_eq!(bare_ref(&bare, "refs/heads/pulse/sec/x"), tip);
        assert_eq!(gh.head_of(7), "pulse/sec/x");
        assert_eq!(gh.title_of(7), "t pulse/sec/x");
        let pr = last_detail(&s, "land.pr");
        assert_eq!(pr["kind"], "job");
        assert_eq!(pr["job"], id);
        assert_eq!(pr["jobs"], json!([id]));
        assert_eq!(pr["head"], "pulse/sec/x");
        assert_eq!(pr["base"], "main");
        assert_eq!(pr["to"], tip);
        assert_eq!(pr["commits"], 1);
        assert_eq!(pr["opened"], true);
        assert_eq!(
            job_row(&s, &id).str("branch"),
            "pulse/sec/x",
            "waits for the merge"
        );

        // The same tip again: nothing. New commits: pushed to the same pull
        // request, recorded as an update.
        assert!(tend_proj(&s, &proj, TEST_NOW).is_empty());
        let tip2 = commit_on(&repo, "pulse/sec/x", "more.txt");
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(
            lines,
            [format!(
                "job pr proj job {id}: #7 updated https://github.com/o/r/pull/7"
            )]
        );
        assert_eq!(bare_ref(&bare, "refs/heads/pulse/sec/x"), tip2);
        let pr = last_detail(&s, "land.pr");
        assert_eq!(pr["opened"], false);
        assert_eq!(pr["to"], tip2);
        assert_eq!(pr["commits"], 2);
        assert_eq!(
            gh.calls()
                .iter()
                .filter(|c| c.starts_with("pr create"))
                .count(),
            1
        );

        merge_on_forge(&gh, &bare, 7);
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("job pr proj: #7 merged")),
            "{lines:?}"
        );
        let merged = last_detail(&s, "land.merged");
        assert_eq!(merged["kind"], "job");
        assert_eq!(merged["job"], id);
        assert_eq!(merged["head"], "pulse/sec/x");
        assert_eq!(job_row(&s, &id).str("branch"), "");
        assert!(
            rev(&repo, "refs/heads/pulse/sec/x").is_empty(),
            "branch deleted"
        );
        assert_eq!(
            ref_of(&repo, "refs/heads/main"),
            tip2,
            "main follows the upstream"
        );
        assert!(tend_proj(&s, &proj, TEST_NOW).is_empty());
    }

    #[test]
    fn a_pr_fix_job_pushes_onto_the_pull_request_it_fixes() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let job = push_branch(&s, &proj, &repo, "pulse/worker/a");
        let id = job.str("id");
        exec1(
            &s,
            "UPDATE jobs SET kind = 'suggestion' WHERE id = ?",
            &[&id],
        );
        tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(gh.head_of(7), "pulse/worker/a");
        let before = bare_ref(&bare, "refs/heads/pulse/worker/a");

        // The fix was built on the pull request's head (`pulse review
        // checkout`) and left on its own branch.
        let fix = push_branch(&s, &proj, &repo, "pulse/fixer/f");
        let fix_id = fix.str("id");
        run(
            &repo,
            &["git", "branch", "-f", "pulse/fixer/f", "pulse/worker/a"],
        );
        let fix_tip = commit_on(&repo, "pulse/fixer/f", "fix.txt");
        exec1(
            &s,
            "UPDATE jobs SET kind = 'suggestion', source = 'pr-fix:proj:7' WHERE id = ?",
            &[&fix_id],
        );

        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(
            lines,
            [format!(
                "fix pr proj {fix_id}: #7 updated with 1 commit from pulse/fixer/f https://github.com/o/r/pull/7"
            )]
        );
        assert_eq!(bare_ref(&bare, "refs/heads/pulse/worker/a"), fix_tip);
        let pr = last_detail(&s, "land.pr");
        assert_eq!(pr["kind"], "job");
        assert_eq!(pr["job"], id, "the pull request keeps its ids");
        assert_eq!(pr["number"], 7);
        assert_eq!(pr["head"], "pulse/worker/a");
        assert_eq!(pr["opened"], false);
        assert_eq!(pr["fix"], fix_id);
        assert_eq!(pr["from"], before);
        assert_eq!(pr["to"], fix_tip);
        assert_eq!(pr["jobs"], json!([id, fix_id]));
        assert_eq!(
            gh.calls()
                .iter()
                .filter(|c| c.starts_with("pr create"))
                .count(),
            1,
            "no second pull request"
        );
        assert!(tend_proj(&s, &proj, TEST_NOW).is_empty(), "pushed once");

        // A fix not built on the head is held, not force-pushed over it.
        let stray = push_branch(&s, &proj, &repo, "pulse/fixer/stray");
        exec1(
            &s,
            "UPDATE jobs SET source = 'pr-fix:proj:7' WHERE id = ?",
            &[&stray.str("id")],
        );
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l
                .contains("held: pulse/fixer/stray is not built on the head of pull request #7")),
            "{lines:?}"
        );
        assert_eq!(bare_ref(&bare, "refs/heads/pulse/worker/a"), fix_tip);
        assert!(tend_proj(&s, &proj, TEST_NOW).is_empty(), "held once");

        // Merged: both jobs of the pull request are cleared.
        merge_on_forge(&gh, &bare, 7);
        tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(job_row(&s, &id).str("branch"), "");
        assert_eq!(job_row(&s, &fix_id).str("branch"), "");
        assert_eq!(ref_of(&repo, "refs/heads/main"), fix_tip);
    }

    #[test]
    fn a_conversation_branch_is_proposed_once_its_last_commit_is_ten_minutes_old() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        run(&repo, &["git", "branch", "pulse/chat-abc123", "main"]);
        // Two minutes old: the conversation may still be committing.
        let tip = commit_dated(
            &repo,
            "pulse/chat-abc123",
            "chat.txt",
            "2026-09-06T09:58:00Z",
        );
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(events(&s, "land.pr"), 0);
        let dry = dry_run(&s, &proj, TEST_NOW);
        assert!(
            dry.iter().any(|l| l.contains(
                "conversation pulse/chat-abc123: 1 commit ahead of main, last commit 2 minutes ago (waits until it is 10 minutes old)"
            )),
            "{dry:?}"
        );

        // Ten minutes on: proposed.
        let later = "2026-09-06T10:08:30Z";
        let lines = tend_proj(&s, &proj, later);
        assert_eq!(
            lines,
            ["proposed pr proj conversation abc123: #7 opened https://github.com/o/r/pull/7"]
        );
        assert_eq!(bare_ref(&bare, "refs/heads/pulse/chat-abc123"), tip);
        let pr = last_detail(&s, "land.pr");
        assert_eq!(pr["kind"], "proposed");
        assert_eq!(pr["conversation"], "abc123");
        assert_eq!(pr["head"], "pulse/chat-abc123");
        assert_eq!(pr["to"], tip);
        assert_eq!(pr["title"], "more on pulse/chat-abc123");
        assert_eq!(pr["jobs"], json!([]));
        assert!(tend_proj(&s, &proj, later).is_empty());

        // The hub's chat worktree has the branch checked out: a later
        // commit is still pushed from the root, and after the merge the
        // branch stays for the worktree.
        let wt = format!("{repo}/../chat-wt");
        run(
            &repo,
            &["git", "worktree", "add", "-q", &wt, "pulse/chat-abc123"],
        );
        let tip2 = commit_dated(
            &wt,
            "pulse/chat-abc123",
            "again.txt",
            "2026-09-06T09:40:00Z",
        );
        assert_eq!(ref_of(&repo, "refs/heads/pulse/chat-abc123"), tip2);
        let lines = tend_proj(&s, &proj, later);
        assert_eq!(
            lines,
            ["proposed pr proj conversation abc123: #7 updated https://github.com/o/r/pull/7"]
        );
        assert_eq!(bare_ref(&bare, "refs/heads/pulse/chat-abc123"), tip2);
        assert_eq!(last_detail(&s, "land.pr")["opened"], false);

        merge_on_forge(&gh, &bare, 7);
        let lines = tend_proj(&s, &proj, later);
        assert!(
            lines.iter().any(|l| l.contains("#7 merged")
                && l.contains("pulse/chat-abc123 kept: a worktree has it checked out")),
            "{lines:?}"
        );
        let merged = last_detail(&s, "land.merged");
        assert_eq!(merged["kind"], "proposed");
        assert_eq!(merged["conversation"], "abc123");
        assert_eq!(ref_of(&repo, "refs/heads/pulse/chat-abc123"), tip2);
        assert_eq!(ref_of(&repo, "refs/heads/main"), tip2);
        // Merged by patch: not proposed again.
        assert!(tend_proj(&s, &proj, later).is_empty());
        run(&repo, &["git", "worktree", "remove", "--force", &wt]);
    }

    #[test]
    fn a_project_without_an_upstream_lands_locally_as_before() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo) = new_repo(&s);
        exec1(
            &s,
            "UPDATE projects SET verify_cmd = 'printf verified' WHERE slug = 'proj'",
            &[],
        );
        set_setting(&s, "daemon.min_free_mb", "0");
        let proj = crate::board::project_by_slug(&s.conn().unwrap(), proj.str("slug"));
        let job = push_branch(&s, &proj, &repo, "pulse/worker/local");
        let tip = ref_of(&repo, "refs/heads/pulse/worker/local");
        let dry = dry_run(&s, &proj, TEST_NOW);
        assert_eq!(
            dry[0],
            "proj: no upstream, finished branches land locally on main"
        );
        assert!(
            dry[1].contains("pulse/worker/local -> lands locally on main"),
            "{dry:?}"
        );

        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(lines[0], "proj: no upstream, landing locally");
        assert!(
            lines.iter().any(|l| l.starts_with("proj: landed")),
            "{lines:?}"
        );
        assert_eq!(events(&s, "land.landed"), 1);
        assert_eq!(events(&s, "land.pr"), 0);
        assert!(gh.calls().is_empty(), "no forge call");
        assert_eq!(job_row(&s, &job.str("id")).str("branch"), "");
        run(
            &repo,
            &[
                "git",
                "merge-base",
                "--is-ancestor",
                &tip,
                "refs/heads/main",
            ],
        );
        assert!(tend_proj(&s, &proj, TEST_NOW).is_empty());
    }

    #[test]
    fn a_done_job_whose_branch_is_gone_is_told_once_and_forgets_it() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, _bare) = push_fixture(&s);
        let job = push_branch(&s, &proj, &repo, "pulse/worker/gone");
        let id = job.str("id");
        exec1(
            &s,
            "UPDATE jobs SET kind = 'suggestion' WHERE id = ?",
            &[&id],
        );
        run(&repo, &["git", "branch", "-D", "pulse/worker/gone"]);

        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.starts_with(&format!(
                "job pr proj {id}: branch pulse/worker/gone is not in"
            ))),
            "{lines:?}"
        );
        let held = last_detail(&s, "land.pr-held");
        assert_eq!(held["job"], id);
        assert_eq!(held["work"], "pulse/worker/gone");
        assert_eq!(job_row(&s, &id).str("branch"), "");
        assert!(gh.calls().is_empty());
        assert!(tend_proj(&s, &proj, TEST_NOW).is_empty(), "not retried");
        assert_eq!(events(&s, "land.pr-held"), 1);
    }

    #[test]
    fn a_pass_opens_at_most_ten_pull_requests_per_project() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, _bare) = push_fixture(&s);
        for i in 0..12 {
            let j = push_branch(&s, &proj, &repo, &format!("pulse/worker/n{i}"));
            exec1(
                &s,
                "UPDATE jobs SET kind = 'suggestion' WHERE id = ?",
                &[&j.str("id")],
            );
        }
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(lines.len(), 10, "{lines:?}");
        assert_eq!(events(&s, "land.pr"), 10);
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(
            gh.calls()
                .iter()
                .filter(|c| c.starts_with("pr create"))
                .count(),
            12
        );
    }

    #[test]
    fn propose_branch_opens_a_pull_request_for_a_branch_named_by_hand() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        run(&repo, &["git", "branch", "feat/by-hand", "main"]);
        let tip = commit_on(&repo, "feat/by-hand", "hand.txt");
        let d = propose_branch(&s, &proj, "feat/by-hand", TEST_NOW).unwrap();
        assert_eq!((d.number, d.opened), (7, true));
        assert_eq!(bare_ref(&bare, "refs/heads/feat/by-hand"), tip);
        let pr = last_detail(&s, "land.pr");
        assert_eq!(pr["kind"], "proposed");
        assert!(pr.get("conversation").is_none(), "{pr}");
        assert_eq!(gh.head_of(7), "feat/by-hand");
        let again = propose_branch(&s, &proj, "feat/by-hand", TEST_NOW).unwrap();
        assert_eq!((again.number, again.opened), (7, false));
        assert!(again.line.contains("is delivered at"), "{}", again.line);
        assert!(
            propose_branch(&s, &proj, "main", TEST_NOW)
                .unwrap_err()
                .contains("is the default branch")
        );
        assert!(
            propose_branch(&s, &proj, "nope", TEST_NOW)
                .unwrap_err()
                .contains("is not in")
        );
    }
}
