//! Where a task's code stands once its steps are done: waiting for the
//! task's pull request, the pull request open, under review, handed to the
//! owner, merged, or held because it could not be opened - plus the older
//! landing states. The hub's task sheet shows the state with its next step,
//! and the task feed lists the same events, so a lead agent that reads the
//! feed tells the owner the same story. The same reading, per pull request
//! rather than per task, is `pr_states_for`: the Sessions page and a
//! project's pull request list word every Hive pull request from it.

use std::collections::BTreeMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// The remote branch Hive pushes the default branch to for a pull request.
pub const PR_HEAD: &str = "hive/landed";

/// The events that say where a pull request stands, every one carrying its
/// `number`: the delivery's own (`land.pr` opened or brought up to date,
/// `land.merged`, `land.pr-closed`) and the reviewer's (`review.queued`,
/// `review.owner`, `review.changes`, `review.failed`, `review.merged`), plus
/// the owner's `review.ask`.
pub const PR_ACTIONS: [&str; 9] = [
    "land.pr",
    "land.merged",
    "land.pr-closed",
    "review.queued",
    "review.owner",
    "review.ask",
    "review.changes",
    "review.failed",
    "review.merged",
];

/// The event the hub writes when the owner clicks Merge on a pull request
/// the reviewer handed over (or whose review failed); the daemon merges it
/// on GitHub as the owner on its next pass and answers with
/// `review.merged`, or `review.failed` when the merge did not go through.
pub const MERGE_ASKED: &str = "review.ask";

/// How long a merged or closed pull request stays in `pr_states_for`. The
/// local default branch catches up with a merge on the daemon's next pass
/// (minutes), and a page that groups by the pull request's state must still
/// say "merged" meanwhile instead of "waiting for its pull request"; two
/// weeks also keeps the project's list readable as a recent history.
const SETTLED_DAYS: i64 = 14;

/// The event the hub or `hive land --pr` writes when the owner asks for a
/// pull request; the daemon answers it on its next landing pass.
pub const PR_ASKED: &str = "land.pr-asked";

/// The events that answer an ask: a pull request opened or brought up to
/// date, a plain push of the default branch, or the daemon's word that the
/// default branch had nothing the upstream lacks.
pub const PR_ANSWERS: [&str; 3] = ["land.pr", "land.pushed", "land.pr-answered"];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskDelivery {
    /// `none`: nothing finished yet. `waiting`: finished steps' branches
    /// wait for the task's pull request (opened once every job is done).
    /// `held`: the pull request could not be opened (`note` says why).
    /// `pr_open`: the task's pull request is open (`url`, `number`) and no
    /// review has started; `in_review`: the reviewer is at it;
    /// `needs_owner`: the reviewer handed it to the owner (`reasons`,
    /// `note`); `merge_asked`: the owner clicked Merge and the daemon is
    /// merging; `changes_requested`: the reviewer asked for changes
    /// (`note`, `follow_up` names the job); `review_failed`: the review or
    /// the merge did not go through (`note`). `merged`: it was merged and
    /// the task is done. Older work may still show `landed`, `pushed`, `pr`
    /// or `asked` from the local landings.
    pub state: String,
    /// Landings the task's steps made on the default branch.
    pub landed: i64,
    /// Finished steps whose branch is not delivered yet.
    pub waiting: i64,
    /// The newest landed commit.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub sha: String,
    /// When the state was reached.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub at: String,
    /// `origin/main`: where a push or pull request went.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub remote: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub number: i64,
    /// The hold note, or why no delivery is coming on its own.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// What GitHub Actions made of the pushed commit, when the daemon watches
    /// pipelines (`pipeline.watch`): `watching` until the runs finish, then
    /// `passed`, `failed`, `none` (nothing ran) or `gave-up`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub pipeline: String,
    /// The fix job the daemon planned for a red pipeline.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub pipeline_job: String,
    /// Whether asking for a pull request does something now.
    pub can_ask: bool,
    /// Why the reviewer handed the pull request to the owner (`needs_owner`:
    /// the complexity rules it tripped), as the reviewer worded them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<String>,
    /// The job queued to address the review (`changes_requested`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub follow_up: String,
    /// The pull request's kind: `task` for the task's own pull request;
    /// empty before one exists.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub kind: String,
    /// The branch the pull request merges into (`main`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub base: String,
    /// Whether the owner's Merge does something now: only once the reviewer
    /// handed the pull request over or its review failed.
    pub can_merge: bool,
}

