//! Every finished branch of a project delivered as a pull request on its
//! upstream, and every pull request Hive opened followed until the forge
//! settles it. Nothing merges into the local default branch on this
//! machine: it only follows the upstream (`forge::sync_default_from_upstream`).
//!
//! What is delivered, each pass (`tend`):
//! - a task whose jobs are all finished: the jobs' branches merged onto the
//!   default branch as `hive/task-<id>`, one pull request (`land.pr`,
//!   kind `task`); a branch that conflicts with the default branch goes to
//!   a rebase job on the task once (`land.rebase`) before the task is held,
//!   and branches of the task that conflict only with each other go to one
//!   join job (`land.rebase` with `join` true); a branch whose commits
//!   another side of the build carries already is not merged again
//!   (`carried_elsewhere`);
//! - every other done job that still has a branch, one pull request each
//!   from the branch as the agent left it (kind `job`) - 92% of the 260
//!   landings of the week before this (suggestion, security, lead, schedule,
//!   research...) were not task work and the per-task flow alone would have
//!   stranded them;
//! - a `pr-fix:<slug>:<n>` job: its branch pushed onto pull request n's head
//!   (`land.pr`, `opened` false);
//! - a conversation branch (`hive/chat-*`) with commits the default branch
//!   has not got, once its last commit is ten minutes old (kind `proposed`);
//! - a kept experiment, from the experiment lane (`experiment.rs`, kind
//!   `experiment`), and a branch named by hand (`hive land --branch`, kind
//!   `proposed`).
//!
//! The pull requests of a project form one stack (`stack_top`): a new pull
//! request is built on the head of the newest open one and targets it
//! (`base`, `stack_on` in its `land.pr`), so two open pull requests never
//! hold changes to the same lines that only meet at merge time. The stack
//! is plain git branches and explicit bases, nothing of the forge's: when
//! the pull request below merges, the one above is moved onto its base
//! (`retarget_stacked`); when it is closed unmerged, the one above is
//! rebuilt on that base from its own commits alone, the rejected ones
//! dropped, and then moved (`rebuild_stacked`, `land.pr-rebuilt`; a
//! rebuild that conflicts holds the pull request where it is,
//! `land.pr-held`). The reviewer takes the stack bottom up (`review.rs`).
//! A conversation's pull request (`hive/chat-<id>`) stays out of the
//! stack: the hub keeps its branch checked out.
//!
//! The same pass reads the state of every open pull request in one `gh pr
//! list` per repository. Merged: `land.merged`, the delivered jobs lose
//! their branch, a task is done, the local branches go and the default
//! branch is fast-forwarded from the upstream, and a conversation's session
//! (`hive/chat-<id>`) is finished (`chat.finish`, which the hub's wake loop
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

pub(crate) mod carry;
mod experiments;
mod merge_check;

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
    format!("hive/task-{task}")
}

/// One pass over the projects: settle the pull requests the forge merged or
/// closed, deliver the work now owed, bring every default branch up to its
/// upstream. Returns the log lines.
pub(crate) fn tend(s: &hive_store::Store, projects: &[Row], now: &str) -> Vec<String> {
    let mut lines = retire_holds(s);
    for p in projects {
        lines.extend(tend_project(s, p, now));
    }
    lines
}

/// The part of [`tend`] that is not one project's: the old landing holds.
pub(crate) fn retire_holds(s: &hive_store::Store) -> Vec<String> {
    retire_landing_holds(s)
}

/// [`tend`] for one project; the maintenance pass budgets it project by
/// project (stage_budget.rs).
pub(crate) fn tend_project(s: &hive_store::Store, p: &Row, now: &str) -> Vec<String> {
    let mut lines = Vec::new();
    if p.str("path").is_empty() {
        return lines;
    }
    lines.extend(poll_open(s, p, now));
    lines.extend(backfill_merge_evidence(s, p, now));
    lines.extend(deliver(s, p, now));
    if let Some(line) = crate::forge::sync_default_from_upstream(s, p, ACTOR, now, false) {
        lines.push(line);
    }
    lines
}

/// The meta key the local land lane kept its per-project holds under ("1
/// pending landing job; deferred: main moved while the merge was verified").
const LANDING_HOLDS_KEY: &str = "landing";

