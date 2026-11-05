//! Every finished branch of a project delivered as a pull request on its
//! upstream, and every pull request PULSE opened followed until the forge
//! settles it. Nothing merges into the local default branch on this
//! machine: it only follows the upstream (`forge::sync_default_from_upstream`).
//!
//! What is delivered, each pass (`tend`):
//! - a task whose jobs are all finished: the jobs' branches merged onto the
//!   default branch as `pulse/task-<id>`, one pull request (`land.pr`,
//!   kind `task`); a branch that conflicts with the default branch goes to
//!   a rebase job on the task once (`land.rebase`) before the task is held;
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
//! The pull requests of a project form one stack (`stack_top`): a new pull
//! request is built on the head of the newest open one and targets it
//! (`base`, `stack_on` in its `land.pr`), so two open pull requests never
//! hold changes to the same lines that only meet at merge time. The stack
//! is plain git branches and explicit bases, nothing of the forge's: when
//! the pull request below merges or closes, the one above is moved onto
//! its base (`retarget_stacked`), and the reviewer takes the stack bottom
//! up (`review.rs`). A conversation's pull request (`pulse/chat-<id>`)
//! stays out of the stack: the hub keeps its branch checked out.
//!
//! The same pass reads the state of every open pull request in one `gh pr
//! list` per repository. Merged: `land.merged`, the delivered jobs lose
//! their branch, a task is done, the local branches go and the default
//! branch is fast-forwarded from the upstream, and a conversation's session
//! (`pulse/chat-<id>`) is finished (`chat.finish`, which the hub's wake loop
//! hands to it). Closed unmerged: `land.pr-closed`, the branches are cleared
//! too (the work stays on the forge's closed pull request; nothing
//! re-delivers it), a task goes back to in_progress with a note for its
//! lead and a conversation gets a note in its transcript, nothing more. A
//! `gh` failure changes nothing: the pull request is read again next pass.
//!
//! An open pull request the forge cannot merge as it stands is looked after
//! once per head sha (`follow_open`): a head that conflicts with its base
//! because the base moved gets the base merged in and pushed when that
//! merge is clean (`land.pr-refreshed`); when it is not, the conflict is
//! recorded (`land.pr-conflict`) and the origin told - a task through its
//! messages, a conversation through a `chat.wake` that gives its agent the
//! turn. Failing checks are recorded (`land.pr-checks`) and told the same
//! way, once per head, never queued as a job: the next job on the work is
//! the fix.
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
                "SELECT id, status, branch,
                    EXISTS (SELECT 1 FROM runs r WHERE r.job_id = jobs.id AND r.status = 'running') AS running
                 FROM jobs WHERE project = ? AND {} ORDER BY created_at, id",
                pulse_store::task_source_clause("source")
            ),
            &[&slug, &exact, &like],
        )
        .unwrap_or_default();
        // A worker marks its own job done (`pulse job finish ... --branch`)
        // as its session ends, naming the branch the runner creates only
        // once the run's verification has passed, up to forty minutes
        // later. A job whose run is still running is in flight, whatever
        // its status says: nine of a day's task holds were a branch "not
        // something we can merge" that existed an hour later.
        if jobs.iter().any(|j| {
            matches!(j.str("status").as_str(), "queued" | "leased" | "blocked")
                || j.int("running") != 0
        }) {
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

/// The hold key of a task's work: every job branch with its sha now, a
/// branch the checkout has not got by name alone.
fn task_work_key(root: &str, branches: &[(String, String)]) -> String {
    branches
        .iter()
        .map(|(_, b)| {
            let sha = rev(root, &format!("refs/heads/{b}"));
            if sha.is_empty() {
                b.clone()
            } else {
                work_key(b, &sha)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A job branch that does not merge onto the task's base with the task's
/// other branches: what conflicted, for the job that rebases it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Conflict {
    job: String,
    branch: String,
    sha: String,
    /// The paths left unmerged, as `git diff --diff-filter=U` names them.
    files: Vec<String>,
    /// What the branch was being merged onto.
    base: Base,
}

/// What a pull request targets: the default branch, or the head of the open
/// pull request it stacks on.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Base {
    /// The branch the pull request targets on the upstream.
    branch: String,
    /// That branch's tip as the work was built on it; empty for the default
    /// branch, whose local copy is the start.
    sha: String,
    /// The pull request whose head `branch` is; 0 for the default branch.
    stack_on: i64,
}

impl Base {
    /// The upstream's default branch, as every pull request targeted before
    /// the stack.
    fn default_branch(up: &Upstream) -> Self {
        Base {
            branch: up.remote_branch.clone(),
            sha: String::new(),
            stack_on: 0,
        }
    }

    fn stacked(&self) -> bool {
        self.stack_on != 0
    }

    /// The git rev the work starts from or is merged with: the checkout's
    /// default branch `def` (it follows the upstream), or the stacked head.
    fn rev(&self, def: &str) -> String {
        if self.sha.is_empty() {
            format!("refs/heads/{def}")
        } else {
            self.sha.clone()
        }
    }

    /// The base in words: `main`, or `pulse/task-X (pull request #N)`.
    fn describe(&self) -> String {
        if self.stacked() {
            format!("{} (pull request #{})", self.branch, self.stack_on)
        } else {
            self.branch.clone()
        }
    }
}

/// Whether a pull request takes part in the stack: every kind but a
/// conversation's (`pulse/chat-<id>`), whose branch the hub keeps checked
/// out in a worktree of its own - nothing may rebuild it, and nothing is
/// built on it.
fn stacks(pr: &Value) -> bool {
    field(pr, "conversation").is_empty() && !field(pr, "head").starts_with("pulse/chat-")
}

/// The top of the project's stack, as its newest `land.pr`: the open pull
/// request opened last, of the kinds that stack. A new pull request is
/// built on its head. None when nothing stacking is open.
fn stack_top(q: &(impl crate::util::Querier + ?Sized), slug: &str) -> Option<Value> {
    open_prs_in_order(q, slug)
        .into_iter()
        .filter(|(_, d)| stacks(d))
        .max_by_key(|(first, _)| *first)
        .map(|(_, d)| d)
}

/// The hold key of work built on the stack's top names the top too: a hold
/// stands until the work or the stack changes.
fn stack_key(key: String, top: Option<&Value>) -> String {
    match top {
        Some(t) => format!("{key} on #{}", number_of(t)),
        None => key,
    }
}

/// The base a new pull request gets when the stack is not empty: the top's
/// head as the upstream has it now (fetched, so a commit the reviewer or
/// the owner pushed onto it is built on too).
fn stack_base(
    s: &pulse_store::Store,
    root: &str,
    up: &Upstream,
    top: &Value,
) -> Result<Base, String> {
    let branch = field_of(top, "head");
    let stack_on = number_of(top);
    let sha = fetch_head(s, root, up, &branch)?;
    if sha.is_empty() {
        return Err(format!(
            "{branch} (pull request #{stack_on}, which the work would stack on) is gone from {}",
            up.remote
        ));
    }
    Ok(Base {
        branch,
        sha,
        stack_on,
    })
}

/// The branch an open pull request targets on record: the default branch,
/// or the head of the pull request it stacks on.
fn base_branch_of<'a>(pr: &'a Value, up: &'a Upstream) -> &'a str {
    match pr.get("base").and_then(Value::as_str) {
        Some(b) if !b.is_empty() => b,
        _ => up.remote_branch.as_str(),
    }
}

/// The base an open pull request has on record: the default branch, or the
/// head of the pull request it stacks on, fetched so a rebuild starts from
/// what the upstream holds.
fn recorded_base(
    s: &pulse_store::Store,
    root: &str,
    up: &Upstream,
    pr: &Value,
) -> Result<Base, String> {
    let branch = base_branch_of(pr, up).to_string();
    if branch == up.remote_branch {
        return Ok(Base::default_branch(up));
    }
    let stack_on = pr.get("stack_on").and_then(Value::as_i64).unwrap_or(0);
    let sha = fetch_head(s, root, up, &branch)?;
    if sha.is_empty() {
        return Err(format!(
            "{branch} (pull request #{stack_on}, which pull request #{} stacks on) is gone from {}",
            number_of(pr),
            up.remote
        ));
    }
    Ok(Base {
        branch,
        sha,
        stack_on,
    })
}

/// Builds `branch` on the stack's top: the top's head merged into the
/// branch in a scratch worktree (the branch is PULSE's own, so a merge
/// commit on it is fine) and the local ref moved to the merge. Returns the
/// branch's tip, unchanged when it holds the top already. A merge that
/// conflicts names the paths and the pull request, and moves nothing.
fn take_in(root: &str, branch: &str, sha: &str, base: &Base) -> Result<String, String> {
    if is_ancestor(root, &base.sha, sha) {
        return Ok(sha.to_string());
    }
    let git = |args: &[&str]| crate::util::git_timeout(root, Duration::from_secs(120), args);
    let git_dir = git(&["rev-parse", "--absolute-git-dir"])
        .stdout
        .trim()
        .to_string();
    if git_dir.is_empty() {
        return Err(format!("{root} is not a git checkout"));
    }
    let wt_path = format!("{git_dir}/pulse-stack/{}", branch.replace('/', "_"));
    let _ = git(&["worktree", "remove", "--force", &wt_path]);
    let _ = std::fs::remove_dir_all(&wt_path);
    let add = git(&["worktree", "add", "--detach", &wt_path, sha]);
    if !add.ok() {
        return Err(format!(
            "worktree add failed: {}",
            crate::run::squeeze(&add.stderr, 200)
        ));
    }
    let in_wt = |args: &[&str]| crate::util::git_timeout(&wt_path, Duration::from_secs(300), args);
    let what = base.describe();
    let m = in_wt(&[
        "-c",
        "user.name=PULSE",
        "-c",
        "user.email=pulse@localhost",
        "merge",
        "--no-ff",
        "--no-edit",
        "-m",
        &format!("Merge {what} into {branch}"),
        &base.sha,
    ]);
    let result = if m.ok() {
        let tip = in_wt(&["rev-parse", "HEAD"]).stdout.trim().to_string();
        let moved = git(&["update-ref", &format!("refs/heads/{branch}"), &tip, sha]);
        if moved.ok() {
            Ok(tip)
        } else {
            Err(format!(
                "{branch} could not be moved onto {what}: {}",
                crate::run::squeeze(&moved.stderr, 200)
            ))
        }
    } else {
        let unmerged: Vec<String> = in_wt(&["diff", "--name-only", "--diff-filter=U"])
            .stdout
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect();
        let _ = in_wt(&["merge", "--abort"]);
        Err(if unmerged.is_empty() {
            format!(
                "merge of {what} into {branch} failed: {}",
                crate::run::squeeze(&format!("{} {}", m.stdout, m.stderr), 200)
            )
        } else {
            format!(
                "{branch} does not merge cleanly onto {what}, which it would stack on: conflicts in {}",
                unmerged.join(", ")
            )
        })
    };
    let _ = git(&["worktree", "remove", "--force", &wt_path]);
    result
}

/// Why a task's pull request could not be built this pass.
#[derive(Debug)]
enum TaskFault {
    Conflict(Box<Conflict>),
    Other(String),
}

impl From<String> for TaskFault {
    fn from(note: String) -> Self {
        TaskFault::Other(note)
    }
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
        // The key names each branch's sha, not the branch alone: a hold
        // stands until the work changes, and a branch that appears or moves
        // (a retry's new run) is new work the pass tries again, rather than
        // waiting for a new job of the task.
        let Ok(conn) = s.conn() else { break };
        let top = if rt.open.is_none() {
            stack_top(&conn, &slug)
        } else {
            None
        };
        let key = stack_key(task_work_key(&root, &rt.branches), top.as_ref());
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
        match deliver_task(s, project, &up, rt, top.as_ref(), now) {
            Ok(line) => lines.push(line),
            Err(TaskFault::Conflict(c)) => {
                lines.push(rebase_or_hold(s, project, rt, &key, *c, now))
            }
            Err(TaskFault::Other(note)) => {
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
        let Ok(conn) = s.conn() else { break };
        // A fix goes onto the pull request it fixes, wherever that stands
        // in the stack; only a pull request of the job's own stacks.
        let top = if rj.fix_of.is_none() {
            stack_top(&conn, &slug)
        } else {
            None
        };
        let key = stack_key(work_key(&branch, &sha), top.as_ref());
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
                deliver_job(s, project, &up, job, &sha, next, top.as_ref(), now).map(|d| d.line)
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
            None,
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
    /// What the pull request targets: the default branch, or the head of
    /// the open pull request it stacks on.
    base: Base,
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

/// Pushes the work's branch to the upstream and opens its pull request on
/// its base, or finds the one open for it, then records `land.pr`.
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
            &["merge-base", &work.base.rev(&def), &work.sha],
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
            &work.base.branch,
        )? {
            Some((n, u)) => (n, u, false),
            None => {
                let (n, u) = crate::landing_pr::create_pull_request(
                    &root,
                    &up.repo,
                    &work.head,
                    &work.base.branch,
                    &work.title,
                    &work.body,
                )?;
                (n, u, true)
            }
        },
    };
    let mut detail = json!({"project": slug, "kind": work.kind, "number": number, "url": url,
        "head": work.head, "base": work.base.branch, "remote": up.remote, "from": from, "to": work.sha,
        "jobs": work.jobs, "commits": commits, "opened": opened, "title": work.title});
    if work.base.stacked() {
        detail["stack_on"] = json!(work.base.stack_on);
    }
    for (k, v) in &work.ids {
        detail[*k] = json!(v);
    }
    event(s, now, "land.pr", &slug, detail);
    let what = work
        .ids
        .first()
        .map(|(k, v)| format!("{k} {v}"))
        .unwrap_or_else(|| work.head.clone());
    let on = if opened && work.base.stacked() {
        format!(" on #{}", work.base.stack_on)
    } else {
        String::new()
    };
    Ok(Delivered {
        number,
        url: url.clone(),
        opened,
        line: format!(
            "{} pr {slug} {what}: #{number} {}{on} {url}",
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
    push_rev(s, root, up, &format!("refs/heads/{local}"), head, lease)
}

/// `push` of any commit the checkout holds (`src`: a ref or a sha) to
/// `head` on the upstream.
fn push_rev(
    s: &pulse_store::Store,
    root: &str,
    up: &Upstream,
    src: &str,
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
        &["push", &force, &up.url, &format!("{src}:refs/heads/{head}")],
    );
    cleanup("", &[]);
    if !pushed.ok() {
        return Err(pulse_cred::redact(
            &format!(
                "push of {} to {head} on {} failed: {}",
                src.strip_prefix("refs/heads/").unwrap_or(src),
                up.name(),
                crate::run::squeeze(&format!("{} {}", pushed.stderr, pushed.stdout), 200)
            ),
            &[],
        ));
    }
    Ok(())
}

/// Builds `pulse/task-<id>` from its base plus every job branch, pushes
/// it and opens (or finds) its pull request. The base is the default branch,
/// or the head of the stack's top `top` when another pull request is open:
/// the task's then stacks on it. When the task's pull request is open
/// already, the base is the one on record, and the rebuilt branch also
/// takes in whatever the pull request's head holds beyond the job branches
/// (a reviewer's or the owner's commits on the forge), so a follow-up job
/// never pushes those away, and the push expects that head.
fn deliver_task(
    s: &pulse_store::Store,
    project: &Row,
    up: &Upstream,
    rt: &ReadyTask,
    top: Option<&Value>,
    now: &str,
) -> Result<String, TaskFault> {
    let root = project.str("path");
    let def = crate::baseline::default_branch(project.clone());
    let task = rt.task.as_str();
    let branches = rt.branches.as_slice();
    let head = task_branch(task);
    let base = match (&rt.open, top) {
        (Some(open), _) => recorded_base(s, &root, up, open)?,
        (None, Some(top)) => stack_base(s, &root, up, top)?,
        (None, None) => Base::default_branch(up),
    };
    let start = base.rev(&def);
    // Each branch is resolved here, in the checkout, before anything is
    // merged: a name git cannot resolve inside the worktree reads as "not
    // something we can merge", which says nothing about which branch or
    // why.
    let mut shas = Vec::with_capacity(branches.len());
    for (job, b) in branches {
        let sha = rev(&root, &format!("refs/heads/{b}"));
        if sha.is_empty() {
            return Err(format!("{b} (job {job}) is not in {root}").into());
        }
        shas.push(sha);
    }
    let remote_head = match &rt.open {
        Some(_) => {
            let sha = fetch_head(s, &root, up, &head)?;
            if sha.is_empty() {
                return Err(format!(
                    "{head} (pull request #{}) is gone from {}",
                    rt.open.as_ref().map(number_of).unwrap_or_default(),
                    up.name()
                )
                .into());
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
        return Err(format!("{root} is not a git checkout").into());
    }
    let wt_path = format!("{git_dir}/pulse-task-pr/{task}");
    let _ = git(&["worktree", "remove", "--force", &wt_path]);
    let _ = std::fs::remove_dir_all(&wt_path);
    let add = git(&["worktree", "add", "--force", "-B", &head, &wt_path, &start]);
    if !add.ok() {
        return Err(format!(
            "worktree add failed: {}",
            crate::run::squeeze(&add.stderr, 200)
        )
        .into());
    }
    let in_wt = |args: &[&str]| crate::util::git_timeout(&wt_path, Duration::from_secs(300), args);
    // (rev, what it is, the job whose branch it is).
    let mut merges: Vec<(String, String, Option<usize>)> = branches
        .iter()
        .enumerate()
        .map(|(i, (_, b))| (shas[i].clone(), b.clone(), Some(i)))
        .collect();
    if let Some(sha) = &remote_head {
        merges.push((
            sha.clone(),
            format!(
                "{head} as pull request #{} has it",
                rt.open.as_ref().map(number_of).unwrap_or_default()
            ),
            None,
        ));
    }
    let mut result: Result<(), TaskFault> = Ok(());
    for (rev, what, which) in &merges {
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
            let unmerged: Vec<String> = in_wt(&["diff", "--name-only", "--diff-filter=U"])
                .stdout
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect();
            let _ = in_wt(&["merge", "--abort"]);
            result = Err(match which {
                Some(i) if !unmerged.is_empty() => TaskFault::Conflict(Box::new(Conflict {
                    job: branches[*i].0.clone(),
                    branch: what.clone(),
                    sha: rev.clone(),
                    files: unmerged,
                    base: base.clone(),
                })),
                _ => TaskFault::Other(format!(
                    "{what} does not merge cleanly onto {} with the task's other branches: {}",
                    base.describe(),
                    crate::run::squeeze(&format!("{} {}", m.stdout, m.stderr), 200)
                )),
            });
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
            base,
            open: rt.open.clone(),
            lease: remote_head,
        },
        now,
    )?
    .line)
}

/// How many times a task's conflicting branch is handed to a job before the
/// owner is asked: the rebase job starts from the base as it is, so a
/// second conflict means the work itself is at odds with what landed.
const REBASES_PER_TASK: usize = 1;

/// A job branch that conflicts with the task's base - the default branch,
/// or the pull request the task's stacks on - is work for an agent before
/// it is a question for the owner: a `task` job on the same task is queued
/// to bring the branch over onto the base and resolve the conflict, the
/// conflicting job keeps its result but loses the branch (the task's pull
/// request opens from the rebased one), and `land.rebase` says so. After
/// `REBASES_PER_TASK` such jobs the task is held with the conflict named,
/// as before.
fn rebase_or_hold(
    s: &pulse_store::Store,
    project: &Row,
    rt: &ReadyTask,
    key: &str,
    c: Conflict,
    now: &str,
) -> String {
    let slug = project.str("slug");
    let task = rt.task.as_str();
    let files = c.files.join(", ");
    let onto = c.base.describe();
    let def = c.base.branch.as_str();
    let note = format!(
        "{} (job {}) does not merge cleanly onto {onto} with the task's other branches: conflicts in {files}",
        c.branch, c.job
    );
    let Ok(conn) = s.conn() else {
        return format!("task pr {slug} {task}: {note}");
    };
    let rebases = crate::util::query_rows(
        &conn,
        "SELECT detail FROM events WHERE subject = ? AND action = 'land.rebase' AND json_valid(detail)
         AND json_extract(detail, '$.task') = ? ORDER BY seq DESC",
        &[&slug, &task],
    )
    .unwrap_or_default();
    if rebases.len() >= REBASES_PER_TASK {
        let last: Value = serde_json::from_str(&rebases[0].str("detail")).unwrap_or(Value::Null);
        drop(conn);
        let note = format!(
            "{note}; job {} already rebased the task's work once, so this one is for you",
            field(&last, "follow_up")
        );
        hold(s, now, &slug, json!({"task": task, "work": key}), &note);
        return format!("task pr {slug} {task}: held: {note}");
    }
    let job = crate::util::must_one(&conn, "SELECT * FROM jobs WHERE id = ?", &[&c.job]);
    drop(conn);
    let short = crate::recipe::short(&c.sha, 12);
    // A stacked task's branch goes onto the pull request below, not onto
    // the default branch: the brief says which, and that the worker has
    // to fetch it.
    let which = if c.base.stacked() {
        format!(
            " - the head of open pull request #{}, which the task's pull request stacks on; fetch it from the upstream first",
            c.base.stack_on
        )
    } else {
        String::new()
    };
    let brief = format!(
        "Branch `{branch}` (job {job_id}, commit {short}) is finished work for task {task} ({title}), but it no longer merges onto `{def}`: {files} conflict with what landed on `{def}` after the branch was cut.\n\n\
         Start from the current `{def}`{which}, bring the branch's commits over (`git cherry-pick` the commits `{def}..{short}` has, or rebase the branch), resolve the conflicts keeping the intent of both sides, run the project's check, and commit on your own branch. \
         Leave `{branch}` as it is; the task's pull request opens from your branch once it merges cleanly.\n\n\
         The job's own account of its work:\n\n{result}",
        branch = c.branch,
        job_id = c.job,
        title = rt.title,
        result = crate::run::squeeze(&job.str("result"), 1200),
    );
    let queued = crate::jobs::add_job(
        s,
        slug.clone(),
        "task".into(),
        format!("Rebase {} onto {def}: {}", c.branch, rt.title),
        brief,
        job.int("tier").max(1),
        job.int("priority"),
        format!("task:{task}"),
        ACTOR.into(),
        false,
    );
    let follow_up = match queued {
        Ok((j, _)) => j.str("id"),
        Err(e) => {
            let note = format!("{note}; no job could be queued to rebase it: {e}");
            hold(s, now, &slug, json!({"task": task, "work": key}), &note);
            return format!("task pr {slug} {task}: held: {note}");
        }
    };
    if let Ok(conn) = s.conn() {
        let _ = conn.execute(
            "UPDATE jobs SET branch = '', result = result || ? WHERE id = ? AND project = ?",
            rusqlite::params![
                format!(
                    "\n\nIts branch {} conflicts with {def} in {files}; job {follow_up} rebases it.",
                    c.branch
                ),
                c.job,
                slug
            ],
        );
    }
    event(
        s,
        now,
        "land.rebase",
        &slug,
        json!({
            "project": slug, "task": task, "job": c.job, "branch": c.branch, "sha": c.sha,
            "files": c.files, "follow_up": follow_up, "base": def,
        }),
    );
    let _ = pulse_tasks::send_message(
        s,
        &slug,
        task,
        ACTOR,
        &format!(
            "Branch {} of job {} conflicts with {onto} in {files}; job {follow_up} rebases it before the task's pull request opens.",
            c.branch, c.job
        ),
    );
    format!("task pr {slug} {task}: {note}; job {follow_up} rebases it")
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

/// The base of a stand-alone branch's pull request and the branch's tip
/// once built on it: a pull request to open stacks on `top`, whose head is
/// merged into the branch (`take_in`); one open already keeps the base on
/// record and the branch as it is. A branch some worktree has checked out
/// is left as it is too, on the default branch: moving it would move the
/// checkout under whoever works there.
fn stacked_tip(
    s: &pulse_store::Store,
    root: &str,
    up: &Upstream,
    branch: &str,
    sha: &str,
    next: &Next,
    top: Option<&Value>,
) -> Result<(Base, String), String> {
    match (next, top) {
        (Next::Update(d), _) => Ok((recorded_base(s, root, up, d)?, sha.to_string())),
        (_, Some(top)) if !worktree_branches(root).iter().any(|b| b == branch) => {
            let base = stack_base(s, root, up, top)?;
            let tip = take_in(root, branch, sha, &base)?;
            Ok((base, tip))
        }
        _ => Ok((Base::default_branch(up), sha.to_string())),
    }
}

/// One done job's branch as the agent left it, as one pull request - built
/// on the stack's top `top` when one is open.
#[allow(clippy::too_many_arguments)]
fn deliver_job(
    s: &pulse_store::Store,
    project: &Row,
    up: &Upstream,
    job: &Row,
    sha: &str,
    next: Next,
    top: Option<&Value>,
    now: &str,
) -> Result<Delivered, String> {
    let root = project.str("path");
    let id = job.str("id");
    let branch = job.str("branch");
    let (base, tip) = stacked_tip(s, &root, up, &branch, sha, &next, top)?;
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
            sha: tip,
            base,
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
/// A branch named by hand stacks on `top`; a conversation's never does
/// (the hub has it checked out, so nothing is merged into it).
#[allow(clippy::too_many_arguments)]
fn deliver_proposed(
    s: &pulse_store::Store,
    project: &Row,
    up: &Upstream,
    branch: &str,
    sha: &str,
    conversation: Option<String>,
    next: Next,
    top: Option<&Value>,
    now: &str,
) -> Result<Delivered, String> {
    let root = project.str("path");
    let def = crate::baseline::default_branch(project.clone());
    let top = if conversation.is_some() { None } else { top };
    let (base, sha) = stacked_tip(s, &root, up, branch, sha, &next, top)?;
    let sha = sha.as_str();
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
            base,
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
    let conversation = branch
        .strip_prefix("pulse/chat-")
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    let conn = s.conn().map_err(|e| e.to_string())?;
    let top = if conversation.is_some() {
        None
    } else {
        stack_top(&conn, &slug)
    };
    let next = next_for(
        &conn,
        &root,
        &slug,
        "head",
        branch,
        &sha,
        &stack_key(work_key(branch, &sha), top.as_ref()),
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
    deliver_proposed(
        s,
        project,
        &up,
        branch,
        &sha,
        conversation,
        next,
        top.as_ref(),
        now,
    )
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
    let top = stack_top(&conn, &slug);
    let next = next_for(
        &conn,
        &root,
        &slug,
        "job",
        &id,
        &sha,
        &stack_key(work_key(&branch, &sha), top.as_ref()),
    );
    drop(conn);
    if next == Next::Nothing {
        let conn = s.conn().map_err(|e| e.to_string())?;
        let d = open_pr_for_job(&conn, &slug, &id).ok_or_else(|| {
            format!(
                "{branch} was delivered at {} already",
                crate::recipe::short(&sha, 12)
            )
        })?;
        return Ok((number_of(&d), field_of(&d, "url"), false));
    }
    let (base, sha) = stacked_tip(s, &root, &up, &branch, &sha, &next, top.as_ref())?;
    let open = match next {
        Next::Update(d) => Some(d),
        _ => None,
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
            base,
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
    open_prs_in_order(q, slug)
        .into_iter()
        .map(|(_, d)| d)
        .collect()
}

/// `open_prs` with, for each, the seq of the `land.pr` that opened it: the
/// order the stack was built in, which a later push does not change.
fn open_prs_in_order(q: &(impl crate::util::Querier + ?Sized), slug: &str) -> Vec<(i64, Value)> {
    let rows = crate::util::query_rows(
        q,
        "SELECT seq, action, detail FROM events WHERE subject = ? AND action IN ('land.pr', 'land.merged', 'land.pr-closed')
         AND json_valid(detail) ORDER BY seq",
        &[&slug],
    )
    .unwrap_or_default();
    let mut by_number: BTreeMap<i64, (i64, String, Value)> = BTreeMap::new();
    for r in rows {
        let d: Value = serde_json::from_str(&r.str("detail")).unwrap_or(Value::Null);
        let n = number_of(&d);
        if n == 0 {
            continue;
        }
        let action = r.str("action");
        let seq = i64::from(r.int("seq"));
        match by_number.get(&n) {
            // A later land.pr of a settled number is a new pull request's
            // push only when the forge reused the number, which it does not;
            // it is the push of new commits recorded after the merge was
            // seen, and the merge stands.
            Some((_, prev, _)) if prev != "land.pr" && action == "land.pr" => {}
            Some((first, prev, _)) if prev == "land.pr" => {
                let first = *first;
                by_number.insert(n, (first, action, d));
            }
            _ => {
                by_number.insert(n, (seq, action, d));
            }
        }
    }
    by_number
        .into_values()
        .filter(|(_, a, _)| a == "land.pr")
        .map(|(first, _, d)| (first, d))
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
            "MERGED" => {
                lines.push(settle_merged(s, project, &pr, &state, now));
                lines.extend(retarget_stacked(s, project, &up, &pr, &listed, false, now));
            }
            "CLOSED" => {
                lines.push(settle_closed(s, project, &pr, &state, now));
                lines.extend(retarget_stacked(s, project, &up, &pr, &listed, true, now));
            }
            "OPEN" => lines.extend(follow_open(s, project, &up, &pr, &state, now)),
            _ => {}
        }
    }
    lines
}

/// An open pull request the forge cannot merge as it stands, looked after
/// once per head sha: a head that conflicts with its base gets the base
/// merged in and pushed when that merge is clean, else the conflict is
/// recorded and the origin told; failing checks are recorded and the origin
/// told. Each follow-up event carries the head sha it was about, so a head
/// is never acted on twice, and a head the forge has not judged yet
/// (`UNKNOWN`, or no sha at all from an older gh) waits for the next pass.
fn follow_open(
    s: &pulse_store::Store,
    project: &Row,
    up: &Upstream,
    pr: &Value,
    st: &crate::landing_pr::PrState,
    now: &str,
) -> Vec<String> {
    let slug = project.str("slug");
    let number = number_of(pr);
    let mut lines = Vec::new();
    if st.head_sha.is_empty() {
        return lines;
    }
    let Ok(conn) = s.conn() else {
        return lines;
    };
    let conflict = st.conflicting()
        && !followed(
            &conn,
            &slug,
            number,
            &["land.pr-refreshed", "land.pr-conflict"],
            &st.head_sha,
        );
    let checks = !st.failing_checks.is_empty()
        && !followed(&conn, &slug, number, &["land.pr-checks"], &st.head_sha);
    drop(conn);
    if conflict {
        lines.push(refresh_pr_branch(s, project, up, pr, st, now));
    }
    if checks {
        lines.push(note_failing_checks(s, project, up, pr, st, now));
    }
    lines
}

/// Whether one of `actions` was already recorded for pull request `number`
/// at head `sha`.
fn followed(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    number: i64,
    actions: &[&str],
    sha: &str,
) -> bool {
    let actions = actions
        .iter()
        .map(|a| format!("'{a}'"))
        .collect::<Vec<_>>()
        .join(", ");
    !crate::util::query_rows(
        q,
        &format!(
            "SELECT seq FROM events WHERE subject = ?1 AND action IN ({actions}) AND json_valid(detail)
             AND json_extract(detail, '$.number') = ?2 AND json_extract(detail, '$.head_sha') = ?3 LIMIT 1"
        ),
        &[&slug, &number, &sha],
    )
    .unwrap_or_default()
    .is_empty()
}

/// How a pull request's head took its base.
enum Refreshed {
    /// The merge was clean and is on the pull request now, at this sha.
    Pushed(String),
    /// The merge conflicted in these paths; nothing was pushed.
    Conflict(Vec<String>),
}

/// The pull request's head conflicts with its base: the base moved after
/// the head was pushed. The base is merged into the head in a scratch
/// worktree and the merge pushed onto the pull request when it is clean
/// (`land.pr-refreshed`, and a `land.pr` naming the new tip, so the next
/// push of the work leases on what the pull request holds, as a fix's
/// does). A merge that conflicts is recorded once for this head
/// (`land.pr-conflict`, with the paths) and the origin told: the work
/// itself is at odds with what landed, which is an agent's to resolve. A
/// head that moved since the forge was read, or a fetch or push that
/// fails, leaves no record and is read again next pass.
fn refresh_pr_branch(
    s: &pulse_store::Store,
    project: &Row,
    up: &Upstream,
    pr: &Value,
    st: &crate::landing_pr::PrState,
    now: &str,
) -> String {
    let slug = project.str("slug");
    let root = project.str("path");
    let kind = kind_of(pr);
    let number = number_of(pr);
    let head = if st.head.is_empty() {
        field_of(pr, "head")
    } else {
        st.head.clone()
    };
    let base = base_branch_of(pr, up);
    let url = field_of(pr, "url");
    let short = crate::recipe::short(&st.head_sha, 12);
    let line = format!("{kind} pr {slug}: #{number} conflicts with {base} at {short}");
    match merge_base_into_head(s, &root, up, &head, &st.head_sha, base) {
        Ok(Refreshed::Pushed(tip)) => {
            event(
                s,
                now,
                "land.pr-refreshed",
                &slug,
                json!({"project": slug, "kind": kind, "number": number, "head": head,
                    "head_sha": st.head_sha, "new_sha": tip, "base": base}),
            );
            let commits = crate::util::git_timeout(
                &root,
                Duration::from_secs(30),
                &["rev-list", "--count", &format!("{}..{tip}", st.head_sha)],
            )
            .stdout
            .trim()
            .parse::<i64>()
            .unwrap_or(1);
            let mut detail = pr.clone();
            for (k, v) in [
                ("project", json!(slug)),
                ("kind", json!(kind)),
                ("from", json!(st.head_sha)),
                ("to", json!(tip)),
                ("commits", json!(commits)),
                ("opened", json!(false)),
                ("refreshed", json!(true)),
            ] {
                detail[k] = v;
            }
            event(s, now, "land.pr", &slug, detail);
            format!(
                "{line}; {base} merged in and pushed, head now {}",
                crate::recipe::short(&tip, 12)
            )
        }
        Ok(Refreshed::Conflict(files)) => {
            event(
                s,
                now,
                "land.pr-conflict",
                &slug,
                json!({"project": slug, "kind": kind, "number": number, "head": head,
                    "head_sha": st.head_sha, "base": base, "files": files}),
            );
            let files = files.join(", ");
            let told = tell_origin(
                s,
                pr,
                now,
                &format!(
                    "Pull request #{number} ({url}) conflicts with {base} at {short}: {files}. \
                     {base} moved after {head} was pushed and does not merge in cleanly. \
                     Bring {base} into {head}, resolve the conflicts keeping the intent of both sides, run the project's check and push; the pull request follows the branch."
                ),
            );
            format!("{line}: {files}; {base} does not merge in cleanly{told}")
        }
        Err(note) => format!("{line}; not refreshed: {note}"),
    }
}

/// Merges the upstream's base into the pull request's head in a scratch
/// worktree of the checkout and pushes the merge onto the head, leasing on
/// the tip the forge named; the worktree goes whatever happens.
fn merge_base_into_head(
    s: &pulse_store::Store,
    root: &str,
    up: &Upstream,
    head: &str,
    head_sha: &str,
    base: &str,
) -> Result<Refreshed, String> {
    let remote_sha = fetch_head(s, root, up, head)?;
    if remote_sha.is_empty() {
        return Err(format!("{head} is gone from {}", up.name()));
    }
    if remote_sha != head_sha {
        return Err(format!(
            "{head} moved to {} since the forge was read",
            crate::recipe::short(&remote_sha, 12)
        ));
    }
    let base_sha = fetch_head(s, root, up, base)?;
    if base_sha.is_empty() {
        return Err(format!("{base} is gone from {}", up.remote));
    }
    let git = |args: &[&str]| crate::util::git_timeout(root, Duration::from_secs(120), args);
    let git_dir = git(&["rev-parse", "--absolute-git-dir"])
        .stdout
        .trim()
        .to_string();
    if git_dir.is_empty() {
        return Err(format!("{root} is not a git checkout"));
    }
    let wt_path = format!("{git_dir}/pulse-pr-refresh/{}", head.replace('/', "_"));
    let _ = git(&["worktree", "remove", "--force", &wt_path]);
    let _ = std::fs::remove_dir_all(&wt_path);
    let add = git(&["worktree", "add", "--detach", &wt_path, &remote_sha]);
    if !add.ok() {
        return Err(format!(
            "worktree add failed: {}",
            crate::run::squeeze(&add.stderr, 200)
        ));
    }
    let in_wt = |args: &[&str]| crate::util::git_timeout(&wt_path, Duration::from_secs(300), args);
    let m = in_wt(&[
        "-c",
        "user.name=PULSE",
        "-c",
        "user.email=pulse@localhost",
        "merge",
        "--no-ff",
        "--no-edit",
        "-m",
        &format!("Merge {base} into {head}"),
        &base_sha,
    ]);
    let result = if m.ok() {
        let tip = in_wt(&["rev-parse", "HEAD"]).stdout.trim().to_string();
        if tip == remote_sha {
            // The forge's judgement is from before a push that took the
            // base in already; it judges the head again on its own.
            Err(format!(
                "{head} has {} already; the forge's judgement is stale",
                up.name()
            ))
        } else {
            push_rev(s, root, up, &tip, head, Some(&remote_sha)).map(|()| Refreshed::Pushed(tip))
        }
    } else {
        let unmerged: Vec<String> = in_wt(&["diff", "--name-only", "--diff-filter=U"])
            .stdout
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect();
        let _ = in_wt(&["merge", "--abort"]);
        if unmerged.is_empty() {
            Err(format!(
                "merge of {base} into {head} failed: {}",
                crate::run::squeeze(&format!("{} {}", m.stdout, m.stderr), 200)
            ))
        } else {
            Ok(Refreshed::Conflict(unmerged))
        }
    };
    let _ = git(&["worktree", "remove", "--force", &wt_path]);
    result
}

/// The checks on the pull request's head failed: recorded once for this
/// head (`land.pr-checks`) and the origin told, so the next job on the work
/// fixes them. No job is queued here: a new head is judged anew, and a job
/// per failing run would chase its own pushes.
fn note_failing_checks(
    s: &pulse_store::Store,
    project: &Row,
    up: &Upstream,
    pr: &Value,
    st: &crate::landing_pr::PrState,
    now: &str,
) -> String {
    let slug = project.str("slug");
    let kind = kind_of(pr);
    let number = number_of(pr);
    let head = if st.head.is_empty() {
        field_of(pr, "head")
    } else {
        st.head.clone()
    };
    let url = field_of(pr, "url");
    let short = crate::recipe::short(&st.head_sha, 12);
    event(
        s,
        now,
        "land.pr-checks",
        &slug,
        json!({"project": slug, "kind": kind, "number": number, "head": head,
            "head_sha": st.head_sha, "base": base_branch_of(pr, up), "checks": st.failing_checks}),
    );
    let names = st.failing_checks.join(", ");
    let told = tell_origin(
        s,
        pr,
        now,
        &format!(
            "Checks are failing on pull request #{number} ({url}) at {short}: {names}. \
             A fix on {head} is the next job on this work; the pull request follows the branch."
        ),
    );
    format!("{kind} pr {slug}: #{number} checks failing at {short}: {names}{told}")
}

/// Tells the pull request's origin what stands in its way: a task through
/// its messages (its lead reads them and plans the fix), a conversation
/// through a `chat.wake` on the bus that gives its agent a turn on the text
/// (the hub's wake loop delivers it, as it does `pulse chat wake`), anything
/// else only through the pass's log line. Returns the log line's tail.
fn tell_origin(s: &pulse_store::Store, pr: &Value, now: &str, text: &str) -> String {
    let slug = field(pr, "project");
    let task = field(pr, "task");
    if !task.is_empty() {
        return match pulse_tasks::send_message(s, slug, task, ACTOR, text) {
            Ok(_) => format!("; task {task} told"),
            Err(e) => format!("; task {task} not told: {e}"),
        };
    }
    let Some(id) = told_conversation(s, pr) else {
        return String::new();
    };
    let Ok(conn) = s.conn() else {
        return String::new();
    };
    crate::util::event_at(
        &conn,
        now,
        ACTOR,
        pulse_harness::conversation::WAKE_ACTION,
        &id,
        json!({"text": text}),
    );
    format!("; conversation {id} woken")
}

/// The conversation a pull request came from (the `conversation` of its
/// `land.pr`), when it still stands where it can be told: there, and not
/// archived.
fn told_conversation(s: &pulse_store::Store, pr: &Value) -> Option<String> {
    let id = field(pr, "conversation").trim();
    if id.is_empty() {
        return None;
    }
    match s.chat_get(id) {
        Ok(Some(row)) if row.status != "archived" => Some(id.to_string()),
        _ => None,
    }
}

/// A conversation's pull request merged: its session is finished with the
/// pull request named. The daemon cannot reach the hub's live session, so
/// it says so on the bus (`chat.finish`); the hub's wake loop finishes it
/// there, after a turn in flight rather than through it, and a hub that is
/// down picks the event up from its cursor when it starts. Returns the log
/// note, None when the pull request has no conversation to finish.
fn finish_conversation(
    s: &pulse_store::Store,
    pr: &Value,
    number: i64,
    url: &str,
    now: &str,
) -> Option<String> {
    let id = told_conversation(s, pr)?;
    let base = field(pr, "base");
    let into = if base.is_empty() {
        String::new()
    } else {
        format!(" into {base}")
    };
    let text = if url.is_empty() {
        format!("pull request #{number} merged{into}; this session is finished")
    } else {
        format!("pull request #{number} ({url}) merged{into}; this session is finished")
    };
    let conn = s.conn().ok()?;
    crate::util::event_at(
        &conn,
        now,
        ACTOR,
        pulse_harness::conversation::FINISH_ACTION,
        &id,
        json!({"text": text, "number": number, "url": url}),
    );
    Some(format!("conversation {id} finished"))
}

/// A conversation's pull request closed without a merge: a note in its
/// transcript says so and nothing else changes - the conversation stays as
/// it is, its agent gets no turn, nothing is marked done. Returns the log
/// note, None when there is no conversation to tell.
fn note_conversation_closed(
    s: &pulse_store::Store,
    pr: &Value,
    number: i64,
    url: &str,
) -> Option<String> {
    let id = told_conversation(s, pr)?;
    let at = if url.is_empty() {
        String::new()
    } else {
        format!(" ({url})")
    };
    let text = format!(
        "Pull request #{number}{at} was closed without merging. Nothing is marked done; the branch stays on the remote. Write here to carry on."
    );
    s.chat_add_message(
        &id,
        "note",
        &text,
        &json!({"pr": {"number": number, "url": url, "state": "closed"}}),
    )
    .ok()?;
    Some(format!("conversation {id} told"))
}

/// Moves the pull requests stacked on a settled one onto its base: each
/// open pull request whose `base` is the settled head is retargeted on the
/// forge (`gh pr edit --base`) and gets a fresh `land.pr` with the new base
/// (`retargeted_from` names the old one), so delivery and the reviewer see
/// where it stands. The forge may move them itself when it deletes a merged
/// branch; PULSE does not count on it. One retargeted because the pull
/// request below was closed unmerged carries that one's commits now, which
/// its task is told once (`land.pr-unstacked` for the other kinds). A
/// failure of the forge leaves no event, so the next pass tries again.
fn retarget_stacked(
    s: &pulse_store::Store,
    project: &Row,
    up: &Upstream,
    settled: &Value,
    listed: &BTreeMap<i64, crate::landing_pr::PrState>,
    closed: bool,
    now: &str,
) -> Vec<String> {
    let slug = project.str("slug");
    let root = project.str("path");
    let head = field_of(settled, "head");
    let settled_number = number_of(settled);
    let Ok(conn) = s.conn() else {
        return Vec::new();
    };
    let open = open_prs(&conn, &slug);
    drop(conn);
    let above: Vec<&Value> = open
        .iter()
        .filter(|pr| number_of(pr) != settled_number && field(pr, "base") == head)
        .collect();
    if above.is_empty() {
        return Vec::new();
    }
    // The new base is the settled pull request's own, while that is still
    // an open pull request's head; the default branch otherwise.
    let (base, stack_on) = {
        let b = field_of(settled, "base");
        let on = settled.get("stack_on").and_then(Value::as_i64).unwrap_or(0);
        let still_open = on != 0
            && open.iter().any(|pr| number_of(pr) == on)
            && listed.get(&on).is_none_or(|st| st.state == "OPEN");
        if !b.is_empty() && b != up.remote_branch && still_open {
            (b, on)
        } else {
            (up.remote_branch.clone(), 0)
        }
    };
    let mut lines = Vec::new();
    for pr in above {
        let n = number_of(pr);
        let kind = kind_of(pr);
        // One merged or closed too settles in this very pass.
        if listed.get(&n).is_some_and(|st| st.state != "OPEN") {
            continue;
        }
        if let Err(e) = crate::landing_pr::retarget_pull_request(&root, &up.repo, n, &base) {
            lines.push(format!(
                "{kind} pr {slug}: #{n} not retargeted from {head} to {base}: {e}"
            ));
            continue;
        }
        let mut d = pr.clone();
        d["base"] = json!(base);
        if stack_on != 0 {
            d["stack_on"] = json!(stack_on);
        } else if let Some(o) = d.as_object_mut() {
            o.remove("stack_on");
        }
        d["opened"] = json!(false);
        d["retargeted_from"] = json!(head);
        event(s, now, "land.pr", &slug, d);
        let mut line = format!("{kind} pr {slug}: #{n} retargeted from {head} to {base}");
        if closed {
            let note = format!(
                "Pull request #{n} was stacked on #{settled_number}, which was closed unmerged; #{n} now carries #{settled_number}'s commits as well as its own. Rebuild it without them, or merge it as it is."
            );
            let task = field_of(pr, "task");
            if task.is_empty() {
                event(
                    s,
                    now,
                    "land.pr-unstacked",
                    &slug,
                    json!({"project": slug, "kind": kind, "number": n, "url": field_of(pr, "url"),
                        "head": field_of(pr, "head"), "base": base, "closed": settled_number, "note": note}),
                );
            } else {
                let _ = pulse_tasks::send_message(s, &slug, &task, ACTOR, &note);
            }
            line.push_str(&format!("; it carries #{settled_number}'s commits"));
        }
        lines.push(line);
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
    let url = if st.url.is_empty() {
        field(pr, "url")
    } else {
        st.url.as_str()
    };
    if let Some(note) = finish_conversation(s, pr, number, url, now) {
        line.push_str(&format!("; {note}"));
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
    let told = note_conversation_closed(s, pr, number, &url)
        .map(|note| format!("; {note}"))
        .unwrap_or_default();
    let task = field(pr, "task");
    if task.is_empty() {
        return format!(
            "{kind} pr {slug}: #{number} closed unmerged; the branch stays on the remote{told}"
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
        task_with_branches(
            s,
            proj,
            repo,
            "Add the thing",
            &["pulse/worker/a", "pulse/worker/b"],
        )
    }

    /// A task in progress with one done job per branch named.
    fn task_with_branches(
        s: &pulse_store::Store,
        proj: &Row,
        repo: &str,
        title: &str,
        names: &[&str],
    ) -> String {
        let task = pulse_tasks::new(s, "proj", title, "b", 0, pulse_store::HUMAN_AGENT).unwrap();
        for name in names {
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

    /// Moves the upstream's `main` on by one commit writing `file`, made on
    /// a throwaway branch of the checkout so the local `main` keeps
    /// following the upstream; returns the new upstream tip.
    fn move_base_on_forge(repo: &str, bare: &str, file: &str) -> String {
        run(repo, &["git", "branch", "base-move", "main"]);
        let tip = commit_on(repo, "base-move", file);
        run(repo, &["git", "push", "-q", bare, "base-move:main"]);
        run(repo, &["git", "branch", "-D", "base-move"]);
        tip
    }

    /// The messages on `task` whose body has `like` in it.
    fn task_messages(s: &pulse_store::Store, task: &str, like: &str) -> i32 {
        count(
            s,
            "SELECT COUNT(*) AS n FROM messages WHERE task_id = ? AND body LIKE ?",
            &[&task, &format!("%{like}%")],
        )
    }

    /// The checkout's worktrees besides the root, by path.
    fn extra_worktrees(repo: &str) -> Vec<String> {
        run(repo, &["git", "worktree", "list", "--porcelain"])
            .lines()
            .filter_map(|l| l.strip_prefix("worktree "))
            .skip(1)
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn an_open_pull_request_that_conflicts_with_its_base_takes_the_base_in_once_per_head() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        gh.track_remote(&bare);
        let task = task_with_two_branches(&s, &proj, &repo);
        let head = task_branch(&task);
        tend_proj(&s, &proj, TEST_NOW);
        let first = bare_ref(&bare, &format!("refs/heads/{head}"));
        assert_eq!(last_detail(&s, "land.pr")["to"], first);

        // The base moves on in another file: the forge calls the head
        // conflicting, the pass merges the base in and pushes.
        let base = move_base_on_forge(&repo, &bare, "elsewhere.txt");
        gh.set_mergeable(7, "CONFLICTING");
        let lines = tend_proj(&s, &proj, TEST_NOW);
        let tip = bare_ref(&bare, &format!("refs/heads/{head}"));
        assert_ne!(tip, first);
        assert!(
            lines.iter().any(|l| l.contains(&format!(
                "task pr proj: #7 conflicts with main at {}; main merged in and pushed, head now {}",
                crate::recipe::short(&first, 12),
                crate::recipe::short(&tip, 12)
            ))),
            "{lines:?}"
        );
        let parents = run(&repo, &["git", "rev-list", "--parents", "-1", &tip]);
        assert_eq!(
            parents.split_whitespace().skip(1).collect::<Vec<_>>(),
            [first.as_str(), base.as_str()],
            "a merge of the base into the head as it was"
        );
        assert_eq!(events(&s, "land.pr-refreshed"), 1);
        let refreshed = last_detail(&s, "land.pr-refreshed");
        assert_eq!(refreshed["kind"], "task");
        assert_eq!(refreshed["number"], 7);
        assert_eq!(refreshed["head"], head);
        assert_eq!(refreshed["head_sha"], first);
        assert_eq!(refreshed["new_sha"], tip);
        assert_eq!(refreshed["base"], "main");
        // The pull request's record names the new tip: the next push onto
        // it leases on what it holds.
        let pr = last_detail(&s, "land.pr");
        assert_eq!(pr["to"], tip);
        assert_eq!(pr["from"], first);
        assert_eq!(pr["refreshed"], true);
        assert_eq!(pr["opened"], false);
        assert_eq!(pr["task"], task);
        assert_eq!(events(&s, "land.pr-conflict"), 0);
        assert_eq!(
            task_messages(&s, &task, "conflicts"),
            0,
            "a clean refresh is no news"
        );
        assert!(
            extra_worktrees(&repo).is_empty(),
            "the scratch worktree is gone"
        );
        assert_eq!(status(&s, &task), "in_progress");

        // The forge's listing lags the push and still names the old head:
        // done for that head already, nothing happens.
        gh.pin_head_sha(7, &first);
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(events(&s, "land.pr-refreshed"), 1);
        assert_eq!(bare_ref(&bare, &format!("refs/heads/{head}")), tip);
        // The listing names the new head but the judgement is stale: the
        // base is in it already, nothing is pushed or recorded.
        gh.pin_head_sha(7, "");
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains("has origin/main already")),
            "{lines:?}"
        );
        assert_eq!(events(&s, "land.pr-refreshed"), 1);
        assert_eq!(events(&s, "land.pr"), 2);
        assert_eq!(bare_ref(&bare, &format!("refs/heads/{head}")), tip);
        gh.set_mergeable(7, "MERGEABLE");
        assert!(tend_proj(&s, &proj, TEST_NOW).is_empty());

        // Merged with the base in it: the task is done as ever.
        merge_on_forge(&gh, &bare, 7);
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains("#7 merged; task done")),
            "{lines:?}"
        );
        assert_eq!(status(&s, &task), "done");
        assert_eq!(ref_of(&repo, "refs/heads/main"), tip);
    }

    #[test]
    fn a_conflict_the_base_merge_cannot_settle_is_recorded_once_and_the_task_told() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        gh.track_remote(&bare);
        let task = task_with_two_branches(&s, &proj, &repo);
        let head = task_branch(&task);
        tend_proj(&s, &proj, TEST_NOW);
        let first = bare_ref(&bare, &format!("refs/heads/{head}"));

        // The base rewrites the file a job branch wrote.
        move_base_on_forge(&repo, &bare, "pulse_worker_a.txt");
        gh.set_mergeable(7, "CONFLICTING");
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains(&format!(
                "task pr proj: #7 conflicts with main at {}: pulse_worker_a.txt; main does not merge in cleanly; task {task} told",
                crate::recipe::short(&first, 12)
            ))),
            "{lines:?}"
        );
        assert_eq!(
            bare_ref(&bare, &format!("refs/heads/{head}")),
            first,
            "nothing pushed"
        );
        assert_eq!(events(&s, "land.pr-refreshed"), 0);
        assert_eq!(events(&s, "land.pr-conflict"), 1);
        let conflict = last_detail(&s, "land.pr-conflict");
        assert_eq!(conflict["kind"], "task");
        assert_eq!(conflict["number"], 7);
        assert_eq!(conflict["head"], head);
        assert_eq!(conflict["head_sha"], first);
        assert_eq!(conflict["base"], "main");
        assert_eq!(conflict["files"], json!(["pulse_worker_a.txt"]));
        assert_eq!(task_messages(&s, &task, "conflicts with main"), 1);
        assert_eq!(task_messages(&s, &task, "pulse_worker_a.txt"), 1);
        assert_eq!(events(&s, "land.pr"), 1, "the pull request's record stands");
        assert!(
            extra_worktrees(&repo).is_empty(),
            "the scratch worktree is gone"
        );
        assert_eq!(status(&s, &task), "in_progress");

        // Still conflicting at the same head: told once.
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(events(&s, "land.pr-conflict"), 1);
        assert_eq!(task_messages(&s, &task, "conflicts with main"), 1);
    }

    #[test]
    fn failing_checks_on_an_open_pull_request_are_told_once_per_head() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        gh.track_remote(&bare);
        let task = task_with_two_branches(&s, &proj, &repo);
        let head = task_branch(&task);
        tend_proj(&s, &proj, TEST_NOW);
        let first = bare_ref(&bare, &format!("refs/heads/{head}"));

        gh.set_checks(
            7,
            &json!([
                {"__typename": "CheckRun", "name": "build", "status": "COMPLETED", "conclusion": "FAILURE"},
                {"__typename": "CheckRun", "name": "lint", "status": "COMPLETED", "conclusion": "SUCCESS"},
                {"__typename": "StatusContext", "context": "ci/smoke", "state": "ERROR"}
            ]),
        );
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains(&format!(
                "task pr proj: #7 checks failing at {}: build, ci/smoke; task {task} told",
                crate::recipe::short(&first, 12)
            ))),
            "{lines:?}"
        );
        assert_eq!(events(&s, "land.pr-checks"), 1);
        let checks = last_detail(&s, "land.pr-checks");
        assert_eq!(checks["kind"], "task");
        assert_eq!(checks["number"], 7);
        assert_eq!(checks["head_sha"], first);
        assert_eq!(checks["checks"], json!(["build", "ci/smoke"]));
        assert_eq!(
            task_messages(&s, &task, "Checks are failing on pull request #7"),
            1
        );
        assert_eq!(task_messages(&s, &task, "build, ci/smoke"), 1);
        assert_eq!(status(&s, &task), "in_progress");

        // The same head again: nothing more.
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(events(&s, "land.pr-checks"), 1);

        // A new head (a fix pushed by hand) whose checks fail too: told
        // once more.
        run(&repo, &["git", "branch", "hand", &head]);
        commit_on(&repo, "hand", "hand.txt");
        run(
            &repo,
            &["git", "push", "-q", &bare, &format!("hand:{head}")],
        );
        run(&repo, &["git", "branch", "-D", "hand"]);
        let second = bare_ref(&bare, &format!("refs/heads/{head}"));
        assert_ne!(second, first);
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains("#7 checks failing")
                && l.contains(&crate::recipe::short(&second, 12))),
            "{lines:?}"
        );
        assert_eq!(events(&s, "land.pr-checks"), 2);
        assert_eq!(last_detail(&s, "land.pr-checks")["head_sha"], second);
        assert_eq!(
            task_messages(&s, &task, "Checks are failing on pull request #7"),
            2
        );

        // Checks that pass, or are still running, are no news.
        gh.set_checks(
            7,
            &json!([{"__typename": "CheckRun", "name": "build", "status": "IN_PROGRESS", "conclusion": null}]),
        );
        assert!(tend_proj(&s, &proj, TEST_NOW).is_empty());
        assert_eq!(events(&s, "land.pr-checks"), 2);
    }

    #[test]
    fn a_forge_that_cannot_be_read_changes_nothing() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        gh.track_remote(&bare);
        let task = task_with_two_branches(&s, &proj, &repo);
        let head = task_branch(&task);
        tend_proj(&s, &proj, TEST_NOW);
        let first = bare_ref(&bare, &format!("refs/heads/{head}"));
        let pr_events = "SELECT COUNT(*) AS n FROM events WHERE action LIKE 'land.pr%' OR action = 'land.merged'";
        let before = count(&s, pr_events, &[]);

        // Everything the forge would say, said while gh is down: nothing
        // is heard, nothing recorded, nothing pushed. (The checkout's main
        // still follows the upstream: that is git, not gh.)
        move_base_on_forge(&repo, &bare, "elsewhere.txt");
        gh.set_mergeable(7, "CONFLICTING");
        gh.set_checks(
            7,
            &json!([{"__typename": "CheckRun", "name": "build", "status": "COMPLETED", "conclusion": "FAILURE"}]),
        );
        gh.set_state(7, "MERGED");
        gh.fail_reads("HTTP 502: Bad Gateway");
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(
            lines
                .iter()
                .filter(|l| !l.contains("fast-forwarded"))
                .collect::<Vec<_>>(),
            [
                "proj: gh pr list failed: HTTP 502: Bad Gateway",
                "proj: pull request #7: gh pr view failed: HTTP 502: Bad Gateway"
            ]
        );
        assert_eq!(
            count(&s, pr_events, &[]) - before,
            0,
            "no pull request event while the forge cannot be read"
        );
        assert_eq!(bare_ref(&bare, &format!("refs/heads/{head}")), first);
        assert_eq!(status(&s, &task), "in_progress");
        assert_eq!(task_messages(&s, &task, "pull request"), 0);
        assert_eq!(
            pulse_tasks::delivery_for(&s, &task).unwrap().state,
            "pr_open"
        );

        // Heard once gh answers again.
        gh.heal();
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains("#7 merged; task done")),
            "{lines:?}"
        );
        assert_eq!(status(&s, &task), "done");
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

    /// A conversation row at `id`, as the hub would have made it, so the
    /// delivery pass finds something to tell.
    fn seed_conversation(s: &pulse_store::Store, id: &str, repo: &str) {
        s.chat_create(&pulse_store::Conversation {
            id: id.into(),
            title: "a chat".into(),
            project: "proj".into(),
            cwd: repo.into(),
            branch: format!("pulse/chat-{id}"),
            status: "idle".into(),
            ..Default::default()
        })
        .unwrap();
    }

    #[test]
    fn a_conversation_branch_is_proposed_once_its_last_commit_is_ten_minutes_old() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        seed_conversation(&s, "abc123", &repo);
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
                && l.contains("pulse/chat-abc123 kept: a worktree has it checked out")
                && l.contains("conversation abc123 finished")),
            "{lines:?}"
        );
        let merged = last_detail(&s, "land.merged");
        assert_eq!(merged["kind"], "proposed");
        assert_eq!(merged["conversation"], "abc123");
        assert_eq!(ref_of(&repo, "refs/heads/pulse/chat-abc123"), tip2);
        assert_eq!(ref_of(&repo, "refs/heads/main"), tip2);
        // The conversation is finished on the bus, where the hub's wake
        // loop ends its session after any turn in flight; the daemon does
        // not reach the session itself.
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM events WHERE action = ? AND subject = 'abc123'",
                &[&pulse_harness::conversation::FINISH_ACTION],
            ),
            1
        );
        let finish = last_detail(&s, pulse_harness::conversation::FINISH_ACTION);
        assert_eq!(finish["number"], 7);
        assert_eq!(
            finish["text"],
            "pull request #7 (https://github.com/o/r/pull/7) merged into main; this session is finished"
        );
        // Merged by patch: not proposed again, not finished again.
        assert!(tend_proj(&s, &proj, later).is_empty());
        assert_eq!(events(&s, pulse_harness::conversation::FINISH_ACTION), 1);
        run(&repo, &["git", "worktree", "remove", "--force", &wt]);
    }

    #[test]
    fn a_conversation_hears_of_its_pull_request_conflicting_and_of_it_closing_unmerged() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        gh.track_remote(&bare);
        seed_conversation(&s, "abc123", &repo);
        run(&repo, &["git", "branch", "pulse/chat-abc123", "main"]);
        let tip = commit_dated(
            &repo,
            "pulse/chat-abc123",
            "pulse_chat.txt",
            "2026-09-06T09:40:00Z",
        );
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(
            lines,
            ["proposed pr proj conversation abc123: #7 opened https://github.com/o/r/pull/7"]
        );

        // The base rewrites the chat's file: the conversation's agent is
        // woken with the conflict, once for this head.
        move_base_on_forge(&repo, &bare, "pulse_chat.txt");
        gh.set_mergeable(7, "CONFLICTING");
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(
                |l| l.contains("proposed pr proj: #7 conflicts with main at")
                    && l.contains(
                        "pulse_chat.txt; main does not merge in cleanly; conversation abc123 woken"
                    )
            ),
            "{lines:?}"
        );
        assert_eq!(events(&s, "land.pr-conflict"), 1);
        assert_eq!(bare_ref(&bare, "refs/heads/pulse/chat-abc123"), tip);
        let wakes = "SELECT COUNT(*) AS n FROM events WHERE action = ? AND subject = 'abc123'";
        let wake_action = pulse_harness::conversation::WAKE_ACTION;
        assert_eq!(count(&s, wakes, &[&wake_action]), 1);
        let wake = last_detail(&s, wake_action);
        let text = wake["text"].as_str().unwrap_or_default();
        assert!(
            text.starts_with(
                "Pull request #7 (https://github.com/o/r/pull/7) conflicts with main at"
            ) && text.contains("pulse_chat.txt"),
            "{text}"
        );
        assert!(tend_proj(&s, &proj, TEST_NOW).is_empty());
        assert_eq!(count(&s, wakes, &[&wake_action]), 1);

        // Closed without a merge: a note in the transcript, no turn, and
        // the conversation stands as it was.
        gh.set_state(7, "CLOSED");
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(
            lines,
            [
                "proposed pr proj: #7 closed unmerged; the branch stays on the remote; conversation abc123 told"
            ]
        );
        assert_eq!(events(&s, "land.pr-closed"), 1);
        let notes = s.chat_messages("abc123", 0).unwrap();
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert_eq!(notes[0].role, "note");
        assert_eq!(
            notes[0].content,
            "Pull request #7 (https://github.com/o/r/pull/7) was closed without merging. Nothing is marked done; the branch stays on the remote. Write here to carry on."
        );
        assert_eq!(notes[0].extra["pr"]["number"], 7);
        assert_eq!(s.chat_get("abc123").unwrap().unwrap().status, "idle");
        assert_eq!(count(&s, wakes, &[&wake_action]), 1);
        assert_eq!(events(&s, pulse_harness::conversation::FINISH_ACTION), 0);
        assert_eq!(bare_ref(&bare, "refs/heads/pulse/chat-abc123"), tip);
        assert_eq!(events(&s, "land.merged"), 0);
        // Told once: the closing is settled.
        assert!(tend_proj(&s, &proj, TEST_NOW).is_empty());
        assert_eq!(s.chat_messages("abc123", 0).unwrap().len(), 1);
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

    /// The worker marks its job done itself, naming the branch the runner
    /// creates only after the run's verification: while that run is still
    /// running the task is in flight, not held on a branch that is not
    /// there yet.
    #[test]
    fn a_task_whose_job_is_still_running_waits_for_the_run_instead_of_being_held() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, _bare) = push_fixture(&s);
        let task = task_with_two_branches(&s, &proj, &repo);
        let b = job_row_by_branch(&s, "pulse/worker/b");
        let tip = ref_of(&repo, "refs/heads/pulse/worker/b");
        run(&repo, &["git", "branch", "-D", "pulse/worker/b"]);
        exec1(
            &s,
            "INSERT INTO runs (id, job_id, agent, project, started_at, status) VALUES ('run-b', ?, 'worker', 'proj', ?, 'running')",
            &[&b, &TEST_NOW],
        );

        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(events(&s, "land.pr-held"), 0, "nothing held");
        assert!(gh.calls().is_empty());

        // The run ends with the branch in place: one pull request, both jobs.
        run(&repo, &["git", "branch", "pulse/worker/b", &tip]);
        exec1(
            &s,
            "UPDATE runs SET status = 'done', finished_at = ? WHERE id = 'run-b'",
            &[&TEST_NOW],
        );
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines
                .iter()
                .any(|l| l.contains(&format!("{task}: #7 opened"))),
            "{lines:?}"
        );
        assert_eq!(
            last_detail(&s, "land.pr")["jobs"].as_array().map(Vec::len),
            Some(2)
        );
    }

    /// A job branch that conflicts with the default branch goes to a rebase
    /// job on the same task, once; the task's pull request opens from the
    /// rebased branch, and a second conflict is held for the owner.
    #[test]
    fn a_conflicting_task_branch_is_handed_to_a_rebase_job_once_then_held() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, _bare) = push_fixture(&s);
        let task = task_with_two_branches(&s, &proj, &repo);
        let b = job_row_by_branch(&s, "pulse/worker/b");
        // main changes the file b's commit adds, so b no longer merges.
        commit_on(&repo, "main", "pulse_worker_b.txt");
        run(&repo, &["git", "push", "-q", "origin", "main"]);
        let b_tip = ref_of(&repo, "refs/heads/pulse/worker/b");

        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains(&format!(
                "task pr proj {task}: pulse/worker/b (job {b}) does not merge cleanly onto main"
            )) && l.contains("conflicts in pulse_worker_b.txt")
                && l.contains("rebases it")),
            "{lines:?}"
        );
        assert!(
            gh.calls().is_empty(),
            "no pull request yet: {:?}",
            gh.calls()
        );
        assert_eq!(events(&s, "land.pr-held"), 0);
        let rebase = last_detail(&s, "land.rebase");
        assert_eq!(rebase["task"], task);
        assert_eq!(rebase["job"], b);
        assert_eq!(rebase["sha"], b_tip);
        assert_eq!(rebase["files"], json!(["pulse_worker_b.txt"]));
        let follow_up = rebase["follow_up"].as_str().unwrap().to_string();
        let fu = job_row(&s, &follow_up);
        assert_eq!(fu.str("status"), "queued");
        assert_eq!(fu.str("source"), format!("task:{task}"));
        assert!(
            fu.str("title")
                .starts_with("Rebase pulse/worker/b onto main:"),
            "{}",
            fu.str("title")
        );
        assert!(
            fu.str("brief")
                .contains("conflict with what landed on `main`"),
            "{}",
            fu.str("brief")
        );
        assert_eq!(
            job_row(&s, &b).str("branch"),
            "",
            "b's branch is the rebase job's now"
        );
        assert!(
            job_row(&s, &b)
                .str("result")
                .contains(&format!("job {follow_up} rebases it"))
        );
        assert_eq!(status(&s, &task), "in_progress");
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM messages WHERE task_id = ? AND body LIKE '%rebases it%'",
                &[&task]
            ),
            1
        );
        // The rebase job is queued: the task waits for it.
        assert!(tend_proj(&s, &proj, TEST_NOW).is_empty());

        // The rebase job finishes on a branch cut from main: one pull
        // request from a and the rebased work.
        run(&repo, &["git", "branch", "pulse/worker/rebased", "main"]);
        let c_tip = commit_on(&repo, "pulse/worker/rebased", "rebased.txt");
        exec1(
            &s,
            "UPDATE jobs SET status = 'done', finished_at = ?, branch = 'pulse/worker/rebased' WHERE id = ?",
            &[&TEST_NOW, &follow_up],
        );
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines
                .iter()
                .any(|l| l.contains(&format!("{task}: #7 opened"))),
            "{lines:?}"
        );
        let pr = last_detail(&s, "land.pr");
        let mut jobs = jobs_of(&pr);
        jobs.sort();
        let mut want = vec![job_row_by_branch(&s, "pulse/worker/a"), follow_up.clone()];
        want.sort();
        assert_eq!(jobs, want, "a and the rebased work, not b");
        run(
            &repo,
            &[
                "git",
                "merge-base",
                "--is-ancestor",
                &c_tip,
                pr["to"].as_str().unwrap(),
            ],
        );

        // A later job of the task conflicts too: the task's one rebase is
        // spent, so this one is held for the owner, with the conflict named.
        let d = push_branch(&s, &proj, &repo, "pulse/worker/d");
        exec1(
            &s,
            "UPDATE jobs SET source = ? WHERE id = ?",
            &[&format!("task:{task}"), &d.str("id")],
        );
        commit_on(&repo, "main", "pulse_worker_d.txt");
        run(&repo, &["git", "push", "-q", "origin", "main"]);
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains("held:")
                && l.contains("conflicts in pulse_worker_d.txt")
                && l.contains(&format!(
                    "job {follow_up} already rebased the task's work once"
                ))),
            "{lines:?}"
        );
        let held = last_detail(&s, "land.pr-held");
        assert_eq!(held["task"], task);
        assert_eq!(
            job_row(&s, &d.str("id")).str("branch"),
            "pulse/worker/d",
            "kept"
        );
        assert_eq!(events(&s, "land.rebase"), 1);
        assert!(
            tend_proj(&s, &proj, TEST_NOW).is_empty(),
            "held once per sha"
        );
        assert_eq!(events(&s, "land.pr-held"), 1);
    }

    fn job_row_by_branch(s: &pulse_store::Store, branch: &str) -> String {
        crate::util::must_one(
            &s.conn().unwrap(),
            "SELECT id FROM jobs WHERE branch = ?",
            &[&branch],
        )
        .str("id")
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

    /// A task delivered while #7 is open: `pulse/task-<id>` is built on #7's
    /// head, its pull request targets #7's branch and records the stack.
    fn stacked_task(
        s: &pulse_store::Store,
        proj: &Row,
        repo: &str,
        bare: &str,
        gh: &FakeGh,
    ) -> (String, String, String) {
        let first = task_with_two_branches(s, proj, repo);
        let lines = tend_proj(s, proj, TEST_NOW);
        assert!(
            lines
                .iter()
                .any(|l| l.contains(&format!("{first}: #7 opened https"))),
            "{lines:?}"
        );
        assert_eq!(gh.base_of(7), "main");
        let first_head = bare_ref(bare, &format!("refs/heads/{}", task_branch(&first)));

        let second = task_with_branches(s, proj, repo, "Add the other thing", &["pulse/worker/c"]);
        let lines = tend_proj(s, proj, TEST_NOW);
        assert!(
            lines
                .iter()
                .any(|l| l.contains(&format!("{second}: #8 opened on #7 https"))),
            "{lines:?}"
        );
        let second_head = bare_ref(bare, &format!("refs/heads/{}", task_branch(&second)));
        run(
            repo,
            &[
                "git",
                "merge-base",
                "--is-ancestor",
                &first_head,
                &second_head,
            ],
        );
        assert_eq!(gh.base_of(8), task_branch(&first));
        let pr = last_detail(s, "land.pr");
        assert_eq!(pr["number"], 8);
        assert_eq!(pr["task"], second);
        assert_eq!(pr["base"], task_branch(&first));
        assert_eq!(pr["stack_on"], 7);
        assert_eq!(pr["from"], first_head, "counted from the base, not main");
        assert!(tend_proj(s, proj, TEST_NOW).is_empty());
        (first, second, first_head)
    }

    #[test]
    fn a_second_task_pull_request_stacks_on_the_open_one_and_moves_onto_main_when_it_merges() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let (first, second, _) = stacked_task(&s, &proj, &repo, &bare, &gh);
        let first_branch = task_branch(&first);

        // The bottom merges: #8 is moved onto main on the forge and on
        // record, and the reviewer takes it next.
        merge_on_forge(&gh, &bare, 7);
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains("#7 merged; task done")),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l == &format!("task pr proj: #8 retargeted from {first_branch} to main")),
            "{lines:?}"
        );
        assert!(
            gh.calls().contains(&"pr edit 8".to_string()),
            "{:?}",
            gh.calls()
        );
        assert_eq!(gh.base_of(8), "main");
        let pr = last_detail(&s, "land.pr");
        assert_eq!(pr["number"], 8);
        assert_eq!(pr["task"], second);
        assert_eq!(pr["base"], "main");
        assert!(pr.get("stack_on").is_none(), "{pr}");
        assert_eq!(pr["retargeted_from"], first_branch);
        assert_eq!(pr["opened"], false);
        assert_eq!(status(&s, &first), "done");
        assert_eq!(status(&s, &second), "in_progress");
        assert!(tend_proj(&s, &proj, TEST_NOW).is_empty());

        let lines = crate::review::tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains("#8 queued for review")),
            "{lines:?}"
        );

        merge_on_forge(&gh, &bare, 8);
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains("#8 merged; task done")),
            "{lines:?}"
        );
        assert_eq!(status(&s, &second), "done");
    }

    #[test]
    fn a_task_stacked_on_a_pull_request_closed_unmerged_is_told_once() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let (first, second, _) = stacked_task(&s, &proj, &repo, &bare, &gh);
        let told = |task: &str| {
            count(
                &s,
                "SELECT COUNT(*) AS n FROM messages WHERE task_id = ? AND body LIKE '%was stacked on #7%'",
                &[&task],
            )
        };

        gh.set_state(7, "CLOSED");
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines
                .iter()
                .any(|l| l.contains("#7 closed unmerged; task back in progress")),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l
                == &format!(
                    "task pr proj: #8 retargeted from {} to main; it carries #7's commits",
                    task_branch(&first)
                )),
            "{lines:?}"
        );
        assert_eq!(gh.base_of(8), "main");
        let pr = last_detail(&s, "land.pr");
        assert_eq!(pr["number"], 8);
        assert_eq!(pr["base"], "main");
        assert!(pr.get("stack_on").is_none(), "{pr}");
        assert_eq!(told(&second), 1);
        assert_eq!(told(&first), 0);
        assert_eq!(
            events(&s, "land.pr-unstacked"),
            0,
            "a task is told, not an event"
        );

        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(told(&second), 1);
    }

    #[test]
    fn a_job_pull_request_takes_in_the_stack_top_and_is_unstacked_when_it_closes() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let first = task_with_two_branches(&s, &proj, &repo);
        tend_proj(&s, &proj, TEST_NOW);
        let first_head = bare_ref(&bare, &format!("refs/heads/{}", task_branch(&first)));
        let job = push_branch(&s, &proj, &repo, "pulse/sec/x");
        let id = job.str("id");
        exec1(
            &s,
            "UPDATE jobs SET kind = 'security', source = 'security:proj:2026-09-06' WHERE id = ?",
            &[&id],
        );
        let tip = ref_of(&repo, "refs/heads/pulse/sec/x");

        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(
            lines,
            [format!(
                "job pr proj job {id}: #8 opened on #7 https://github.com/o/r/pull/8"
            )]
        );
        let pushed = bare_ref(&bare, "refs/heads/pulse/sec/x");
        assert_ne!(pushed, tip, "the top was merged in");
        assert_eq!(ref_of(&repo, "refs/heads/pulse/sec/x"), pushed);
        run(
            &repo,
            &["git", "merge-base", "--is-ancestor", &first_head, &pushed],
        );
        run(
            &repo,
            &["git", "merge-base", "--is-ancestor", &tip, &pushed],
        );
        assert_eq!(gh.base_of(8), task_branch(&first));
        let pr = last_detail(&s, "land.pr");
        assert_eq!(pr["kind"], "job");
        assert_eq!(pr["base"], task_branch(&first));
        assert_eq!(pr["stack_on"], 7);
        assert_eq!(pr["to"], pushed);
        assert_eq!(pr["from"], first_head);
        assert!(
            crate::util::git_timeout(&repo, Duration::from_secs(10), &["worktree", "list"])
                .stdout
                .lines()
                .count()
                == 1,
            "the scratch worktree is gone"
        );
        assert!(tend_proj(&s, &proj, TEST_NOW).is_empty());

        // The bottom is closed unmerged: #8 moves onto main carrying #7's
        // commits, which `land.pr-unstacked` says, once.
        gh.set_state(7, "CLOSED");
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l
                == &format!(
                    "job pr proj: #8 retargeted from {} to main; it carries #7's commits",
                    task_branch(&first)
                )),
            "{lines:?}"
        );
        assert_eq!(gh.base_of(8), "main");
        let un = last_detail(&s, "land.pr-unstacked");
        assert_eq!(un["number"], 8);
        assert_eq!(un["closed"], 7);
        assert_eq!(un["base"], "main");
        assert!(
            un["note"]
                .as_str()
                .unwrap_or_default()
                .contains("#8 was stacked on #7, which was closed unmerged"),
            "{un}"
        );
        assert_eq!(last_detail(&s, "land.pr")["base"], "main");
        assert!(tend_proj(&s, &proj, TEST_NOW).is_empty());
        assert_eq!(events(&s, "land.pr-unstacked"), 1);
    }

    #[test]
    fn a_conversation_pull_request_neither_stacks_nor_is_stacked_on() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let first = task_with_two_branches(&s, &proj, &repo);
        tend_proj(&s, &proj, TEST_NOW);
        run(&repo, &["git", "branch", "pulse/chat-abc123", "main"]);
        let tip = commit_dated(
            &repo,
            "pulse/chat-abc123",
            "chat.txt",
            "2026-09-06T09:58:00Z",
        );
        let later = "2026-09-06T10:08:30Z";
        let lines = tend_proj(&s, &proj, later);
        assert_eq!(
            lines,
            ["proposed pr proj conversation abc123: #8 opened https://github.com/o/r/pull/8"]
        );
        assert_eq!(gh.base_of(8), "main");
        assert_eq!(
            bare_ref(&bare, "refs/heads/pulse/chat-abc123"),
            tip,
            "nothing merged in"
        );
        let pr = last_detail(&s, "land.pr");
        assert_eq!(pr["base"], "main");
        assert!(pr.get("stack_on").is_none(), "{pr}");

        // The next pull request stacks on the task's, not the conversation's.
        let job = push_branch(&s, &proj, &repo, "pulse/sec/x");
        let id = job.str("id");
        exec1(
            &s,
            "UPDATE jobs SET kind = 'security', source = 'security:proj:2026-09-06' WHERE id = ?",
            &[&id],
        );
        let lines = tend_proj(&s, &proj, later);
        assert_eq!(
            lines,
            [format!(
                "job pr proj job {id}: #9 opened on #7 https://github.com/o/r/pull/9"
            )]
        );
        assert_eq!(gh.base_of(9), task_branch(&first));
        assert_eq!(last_detail(&s, "land.pr")["stack_on"], 7);
    }

    #[test]
    fn a_task_branch_conflicting_with_the_stack_top_goes_to_a_rebase_job_naming_it() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, _bare) = push_fixture(&s);
        let first = task_with_two_branches(&s, &proj, &repo);
        tend_proj(&s, &proj, TEST_NOW);
        let second =
            task_with_branches(&s, &proj, &repo, "Add the other thing", &["pulse/worker/c"]);
        // c adds the file a's commit adds, with other content: it merges
        // onto main, not onto #7's head.
        commit_on(&repo, "pulse/worker/c", "pulse_worker_a.txt");
        let c = job_row_by_branch(&s, "pulse/worker/c");

        let lines = tend_proj(&s, &proj, TEST_NOW);
        let onto = format!("{} (pull request #7)", task_branch(&first));
        assert!(
            lines.iter().any(|l| l.contains(&format!(
                "task pr proj {second}: pulse/worker/c (job {c}) does not merge cleanly onto {onto}"
            )) && l.contains("rebases it")),
            "{lines:?}"
        );
        assert_eq!(
            gh.calls()
                .iter()
                .filter(|c| c.starts_with("pr create"))
                .count(),
            1
        );
        let rebase = last_detail(&s, "land.rebase");
        assert_eq!(rebase["task"], second);
        assert_eq!(rebase["base"], task_branch(&first));
        let fu = job_row(&s, rebase["follow_up"].as_str().unwrap());
        assert_eq!(
            fu.str("title"),
            format!(
                "Rebase pulse/worker/c onto {}: Add the other thing",
                task_branch(&first)
            )
        );
        assert!(
            fu.str("brief").contains(
                "the head of open pull request #7, which the task's pull request stacks on"
            ),
            "{}",
            fu.str("brief")
        );
    }
}