/// Where one of the project's pull requests stands, whatever it carries: a
/// task's work, one job's, an experiment's, or a conversation's branch
/// proposed as is. The Sessions page matches a conversation to its pull
/// request by `conversation` or `head`; a task's own sheet reads the same
/// states through `delivery_for`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrState {
    pub number: i64,
    pub url: String,
    pub title: String,
    /// `task`, `job`, `experiment` or `proposed`; empty for a pull request
    /// opened before the kinds were named.
    pub kind: String,
    pub task: String,
    pub job: String,
    pub experiment: String,
    /// The chat whose branch the pull request proposes.
    pub conversation: String,
    pub head: String,
    pub base: String,
    /// `pr_open`, `in_review`, `needs_owner`, `merge_asked`,
    /// `changes_requested`, `review_failed`, `merged` or `closed` (closed
    /// without merging), as `TaskDelivery::state` words them.
    pub state: String,
    pub reasons: Vec<String>,
    pub note: String,
    pub follow_up: String,
    /// When the state was reached.
    pub at: String,
    /// When the pull request was opened.
    pub opened_at: String,
    pub can_merge: bool,
}

impl PrState {
    /// Merged or closed: nothing waits on anyone.
    pub fn settled(&self) -> bool {
        self.state == "merged" || self.state == "closed"
    }
}