/// Clears the holds the local land lane left in the store. Delivery goes
/// through pull requests since 4271f214 and nothing writes or lifts those
/// notes any more; the console kept showing one for amistio-template from
/// 2026-11-04 as "Can't merge into amistio-template (older note)" a day
/// after the last landing. Says what it cleared, once.
fn retire_landing_holds(s: &hive_store::Store) -> Vec<String> {
    let Ok((raw, exists)) = s.meta(LANDING_HOLDS_KEY) else {
        return Vec::new();
    };
    if !exists {
        return Vec::new();
    }
    let Ok(conn) = s.conn() else {
        return Vec::new();
    };
    crate::util::exec1(
        &conn,
        "DELETE FROM meta WHERE key = ?",
        &[&LANDING_HOLDS_KEY],
    );
    let projects = serde_json::from_str::<serde_json::Map<String, Value>>(&raw)
        .map(|m| m.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let which = if projects.is_empty() {
        String::new()
    } else {
        format!(" for {}", projects.join(", "))
    };
    vec![format!(
        "landing: cleared the hold note{which} the retired land lane left behind; work is delivered as pull requests"
    )]
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
    /// Why the store could not say whether the task has a pull request
    /// open: the task is held, not taken for one with none (`open_pr`).
    unread: Option<String>,
}

fn ready_tasks(s: &hive_store::Store, slug: &str) -> Vec<ReadyTask> {
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
        let (exact, like) = hive_store::task_source_params(&id);
        let jobs = crate::util::query_rows(
            &conn,
            &format!(
                "SELECT id, status, branch,
                    EXISTS (SELECT 1 FROM runs r WHERE r.job_id = jobs.id AND r.status = 'running') AS running
                 FROM jobs WHERE project = ? AND {} ORDER BY created_at, id",
                hive_store::task_source_clause("source")
            ),
            &[&slug, &exact, &like],
        )
        .unwrap_or_default();
        // A worker marks its own job done (`hive job finish ... --branch`)
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
        let (open, unread) = match open_pr(&conn, slug, &id) {
            Ok(open) => (open, None),
            Err(why) => (None, Some(why)),
        };
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
            unread,
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
            .strip_prefix("hive/chat-")
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

fn ready_work(s: &hive_store::Store, project: &Row, now: &str) -> Ready {
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

/// The `hive/chat-*` branches with commits the default branch has not
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
            "refs/heads/hive/chat-*",
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

/// The task's pull request still open as far as Hive knows: the newest
/// `land.pr` of the task with no `land.merged` or `land.pr-closed` after it,
/// as its event detail. `Err` when the store could not be read: that is no
/// answer, and above all not "no pull request" - work with an open pull
/// request taken for never delivered is pushed with a plain force onto the
/// pull request's head.
pub(crate) fn open_pr(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    task: &str,
) -> Result<Option<Value>, String> {
    Ok(last_event(q, slug, "task", task, false)?
        .and_then(|(action, detail)| (action == "land.pr").then_some(detail)))
}

/// The job's pull request still open as far as Hive knows, as its `land.pr`
/// detail: the experiment lane leaves such a job alone. `Err` when the
/// store could not be read, as for [`open_pr`].
pub(crate) fn open_pr_for_job(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    job: &str,
) -> Result<Option<Value>, String> {
    Ok(last_event(q, slug, "job", job, false)?
        .and_then(|(action, detail)| (action == "land.pr").then_some(detail)))
}

/// The newest delivery event about one piece of work - the task, job,
/// experiment or head branch named by `field` - as (action, detail). A job
/// is also found in the `jobs` list of a pull request that carried it (a
/// task's or a fix's). With `holds` the `land.pr-held` notes count too.
/// `Err` (opening with `STORE_UNREAD`) when the store could not be read:
/// a caller must not take that for a work never delivered.
fn last_event(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    field: &str,
    value: &str,
    holds: bool,
) -> Result<Option<(String, Value)>, String> {
    Ok(last_event_at(q, slug, field, value, holds)?.map(|(action, detail, _)| (action, detail)))
}

/// [`last_event`] with the time the event was written.
fn last_event_at(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    field: &str,
    value: &str,
    holds: bool,
) -> Result<Option<(String, Value, String)>, String> {
    Ok(last_events(q, slug, field, value, holds, 1)?
        .into_iter()
        .next())
}

/// The newest delivery events about one piece of work, newest first, at
/// most `limit` of them, each as [`last_event_at`] gives the newest.
fn last_events(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    field: &str,
    value: &str,
    holds: bool,
    limit: usize,
) -> Result<Vec<(String, Value, String)>, String> {
    let Some(about) = about_work(field) else {
        return Ok(Vec::new());
    };
    let actions = if holds {
        "'land.pr', 'land.pr-held', 'land.merged', 'land.pr-closed'"
    } else {
        DELIVERED
    };
    delivery_rows(
        q,
        &format!(
            "SELECT action, detail, at FROM events WHERE subject = ?1 AND action IN ({actions})
             AND json_valid(detail) AND {about}
             ORDER BY seq DESC LIMIT {limit}"
        ),
        slug,
        value,
    )
}

/// The events that say a work went to the forge or came back from it.
const DELIVERED: &str = "'land.pr', 'land.merged', 'land.pr-closed'";

/// The SQL condition that an event's detail is about the work `?2` names
/// in `field`; a job is also found in the `jobs` list of a pull request
/// that carried it, and in a pull request whose head is the job's branch:
/// one proposed by hand (`hive land --branch`) before such a pull request
/// named its job. Without that the job path answered `Open` for a branch
/// already on a pull request, merged the stack's top into it and pushed
/// it - the head of the pull request by hand and the base of the one above,
/// which the forge then took for merged.
fn about_work(field: &str) -> Option<String> {
    let path = match field {
        "task" => "$.task",
        "job" => "$.job",
        "experiment" => "$.experiment",
        "head" => "$.head",
        _ => return None,
    };
    let in_jobs = if field == "job" {
        " OR EXISTS (SELECT 1 FROM json_each(detail, '$.jobs') WHERE value = ?2)
          OR json_extract(detail, '$.head') = (SELECT j.branch FROM jobs j WHERE j.id = ?2 AND j.branch <> '')"
    } else {
        ""
    };
    Some(format!("(json_extract(detail, '{path}') = ?2{in_jobs})"))
}

/// Every `land.pr-held` about one piece of work written after its newest
/// `land.pr`, `land.merged` or `land.pr-closed` (by `seq`), newest first,
/// with no row limit: a safety decision read from these holds (`tried_since`)
/// must not lapse because more of them were written than a window holds.
/// A store that cannot be read is an error, not "no holds": the decision
/// fails closed.
fn holds_since_delivery(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    field: &str,
    value: &str,
) -> Result<Vec<Value>, String> {
    let Some(about) = about_work(field) else {
        return Ok(Vec::new());
    };
    let rows = crate::util::query_rows(
        q,
        &format!(
            "SELECT detail FROM events WHERE subject = ?1 AND action = 'land.pr-held'
             AND json_valid(detail) AND {about}
             AND seq > COALESCE((SELECT MAX(seq) FROM events WHERE subject = ?1
                 AND action IN ({DELIVERED}) AND json_valid(detail) AND {about}), 0)
             ORDER BY seq DESC"
        ),
        &[&slug, &value],
    )
    .map_err(|e| format!("{STORE_UNREAD}the holds of {value} could not be read: {e}"))?;
    Ok(rows
        .iter()
        .map(|row| serde_json::from_str(&row.str("detail")).unwrap_or(Value::Null))
        .collect())
}

#[cfg(test)]
thread_local! {
    /// Tests: every read through `delivery_rows` fails while set and the
    /// store's other reads go on, as when one read fails and the next one
    /// succeeds.
    static DELIVERIES_UNREAD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The rows of a delivery-event query about `value`. A failed read is an
/// error naming the work, never an empty answer: no row reads as "never
/// delivered", and the push that follows such an answer forces.
fn delivery_rows(
    q: &(impl crate::util::Querier + ?Sized),
    sql: &str,
    slug: &str,
    value: &str,
) -> Result<Vec<(String, Value, String)>, String> {
    #[cfg(test)]
    if DELIVERIES_UNREAD.get() {
        return Err(format!(
            "{STORE_UNREAD}the deliveries of {value} could not be read: disk I/O error"
        ));
    }
    Ok(crate::util::query_rows(q, sql, &[&slug, &value])
        .map_err(|e| format!("{STORE_UNREAD}the deliveries of {value} could not be read: {e}"))?
        .iter()
        .map(|row| {
            (
                row.str("action"),
                serde_json::from_str(&row.str("detail")).unwrap_or(Value::Null),
                row.str("at"),
            )
        })
        .collect())
}

/// The newest `land.pr` of pull request `number`, as its detail; `Err`
/// when the store could not be read.
fn pr_event(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    number: i64,
) -> Result<Option<Value>, String> {
    let rows = crate::util::query_rows(
        q,
        "SELECT detail FROM events WHERE subject = ? AND action = 'land.pr' AND json_valid(detail)
         AND json_extract(detail, '$.number') = ? ORDER BY seq DESC LIMIT 1",
        &[&slug, &number],
    )
    .map_err(|e| format!("{STORE_UNREAD}pull request #{number} could not be read: {e}"))?;
    Ok(rows
        .first()
        .map(|r| serde_json::from_str(&r.str("detail")).unwrap_or(Value::Null)))
}

fn field<'a>(v: &'a Value, name: &str) -> &'a str {
    v.get(name).and_then(Value::as_str).unwrap_or_default()
}

fn number_of(v: &Value) -> i64 {
    v.get("number").and_then(Value::as_i64).unwrap_or(0)
}

/// The task's last pull request was closed unmerged and no job of the task
/// has been queued since: the lead plans again before the task can close.
pub(crate) fn closed_awaiting_work(s: &hive_store::Store, slug: &str, task: &str) -> bool {
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
    let (exact, like) = hive_store::task_source_params(task);
    let at = last.str("at");
    crate::util::query_rows(
        &conn,
        &format!(
            "SELECT id FROM jobs WHERE project = ? AND created_at > ? AND {} LIMIT 1",
            hive_store::task_source_clause("source")
        ),
        &[&slug, &at, &exact, &like],
    )
    .unwrap_or_default()
    .is_empty()
}

fn event(s: &hive_store::Store, now: &str, action: &str, slug: &str, detail: Value) {
    if let Ok(conn) = s.conn() {
        crate::util::event_at(&conn, now, ACTOR, action, slug, detail);
    }
}

/// A hold already recorded for exactly this work (`key`), newer than any
/// pull request of it, that still stands at `now`. A hold about the work
/// itself (a conflict, a branch that is gone) stands until the work
/// changes. One written because the way to the forge failed (`forge_fault`)
/// clears with its cause, which no commit of the work repairs, so it stands
/// for a wait and then the pass tries the same work again. The wait is the
/// pause of a failed job's recovery (`failure::recover_backoff`: 5 minutes,
/// then 15, 45 and 135, then 6 hours), by the tries in a row the hold
/// carries (`hold`): an outage of minutes heals within minutes, and a fault
/// that lasts is tried four times a day, never every pass. Before this
/// such a hold stood until the branch moved: three finished jobs stood for
/// a day after an account was repaired, and nine after a remote was.
fn held_already(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    field: &str,
    value: &str,
    key: &str,
    now: &str,
) -> Result<bool, String> {
    Ok(
        last_event_at(q, slug, field, value, true)?.is_some_and(|(action, d, at)| {
            action == "land.pr-held"
            && field_of(&d, "work") == key
            // A hold for a newer job carrying the same commits is read
            // again each pass (`repeated_by`): it stands while that job's
            // pull request is open or merged, not until the work changes.
            && d.get("repeated_by").is_none()
            && !(forge_fault(&field_of(&d, "note"))
                && crate::failure::recover_due(at, forge_tries(&d) - 1).as_str() <= now)
        }),
    )
}

/// The tries in a row a forge hold stands for (`tries`). A hold without
/// the count is the first: one from before the count was kept, or a kept
/// experiment's, which the experiment lane writes once and then retries on
/// its own schedule (`experiment.rs`).
fn forge_tries(hold: &Value) -> i32 {
    let tries = hold.get("tries").and_then(Value::as_i64).unwrap_or(1);
    i32::try_from(tries.max(1)).unwrap_or(i32::MAX)
}

/// How the note of a push or fetch starts when the push credential could
/// not be read for it.
const PUSH_CREDENTIAL: &str = "push credential: ";

/// How the note of a delivery starts when the store could not be read to
/// decide its lease: passing like a failure on the way to the forge, so the
/// hold is tried again after its wait, and never a push on that silence.
const STORE_UNREAD: &str = "store not read: ";

/// Whether a hold's note names a failure on the way to the forge rather
/// than the work: a push or a fetch git could not make (`push_rev`,
/// `fetch_head`), a `gh` call that failed (`landing_pr.rs`), a push
/// credential that could not be read, a store that could not be read to
/// decide a push's lease (`STORE_UNREAD`). Any other note is the work's, and
/// so is one that only begins with a branch of such a name. A push the
/// upstream answered and turned down (`rejected`) and a pull request with
/// nothing to show (`landing_pr::gh_failure`) open their notes otherwise
/// for that reason: no later try changes the answer.
fn forge_fault(note: &str) -> bool {
    // A fix's fetch names the pull request first (`deliver_fix`).
    let note = note
        .strip_prefix("pull request #")
        .and_then(|rest| rest.split_once(": "))
        .map_or(note, |(_, why)| why);
    [
        "push of ",
        "fetch of ",
        "gh pr ",
        "gh did not name ",
        "gh (the GitHub CLI) ",
        PUSH_CREDENTIAL,
        STORE_UNREAD,
    ]
    .iter()
    .any(|said| note.starts_with(said))
}

/// The note of a push that was turned down for what it is: git refused it
/// itself (`! [rejected]`: a lease that no longer holds), or the upstream
/// refused it for a rule of the repository (`! [remote rejected]` with a
/// hook that declined, a protected branch, a push declined by a rule, a
/// non-fast-forward). The same push gets the same answer, so the note
/// opens with the upstream, not with the push (`forge_fault` reads how a
/// note begins), and its hold stands until the work changes. The upstream
/// reports its own passing trouble under the same mark - a ref it could
/// not lock, objects it could not store - and any reason that is not a
/// rule stays a failed push, which is tried again. The reason is the text
/// inside the line's final parentheses (`reason_of`): the line also names
/// the branches, and a branch named like a rule (`fix/non-fast-forward`)
/// must not make every passing trouble of its push a standing hold. Read
/// from all git said: the note keeps the last 200 characters, which can
/// start after the mark.
fn rejected(
    pushed: &crate::util::GitResult,
    src: &str,
    head: &str,
    up: &Upstream,
) -> Option<String> {
    // "hook declined" is also how a pre-receive hook and a protected
    // branch are worded.
    const RULES: &[&str] = &[
        "hook declined",
        "push declined due to",
        "refusing to allow",
        "non-fast-forward",
    ];
    let said = format!("{} {}", pushed.stderr, pushed.stdout);
    let turned_down = !pushed.ok()
        && said.lines().map(str::trim_start).any(|line| {
            line.starts_with("! [rejected]")
                || (line.starts_with("! [remote rejected]")
                    && reason_of(line)
                        .is_some_and(|why| RULES.iter().any(|rule| why.contains(rule))))
        });
    turned_down.then(|| {
        hive_cred::redact(
            &format!(
                "{} rejected the push of {} to {head}: {}",
                up.name(),
                src.strip_prefix("refs/heads/").unwrap_or(src),
                crate::run::squeeze(&said, 200)
            ),
            &[],
        )
    })
}

/// The reason git gives at the end of a status line of a push, inside its
/// final parentheses: `! [remote rejected] a -> a (hook declined)` gives
/// `hook declined`. The reason may hold parentheses of its own, so the
/// group is the one the line's last `)` closes, matched back to its `(`.
/// `None` when the line does not end in such a group.
fn reason_of(line: &str) -> Option<&str> {
    let line = line.trim_end();
    let body = line.strip_suffix(')')?;
    let mut depth = 0usize;
    for (i, c) in body.char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' if depth == 0 => return Some(&body[i + 1..]),
            '(' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// What the holds since a work's last delivery on record say of the tries
/// made since, when one of them failed on the way to the forge: such a try
/// may have pushed the branch before the call that failed. The holds since
/// that delivery are read, not the newest alone: a try that found commits
/// on the upstream and did not push over them is held for the work, and
/// the next change of the work must not push over them either.
struct Tried {
    /// The commit the last of those tries left on the upstream, with the
    /// work key it was a push of: a task's hold records it (`pushed`) when
    /// the push went through and a later step failed.
    pushed: Option<(String, String)>,
}

/// `Err` when the holds cannot be read: the caller holds the delivery
/// rather than push with a plain force over what a hold it could not read
/// said it would not push over.
fn tried_since(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    field: &str,
    value: &str,
) -> Result<Option<Tried>, String> {
    // Every hold since the last delivery, however many: each refusal to
    // push is a hold of its own, and a new one is written whenever the
    // hold key changes (each pull request the project opens moves the
    // stack's top). A window of the newest few would let the forge hold
    // fall out of it, and the next try would push with a plain force over
    // the commits the holds since said it would not push over.
    let holds = holds_since_delivery(q, slug, field, value)?;
    Ok(holds
        .iter()
        .any(|hold| forge_fault(&field_of(hold, "note")))
        .then(|| Tried {
            pushed: holds.iter().find_map(|hold| {
                Some((
                    hold.get("pushed")?.as_str()?.to_string(),
                    field_of(hold, "work"),
                ))
            }),
        }))
}

fn field_of(v: &Value, name: &str) -> String {
    field(v, name).to_string()
}

/// Records that the work `detail` names (`task`, `job` or `head`, with its
/// `work` key) could not be delivered, and why. A hold for a failure on the
/// way to the forge (`forge_fault`) carries `tries`: one more than the
/// forge hold it follows for the same work key, one when the work's newest
/// event is anything else - a delivery, a hold for another key, a hold
/// about the work. `held_already` reads its wait from that.
fn hold(s: &hive_store::Store, now: &str, slug: &str, detail: Value, note: &str) {
    hold_as(s, now, ACTOR, slug, detail, note);
}

/// A kept experiment the experiment lane could not deliver, held as the
/// pass holds its own work: not again while a hold for the same work
/// stands, and anew - counted, and dated to this try - when a try the wait
/// let through fails too. So the lane's tries reach the forge on the waits
/// of any failed delivery, however often the lane asks. With one hold,
/// never written again, every try after its first five minutes went to
/// the forge: one every two minutes, about 700 in the lane's day, when
/// the forge answered 429.
pub(crate) fn hold_experiment(
    s: &hive_store::Store,
    now: &str,
    actor: &str,
    project: &Row,
    job: &Row,
    archive: &str,
    note: &str,
) {
    let slug = project.str("slug");
    let id = job.str("id");
    let branch = job.str("branch");
    let work = work_key(
        &branch,
        &rev(&project.str("path"), &format!("refs/heads/{branch}")),
    );
    let held = s
        .conn()
        .is_ok_and(|conn| held_already(&conn, &slug, "job", &id, &work, now).unwrap_or(false));
    if !held {
        hold_as(
            s,
            now,
            actor,
            &slug,
            json!({"job": id, "experiment": id, "archive": archive, "work": work}),
            note,
        );
    }
}

/// [`hold`], written as `actor`.
fn hold_as(
    s: &hive_store::Store,
    now: &str,
    actor: &str,
    slug: &str,
    mut detail: Value,
    note: &str,
) {
    detail["project"] = json!(slug);
    detail["note"] = json!(note);
    if forge_fault(note) {
        let before = s.conn().ok().and_then(|conn| {
            let (name, id) = ["task", "job", "head"]
                .into_iter()
                .find_map(|name| Some((name, detail.get(name)?.as_str()?)))?;
            // A failed read counts the hold as the first of its run: a
            // shorter wait, never a push.
            let (action, last) = last_event(&conn, slug, name, id, true).ok().flatten()?;
            (action == "land.pr-held"
                && field(&last, "work") == field(&detail, "work")
                && forge_fault(field(&last, "note")))
            .then(|| forge_tries(&last))
        });
        detail["tries"] = json!(before.unwrap_or(0).saturating_add(1));
    }
    if let Ok(conn) = s.conn() {
        crate::util::event_at(&conn, now, actor, "land.pr-held", slug, detail);
    }
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

#[allow(clippy::too_many_arguments)]
fn next_for(
    q: &(impl crate::util::Querier + ?Sized),
    root: &str,
    slug: &str,
    field: &str,
    value: &str,
    sha: &str,
    held_key: &str,
    now: &str,
) -> Result<Next, String> {
    // A store that cannot be read is no answer: `Open` on that silence
    // took work with a pull request open for never delivered, and its push
    // went out with a plain force onto the pull request's head.
    if held_already(q, slug, field, value, held_key, now)? {
        return Ok(Next::Nothing);
    }
    Ok(match last_event(q, slug, field, value, false)? {
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
            if pr_event(q, slug, number_of(&d))?.is_some_and(|pr| field_of(&pr, "to") == sha) {
                Next::Nothing
            } else {
                Next::Open
            }
        }
    })
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

/// The sides a task's build merges, without the job branches another side
/// carries already: a job branch whose own commits past `start` (merges
/// left out) are all in another side kept - by ancestry, or as the same
/// patch (`git cherry`) - adds nothing but conflicts. A rebuilt pull
/// request's head holds its first jobs' commits replayed onto a new base,
/// and a follow-up cut from it edits them again: merging the old branch
/// first conflicts with the follow-up (#268, 26 November). A join job's
/// branch merges the task's other branches: merging them first, in
/// creation order, meets the conflict the join resolved. The order stays
/// as given and the forge's head (no job index) is never dropped; of two
/// branches that carry each other the later one stays. A branch git cannot
/// read is kept: merging it is what was done before.
fn carried_elsewhere(
    root: &str,
    start: &str,
    merges: Vec<(String, String, Option<usize>)>,
) -> Vec<(String, String, Option<usize>)> {
    let mut kept = vec![true; merges.len()];
    for i in 0..merges.len() {
        if merges[i].2.is_none() {
            continue;
        }
        let own = commits_past(root, start, &merges[i].0);
        if own.is_empty() {
            continue;
        }
        let carried = (0..merges.len())
            .filter(|&j| j != i && kept[j] && merges[j].0 != merges[i].0)
            .any(|j| patches_in(root, start, &merges[i].0, &own, &merges[j].0));
        if carried {
            kept[i] = false;
        }
    }
    merges
        .into_iter()
        .zip(kept)
        .filter_map(|(m, k)| k.then_some(m))
        .collect()
}

/// Whether `rev` merges onto `start` alone without a conflict, read from
/// objects only (`git merge-tree --write-tree`: exit 0 clean, 1 conflicts).
/// False when git cannot say, so a branch is taken for one at odds with
/// its base, as before. No `--quiet`: git learned it in 2.46, and the
/// git of a rented Ubuntu 24.04 machine (2.43) refused it, so every branch
/// read as at odds with its base there and two delivery tests were red on
/// Linux only (TASK-156 item 1). The tree it prints is not read.
fn merges_alone(dir: &str, start: &str, rev: &str) -> bool {
    let out = crate::util::git_timeout(
        dir,
        Duration::from_secs(120),
        &["merge-tree", "--write-tree", start, rev],
    );
    out.err.is_none() && out.code == 0
}

/// The commits `sha` has past `start`, merges left out; empty when git
/// cannot say.
fn commits_past(root: &str, start: &str, sha: &str) -> Vec<String> {
    let listed = crate::util::git_timeout(
        root,
        Duration::from_secs(30),
        &["rev-list", "--no-merges", sha, "--not", start, "--"],
    );
    if !listed.ok() {
        return Vec::new();
    }
    listed
        .stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Whether every commit of `own` (of `sha`, past `start`) is in `other`:
/// reachable from it, or the same patch there. False when git fails.
fn patches_in(root: &str, start: &str, sha: &str, own: &[String], other: &str) -> bool {
    let cherry = crate::util::git_timeout(
        root,
        Duration::from_secs(60),
        &["cherry", other, sha, start],
    );
    if !cherry.ok() {
        return false;
    }
    let missing: std::collections::BTreeSet<&str> = cherry
        .stdout
        .lines()
        .filter_map(|l| l.strip_prefix("+ "))
        .map(str::trim)
        .collect();
    !own.iter().any(|c| missing.contains(c.as_str()))
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
    /// Whether the branch merges cleanly onto the base alone: then what it
    /// conflicts with is the task's other branches, and one job joins them
    /// all (`join_or_hold`).
    siblings: bool,
    /// The task's job branches the build merges, as indices into its
    /// branches: the ones another side carries already are passed over
    /// (`carried_elsewhere`), and a join merges these alone.
    sides: Vec<usize>,
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

    /// The base in words: `main`, or `hive/task-X (pull request #N)`.
    fn describe(&self) -> String {
        if self.stacked() {
            format!("{} (pull request #{})", self.branch, self.stack_on)
        } else {
            self.branch.clone()
        }
    }
}

/// Whether a pull request takes part in the stack: every kind but a
/// conversation's (`hive/chat-<id>`), whose branch the hub keeps checked
/// out in a worktree of its own - nothing may rebuild it, and nothing is
/// built on it - and an experiment's (`evolve/<id>`), which is measured on
/// the default branch alone: built on another pull request it would carry
/// that one's commits, and a pull request merged into a pull request
/// strands its change when the one below is closed or squashed
/// (`experiments.rs`).
fn stacks(pr: &Value) -> bool {
    field(pr, "conversation").is_empty()
        && !field(pr, "head").starts_with("hive/chat-")
        && !experiments::is_experiment(pr)
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
    s: &hive_store::Store,
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
    s: &hive_store::Store,
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

/// Who `take_in`'s merges are committed as; `own_tip` knows them by it.
const TAKE_IN_EMAIL: &str = "hive@localhost";

/// Builds `branch` on the stack's top: the top's head merged into the
/// branch in a scratch worktree (the branch is Hive's own, so a merge
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
    let wt_path = format!("{git_dir}/hive-stack/{}", branch.replace('/', "_"));
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
    // The committer is pinned in the environment too, where it wins over
    // `-c`: `own_tip` knows these merges by it.
    let committer = [
        "GIT_COMMITTER_NAME=Hive".to_string(),
        format!("GIT_COMMITTER_EMAIL={TAKE_IN_EMAIL}"),
    ];
    let m = crate::util::git_timeout_env(
        &wt_path,
        Duration::from_secs(300),
        &committer,
        &[
            "-c",
            "user.name=Hive",
            "-c",
            &format!("user.email={TAKE_IN_EMAIL}"),
            "merge",
            "--no-ff",
            "--no-edit",
            "-m",
            &format!("Merge {what} into {branch}"),
            &base.sha,
        ],
    );
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

/// Why a delivery failed, as the note of its hold, with the commit the try
/// left on the upstream when it got as far as the push: the pull request
/// is found or opened after it. A later try of a task goes by that commit
/// to tell its own push from somebody else's (`task_retry`).
#[derive(Debug)]
struct Undelivered {
    note: String,
    pushed: Option<String>,
}

impl From<String> for Undelivered {
    fn from(note: String) -> Self {
        Undelivered { note, pushed: None }
    }
}

/// Why a task's pull request could not be built this pass.
#[derive(Debug)]
enum TaskFault {
    Conflict(Box<Conflict>),
    /// Merged without a conflict, but the merged tree does not compile.
    Broken(Box<merge_check::Broken>),
    Other(Undelivered),
}

impl From<String> for TaskFault {
    fn from(note: String) -> Self {
        TaskFault::Other(note.into())
    }
}

impl From<Undelivered> for TaskFault {
    fn from(why: Undelivered) -> Self {
        TaskFault::Other(why)
    }
}

/// Delivers what the project has waiting: pull requests when it has an
/// upstream, local landings when it has none.
fn deliver(s: &hive_store::Store, project: &Row, now: &str) -> Vec<String> {
    let slug = project.str("slug");
    let root = project.str("path");
    let def = crate::baseline::default_branch(project.clone());
    let Some(up) = crate::forge::upstream_of(&root, &def) else {
        return land_locally(s, project, now);
    };
    let ready = ready_work(s, project, now);
    let retries = retries_this_pass(s, &root, &def, &slug, &ready, now);
    // A held work the pass would try again but has no room for this pass.
    let mut deferred = 0usize;
    let mut lines = Vec::new();
    let mut opened = 0usize;
    for rt in &ready.tasks {
        let task = &rt.task;
        // The key names each branch's sha, not the branch alone: a hold
        // stands until the work changes, and a branch that appears or moves
        // (a retry's new run) is new work the pass tries again, rather than
        // waiting for a new job of the task.
        let Ok(conn) = s.conn() else { break };
        let (key, top) = task_key(&conn, &root, &slug, rt);
        // Whether the task has a pull request open, or a hold standing,
        // could not be read: held for that, never taken for "nothing open".
        let unread = match held_already(&conn, &slug, "task", task, &key, now) {
            Ok(true) => continue,
            Ok(false) => rt.unread.clone(),
            Err(why) => Some(why),
        };
        if let Some(why) = unread {
            drop(conn);
            hold(s, now, &slug, json!({"task": task, "work": key}), &why);
            lines.push(format!("task pr {slug} {task}: held: {why}"));
            continue;
        }
        if !retries.allows("task", task, &key) {
            deferred += 1;
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
            Err(TaskFault::Broken(b)) => {
                lines.push(merge_check::fix_or_hold(s, project, rt, &key, *b, now))
            }
            Err(TaskFault::Other(why)) => {
                let mut held = json!({"task": task, "work": key});
                if let Some(sha) = &why.pushed {
                    held["pushed"] = json!(sha);
                }
                hold(s, now, &slug, held, &why.note);
                lines.push(format!("task pr {slug} {task}: held: {}", why.note));
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
                if !held_already(&conn, &slug, "job", &id, &branch, now).unwrap_or(true) {
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
        let Ok(conn) = s.conn() else { break };
        let (key, sha, top) = job_key(&conn, &root, &slug, rj);
        let next = match next_for(&conn, &root, &slug, "job", &id, &sha, &key, now) {
            Ok(Next::Nothing) => continue,
            Ok(next) => next,
            Err(why) => {
                drop(conn);
                hold(s, now, &slug, json!({"job": id, "work": key}), &why);
                lines.push(format!("job pr {slug} {id}: held: {why}"));
                continue;
            }
        };
        // A try again of a job whose commits another job carries on a pull
        // request still open or merged: held for that, under the branch and
        // commit alone, which stands whatever pull request the work would
        // stack on. It is read again each pass: should that pull request
        // close unmerged, this one is tried again. A read that fails - the
        // store, or a git call that fails or times out - says nothing of
        // the commits: the work stays held and is read again next pass.
        let repeat = if retries.again("job", &id, &key) {
            repeated_by(&conn, &root, &def, &slug, job, &sha)
        } else {
            Ok(None)
        };
        let newer = match repeat {
            Ok(newer) => newer,
            Err(why) => {
                lines.push(format!("job pr {slug} {id}: kept held: {why}"));
                continue;
            }
        };
        if let Some(newer) = newer {
            let key = work_key(&branch, &own_tip(&root, &sha));
            if repeat_held(&conn, &slug, &id, &key, &newer.job) {
                continue;
            }
            drop(conn);
            let note = format!(
                "{branch} is not tried again: job {} has its commits on pull request #{} ({}), which carries every commit of it",
                newer.job,
                newer.number,
                if newer.merged { "merged" } else { "open" }
            );
            hold(
                s,
                now,
                &slug,
                json!({"job": id, "work": key, "repeated_by": newer.job}),
                &note,
            );
            lines.push(format!("job pr {slug} {id}: held: {note}"));
            continue;
        }
        if !retries.allows("job", &id, &key) {
            deferred += 1;
            continue;
        }
        drop(conn);
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
        let next = match next_for(&conn, &root, &slug, "head", &cb.branch, &cb.sha, &key, now) {
            Ok(Next::Nothing) => continue,
            Ok(next) => next,
            Err(why) => {
                drop(conn);
                let mut d = json!({"head": cb.branch, "work": key});
                if let Some(id) = cb.conversation() {
                    d["conversation"] = json!(id);
                }
                hold(s, now, &slug, d, &why);
                lines.push(format!("proposed {slug} {}: held: {why}", cb.branch));
                continue;
            }
        };
        if !retries.allows("head", &cb.branch, &key) {
            deferred += 1;
            continue;
        }
        drop(conn);
        // A change that has a face opens its pull request with its report
        // (`landing.ui_report = require`): the session is asked once and the
        // pull request waits for it a while (ui_evidence.rs).
        if matches!(next, Next::Open)
            && let Some(line) =
                crate::ui_evidence::session_waits_for_report(s, project, &cb.branch, now)
        {
            lines.push(line);
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
    if deferred > 0 {
        lines.push(format!(
            "{slug}: {deferred} held deliveries wait for a later pass ({RETRIES_PER_PASS} are tried again a pass, the oldest hold first)"
        ));
    }
    lines
}

/// A task's hold key and the open pull request it would stack on: every
/// job branch with its sha now, and the stack's top when the task has no
/// pull request open.
fn task_key(
    q: &(impl crate::util::Querier + ?Sized),
    root: &str,
    slug: &str,
    rt: &ReadyTask,
) -> (String, Option<Value>) {
    let top = if rt.open.is_none() {
        stack_top(q, slug)
    } else {
        None
    };
    (
        stack_key(task_work_key(root, &rt.branches), top.as_ref()),
        top,
    )
}

/// A job's hold key, its branch's sha and the pull request it would stack
/// on. A fix goes onto the pull request it fixes, wherever that stands in
/// the stack; only a pull request of the job's own stacks.
fn job_key(
    q: &(impl crate::util::Querier + ?Sized),
    root: &str,
    slug: &str,
    rj: &ReadyJob,
) -> (String, String, Option<Value>) {
    let branch = rj.job.str("branch");
    let sha = rev(root, &format!("refs/heads/{branch}"));
    let top = if rj.fix_of.is_none() {
        stack_top(q, slug)
    } else {
        None
    };
    (
        stack_key(work_key(&branch, &own_tip(root, &sha)), top.as_ref()),
        sha,
        top,
    )
}

/// The commit a branch at `sha` stands for as a job's work: the tip, or,
/// when `take_in` merged the stack's top into it, the commit below those
/// merges along the first parents. Moving a job's branch onto a new top is
/// no change of the job's work: a hold keyed on it still stands for the
/// moved branch, and a try of it is a try again, which waits for its hold
/// and counts against the bound of a pass. Only `take_in`'s own merges are
/// passed over (committed as `hive@localhost`): a merge the worker made is
/// its work, and may be what resolved a conflict.
fn own_tip(root: &str, sha: &str) -> String {
    if sha.is_empty() {
        return String::new();
    }
    let log = crate::util::git_timeout(
        root,
        Duration::from_secs(20),
        &[
            "log",
            "--first-parent",
            "-n",
            "64",
            "--format=%H %ce %P",
            sha,
        ],
    );
    if !log.ok() {
        return sha.to_string();
    }
    for line in log.stdout.lines() {
        let mut parts = line.split_whitespace();
        let (Some(commit), Some(email)) = (parts.next(), parts.next()) else {
            break;
        };
        let take_in_merge = email == TAKE_IN_EMAIL && parts.count() > 1;
        if !take_in_merge {
            return commit.to_string();
        }
    }
    sha.to_string()
}

/// A work's newest delivery event when that event is a hold, as its `seq`
/// and the `work` key it was held for.
fn newest_hold(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    field: &str,
    value: &str,
) -> Option<(i64, String)> {
    let about = about_work(field)?;
    let rows = crate::util::query_rows(
        q,
        &format!(
            "SELECT action, seq, detail FROM events WHERE subject = ?1
             AND action IN ('land.pr-held', {DELIVERED})
             AND json_valid(detail) AND {about} ORDER BY seq DESC LIMIT 1"
        ),
        &[&slug, &value],
    )
    .unwrap_or_default();
    rows.first()
        .filter(|row| row.str("action") == "land.pr-held")
        .map(|row| {
            let detail = serde_json::from_str(&row.str("detail")).unwrap_or(Value::Null);
            (i64::from(row.int("seq")), field_of(&detail, "work"))
        })
}

/// Whether a try under hold key `key` is a try again of the work its
/// newest hold (`held`) was written for: the same branches at the same
/// commits. The pull request the work would stack on may have moved
/// (`stack_key`'s ` on #N`): another pull request opening moves it under
/// every held work at once. A work whose own commits changed is new work,
/// tried at once.
fn same_work(held: &str, key: &str) -> bool {
    let bare = |k: &str| match k.rsplit_once(" on #") {
        Some((work, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => {
            work.to_string()
        }
        _ => k.to_string(),
    };
    bare(held) == bare(key)
}

/// Held deliveries one pass tries again. When holds stop standing together
/// (a forge that is back, a pull request that moves the stack's top under
/// every held work), a project must not get a pull request for each of them
/// at once: on 26 November one new pull request released 38 holds of one
/// project together. The rest wait for the next pass.
const RETRIES_PER_PASS: usize = 3;

type WorkId = (&'static str, String);

/// The held works this pass tries again: of those whose hold no longer
/// stands, the `RETRIES_PER_PASS` with the oldest holds. Work never held,
/// and work whose commits changed since its hold (`same_work`), is new and
/// not counted. A job another job repeats is held for that instead
/// (`repeated_by`) and takes no place.
#[derive(Debug, Default)]
struct Retries {
    /// Each ready work whose newest delivery event is a hold, with that
    /// hold's `work` key.
    held: BTreeMap<WorkId, String>,
    chosen: std::collections::BTreeSet<WorkId>,
}

impl Retries {
    /// Whether a try of this work under hold key `key` is a try again.
    fn again(&self, field: &'static str, value: &str, key: &str) -> bool {
        self.held
            .get(&(field, value.to_string()))
            .is_some_and(|held| same_work(held, key))
    }

    /// Whether the pass may try this work under hold key `key`: it is not
    /// a try again, or it is one of the retries chosen for this pass.
    fn allows(&self, field: &'static str, value: &str, key: &str) -> bool {
        !self.again(field, value, key) || self.chosen.contains(&(field, value.to_string()))
    }
}

/// Ranks the held works of `ready` for this pass: one query a ready work,
/// and the hold keys and forge waits of the held ones only.
fn retries_this_pass(
    s: &hive_store::Store,
    root: &str,
    def: &str,
    slug: &str,
    ready: &Ready,
    now: &str,
) -> Retries {
    let mut r = Retries::default();
    let Ok(conn) = s.conn() else {
        return r;
    };
    let mut due: Vec<(i64, WorkId)> = Vec::new();
    for rt in &ready.tasks {
        let Some((seq, held)) = newest_hold(&conn, slug, "task", &rt.task) else {
            continue;
        };
        let (key, _) = task_key(&conn, root, slug, rt);
        if same_work(&held, &key)
            && held_already(&conn, slug, "task", &rt.task, &key, now) == Ok(false)
        {
            due.push((seq, ("task", rt.task.clone())));
        }
        r.held.insert(("task", rt.task.clone()), held);
    }
    for rj in &ready.jobs {
        let id = rj.job.str("id");
        let Some((seq, held)) = newest_hold(&conn, slug, "job", &id) else {
            continue;
        };
        let (key, sha, _) = job_key(&conn, root, slug, rj);
        if same_work(&held, &key)
            && next_for(&conn, root, slug, "job", &id, &sha, &key, now)
                .is_ok_and(|next| next != Next::Nothing)
            && matches!(repeated_by(&conn, root, def, slug, &rj.job, &sha), Ok(None))
        {
            due.push((seq, ("job", id.clone())));
        }
        r.held.insert(("job", id), held);
    }
    for cb in ready.conversations.iter().filter(|cb| cb.settled()) {
        let Some((seq, held)) = newest_hold(&conn, slug, "head", &cb.branch) else {
            continue;
        };
        let key = work_key(&cb.branch, &cb.sha);
        if same_work(&held, &key)
            && next_for(&conn, root, slug, "head", &cb.branch, &cb.sha, &key, now)
                .is_ok_and(|next| next != Next::Nothing)
        {
            due.push((seq, ("head", cb.branch.clone())));
        }
        r.held.insert(("head", cb.branch.clone()), held);
    }
    due.sort();
    r.chosen = due
        .into_iter()
        .take(RETRIES_PER_PASS)
        .map(|(_, work)| work)
        .collect();
    r
}

/// Another job that carries a held job's commits, and where it carries
/// them: on its pull request still open, or merged.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Repeat {
    job: String,
    number: i64,
    merged: bool,
}

/// How the repeat rule's note begins when the store or git failed or
/// timed out: the rule could not tell, and a hold it could not read again
/// is kept rather than released.
const REPEAT_UNREAD: &str = "whether another job carries its commits could not be read: ";

/// The commits a job made itself on its branch at `sha`: those after the
/// default branch along the first parents, merges left out - the stack's
/// top merged in by `take_in` is a merge, and what it brought sits on its
/// second parent, so it is not the job's. `Err` when git failed.
fn own_commits(root: &str, def: &str, sha: &str) -> Result<Vec<String>, String> {
    let listed = crate::util::git_timeout(
        root,
        Duration::from_secs(30),
        &[
            "rev-list",
            "--first-parent",
            "--no-merges",
            &format!("refs/heads/{def}..{sha}"),
        ],
    );
    if !listed.ok() {
        return Err(git_said(&listed));
    }
    Ok(listed
        .stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

/// What a failed git call of the repeat rule said, for its note.
fn git_said(out: &crate::util::GitResult) -> String {
    let said = out
        .err
        .clone()
        .unwrap_or_else(|| format!("git exited {}: {}", out.code, out.stderr.trim()));
    format!("{REPEAT_UNREAD}{}", crate::run::squeeze(&said, 200))
}

/// Whether every commit of `own` is in `other` by its change: reachable
/// from it, or the same patch there (`git cherry`, which compares patch
/// ids, so a commit picked or rebased onto another branch counts). `Err`
/// when git failed or timed out: that says nothing of the commits.
fn carried_by(
    root: &str,
    def: &str,
    sha: &str,
    own: &[String],
    other: &str,
) -> Result<bool, String> {
    let cherry = crate::util::git_timeout(
        root,
        Duration::from_secs(60),
        &["cherry", other, sha, &format!("refs/heads/{def}")],
    );
    if !cherry.ok() {
        return Err(git_said(&cherry));
    }
    let missing: std::collections::BTreeSet<&str> = cherry
        .stdout
        .lines()
        .filter_map(|l| l.strip_prefix("+ "))
        .map(str::trim)
        .collect();
    Ok(!own.iter().any(|c| missing.contains(c.as_str())))
}

/// Every pull request on record that delivers a job other than `job` and
/// is still open or merged, newest first, as (job, number, merged, the
/// pushed head `to`). A task's pull request names its jobs in `jobs`; the
/// first of them other than `job` stands for it. Any age: a window of the
/// newest few let a carrier fall out of it, and the held job was tried
/// again and opened a second pull request of the same commits.
fn carriers(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    job: &str,
    branch: &str,
) -> Result<Vec<(String, i64, bool, String)>, String> {
    let rows = crate::util::query_rows(
        q,
        &format!(
            "SELECT action, detail FROM events WHERE subject = ? AND action IN ({DELIVERED})
             AND json_valid(detail) ORDER BY seq"
        ),
        &[&slug],
    )
    .map_err(|e| format!("{STORE_UNREAD}the pull requests of {slug} could not be read: {e}"))?;
    // By number: the newest state, the newest pushed head, and the jobs
    // the pull request delivers.
    #[derive(Default)]
    struct Pr {
        action: String,
        to: String,
        head: String,
        jobs: Vec<String>,
    }
    let mut by_number: BTreeMap<i64, Pr> = BTreeMap::new();
    for row in rows {
        let d: Value = serde_json::from_str(&row.str("detail")).unwrap_or(Value::Null);
        let number = number_of(&d);
        if number == 0 {
            continue;
        }
        let action = row.str("action");
        let entry = by_number.entry(number).or_default();
        // A later `land.pr` of a merged number is a push recorded after the
        // merge was seen: the merge stands (`open_prs_in_order`).
        if action == "land.pr" && entry.action == "land.merged" {
            continue;
        }
        let mut jobs: Vec<String> = d
            .get("jobs")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        if !field(&d, "job").is_empty() {
            jobs.insert(0, field_of(&d, "job"));
        }
        if action == "land.pr" {
            entry.to = field_of(&d, "to");
            entry.head = field_of(&d, "head");
        }
        if !jobs.is_empty() {
            entry.jobs = jobs;
        }
        entry.action = action;
    }
    Ok(by_number
        .into_iter()
        .rev()
        .filter(|(_, pr)| {
            pr.action != "land.pr-closed"
                && !pr.to.is_empty()
                // The held job's own pull request is no repeat of it, nor
                // is one proposed by hand from its branch.
                && !pr.jobs.iter().any(|j| j == job)
                && (branch.is_empty() || pr.head != branch)
        })
        .filter_map(|(number, pr)| {
            let other = pr.jobs.into_iter().next()?;
            Some((other, number, pr.action == "land.merged", pr.to))
        })
        .collect())
}

/// Another job whose delivered or open work carries every commit `job`
/// made itself on its branch at `sha`: its pull request is open, or
/// merged, and holds those commits by their change (`carried_by`). Trying
/// `job` again would open a second pull request of the same commits. Any
/// other job counts, older or newer: of two held jobs with the same
/// commits, released together, the one tried first opens its pull request
/// and the other is held for it. Where the jobs came from says nothing:
/// one source names different work from one sitting to the next (a lead's
/// idle work counts again at every sitting, a schedule does what it finds,
/// a newer security job answers only the alerts the older one left). A job
/// whose delivery is held, or whose pull request was closed unmerged,
/// carries nothing; nor does a job with no commits of its own.
///
/// `Err` when the store or git could not say: the caller keeps the work
/// held, as a failed read must not release a hold.
fn repeated_by(
    q: &(impl crate::util::Querier + ?Sized),
    root: &str,
    def: &str,
    slug: &str,
    job: &Row,
    sha: &str,
) -> Result<Option<Repeat>, String> {
    if sha.is_empty() {
        return Ok(None);
    }
    let own = own_commits(root, def, sha)?;
    if own.is_empty() {
        return Ok(None);
    }
    let candidates = carriers(q, slug, &job.str("id"), &job.str("branch"))?;
    if candidates.is_empty() {
        return Ok(None);
    }
    // The pushed heads the checkout has: a head it has not got carries
    // nothing it can show (a pull request from another clone).
    let tos: Vec<&str> = candidates.iter().map(|(_, _, _, to)| to.as_str()).collect();
    let mut args = vec!["rev-list", "--no-walk", "--ignore-missing"];
    args.extend(tos.iter().copied());
    let present = crate::util::git_timeout(root, Duration::from_secs(30), &args);
    if !present.ok() {
        return Err(git_said(&present));
    }
    let present: std::collections::BTreeSet<&str> = present.stdout.lines().map(str::trim).collect();
    let candidates: Vec<_> = candidates
        .iter()
        .filter(|(_, _, _, to)| present.contains(to.as_str()))
        .collect();
    if candidates.is_empty() {
        return Ok(None);
    }
    // One call over every candidate first: a commit of the job that none
    // of them has, by patch or by reachability, is listed with `<`, and
    // then no one of them carries all. Only when they do together is each
    // read on its own, so a pass reads every pull request on record for a
    // held job without a `git cherry` for each.
    let ranges: Vec<String> = candidates
        .iter()
        .map(|(_, _, _, to)| format!("{sha}...{to}"))
        .collect();
    let mut args = vec!["rev-list", "--left-right", "--cherry-mark", "--no-merges"];
    args.extend(ranges.iter().map(String::as_str));
    args.push("--");
    let marked = crate::util::git_timeout(root, Duration::from_secs(60), &args);
    if !marked.ok() {
        return Err(git_said(&marked));
    }
    let alone: std::collections::BTreeSet<&str> = marked
        .stdout
        .lines()
        .filter_map(|l| l.strip_prefix('<'))
        .map(str::trim)
        .collect();
    if own.iter().any(|c| alone.contains(c.as_str())) {
        return Ok(None);
    }
    for (other, number, merged, to) in candidates {
        if carried_by(root, def, sha, &own, to)? {
            return Ok(Some(Repeat {
                job: other.clone(),
                number: *number,
                merged: *merged,
            }));
        }
    }
    Ok(None)
}

/// Whether the job's newest delivery event is the hold `repeated_by`
/// wrote for this very work (`key`) and other job: written once, and read
/// again each pass, so it stands while that job's work is open or merged.
/// A store that cannot be read keeps it standing.
fn repeat_held(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    job: &str,
    key: &str,
    newer: &str,
) -> bool {
    last_event(q, slug, "job", job, true).map_or(true, |last| {
        last.is_some_and(|(action, d)| {
            action == "land.pr-held"
                && field(&d, "work") == key
                && field(&d, "repeated_by") == newer
        })
    })
}

/// What `hive land --dry-run` prints: the work a pass would deliver.
pub(crate) fn dry_run(s: &hive_store::Store, project: &Row, now: &str) -> Vec<String> {
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
fn land_locally(s: &hive_store::Store, project: &Row, now: &str) -> Vec<String> {
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
    let ctx = hive_bus::CancellationToken::new();
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
    s: &hive_store::Store,
    project: &Row,
    up: &Upstream,
    work: Work<'_>,
    now: &str,
) -> Result<Delivered, Undelivered> {
    let slug = project.str("slug");
    let root = project.str("path");
    let def = crate::baseline::default_branch(project.clone());
    let lease = work.lease.clone().or_else(|| {
        work.open
            .as_ref()
            .map(|d| field_of(d, "to"))
            .filter(|to| !to.is_empty())
    });
    let lease = match lease {
        Some(tip) => Some(tip),
        None => first_push_lease(s, &slug, &root, up, &work)?,
    };
    // The upstream holds the tip already when a try before this one
    // pushed it and nothing has changed: nothing to push.
    if lease.as_deref() != Some(work.sha.as_str()) {
        push(s, &root, up, &work.local, &work.head, lease.as_deref())?;
    }
    // From here the branch on the upstream is at the tip, and a failure
    // says so.
    let pushed = |note: String| Undelivered {
        note,
        pushed: Some(work.sha.clone()),
    };
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
            s,
            &root,
            &up.repo,
            &work.head,
            &work.base.branch,
        )
        .map_err(&pushed)?
        {
            Some((n, u)) => (n, u, false),
            None => {
                let (n, u) = crate::landing_pr::create_pull_request(
                    s,
                    &root,
                    &up.repo,
                    &work.head,
                    &work.base.branch,
                    &work.title,
                    &work.body,
                )
                .map_err(&pushed)?;
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

/// The lease of a branch no pull request is on record for. Its first push
/// forces (`push`) - but not after a hold for a failure on the way to the
/// forge (`tried_since`). A try made since may have pushed the branch
/// before the call that failed: `gh pr create` fails after the push, and
/// its answer can be lost once the pull request is open. And somebody may
/// have pushed onto the branch during the wait. So the branch on the
/// upstream is read first, as the head of an open pull request is: not
/// there, the push must find it so (an empty lease); there with nothing the
/// tip has not, the push leases on it; there with commits the tip has not,
/// nothing is pushed and the note says so. None when the push is a first
/// one as far as the record goes.
fn first_push_lease(
    s: &hive_store::Store,
    slug: &str,
    root: &str,
    up: &Upstream,
    work: &Work<'_>,
) -> Result<Option<String>, String> {
    // The work as its holds name it: a task or a job by its id (an
    // experiment is held as its job), any other branch by its name.
    let (field, value) = work
        .ids
        .iter()
        .find(|(name, _)| matches!(*name, "task" | "job"))
        .map_or(("head", work.head.as_str()), |(name, id)| {
            (*name, id.as_str())
        });
    // A store that cannot be read says nothing of the tries since: the
    // delivery is held, never pushed with a plain force on that silence.
    let conn = s.conn().map_err(|e| {
        format!(
            "{STORE_UNREAD}the lease of {} could not be decided: {e}",
            work.head
        )
    })?;
    let retried = tried_since(&conn, slug, field, value)?.is_some();
    drop(conn);
    if !retried {
        return Ok(None);
    }
    let there = fetch_head(s, root, up, &work.head)?;
    if there.is_empty() || there == work.sha || is_ancestor(root, &there, &work.sha) {
        return Ok(Some(there));
    }
    Err(format!(
        "{head} on {remote} is at {}, which the branch here is not built on, and a try after a failed one does not push over it: take those commits into {local}, or delete {head} on {remote} and run hive land --branch {local}",
        crate::recipe::short(&there, 12),
        local = work.local,
        head = work.head,
        remote = up.remote
    ))
}

/// Pushes `local` to `head` on the upstream. A known previous tip is the
/// lease, and an empty one says the upstream has no such branch; the first
/// push of a branch forces, since the `hive/` namespace on the remote is
/// the daemon's own and a bare `--force-with-lease` has no tracking ref to
/// read when the push goes to a url.
fn push(
    s: &hive_store::Store,
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
    s: &hive_store::Store,
    root: &str,
    up: &Upstream,
    src: &str,
    head: &str,
    lease: Option<&str>,
) -> Result<(), String> {
    let (mut env, cleanup, why) = crate::forge::remote_credential_env(s, root.to_string(), &up.url);
    // The push's status line is read below (`rejected`): a rule the
    // upstream turned the push down for stands, a passing trouble is tried
    // again. So the push speaks git's own tongue whatever the daemon's
    // locale, as the fetch does (`fetch_head`).
    env.push("LC_ALL=C".into());
    if !why.is_empty() {
        return Err(format!("{PUSH_CREDENTIAL}{why}"));
    }
    let force = match lease {
        Some(sha) => format!("--force-with-lease=refs/heads/{head}:{sha}"),
        None => "--force".to_string(),
    };
    let (pushed, https) = crate::forge::git_upstream_said(
        root,
        Duration::from_secs(300),
        &env,
        &up.url,
        &["push", &force, &up.url, &format!("{src}:refs/heads/{head}")],
    );
    cleanup("", &[]);
    if let Some(note) = rejected(&pushed, src, head, up) {
        return Err(note);
    }
    if !pushed.ok() {
        let why = format!("{} {}", pushed.stderr, pushed.stdout);
        return Err(hive_cred::redact(
            &format!(
                "push of {} to {head} on {} {}: {}{}",
                src.strip_prefix("refs/heads/").unwrap_or(src),
                up.name(),
                crate::forge::failed_after(https.as_deref()),
                crate::run::squeeze(&why, 200),
                crate::forge::auth_fix(up, &env, &why)
            ),
            &[],
        ));
    }
    Ok(())
}

/// What a try of a task with no pull request on record finds of its branch
/// on the upstream after a failure on the way to the forge (`tried_since`):
/// the commit the push is to find there (empty: no such branch), and
/// whether the branch is to be left as it is. None when the push is a
/// first one as far as the record goes, and forces.
///
/// A try made since may have pushed the branch before `gh` failed, and
/// somebody may have pushed onto it during the wait. What the branch holds
/// does not tell the two apart: a build stacked on a pull request that has
/// since closed holds the task's commits and that one's too, and once a
/// rebase job has replaced the task's branches it holds none of them. So
/// Hive goes by what it recorded. The branch is its own at the commit a try
/// since pushed (`pushed` of its hold), and at the last push of the task's
/// own pull request once that is closed: it keeps its branch on the
/// upstream, and the task's next round has the same name for its own.
/// There the branch is replaced by a build on the base as it is now, as a
/// first delivery makes it, leasing on that commit - or left, when it is
/// the build of this very work (`key`) on a base (`start`) that has not
/// moved. At any other commit nothing is pushed, and the note says so.
fn task_retry(
    s: &hive_store::Store,
    project: &Row,
    up: &Upstream,
    task: &str,
    head: &str,
    key: &str,
    start: &str,
) -> Result<Option<(String, bool)>, String> {
    let slug = project.str("slug");
    let root = project.str("path");
    // A store that cannot be read holds the delivery: the build after this
    // would replace the branch on the upstream with a plain force.
    let conn = s
        .conn()
        .map_err(|e| format!("{STORE_UNREAD}the lease of {head} could not be decided: {e}"))?;
    let Some(tried) = tried_since(&conn, &slug, "task", task)? else {
        return Ok(None);
    };
    let closed = match last_event(&conn, &slug, "task", task, false)? {
        Some((action, closed)) if action != "land.pr" => {
            pr_event(&conn, &slug, number_of(&closed))?
        }
        _ => None,
    }
    .filter(|pr| field(pr, "head") == head)
    .map(|pr| field_of(&pr, "to"));
    drop(conn);
    let there = fetch_head(s, &root, up, head)?;
    if there.is_empty() {
        return Ok(Some((there, false)));
    }
    match tried.pushed {
        Some((sha, work)) if sha == there => {
            let same = work == key && is_ancestor(&root, start, &there);
            Ok(Some((there, same)))
        }
        _ if closed.as_deref() == Some(there.as_str()) => Ok(Some((there, false))),
        _ => Err(format!(
            "{head} on {remote} holds commits Hive did not put there (it is at {}), and a try after a failed one does not push over them: delete {head} on {remote} once nothing there is wanted; the task's pull request opens when its work next changes",
            crate::recipe::short(&there, 12),
            remote = up.remote
        )),
    }
}

/// Builds `hive/task-<id>` from its base plus every job branch, pushes
/// it and opens (or finds) its pull request. The base is the default branch,
/// or the head of the stack's top `top` when another pull request is open:
/// the task's then stacks on it. When the task's pull request is open
/// already, the base is the one on record, and the rebuilt branch also
/// takes in whatever the pull request's head holds beyond the job branches
/// (a reviewer's or the owner's commits on the forge), so a follow-up job
/// never pushes those away, and the push expects that head. A try after a
/// hold for a failure on the way to the forge reads the branch on the
/// upstream first and goes by what Hive recorded of it (`task_retry`).
fn deliver_task(
    s: &hive_store::Store,
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
    // What a try after a failure on the way to the forge finds of the
    // branch on the upstream: the commit the push is to find there, and
    // whether the branch is this very build.
    let retry = match &rt.open {
        Some(_) => None,
        None => {
            let key = stack_key(task_work_key(&root, branches), top);
            task_retry(s, project, up, task, &head, &key, &start)?
        }
    };
    let work = |sha: String, lease: Option<String>| Work {
        kind: "task",
        ids: vec![("task", task.to_string())],
        jobs: branches.iter().map(|(j, _)| j.clone()).collect(),
        title: format!("{task}: {}", rt.title),
        body: task_body(task, &rt.title, branches),
        local: head.clone(),
        head: head.clone(),
        sha,
        base: base.clone(),
        open: rt.open.clone(),
        lease,
    };
    if let Some((there, true)) = &retry {
        // Nothing to build and nothing to push: the pull request is what
        // is still to open.
        let left = work(there.clone(), Some(there.clone()));
        return Ok(deliver_work(s, project, up, left, now)?.line);
    }
    let git = |args: &[&str]| crate::util::git_timeout(&root, Duration::from_secs(120), args);
    let git_dir = git(&["rev-parse", "--absolute-git-dir"])
        .stdout
        .trim()
        .to_string();
    if git_dir.is_empty() {
        return Err(format!("{root} is not a git checkout").into());
    }
    let wt_path = format!("{git_dir}/hive-task-pr/{task}");
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
    // A job branch is merged only when no other side carries it already
    // (`carried_elsewhere`).
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
    let merges = carried_elsewhere(&root, &start, merges);
    let mut result: Result<(), TaskFault> = Ok(());
    // The job branches this pass merged, as indices into `branches`.
    let mut merged_jobs: Vec<usize> = Vec::new();
    let mut skipped = String::new();
    for (rev, what, which) in &merges {
        let contained = in_wt(&["merge-base", "--is-ancestor", rev, "HEAD"]);
        if contained.ok() {
            continue;
        }
        merged_jobs.extend(*which);
        let m = in_wt(&[
            "-c",
            "user.name=Hive",
            "-c",
            "user.email=hive@localhost",
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
                    siblings: merges_alone(&wt_path, &start, rev),
                    sides: merges.iter().filter_map(|m| m.2).collect(),
                })),
                _ => format!(
                    "{what} does not merge cleanly onto {} with the task's other branches: {}",
                    base.describe(),
                    crate::run::squeeze(&format!("{} {}", m.stdout, m.stderr), 200)
                )
                .into(),
            });
            break;
        }
    }
    let tip = in_wt(&["rev-parse", "HEAD"]).stdout.trim().to_string();
    // Every side passed its own check; a merged tree none of them had -
    // not the base's, not a branch's - is code nobody compiled
    // (merge_check.rs).
    if result.is_ok() && !merged_jobs.is_empty() {
        let tree = |r: &str| {
            in_wt(&["rev-parse", &format!("{r}^{{tree}}")])
                .stdout
                .trim()
                .to_string()
        };
        let merged_tree = tree("HEAD");
        let new_tree = std::iter::once(&start)
            .chain(merges.iter().map(|(rev, _, _)| rev))
            .all(|rev| tree(rev) != merged_tree);
        if new_tree {
            match merge_check::check_merged_tree(s, &project.str("slug"), &root, &def, &wt_path) {
                merge_check::Checked::Broken(errors) => {
                    result = Err(TaskFault::Broken(Box::new(merge_check::Broken {
                        merged: merged_jobs
                            .iter()
                            .map(|&i| {
                                (
                                    branches[i].0.clone(),
                                    branches[i].1.clone(),
                                    shas[i].clone(),
                                )
                            })
                            .collect(),
                        tip: tip.clone(),
                        errors,
                        base: base.clone(),
                    })));
                }
                merge_check::Checked::Skipped(why) if !why.is_empty() => {
                    skipped = format!(" (merge check skipped: {why})");
                }
                _ => {}
            }
        }
    }
    let _ = git(&["worktree", "remove", "--force", &wt_path]);
    result?;
    let lease = remote_head.or(retry.map(|(there, _)| there));
    let delivered = deliver_work(s, project, up, work(tip, lease), now)?.line;
    Ok(format!("{delivered}{skipped}"))
}

/// How many times a task's conflicting branch is handed to a job before the
/// owner is asked: the rebase job starts from the base as it is, so a
/// second conflict means the work itself is at odds with what landed.
const REBASES_PER_TASK: usize = 1;

/// How many join jobs a task gets before the owner is asked: a join merges
/// all the task's branches at once, so a second conflict among them means
/// the work itself is at odds. Counted apart from the rebases: a rebase
/// that ran for a conflict with the base says nothing about the task's
/// branches among themselves (TASK-129 was held at once so).
const JOINS_PER_TASK: usize = 1;

/// The recipe both a rebase and a join job follow. A merge, never a copy:
/// the landing accepts commits already on another of Hive's branches as
/// they are and answers only for the run's own, so the merge commits are
/// the worker's and the branches' commits stay their authors'. A
/// cherry-pick or a rebase writes the commits anew under the worker's name
/// from another agent's work, and the landing refuses them (TASK-128).
const MERGE_RECIPE: &str = "Merge, do not copy: `git merge --no-ff <commit>` for each branch named, resolve the conflicts in the merge commit keeping the intent of both sides, and run the project's check. \
     Do not `git cherry-pick` or `git rebase` them: the landing accepts another agent's commits only as they are, reached through a merge, and refuses copies of them as work you did not write.";

/// A job branch that conflicts with the task's base - the default branch,
/// or the pull request the task's stacks on - is work for an agent before
/// it is a question for the owner: a `task` job on the same task is queued
/// to bring the branch over onto the base and resolve the conflict, the
/// conflicting job keeps its result but loses the branch (the task's pull
/// request opens from the rebased one), and `land.rebase` says so. After
/// `REBASES_PER_TASK` such jobs the task is held with the conflict named,
/// as before.
fn rebase_or_hold(
    s: &hive_store::Store,
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
    if c.siblings {
        return join_or_hold(s, project, rt, key, &c, &note, now);
    }
    let Ok(conn) = s.conn() else {
        return format!("task pr {slug} {task}: {note}");
    };
    let rebases = crate::util::query_rows(
        &conn,
        "SELECT detail FROM events WHERE subject = ? AND action = 'land.rebase' AND json_valid(detail)
         AND json_extract(detail, '$.task') = ? AND coalesce(json_extract(detail, '$.join'), 0) = 0
         ORDER BY seq DESC",
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
            " - the head of open pull request #{}, which the task's pull request stacks on; your clone carries it as a branch of that name, beside the branches to merge",
            c.base.stack_on
        )
    } else {
        String::new()
    };
    let brief = format!(
        "Branch `{branch}` (job {job_id}, commit {short}) is finished work for task {task} ({title}), but it no longer merges onto `{def}`: {files} conflict with what landed on `{def}` after the branch was cut.\n\n\
         Start from the current `{def}`{which} and merge the branch into it: `git merge --no-ff {sha}`. {MERGE_RECIPE} \
         Commit on your own branch and leave `{branch}` as it is; the task's pull request opens from your branch once it merges cleanly.\n\n\
         The job's own account of its work:\n\n{result}",
        branch = c.branch,
        job_id = c.job,
        sha = c.sha,
        title = rt.title,
        result = crate::run::squeeze(&job.str("result"), 1200),
    );
    let queued = crate::jobs::add_job(
        s,
        slug.clone(),
        "task".into(),
        format!("Merge {} onto {def}: {}", c.branch, rt.title),
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
    let _ = hive_tasks::send_message(
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

/// A task whose own job branches conflict with each other - each merges
/// onto the base alone - is one job's work, not a rebase of the one that
/// came last: a `task` job on the task is queued to merge every branch of
/// the task onto the base and resolve the conflicts in the merge commits,
/// and `land.rebase` with `join` true says so. The branches stay where they
/// are: the join's branch carries them, so the next build merges it alone
/// (`carried_elsewhere`). After `JOINS_PER_TASK` joins the task is held,
/// whatever rebases ran before.
fn join_or_hold(
    s: &hive_store::Store,
    project: &Row,
    rt: &ReadyTask,
    key: &str,
    c: &Conflict,
    note: &str,
    now: &str,
) -> String {
    let slug = project.str("slug");
    let root = project.str("path");
    let task = rt.task.as_str();
    let files = c.files.join(", ");
    let onto = c.base.describe();
    let def = c.base.branch.as_str();
    let note = format!(
        "{note}; each merges onto {onto} alone, so the task's branches conflict with each other"
    );
    let Ok(conn) = s.conn() else {
        return format!("task pr {slug} {task}: {note}");
    };
    let joins = crate::util::query_rows(
        &conn,
        "SELECT detail FROM events WHERE subject = ? AND action = 'land.rebase' AND json_valid(detail)
         AND json_extract(detail, '$.task') = ? AND json_extract(detail, '$.join') = 1 ORDER BY seq DESC",
        &[&slug, &task],
    )
    .unwrap_or_default();
    drop(conn);
    if joins.len() >= JOINS_PER_TASK {
        let last: Value = serde_json::from_str(&joins[0].str("detail")).unwrap_or(Value::Null);
        let note = format!(
            "{note}; job {} already joined the task's branches once, so this one is for you",
            field(&last, "follow_up")
        );
        hold(s, now, &slug, json!({"task": task, "work": key}), &note);
        return format!("task pr {slug} {task}: held: {note}");
    }
    let which = if c.base.stacked() {
        format!(
            " - the head of open pull request #{}, which the task's pull request stacks on; your clone carries it as a branch of that name, beside the branches to merge",
            c.base.stack_on
        )
    } else {
        String::new()
    };
    // Only the branches the build merges: one another side carries already
    // (a first job's old branch after a rebuild, the ones a join merged) is
    // passed over, and merging it again meets the conflict it was left out
    // for. An empty list (no build named any) keeps every branch.
    let sides: Vec<&(String, String)> = if c.sides.is_empty() {
        rt.branches.iter().collect()
    } else {
        c.sides.iter().filter_map(|&i| rt.branches.get(i)).collect()
    };
    let mut list = String::new();
    let mut shas = Vec::new();
    let mut tier = 1;
    let mut priority = 0;
    for (job, branch) in sides.iter().copied() {
        let sha = rev(&root, &format!("refs/heads/{branch}"));
        list.push_str(&format!(
            "- `{branch}` (job {job}): `git merge --no-ff {sha}`\n"
        ));
        shas.push(sha);
        if let Ok(conn) = s.conn()
            && let Some(j) = crate::util::query_rows(
                &conn,
                "SELECT tier, priority FROM jobs WHERE id = ?",
                &[job],
            )
            .unwrap_or_default()
            .first()
        {
            tier = tier.max(j.int("tier"));
            priority = priority.max(j.int("priority"));
        }
    }
    let brief = format!(
        "Task {task} ({title}) has {n} finished job branches to bring together{passed}. Each merges onto `{def}` alone, but they conflict with each other: `{branch}` (job {job_id}) conflicts with the ones before it in {files}.\n\n\
         Start from the current `{def}`{which} and merge every branch below into it, in this order:\n\n{list}\n\
         {MERGE_RECIPE} Commit on your own branch and leave the branches as they are: your branch then carries the task's work, and the task's pull request opens from it alone.",
        title = rt.title,
        n = sides.len(),
        passed = match rt.branches.len() - sides.len() {
            0 => String::new(),
            k => format!(
                " ({k} more of its branches are carried by these already and are left out: do not merge them)"
            ),
        },
        branch = c.branch,
        job_id = c.job,
    );
    let queued = crate::jobs::add_job(
        s,
        slug.clone(),
        "task".into(),
        format!("Join the branches of {task} onto {def}: {}", rt.title),
        brief,
        tier,
        priority,
        format!("task:{task}"),
        ACTOR.into(),
        false,
    );
    let follow_up = match queued {
        Ok((j, _)) => j.str("id"),
        Err(e) => {
            let note = format!("{note}; no job could be queued to join them: {e}");
            hold(s, now, &slug, json!({"task": task, "work": key}), &note);
            return format!("task pr {slug} {task}: held: {note}");
        }
    };
    let jobs: Vec<&str> = sides.iter().map(|(j, _)| j.as_str()).collect();
    let branches: Vec<&str> = sides.iter().map(|(_, b)| b.as_str()).collect();
    event(
        s,
        now,
        "land.rebase",
        &slug,
        json!({
            "project": slug, "task": task, "job": c.job, "branch": c.branch, "sha": c.sha,
            "files": c.files, "follow_up": follow_up, "base": def, "join": true,
            "jobs": jobs, "branches": branches, "shas": shas,
        }),
    );
    let _ = hive_tasks::send_message(
        s,
        &slug,
        task,
        ACTOR,
        &format!(
            "The task's branches conflict with each other ({} with the ones before it in {files}); job {follow_up} joins them onto {onto} before the task's pull request opens.",
            c.branch
        ),
    );
    format!("task pr {slug} {task}: {note}; job {follow_up} joins them")
}

/// Fetches the upstream's `head` into `refs/remotes/<remote>/<head>` and
/// returns its sha, empty when the upstream has no such branch.
fn fetch_head(
    s: &hive_store::Store,
    root: &str,
    up: &Upstream,
    head: &str,
) -> Result<String, String> {
    let tracking = format!("refs/remotes/{}/{head}", up.remote);
    let (mut env, cleanup, why) = crate::forge::remote_credential_env(s, root.to_string(), &up.url);
    // Whether the upstream has the branch is read from git's words below,
    // and a try after a failed one decides by it. So this fetch speaks
    // git's own tongue whatever the daemon's locale: a translated git said
    // it otherwise, and every such try failed here.
    env.push("LC_ALL=C".into());
    if !why.is_empty() {
        return Err(format!("{PUSH_CREDENTIAL}{why}"));
    }
    let (fetched, https) = crate::forge::git_upstream_said(
        root,
        Duration::from_secs(120),
        &env,
        &up.url,
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
        return Err(hive_cred::redact(
            &format!(
                "fetch of {head} from {} {}: {}",
                up.name(),
                crate::forge::failed_after(https.as_deref()),
                crate::run::squeeze(&fetched.stderr, 200)
            ),
            &[],
        ));
    }
    Ok(rev(root, &tracking))
}

fn task_body(task: &str, title: &str, branches: &[(String, String)]) -> String {
    let mut b = format!("The work of Hive task {task}: {title}.\n\nJobs:\n");
    for (j, br) in branches {
        b.push_str(&format!("- {j} (`{br}`)\n"));
    }
    b.push_str("\nThe Hive reviewer reviews this pull request. Merging it closes the task; closing it unmerged sends the task back to its lead.\n");
    b
}

/// The base of a stand-alone branch's pull request and the branch's tip
/// once built on it: a pull request to open stacks on `top`, whose head is
/// merged into the branch (`take_in`); one open already keeps the base on
/// record and the branch as it is. A branch some worktree has checked out
/// is left as it is too, on the default branch: moving it would move the
/// checkout under whoever works there.
fn stacked_tip(
    s: &hive_store::Store,
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
    s: &hive_store::Store,
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
    .map_err(|why| why.note)
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
        "Job {} ({}{}), branch `{}` as the agent left it.\n\nThe Hive reviewer reviews this pull request; merging it closes the job.\n",
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
/// worked on the pull request's head (`hive review checkout`), so its tip
/// must descend from the head as the remote has it now, and the push
/// leases on that.
fn deliver_fix(
    s: &hive_store::Store,
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
    let orig = pr_event(&conn, &slug, number)?
        .ok_or_else(|| format!("no pull request #{number} of {slug} is known to Hive"))?;
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
    s: &hive_store::Store,
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
    let top = if conversation.is_some() || branch.starts_with("evolve/") {
        None
    } else {
        top
    };
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
        None => format!("Branch `{branch}` proposed by hand with `hive land --branch`.\n\n"),
    };
    if !log.trim().is_empty() {
        body.push_str("Commits:\n");
        body.push_str(&crate::util::trunc(log.trim_end(), 4000));
        body.push_str("\n\n");
    }
    body.push_str("The Hive reviewer reviews this pull request.\n");
    let mut ids = Vec::new();
    if let Some(id) = &conversation {
        ids.push(("conversation", id.clone()));
    }
    // A finished job's branch proposed by hand is that job's delivery: the
    // pull request names the job, so the job path finds it open and leaves
    // the branch alone, and merging it clears the job's branch.
    let jobs = if conversation.is_some() {
        Vec::new()
    } else {
        job_of_branch(s, &project.str("slug"), branch)
            .into_iter()
            .collect()
    };
    deliver_work(
        s,
        project,
        up,
        Work {
            kind: "proposed",
            ids,
            jobs,
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
    .map_err(|why| why.note)
}

/// The finished job whose branch `branch` is, if any.
fn job_of_branch(s: &hive_store::Store, slug: &str, branch: &str) -> Option<String> {
    let conn = s.conn().ok()?;
    let id = crate::util::query_one(
        &conn,
        "SELECT id FROM jobs WHERE project = ? AND branch = ? AND status = 'done'
         ORDER BY finished_at DESC LIMIT 1",
        &[&slug, &branch],
    )
    .ok()??
    .str("id");
    (!id.is_empty()).then_some(id)
}

/// `hive land --branch B`: pushes the branch and opens its pull request
/// (or pushes the new commits to the one open). The project needs an
/// upstream; the caller lands locally when it has none.
pub(crate) fn propose_branch(
    s: &hive_store::Store,
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
    let mut sha = rev(&root, &format!("refs/heads/{branch}"));
    // A job branch forked before {def} was squashed to a new root shares no commit
    // with it: proposed as it stands, the pull request would rewrite every file (or
    // the forge refuses it). The job's own commits after its run base are copied
    // onto {def} first, as `land_branch` does for a local landing (land.rs).
    let replayed = replay_unrelated_job_branch(s, &slug, &root, &def, branch)?;
    if let Some((tip, _)) = &replayed {
        sha = tip.clone();
    }
    let conversation = branch
        .strip_prefix("hive/chat-")
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    let conn = s.conn().map_err(|e| e.to_string())?;
    let top = if conversation.is_some() || branch.starts_with("evolve/") {
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
        now,
    )?;
    drop(conn);
    if next == Next::Nothing {
        let conn = s.conn().map_err(|e| e.to_string())?;
        let d = last_event(&conn, &slug, "head", branch, false)?
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
    let mut d = deliver_proposed(
        s,
        project,
        &up,
        branch,
        &sha,
        conversation,
        next,
        top.as_ref(),
        now,
    )?;
    if let Some((_, note)) = replayed {
        d.line = format!("{} ({note})", d.line);
    }
    Ok(d)
}

/// For `propose_branch`: when `branch` shares no history with `def` (the
/// default branch was squashed to a new root after the branch forked) and a
/// job made the branch, its commits after the job's last run base are copied
/// onto `def` in a scratch worktree and the local branch is moved to the
/// copy, so the pull request carries the job's own commits only. Returns the
/// new tip and a note, or `None` when nothing was replayed: the branch shares
/// history with `def`, no job or run base is known for it, or a worktree has
/// it checked out (moving it would move that checkout). A replay that fails
/// is an error: the branch as it stands would rewrite every file.
fn replay_unrelated_job_branch(
    s: &hive_store::Store,
    slug: &str,
    root: &str,
    def: &str,
    branch: &str,
) -> Result<Option<(String, String)>, String> {
    let git = |args: &[&str]| crate::util::git_timeout(root, Duration::from_secs(60), args);
    let def_ref = format!("refs/heads/{def}");
    let branch_ref = format!("refs/heads/{branch}");
    let shared = git(&["merge-base", &def_ref, &branch_ref]);
    if !shared.stdout.trim().is_empty() || rev(root, &def_ref).is_empty() {
        return Ok(None);
    }
    let job = {
        let conn = s.conn().map_err(|e| e.to_string())?;
        crate::util::must_one(
            &conn,
            "SELECT id FROM jobs WHERE project = ? AND branch = ? ORDER BY finished_at DESC",
            &[&slug, &branch],
        )
        .str("id")
    };
    if job.is_empty() {
        return Ok(None);
    }
    let run_base = crate::experiment::run_base(s, &crate::experiment::last_run_id(s, job.clone()));
    let fork = crate::experiment::run_base_fork(root, &branch_ref, &run_base);
    if fork.is_empty() || worktree_branches(root).iter().any(|b| b == branch) {
        return Ok(None);
    }
    let old = rev(root, &branch_ref);
    let git_dir = git(&["rev-parse", "--absolute-git-dir"])
        .stdout
        .trim()
        .to_string();
    if git_dir.is_empty() {
        return Err(format!("{root} is not a git checkout"));
    }
    let wt_path = format!("{git_dir}/hive-replay/{}", branch.replace('/', "_"));
    let _ = git(&["worktree", "remove", "--force", &wt_path]);
    let _ = std::fs::remove_dir_all(&wt_path);
    let add = git(&["worktree", "add", "--detach", &wt_path, &def_ref]);
    if !add.ok() {
        return Err(format!(
            "worktree add failed: {}",
            crate::run::squeeze(&add.stderr, 200)
        ));
    }
    let copied = crate::land::replay_from_fork(&wt_path, &fork, &branch_ref);
    let _ = git(&["worktree", "remove", "--force", &wt_path]);
    let short = crate::recipe::short(&fork, 12);
    let (tip, count) = copied.map_err(|why| {
        format!(
            "{branch} shares no history with {def}; replaying job {job}'s commits from its run base {short} failed: {why}"
        )
    })?;
    let moved = git(&["update-ref", &branch_ref, &tip, &old]);
    if !moved.ok() {
        return Err(format!(
            "{branch} could not be moved onto its replay on {def}: {}",
            crate::run::squeeze(&moved.stderr, 200)
        ));
    }
    Ok(Some((
        tip,
        format!(
            "{count} {} replayed from its run base {short} ({def} shares no history with it)",
            crate::util::plural(count as i32, "commit")
        ),
    )))
}

/// A kept experiment's branch as a pull request, from the experiment lane:
/// `body` is the measurement summary the landing note used to carry.
/// Returns (number, url, opened).
pub(crate) fn deliver_experiment(
    s: &hive_store::Store,
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
    // An experiment is based on the default branch, never on another open
    // pull request: its pull request neither stacks nor is stacked on.
    let top: Option<Value> = None;
    let conn = s.conn().map_err(|e| e.to_string())?;
    let next = next_for(
        &conn,
        &root,
        &slug,
        "job",
        &id,
        &sha,
        &stack_key(work_key(&branch, &sha), top.as_ref()),
        now,
    )?;
    drop(conn);
    if next == Next::Nothing {
        let conn = s.conn().map_err(|e| e.to_string())?;
        let d = open_pr_for_job(&conn, &slug, &id)?.ok_or_else(|| {
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
        "Experiment {id} (archive {archive_id}), measured and kept.\n\n{}\n\nJob {id} ({}, from {}), branch `{branch}`.\n\nThe Hive reviewer reviews this pull request; merging it finalizes the experiment.\n",
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
    )
    .map_err(|why| why.note)?;
    Ok((d.number, d.url, d.opened))
}

/// The pull requests of the project still open as far as Hive knows: the
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
fn poll_open(s: &hive_store::Store, project: &Row, now: &str) -> Vec<String> {
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
    let listed = match crate::landing_pr::list_prs(s, &root, &up.repo) {
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
            None => match crate::landing_pr::view_pr(s, &root, &up.repo, number) {
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
                let stacked = stacked_on_head(s, &slug, &pr) || kept_unread(s, &slug, &pr);
                lines.extend(retarget_stacked(s, project, &up, &pr, &listed, false, now));
                if stacked {
                    lines.extend(delete_kept_head(s, project, &up, &pr, &state));
                }
            }
            "CLOSED" => {
                lines.push(settle_closed(s, project, &pr, &state, now));
                lines.extend(retarget_stacked(s, project, &up, &pr, &listed, true, now));
            }
            "OPEN" => {
                // An experiment's pull request whose change is on the
                // default branch already, or that sat open too long, is
                // settled here; the rest are followed as any other.
                if experiments::is_experiment(&pr) {
                    let (settled, l) =
                        experiments::tend_open(s, project, &up, &pr, &state, &listed, now);
                    lines.extend(l);
                    if settled {
                        continue;
                    }
                }
                lines.extend(follow_open(s, project, &up, &pr, &state, now))
            }
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
    s: &hive_store::Store,
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

/// Records a pull request's head that took its base in and was pushed
/// (`merge_base_into_head`): `land.pr-refreshed`, and a `land.pr` naming
/// the new tip, so the next push of the work leases on what the pull
/// request holds.
#[allow(clippy::too_many_arguments)]
fn record_refreshed(
    s: &hive_store::Store,
    project: &Row,
    pr: &Value,
    head: &str,
    head_sha: &str,
    tip: &str,
    base: &str,
    now: &str,
) {
    let slug = project.str("slug");
    let root = project.str("path");
    let kind = kind_of(pr);
    let number = number_of(pr);
    event(
        s,
        now,
        "land.pr-refreshed",
        &slug,
        json!({"project": slug, "kind": kind, "number": number, "head": head,
            "head_sha": head_sha, "new_sha": tip, "base": base}),
    );
    let commits = crate::util::git_timeout(
        &root,
        Duration::from_secs(30),
        &["rev-list", "--count", &format!("{head_sha}..{tip}")],
    )
    .stdout
    .trim()
    .parse::<i64>()
    .unwrap_or(1);
    let mut detail = pr.clone();
    for (k, v) in [
        ("project", json!(slug)),
        ("kind", json!(kind)),
        ("from", json!(head_sha)),
        ("to", json!(tip)),
        ("commits", json!(commits)),
        ("opened", json!(false)),
        ("refreshed", json!(true)),
    ] {
        detail[k] = v;
    }
    event(s, now, "land.pr", &slug, detail);
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
    s: &hive_store::Store,
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
            record_refreshed(s, project, pr, &head, &st.head_sha, &tip, base, now);
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
    s: &hive_store::Store,
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
    let wt_path = format!("{git_dir}/hive-pr-refresh/{}", head.replace('/', "_"));
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
        "user.name=Hive",
        "-c",
        "user.email=hive@localhost",
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
    s: &hive_store::Store,
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
/// (the hub's wake loop delivers it, as it does `hive chat wake`), anything
/// else only through the pass's log line. Returns the log line's tail.
fn tell_origin(s: &hive_store::Store, pr: &Value, now: &str, text: &str) -> String {
    let slug = field(pr, "project");
    let task = field(pr, "task");
    if !task.is_empty() {
        return match hive_tasks::send_message(s, slug, task, ACTOR, text) {
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
        hive_harness::conversation::WAKE_ACTION,
        &id,
        json!({"text": text}),
    );
    format!("; conversation {id} woken")
}

/// The conversation a pull request came from (the `conversation` of its
/// `land.pr`), when it still stands where it can be told: there, and not
/// archived.
fn told_conversation(s: &hive_store::Store, pr: &Value) -> Option<String> {
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
    s: &hive_store::Store,
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
        hive_harness::conversation::FINISH_ACTION,
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
    s: &hive_store::Store,
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

/// Moves the pull requests stacked on a settled one onto its base. Merged:
/// each open pull request whose `base` is the settled head is retargeted
/// on the forge (`gh pr edit --base`) and gets a fresh `land.pr` with the
/// new base (`retargeted_from` names the old one), so delivery and the
/// reviewer see where it stands; the merge kept the settled head on the
/// upstream for them, and [`delete_kept_head`] deletes it once they moved.
/// Closed unmerged:
/// the pull requests above hold the closed one's commits in their history,
/// and retargeting them would carry the rejected work into the base
/// through the next merge, so each is rebuilt instead
/// ([`rebuild_stacked`]): its own commits on the new base, the closed
/// one's dropped, force-pushed to the same branch, then retargeted
/// (`land.pr-rebuilt`); a pull request stacked on a rebuilt one is rebuilt
/// the same way, so the whole stack above loses the commits. One the
/// rebuild cannot do cleanly stays where it is, held with the conflict
/// named (`land.pr-held`), and its origin is told. A failure of the forge
/// leaves no event, so the next pass tries again.
fn retarget_stacked(
    s: &hive_store::Store,
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
    if closed {
        let closed_sha = listed
            .get(&settled_number)
            .map(|st| st.head_sha.clone())
            .filter(|sha| !sha.is_empty())
            .unwrap_or_else(|| field_of(settled, "to"));
        return rebuild_stacked(
            s,
            project,
            up,
            &open,
            listed,
            Dropped {
                number: settled_number,
                head: head.clone(),
                sha: closed_sha,
            },
            &base,
            stack_on,
            now,
        );
    }
    let mut lines = Vec::new();
    for pr in above {
        let n = number_of(pr);
        let kind = kind_of(pr);
        // One merged or closed too settles in this very pass.
        if listed.get(&n).is_some_and(|st| st.state != "OPEN") {
            continue;
        }
        if let Err(e) = crate::landing_pr::retarget_pull_request(s, &root, &up.repo, n, &base) {
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
        lines.push(format!(
            "{kind} pr {slug}: #{n} retargeted from {head} to {base}"
        ));
    }
    lines
}

/// Whether an open pull request other than `pr` is based on `pr`'s head.
fn stacked_on_head(s: &hive_store::Store, slug: &str, pr: &Value) -> bool {
    let head = field(pr, "head");
    let n = number_of(pr);
    !head.is_empty()
        && s.conn().is_ok_and(|c| {
            open_prs(&c, slug)
                .iter()
                .any(|o| number_of(o) != n && field(o, "base") == head)
        })
}

/// Whether the merge of `pr` kept its branch because the store could not
/// say which pull requests were open (`review.merged` with `branch_kept`):
/// it is deleted here once a pass reads that none is based on it.
fn kept_unread(s: &hive_store::Store, slug: &str, pr: &Value) -> bool {
    let n = number_of(pr);
    s.conn().is_ok_and(|c| {
        crate::util::query_rows(
            &c,
            "SELECT 1 AS k FROM events WHERE subject = ? AND action = 'review.merged' AND json_valid(detail)
             AND json_extract(detail, '$.number') = ? AND json_extract(detail, '$.branch_kept') IS NOT NULL LIMIT 1",
            &[&slug, &n],
        )
        .is_ok_and(|rows| !rows.is_empty())
    })
}

/// Deletes a merged pull request's head on the upstream once no open pull
/// request is based on it any more. The merge kept it while some were
/// (`review::merge`: GitHub closes, never retargets, the pull requests
/// whose base branch an API call deletes, and a closed one never reopens);
/// [`retarget_stacked`] has just moved them. Leased on the merged head: a
/// push made since keeps the branch. One that is still based on it (its
/// retarget failed) keeps it until a later pass moves it.
fn delete_kept_head(
    s: &hive_store::Store,
    project: &Row,
    up: &Upstream,
    pr: &Value,
    st: &crate::landing_pr::PrState,
) -> Vec<String> {
    let slug = project.str("slug");
    let head = field_of(pr, "head");
    if stacked_on_head(s, &slug, pr) {
        return vec![format!(
            "pr {slug}: {head} kept on {}: a pull request is still based on it",
            up.remote
        )];
    }
    let sha = if st.head_sha.is_empty() {
        field_of(pr, "to")
    } else {
        st.head_sha.clone()
    };
    if sha.is_empty() {
        return Vec::new();
    }
    let root = project.str("path");
    vec![
        match experiments::delete_remote_branch(s, &root, up, &head, &sha) {
            Ok(()) => format!("pr {slug}: {head} deleted on {}", up.remote),
            Err(e) => format!("pr {slug}: {head} not deleted on {}: {e}", up.remote),
        },
    ]
}

/// The pull request closed unmerged whose commits the stack above must
/// lose: its number, head branch and head sha as the forge last had it.
struct Dropped {
    number: i64,
    head: String,
    sha: String,
}

/// What stands below the pull requests being rebuilt: the head branch they
/// have as their base now (`below`), the branch their own commits go onto
/// and its sha on the upstream, the pull request that branch is the head
/// of (0 for the default branch), and the sha they were built on that is
/// to go - the closed head, or the old tip of a pull request rebuilt just
/// before.
struct Onto {
    below: String,
    branch: String,
    sha: String,
    stack_on: i64,
    old_below: String,
}

/// Rebuilds every open pull request stacked on `dropped` without its
/// commits, bottom up and then up the stack: the ones whose base is the
/// closed head go onto `base` (the closed pull request's own base) and are
/// retargeted there; the ones stacked on a rebuilt pull request keep their
/// base and are rebuilt on its new tip. Each rebuilt one gets
/// `land.pr-rebuilt` and a fresh `land.pr`, and its origin is told once; a
/// conflict holds it where it is (`land.pr-held`) and tells the origin what
/// to do by hand. The pull requests above a held one are left alone: they
/// still stand on a head that holds the rejected commits, which the hold
/// note says.
#[allow(clippy::too_many_arguments)]
fn rebuild_stacked(
    s: &hive_store::Store,
    project: &Row,
    up: &Upstream,
    open: &[Value],
    listed: &BTreeMap<i64, crate::landing_pr::PrState>,
    dropped: Dropped,
    base: &str,
    stack_on: i64,
    now: &str,
) -> Vec<String> {
    let slug = project.str("slug");
    let root = project.str("path");
    let mut lines = Vec::new();
    let base_sha = match fetch_head(s, &root, up, base) {
        Ok(sha) if !sha.is_empty() => sha,
        Ok(_) => {
            lines.push(format!(
                "{slug}: the stack on #{} is not rebuilt: {base} is gone from {}",
                dropped.number,
                up.name()
            ));
            return lines;
        }
        Err(e) => {
            lines.push(format!(
                "{slug}: the stack on #{} is not rebuilt: {e}",
                dropped.number
            ));
            return lines;
        }
    };
    // The closed head stays on the forge; fetched, so the checkout knows
    // the commits it is to drop even when nothing here ever had them.
    let _ = fetch_head(s, &root, up, &dropped.head);
    let dropped_count = crate::util::git_timeout(
        &root,
        Duration::from_secs(30),
        &["rev-list", "--count", &dropped.sha, &format!("^{base_sha}")],
    )
    .stdout
    .trim()
    .parse::<i64>()
    .unwrap_or(0);
    let mut work = vec![Onto {
        below: dropped.head.clone(),
        branch: base.to_string(),
        sha: base_sha,
        stack_on,
        old_below: dropped.sha.clone(),
    }];
    // The head branches rebuilt on so far, so a stack that loops through
    // bad data cannot run forever.
    let mut seen = std::collections::BTreeSet::new();
    while let Some(onto) = work.pop() {
        let below = onto.below.clone();
        if !seen.insert(below.clone()) {
            continue;
        }
        let above: Vec<&Value> = open
            .iter()
            .filter(|pr| {
                number_of(pr) != dropped.number
                    && field(pr, "base") == below
                    && listed
                        .get(&number_of(pr))
                        .is_none_or(|st| st.state == "OPEN")
            })
            .collect();
        for pr in above {
            let n = number_of(pr);
            let kind = kind_of(pr);
            let head = field_of(pr, "head");
            let url = field_of(pr, "url");
            let retarget = below != onto.branch;
            match rebuild_branch(s, &root, up, &head, &onto) {
                Ok(built) => {
                    let retargeted = if retarget {
                        crate::landing_pr::retarget_pull_request(
                            s,
                            &root,
                            &up.repo,
                            n,
                            &onto.branch,
                        )
                    } else {
                        Ok(())
                    };
                    if let Err(e) = retargeted {
                        lines.push(format!(
                            "{kind} pr {slug}: #{n} rebuilt on {} without #{}'s commits but not retargeted from {below} to {}: {e}",
                            onto.branch, dropped.number, onto.branch
                        ));
                    }
                    let mut d = pr.clone();
                    d["base"] = json!(onto.branch);
                    if onto.stack_on != 0 {
                        d["stack_on"] = json!(onto.stack_on);
                    } else if let Some(o) = d.as_object_mut() {
                        o.remove("stack_on");
                    }
                    d["from"] = json!(onto.sha);
                    d["to"] = json!(built.tip);
                    d["commits"] = json!(built.own);
                    d["opened"] = json!(false);
                    d["rebuilt"] = json!(true);
                    if retarget {
                        d["retargeted_from"] = json!(below);
                    }
                    event(s, now, "land.pr", &slug, d);
                    event(
                        s,
                        now,
                        "land.pr-rebuilt",
                        &slug,
                        json!({"project": slug, "kind": kind, "number": n, "url": url, "head": head,
                            "old_base": below, "base": onto.branch, "from": built.was, "to": built.tip,
                            "commits": built.own,
                            "dropped": {"number": dropped.number, "commits": dropped_count}}),
                    );
                    let told = tell_origin(
                        s,
                        pr,
                        now,
                        &format!(
                            "Pull request #{n} ({url}) was stacked on #{}, which was closed without merging. \
                             {head} was rebuilt on {} from its own {} commit{} without #{}'s {dropped_count}, and the pull request targets {} now; its head is {}. \
                             The project's check runs on it again, and the reviewer reads it again.",
                            dropped.number,
                            onto.branch,
                            built.own,
                            if built.own == 1 { "" } else { "s" },
                            dropped.number,
                            onto.branch,
                            crate::recipe::short(&built.tip, 12)
                        ),
                    );
                    lines.push(format!(
                        "{kind} pr {slug}: #{n} rebuilt on {} without #{}'s commits ({} of its own, {dropped_count} dropped), head now {}{told}",
                        onto.branch,
                        dropped.number,
                        built.own,
                        crate::recipe::short(&built.tip, 12)
                    ));
                    // The ones stacked on this one are rebuilt on its new
                    // tip, dropping its old history the same way.
                    work.push(Onto {
                        below: head.clone(),
                        branch: head.clone(),
                        sha: built.tip,
                        stack_on: n,
                        old_below: built.was,
                    });
                }
                Err(Rebuild::Conflict { was, files }) => {
                    let list = files.join(", ");
                    let mut d = json!({"kind": kind, "number": n, "url": url, "head": head,
                        "head_sha": was, "base": onto.branch, "closed": dropped.number, "files": files,
                        "work": format!("{head} {} rebuild", crate::recipe::short(&was, 12))});
                    for key in ["task", "job", "experiment", "conversation"] {
                        let v = field_of(pr, key);
                        if !v.is_empty() {
                            d[key] = json!(v);
                        }
                    }
                    let note = format!(
                        "#{n} was stacked on #{}, which was closed unmerged; rebuilding {head} on {} without #{}'s commits conflicts in {list}",
                        dropped.number, onto.branch, dropped.number
                    );
                    hold(s, now, &slug, d, &note);
                    let told = tell_origin(
                        s,
                        pr,
                        now,
                        &format!(
                            "Pull request #{n} ({url}) was stacked on #{}, which was closed without merging, so #{n} carries #{}'s rejected commits. \
                             Rebuilding {head} on {} without them conflicts in {list}, so the pull request was left as it is and still targets {below}. \
                             Start a branch from {}, merge #{n}'s head ({}) into it with `git merge --no-ff` - not a copy of its commits, which the landing refuses as work the worker did not write - and take out what #{} added (its commits up to {}) in that merge or a commit of your own, keeping the intent of #{n}'s own work; run the project's check, force-push it to {head} and retarget the pull request to {}; the pull request follows the branch.",
                            dropped.number,
                            dropped.number,
                            onto.branch,
                            onto.branch,
                            crate::recipe::short(&was, 12),
                            dropped.number,
                            crate::recipe::short(&onto.old_below, 12),
                            onto.branch
                        ),
                    );
                    lines.push(format!(
                        "{kind} pr {slug}: #{n} not rebuilt on {} without #{}'s commits: conflicts in {list}; held{told}",
                        onto.branch, dropped.number
                    ));
                }
                Err(Rebuild::Other(e)) => lines.push(format!(
                    "{kind} pr {slug}: #{n} not rebuilt on {} without #{}'s commits: {e}",
                    onto.branch, dropped.number
                )),
            }
        }
    }
    lines
}

/// A branch rebuilt without the commits below it.
struct Rebuilt {
    /// The head as the upstream had it before.
    was: String,
    /// The new tip, pushed.
    tip: String,
    /// Its own commits on the new base.
    own: i64,
}

enum Rebuild {
    /// The commits do not go onto the new base cleanly; nothing moved.
    Conflict {
        was: String,
        files: Vec<String>,
    },
    Other(String),
}

impl From<String> for Rebuild {
    fn from(e: String) -> Self {
        Rebuild::Other(e)
    }
}

/// Rebuilds `head` on `onto`: the branch as the upstream has it, fetched,
/// its commits past where it forked from `onto.old_below` rebased onto
/// `onto.sha` in a scratch worktree of the checkout (`git rebase --onto`:
/// the merge commits that took the stack in go, and so does a patch the
/// base has already), the upstream's branch pushed there leasing on the
/// head fetched, and the local branch moved to the result; the worktree
/// goes whatever happens.
fn rebuild_branch(
    s: &hive_store::Store,
    root: &str,
    up: &Upstream,
    head: &str,
    onto: &Onto,
) -> Result<Rebuilt, Rebuild> {
    let was = fetch_head(s, root, up, head)?;
    if was.is_empty() {
        return Err(format!("{head} is gone from {}", up.name()).into());
    }
    let git = |args: &[&str]| crate::util::git_timeout(root, Duration::from_secs(120), args);
    // Where the branch left what is below it: its commits past that point
    // are its own. The branch may stand on an older tip of the one below
    // than the forge last had, so the fork point is the cut, not that tip.
    let cut = git(&["merge-base", &onto.old_below, &was])
        .stdout
        .trim()
        .to_string();
    if cut.is_empty() {
        return Err(format!(
            "{head} at {} shares no history with {} to drop it",
            crate::recipe::short(&was, 12),
            crate::recipe::short(&onto.old_below, 12)
        )
        .into());
    }
    let git_dir = git(&["rev-parse", "--absolute-git-dir"])
        .stdout
        .trim()
        .to_string();
    if git_dir.is_empty() {
        return Err(format!("{root} is not a git checkout").into());
    }
    let wt_path = format!("{git_dir}/hive-stack/{}", head.replace('/', "_"));
    let _ = git(&["worktree", "remove", "--force", &wt_path]);
    let _ = std::fs::remove_dir_all(&wt_path);
    let add = git(&["worktree", "add", "--detach", &wt_path, &was]);
    if !add.ok() {
        return Err(format!(
            "worktree add failed: {}",
            crate::run::squeeze(&add.stderr, 200)
        )
        .into());
    }
    let in_wt = |args: &[&str]| crate::util::git_timeout(&wt_path, Duration::from_secs(300), args);
    let rebase = |merges: bool| {
        let mut args = vec![
            "-c",
            "user.name=Hive",
            "-c",
            "user.email=hive@localhost",
            "rebase",
        ];
        if merges {
            args.push("--rebase-merges");
        }
        args.extend(["--onto", onto.sha.as_str(), cut.as_str()]);
        in_wt(&args)
    };
    let unmerged = || -> Vec<String> {
        in_wt(&["diff", "--name-only", "--diff-filter=U"])
            .stdout
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect()
    };
    let mut r = rebase(false);
    // A branch a join built holds the join's resolutions in merge commits,
    // which a plain rebase drops: it replays the sides one by one into the
    // conflict the join resolved. Such a branch is replayed again with its
    // merges kept, and when a kept merge meets that conflict once more (its
    // first parent moved), the branch's own change is taken whole - its
    // range merged onto the new base from the cut, which carries the
    // resolutions - before the conflict is the task's.
    let mut conflicted = Vec::new();
    if !r.ok() {
        conflicted = unmerged();
        let _ = in_wt(&["rebase", "--abort"]);
        let merges = git(&["rev-list", "--merges", "--count", &format!("{cut}..{was}")])
            .stdout
            .trim()
            .parse::<i64>()
            .unwrap_or(0);
        if merges > 0 {
            let kept = rebase(true);
            if kept.ok() {
                r = kept;
            } else {
                let _ = in_wt(&["rebase", "--abort"]);
                let _ = in_wt(&["checkout", "-q", "--detach", &was]);
                let whole = replay_whole(&wt_path, head, onto, &cut, &was);
                if whole.ok() {
                    r = whole;
                }
            }
        }
    }
    let result = if r.ok() {
        let tip = in_wt(&["rev-parse", "HEAD"]).stdout.trim().to_string();
        let own = in_wt(&["rev-list", "--count", &format!("{}..{tip}", onto.sha)])
            .stdout
            .trim()
            .parse::<i64>()
            .unwrap_or(0);
        if tip.is_empty() {
            Err(Rebuild::Other(format!(
                "{head} has no tip after the rebuild"
            )))
        } else {
            Ok((tip, own))
        }
    } else {
        let _ = in_wt(&["rebase", "--abort"]);
        let unmerged = conflicted;
        Err(if unmerged.is_empty() {
            Rebuild::Other(format!(
                "rebase of {head} onto {} failed: {}",
                onto.branch,
                crate::run::squeeze(&format!("{} {}", r.stdout, r.stderr), 200)
            ))
        } else {
            Rebuild::Conflict {
                was: was.clone(),
                files: unmerged,
            }
        })
    };
    let _ = git(&["worktree", "remove", "--force", &wt_path]);
    let (tip, own) = result?;
    push_rev(s, root, up, &tip, head, Some(&was))?;
    let _ = git(&["update-ref", &format!("refs/heads/{head}"), &tip]);
    Ok(Rebuilt { was, tip, own })
}

/// Replays a branch's own change, `cut..was`, onto `onto` as one commit:
/// the three-way merge of `was` onto the new base with the cut as its
/// base, so the resolutions the branch's merge commits hold stay. Leaves
/// the worktree on the new commit, or fails and leaves it as it was when
/// the change does not merge onto the new base.
fn replay_whole(
    wt: &str,
    head: &str,
    onto: &Onto,
    cut: &str,
    was: &str,
) -> hive_hand_core::util::GitResult {
    let git = |args: &[&str]| crate::util::git_timeout(wt, Duration::from_secs(300), args);
    let mt = git(&[
        "merge-tree",
        "--write-tree",
        "--merge-base",
        cut,
        &onto.sha,
        was,
    ]);
    let tree = mt.stdout.lines().next().unwrap_or("").trim().to_string();
    if !mt.ok() || tree.is_empty() {
        return mt;
    }
    let message = format!(
        "{head}: its own change rebuilt on {} without what was below it\n\nWas {was}, cut from {cut}; the merges on it are kept in this one commit.",
        onto.branch
    );
    let commit = git(&[
        "-c",
        "user.name=Hive",
        "-c",
        "user.email=hive@localhost",
        "commit-tree",
        &tree,
        "-p",
        &onto.sha,
        "-m",
        &message,
    ]);
    let sha = commit.stdout.trim().to_string();
    if !commit.ok() || sha.is_empty() {
        return commit;
    }
    git(&["checkout", "-q", "--detach", &sha])
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
fn clear_branches(s: &hive_store::Store, slug: &str, pr: &Value) {
    let Ok(conn) = s.conn() else {
        return;
    };
    let task = field(pr, "task");
    if !task.is_empty() {
        let (exact, like) = hive_store::task_source_params(task);
        let _ = conn.execute(
            &format!(
                "UPDATE jobs SET branch = '' WHERE project = ? AND status = 'done' AND {}",
                hive_store::task_source_clause("source")
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

/// The transition a merged pull request made on the default branch: the
/// merge commit that has the head the forge merged (`to`) as a parent, as
/// its first parent and itself; else, when the history no longer holds that
/// merge (a squashed main, a squash merge), the base the pull request was
/// first built on (`from` of its first `land.pr`) and the head.
fn merged_transition(root: &str, def: &str, first_from: &str, to: &str) -> (String, String) {
    if to.is_empty() {
        return (String::new(), String::new());
    }
    for tip in [
        format!("refs/heads/{def}"),
        format!("refs/remotes/origin/{def}"),
    ] {
        let out = crate::util::git_timeout(
            root,
            Duration::from_secs(30),
            &[
                "rev-list",
                "--merges",
                "--parents",
                "--ancestry-path",
                "--reverse",
                &format!("{to}..{tip}"),
            ],
        );
        if !out.ok() {
            continue;
        }
        for line in out.stdout.lines() {
            let ids: Vec<&str> = line.split_whitespace().collect();
            if ids.len() >= 3 && ids[2..].contains(&to) {
                return (ids[1].to_string(), ids[0].to_string());
            }
        }
    }
    (first_from.to_string(), to.to_string())
}

/// The changed-file evidence of a merged pull request (see
/// [`merged_transition`]), with the snapshot of the job that did the work
/// and the commit that carried it. A diff the checkout cannot read any more
/// is recorded as unavailable with its reason, never as zero.
fn merge_evidence(
    s: &hive_store::Store,
    project: &Row,
    first_from: &str,
    pr: &Value,
) -> (Value, Value, String) {
    let slug = project.str("slug");
    let root = project.str("path");
    let def = crate::baseline::default_branch(project.clone());
    let (before, after) = merged_transition(&root, &def, first_from, field(pr, "to"));
    let ctx = hive_bus::CancellationToken::new();
    let evidence =
        crate::landing_evidence::landing_code_delivery_evidence(&ctx, root, before, after.clone());
    let evidence = serde_json::to_value(evidence).unwrap_or(Value::Null);
    let job = jobs_of(pr)
        .last()
        .and_then(|id| {
            let conn = s.conn().ok()?;
            crate::util::query_one(
                &conn,
                "SELECT * FROM jobs WHERE id = ? AND project = ?",
                &[id, &slug],
            )
            .ok()
            .flatten()
        })
        .unwrap_or_default();
    let work = crate::landing_evidence::landing_work_snapshot(
        s,
        project.clone(),
        job,
        crate::land::LandOpts {
            run_id: field_of(pr, "run_id"),
            ..Default::default()
        },
    );
    let work = if work.job.is_some() || work.worker.is_some() {
        serde_json::to_value(work).unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    (evidence, work, after)
}

/// Merged pull requests whose changed files one pass gathers at most per
/// project: each costs a `rev-list` and two bounded `git diff`s.
const EVIDENCE_PER_PASS: usize = 10;

/// Gathers the changed files of the project's merged pull requests into a
/// `land.evidence` naming the merge (`merged_seq`), newest first, once each,
/// found or not. It runs right after [`poll_open`] settled the merges and
/// moved the checkout, so a merge is counted by the Code delivered report
/// within the pass that saw it; the merges recorded before this existed are
/// worked through the same way, a few a pass. Without it the report counted
/// none of the pull-request lane's merges (TASK-085).
fn backfill_merge_evidence(s: &hive_store::Store, project: &Row, now: &str) -> Vec<String> {
    let slug = project.str("slug");
    let Ok(conn) = s.conn() else {
        return Vec::new();
    };
    let merged = crate::util::query_rows(
        &conn,
        "SELECT m.seq, m.detail FROM events m
         WHERE m.action = 'land.merged' AND m.subject = ?1 AND json_valid(m.detail)
           AND json_extract(m.detail, '$.number') IS NOT NULL
           AND json_extract(m.detail, '$.code_delivery') IS NULL
           AND NOT EXISTS (SELECT 1 FROM events e WHERE e.action = ?2 AND e.subject = ?1
             AND json_valid(e.detail) AND json_extract(e.detail, '$.merged_seq') = m.seq)
         ORDER BY m.seq DESC LIMIT ?3",
        &[
            &slug,
            &hive_codedelivery::MERGE_EVIDENCE_ACTION,
            &(EVIDENCE_PER_PASS as i64),
        ],
    )
    .unwrap_or_default();
    if merged.is_empty() {
        return Vec::new();
    }
    let mut readable = 0;
    for m in &merged {
        let seq = i64::from(m.int("seq"));
        let merged: Value = serde_json::from_str(&m.str("detail")).unwrap_or(Value::Null);
        let number = number_of(&merged);
        let prs: Vec<Value> = crate::util::query_rows(
            &conn,
            "SELECT detail FROM events WHERE action = 'land.pr' AND subject = ? AND json_valid(detail)
             AND json_extract(detail, '$.number') = ? AND seq < ? ORDER BY seq",
            &[&slug, &number, &seq],
        )
        .unwrap_or_default()
        .iter()
        .filter_map(|r| serde_json::from_str::<Value>(&r.str("detail")).ok())
        .collect();
        let first_from = prs.first().map(|d| field_of(d, "from")).unwrap_or_default();
        let pr = prs.last().cloned().unwrap_or_else(|| merged.clone());
        let (evidence, work, sha) = merge_evidence(s, project, &first_from, &pr);
        if field(&evidence, "evidence") != hive_codedelivery::EVIDENCE_UNAVAILABLE {
            readable += 1;
        }
        let mut detail = json!({"project": slug, "number": number, "merged_seq": seq,
            "code_delivery": evidence});
        if !work.is_null() {
            detail["work"] = work;
        }
        if !sha.is_empty() {
            detail["sha"] = json!(sha);
        }
        crate::util::event_at(
            &conn,
            now,
            ACTOR,
            hive_codedelivery::MERGE_EVIDENCE_ACTION,
            &slug,
            detail,
        );
    }
    vec![format!(
        "{slug}: gathered the changed files of {} merged pull request{} ({readable} readable)",
        merged.len(),
        if merged.len() == 1 { "" } else { "s" }
    )]
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
    s: &hive_store::Store,
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
        if let Err(e) = hive_tasks::set_status(s, task, "done", ACTOR) {
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
    let merged_at = if st.merged_at.is_empty() {
        String::new()
    } else {
        format!(" at {}", st.merged_at)
    };
    if let Some(note) = answer_merge_question(
        s,
        &slug,
        number,
        &format!("Pull request #{number} was merged{merged_at}; nothing is left to decide."),
        now,
    ) {
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
/// (`hive_evolve::finalization`) reads exactly that.
fn finalize_experiment(
    s: &hive_store::Store,
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
/// branches, a task's `hive/task-<id>` - so the checkout does not fill
/// with branches whose commits are on the default branch. A branch some
/// worktree has checked out (the hub's chat worktrees) stays, with a note.
fn delete_delivered_branches(
    s: &hive_store::Store,
    slug: &str,
    root: &str,
    pr: &Value,
) -> Vec<String> {
    let mut branches: Vec<String> = Vec::new();
    let task = field(pr, "task");
    if !task.is_empty() {
        branches.push(task_branch(task));
        if let Ok(conn) = s.conn() {
            let (exact, like) = hive_store::task_source_params(task);
            for j in crate::util::query_rows(
                &conn,
                &format!(
                    "SELECT branch FROM jobs WHERE project = ? AND status = 'done' AND branch != '' AND {}",
                    hive_store::task_source_clause("source")
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

/// The owner's open "Merge pull request #N...?" questions
/// (`review::hand_to_owner`) of `slug`: marked for the owner, with no
/// answer or result naming them.
fn open_merge_questions(s: &hive_store::Store, slug: &str, number: i64) -> Vec<Row> {
    let Ok(conn) = s.conn() else {
        return Vec::new();
    };
    let exact = format!("Merge pull request #{number}?");
    let titled = format!("Merge pull request #{number}: ");
    crate::util::query_rows(
        &conn,
        "SELECT * FROM posts q WHERE q.project = ? AND q.kind = 'question'
         AND q.summary LIKE 'Merge pull request #%'
         AND NOT EXISTS (SELECT 1 FROM posts r WHERE r.on_post = q.id AND r.kind IN ('answer', 'result'))
         ORDER BY q.date, q.id",
        &[&slug],
    )
    .unwrap_or_default()
    .into_iter()
    .filter(|q| {
        let summary = q.str("summary");
        (summary == exact || summary.starts_with(&titled))
            && hive_store::post_for_owner(&q.str("extra")).unwrap_or(false)
    })
    .collect()
}

/// A settled pull request answers the owner's open merge question for it:
/// an `answer` naming the question, resolution `answered`, the outcome in
/// one line. The answer goes on a thread of its own (`review-pr-N` in the
/// question's area), not on the question's: `runner/review` holds every
/// pull request's question, and a later answer or a resolution there would
/// settle the ones still waiting too. Returns the log note, None when no
/// question was open.
fn answer_merge_question(
    s: &hive_store::Store,
    slug: &str,
    number: i64,
    outcome: &str,
    now: &str,
) -> Option<String> {
    let mut answered = 0;
    for q in open_merge_questions(s, slug, number) {
        let id = q.str("id");
        let posted = crate::board::create_post(
            s,
            now.to_string(),
            crate::board::PostOpts {
                kind: "answer".into(),
                project: slug.to_string(),
                area: q.str("area"),
                thread: format!("{}-pr-{number}", q.str("thread")),
                summary: crate::util::trunc(outcome, 230),
                body: format!("{outcome}\nre: {id}\n"),
                resolution: "answered".into(),
                on: id,
                agent: crate::board::actor_agent(s, ACTOR.to_string()),
                date: now.to_string(),
                ..Default::default()
            },
        );
        if posted.is_ok() {
            answered += 1;
        }
    }
    (answered > 0).then(|| "the owner's merge question answered".to_string())
}

fn settle_closed(
    s: &hive_store::Store,
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
    let mut told = note_conversation_closed(s, pr, number, &url)
        .map(|note| format!("; {note}"))
        .unwrap_or_default();
    if let Some(note) = answer_merge_question(
        s,
        &slug,
        number,
        &format!("Pull request #{number} was closed without merging; nothing is left to merge."),
        now,
    ) {
        told.push_str(&format!("; {note}"));
    }
    let task = field(pr, "task");
    if task.is_empty() {
        return format!(
            "{kind} pr {slug}: #{number} closed unmerged; the branch stays on the remote{told}"
        );
    }
    let _ = hive_tasks::set_status(s, task, "in_progress", ACTOR);
    let _ = hive_tasks::send_message(
        s,
        &slug,
        task,
        ACTOR,
        &format!(
            "Pull request #{number} ({url}) was closed without merging. The task is back in progress: its lead reads why on the pull request and plans what comes next; the branch {} stays on the remote.",
            task_branch(task)
        ),
    );
    format!("task pr {slug} {task}: #{number} closed unmerged; task back in progress{told}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::*;
    use crate::test_helpers_daemon::*;

    #[test]
    fn the_pass_clears_the_hold_note_the_land_lane_left_and_says_so_once() {
        let s = new_store();
        s.set_meta(
            LANDING_HOLDS_KEY,
            r#"{"amistio-template":{"at":"2026-11-04T13:37:39Z","note":"1 pending landing job; deferred: main moved while the merge was verified; nothing landed, try again"}}"#,
        )
        .expect("old hold");
        let lines = tend(&s, &[], TEST_NOW);
        assert_eq!(
            lines,
            vec![
                "landing: cleared the hold note for amistio-template the retired land lane left behind; work is delivered as pull requests".to_string()
            ]
        );
        assert!(
            !s.meta(LANDING_HOLDS_KEY).expect("meta").1,
            "the note is gone, so the console shows no hold"
        );
        assert!(tend(&s, &[], TEST_NOW).is_empty(), "nothing to say twice");
    }

    fn last_seq(s: &hive_store::Store, action: &str) -> i64 {
        let conn = s.conn().unwrap();
        conn.query_row(
            "SELECT MAX(seq) FROM events WHERE action = ?",
            [action],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn a_merge_commit_names_what_the_pull_request_changed_on_main() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        let (_proj, repo) = new_repo(&s);
        let base = ref_of(&repo, "refs/heads/main");
        run(
            &repo,
            &["git", "checkout", "-q", "-b", "hive/w/code", "main"],
        );
        std::fs::create_dir_all(std::path::Path::new(&repo).join("src")).unwrap();
        std::fs::write(
            std::path::Path::new(&repo).join("src/lib.rs"),
            "pub fn f() {}\n",
        )
        .unwrap();
        run(&repo, &["git", "add", "-A"]);
        run(&repo, &["git", "commit", "-q", "-m", "code"]);
        let head = ref_of(&repo, "refs/heads/hive/w/code");
        run(&repo, &["git", "checkout", "-q", "main"]);
        std::fs::write(std::path::Path::new(&repo).join("NOTES.md"), "other work\n").unwrap();
        run(&repo, &["git", "add", "-A"]);
        run(&repo, &["git", "commit", "-q", "-m", "other"]);
        let other = ref_of(&repo, "refs/heads/main");
        run(
            &repo,
            &[
                "git",
                "merge",
                "-q",
                "--no-ff",
                "-m",
                "Merge pull request #9",
                "hive/w/code",
            ],
        );
        let merge = ref_of(&repo, "refs/heads/main");

        // The merge commit's own change, not the base the pull request
        // was built on: the other work merged in between is not its.
        assert_eq!(
            merged_transition(&repo, "main", &base, &head),
            (other.clone(), merge.clone())
        );
        let ctx = hive_bus::CancellationToken::new();
        let e = crate::landing_evidence::landing_code_delivery_evidence(
            &ctx,
            repo.clone(),
            other,
            merge,
        );
        assert_eq!(e.code_files, 1);
        assert_eq!(e.code_paths, vec!["src/lib.rs".to_string()]);

        // A head the history does not hold as a merged parent (a squashed
        // main): the pull request's own base to its head.
        assert_eq!(
            merged_transition(&repo, "main", &base, &base),
            (base.clone(), base)
        );
        assert_eq!(
            merged_transition(&repo, "main", "", ""),
            (String::new(), String::new())
        );
    }

    /// A task with two done jobs on their own branches.
    fn task_with_two_branches(s: &hive_store::Store, proj: &Row, repo: &str) -> String {
        task_with_branches(
            s,
            proj,
            repo,
            "Add the thing",
            &["hive/worker/a", "hive/worker/b"],
        )
    }

    /// A task in progress with one done job per branch named.
    fn task_with_branches(
        s: &hive_store::Store,
        proj: &Row,
        repo: &str,
        title: &str,
        names: &[&str],
    ) -> String {
        let task = hive_tasks::new(s, "proj", title, "b", 0, hive_store::HUMAN_AGENT).unwrap();
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

    fn status(s: &hive_store::Store, id: &str) -> String {
        hive_tasks::get(s, id).unwrap().0.status
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

    fn events(s: &hive_store::Store, action: &str) -> i32 {
        count(
            s,
            "SELECT COUNT(*) AS n FROM events WHERE action = ?",
            &[&action],
        )
    }

    fn tend_proj(s: &hive_store::Store, proj: &Row, now: &str) -> Vec<String> {
        crate::forge::forget_sync_memo();
        tend(s, std::slice::from_ref(proj), now)
    }

    #[test]
    fn a_finished_task_opens_one_pull_request_and_is_done_once_merged() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
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
        for b in ["hive/worker/a", "hive/worker/b"] {
            let tip = ref_of(&repo, &format!("refs/heads/{b}"));
            run(&repo, &["git", "merge-base", "--is-ancestor", &tip, &head]);
        }
        let d = hive_tasks::delivery_for(&s, &task).unwrap();
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
        let d = hive_tasks::delivery_for(&s, &task).unwrap();
        assert_eq!(d.state, "merged", "{d:?}");
        assert_eq!(d.waiting, 0, "the jobs' branches are cleared");
        assert_eq!(
            ref_of(&repo, "refs/heads/main"),
            head,
            "main fast-forwarded"
        );
        for b in ["hive/worker/a", "hive/worker/b", &task_branch(&task)] {
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
        // The pass that saw the merge gathered what it changed, so the Code
        // delivered report counts it (TASK-085): the two branch files,
        // from the base the pull request was built on to the merged head.
        assert!(
            lines
                .iter()
                .any(|l| l
                    .contains("gathered the changed files of 1 merged pull request (1 readable)")),
            "{lines:?}"
        );
        let evidence = last_detail(&s, hive_codedelivery::MERGE_EVIDENCE_ACTION);
        assert_eq!(evidence["merged_seq"], last_seq(&s, "land.merged"));
        assert_eq!(evidence["number"], 7);
        assert_eq!(evidence["sha"], head);
        assert_eq!(evidence["code_delivery"]["before"], main_before);
        assert_eq!(evidence["code_delivery"]["files_changed"], 2);
        assert_eq!(evidence["work"]["job"]["source"], format!("task:{task}"));
        // Settled: no more polls.
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(lines.is_empty(), "{lines:?}");
    }

    /// The owner's merge question for pull request `n`, as
    /// `review::hand_to_owner` asks it.
    fn merge_question(s: &hive_store::Store, n: i64, title: &str) -> String {
        crate::board::create_post(
            s,
            TEST_NOW.to_string(),
            crate::board::PostOpts {
                kind: "question".into(),
                project: "proj".into(),
                area: "runner".into(),
                thread: "review".into(),
                summary: format!("Merge pull request #{n}: {title}?"),
                body: format!("Pull request #{n}: {title}\n"),
                agent: crate::board::actor_agent(s, ACTOR.to_string()),
                for_owner: true,
                ..Default::default()
            },
        )
        .expect("merge question")
        .str("id")
    }

    /// The answers naming post `id`, as `resolution|summary`.
    fn answers_on(s: &hive_store::Store, id: &str) -> Vec<String> {
        must_rows(
            s,
            "SELECT resolution, summary FROM posts WHERE kind = 'answer' AND on_post = ?",
            &[&id],
        )
        .iter()
        .map(|r| format!("{}|{}", r.str("resolution"), r.str("summary")))
        .collect()
    }

    #[test]
    fn a_merged_pull_request_answers_its_owner_question() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let task = task_with_two_branches(&s, &proj, &repo);
        tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(last_detail(&s, "land.pr")["number"], 7);
        let asked = merge_question(&s, 7, "Add the thing");
        // Another pull request's question, and one whose number only starts
        // like it, still wait for the owner.
        let other = merge_question(&s, 8, "Something else");
        let longer = merge_question(&s, 70, "Seventy");

        merge_on_forge(&gh, &bare, 7);
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains("#7 merged; task done")
                && l.contains("the owner's merge question answered")),
            "{lines:?}"
        );
        assert_eq!(status(&s, &task), "done");
        assert_eq!(
            answers_on(&s, &asked),
            [
                "answered|Pull request #7 was merged at 2026-11-04T07:00:00Z; nothing is left to decide."
            ]
        );
        assert!(answers_on(&s, &other).is_empty());
        assert!(answers_on(&s, &longer).is_empty());
        // The answer settles the question where the owner reads it, and
        // leaves the thread the other questions wait on open.
        let open = crate::plan::open_questions(&s, "proj".into(), TEST_NOW.into());
        let open: Vec<String> = open.iter().map(|q| q.str("id")).collect();
        assert!(!open.contains(&asked), "{open:?}");
        assert!(open.contains(&other) && open.contains(&longer), "{open:?}");
        // Settled once: the next pass answers nothing again.
        tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(answers_on(&s, &asked).len(), 1);
    }

    #[test]
    fn a_pull_request_closed_unmerged_answers_its_owner_question() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, _bare) = push_fixture(&s);
        task_with_two_branches(&s, &proj, &repo);
        tend_proj(&s, &proj, TEST_NOW);
        let asked = merge_question(&s, 7, "Add the thing");

        gh.set_state(7, "CLOSED");
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains("closed unmerged")
                && l.contains("the owner's merge question answered")),
            "{lines:?}"
        );
        assert_eq!(
            answers_on(&s, &asked),
            ["answered|Pull request #7 was closed without merging; nothing is left to merge."]
        );
    }

    #[test]
    fn a_follow_up_job_is_pushed_onto_the_task_pull_request_with_what_the_forge_holds() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
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
        // request's head as `hive review checkout` leaves it.
        run(&repo, &["git", "branch", "hand", &head]);
        let hand = commit_on(&repo, "hand", "hand.txt");
        run(
            &repo,
            &["git", "push", "-q", &bare, &format!("hand:{head}")],
        );
        run(&repo, &["git", "branch", "-D", "hand"]);
        let fix = push_branch(&s, &proj, &repo, "hive/worker/c");
        run(&repo, &["git", "branch", "-f", "hive/worker/c", &head]);
        let c_tip = commit_on(&repo, "hive/worker/c", "c.txt");
        exec1(
            &s,
            "UPDATE jobs SET source = ? WHERE id = ?",
            &[&format!("task:{task}"), &fix.str("id")],
        );
        assert!(
            dry_run(&s, &proj, TEST_NOW)
                .iter()
                .any(|l| l.contains("hive/worker/c") && l.contains("push onto pull request #7")),
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
        assert_eq!(hive_tasks::delivery_for(&s, &task).unwrap().waiting, 3);

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
        assert_eq!(hive_tasks::delivery_for(&s, &task).unwrap().waiting, 0);
        assert!(rev(&repo, "refs/heads/hive/worker/c").is_empty());
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
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, _bare) = push_fixture(&s);
        let task = task_with_two_branches(&s, &proj, &repo);
        tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(
            hive_tasks::delivery_for(&s, &task).unwrap().state,
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
        let d = hive_tasks::delivery_for(&s, &task).unwrap();
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
    fn task_messages(s: &hive_store::Store, task: &str, like: &str) -> i32 {
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
        let _env = hive_testutil::env_lock();
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
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        gh.track_remote(&bare);
        let task = task_with_two_branches(&s, &proj, &repo);
        let head = task_branch(&task);
        tend_proj(&s, &proj, TEST_NOW);
        let first = bare_ref(&bare, &format!("refs/heads/{head}"));

        // The base rewrites the file a job branch wrote.
        move_base_on_forge(&repo, &bare, "hive_worker_a.txt");
        gh.set_mergeable(7, "CONFLICTING");
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains(&format!(
                "task pr proj: #7 conflicts with main at {}: hive_worker_a.txt; main does not merge in cleanly; task {task} told",
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
        assert_eq!(conflict["files"], json!(["hive_worker_a.txt"]));
        assert_eq!(task_messages(&s, &task, "conflicts with main"), 1);
        assert_eq!(task_messages(&s, &task, "hive_worker_a.txt"), 1);
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
        let _env = hive_testutil::env_lock();
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
        let _env = hive_testutil::env_lock();
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
            hive_tasks::delivery_for(&s, &task).unwrap().state,
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
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let job = push_branch(&s, &proj, &repo, "hive/sec/x");
        let id = job.str("id");
        exec1(
            &s,
            "UPDATE jobs SET kind = 'security', source = 'security:proj:2026-09-06', result = 'Tightened the thing.' WHERE id = ?",
            &[&id],
        );
        let tip = ref_of(&repo, "refs/heads/hive/sec/x");

        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(
            lines,
            [format!(
                "job pr proj job {id}: #7 opened https://github.com/o/r/pull/7"
            )]
        );
        assert_eq!(bare_ref(&bare, "refs/heads/hive/sec/x"), tip);
        assert_eq!(gh.head_of(7), "hive/sec/x");
        assert_eq!(gh.title_of(7), "t hive/sec/x");
        let pr = last_detail(&s, "land.pr");
        assert_eq!(pr["kind"], "job");
        assert_eq!(pr["job"], id);
        assert_eq!(pr["jobs"], json!([id]));
        assert_eq!(pr["head"], "hive/sec/x");
        assert_eq!(pr["base"], "main");
        assert_eq!(pr["to"], tip);
        assert_eq!(pr["commits"], 1);
        assert_eq!(pr["opened"], true);
        assert_eq!(
            job_row(&s, &id).str("branch"),
            "hive/sec/x",
            "waits for the merge"
        );

        // The same tip again: nothing. New commits: pushed to the same pull
        // request, recorded as an update.
        assert!(tend_proj(&s, &proj, TEST_NOW).is_empty());
        let tip2 = commit_on(&repo, "hive/sec/x", "more.txt");
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(
            lines,
            [format!(
                "job pr proj job {id}: #7 updated https://github.com/o/r/pull/7"
            )]
        );
        assert_eq!(bare_ref(&bare, "refs/heads/hive/sec/x"), tip2);
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
        assert_eq!(merged["head"], "hive/sec/x");
        assert_eq!(job_row(&s, &id).str("branch"), "");
        assert!(
            rev(&repo, "refs/heads/hive/sec/x").is_empty(),
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
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let job = push_branch(&s, &proj, &repo, "hive/worker/a");
        let id = job.str("id");
        exec1(
            &s,
            "UPDATE jobs SET kind = 'suggestion' WHERE id = ?",
            &[&id],
        );
        tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(gh.head_of(7), "hive/worker/a");
        let before = bare_ref(&bare, "refs/heads/hive/worker/a");

        // The fix was built on the pull request's head (`hive review
        // checkout`) and left on its own branch.
        let fix = push_branch(&s, &proj, &repo, "hive/fixer/f");
        let fix_id = fix.str("id");
        run(
            &repo,
            &["git", "branch", "-f", "hive/fixer/f", "hive/worker/a"],
        );
        let fix_tip = commit_on(&repo, "hive/fixer/f", "fix.txt");
        exec1(
            &s,
            "UPDATE jobs SET kind = 'suggestion', source = 'pr-fix:proj:7' WHERE id = ?",
            &[&fix_id],
        );

        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(
            lines,
            [format!(
                "fix pr proj {fix_id}: #7 updated with 1 commit from hive/fixer/f https://github.com/o/r/pull/7"
            )]
        );
        assert_eq!(bare_ref(&bare, "refs/heads/hive/worker/a"), fix_tip);
        let pr = last_detail(&s, "land.pr");
        assert_eq!(pr["kind"], "job");
        assert_eq!(pr["job"], id, "the pull request keeps its ids");
        assert_eq!(pr["number"], 7);
        assert_eq!(pr["head"], "hive/worker/a");
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
        let stray = push_branch(&s, &proj, &repo, "hive/fixer/stray");
        exec1(
            &s,
            "UPDATE jobs SET source = 'pr-fix:proj:7' WHERE id = ?",
            &[&stray.str("id")],
        );
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l
                .contains("held: hive/fixer/stray is not built on the head of pull request #7")),
            "{lines:?}"
        );
        assert_eq!(bare_ref(&bare, "refs/heads/hive/worker/a"), fix_tip);
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
    fn seed_conversation(s: &hive_store::Store, id: &str, repo: &str) {
        s.chat_create(&hive_store::Conversation {
            id: id.into(),
            title: "a chat".into(),
            project: "proj".into(),
            cwd: repo.into(),
            branch: format!("hive/chat-{id}"),
            status: "idle".into(),
            ..Default::default()
        })
        .unwrap();
    }

    #[test]
    fn a_conversation_branch_is_proposed_once_its_last_commit_is_ten_minutes_old() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        seed_conversation(&s, "abc123", &repo);
        run(&repo, &["git", "branch", "hive/chat-abc123", "main"]);
        // Two minutes old: the conversation may still be committing.
        let tip = commit_dated(
            &repo,
            "hive/chat-abc123",
            "chat.txt",
            "2026-09-06T09:58:00Z",
        );
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(events(&s, "land.pr"), 0);
        let dry = dry_run(&s, &proj, TEST_NOW);
        assert!(
            dry.iter().any(|l| l.contains(
                "conversation hive/chat-abc123: 1 commit ahead of main, last commit 2 minutes ago (waits until it is 10 minutes old)"
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
        assert_eq!(bare_ref(&bare, "refs/heads/hive/chat-abc123"), tip);
        let pr = last_detail(&s, "land.pr");
        assert_eq!(pr["kind"], "proposed");
        assert_eq!(pr["conversation"], "abc123");
        assert_eq!(pr["head"], "hive/chat-abc123");
        assert_eq!(pr["to"], tip);
        assert_eq!(pr["title"], "more on hive/chat-abc123");
        assert_eq!(pr["jobs"], json!([]));
        assert!(tend_proj(&s, &proj, later).is_empty());

        // The hub's chat worktree has the branch checked out: a later
        // commit is still pushed from the root, and after the merge the
        // branch stays for the worktree.
        let wt = format!("{repo}/../chat-wt");
        run(
            &repo,
            &["git", "worktree", "add", "-q", &wt, "hive/chat-abc123"],
        );
        let tip2 = commit_dated(&wt, "hive/chat-abc123", "again.txt", "2026-09-06T09:40:00Z");
        assert_eq!(ref_of(&repo, "refs/heads/hive/chat-abc123"), tip2);
        let lines = tend_proj(&s, &proj, later);
        assert_eq!(
            lines,
            ["proposed pr proj conversation abc123: #7 updated https://github.com/o/r/pull/7"]
        );
        assert_eq!(bare_ref(&bare, "refs/heads/hive/chat-abc123"), tip2);
        assert_eq!(last_detail(&s, "land.pr")["opened"], false);

        merge_on_forge(&gh, &bare, 7);
        let lines = tend_proj(&s, &proj, later);
        assert!(
            lines.iter().any(|l| l.contains("#7 merged")
                && l.contains("hive/chat-abc123 kept: a worktree has it checked out")
                && l.contains("conversation abc123 finished")),
            "{lines:?}"
        );
        let merged = last_detail(&s, "land.merged");
        assert_eq!(merged["kind"], "proposed");
        assert_eq!(merged["conversation"], "abc123");
        assert_eq!(ref_of(&repo, "refs/heads/hive/chat-abc123"), tip2);
        assert_eq!(ref_of(&repo, "refs/heads/main"), tip2);
        // The conversation is finished on the bus, where the hub's wake
        // loop ends its session after any turn in flight; the daemon does
        // not reach the session itself.
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM events WHERE action = ? AND subject = 'abc123'",
                &[&hive_harness::conversation::FINISH_ACTION],
            ),
            1
        );
        let finish = last_detail(&s, hive_harness::conversation::FINISH_ACTION);
        assert_eq!(finish["number"], 7);
        assert_eq!(
            finish["text"],
            "pull request #7 (https://github.com/o/r/pull/7) merged into main; this session is finished"
        );
        // Merged by patch: not proposed again, not finished again.
        assert!(tend_proj(&s, &proj, later).is_empty());
        assert_eq!(events(&s, hive_harness::conversation::FINISH_ACTION), 1);
        run(&repo, &["git", "worktree", "remove", "--force", &wt]);
    }

    /// The before/after report of a session's change: two pictures and the
    /// manifest in the session's artifacts folder.
    fn write_session_report(s: &hive_store::Store, id: &str) {
        let dir = hive_harness::conversation::artifacts_dir(&s.home, id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("before.png"), b"\x89PNG\r\n\x1a\n-1").unwrap();
        std::fs::write(dir.join("after.png"), b"\x89PNG\r\n\x1a\n-2").unwrap();
        std::fs::write(
            dir.join("report.json"),
            r#"{"title":"Page","changes":[{"title":"Page","was":"w","now":"n","before":"before.png","after":"after.png"}]}"#,
        )
        .unwrap();
    }

    /// A session's change that has a face opens its pull request with its
    /// before/after report (`landing.ui_report = require`): the first pass
    /// asks the session (a wake) and waits, a pass within the hour waits
    /// without asking again, and the pull request opens once the report
    /// shows the change. A change without a face is proposed as before.
    #[test]
    fn a_conversation_branch_with_a_face_waits_for_its_report() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let _gh = fake_gh();
        let s = new_store();
        let (proj, repo, _bare) = push_fixture(&s);
        seed_conversation(&s, "face1", &repo);
        run(&repo, &["git", "branch", "hive/chat-face1", "main"]);
        commit_dated(&repo, "hive/chat-face1", "page.tsx", "2026-09-06T09:40:00Z");
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(
            lines,
            [
                "proposed proj hive/chat-face1: waits for its before/after report; session face1 asked"
            ]
        );
        assert_eq!(events(&s, "land.pr"), 0);
        let wake = last_detail(&s, hive_harness::conversation::WAKE_ACTION);
        let text = wake["text"].as_str().unwrap_or_default();
        assert!(
            text.starts_with("Your branch hive/chat-face1 changes 1 UI file (`page.tsx`) and there is no report.json in $HIVE_ARTIFACTS."),
            "{text}"
        );
        assert!(
            text.contains("run `hive report` until it names nothing"),
            "{text}"
        );
        // Within the hour: it waits, and the session is not asked again.
        let lines = tend_proj(&s, &proj, "2026-09-06T10:30:00Z");
        assert_eq!(
            lines,
            [
                "proposed proj hive/chat-face1: waits for its before/after report until 2026-09-06T11:00:00Z"
            ]
        );
        assert_eq!(events(&s, hive_harness::conversation::WAKE_ACTION), 1);
        // The report shows the change: the pull request opens, and the
        // review has no report to ask for.
        write_session_report(&s, "face1");
        let lines = tend_proj(&s, &proj, "2026-09-06T10:31:00Z");
        assert_eq!(
            lines,
            ["proposed pr proj conversation face1: #7 opened https://github.com/o/r/pull/7"]
        );
        let view = crate::review::PrView {
            head: "hive/chat-face1".into(),
            files: vec![("page.tsx".into(), 1, 0)],
            ..Default::default()
        };
        assert!(crate::review::reasons_for(&s, &view).is_empty());
    }

    /// The hour passes without the report: the pull request opens anyway,
    /// and the review gives it to the owner for want of the report. The
    /// session's agent proposing the branch itself is told what to bring.
    #[test]
    fn a_conversation_branch_opens_for_the_owner_when_its_report_never_comes() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let _gh = fake_gh();
        let s = new_store();
        let (proj, repo, _bare) = push_fixture(&s);
        seed_conversation(&s, "face2", &repo);
        run(&repo, &["git", "branch", "hive/chat-face2", "main"]);
        commit_dated(&repo, "hive/chat-face2", "site.css", "2026-09-06T09:40:00Z");
        let e = crate::ui_evidence::chat_branch_evidence(&s, &proj, "hive/chat-face2").unwrap();
        assert!(e.wants_report(), "{e:?}");
        tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(events(&s, "land.pr"), 0);
        let lines = tend_proj(&s, &proj, "2026-09-06T11:00:30Z");
        assert_eq!(
            lines,
            ["proposed pr proj conversation face2: #7 opened https://github.com/o/r/pull/7"]
        );
        let view = crate::review::PrView {
            head: "hive/chat-face2".into(),
            files: vec![("site.css".into(), 1, 0)],
            ..Default::default()
        };
        let reasons = crate::review::reasons_for(&s, &view);
        assert_eq!(reasons.len(), 1, "{reasons:?}");
        assert!(
            reasons[0].starts_with(
                "changes 1 UI file with no before/after report (there is no report.json"
            ),
            "{reasons:?}"
        );
        // Warn for this project: no reason, no wait.
        s.set_setting("human", &hive_store::ui_report_setting("proj"), "warn")
            .unwrap();
        assert!(crate::review::reasons_for(&s, &view).is_empty());
    }

    #[test]
    fn a_conversation_hears_of_its_pull_request_conflicting_and_of_it_closing_unmerged() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        gh.track_remote(&bare);
        seed_conversation(&s, "abc123", &repo);
        run(&repo, &["git", "branch", "hive/chat-abc123", "main"]);
        let tip = commit_dated(
            &repo,
            "hive/chat-abc123",
            "hive_chat.txt",
            "2026-09-06T09:40:00Z",
        );
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(
            lines,
            ["proposed pr proj conversation abc123: #7 opened https://github.com/o/r/pull/7"]
        );

        // The base rewrites the chat's file: the conversation's agent is
        // woken with the conflict, once for this head.
        move_base_on_forge(&repo, &bare, "hive_chat.txt");
        gh.set_mergeable(7, "CONFLICTING");
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(
                |l| l.contains("proposed pr proj: #7 conflicts with main at")
                    && l.contains(
                        "hive_chat.txt; main does not merge in cleanly; conversation abc123 woken"
                    )
            ),
            "{lines:?}"
        );
        assert_eq!(events(&s, "land.pr-conflict"), 1);
        assert_eq!(bare_ref(&bare, "refs/heads/hive/chat-abc123"), tip);
        let wakes = "SELECT COUNT(*) AS n FROM events WHERE action = ? AND subject = 'abc123'";
        let wake_action = hive_harness::conversation::WAKE_ACTION;
        assert_eq!(count(&s, wakes, &[&wake_action]), 1);
        let wake = last_detail(&s, wake_action);
        let text = wake["text"].as_str().unwrap_or_default();
        assert!(
            text.starts_with(
                "Pull request #7 (https://github.com/o/r/pull/7) conflicts with main at"
            ) && text.contains("hive_chat.txt"),
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
        assert_eq!(events(&s, hive_harness::conversation::FINISH_ACTION), 0);
        assert_eq!(bare_ref(&bare, "refs/heads/hive/chat-abc123"), tip);
        assert_eq!(events(&s, "land.merged"), 0);
        // Told once: the closing is settled.
        assert!(tend_proj(&s, &proj, TEST_NOW).is_empty());
        assert_eq!(s.chat_messages("abc123", 0).unwrap().len(), 1);
    }

    #[test]
    fn a_project_without_an_upstream_lands_locally_as_before() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
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
        let job = push_branch(&s, &proj, &repo, "hive/worker/local");
        let tip = ref_of(&repo, "refs/heads/hive/worker/local");
        let dry = dry_run(&s, &proj, TEST_NOW);
        assert_eq!(
            dry[0],
            "proj: no upstream, finished branches land locally on main"
        );
        assert!(
            dry[1].contains("hive/worker/local -> lands locally on main"),
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
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, _bare) = push_fixture(&s);
        let job = push_branch(&s, &proj, &repo, "hive/worker/gone");
        let id = job.str("id");
        exec1(
            &s,
            "UPDATE jobs SET kind = 'suggestion' WHERE id = ?",
            &[&id],
        );
        run(&repo, &["git", "branch", "-D", "hive/worker/gone"]);

        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.starts_with(&format!(
                "job pr proj {id}: branch hive/worker/gone is not in"
            ))),
            "{lines:?}"
        );
        let held = last_detail(&s, "land.pr-held");
        assert_eq!(held["job"], id);
        assert_eq!(held["work"], "hive/worker/gone");
        assert_eq!(job_row(&s, &id).str("branch"), "");
        assert!(gh.calls().is_empty());
        assert!(tend_proj(&s, &proj, TEST_NOW).is_empty(), "not retried");
        assert_eq!(events(&s, "land.pr-held"), 1);
    }

    /// A push that fails holds the work for a wait, not until the work
    /// changes: what broke the push is repaired outside the work, and the
    /// finished branch used to stand until somebody moved it. The wait
    /// grows with the tries that failed in a row, as a failed job's
    /// recovery does, so an outage of minutes heals in minutes and a fault
    /// that lasts is tried four times a day.
    #[test]
    fn a_failed_push_is_tried_again_after_5_15_45_and_135_minutes_then_every_six_hours() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let _gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let job = push_branch(&s, &proj, &repo, "hive/worker/a");
        let id = job.str("id");
        let tip = ref_of(&repo, "refs/heads/hive/worker/a");
        let at = |minutes: i32| crate::util::plus_minutes(TEST_NOW, minutes);
        let held = format!(
            "job pr proj {id}: held: push of hive/worker/a to hive/worker/a on origin/main failed: "
        );
        // A later try reads the branch on the upstream before it pushes.
        let held_again =
            format!("job pr proj {id}: held: fetch of hive/worker/a from origin/main failed: ");

        // The upstream is out of reach: the push fails and the job is held.
        run(
            &repo,
            &[
                "git",
                "remote",
                "set-url",
                "origin",
                &format!("{bare}.gone"),
            ],
        );
        let lines = tend_proj(&s, &proj, &at(0));
        assert!(lines.iter().any(|l| l.starts_with(&held)), "{lines:?}");
        assert_eq!(events(&s, "land.pr-held"), 1);

        // Still out of reach: each try that fails writes one hold and makes
        // the next wait longer, and a hold a minute younger than its wait
        // is not tried.
        let mut now = 0;
        for (tries, wait) in [(2, 5), (3, 15), (4, 45), (5, 135), (6, 360), (7, 360)] {
            tend_proj(&s, &proj, &at(now + wait - 1));
            assert_eq!(
                events(&s, "land.pr-held"),
                tries - 1,
                "tried a minute before its {wait} minutes were over"
            );
            now += wait;
            let lines = tend_proj(&s, &proj, &at(now));
            assert!(
                lines.iter().any(|l| l.starts_with(&held_again)),
                "{lines:?}"
            );
            assert_eq!(
                events(&s, "land.pr-held"),
                tries,
                "not tried after {wait} minutes"
            );
            assert_eq!(last_detail(&s, "land.pr-held")["tries"], tries);
        }

        // The upstream is back. The newest hold stands for its wait, then
        // the same commit is delivered.
        run(&repo, &["git", "remote", "set-url", "origin", &bare]);
        let lines = tend_proj(&s, &proj, &at(now + 359));
        assert!(lines.is_empty(), "{lines:?}");
        let lines = tend_proj(&s, &proj, &at(now + 360));
        assert_eq!(
            lines,
            [format!(
                "job pr proj job {id}: #7 opened https://github.com/o/r/pull/7"
            )]
        );
        assert_eq!(bare_ref(&bare, "refs/heads/hive/worker/a"), tip);
        assert_eq!(events(&s, "land.pr-held"), 7);
    }

    /// The tries are of one piece of work failing in a row. A new commit
    /// is new work, tried at once and counted from one; so is whatever
    /// fails after a delivery that went through.
    #[test]
    fn a_new_commit_or_a_delivery_starts_the_wait_of_a_failed_push_again() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let _gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let job = push_branch(&s, &proj, &repo, "hive/worker/a");
        let id = job.str("id");
        let at = |minutes: i32| crate::util::plus_minutes(TEST_NOW, minutes);
        let gone = format!("{bare}.gone");
        let tries = || last_detail(&s, "land.pr-held")["tries"].clone();

        run(&repo, &["git", "remote", "set-url", "origin", &gone]);
        tend_proj(&s, &proj, &at(0));
        tend_proj(&s, &proj, &at(5));
        assert_eq!(events(&s, "land.pr-held"), 2);
        assert_eq!(tries(), 2, "the next wait is 15 minutes");

        // A new commit a minute later: tried at once, counted from one, and
        // tried again 5 minutes on.
        commit_on(&repo, "hive/worker/a", "more.txt");
        tend_proj(&s, &proj, &at(6));
        assert_eq!(events(&s, "land.pr-held"), 3);
        assert_eq!(tries(), 1);
        tend_proj(&s, &proj, &at(10));
        assert_eq!(events(&s, "land.pr-held"), 3, "tried within 5 minutes");
        tend_proj(&s, &proj, &at(11));
        assert_eq!(events(&s, "land.pr-held"), 4);
        assert_eq!(tries(), 2);

        // The upstream is back and the branch goes out when its 15 minutes
        // are over. What fails after that starts at one again.
        run(&repo, &["git", "remote", "set-url", "origin", &bare]);
        let lines = tend_proj(&s, &proj, &at(26));
        assert_eq!(
            lines,
            [format!(
                "job pr proj job {id}: #7 opened https://github.com/o/r/pull/7"
            )]
        );
        commit_on(&repo, "hive/worker/a", "again.txt");
        run(&repo, &["git", "remote", "set-url", "origin", &gone]);
        tend_proj(&s, &proj, &at(27));
        assert_eq!(events(&s, "land.pr-held"), 5);
        assert_eq!(tries(), 1);
    }

    /// The count a forge hold carries, hold by hold: the holds in a row
    /// for one work key, whatever on the way to the forge failed. A new
    /// key, a delivery and a hold about the work itself each end it.
    #[test]
    fn a_forge_hold_counts_the_holds_in_a_row_for_the_same_work() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = new_store();
        let held = |work: &str, note: &str| {
            hold(
                &s,
                TEST_NOW,
                "proj",
                json!({"job": "job-1", "work": work}),
                note,
            );
            last_detail(&s, "land.pr-held")["tries"].clone()
        };
        let push = "push of hive/w/a to hive/w/a on origin/main failed: no route to host";
        assert_eq!(held("hive/w/a 111", push), 1);
        assert_eq!(
            held("hive/w/a 111", "gh pr create failed: HTTP 502: Bad Gateway"),
            2
        );
        assert_eq!(held("hive/w/a 111", push), 3);
        assert_eq!(held("hive/w/a 222", push), 1, "a new work key");
        assert_eq!(held("hive/w/a 222", push), 2);
        event(
            &s,
            TEST_NOW,
            "land.pr",
            "proj",
            json!({"project": "proj", "job": "job-1", "number": 7, "to": "222"}),
        );
        assert_eq!(held("hive/w/a 222", push), 1, "after a delivery");
        assert_eq!(
            held(
                "hive/w/a 222",
                "hive/w/a does not merge cleanly onto main, which it would stack on: conflicts in a.txt"
            ),
            Value::Null,
            "a hold about the work carries no count"
        );
        assert_eq!(held("hive/w/a 222", push), 1, "after a hold about the work");
    }

    /// A push credential that cannot be read is no fault of the work
    /// either: the hold names it and is tried again after its wait.
    #[test]
    fn a_hold_for_a_push_credential_that_cannot_be_read_is_tried_again_after_its_wait() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let _gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let job = push_branch(&s, &proj, &repo, "hive/worker/a");
        let id = job.str("id");
        let tip = ref_of(&repo, "refs/heads/hive/worker/a");
        set_setting(&s, "push.credential", "file:///nonexistent/push-key");

        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.starts_with(&format!(
                "job pr proj {id}: held: push credential: /nonexistent/push-key: no such file or directory"
            ))),
            "{lines:?}"
        );

        // The reference is repaired: nothing within the first wait, the
        // branch after it.
        set_setting(&s, "push.credential", "");
        let lines = tend_proj(&s, &proj, &crate::util::plus_minutes(TEST_NOW, 4));
        assert!(lines.is_empty(), "{lines:?}");
        let lines = tend_proj(&s, &proj, &crate::util::plus_minutes(TEST_NOW, 5));
        assert_eq!(
            lines,
            [format!(
                "job pr proj job {id}: #7 opened https://github.com/o/r/pull/7"
            )]
        );
        assert_eq!(bare_ref(&bare, "refs/heads/hive/worker/a"), tip);
    }

    /// A hold about the work itself is not a matter of waiting: a branch
    /// that conflicts with the pull request it would stack on stands until
    /// the work changes, however old the hold is.
    /// The jobs of `land.pr` (or any other action) events, oldest first.
    fn jobs_of_events(s: &hive_store::Store, action: &str) -> Vec<String> {
        must_rows(
            s,
            "SELECT json_extract(detail, '$.job') AS job FROM events WHERE action = ? ORDER BY seq",
            &[&action],
        )
        .iter()
        .map(|row| row.str("job"))
        .collect()
    }

    /// When holds stop standing together - the forge is back - a pass tries
    /// a few of them again, the oldest hold first, and leaves the rest for
    /// the next pass: one repair does not open a pull request for every
    /// held job at once.
    #[test]
    fn a_pass_tries_a_few_holds_again_the_oldest_first() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let _gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        for n in 0..5 {
            push_branch(&s, &proj, &repo, &format!("hive/worker/j{n}"));
        }
        let at = |minutes: i32| crate::util::plus_minutes(TEST_NOW, minutes);
        let gone = format!("{bare}.gone");
        run(&repo, &["git", "remote", "set-url", "origin", &gone]);

        // Work never held is not bounded: all five are tried and held.
        tend_proj(&s, &proj, &at(0));
        let first = jobs_of_events(&s, "land.pr-held");
        assert_eq!(first.len(), 5, "{first:?}");

        // Their waits are over and the forge is still down: three are tried
        // again, the oldest holds, and two wait.
        let lines = tend_proj(&s, &proj, &at(5));
        assert!(
            lines.contains(&format!(
                "proj: 2 held deliveries wait for a later pass ({RETRIES_PER_PASS} are tried again a pass, the oldest hold first)"
            )),
            "{lines:?}"
        );
        assert_eq!(jobs_of_events(&s, "land.pr-held")[5..], first[..3]);

        // The three held again wait 15 minutes, the two left 5 from the
        // first hold; with the forge back the two oldest holds go first,
        // then the oldest of the three.
        run(&repo, &["git", "remote", "set-url", "origin", &bare]);
        let lines = tend_proj(&s, &proj, &at(20));
        // They open in the order the pass reads its work (the jobs' own),
        // which keeps a stack in the order its work was done.
        let sorted = |mut ids: Vec<String>| {
            ids.sort();
            ids
        };
        let opened = jobs_of_events(&s, "land.pr");
        assert_eq!(
            sorted(opened),
            sorted(vec![first[3].clone(), first[4].clone(), first[0].clone()]),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("proj: 2 held deliveries wait")),
            "{lines:?}"
        );

        // The next pass takes the rest.
        let lines = tend_proj(&s, &proj, &at(21));
        let opened = jobs_of_events(&s, "land.pr");
        assert_eq!(opened.len(), 5, "{lines:?}");
        assert_eq!(sorted(opened[3..].to_vec()), sorted(first[1..3].to_vec()));
        assert!(tend_proj(&s, &proj, &at(22)).is_empty());
    }

    /// A job's branch with `from`'s own commit picked onto it: a newer job
    /// that carries the older one's change, by its patch.
    fn carrying(s: &hive_store::Store, proj: &Row, repo: &str, name: &str, from: &str) -> String {
        let id = push_branch(s, proj, repo, name).str("id");
        run(repo, &["git", "checkout", "-q", name]);
        run(repo, &["git", "cherry-pick", from]);
        run(repo, &["git", "checkout", "-q", "main"]);
        id
    }

    /// Sets a job's source and the time it was made.
    fn sourced(s: &hive_store::Store, id: &str, source: &str, created: &str) {
        exec1(
            s,
            "UPDATE jobs SET source = ?, created_at = ? WHERE id = ?",
            &[&source, &created, &id],
        );
    }

    /// A held job is not tried again while a newer finished job's pull
    /// request, open or merged, carries every commit of it (by patch, so a
    /// commit picked onto the newer branch counts): the older one would
    /// open a second pull request of the same change. The hold is read
    /// again each pass, and once the newer pull request closes unmerged the
    /// older one goes.
    #[test]
    fn a_held_job_whose_commits_a_newer_open_pull_request_carries_waits_for_it() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let old = push_branch(&s, &proj, &repo, "hive/worker/old").str("id");
        let old_tip = ref_of(&repo, "refs/heads/hive/worker/old");
        sourced(&s, &old, "schedule:SCHED-1", "2026-09-06T08:00:00Z");
        let at = |minutes: i32| crate::util::plus_minutes(TEST_NOW, minutes);
        let gone = format!("{bare}.gone");
        run(&repo, &["git", "remote", "set-url", "origin", &gone]);
        tend_proj(&s, &proj, &at(0));
        assert_eq!(events(&s, "land.pr-held"), 1);

        // A newer job of another source carries the old one's change, and
        // opens its pull request while the old one waits.
        let new = carrying(&s, &proj, &repo, "hive/worker/new", &old_tip);
        sourced(&s, &new, "suggestion:x", "2026-09-06T09:00:00Z");
        run(&repo, &["git", "remote", "set-url", "origin", &bare]);
        let lines = tend_proj(&s, &proj, &at(1));
        assert_eq!(
            jobs_of_events(&s, "land.pr"),
            std::slice::from_ref(&new),
            "{lines:?}"
        );

        let lines = tend_proj(&s, &proj, &at(5));
        let held = format!(
            "job pr proj {old}: held: hive/worker/old is not tried again: job {new} has its commits on pull request #7 (open), which carries every commit of it"
        );
        assert_eq!(lines, [held], "{lines:?}");
        assert_eq!(last_detail(&s, "land.pr-held")["repeated_by"], new);
        for minutes in [60, 24 * 60] {
            let lines = tend_proj(&s, &proj, &at(minutes));
            assert!(lines.is_empty(), "{lines:?}");
        }
        assert_eq!(jobs_of_events(&s, "land.pr"), std::slice::from_ref(&new));

        // The newer pull request is closed unmerged: the old job goes.
        gh.set_state(7, "CLOSED");
        let lines = tend_proj(&s, &proj, &at(25 * 60));
        assert_eq!(
            jobs_of_events(&s, "land.pr"),
            [new.clone(), old.clone()],
            "{lines:?}"
        );
    }

    /// One source names different work from one sitting to the next: a
    /// lead's idle work counts again at every sitting, a schedule does what
    /// it finds. A newer job of the same source whose commits are not the
    /// held one's leaves it to be tried again.
    #[test]
    fn a_newer_job_of_the_same_source_with_other_commits_does_not_hold_the_older() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let _gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let source = "lead-idle:proj:2026-09-06:1";
        let old = push_branch(&s, &proj, &repo, "hive/worker/old").str("id");
        sourced(&s, &old, source, "2026-09-06T08:00:00Z");
        let at = |minutes: i32| crate::util::plus_minutes(TEST_NOW, minutes);
        let gone = format!("{bare}.gone");
        run(&repo, &["git", "remote", "set-url", "origin", &gone]);
        tend_proj(&s, &proj, &at(0));

        let new = push_branch(&s, &proj, &repo, "hive/worker/new").str("id");
        sourced(&s, &new, source, "2026-09-06T09:00:00Z");
        run(&repo, &["git", "remote", "set-url", "origin", &bare]);
        tend_proj(&s, &proj, &at(1));
        let lines = tend_proj(&s, &proj, &at(5));
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with(&format!("job pr proj job {old}: #8 opened"))),
            "{lines:?}"
        );
        assert_eq!(jobs_of_events(&s, "land.pr"), [new, old]);
    }

    /// A newer job carrying the held one's commits counts only once its
    /// work is on the forge: one whose own delivery is held carries
    /// nothing yet, and the older job is tried again.
    #[test]
    fn a_newer_job_whose_own_delivery_is_held_does_not_hold_the_older() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let _gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let old = push_branch(&s, &proj, &repo, "hive/worker/old").str("id");
        let old_tip = ref_of(&repo, "refs/heads/hive/worker/old");
        sourced(&s, &old, "schedule:SCHED-1", "2026-09-06T08:00:00Z");
        let at = |minutes: i32| crate::util::plus_minutes(TEST_NOW, minutes);
        let gone = format!("{bare}.gone");
        run(&repo, &["git", "remote", "set-url", "origin", &gone]);
        tend_proj(&s, &proj, &at(0));
        let new = carrying(&s, &proj, &repo, "hive/worker/new", &old_tip);
        sourced(&s, &new, "schedule:SCHED-1", "2026-09-06T09:00:00Z");
        tend_proj(&s, &proj, &at(1));
        assert_eq!(events(&s, "land.pr-held"), 2);

        run(&repo, &["git", "remote", "set-url", "origin", &bare]);
        let lines = tend_proj(&s, &proj, &at(5));
        assert_eq!(jobs_of_events(&s, "land.pr"), [old], "{lines:?}");
    }

    /// Sets `DELIVERIES_UNREAD` for as long as it lives.
    struct DeliveriesUnread;
    impl DeliveriesUnread {
        fn on() -> Self {
            DELIVERIES_UNREAD.set(true);
            DeliveriesUnread
        }
    }
    impl Drop for DeliveriesUnread {
        fn drop(&mut self) {
            DELIVERIES_UNREAD.set(false);
        }
    }

    /// A task whose pull request is open gets a new job while one read of
    /// its deliveries fails. That read said "no pull request" before, and
    /// the build went onto the pull request's head with a plain force. Now
    /// the task is held, its head on the forge stays as it was, and once
    /// the store reads again the pull request is updated, not opened twice.
    #[test]
    fn a_task_whose_deliveries_cannot_be_read_is_held_and_not_pushed() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let task = task_with_two_branches(&s, &proj, &repo);
        tend_proj(&s, &proj, TEST_NOW);
        let head = format!("refs/heads/{}", task_branch(&task));
        let opened = bare_ref(&bare, &head);
        let c = push_branch(&s, &proj, &repo, "hive/worker/c").str("id");
        exec1(
            &s,
            "UPDATE jobs SET source = ? WHERE id = ?",
            &[&format!("task:{task}"), &c],
        );

        let unread = DeliveriesUnread::on();
        let lines = tend_proj(&s, &proj, &crate::util::plus_minutes(TEST_NOW, 1));
        drop(unread);
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with(&format!("task pr proj {task}: held: {STORE_UNREAD}"))),
            "{lines:?}"
        );
        assert_eq!(bare_ref(&bare, &head), opened, "pushed on a failed read");
        assert_eq!(events(&s, "land.pr"), 1);

        let lines = tend_proj(&s, &proj, &crate::util::plus_minutes(TEST_NOW, 7));
        let tip = ref_of(&repo, "refs/heads/hive/worker/c");
        run(
            &repo,
            &[
                "git",
                "merge-base",
                "--is-ancestor",
                &tip,
                &bare_ref(&bare, &head),
            ],
        );
        assert_eq!(last_detail(&s, "land.pr")["number"], 7, "{lines:?}");
        let creates = gh
            .calls()
            .iter()
            .filter(|c| c.starts_with("pr create"))
            .count();
        assert_eq!(creates, 1, "{:?}", gh.calls());
    }

    /// The same for a job's own pull request: new commits on its branch
    /// while one read fails are held, not pushed as a new pull request
    /// with a plain force onto the open one's head.
    #[test]
    fn a_job_whose_deliveries_cannot_be_read_is_held_and_not_pushed() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let job = push_branch(&s, &proj, &repo, "hive/worker/a").str("id");
        tend_proj(&s, &proj, TEST_NOW);
        let head = format!(
            "refs/heads/{}",
            field_of(&last_detail(&s, "land.pr"), "head")
        );
        let opened = bare_ref(&bare, &head);
        let tip = commit_on(&repo, "hive/worker/a", "more.txt");

        let unread = DeliveriesUnread::on();
        let lines = tend_proj(&s, &proj, &crate::util::plus_minutes(TEST_NOW, 1));
        drop(unread);
        assert_eq!(
            lines
                .iter()
                .filter(|l| l.starts_with(&format!("job pr proj {job}: held: {STORE_UNREAD}")))
                .count(),
            1,
            "{lines:?}"
        );
        assert_eq!(bare_ref(&bare, &head), opened, "pushed on a failed read");
        assert_eq!(events(&s, "land.pr"), 1);

        let lines = tend_proj(&s, &proj, &crate::util::plus_minutes(TEST_NOW, 7));
        run(
            &repo,
            &[
                "git",
                "merge-base",
                "--is-ancestor",
                &tip,
                &bare_ref(&bare, &head),
            ],
        );
        assert_eq!(last_detail(&s, "land.pr")["number"], 7, "{lines:?}");
        let creates = gh
            .calls()
            .iter()
            .filter(|c| c.starts_with("pr create"))
            .count();
        assert_eq!(creates, 1, "{:?}", gh.calls());
    }

    /// The job that carries a held job's commits stays found however many
    /// jobs finish after it: the rule read the 50 newest later jobs, and
    /// with fifty more done the held job opened a second pull request of
    /// the same commits.
    #[test]
    fn a_carrier_is_found_past_fifty_later_jobs() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let _gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let old = push_branch(&s, &proj, &repo, "hive/worker/old").str("id");
        let old_tip = ref_of(&repo, "refs/heads/hive/worker/old");
        sourced(&s, &old, "schedule:SCHED-1", "2026-09-06T08:00:00Z");
        let at = |minutes: i32| crate::util::plus_minutes(TEST_NOW, minutes);
        let gone = format!("{bare}.gone");
        run(&repo, &["git", "remote", "set-url", "origin", &gone]);
        tend_proj(&s, &proj, &at(0));
        let new = carrying(&s, &proj, &repo, "hive/worker/new", &old_tip);
        sourced(&s, &new, "suggestion:x", "2026-09-06T09:00:00Z");
        run(&repo, &["git", "remote", "set-url", "origin", &bare]);
        tend_proj(&s, &proj, &at(1));
        assert_eq!(jobs_of_events(&s, "land.pr"), std::slice::from_ref(&new));

        // Fifty-one later jobs: one past the window the rule read.
        for n in 0..51 {
            let (j, _) = hive_board::add_job(
                &s,
                hive_board::JobInput {
                    project: "proj".into(),
                    kind: "task".into(),
                    title: format!("later {n}"),
                    brief: "b".into(),
                    tier: 1,
                    created_by: "worker".into(),
                    ..Default::default()
                },
            )
            .expect("add job");
            exec1(
                &s,
                "UPDATE jobs SET status = 'done', finished_at = ?, created_at = ? WHERE id = ?",
                &[
                    &TEST_NOW,
                    &format!("2026-09-06T10:{:02}:00Z", n % 60),
                    &j.id,
                ],
            );
        }
        let lines = tend_proj(&s, &proj, &at(5));
        assert_eq!(
            last_detail(&s, "land.pr-held")["repeated_by"],
            new,
            "{lines:?}"
        );
        assert_eq!(jobs_of_events(&s, "land.pr"), std::slice::from_ref(&new));
    }

    /// Two held jobs with the same commits, released together: the older
    /// opens its pull request and the newer is held for it. Only a newer
    /// job counted as the one a repeat repeats, so both opened one.
    #[test]
    fn of_two_held_jobs_with_the_same_commits_only_one_opens_a_pull_request() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let _gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let new = push_branch(&s, &proj, &repo, "hive/worker/new").str("id");
        let new_tip = ref_of(&repo, "refs/heads/hive/worker/new");
        let old = carrying(&s, &proj, &repo, "hive/worker/old", &new_tip);
        sourced(&s, &old, "schedule:SCHED-1", "2026-09-06T08:00:00Z");
        sourced(&s, &new, "suggestion:x", "2026-09-06T09:00:00Z");
        let at = |minutes: i32| crate::util::plus_minutes(TEST_NOW, minutes);
        let gone = format!("{bare}.gone");
        run(&repo, &["git", "remote", "set-url", "origin", &gone]);
        tend_proj(&s, &proj, &at(0));
        assert_eq!(events(&s, "land.pr-held"), 2);

        run(&repo, &["git", "remote", "set-url", "origin", &bare]);
        let lines = tend_proj(&s, &proj, &at(5));
        assert_eq!(
            jobs_of_events(&s, "land.pr"),
            std::slice::from_ref(&old),
            "{lines:?}"
        );
        assert_eq!(last_detail(&s, "land.pr-held")["repeated_by"], old);
        let lines = tend_proj(&s, &proj, &at(60));
        assert!(lines.is_empty(), "{lines:?}");
    }

    /// A `git cherry` that fails says nothing of the commits: a job held
    /// for a repeat stays held, and is read again once git answers. It
    /// read as "not carried" and released the hold.
    #[cfg(unix)]
    #[test]
    fn a_failing_git_cherry_keeps_a_repeat_held() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let _gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let old = push_branch(&s, &proj, &repo, "hive/worker/old").str("id");
        let old_tip = ref_of(&repo, "refs/heads/hive/worker/old");
        sourced(&s, &old, "schedule:SCHED-1", "2026-09-06T08:00:00Z");
        let at = |minutes: i32| crate::util::plus_minutes(TEST_NOW, minutes);
        let gone = format!("{bare}.gone");
        run(&repo, &["git", "remote", "set-url", "origin", &gone]);
        tend_proj(&s, &proj, &at(0));
        let new = carrying(&s, &proj, &repo, "hive/worker/new", &old_tip);
        sourced(&s, &new, "suggestion:x", "2026-09-06T09:00:00Z");
        run(&repo, &["git", "remote", "set-url", "origin", &bare]);
        tend_proj(&s, &proj, &at(1));
        tend_proj(&s, &proj, &at(5));
        assert_eq!(last_detail(&s, "land.pr-held")["repeated_by"], new);

        // git fails every `cherry` from here on, and only that.
        let real = hive_gitenv::git_program();
        let bin = tempfile::tempdir().unwrap();
        let wrapper = bin.path().join("git");
        std::fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nfor a in \"$@\"; do [ \"$a\" = cherry ] && {{ echo 'fatal: cherry broke' >&2; exit 128; }}; done\nexec '{}' \"$@\"\n",
                real.display()
            ),
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = hive_testutil::setenv(
            "PATH",
            format!(
                "{}:{}",
                bin.path().display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        );
        let lines = tend_proj(&s, &proj, &at(60));
        assert_eq!(
            jobs_of_events(&s, "land.pr"),
            std::slice::from_ref(&new),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l
                .starts_with(&format!("job pr proj {old}: kept held: {REPEAT_UNREAD}"))
                && l.contains("cherry broke")),
            "{lines:?}"
        );
        drop(path);

        let lines = tend_proj(&s, &proj, &at(120));
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(jobs_of_events(&s, "land.pr"), std::slice::from_ref(&new));
    }

    /// Whether a try is a try again of a held work reads the hold key
    /// without the pull request the work would stack on.
    #[test]
    fn a_try_again_is_the_same_branches_at_the_same_commits_whatever_the_stack() {
        assert!(same_work("hive/w/a abc on #7", "hive/w/a abc on #9"));
        assert!(same_work("hive/w/a abc", "hive/w/a abc on #9"));
        assert!(!same_work("hive/w/a abc", "hive/w/a def"));
    }

    /// A pull request opening moves the stack's top under every held job,
    /// and so changes every hold key at once: on 26 November one new pull
    /// request released 38 holds of one project. Those are tries again,
    /// three a pass, not new work.
    #[test]
    fn holds_released_by_a_moving_top_are_tried_a_few_a_pass() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let _gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let at = |minutes: i32| crate::util::plus_minutes(TEST_NOW, minutes);
        push_branch(&s, &proj, &repo, "hive/worker/top");
        tend_proj(&s, &proj, &at(0));
        assert_eq!(events(&s, "land.pr"), 1);

        // Five jobs finish while the forge is out of reach: each is held
        // on its way to stack on #7.
        for n in 0..5 {
            push_branch(&s, &proj, &repo, &format!("hive/worker/j{n}"));
        }
        let gone = format!("{bare}.gone");
        run(&repo, &["git", "remote", "set-url", "origin", &gone]);
        tend_proj(&s, &proj, &at(1));
        assert_eq!(events(&s, "land.pr-held"), 5);
        assert!(
            last_detail(&s, "land.pr-held")["work"]
                .as_str()
                .unwrap()
                .ends_with(" on #7")
        );

        // The forge is back and a new job opens #8 inside their waits.
        run(&repo, &["git", "remote", "set-url", "origin", &bare]);
        push_branch(&s, &proj, &repo, "hive/worker/moves");
        let lines = tend_proj(&s, &proj, &at(2));
        assert_eq!(events(&s, "land.pr"), 2, "{lines:?}");

        // Every hold key moved with the top: three are tried, two wait.
        let lines = tend_proj(&s, &proj, &at(3));
        assert_eq!(events(&s, "land.pr"), 5, "{lines:?}");
        assert!(
            lines.contains(&format!(
                "proj: 2 held deliveries wait for a later pass ({RETRIES_PER_PASS} are tried again a pass, the oldest hold first)"
            )),
            "{lines:?}"
        );
    }

    /// A try built on the stack's top merges the top into the job's branch
    /// (`take_in`) before it pushes. When the forge call after the push
    /// fails, the branch stays moved, but the work is the same: the hold
    /// stands for its wait and the next try is a try again.
    #[test]
    fn a_job_branch_take_in_moved_is_still_the_held_work() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, _bare) = push_fixture(&s);
        let at = |minutes: i32| crate::util::plus_minutes(TEST_NOW, minutes);
        push_branch(&s, &proj, &repo, "hive/worker/top");
        tend_proj(&s, &proj, &at(0));
        let id = push_branch(&s, &proj, &repo, "hive/worker/a").str("id");
        let tip = ref_of(&repo, "refs/heads/hive/worker/a");

        gh.fail_creates("HTTP 502: Bad Gateway");
        let lines = tend_proj(&s, &proj, &at(1));
        assert_eq!(
            lines,
            [format!(
                "job pr proj {id}: held: gh pr create failed: HTTP 502: Bad Gateway"
            )]
        );
        let moved = ref_of(&repo, "refs/heads/hive/worker/a");
        assert_ne!(moved, tip, "take_in merged the top into the branch");
        assert_eq!(own_tip(&repo, &moved), tip);

        gh.heal();
        let lines = tend_proj(&s, &proj, &at(2));
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(events(&s, "land.pr-held"), 1);
        let lines = tend_proj(&s, &proj, &at(6));
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with(&format!("job pr proj job {id}: #8 opened"))),
            "{lines:?}"
        );
    }

    #[test]
    fn a_hold_about_the_work_itself_is_not_tried_again_after_the_wait() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let _gh = fake_gh();
        let s = new_store();
        let (proj, repo, _bare) = push_fixture(&s);
        push_branch(&s, &proj, &repo, "hive/worker/a");
        tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(events(&s, "land.pr"), 1);
        let job = push_branch(&s, &proj, &repo, "hive/worker/b");
        commit_on(&repo, "hive/worker/b", "hive_worker_a.txt");

        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(
            lines,
            [format!(
                "job pr proj {}: held: hive/worker/b does not merge cleanly onto hive/worker/a (pull request #7), which it would stack on: conflicts in hive_worker_a.txt",
                job.str("id")
            )]
        );
        assert_eq!(events(&s, "land.pr-held"), 1);
        let lines = tend_proj(&s, &proj, &crate::util::plus_hours(TEST_NOW, 24.0));
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(events(&s, "land.pr-held"), 1);
    }

    /// What a hold's note starts with says whose it is: the notes the push,
    /// the fetch, `gh` and the push credential write are the forge's, and
    /// every note about the branches themselves is the work's.
    #[test]
    fn a_hold_is_the_forges_or_the_works_by_what_its_note_names() {
        for (note, forge) in [
            (
                "push of hive/w/a to hive/w/a on origin/main failed: fatal: unable to access",
                true,
            ),
            (
                "fetch of hive/task-T-1 from origin/main failed: Connection refused",
                true,
            ),
            (
                "pull request #7: fetch of hive/w/a from origin/main failed: no output",
                true,
            ),
            ("push credential: op read failed: op timed out", true),
            (
                "gh pr create as someone failed: HTTP 502: Bad Gateway",
                true,
            ),
            ("gh did not name the pull request for hive/w/a", true),
            (
                "gh (the GitHub CLI) is not installed where the daemon can run it; install it and run gh auth login as the user the daemon runs as",
                true,
            ),
            ("gh pr list answered oddly: expected value", true),
            (
                "gh does not merge cleanly onto main, which it would stack on: conflicts in a.txt",
                false,
            ),
            ("gh (job j-1) is not in /srv/checkout", false),
            (
                "hive/w/b does not merge cleanly onto hive/w/a (pull request #7), which it would stack on: conflicts in push of.txt",
                false,
            ),
            ("hive/w/a (job j-1) is not in /srv/checkout", false),
            (
                "hive/w/a (pull request #7, which the work would stack on) is gone from origin",
                false,
            ),
            ("pull request #7 has no head branch on record", false),
            (
                "hive/w/c is not built on the head of pull request #7 (0123456789ab); rebase it onto hive/w/a first",
                false,
            ),
            ("worktree add failed: fatal: could not lock", false),
            (
                "origin/main rejected the push of hive/w/a to hive/w/a: ! [remote rejected] hive/w/a -> hive/w/a (pre-receive hook declined)",
                false,
            ),
            (
                "nothing to open a pull request for: gh pr create failed: pull request create failed: GraphQL: No commits between main and hive/w/a (createPullRequest)",
                false,
            ),
            (
                "hive/w/a on origin is at 0123456789ab, which the branch here is not built on, and a try after a failed one does not push over it: take those commits into hive/w/a, or delete hive/w/a on origin and run hive land --branch hive/w/a",
                false,
            ),
            (
                "hive/task-T-1 on origin holds commits Hive did not put there (it is at 0123456789ab), and a try after a failed one does not push over them: delete hive/task-T-1 on origin once nothing there is wanted; the task's pull request opens when its work next changes",
                false,
            ),
            (
                "push of hive/w/a to hive/w/a on origin/main failed: ! [remote rejected] hive/w/a -> hive/w/a (failed to lock)",
                true,
            ),
        ] {
            assert_eq!(forge_fault(note), forge, "{note}");
        }
    }

    /// A push the upstream answers and turns down is about what was
    /// pushed, not the way there: a hook that refuses the branch, here. No
    /// later try changes the answer, so the hold opens with the upstream,
    /// not with the push, and stands until the work changes.
    #[test]
    fn a_push_the_upstream_rejects_is_the_works_and_is_not_tried_again() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let _gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let job = push_branch(&s, &proj, &repo, "hive/worker/a");
        let id = job.str("id");
        let hook = std::path::Path::new(&bare).join("hooks/pre-receive");
        std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
        std::fs::write(
            &hook,
            "#!/bin/sh\necho 'GH001: Large files detected.' >&2\nexit 1\n",
        )
        .unwrap();
        run(".", &["chmod", "+x", &hook.display().to_string()]);

        let lines = tend_proj(&s, &proj, TEST_NOW);
        let held = lines
            .iter()
            .find(|l| l.starts_with(&format!("job pr proj {id}: held: ")))
            .unwrap_or_else(|| panic!("{lines:?}"));
        assert!(
            held.contains(
                ": held: origin/main rejected the push of hive/worker/a to hive/worker/a: "
            ) && held.contains("(pre-receive hook declined)"),
            "{held}"
        );
        for minutes in [5, 360, 24 * 60] {
            tend_proj(&s, &proj, &crate::util::plus_minutes(TEST_NOW, minutes));
        }
        assert_eq!(
            events(&s, "land.pr-held"),
            1,
            "a push the upstream rejected was tried again"
        );
    }

    /// So is a lease that no longer holds: somebody pushed onto the pull
    /// request's head, and the push of the branch's new commits is turned
    /// down for it. What was pushed there stays.
    #[test]
    fn a_push_whose_lease_no_longer_holds_is_the_works_and_is_not_tried_again() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let _gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let job = push_branch(&s, &proj, &repo, "hive/worker/a");
        let id = job.str("id");
        tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(events(&s, "land.pr"), 1);

        run(&repo, &["git", "branch", "hand", "hive/worker/a"]);
        let hand = commit_on(&repo, "hand", "hand.txt");
        run(&repo, &["git", "push", "-q", &bare, "hand:hive/worker/a"]);
        run(&repo, &["git", "branch", "-D", "hand"]);
        commit_on(&repo, "hive/worker/a", "more.txt");

        let lines = tend_proj(&s, &proj, TEST_NOW);
        let held = lines
            .iter()
            .find(|l| l.starts_with(&format!("job pr proj {id}: held: ")))
            .unwrap_or_else(|| panic!("{lines:?}"));
        assert!(
            held.contains(
                ": held: origin/main rejected the push of hive/worker/a to hive/worker/a: "
            ) && held.contains("(stale info)"),
            "{held}"
        );
        for minutes in [5, 360, 24 * 60] {
            tend_proj(&s, &proj, &crate::util::plus_minutes(TEST_NOW, minutes));
        }
        assert_eq!(
            events(&s, "land.pr-held"),
            1,
            "a push whose lease did not hold was tried again"
        );
        assert_eq!(bare_ref(&bare, "refs/heads/hive/worker/a"), hand);
    }

    /// And so is `gh pr create` answering that the branch holds nothing
    /// its base has not: the note opens with that, not with the call.
    #[test]
    fn a_pull_request_with_no_commits_to_show_is_the_works_and_is_not_tried_again() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, _bare) = push_fixture(&s);
        let job = push_branch(&s, &proj, &repo, "hive/worker/a");
        gh.fail_creates(
            "pull request create failed: GraphQL: No commits between main and hive/worker/a (createPullRequest)",
        );

        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(
            lines,
            [format!(
                "job pr proj {}: held: nothing to open a pull request for: gh pr create failed: pull request create failed: GraphQL: No commits between main and hive/worker/a (createPullRequest)",
                job.str("id")
            )]
        );
        for minutes in [5, 360, 24 * 60] {
            tend_proj(&s, &proj, &crate::util::plus_minutes(TEST_NOW, minutes));
        }
        assert_eq!(
            events(&s, "land.pr-held"),
            1,
            "a pull request with nothing to show was tried again"
        );
    }

    /// A try after a hold for a failure on the way to the forge does not
    /// force the branch onto the upstream, as a first push does. An earlier
    /// try pushed it before `gh` failed, and somebody may have pushed onto
    /// it since. So the branch there is read first: as the try left it, it
    /// stays as it is; holding commits the local branch has not, it is not
    /// pushed over, and the hold says so until the work changes.
    #[test]
    fn a_retry_does_not_push_over_commits_added_to_the_branch_on_the_upstream() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let job = push_branch(&s, &proj, &repo, "hive/worker/a");
        let id = job.str("id");
        let tip = ref_of(&repo, "refs/heads/hive/worker/a");
        let at = |minutes: i32| crate::util::plus_minutes(TEST_NOW, minutes);
        let on_forge = || bare_ref(&bare, "refs/heads/hive/worker/a");

        // The push goes through and `gh pr create` fails, twice.
        gh.fail_creates("HTTP 502: Bad Gateway");
        let down = [format!(
            "job pr proj {id}: held: gh pr create failed: HTTP 502: Bad Gateway"
        )];
        assert_eq!(tend_proj(&s, &proj, &at(0)), down);
        assert_eq!(on_forge(), tip);
        assert_eq!(tend_proj(&s, &proj, &at(5)), down);
        assert_eq!(on_forge(), tip);

        // Somebody pushes a commit onto the branch during the wait, and gh
        // answers again.
        run(&repo, &["git", "branch", "hand", "hive/worker/a"]);
        let hand = commit_on(&repo, "hand", "hand.txt");
        run(&repo, &["git", "push", "-q", &bare, "hand:hive/worker/a"]);
        gh.heal();
        let lines = tend_proj(&s, &proj, &at(20));
        assert_eq!(
            lines,
            [format!(
                "job pr proj {id}: held: hive/worker/a on origin is at {}, which the branch here is not built on, and a try after a failed one does not push over it: take those commits into hive/worker/a, or delete hive/worker/a on origin and run hive land --branch hive/worker/a",
                crate::recipe::short(&hand, 12)
            )]
        );
        assert_eq!(on_forge(), hand, "the commit pushed there is gone");
        assert_eq!(events(&s, "land.pr"), 0);

        // The hold is about the work: it stands, however old it is.
        let lines = tend_proj(&s, &proj, &at(24 * 60));
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(on_forge(), hand);

        // The branch moves without those commits: new work, and still not
        // pushed over them.
        commit_on(&repo, "hive/worker/a", "more.txt");
        let lines = tend_proj(&s, &proj, &at(24 * 60));
        assert!(
            lines.len() == 1 && lines[0].contains("which the branch here is not built on"),
            "{lines:?}"
        );
        assert_eq!(on_forge(), hand, "the commit pushed there is gone");

        // With those commits in the branch it goes out, on top of them.
        run(&repo, &["git", "branch", "-f", "hive/worker/a", "hand"]);
        let tip = commit_on(&repo, "hive/worker/a", "more.txt");
        let lines = tend_proj(&s, &proj, &at(24 * 60));
        assert_eq!(
            lines,
            [format!(
                "job pr proj job {id}: #7 opened https://github.com/o/r/pull/7"
            )]
        );
        assert_eq!(on_forge(), tip);
    }

    /// The refusal to push over somebody's commits does not lapse however
    /// many holds follow it: each refusal is a hold of its own, and a new
    /// one is written whenever the hold key changes (each pull request the
    /// project opens moves the stack's top). Here 65 of them stand between
    /// the forge hold and the next change of the work - one more than the
    /// 64 a window on the history used to read - and that change is still
    /// not pushed over the commits on the upstream.
    #[test]
    fn a_retry_does_not_push_over_commits_on_the_upstream_after_65_holds() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let job = push_branch(&s, &proj, &repo, "hive/worker/a");
        let id = job.str("id");
        let tip = ref_of(&repo, "refs/heads/hive/worker/a");
        let at = |minutes: i32| crate::util::plus_minutes(TEST_NOW, minutes);
        let on_forge = || bare_ref(&bare, "refs/heads/hive/worker/a");

        // The push goes through and `gh pr create` fails.
        gh.fail_creates("HTTP 502: Bad Gateway");
        tend_proj(&s, &proj, &at(0));
        assert_eq!(on_forge(), tip);

        // Somebody pushes onto the branch; the next try is held for it.
        run(&repo, &["git", "branch", "hand", "hive/worker/a"]);
        let hand = commit_on(&repo, "hand", "hand.txt");
        run(&repo, &["git", "push", "-q", &bare, "hand:hive/worker/a"]);
        gh.heal();
        let lines = tend_proj(&s, &proj, &at(20));
        assert!(
            lines.len() == 1 && lines[0].contains("which the branch here is not built on"),
            "{lines:?}"
        );

        // 65 more holds about the work, each for another key.
        let conn = s.conn().unwrap();
        for n in 0..65 {
            crate::util::event_at(
                &conn,
                &at(30 + n),
                ACTOR,
                "land.pr-held",
                "proj",
                json!({
                    "job": id,
                    "work": format!("hive/worker/a {} on #{n}", crate::recipe::short(&tip, 12)),
                    "note": format!("hive/worker/a on origin is at {}, which the branch here is not built on", crate::recipe::short(&hand, 12)),
                }),
            );
        }
        drop(conn);
        assert!(
            s.conn()
                .is_ok_and(|conn| tried_since(&conn, "proj", "job", &id).unwrap().is_some()),
            "the forge hold fell out of what was read"
        );

        // The work changes: still not pushed over those commits.
        commit_on(&repo, "hive/worker/a", "more.txt");
        let lines = tend_proj(&s, &proj, &at(24 * 60));
        assert!(
            lines.len() == 1 && lines[0].contains("which the branch here is not built on"),
            "{lines:?}"
        );
        assert_eq!(on_forge(), hand, "the commit pushed there is gone");
        assert_eq!(events(&s, "land.pr"), 0);
    }

    /// What a try reads of the tries before it ends at the work's last
    /// delivery: a forge hold before a pull request opened, merged or closed
    /// says nothing of the branch since.
    #[test]
    fn the_tries_since_end_at_the_works_last_delivery() {
        let s = new_store();
        let conn = s.conn().unwrap();
        let hold = |at: &str, note: &str, pushed: Option<&str>| {
            let mut d = json!({"job": "j-1", "work": "hive/w/a abc", "note": note});
            if let Some(sha) = pushed {
                d["pushed"] = json!(sha);
            }
            crate::util::event_at(&conn, at, ACTOR, "land.pr-held", "proj", d);
        };
        assert!(tried_since(&conn, "proj", "job", "j-1").unwrap().is_none());
        hold(TEST_NOW, "gh pr create failed: HTTP 502", Some("abc"));
        for _ in 0..65 {
            hold(
                TEST_NOW,
                "hive/w/a on origin is at def, which the branch here is not built on",
                None,
            );
        }
        let tried = tried_since(&conn, "proj", "job", "j-1")
            .unwrap()
            .expect("a forge hold since");
        assert_eq!(tried.pushed, Some(("abc".into(), "hive/w/a abc".into())));
        // Another job's holds and another project's are not this work's.
        assert!(tried_since(&conn, "proj", "job", "j-2").unwrap().is_none());
        assert!(tried_since(&conn, "other", "job", "j-1").unwrap().is_none());
        // A pull request that carried the job ends what is read.
        crate::util::event_at(
            &conn,
            TEST_NOW,
            ACTOR,
            "land.pr",
            "proj",
            json!({"jobs": ["j-1"], "number": 3, "to": "abc"}),
        );
        assert!(tried_since(&conn, "proj", "job", "j-1").unwrap().is_none());
        hold(
            TEST_NOW,
            "push of hive/w/a to hive/w/a on origin/main failed: x",
            None,
        );
        let tried = tried_since(&conn, "proj", "job", "j-1")
            .unwrap()
            .expect("a forge hold since");
        assert_eq!(
            tried.pushed, None,
            "the push recorded before the pull request"
        );
        for action in ["land.merged", "land.pr-closed"] {
            crate::util::event_at(
                &conn,
                TEST_NOW,
                ACTOR,
                action,
                "proj",
                json!({"job": "j-1"}),
            );
            assert!(
                tried_since(&conn, "proj", "job", "j-1").unwrap().is_none(),
                "{action}"
            );
            hold(
                TEST_NOW,
                "fetch of hive/w/a from origin/main failed: x",
                None,
            );
        }
    }

    /// A task's branch is built for its pull request, so what it holds does
    /// not say whose it is. A try after a hold for a failure on the way to
    /// the forge goes by Hive's own record instead: the hold of a try whose
    /// push went through names the commit it pushed. The branch at that
    /// commit is left as it is while nothing has changed, and at any other
    /// commit - somebody pushed onto it - it is not pushed over: the hold
    /// says so until the work changes.
    #[test]
    fn a_task_retry_leaves_its_own_push_as_it_is_and_does_not_push_over_anybody_elses() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let task = task_with_two_branches(&s, &proj, &repo);
        let head = task_branch(&task);
        let at = |minutes: i32| crate::util::plus_minutes(TEST_NOW, minutes);
        let on_forge = || bare_ref(&bare, &format!("refs/heads/{head}"));

        gh.fail_creates("HTTP 502: Bad Gateway");
        let lines = tend_proj(&s, &proj, &at(0));
        assert!(
            lines.iter().any(|l| l.starts_with(&format!(
                "task pr proj {task}: held: gh pr create failed: HTTP 502: Bad Gateway"
            ))),
            "{lines:?}"
        );
        let pushed = on_forge();
        assert_eq!(last_detail(&s, "land.pr-held")["pushed"], pushed);

        // gh still fails when the wait is over: the branch is not built
        // and pushed a second time. (Git dates a commit to the second, so
        // a build made within the same second would be the same commit.)
        std::thread::sleep(Duration::from_millis(1100));
        tend_proj(&s, &proj, &at(5));
        assert_eq!(events(&s, "land.pr-held"), 2);
        assert_eq!(on_forge(), pushed, "the retry pushed a new tip");

        // Somebody pushes a commit onto it, and gh answers again.
        run(&repo, &["git", "branch", "hand", &pushed]);
        let hand = commit_on(&repo, "hand", "hand.txt");
        run(
            &repo,
            &["git", "push", "-q", &bare, &format!("hand:{head}")],
        );
        run(&repo, &["git", "branch", "-D", "hand"]);
        gh.heal();
        let lines = tend_proj(&s, &proj, &at(20));
        assert_eq!(
            lines,
            [format!(
                "task pr proj {task}: held: {head} on origin holds commits Hive did not put there (it is at {}), and a try after a failed one does not push over them: delete {head} on origin once nothing there is wanted; the task's pull request opens when its work next changes",
                crate::recipe::short(&hand, 12)
            )]
        );
        assert_eq!(on_forge(), hand, "the commit pushed there is gone");
        let lines = tend_proj(&s, &proj, &at(24 * 60));
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(events(&s, "land.pr"), 0);

        // The branch is deleted there and the work changes: a first
        // delivery.
        run(
            ".",
            &[
                "git",
                "--git-dir",
                &bare,
                "update-ref",
                "-d",
                &format!("refs/heads/{head}"),
            ],
        );
        let more = commit_on(&repo, "hive/worker/a", "more.txt");
        let lines = tend_proj(&s, &proj, &at(24 * 60));
        assert!(
            lines
                .iter()
                .any(|l| l.contains(&format!("{task}: #7 opened"))),
            "{lines:?}"
        );
        assert!(is_ancestor(&repo, &more, &on_forge()));
        assert!(!is_ancestor(&repo, &hand, &on_forge()));
    }

    /// An earlier try built the task on the pull request the stack then
    /// had on top, pushed it, and `gh` failed. That pull request is closed
    /// unmerged before the next try. The task's commits are all in the
    /// branch on the upstream, and so are the closed pull request's: the
    /// branch is Hive's own push, and is replaced by a build on the base
    /// as it is now, as a first delivery would make it.
    #[test]
    fn a_task_retry_after_the_pull_request_below_closed_drops_that_ones_commits() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        push_branch(&s, &proj, &repo, "hive/worker/n");
        tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(last_detail(&s, "land.pr")["number"], 7);
        let below = ref_of(&repo, "refs/heads/hive/worker/n");
        let task = task_with_two_branches(&s, &proj, &repo);
        let head = task_branch(&task);
        let on_forge = || bare_ref(&bare, &format!("refs/heads/{head}"));

        gh.fail_creates("HTTP 502: Bad Gateway");
        tend_proj(&s, &proj, TEST_NOW);
        let pushed = on_forge();
        assert!(
            is_ancestor(&repo, &below, &pushed),
            "the task was not built on pull request #7"
        );

        gh.set_state(7, "CLOSED");
        gh.heal();
        let lines = tend_proj(&s, &proj, &crate::util::plus_minutes(TEST_NOW, 1));
        assert!(
            lines
                .iter()
                .any(|l| l.contains(&format!("{task}: #8 opened"))),
            "{lines:?}"
        );
        assert_eq!(last_detail(&s, "land.pr")["base"], "main");
        assert!(
            !is_ancestor(&repo, &below, &on_forge()),
            "the closed pull request's commits are in the task's"
        );
        for branch in ["hive/worker/a", "hive/worker/b"] {
            assert!(
                is_ancestor(
                    &repo,
                    &ref_of(&repo, &format!("refs/heads/{branch}")),
                    &on_forge()
                ),
                "{branch} is not in the pull request"
            );
        }
    }

    /// A task's retry reads every hold since its last delivery too: 65
    /// holds after the forge hold (more than a window of 64 held) and the
    /// branch somebody pushed onto is still not replaced.
    #[test]
    fn a_task_retry_does_not_push_over_commits_on_the_upstream_after_65_holds() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let task = task_with_branches(&s, &proj, &repo, "One thing", &["hive/worker/a"]);
        let head = task_branch(&task);
        let on_forge = || bare_ref(&bare, &format!("refs/heads/{head}"));

        gh.fail_creates("HTTP 502: Bad Gateway");
        tend_proj(&s, &proj, TEST_NOW);
        let pushed = on_forge();
        run(&repo, &["git", "branch", "hand", &pushed]);
        let hand = commit_on(&repo, "hand", "hand.txt");
        run(
            &repo,
            &["git", "push", "-q", &bare, &format!("hand:{head}")],
        );
        run(&repo, &["git", "branch", "-D", "hand"]);
        gh.heal();

        let conn = s.conn().unwrap();
        for n in 0..65 {
            crate::util::event_at(
                &conn,
                &crate::util::plus_minutes(TEST_NOW, 1 + n),
                ACTOR,
                "land.pr-held",
                "proj",
                json!({
                    "task": task,
                    "work": format!("hive/worker/a x on #{n}"),
                    "note": format!("{head} on origin holds commits Hive did not put there"),
                }),
            );
        }
        drop(conn);
        run(&repo, &["git", "branch", "-f", "hive/worker/a", "main"]);
        commit_on(&repo, "hive/worker/a", "rebased.txt");
        let lines = tend_proj(&s, &proj, &crate::util::plus_hours(TEST_NOW, 24.0));
        assert_eq!(on_forge(), hand, "the commit pushed there is gone");
        assert!(
            lines.len() == 1 && lines[0].contains("holds commits Hive did not put there"),
            "{lines:?}"
        );
        assert_eq!(events(&s, "land.pr"), 0);
    }

    /// A conversation's branch is pushed as it is, and a try after a
    /// failed one reads the upstream first, as a job's does: somebody's
    /// commit there is not pushed over.
    #[test]
    fn a_conversation_retry_does_not_push_over_commits_on_the_upstream() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        seed_conversation(&s, "abc123", &repo);
        run(&repo, &["git", "branch", "hive/chat-abc123", "main"]);
        let tip = commit_dated(
            &repo,
            "hive/chat-abc123",
            "chat.txt",
            "2026-09-06T09:00:00Z",
        );
        let on_forge = || bare_ref(&bare, "refs/heads/hive/chat-abc123");
        gh.fail_creates("HTTP 502: Bad Gateway");
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(
            lines,
            ["proposed proj hive/chat-abc123: held: gh pr create failed: HTTP 502: Bad Gateway"]
        );
        assert_eq!(on_forge(), tip);

        run(&repo, &["git", "branch", "hand", "hive/chat-abc123"]);
        let hand = commit_on(&repo, "hand", "hand.txt");
        run(
            &repo,
            &["git", "push", "-q", &bare, "hand:hive/chat-abc123"],
        );
        gh.heal();
        let lines = tend_proj(&s, &proj, &crate::util::plus_minutes(TEST_NOW, 5));
        assert!(
            lines.len() == 1
                && lines[0].starts_with(
                    "proposed proj hive/chat-abc123: held: hive/chat-abc123 on origin is at"
                )
                && lines[0].contains("which the branch here is not built on"),
            "{lines:?}"
        );
        assert_eq!(on_forge(), hand, "the commit pushed there is gone");
        assert_eq!(events(&s, "land.pr"), 0);
    }

    /// A kept experiment's branch goes the same way: after a failed try the
    /// upstream is read first and somebody's commit there is not pushed
    /// over.
    #[test]
    fn an_experiment_retry_does_not_push_over_commits_on_the_upstream() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let job = push_branch(&s, &proj, &repo, "evolve/e1");
        exec1(
            &s,
            "UPDATE jobs SET kind = 'experiment' WHERE id = ?",
            &[&job.str("id")],
        );
        let job = job_row(&s, &job.str("id"));
        let tip = ref_of(&repo, "refs/heads/evolve/e1");
        let on_forge = || bare_ref(&bare, "refs/heads/evolve/e1");
        let deliver = |now: &str| deliver_experiment(&s, &proj, &job, "B-1", "run-1", "kept", now);

        gh.fail_creates("HTTP 502: Bad Gateway");
        let note = deliver(TEST_NOW).unwrap_err();
        assert!(note.starts_with("gh pr create failed"), "{note}");
        hold_experiment(&s, TEST_NOW, ACTOR, &proj, &job, "B-1", &note);
        assert_eq!(on_forge(), tip);

        run(&repo, &["git", "branch", "hand", "evolve/e1"]);
        let hand = commit_on(&repo, "hand", "hand.txt");
        run(&repo, &["git", "push", "-q", &bare, "hand:evolve/e1"]);
        gh.heal();
        let note = deliver(&crate::util::plus_minutes(TEST_NOW, 5)).unwrap_err();
        assert!(
            note.contains("which the branch here is not built on"),
            "{note}"
        );
        assert_eq!(on_forge(), hand, "the commit pushed there is gone");
        assert_eq!(events(&s, "land.pr"), 0);
    }

    /// A store that cannot be read says nothing of the tries before: the
    /// lease is not decided on that silence, the delivery is held.
    #[test]
    fn a_lease_the_store_cannot_decide_holds_the_delivery() {
        let s = new_store();
        let conn = s.conn().unwrap();
        conn.execute_batch("ALTER TABLE events RENAME TO events_gone")
            .unwrap();
        let err = tried_since(&conn, "proj", "job", "j-1")
            .map(|_| ())
            .unwrap_err();
        assert!(err.starts_with(STORE_UNREAD), "{err}");
        assert!(forge_fault(&err), "{err}");
        conn.execute_batch("ALTER TABLE events_gone RENAME TO events")
            .unwrap();
    }

    /// `hive land --branch` on a finished job's branch is that job's
    /// delivery: its pull request names the job, and the next pass leaves
    /// the branch alone. Before, the job path answered `Open`, merged the
    /// stack's top into the branch and pushed it: the head of the pull
    /// request by hand and the base of the one above, which the forge then
    /// took for merged (azure-compliance, 26 November). A pull request on
    /// record with the job's branch as its head and no job named (one
    /// proposed by hand before) counts the same.
    #[test]
    fn a_job_branch_delivered_by_hand_is_not_delivered_again_by_the_pass() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        gh.track_remote(&bare);
        let a = push_branch(&s, &proj, &repo, "hive/worker/a").str("id");
        let b = push_branch(&s, &proj, &repo, "hive/worker/b").str("id");
        let tip_a = ref_of(&repo, "refs/heads/hive/worker/a");

        let d = propose_branch(&s, &proj, "hive/worker/a", TEST_NOW).unwrap();
        assert_eq!((d.number, d.opened), (7, true));
        assert_eq!(last_detail(&s, "land.pr")["jobs"], json!([a]));
        let d = propose_branch(&s, &proj, "hive/worker/b", TEST_NOW).unwrap();
        assert_eq!((d.number, d.opened), (8, true));

        let lines = tend_proj(&s, &proj, &crate::util::plus_minutes(TEST_NOW, 1));
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(events(&s, "land.pr"), 2);
        assert_eq!(ref_of(&repo, "refs/heads/hive/worker/a"), tip_a);
        assert_eq!(bare_ref(&bare, "refs/heads/hive/worker/a"), tip_a);
        assert!(
            open_pr_for_job(&s.conn().unwrap(), "proj", &b)
                .unwrap()
                .is_some()
        );

        // A land.pr by hand from before, naming no job: found by its head.
        let c = push_branch(&s, &proj, &repo, "hive/worker/c").str("id");
        let tip_c = ref_of(&repo, "refs/heads/hive/worker/c");
        run(&repo, &["git", "push", "-q", &bare, "hive/worker/c"]);
        let conn = s.conn().unwrap();
        crate::util::event_at(
            &conn,
            TEST_NOW,
            ACTOR,
            "land.pr",
            "proj",
            json!({"project": "proj", "kind": "proposed", "number": 9,
                "url": "https://github.com/o/r/pull/9", "head": "hive/worker/c",
                "base": "main", "remote": "origin", "to": tip_c, "jobs": [], "opened": true}),
        );
        assert!(open_pr_for_job(&conn, "proj", &c).unwrap().is_some());
        drop(conn);
        let lines = tend_proj(&s, &proj, &crate::util::plus_minutes(TEST_NOW, 2));
        assert!(!lines.iter().any(|l| l.contains(&c)), "{lines:?}");
        assert_eq!(ref_of(&repo, "refs/heads/hive/worker/c"), tip_c);
        assert_eq!(events(&s, "land.pr"), 3);
    }

    /// A rebase job replaced the task's one branch, so none of the task's
    /// commits is in the branch an earlier try pushed - and somebody has
    /// pushed a commit onto that. It is not Hive's push any more, whatever
    /// it holds: it is not replaced.
    #[test]
    fn a_task_retry_does_not_replace_a_branch_somebody_pushed_onto_once_its_branches_moved() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let task = task_with_branches(&s, &proj, &repo, "One thing", &["hive/worker/a"]);
        let head = task_branch(&task);
        let on_forge = || bare_ref(&bare, &format!("refs/heads/{head}"));

        gh.fail_creates("HTTP 502: Bad Gateway");
        tend_proj(&s, &proj, TEST_NOW);
        let pushed = on_forge();

        run(&repo, &["git", "branch", "hand", &pushed]);
        let hand = commit_on(&repo, "hand", "hand.txt");
        run(
            &repo,
            &["git", "push", "-q", &bare, &format!("hand:{head}")],
        );
        run(&repo, &["git", "branch", "-D", "hand"]);
        run(&repo, &["git", "branch", "-f", "hive/worker/a", "main"]);
        commit_on(&repo, "hive/worker/a", "rebased.txt");
        gh.heal();

        let lines = tend_proj(&s, &proj, &crate::util::plus_minutes(TEST_NOW, 1));
        assert_eq!(on_forge(), hand, "the commit pushed there is gone");
        assert!(
            lines.len() == 1 && lines[0].contains("holds commits Hive did not put there"),
            "{lines:?}"
        );
        assert_eq!(events(&s, "land.pr"), 0);
    }

    /// The base moved during the wait. The branch on the upstream is
    /// Hive's own push, built on the base as it was: it is replaced by a
    /// build on the base as it is now, and not built on.
    #[test]
    fn a_task_retry_after_its_base_moved_replaces_its_own_push() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let task = task_with_two_branches(&s, &proj, &repo);
        let head = task_branch(&task);
        let at = |minutes: i32| crate::util::plus_minutes(TEST_NOW, minutes);
        let on_forge = || bare_ref(&bare, &format!("refs/heads/{head}"));

        gh.fail_creates("HTTP 502: Bad Gateway");
        tend_proj(&s, &proj, &at(0));
        let pushed = on_forge();
        let moved = move_base_on_forge(&repo, &bare, "base.txt");
        gh.heal();
        // Held for its wait; the local default branch follows the upstream.
        tend_proj(&s, &proj, &at(1));
        assert_eq!(ref_of(&repo, "refs/heads/main"), moved);
        assert_eq!(on_forge(), pushed);

        let lines = tend_proj(&s, &proj, &at(5));
        assert!(
            lines
                .iter()
                .any(|l| l.contains(&format!("{task}: #7 opened"))),
            "{lines:?}"
        );
        let built = on_forge();
        assert_eq!(last_detail(&s, "land.pr")["to"], built);
        assert!(is_ancestor(&repo, &moved, &built), "not on the new base");
        assert!(
            !is_ancestor(&repo, &pushed, &built),
            "built on the push an earlier try made"
        );
        for branch in ["hive/worker/a", "hive/worker/b"] {
            assert!(
                is_ancestor(
                    &repo,
                    &ref_of(&repo, &format!("refs/heads/{branch}")),
                    &built
                ),
                "{branch} is not in the pull request"
            );
        }
    }

    /// The push that replaces Hive's own leases on the commit that was
    /// read. Somebody pushes onto the branch after the try read it and
    /// before it pushes (a hook of the checkout stands in for them: it
    /// runs when the build's worktree is made): the push is refused, and
    /// what they pushed stays.
    #[test]
    fn a_task_retry_is_refused_when_the_upstream_branch_moves_after_it_was_read() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let task = task_with_two_branches(&s, &proj, &repo);
        let head = task_branch(&task);
        let on_forge = || bare_ref(&bare, &format!("refs/heads/{head}"));

        gh.fail_creates("HTTP 502: Bad Gateway");
        tend_proj(&s, &proj, TEST_NOW);
        let pushed = on_forge();
        // The work changes, so the next try builds and pushes anew.
        commit_on(&repo, "hive/worker/a", "more.txt");
        run(&repo, &["git", "branch", "hand", &pushed]);
        let hand = commit_on(&repo, "hand", "hand.txt");
        run(&repo, &["git", "push", "-q", &bare, "hand:refs/heads/hand"]);
        run(&repo, &["git", "branch", "-D", "hand"]);
        let hook = std::path::Path::new(&repo).join(".git/hooks/post-checkout");
        std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
        std::fs::write(
            &hook,
            format!(
                "#!/bin/sh\nunset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE\ngit --git-dir '{bare}' update-ref refs/heads/{head} {hand}\n"
            ),
        )
        .unwrap();
        run(".", &["chmod", "+x", &hook.display().to_string()]);
        gh.heal();

        let lines = tend_proj(&s, &proj, &crate::util::plus_minutes(TEST_NOW, 1));
        std::fs::remove_file(&hook).unwrap();
        let held = lines
            .iter()
            .find(|l| l.starts_with(&format!("task pr proj {task}: held: ")))
            .unwrap_or_else(|| panic!("{lines:?}"));
        assert!(
            held.contains(&format!(
                ": held: origin/main rejected the push of {head} to {head}: "
            )) && held.contains("(stale info)"),
            "{held}"
        );
        assert_eq!(on_forge(), hand, "what was pushed there is gone");
        assert_eq!(events(&s, "land.pr"), 0);
    }

    /// What the try finds on the upstream may be what the task's own pull
    /// request left there: one closed unmerged keeps its branch, and the
    /// task's next round has the same name for its own. Hive knows that
    /// commit from the pull request's record, and replaces it.
    #[test]
    fn a_task_retry_replaces_what_its_closed_pull_request_left_on_the_upstream() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let task = task_with_two_branches(&s, &proj, &repo);
        let head = task_branch(&task);
        let on_forge = || bare_ref(&bare, &format!("refs/heads/{head}"));
        tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(last_detail(&s, "land.pr")["number"], 7);
        let left = on_forge();
        gh.set_state(7, "CLOSED");
        tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(events(&s, "land.pr-closed"), 1);
        assert_eq!(on_forge(), left);

        // The task's next round is one new job. The upstream is out of
        // reach for its first try, and back for the second.
        let next = push_branch(&s, &proj, &repo, "hive/worker/c");
        exec1(
            &s,
            "UPDATE jobs SET source = ? WHERE id = ?",
            &[&format!("task:{task}"), &next.str("id")],
        );
        run(
            &repo,
            &[
                "git",
                "remote",
                "set-url",
                "origin",
                &format!("{bare}.gone"),
            ],
        );
        tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(events(&s, "land.pr-held"), 1);
        run(&repo, &["git", "remote", "set-url", "origin", &bare]);
        let lines = tend_proj(&s, &proj, &crate::util::plus_minutes(TEST_NOW, 5));
        assert!(
            lines
                .iter()
                .any(|l| l.contains(&format!("{task}: #8 opened"))),
            "{lines:?}"
        );
        assert!(
            !is_ancestor(&repo, &left, &on_forge()),
            "the pull request holds what the closed one left"
        );
        assert!(is_ancestor(
            &repo,
            &ref_of(&repo, "refs/heads/hive/worker/c"),
            &on_forge()
        ));
    }

    /// The upstream turns a push down for its own passing trouble under
    /// the same mark as for its rules: a ref it could not lock, objects it
    /// could not store. That is the way to the forge, and is tried again.
    #[test]
    fn a_push_the_upstream_could_not_take_is_tried_again() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let _gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let job = push_branch(&s, &proj, &repo, "hive/worker/a");
        let id = job.str("id");
        // Another process of the upstream holds the branch's lock.
        let lock = std::path::Path::new(&bare).join("refs/heads/hive/worker/a.lock");
        std::fs::create_dir_all(lock.parent().unwrap()).unwrap();
        std::fs::write(&lock, "").unwrap();

        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.starts_with(&format!(
                "job pr proj {id}: held: push of hive/worker/a to hive/worker/a on origin/main failed: "
            ))),
            "{lines:?}"
        );

        std::fs::remove_file(&lock).unwrap();
        let lines = tend_proj(&s, &proj, &crate::util::plus_minutes(TEST_NOW, 5));
        assert_eq!(
            lines,
            [format!(
                "job pr proj job {id}: #7 opened https://github.com/o/r/pull/7"
            )]
        );
    }

    /// Which pushes git reports as turned down are about the work: every
    /// one the client refused (`! [rejected]`), and of those the upstream
    /// refused (`! [remote rejected]`) the ones it refused for a rule of
    /// the repository.
    #[test]
    fn a_rejected_push_is_the_works_when_a_rule_turned_it_down() {
        let up = Upstream {
            remote: "origin".into(),
            remote_branch: "main".into(),
            upstream_ref: "refs/remotes/origin/main".into(),
            url: "https://github.com/owner/repo.git".into(),
            repo: "owner/repo".into(),
        };
        for (mark, work) in [
            ("! [rejected]        a -> a (stale info)", true),
            ("! [rejected]        a -> a (fetch first)", true),
            ("! [rejected]        a -> a (non-fast-forward)", true),
            (
                "! [remote rejected] a -> a (pre-receive hook declined)",
                true,
            ),
            ("! [remote rejected] a -> a (hook declined)", true),
            (
                "! [remote rejected] a -> a (protected branch hook declined)",
                true,
            ),
            (
                "! [remote rejected] a -> a (push declined due to repository rule violations)",
                true,
            ),
            (
                "! [remote rejected] a -> a (refusing to allow a Personal Access Token to create or update workflow `.github/workflows/ci.yml` without `workflow` scope)",
                true,
            ),
            ("! [remote rejected] a -> a (non-fast-forward)", true),
            ("! [remote rejected] a -> a (unpacker error)", false),
            (
                "! [remote rejected] a -> a (unable to migrate objects to permanent storage)",
                false,
            ),
            (
                "! [remote rejected] a -> a (missing necessary objects)",
                false,
            ),
            ("! [remote rejected] a -> a (failed to update ref)", false),
            ("! [remote rejected] a -> a (failed to lock)", false),
            (
                "! [remote rejected] a -> a (transaction failed to start)",
                false,
            ),
        ] {
            let pushed = crate::util::GitResult {
                code: 1,
                stderr: format!(
                    "To https://github.com/owner/repo.git\n {mark}\nerror: failed to push some refs to 'https://github.com/owner/repo.git'\n"
                ),
                ..Default::default()
            };
            assert_eq!(
                rejected(&pushed, "refs/heads/a", "a", &up).is_some(),
                work,
                "{mark}"
            );
        }
    }

    /// The rule a remote rejection names is read from its reason alone,
    /// the text inside the status line's final parentheses: the line also
    /// names the branches, and a branch named like a rule does not make a
    /// passing trouble of its push a hold about the work.
    #[test]
    fn a_remote_rejection_is_the_works_by_its_reason_not_by_the_branch_it_names() {
        let up = Upstream {
            remote: "origin".into(),
            remote_branch: "main".into(),
            upstream_ref: "refs/remotes/origin/main".into(),
            url: "https://github.com/owner/repo.git".into(),
            repo: "owner/repo".into(),
        };
        for branch in [
            "fix/non-fast-forward",
            "hive/hook-declined",
            "a-hook declined",
            "push-declined-due-to",
            "refusing-to-allow",
            "x(non-fast-forward)",
        ] {
            for (reason, work) in [
                ("failed to lock", false),
                ("unpacker error", false),
                ("failed to update ref", false),
                ("pre-receive hook declined", true),
                ("non-fast-forward", true),
            ] {
                let mark = format!("! [remote rejected] {branch} -> {branch} ({reason})");
                let pushed = crate::util::GitResult {
                    code: 1,
                    stderr: format!("To https://github.com/owner/repo.git\n {mark}\n"),
                    ..Default::default()
                };
                let src = format!("refs/heads/{branch}");
                assert_eq!(
                    rejected(&pushed, &src, branch, &up).is_some(),
                    work,
                    "{mark}"
                );
            }
        }
        assert_eq!(
            reason_of("! [remote rejected] a -> a (hook declined)"),
            Some("hook declined")
        );
        assert_eq!(
            reason_of("! [remote rejected] a -> a (refusing to allow a token (classic) to push)  "),
            Some("refusing to allow a token (classic) to push")
        );
        assert_eq!(reason_of("! [remote rejected] non-fast-forward -> a"), None);
        assert_eq!(reason_of("! [remote rejected] a -> a hook declined)"), None);
    }

    /// Whether the upstream has the branch is read from git's words, so
    /// the read speaks git's own tongue whatever the daemon's locale. Here
    /// git says everything in another one unless it is told `LC_ALL=C`: a
    /// try after a first push that failed still finds the branch not
    /// there, and delivers.
    #[test]
    fn a_retry_reads_the_upstream_whatever_tongue_git_speaks() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let _gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let job = push_branch(&s, &proj, &repo, "hive/worker/a");
        let id = job.str("id");
        let tip = ref_of(&repo, "refs/heads/hive/worker/a");
        let bin = tempfile::tempdir().unwrap();
        let git = bin.path().join("git");
        std::fs::write(
            &git,
            format!(
                "#!/bin/sh\nreal='{}'\n[ \"$LC_ALL\" = C ] && exec \"$real\" \"$@\"\nsaid=\"$0.said.$$\"\n\"$real\" \"$@\" 2>\"$said\"\ncode=$?\n[ -s \"$said\" ] && echo 'Fehler: das ist in einer anderen Sprache gesagt' >&2\nrm -f \"$said\"\nexit $code\n",
                hive_gitenv::git_program().display()
            ),
        )
        .unwrap();
        run(".", &["chmod", "+x", &git.display().to_string()]);
        let _path = hive_testutil::setenv(
            "PATH",
            format!(
                "{}:{}",
                bin.path().display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        );

        run(
            &repo,
            &[
                "git",
                "remote",
                "set-url",
                "origin",
                &format!("{bare}.gone"),
            ],
        );
        // The push speaks git's own tongue too: its status line is read
        // for the reason the upstream gave (`rejected`).
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(
                |l| l.contains("held: push of hive/worker/a") && !l.contains("anderen Sprache")
            ),
            "{lines:?}"
        );
        run(&repo, &["git", "remote", "set-url", "origin", &bare]);
        let lines = tend_proj(&s, &proj, &crate::util::plus_minutes(TEST_NOW, 5));
        assert_eq!(
            lines,
            [format!(
                "job pr proj job {id}: #7 opened https://github.com/o/r/pull/7"
            )]
        );
        assert_eq!(bare_ref(&bare, "refs/heads/hive/worker/a"), tip);
    }

    /// The worker marks its job done itself, naming the branch the runner
    /// creates only after the run's verification: while that run is still
    /// running the task is in flight, not held on a branch that is not
    /// there yet.
    #[test]
    fn a_task_whose_job_is_still_running_waits_for_the_run_instead_of_being_held() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, _bare) = push_fixture(&s);
        let task = task_with_two_branches(&s, &proj, &repo);
        let b = job_row_by_branch(&s, "hive/worker/b");
        let tip = ref_of(&repo, "refs/heads/hive/worker/b");
        run(&repo, &["git", "branch", "-D", "hive/worker/b"]);
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
        run(&repo, &["git", "branch", "hive/worker/b", &tip]);
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

    /// A `cargo` on PATH for the merged-tree check (merge_check.rs): it
    /// logs each call, and "does not compile" when the tree holds two or
    /// more `hive_worker_*.txt` files - two job branches that are green
    /// alone and broken together. With `mode` set to `registry` it fails
    /// before compiling anything, as an unreachable registry does.
    struct FakeCargo {
        dir: tempfile::TempDir,
        _path: hive_testutil::SetenvGuard,
    }

    impl FakeCargo {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("fake cargo dir");
            let d = dir.path().display().to_string();
            let script = format!(
                r#"#!/bin/sh
echo "$CARGO_TARGET_DIR|$HIVE_HUB_SKIP_BUILD|$*" >> '{d}/cargo.log'
if [ "$(cat '{d}/mode' 2>/dev/null)" = registry ]; then
  echo "error: failed to get \`serde\` as a dependency of package \`app\`" >&2
  exit 101
fi
n=$(ls hive_worker_*.txt 2>/dev/null | wc -l)
if [ "$n" -ge 2 ]; then
  echo "src/lib.rs:3:5: error[E0061]: this function takes 2 arguments but 1 argument was supplied" >&2
  echo "error: could not compile \`app\` (lib) due to 1 previous error" >&2
  exit 101
fi
exit 0
"#
            );
            let path = dir.path().join("cargo");
            std::fs::write(&path, script).expect("fake cargo");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                    .expect("chmod cargo");
            }
            let guard = hive_testutil::setenv(
                "PATH",
                format!("{d}:{}", std::env::var("PATH").unwrap_or_default()),
            );
            FakeCargo { dir, _path: guard }
        }

        fn calls(&self) -> Vec<String> {
            std::fs::read_to_string(self.dir.path().join("cargo.log"))
                .unwrap_or_default()
                .lines()
                .map(str::to_string)
                .collect()
        }

        fn mode(&self, mode: &str) {
            std::fs::write(self.dir.path().join("mode"), mode).expect("cargo mode");
        }
    }

    /// The fixture project made a Cargo workspace, on main and upstream.
    fn cargo_project(repo: &str) {
        commit_on(repo, "main", "Cargo.toml");
        run(repo, &["git", "push", "-q", "origin", "main"]);
    }

    /// Two job branches of a task that merge without a conflict but do
    /// not compile together (pulse/task-TASK-096 at 770cf7ea) are not
    /// pushed: a fix job on the task takes both, `land.merge-broken` says
    /// so, and once it is done the pull request opens from its branch. A
    /// second broken merge of the same task is held for the owner.
    #[test]
    fn a_task_whose_merged_branches_do_not_compile_gets_a_fix_job_once_then_held() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let cargo = FakeCargo::new();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        cargo_project(&repo);
        let task = task_with_two_branches(&s, &proj, &repo);
        let a = job_row_by_branch(&s, "hive/worker/a");
        let b = job_row_by_branch(&s, "hive/worker/b");

        // The task merges its branches in job order; both jobs share a
        // creation second here, so the order is the ids'.
        let mut order = [(a.clone(), "hive/worker/a"), (b.clone(), "hive/worker/b")];
        order.sort();
        let names = format!("{}, {}", order[0].1, order[1].1);

        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains(&format!(
                "task pr proj {task}: {names} merge onto main without a conflict but do not compile together: src/lib.rs:3:5: error[E0061]"
            )) && l.contains("makes them compile together")),
            "{lines:?}"
        );
        assert!(gh.calls().is_empty(), "no pull request: {:?}", gh.calls());
        assert!(
            rev(&bare, &format!("refs/heads/{}", task_branch(&task))).is_empty(),
            "the broken task branch is not pushed"
        );
        let calls = cargo.calls();
        assert_eq!(calls.len(), 1, "{calls:?}");
        let lane = std::path::Path::new(&s.home).join("cache/cargo-target/proj/0");
        assert_eq!(
            calls[0],
            format!(
                "{}|1|check --workspace --all-targets --message-format=short",
                lane.display()
            ),
            "a build-cache lane, the console's bundle skipped"
        );
        assert_eq!(events(&s, "land.pr-held"), 0);
        assert_eq!(events(&s, "land.rebase"), 0);
        let broken = last_detail(&s, "land.merge-broken");
        assert_eq!(broken["task"], task);
        assert_eq!(broken["base"], "main");
        assert_eq!(broken["jobs"], json!([order[0].0, order[1].0]));
        assert_eq!(broken["branches"], json!([order[0].1, order[1].1]));
        assert_eq!(
            broken["errors"],
            json!([
                "src/lib.rs:3:5: error[E0061]: this function takes 2 arguments but 1 argument was supplied",
                "error: could not compile `app` (lib) due to 1 previous error"
            ])
        );
        let follow_up = broken["follow_up"].as_str().unwrap().to_string();
        let fu = job_row(&s, &follow_up);
        assert_eq!(fu.str("status"), "queued");
        assert_eq!(fu.str("kind"), "task");
        assert_eq!(fu.str("source"), format!("task:{task}"));
        assert!(
            fu.str("title")
                .starts_with(&format!("Make {names} compile together:")),
            "{}",
            fu.str("title")
        );
        let brief = fu.str("brief");
        assert!(
            brief.contains("merged together onto `main` they do not compile"),
            "{brief}"
        );
        assert!(brief.contains("error[E0061]"), "{brief}");
        assert!(brief.contains("- `hive/worker/a` (job "), "{brief}");
        // A merge, never a copy: the landing refuses cherry-picked commits
        // as work the worker did not write.
        assert!(brief.contains("`git merge --no-ff <commit>`"), "{brief}");
        assert!(!brief.contains("cherry-pick their commits"), "{brief}");
        for j in [&a, &b] {
            assert_eq!(
                job_row(&s, j).str("branch"),
                "",
                "the fix job takes the branch"
            );
            assert!(
                job_row(&s, j)
                    .str("result")
                    .contains(&format!("job {follow_up} makes them compile together"))
            );
        }
        assert_eq!(status(&s, &task), "in_progress");
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM messages WHERE task_id = ? AND body LIKE '%do not compile together%'",
                &[&task]
            ),
            1
        );
        // The fix job is queued: the task waits for it.
        assert!(tend_proj(&s, &proj, TEST_NOW).is_empty());

        // The fix job finishes on one branch cut from main: its tree is
        // its own, already checked by its run, and the pull request opens.
        run(&repo, &["git", "branch", "hive/worker/fixed", "main"]);
        commit_on(&repo, "hive/worker/fixed", "hive_worker_fixed.txt");
        exec1(
            &s,
            "UPDATE jobs SET status = 'done', finished_at = ?, branch = 'hive/worker/fixed' WHERE id = ?",
            &[&TEST_NOW, &follow_up],
        );
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines
                .iter()
                .any(|l| l.contains(&format!("{task}: #7 opened"))),
            "{lines:?}"
        );
        assert_eq!(
            cargo.calls().len(),
            1,
            "one branch onto main is not a new tree"
        );
        assert_eq!(
            jobs_of(&last_detail(&s, "land.pr")),
            vec![follow_up.clone()]
        );

        // A later job of the task breaks the merged tree again: held, the
        // fix job that ran named.
        let c = push_branch(&s, &proj, &repo, "hive/worker/c");
        exec1(
            &s,
            "UPDATE jobs SET source = ? WHERE id = ?",
            &[&format!("task:{task}"), &c.str("id")],
        );
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains(&format!("task pr proj {task}: held:"))
                && l.contains(&format!(
                    "job {follow_up} already made the task's branches compile together once, so this one is for you"
                ))),
            "{lines:?}"
        );
        assert_eq!(cargo.calls().len(), 2);
        assert_eq!(events(&s, "land.merge-broken"), 1);
        assert_eq!(events(&s, "land.pr-held"), 1);
        assert_eq!(job_row(&s, &c.str("id")).str("branch"), "hive/worker/c");
        // Held until the work changes.
        assert!(tend_proj(&s, &proj, TEST_NOW).is_empty());
        assert_eq!(cargo.calls().len(), 2);
    }

    /// The check never holds work it could not judge: a cargo that fails
    /// before compiling (a registry out of reach) lets the pull request
    /// open and says the check was skipped; `landing.merge_check` off runs
    /// nothing; a project without a Cargo.toml is never checked.
    #[test]
    fn a_merge_check_that_cannot_judge_or_is_off_never_holds_the_task() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let cargo = FakeCargo::new();

        // No Cargo.toml: nothing to compile.
        let _gh = fake_gh();
        let s = new_store();
        let (proj, repo, _bare) = push_fixture(&s);
        let task = task_with_two_branches(&s, &proj, &repo);
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines
                .iter()
                .any(|l| l.contains(&format!("{task}: #7 opened"))),
            "{lines:?}"
        );
        assert!(cargo.calls().is_empty(), "{:?}", cargo.calls());

        // Off: the pull request opens from what merges, and nothing says
        // the check was skipped.
        let _gh = fake_gh();
        let s = new_store();
        let (proj, repo, _bare) = push_fixture(&s);
        cargo_project(&repo);
        set_setting(&s, merge_check::SETTING, "off");
        let task = task_with_two_branches(&s, &proj, &repo);
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines
                .iter()
                .any(|l| l.ends_with(&format!("{task}: #7 opened https://github.com/o/r/pull/7"))),
            "{lines:?}"
        );
        assert!(cargo.calls().is_empty(), "{:?}", cargo.calls());
        assert_eq!(events(&s, "land.merge-broken"), 0);

        // Cargo fails before compiling: skipped, said, and delivered.
        cargo.mode("registry");
        let _gh = fake_gh();
        let s = new_store();
        let (proj, repo, _bare) = push_fixture(&s);
        cargo_project(&repo);
        let task = task_with_two_branches(&s, &proj, &repo);
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains(&format!("{task}: #"))
                && l.contains("(merge check skipped: cargo check --workspace --all-targets --message-format=short failed before compiling anything (exit 101): error: failed to get")),
            "{lines:?}"
        );
        assert_eq!(cargo.calls().len(), 1);
        assert_eq!(events(&s, "land.merge-broken"), 0);
        assert_eq!(events(&s, "land.pr-held"), 0);
    }

    /// A job branch that conflicts with the default branch goes to a rebase
    /// job on the same task, once; the task's pull request opens from the
    /// rebased branch, and a second conflict is held for the owner.
    #[test]
    fn a_conflicting_task_branch_is_handed_to_a_rebase_job_once_then_held() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, _bare) = push_fixture(&s);
        let task = task_with_two_branches(&s, &proj, &repo);
        let b = job_row_by_branch(&s, "hive/worker/b");
        // main changes the file b's commit adds, so b no longer merges.
        commit_on(&repo, "main", "hive_worker_b.txt");
        run(&repo, &["git", "push", "-q", "origin", "main"]);
        let b_tip = ref_of(&repo, "refs/heads/hive/worker/b");

        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains(&format!(
                "task pr proj {task}: hive/worker/b (job {b}) does not merge cleanly onto main"
            )) && l.contains("conflicts in hive_worker_b.txt")
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
        assert_eq!(rebase["files"], json!(["hive_worker_b.txt"]));
        let follow_up = rebase["follow_up"].as_str().unwrap().to_string();
        let fu = job_row(&s, &follow_up);
        assert_eq!(fu.str("status"), "queued");
        assert_eq!(fu.str("source"), format!("task:{task}"));
        assert!(
            fu.str("title")
                .starts_with("Merge hive/worker/b onto main:"),
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
        run(&repo, &["git", "branch", "hive/worker/rebased", "main"]);
        let c_tip = commit_on(&repo, "hive/worker/rebased", "rebased.txt");
        exec1(
            &s,
            "UPDATE jobs SET status = 'done', finished_at = ?, branch = 'hive/worker/rebased' WHERE id = ?",
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
        let mut want = vec![job_row_by_branch(&s, "hive/worker/a"), follow_up.clone()];
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
        let d = push_branch(&s, &proj, &repo, "hive/worker/d");
        exec1(
            &s,
            "UPDATE jobs SET source = ? WHERE id = ?",
            &[&format!("task:{task}"), &d.str("id")],
        );
        commit_on(&repo, "main", "hive_worker_d.txt");
        run(&repo, &["git", "push", "-q", "origin", "main"]);
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains("held:")
                && l.contains("conflicts in hive_worker_d.txt")
                && l.contains(&format!(
                    "job {follow_up} already rebased the task's work once"
                ))),
            "{lines:?}"
        );
        let held = last_detail(&s, "land.pr-held");
        assert_eq!(held["task"], task);
        assert_eq!(
            job_row(&s, &d.str("id")).str("branch"),
            "hive/worker/d",
            "kept"
        );
        assert_eq!(events(&s, "land.rebase"), 1);
        assert!(
            tend_proj(&s, &proj, TEST_NOW).is_empty(),
            "held once per sha"
        );
        assert_eq!(events(&s, "land.pr-held"), 1);
    }

    /// The rebase job's brief tells the worker to merge the branch, never
    /// to copy its commits: the landing refuses a cherry-picked or rebased
    /// commit another agent wrote (TASK-128 was refused so), and accepts
    /// the branch's commits as they are when a merge reaches them.
    #[test]
    fn a_rebase_job_is_told_to_merge_the_branch_not_to_copy_its_commits() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let _gh = fake_gh();
        let s = new_store();
        let (proj, repo, _bare) = push_fixture(&s);
        let task = task_with_two_branches(&s, &proj, &repo);
        commit_on(&repo, "main", "hive_worker_b.txt");
        run(&repo, &["git", "push", "-q", "origin", "main"]);
        let b_tip = ref_of(&repo, "refs/heads/hive/worker/b");

        tend_proj(&s, &proj, TEST_NOW);
        let rebase = last_detail(&s, "land.rebase");
        assert_eq!(rebase["task"], task);
        assert!(rebase.get("join").is_none(), "{rebase}");
        let brief = job_row(&s, rebase["follow_up"].as_str().unwrap()).str("brief");
        assert!(
            brief.contains(&format!("`git merge --no-ff {b_tip}`")),
            "{brief}"
        );
        assert!(brief.contains("Merge, do not copy"), "{brief}");
        assert!(
            !brief.contains("`git cherry-pick` the commits")
                && !brief.contains("or rebase the branch"),
            "{brief}"
        );
    }

    /// A join lists only the branches the build merges: one another branch
    /// carries already is passed over by the build, so the join brief does
    /// not name it either (TASK-139 item 7) - merging it again would meet
    /// the conflict it was left out for.
    #[test]
    fn a_join_brief_leaves_out_a_branch_another_one_carries() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let _gh = fake_gh();
        let s = new_store();
        let (proj, repo, _bare) = push_fixture(&s);
        let task = task_with_branches(
            &s,
            &proj,
            &repo,
            "Add the thing",
            &["hive/worker/a", "hive/worker/b", "hive/worker/c"],
        );
        // `a` and `b` write the same file; `c` was cut from `a` and carries
        // it whole, so the build passes `a` over and `b` meets `c`.
        let a_tip = commit_on(&repo, "hive/worker/a", "shared.txt");
        let b_tip = commit_on(&repo, "hive/worker/b", "shared.txt");
        run(&repo, &["git", "branch", "-f", "hive/worker/c", &a_tip]);
        let c_tip = commit_on(&repo, "hive/worker/c", "c.txt");
        let a = job_row_by_branch(&s, "hive/worker/a");
        let b = job_row_by_branch(&s, "hive/worker/b");
        let c = job_row_by_branch(&s, "hive/worker/c");

        tend_proj(&s, &proj, TEST_NOW);
        let join = last_detail(&s, "land.rebase");
        assert_eq!(join["join"], true, "{join}");
        assert_eq!(join["task"], task);
        let mut joined: Vec<String> = join["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|j| j.as_str().unwrap().to_string())
            .collect();
        joined.sort();
        let mut sides = vec![b.clone(), c.clone()];
        sides.sort();
        assert_eq!(joined, sides, "{join}");
        assert!(!joined.contains(&a), "{join}");
        let brief = job_row(&s, join["follow_up"].as_str().unwrap()).str("brief");
        for tip in [&b_tip, &c_tip] {
            assert!(
                brief.contains(&format!("`git merge --no-ff {tip}`")),
                "{brief}"
            );
        }
        assert!(
            !brief.contains(&format!("`git merge --no-ff {a_tip}`"))
                && !brief.contains("hive/worker/a`"),
            "{brief}"
        );
        assert!(brief.contains("has 2 finished job branches"), "{brief}");
        assert!(brief.contains("1 more of its branches"), "{brief}");
    }

    /// Branches of one task that each merge onto the base alone but
    /// conflict with each other are one join job's work, even after a
    /// rebase job ran for the task (TASK-129 was held for the owner so).
    /// The join merges them all; its branch carries them, so the build
    /// merges it alone and the pull request opens. A second conflict among
    /// the branches is the owner's.
    #[test]
    fn branches_of_one_task_that_conflict_with_each_other_go_to_one_join_job() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, _bare) = push_fixture(&s);
        let task = task_with_two_branches(&s, &proj, &repo);
        // Both branches write the same file: each merges onto main, but
        // not together.
        let a_tip = commit_on(&repo, "hive/worker/a", "shared.txt");
        let b_tip = commit_on(&repo, "hive/worker/b", "shared.txt");
        let a = job_row_by_branch(&s, "hive/worker/a");
        let b = job_row_by_branch(&s, "hive/worker/b");
        // A rebase job already ran for the task, for another conflict.
        event(
            &s,
            TEST_NOW,
            "land.rebase",
            "proj",
            json!({"project": "proj", "task": task, "job": "job-earlier", "branch": "hive/worker/old",
                "sha": "f00", "files": ["x"], "follow_up": "job-earlier-rebase", "base": "main"}),
        );

        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| (l.contains(&format!(
                "task pr proj {task}: hive/worker/b (job {b}) does not merge cleanly onto main"
            )) || l.contains(&format!(
                "task pr proj {task}: hive/worker/a (job {a}) does not merge cleanly onto main"
            ))) && l.contains("conflicts in shared.txt")
                && l.contains("the task's branches conflict with each other")
                && l.contains("joins them")),
            "{lines:?}"
        );
        assert_eq!(events(&s, "land.pr-held"), 0, "not held for the owner");
        assert!(gh.calls().is_empty(), "{:?}", gh.calls());
        let join = last_detail(&s, "land.rebase");
        assert_eq!(join["join"], true);
        assert_eq!(join["task"], task);
        assert_eq!(join["files"], json!(["shared.txt"]));
        let mut joined: Vec<String> = join["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|j| j.as_str().unwrap().to_string())
            .collect();
        joined.sort();
        let mut both = vec![a.clone(), b.clone()];
        both.sort();
        assert_eq!(joined, both);
        let follow_up = join["follow_up"].as_str().unwrap().to_string();
        let fu = job_row(&s, &follow_up);
        assert_eq!(fu.str("source"), format!("task:{task}"));
        assert!(
            fu.str("title")
                .starts_with(&format!("Join the branches of {task} onto main:")),
            "{}",
            fu.str("title")
        );
        let brief = fu.str("brief");
        for tip in [&a_tip, &b_tip] {
            assert!(
                brief.contains(&format!("`git merge --no-ff {tip}`")),
                "{brief}"
            );
        }
        assert!(brief.contains("Do not `git cherry-pick`"), "{brief}");
        // The branches stay with their jobs: the join's branch carries them.
        assert_eq!(job_row(&s, &a).str("branch"), "hive/worker/a");
        assert_eq!(job_row(&s, &b).str("branch"), "hive/worker/b");
        assert!(
            tend_proj(&s, &proj, TEST_NOW).is_empty(),
            "the join job is queued: the task waits for it"
        );

        // The join merges both on main and resolves the conflict in the
        // merge commit.
        run(
            &repo,
            &["git", "checkout", "-q", "-b", "hive/worker/join", "main"],
        );
        run(
            &repo,
            &["git", "merge", "-q", "--no-ff", "--no-edit", &a_tip],
        );
        // With the identity `run` gives git: a user without a name in the
        // password database (a rented Linux machine's) otherwise fails the
        // merge on "empty ident name" before it stops at the conflict, and
        // the commit below finds no merge message. LC_ALL=C keeps git's
        // "CONFLICT" untranslated under a machine's own locale.
        let merged = std::process::Command::new("git")
            .args(["merge", "-q", "--no-ff", "--no-edit", &b_tip])
            .current_dir(&repo)
            .env("LC_ALL", "C")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@x")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@x")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(!merged.status.success(), "the two conflict");
        assert!(
            String::from_utf8_lossy(&merged.stdout).contains("CONFLICT"),
            "the merge stops at the conflict: {}",
            String::from_utf8_lossy(&merged.stderr)
        );
        std::fs::write(std::path::Path::new(&repo).join("shared.txt"), "both\n").unwrap();
        run(&repo, &["git", "add", "shared.txt"]);
        run(&repo, &["git", "commit", "-q", "--no-edit"]);
        let join_tip = ref_of(&repo, "HEAD");
        run(&repo, &["git", "checkout", "-q", "main"]);
        exec1(
            &s,
            "UPDATE jobs SET status = 'done', finished_at = ?, branch = 'hive/worker/join' WHERE id = ?",
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
        let mut delivered: Vec<String> = pr["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|j| j.as_str().unwrap().to_string())
            .collect();
        delivered.sort();
        let mut all = vec![a.clone(), b.clone(), follow_up.clone()];
        all.sort();
        assert_eq!(delivered, all);
        run(
            &repo,
            &[
                "git",
                "merge-base",
                "--is-ancestor",
                &join_tip,
                pr["to"].as_str().unwrap(),
            ],
        );
        assert_eq!(events(&s, "land.pr-held"), 0);
    }

    /// A second conflict among a task's own branches, after its join job,
    /// is the owner's.
    #[test]
    fn a_task_joined_once_whose_branches_conflict_again_is_held() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let _gh = fake_gh();
        let s = new_store();
        let (proj, repo, _bare) = push_fixture(&s);
        let task = task_with_two_branches(&s, &proj, &repo);
        commit_on(&repo, "hive/worker/a", "shared.txt");
        commit_on(&repo, "hive/worker/b", "shared.txt");
        event(
            &s,
            TEST_NOW,
            "land.rebase",
            "proj",
            json!({"project": "proj", "task": task, "job": "job-x", "branch": "hive/worker/x",
                "sha": "f00", "files": ["shared.txt"], "follow_up": "job-join", "base": "main", "join": true}),
        );
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains("held:")
                && l.contains("job job-join already joined the task's branches once")),
            "{lines:?}"
        );
        assert_eq!(events(&s, "land.pr-held"), 1);
        assert_eq!(events(&s, "land.rebase"), 1);
    }

    fn job_row_by_branch(s: &hive_store::Store, branch: &str) -> String {
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
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, _bare) = push_fixture(&s);
        for i in 0..12 {
            let j = push_branch(&s, &proj, &repo, &format!("hive/worker/n{i}"));
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
        let _env = hive_testutil::env_lock();
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

    /// `hive land --branch` on a job branch forked before main was squashed
    /// to a new root: the pull request carries the job's own commit, copied
    /// onto the squashed main from the job's run base, not the whole
    /// pre-squash history that would rewrite every file.
    #[test]
    fn propose_branch_replays_a_job_branch_forked_before_main_was_squashed() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let fork = ref_of(&repo, "refs/heads/main");
        let branch = "hive/aurora/job-squashed-propose";
        run(&repo, &["git", "branch", branch, "main"]);
        commit_on(&repo, branch, "job.txt");
        // The owner squashes main to a new root commit after the job's run forked.
        std::fs::write(std::path::Path::new(&repo).join("squash.txt"), "squash\n").unwrap();
        run(&repo, &["git", "add", "squash.txt"]);
        let tree = run(&repo, &["git", "write-tree"]).trim().to_string();
        let squashed = run(&repo, &["git", "commit-tree", &tree, "-m", "Squash"])
            .trim()
            .to_string();
        run(&repo, &["git", "reset", "-q", "--hard", &squashed]);
        run(&repo, &["git", "push", "-q", "--force", "origin", "main"]);
        let (job, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "add job.txt".into(),
            "add job.txt".into(),
            1,
            0,
            "test".into(),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .expect("add job");
        exec1(
            &s,
            "UPDATE jobs SET status = 'done', branch = ?, finished_at = ? WHERE id = ?",
            &[&branch, &TEST_NOW, &job.str("id")],
        );
        exec1(
            &s,
            "INSERT INTO runs (id, job_id, agent, project, started_at, finished_at, status, base) VALUES ('run-squashed-propose', ?, 'aurora', 'proj', ?, ?, 'done', ?)",
            &[&job.str("id"), &TEST_NOW, &TEST_NOW, &fork],
        );

        let d = propose_branch(&s, &proj, branch, TEST_NOW).unwrap();
        assert_eq!((d.number, d.opened), (7, true));
        assert!(d.line.contains("replayed from its run base"), "{}", d.line);
        let tip = ref_of(&repo, &format!("refs/heads/{branch}"));
        assert_eq!(bare_ref(&bare, &format!("refs/heads/{branch}")), tip);
        assert_eq!(
            run(
                &repo,
                &["git", "rev-list", "--count", &format!("main..{tip}")]
            )
            .trim(),
            "1",
            "the job's own commit alone"
        );
        assert_eq!(
            run(&repo, &["git", "rev-parse", &format!("{tip}^")]).trim(),
            squashed,
            "built on the squashed main"
        );
        assert_eq!(
            run(&repo, &["git", "diff", "--name-only", &squashed, &tip]).trim(),
            "job.txt"
        );
        let pr = last_detail(&s, "land.pr");
        assert_eq!(pr["commits"], 1, "{pr}");
        assert_eq!(pr["from"], squashed, "{pr}");
        assert_eq!(gh.head_of(7), branch);
        let again = propose_branch(&s, &proj, branch, TEST_NOW).unwrap();
        assert_eq!((again.number, again.opened), (7, false));
        assert!(again.line.contains("is delivered at"), "{}", again.line);
    }

    /// A task delivered while #7 is open: `hive/task-<id>` is built on #7's
    /// head, its pull request targets #7's branch and records the stack.
    fn stacked_task(
        s: &hive_store::Store,
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

        let second = task_with_branches(s, proj, repo, "Add the other thing", &["hive/worker/c"]);
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
        let _env = hive_testutil::env_lock();
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
        // The merge kept #7's branch for #8 (deleting a base closes the pull
        // requests on it); with #8 moved, it goes, and #8 stays open.
        assert!(
            lines
                .iter()
                .any(|l| l == &format!("pr proj: {first_branch} deleted on origin")),
            "{lines:?}"
        );
        let gone = std::process::Command::new("git")
            .args(["--git-dir", &bare, "rev-parse", "-q", "--verify"])
            .arg(format!("refs/heads/{first_branch}"))
            .output()
            .unwrap();
        assert!(
            !gone.status.success(),
            "{first_branch} is still on the upstream"
        );
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
    fn a_task_stacked_on_a_pull_request_closed_unmerged_is_rebuilt_without_its_commits_and_told_once()
     {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        gh.track_remote(&bare);
        let (first, second, first_head) = stacked_task(&s, &proj, &repo, &bare, &gh);
        let first_branch = task_branch(&first);
        let second_branch = task_branch(&second);
        let main = bare_ref(&bare, "refs/heads/main");
        let own = ref_of(&repo, "refs/heads/hive/worker/c");
        let told = |task: &str| {
            count(
                &s,
                "SELECT COUNT(*) AS n FROM messages WHERE task_id = ? AND body LIKE '%was stacked on #7%'",
                &[&task],
            )
        };

        // The bottom is closed unmerged: #8 is rebuilt on main from its own
        // commit alone, pushed, retargeted, and the task is told once.
        gh.set_state(7, "CLOSED");
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines
                .iter()
                .any(|l| l.contains("#7 closed unmerged; task back in progress")),
            "{lines:?}"
        );
        let tip = bare_ref(&bare, &format!("refs/heads/{second_branch}"));
        let dropped = run(
            &repo,
            &[
                "git",
                "rev-list",
                "--count",
                &format!("{main}..{first_head}"),
            ],
        )
        .trim()
        .to_string();
        assert!(
            lines.iter().any(|l| l
                == &format!(
                    "task pr proj: #8 rebuilt on main without #7's commits (1 of its own, {dropped} dropped), head now {}; task {second} told",
                    crate::recipe::short(&tip, 12)
                )),
            "{lines:?}"
        );
        assert_eq!(gh.base_of(8), "main");
        assert_eq!(
            run(
                &repo,
                &["git", "rev-list", "--count", &format!("{main}..{tip}")]
            )
            .trim(),
            "1",
            "only its own commit on main"
        );
        assert!(
            !crate::util::git_timeout(
                &repo,
                Duration::from_secs(10),
                &["merge-base", "--is-ancestor", &first_head, &tip],
            )
            .ok(),
            "#7's commits are gone"
        );
        assert_eq!(
            run(&repo, &["git", "show", "-s", "--format=%s", &tip]).trim(),
            run(&repo, &["git", "show", "-s", "--format=%s", &own]).trim(),
            "the own commit, replayed"
        );
        assert_eq!(
            ref_of(&repo, &format!("refs/heads/{second_branch}")),
            tip,
            "the local branch follows"
        );
        let pr = last_detail(&s, "land.pr");
        assert_eq!(pr["number"], 8);
        assert_eq!(pr["base"], "main");
        assert_eq!(pr["from"], main);
        assert_eq!(pr["to"], tip);
        assert_eq!(pr["commits"], 1);
        assert_eq!(pr["rebuilt"], true);
        assert_eq!(pr["retargeted_from"], first_branch);
        assert!(pr.get("stack_on").is_none(), "{pr}");
        let re = last_detail(&s, "land.pr-rebuilt");
        assert_eq!(re["kind"], "task");
        assert_eq!(re["number"], 8);
        assert_eq!(re["old_base"], first_branch);
        assert_eq!(re["base"], "main");
        assert_eq!(re["to"], tip);
        assert_eq!(re["dropped"]["number"], 7);
        assert_eq!(re["dropped"]["commits"].to_string(), dropped);
        assert_eq!(events(&s, "land.pr-rebuilt"), 1);
        assert_eq!(events(&s, "land.pr-held"), 0);
        assert_eq!(told(&second), 1);
        assert_eq!(told(&first), 0);
        assert!(
            crate::util::git_timeout(&repo, Duration::from_secs(10), &["worktree", "list"])
                .stdout
                .lines()
                .count()
                == 1,
            "the scratch worktree is gone"
        );

        // The next pass has nothing to do: the rebuilt tip is what the
        // pull request has.
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(told(&second), 1);
        assert_eq!(events(&s, "land.pr-rebuilt"), 1);
    }

    /// A stacked pull request built by a join holds the join's
    /// resolution in a merge commit. When the pull request below closes
    /// unmerged, the rebuild keeps that resolution instead of replaying the
    /// branches one by one into the conflict the join resolved, which held
    /// the pull request for the owner (TASK-139 item 5).
    #[test]
    fn a_stacked_pull_request_built_by_a_join_is_rebuilt_with_the_joins_resolution() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        gh.track_remote(&bare);
        let (first, second, first_head) = stacked_task(&s, &proj, &repo, &bare, &gh);
        let second_branch = task_branch(&second);
        let main = bare_ref(&bare, "refs/heads/main");

        // #8's head is a join: two sides that write the same file, merged
        // on top of #7's head, the second merge resolved by hand.
        let second_head = bare_ref(&bare, &format!("refs/heads/{second_branch}"));
        run(&repo, &["git", "branch", "-f", "side-x", &second_head]);
        run(&repo, &["git", "branch", "-f", "side-y", &second_head]);
        commit_on(&repo, "side-x", "joined.txt");
        commit_on(&repo, "side-y", "joined.txt");
        run(
            &repo,
            &["git", "checkout", "-q", "-B", "join", &second_head],
        );
        run(
            &repo,
            &["git", "merge", "-q", "--no-ff", "-m", "take x", "side-x"],
        );
        let clash = std::process::Command::new("git")
            .args(["merge", "-q", "--no-ff", "-m", "take y", "side-y"])
            .current_dir(&repo)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@x")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@x")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(!clash.status.success(), "the sides must conflict");
        std::fs::write(std::path::Path::new(&repo).join("joined.txt"), "x and y\n").unwrap();
        run(&repo, &["git", "add", "joined.txt"]);
        run(&repo, &["git", "commit", "-q", "--no-edit"]);
        run(&repo, &["git", "checkout", "-q", "main"]);
        let joined = ref_of(&repo, "refs/heads/join");
        run(
            &repo,
            &[
                "git",
                "push",
                "-q",
                "-f",
                &bare,
                &format!("join:{second_branch}"),
            ],
        );

        gh.set_state(7, "CLOSED");
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines
                .iter()
                .any(|l| l.contains("#7 closed unmerged; task back in progress")),
            "{lines:?}"
        );
        assert_eq!(events(&s, "land.pr-held"), 0, "{lines:?}");
        assert_eq!(events(&s, "land.pr-rebuilt"), 1, "{lines:?}");
        let tip = bare_ref(&bare, &format!("refs/heads/{second_branch}"));
        assert_ne!(tip, joined);
        assert_eq!(gh.base_of(8), "main");
        // The join's resolution is what the rebuilt head holds, and #7's
        // commits are gone from it.
        assert_eq!(
            run(&repo, &["git", "show", &format!("{tip}:joined.txt")]),
            "x and y\n"
        );
        let first_own = run(
            &repo,
            &[
                "git",
                "diff",
                "--name-only",
                &format!("{main}..{first_head}"),
            ],
        );
        let rebuilt = run(
            &repo,
            &["git", "diff", "--name-only", &format!("{main}..{tip}")],
        );
        for f in first_own.lines() {
            assert!(
                !rebuilt.lines().any(|r| r == f),
                "{f} is still on #8: {rebuilt}"
            );
        }
        assert!(rebuilt.lines().any(|r| r == "joined.txt"), "{rebuilt}");
        assert!(
            rebuilt.lines().any(|r| r == "hive_worker_c.txt"),
            "{rebuilt}"
        );
        let _ = first;
    }

    /// A follow-up job on a rebuilt pull request is cut from the rebuilt
    /// head and edits what the first job added; the first job's branch
    /// still holds the commits from before the rebuild. Those are on the
    /// forge's head by patch, so the build does not merge them again and
    /// the follow-up goes onto the pull request (#268 was held for the
    /// owner so on 26 November).
    #[test]
    fn a_follow_up_on_a_rebuilt_pull_request_is_delivered_past_the_first_jobs_old_branch() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        gh.track_remote(&bare);
        let (_first, second, _) = stacked_task(&s, &proj, &repo, &bare, &gh);
        let second_branch = task_branch(&second);
        let old_c = ref_of(&repo, "refs/heads/hive/worker/c");

        // main moves (another pull request merged), then #7 is closed: #8's
        // own commit is replayed onto the new main, a new commit.
        commit_on(&repo, "main", "elsewhere.txt");
        run(&repo, &["git", "push", "-q", "origin", "main"]);
        gh.set_state(7, "CLOSED");
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains("#8 rebuilt on main")),
            "{lines:?}"
        );
        let rebuilt = bare_ref(&bare, &format!("refs/heads/{second_branch}"));
        assert!(
            !crate::util::git_timeout(
                &repo,
                Duration::from_secs(10),
                &["merge-base", "--is-ancestor", &old_c, &rebuilt],
            )
            .ok(),
            "replayed, not kept"
        );
        assert_eq!(
            ref_of(&repo, "refs/heads/hive/worker/c"),
            old_c,
            "the first job's branch stays where it was"
        );

        // The review asks for changes; the follow-up edits c's file on the
        // rebuilt head.
        let fix = push_branch(&s, &proj, &repo, "hive/worker/d");
        run(&repo, &["git", "branch", "-f", "hive/worker/d", &rebuilt]);
        let d_tip = commit_on(&repo, "hive/worker/d", "hive_worker_c.txt");
        exec1(
            &s,
            "UPDATE jobs SET source = ? WHERE id = ?",
            &[&format!("task:{second}"), &fix.str("id")],
        );

        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines
                .iter()
                .any(|l| l.contains(&format!("{second}: #8 updated"))),
            "{lines:?}"
        );
        assert_eq!(events(&s, "land.pr-held"), 0);
        assert_eq!(events(&s, "land.rebase"), 0);
        let pushed = bare_ref(&bare, &format!("refs/heads/{second_branch}"));
        run(
            &repo,
            &["git", "merge-base", "--is-ancestor", &d_tip, &pushed],
        );
        assert!(
            !crate::util::git_timeout(
                &repo,
                Duration::from_secs(10),
                &["merge-base", "--is-ancestor", &old_c, &pushed],
            )
            .ok(),
            "the commits from before the rebuild are not merged again"
        );
        assert!(tend_proj(&s, &proj, TEST_NOW).is_empty());
    }

    #[test]
    fn a_pull_request_whose_rebuild_conflicts_is_held_where_it_is_and_its_task_told() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        gh.track_remote(&bare);
        // #7 changes `shared`; #8 changes the same lines on top of it, so
        // #8's own commit does not apply without #7's.
        let first = task_with_branches(&s, &proj, &repo, "Add the thing", &["hive/worker/a"]);
        commit_on(&repo, "hive/worker/a", "shared");
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines
                .iter()
                .any(|l| l.contains(&format!("{first}: #7 opened https"))),
            "{lines:?}"
        );
        let first_branch = task_branch(&first);
        let first_head = bare_ref(&bare, &format!("refs/heads/{first_branch}"));
        let second =
            task_with_branches(&s, &proj, &repo, "Add the other thing", &["hive/worker/c"]);
        // The own commit of #8 rewrites `shared` as #7 left it: without
        // #7 the file is not there to change.
        run(&repo, &["git", "checkout", "-q", "hive/worker/c"]);
        run(
            &repo,
            &[
                "git",
                "merge",
                "-q",
                "--no-edit",
                "-m",
                "take #7",
                &first_head,
            ],
        );
        std::fs::write(format!("{repo}/shared"), "theirs, then mine\n").unwrap();
        run(&repo, &["git", "add", "shared"]);
        run(&repo, &["git", "commit", "-q", "-m", "c over a"]);
        run(&repo, &["git", "checkout", "-q", "main"]);
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines
                .iter()
                .any(|l| l.contains(&format!("{second}: #8 opened on #7 https"))),
            "{lines:?}"
        );
        assert_eq!(gh.base_of(8), first_branch);
        let second_branch = task_branch(&second);
        let before = bare_ref(&bare, &format!("refs/heads/{second_branch}"));
        assert!(tend_proj(&s, &proj, TEST_NOW).is_empty());

        gh.set_state(7, "CLOSED");
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l
                == &format!(
                    "task pr proj: #8 not rebuilt on main without #7's commits: conflicts in shared; held; task {second} told"
                )),
            "{lines:?}"
        );
        // Nothing moved: the pull request still targets #7's branch and its
        // head is what it was, on the forge and locally.
        assert_eq!(gh.base_of(8), first_branch);
        assert_eq!(
            bare_ref(&bare, &format!("refs/heads/{second_branch}")),
            before
        );
        assert_eq!(
            ref_of(&repo, &format!("refs/heads/{second_branch}")),
            before
        );
        assert_eq!(events(&s, "land.pr-rebuilt"), 0);
        assert_eq!(events(&s, "land.pr-held"), 1);
        let held = last_detail(&s, "land.pr-held");
        assert_eq!(held["number"], 8);
        assert_eq!(held["task"], second);
        assert_eq!(held["closed"], 7);
        assert_eq!(held["base"], "main");
        assert_eq!(held["files"], json!(["shared"]));
        assert!(
            held["note"]
                .as_str()
                .unwrap_or_default()
                .contains("#8 was stacked on #7, which was closed unmerged; rebuilding"),
            "{held}"
        );
        let told = count(
            &s,
            "SELECT COUNT(*) AS n FROM messages WHERE task_id = ? AND body LIKE '%conflicts in shared%'",
            &[&second],
        );
        assert_eq!(told, 1);
        // The way out it names is a merge, never a copy of the commits.
        let body = crate::util::query_rows(
            &s.conn().unwrap(),
            "SELECT body FROM messages WHERE task_id = ? AND body LIKE '%conflicts in shared%'",
            &[&second],
        )
        .unwrap()[0]
            .str("body");
        assert!(body.contains("`git merge --no-ff`"), "{body}");
        assert!(!body.contains("git cherry-pick"), "{body}");
        assert!(
            crate::util::git_timeout(&repo, Duration::from_secs(10), &["worktree", "list"])
                .stdout
                .lines()
                .count()
                == 1,
            "the scratch worktree is gone"
        );

        // The next pass leaves it be: no second hold, no second message.
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(events(&s, "land.pr-held"), 1);
    }

    #[test]
    fn a_job_pull_request_takes_in_the_stack_top_and_is_rebuilt_without_it_when_it_closes() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        gh.track_remote(&bare);
        let first = task_with_two_branches(&s, &proj, &repo);
        tend_proj(&s, &proj, TEST_NOW);
        let first_head = bare_ref(&bare, &format!("refs/heads/{}", task_branch(&first)));
        let main = bare_ref(&bare, "refs/heads/main");
        let job = push_branch(&s, &proj, &repo, "hive/sec/x");
        let id = job.str("id");
        exec1(
            &s,
            "UPDATE jobs SET kind = 'security', source = 'security:proj:2026-09-06' WHERE id = ?",
            &[&id],
        );
        let tip = ref_of(&repo, "refs/heads/hive/sec/x");

        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert_eq!(
            lines,
            [format!(
                "job pr proj job {id}: #8 opened on #7 https://github.com/o/r/pull/8"
            )]
        );
        let pushed = bare_ref(&bare, "refs/heads/hive/sec/x");
        assert_ne!(pushed, tip, "the top was merged in");
        assert_eq!(ref_of(&repo, "refs/heads/hive/sec/x"), pushed);
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

        // The bottom is closed unmerged: #8 is rebuilt on main without
        // #7's commits (the merge that took them in goes too), which
        // `land.pr-rebuilt` says, once.
        gh.set_state(7, "CLOSED");
        let lines = tend_proj(&s, &proj, TEST_NOW);
        let rebuilt = bare_ref(&bare, "refs/heads/hive/sec/x");
        let dropped = run(
            &repo,
            &[
                "git",
                "rev-list",
                "--count",
                &format!("{main}..{first_head}"),
            ],
        )
        .trim()
        .to_string();
        assert!(
            lines.iter().any(|l| l
                == &format!(
                    "job pr proj: #8 rebuilt on main without #7's commits (1 of its own, {dropped} dropped), head now {}",
                    crate::recipe::short(&rebuilt, 12)
                )),
            "{lines:?}"
        );
        assert_eq!(gh.base_of(8), "main");
        assert_eq!(
            run(
                &repo,
                &["git", "rev-list", "--count", &format!("{main}..{rebuilt}")]
            )
            .trim(),
            "1"
        );
        assert_eq!(ref_of(&repo, "refs/heads/hive/sec/x"), rebuilt);
        let re = last_detail(&s, "land.pr-rebuilt");
        assert_eq!(re["kind"], "job");
        assert_eq!(re["number"], 8);
        assert_eq!(re["head"], "hive/sec/x");
        assert_eq!(re["old_base"], task_branch(&first));
        assert_eq!(re["base"], "main");
        assert_eq!(re["from"], pushed);
        assert_eq!(re["to"], rebuilt);
        assert_eq!(re["commits"], 1);
        assert_eq!(re["dropped"]["number"], 7);
        assert_eq!(re["dropped"]["commits"].to_string(), dropped);
        let pr = last_detail(&s, "land.pr");
        assert_eq!(pr["number"], 8);
        assert_eq!(pr["base"], "main");
        assert_eq!(pr["to"], rebuilt);
        assert_eq!(pr["from"], main);
        assert!(pr.get("stack_on").is_none(), "{pr}");
        assert!(tend_proj(&s, &proj, TEST_NOW).is_empty());
        assert_eq!(events(&s, "land.pr-rebuilt"), 1);
        assert_eq!(events(&s, "land.pr-unstacked"), 0);
    }

    #[test]
    fn a_conversation_pull_request_neither_stacks_nor_is_stacked_on() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let first = task_with_two_branches(&s, &proj, &repo);
        tend_proj(&s, &proj, TEST_NOW);
        run(&repo, &["git", "branch", "hive/chat-abc123", "main"]);
        let tip = commit_dated(
            &repo,
            "hive/chat-abc123",
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
            bare_ref(&bare, "refs/heads/hive/chat-abc123"),
            tip,
            "nothing merged in"
        );
        let pr = last_detail(&s, "land.pr");
        assert_eq!(pr["base"], "main");
        assert!(pr.get("stack_on").is_none(), "{pr}");

        // The next pull request stacks on the task's, not the conversation's.
        let job = push_branch(&s, &proj, &repo, "hive/sec/x");
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
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, _bare) = push_fixture(&s);
        let first = task_with_two_branches(&s, &proj, &repo);
        tend_proj(&s, &proj, TEST_NOW);
        let second =
            task_with_branches(&s, &proj, &repo, "Add the other thing", &["hive/worker/c"]);
        // c adds the file a's commit adds, with other content: it merges
        // onto main, not onto #7's head.
        commit_on(&repo, "hive/worker/c", "hive_worker_a.txt");
        let c = job_row_by_branch(&s, "hive/worker/c");

        let lines = tend_proj(&s, &proj, TEST_NOW);
        let onto = format!("{} (pull request #7)", task_branch(&first));
        assert!(
            lines.iter().any(|l| l.contains(&format!(
                "task pr proj {second}: hive/worker/c (job {c}) does not merge cleanly onto {onto}"
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
                "Merge hive/worker/c onto {}: Add the other thing",
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