/// A detail value's field as text: strings as they are, numbers rendered.
fn text_of(v: &serde_json::Value, key: &str) -> String {
    match v.get(key) {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// A detail value's field as a list of words (`reasons`); anything that is
/// not a list of strings reads as none.
fn list_of(v: &serde_json::Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(serde_json::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Where a pull request stands, read from its events oldest first. The
/// newest event names the state: `land.pr` (opened, or brought up to date
/// with new commits) puts it back to `pr_open` because the reviewer reads
/// the new head afresh; `review.queued` is `in_review`, or `needs_owner`
/// right away when its `reasons` say the reviewer will hand it over;
/// `review.owner` is `needs_owner`; the owner's `review.ask` is
/// `merge_asked`; `review.changes` is `changes_requested`; `review.failed`
/// is `review_failed`; `review.merged` or `land.merged` is `merged` and
/// `land.pr-closed` is `closed` - both final, so a review event that
/// arrives late changes nothing. The Merge button is offered only in
/// `needs_owner` and `review_failed`: everywhere else the reviewer or the
/// daemon is still acting, and a second ask would race it.
pub fn pr_state_of(events: &[LandEvent]) -> PrState {
    let mut p = PrState::default();
    for e in events {
        let v: serde_json::Value = serde_json::from_str(&e.detail).unwrap_or_default();
        let keep = |field: &mut String, key: &str| {
            let t = text_of(&v, key);
            if !t.is_empty() {
                *field = t;
            }
        };
        if p.settled() && e.action.starts_with("review.") {
            continue;
        }
        match e.action.as_str() {
            "land.pr" => {
                p.number = text_of(&v, "number").parse().unwrap_or(p.number);
                for (field, key) in [
                    (&mut p.url, "url"),
                    (&mut p.title, "title"),
                    (&mut p.kind, "kind"),
                    (&mut p.task, "task"),
                    (&mut p.job, "job"),
                    (&mut p.experiment, "experiment"),
                    (&mut p.conversation, "conversation"),
                    (&mut p.head, "head"),
                    (&mut p.base, "base"),
                ] {
                    keep(field, key);
                }
                if p.opened_at.is_empty() {
                    p.opened_at = e.at.clone();
                }
                p.state = "pr_open".into();
                p.reasons.clear();
                p.note.clear();
                p.follow_up.clear();
            }
            "land.merged" | "review.merged" => {
                keep(&mut p.url, "url");
                p.state = "merged".into();
                p.reasons.clear();
                p.note.clear();
                p.follow_up.clear();
            }
            "land.pr-closed" => {
                keep(&mut p.url, "url");
                p.state = "closed".into();
                p.reasons.clear();
                p.note.clear();
                p.follow_up.clear();
            }
            "review.queued" => {
                p.reasons = list_of(&v, "reasons");
                p.state = if p.reasons.is_empty() {
                    "in_review"
                } else {
                    "needs_owner"
                }
                .into();
                p.note.clear();
                p.follow_up.clear();
            }
            "review.owner" => {
                let reasons = list_of(&v, "reasons");
                if !reasons.is_empty() {
                    p.reasons = reasons;
                }
                p.note = text_of(&v, "note");
                p.follow_up.clear();
                p.state = "needs_owner".into();
            }
            MERGE_ASKED => {
                p.state = "merge_asked".into();
                p.reasons.clear();
                p.note.clear();
                p.follow_up.clear();
            }
            "review.changes" => {
                p.state = "changes_requested".into();
                p.note = text_of(&v, "note");
                p.follow_up = text_of(&v, "follow_up");
            }
            "review.failed" => {
                p.state = "review_failed".into();
                p.note = text_of(&v, "error");
                p.follow_up.clear();
            }
            _ => continue,
        }
        p.at = e.at.clone();
    }
    p.can_merge = matches!(p.state.as_str(), "needs_owner" | "review_failed");
    p
}

/// The events of one pull request of the project, oldest first.
fn pr_events(conn: &rusqlite::Connection, project: &str, number: i64) -> Result<Vec<LandEvent>> {
    if number == 0 {
        return Ok(Vec::new());
    }
    let marks = vec!["?"; PR_ACTIONS.len()].join(",");
    let mut stmt = conn.prepare(&format!(
        "SELECT seq, at, actor, action, detail FROM events WHERE subject = ? AND action IN ({marks}) \
         AND json_extract(detail, '$.number') = ? ORDER BY seq"
    ))?;
    let mut params: Vec<rusqlite::types::Value> =
        vec![rusqlite::types::Value::Text(project.into())];
    params.extend(
        PR_ACTIONS
            .iter()
            .map(|a| rusqlite::types::Value::Text((*a).to_string())),
    );
    params.push(rusqlite::types::Value::Integer(number));
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params), |r| {
            Ok(LandEvent {
                seq: r.get(0)?,
                at: r.get(1)?,
                actor: r.get(2)?,
                action: r.get(3)?,
                detail: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// The oldest `at` a merged or closed pull request may have and still be listed.
fn settled_cutoff(now: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(now)
        .map(|t| {
            (t.with_timezone(&chrono::Utc) - chrono::Duration::days(SETTLED_DAYS))
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        })
        .unwrap_or_default()
}

/// Every pull request Hive opened for the project that is still open, plus
/// the ones merged or closed in the last `SETTLED_DAYS`, newest movement
/// first. The project-level pull request of the older landings (head
/// `hive/landed`) is left out: nothing polls it, so it would read as open
/// for ever.
pub fn pr_states_for(s: &hive_store::Store, project: &str) -> Result<Vec<PrState>> {
    let conn = s.conn()?;
    let mut by_number: BTreeMap<i64, Vec<LandEvent>> = BTreeMap::new();
    for e in land_events(&conn, &[project.to_string()], &PR_ACTIONS, 0)? {
        let number: i64 = field(&e.detail, "number").parse().unwrap_or(0);
        if number != 0 {
            by_number.entry(number).or_default().push(e);
        }
    }
    let cutoff = settled_cutoff(&s.now());
    let mut out: Vec<PrState> = by_number
        .into_values()
        .map(|events| pr_state_of(&events))
        .filter(|p| !p.state.is_empty() && p.head != PR_HEAD)
        .filter(|p| !p.settled() || p.at >= cutoff)
        .collect();
    out.sort_by(|a, b| b.at.cmp(&a.at).then(b.number.cmp(&a.number)));
    Ok(out)
}

/// The owner asks the daemon to merge a pull request the reviewer handed
/// over. The daemon's next pass merges it on GitHub as the owner and
/// answers with `review.merged`; the state reads `merge_asked` meanwhile.
pub fn ask_merge(
    s: &hive_store::Store,
    project: &str,
    number: i64,
    task: &str,
    actor: &str,
) -> Result<()> {
    let mut detail = serde_json::json!({
        "project": project,
        "number": number,
        "action": "merge",
    });
    if !task.is_empty() {
        detail["task"] = serde_json::Value::String(task.to_string());
    }
    hive_bus::emit(s, actor, MERGE_ASKED, project, detail)?;
    Ok(())
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

/// One `land.*` event of a project or branch, as the feed and the state
/// read it.
#[derive(Debug, Clone)]
pub struct LandEvent {
    pub seq: i64,
    pub at: String,
    pub actor: String,
    pub action: String,
    pub detail: String,
}

/// A detail field as text: strings as they are, numbers rendered (a pull
/// request `number` is numeric), anything else empty.
pub(crate) fn field(detail: &str, key: &str) -> String {
    serde_json::from_str::<serde_json::Value>(detail)
        .map(|v| text_of(&v, key))
        .unwrap_or_default()
}

/// The task's steps and the branches they worked on: the jobs' ids, their
/// current branch and every run's branch (a landed job's branch column is
/// emptied, the run keeps it and the landing event names the job).
struct TaskWork {
    jobs: Vec<String>,
    branches: Vec<String>,
    /// Finished steps whose branch still waits to land.
    waiting: i64,
}

fn task_work(conn: &rusqlite::Connection, task: &crate::Task) -> Result<TaskWork> {
    let (exact, like) = hive_store::task_source_params(&task.id);
    let mut jobs: Vec<String> = Vec::new();
    let mut branches: Vec<String> = Vec::new();
    let mut waiting = 0;
    let mut stmt = conn.prepare(&format!(
        "SELECT j.id, j.status, j.branch, \
                (SELECT group_concat(r.branch, '\n') FROM runs r WHERE r.job_id = j.id AND r.branch != ''), \
                EXISTS (SELECT 1 FROM runs r WHERE r.job_id = j.id AND r.status = 'running') \
         FROM jobs j WHERE j.project = ? AND ({} OR (? != '' AND j.id = ?)) ORDER BY j.created_at, j.id",
        hive_store::task_source_clause("j.source")
    ))?;
    let rows = stmt
        .query_map(
            rusqlite::params![task.project, exact, like, task.job_id, task.job_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    r.get::<_, i64>(4)? != 0,
                ))
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, status, branch, run_branches, running) in rows {
        jobs.push(id);
        if !branch.is_empty() {
            // The worker marks its job done as its session ends; the branch
            // it names exists once the run's verification has passed. Until
            // the run ends the step is still at work, not waiting to land.
            if status == "done" && !running {
                waiting += 1;
            }
            if !branches.contains(&branch) {
                branches.push(branch);
            }
        }
        for b in run_branches.lines() {
            if !b.is_empty() && !branches.iter().any(|x| x == b) {
                branches.push(b.to_string());
            }
        }
    }
    Ok(TaskWork {
        jobs,
        branches,
        waiting,
    })
}

/// The landings of the task's work: `land.landed` on one of its branches,
/// or naming one of its jobs.
fn landings_of(conn: &rusqlite::Connection, work: &TaskWork) -> Result<Vec<LandEvent>> {
    let mut landings = land_events(conn, &work.branches, &["land.landed"], 0)?;
    if !work.jobs.is_empty() {
        let marks = vec!["?"; work.jobs.len()].join(",");
        let mut stmt = conn.prepare(&format!(
            "SELECT seq, at, actor, action, detail FROM events \
             WHERE action = 'land.landed' AND json_valid(detail) AND json_extract(detail, '$.job') IN ({marks}) ORDER BY seq"
        ))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(work.jobs.iter()), |r| {
                Ok(LandEvent {
                    seq: r.get(0)?,
                    at: r.get(1)?,
                    actor: r.get(2)?,
                    action: r.get(3)?,
                    detail: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for e in rows {
            if !landings.iter().any(|l| l.seq == e.seq) {
                landings.push(e);
            }
        }
        landings.sort_by_key(|e| e.seq);
    }
    Ok(landings)
}

fn land_events(
    conn: &rusqlite::Connection,
    subjects: &[String],
    actions: &[&str],
    after_seq: i64,
) -> Result<Vec<LandEvent>> {
    if subjects.is_empty() {
        return Ok(Vec::new());
    }
    let marks = |n: usize| vec!["?"; n].join(",");
    let sql = format!(
        "SELECT seq, at, actor, action, subject, detail FROM events \
         WHERE subject IN ({}) AND action IN ({}) AND seq > ? ORDER BY seq",
        marks(subjects.len()),
        marks(actions.len())
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut params: Vec<rusqlite::types::Value> = subjects
        .iter()
        .map(|s| rusqlite::types::Value::Text(s.clone()))
        .collect();
    params.extend(
        actions
            .iter()
            .map(|a| rusqlite::types::Value::Text((*a).to_string())),
    );
    params.push(rusqlite::types::Value::Integer(after_seq));
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params), |r| {
            Ok(LandEvent {
                seq: r.get(0)?,
                at: r.get(1)?,
                actor: r.get(2)?,
                action: r.get(3)?,
                detail: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// A `land.held` about the push or the pull request, not about another
/// job's landing: the daemon marks a push-only hold `push-failed`, and
/// appends a push note to a landing hold with these words.
fn push_related_hold(detail: &str) -> bool {
    let note = field(detail, "note");
    field(detail, "state") == "push-failed"
        || note.contains("not pushed")
        || note.contains("not delivered")
        || note.contains("pull request")
}

/// The project-level delivery events that belong to a task's landings: from
/// the first landing on, up to and including the first push or pull request
/// after the last one. Later events belong to later work.
pub fn delivery_events(
    conn: &rusqlite::Connection,
    project: &str,
    first_landed_seq: i64,
    last_landed_seq: i64,
) -> Result<Vec<LandEvent>> {
    let mut out = Vec::new();
    for e in land_events(
        conn,
        &[project.to_string()],
        &[
            "land.pushed",
            "land.pr",
            PR_ASKED,
            "land.pr-answered",
            "land.held",
        ],
        first_landed_seq,
    )? {
        if e.action == "land.held" && !push_related_hold(&e.detail) {
            continue;
        }
        let delivered = e.action == "land.pushed" || e.action == "land.pr";
        out.push(e.clone());
        if delivered && e.seq > last_landed_seq {
            break;
        }
    }
    Ok(out)
}

/// The `land.landed` events of a task's branches, oldest first.
pub fn task_landings(conn: &rusqlite::Connection, task: &crate::Task) -> Result<Vec<LandEvent>> {
    landings_of(conn, &task_work(conn, task)?)
}

/// The newest pull-request event of the task's own pull request
/// (`land.pr`, `land.merged`, `land.pr-closed`, `land.pr-held`, detail
/// `task`), written by the daemon's per-task delivery.
fn task_pr_event(
    conn: &rusqlite::Connection,
    project: &str,
    task: &str,
) -> Result<Option<LandEvent>> {
    let mut stmt = conn.prepare(
        "SELECT seq, at, actor, action, detail FROM events WHERE subject = ? \
         AND action IN ('land.pr', 'land.merged', 'land.pr-closed', 'land.pr-held') \
         AND json_extract(detail, '$.task') = ? ORDER BY seq DESC LIMIT 1",
    )?;
    let mut rows = stmt.query_map(rusqlite::params![project, task], |r| {
        Ok(LandEvent {
            seq: r.get(0)?,
            at: r.get(1)?,
            actor: r.get(2)?,
            action: r.get(3)?,
            detail: r.get(4)?,
        })
    })?;
    Ok(rows.next().transpose()?)
}

/// Where the task's code stands now.
pub fn delivery_for(s: &hive_store::Store, task_id: &str) -> Result<TaskDelivery> {
    let (task, _) = crate::get(s, task_id)?;
    let conn = s.conn()?;
    let work = task_work(&conn, &task)?;
    let landings = landings_of(&conn, &work)?;
    let mut d = TaskDelivery {
        landed: landings.len() as i64,
        waiting: work.waiting,
        ..TaskDelivery::default()
    };
    if let Some(e) = task_pr_event(&conn, &task.project, &task.id)? {
        d.at = e.at.clone();
        d.url = field(&e.detail, "url");
        d.number = field(&e.detail, "number").parse().unwrap_or(0);
        match e.action.as_str() {
            "land.pr" => {
                d.remote = format!(
                    "{}/{}",
                    field(&e.detail, "remote"),
                    field(&e.detail, "base")
                );
                d.base = field(&e.detail, "base");
                d.sha = field(&e.detail, "to");
                (d.pipeline, d.pipeline_job) = pipeline_verdict(s, &conn, &task.project, &d.sha)?;
                // The review events name the pull request by number, not
                // by task, so the state is read per number.
                let pr = pr_state_of(&pr_events(&conn, &task.project, d.number)?);
                if pr.state != "closed" {
                    d.state = if pr.state.is_empty() {
                        "pr_open".into()
                    } else {
                        pr.state
                    };
                    d.kind = if pr.kind.is_empty() {
                        "task".into()
                    } else {
                        pr.kind
                    };
                    if !pr.at.is_empty() {
                        d.at = pr.at;
                    }
                    if !pr.url.is_empty() {
                        d.url = pr.url;
                    }
                    d.reasons = pr.reasons;
                    d.note = pr.note;
                    d.follow_up = pr.follow_up;
                    d.can_merge = pr.can_merge;
                    return Ok(d);
                }
                // Closed unmerged under a number-only event: the task is
                // back in progress, like a closing that named the task.
                d.remote = String::new();
                d.base = String::new();
                d.sha = String::new();
                (d.pipeline, d.pipeline_job) = (String::new(), String::new());
            }
            "land.merged" => {
                d.state = "merged".into();
                d.kind = "task".into();
                return Ok(d);
            }
            "land.pr-held" if work.waiting > 0 => {
                d.state = "held".into();
                d.note = field(&e.detail, "note");
                return Ok(d);
            }
            // Closed unmerged: the task is back in progress; what its next
            // jobs leave waits again.
            _ => {}
        }
    }
    let Some(last) = landings.last() else {
        d.state = if work.waiting > 0 { "waiting" } else { "none" }.into();
        return Ok(d);
    };
    d.sha = serde_json::from_str::<serde_json::Value>(&last.detail)
        .ok()
        .and_then(|v| {
            v.pointer("/code_delivery/after")
                .or_else(|| v.get("after"))
                .and_then(|a| a.as_str())
                .map(ToString::to_string)
        })
        .unwrap_or_default();
    d.at = last.at.clone();
    d.state = "landed".into();
    let events = delivery_events(&conn, &task.project, landings[0].seq, last.seq)?;
    let mut answered_seq = 0;
    let mut asked: Option<&LandEvent> = None;
    for e in &events {
        match e.action.as_str() {
            "land.pushed" | "land.pr" if e.seq > last.seq => {
                d.state = if e.action == "land.pr" {
                    "pr"
                } else {
                    "pushed"
                }
                .into();
                d.at = e.at.clone();
                d.remote = format!(
                    "{}/{}",
                    field(&e.detail, "remote"),
                    if e.action == "land.pr" {
                        field(&e.detail, "base")
                    } else {
                        field(&e.detail, "branch")
                    }
                );
                d.url = field(&e.detail, "url");
                d.number = field(&e.detail, "number").parse().unwrap_or(0);
                d.note = String::new();
                d.can_ask = false;
                (d.pipeline, d.pipeline_job) =
                    pipeline_verdict(s, &conn, &task.project, &field(&e.detail, "to"))?;
                return Ok(d);
            }
            a if PR_ANSWERS.contains(&a) => {
                answered_seq = e.seq;
                asked = None;
            }
            a if a == PR_ASKED => asked = Some(e),
            "land.held" if e.seq > last.seq => {
                d.state = "held".into();
                d.at = e.at.clone();
                d.note = field(&e.detail, "note");
            }
            _ => {}
        }
    }
    if let Some(a) = asked.filter(|a| a.seq > answered_seq) {
        d.state = "asked".into();
        d.at = a.at.clone();
        d.note = String::new();
        d.can_ask = false;
        return Ok(d);
    }
    if d.state == "landed" && s.setting("landing.push", "off") == "off" {
        d.note = "Hive does not push for this machine (landing.push is off); you push, or ask for a pull request.".into();
    }
    d.can_ask = true;
    Ok(d)
}

/// The daemon's verdict on the pipeline of the pushed commit `sha`
/// (`pipeline.*` events, the runner's pipeline watcher) and the fix job it
/// planned, as (verdict, job). `watching` while no verdict is in and the
/// watcher is on - a rerun of the failed jobs for a fresh log is not a
/// verdict yet; empty when it is off.
fn pipeline_verdict(
    s: &hive_store::Store,
    conn: &rusqlite::Connection,
    project: &str,
    sha: &str,
) -> Result<(String, String)> {
    if sha.is_empty() {
        return Ok((String::new(), String::new()));
    }
    let mut stmt = conn.prepare(
        "SELECT action, detail FROM events WHERE subject = ?1 AND action LIKE 'pipeline.%' AND action NOT IN ('pipeline.unreadable', 'pipeline.rerun') AND json_extract(detail, '$.sha') = ?2 ORDER BY seq DESC LIMIT 1",
    )?;
    let verdict = stmt
        .query_row(rusqlite::params![project, sha], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .ok();
    Ok(match verdict {
        Some((action, detail)) => (
            action.trim_start_matches("pipeline.").to_string(),
            field(&detail, "job"),
        ),
        None if s.setting("pipeline.watch", "fix") != "off" => ("watching".into(), String::new()),
        None => (String::new(), String::new()),
    })
}

/// The owner asks for the project's landed work as a pull request. The
/// daemon's next landing pass pushes the default branch to `hive/landed`
/// and opens or updates the pull request, whatever `landing.push` says.
pub fn ask_pull_request(
    s: &hive_store::Store,
    project: &str,
    task_id: &str,
    actor: &str,
) -> Result<()> {
    hive_bus::emit(
        s,
        actor,
        PR_ASKED,
        project,
        hive_store::Detail::json(serde_json::json!({
            "project": project,
            "task": task_id,
            "head": PR_HEAD,
        }))?,
    )?;
    Ok(())
}

/// Whether an ask waits for the daemon: one newer than the project's last
/// push, pull request or "nothing to deliver" answer.
pub fn pull_request_asked(s: &hive_store::Store, project: &str) -> bool {
    let Ok(conn) = s.conn() else {
        return false;
    };
    let asked: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(seq), 0) FROM events WHERE action = ? AND subject = ?",
            rusqlite::params![PR_ASKED, project],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if asked == 0 {
        return false;
    }
    let answered: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(seq), 0) FROM events WHERE action IN (?, ?, ?) AND subject = ?",
            rusqlite::params![PR_ANSWERS[0], PR_ANSWERS[1], PR_ANSWERS[2], project],
            |r| r.get(0),
        )
        .unwrap_or(0);
    asked > answered
}
