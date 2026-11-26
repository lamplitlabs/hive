//! Where a task stands in the lifecycle (TASK-141, the AI-native SDLC of
//! `docs/sdlc.md`): the phase it is in, who it waits on, and for every
//! phase of its path what the phase left behind and the gate that let it
//! pass.
//!
//! No ledger is kept for it: the phase is derived from rows the store
//! already holds - the task's status, the jobs whose source names the task
//! (by kind, status, branch and whether a run is still going), the task's
//! own `task.*` events, and the `land.*` and `review.*` events of the
//! task's pull request. [`derive_lifecycle`] is the pure reading over those
//! rows, so every phase and gate is tested without a store; [`lifecycle_for`]
//! and [`lifecycles_for`] read the rows and call it, for the hub's task
//! page and a project's tasks by phase.
//!
//! The rule: a task is in the first phase of its path whose gate has not
//! passed. Review and delivery interleave (the runner opens the pull
//! request, the reviewer rules, the runner finishes after the merge), so a
//! task whose steps are all finished is in Review until a verdict of merge,
//! and in Deliver while that merge waits on its checks or the daemon.

use std::collections::BTreeMap;

use anyhow::Result;
use serde::Serialize;

use crate::delivery::{LandEvent, PR_ACTIONS, field, pr_state_of};

/// The ten phases of the lifecycle in the order a change passes them, as
/// `docs/sdlc.md` names them, plus where a task ends (`done`, `cancelled`).
/// A task is only ever derived into Intake through Deliver, Done or
/// Cancelled: Direction stands above every phase, and Release, Operate and
/// Learn are a project's, read from its schedules, pipelines and reflection
/// rather than from one task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Direction,
    Intake,
    Shape,
    Build,
    Verify,
    Review,
    Deliver,
    Release,
    Operate,
    Learn,
    Done,
    Cancelled,
}

impl Phase {
    /// The ten phases of `docs/sdlc.md`, in order.
    pub const ALL: [Phase; 10] = [
        Phase::Direction,
        Phase::Intake,
        Phase::Shape,
        Phase::Build,
        Phase::Verify,
        Phase::Review,
        Phase::Deliver,
        Phase::Release,
        Phase::Operate,
        Phase::Learn,
    ];

    /// The phases a task passes, in order: its path through the lifecycle.
    pub const TASK_PATH: [Phase; 6] = [
        Phase::Intake,
        Phase::Shape,
        Phase::Build,
        Phase::Verify,
        Phase::Review,
        Phase::Deliver,
    ];

    /// The word stored and sent to the console.
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Direction => "direction",
            Phase::Intake => "intake",
            Phase::Shape => "shape",
            Phase::Build => "build",
            Phase::Verify => "verify",
            Phase::Review => "review",
            Phase::Deliver => "deliver",
            Phase::Release => "release",
            Phase::Operate => "operate",
            Phase::Learn => "learn",
            Phase::Done => "done",
            Phase::Cancelled => "cancelled",
        }
    }

    /// The phase's name as `docs/sdlc.md` heads it.
    pub fn label(self) -> &'static str {
        match self {
            Phase::Direction => "Direction",
            Phase::Intake => "Intake",
            Phase::Shape => "Shape and plan",
            Phase::Build => "Build",
            Phase::Verify => "Verify",
            Phase::Review => "Review",
            Phase::Deliver => "Deliver",
            Phase::Release => "Release",
            Phase::Operate => "Operate",
            Phase::Learn => "Learn",
            Phase::Done => "Done",
            Phase::Cancelled => "Cancelled",
        }
    }

    /// The gate from this phase to the next one on a task's path, in the
    /// words of `docs/sdlc.md`'s gates; empty for a phase off the path.
    pub fn gate(self) -> &'static str {
        match self {
            Phase::Intake => {
                "a lead takes the task up: the owner's task passes no other gate, and suggested work came with a different agent's verdict"
            }
            Phase::Shape => "a job exists for the task, Tier 0 or Tier 1",
            Phase::Build => "every step's run has ended",
            Phase::Verify => {
                "each step's own commits passed the project's check, and none of the task's jobs is queued, running or blocked"
            }
            Phase::Review => {
                "the task's pull request is open and the reviewer (or the owner) says merge, with GitHub's own gate passed"
            }
            Phase::Deliver => "the pull request is merged on the upstream",
            _ => "",
        }
    }

    fn path_index(self) -> Option<usize> {
        Phase::TASK_PATH.iter().position(|p| *p == self)
    }
}

/// Who the task waits on now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitsOn {
    /// Nobody: the task is done or cancelled.
    #[default]
    Nobody,
    /// The owner: a question, a hand-over, a Merge to click.
    Owner,
    /// The project's lead: to take the task up or plan its next step.
    Lead,
    /// A worker agent at its run.
    Agent,
    /// The reviewer agent.
    Reviewer,
    /// The runner, without a model: a free slot, the end check, opening or
    /// merging the pull request.
    Runner,
    /// GitHub: the pull request's checks or the merge itself.
    Github,
}

/// Where one phase of a task's path stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    /// Its gate passed.
    Passed,
    /// The task is in it.
    Current,
    /// Not reached yet.
    #[default]
    Ahead,
    /// Passed over: the task ended without it (closed with no code, or
    /// cancelled before it).
    Skipped,
}

/// One thing a phase left behind, every id a link in the console: the task,
/// the lead's sitting, a job, a branch, the pull request, the reviewer's
/// verdict, the merge.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Mark {
    /// `task`, `origin`, `lead`, `job`, `branch`, `pr`, `verdict` or `merge`.
    pub kind: String,
    /// The id: `TASK-001`, a job id, a branch, a pull request number.
    pub id: String,
    /// A word on it: a job's status, the verdict's state.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub at: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub url: String,
}

impl Mark {
    fn new(kind: &str, id: impl Into<String>) -> Mark {
        Mark {
            kind: kind.to_string(),
            id: id.into(),
            ..Mark::default()
        }
    }
    fn note(mut self, note: impl Into<String>) -> Mark {
        self.note = note.into();
        self
    }
    fn at(mut self, at: impl Into<String>) -> Mark {
        self.at = at.into();
        self
    }
    fn url(mut self, url: impl Into<String>) -> Mark {
        self.url = url.into();
        self
    }
}

/// One phase of the task's path: where it stands, when the task entered it
/// (when the rows say), what it left behind and its gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PhaseStep {
    pub phase: Phase,
    pub state: StepState,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub at: String,
    pub left: Vec<Mark>,
    pub gate: &'static str,
}

/// Where a task stands in the lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskLifecycle {
    pub task: String,
    pub phase: Phase,
    pub waits_on: WaitsOn,
    /// One sentence on what holds the task, or how it ended.
    pub note: String,
    /// True when work went round again: a step queued after a pull request
    /// was already open, or a review that asked for changes.
    pub rework: bool,
    /// The phases of the task's path in order.
    pub steps: Vec<PhaseStep>,
}

/// One job whose source names the task, as the derivation reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LifecycleJob {
    pub id: String,
    pub kind: String,
    /// `queued`, `leased`, `blocked`, `done`, `failed` or `cancelled`.
    pub status: String,
    /// The job's branch, or the branch its run left when the column was
    /// emptied by a landing.
    pub branch: String,
    pub created_at: String,
    pub finished_at: String,
    /// A run of the job is still going: for a `done` job, the runner's end
    /// check of its commits (Verify).
    pub running: bool,
    /// The job's result line: why a blocked job is blocked.
    pub result: String,
}

impl LifecycleJob {
    fn active(&self) -> bool {
        matches!(self.status.as_str(), "queued" | "leased" | "blocked")
            || (self.status == "done" && self.running)
    }
    /// Finished with a branch to deliver.
    fn delivers(&self) -> bool {
        self.status == "done" && !self.running && !self.branch.is_empty()
    }
}

/// The task's pull request as its events tell it.
struct TaskPr {
    /// The newest task pull-request event's action: `land.pr`,
    /// `land.merged`, `land.pr-closed` or `land.pr-held`.
    last: String,
    number: i64,
    url: String,
    /// `PrState::state` for an open one (`pr_open`, `in_review`, ...).
    state: String,
    note: String,
    follow_up: String,
    opened_at: String,
    at: String,
    /// A review asked for changes at some point.
    changes_asked: bool,
}

fn task_pr(task_id: &str, events: &[LandEvent]) -> Option<TaskPr> {
    let mine = |e: &&LandEvent| {
        matches!(
            e.action.as_str(),
            "land.pr" | "land.merged" | "land.pr-closed" | "land.pr-held"
        ) && field(&e.detail, "task") == task_id
    };
    let newest = events.iter().filter(mine).max_by_key(|e| e.seq)?;
    let mut pr = TaskPr {
        last: newest.action.clone(),
        number: field(&newest.detail, "number").parse().unwrap_or(0),
        url: field(&newest.detail, "url"),
        state: String::new(),
        note: field(&newest.detail, "note"),
        follow_up: String::new(),
        opened_at: String::new(),
        at: newest.at.clone(),
        changes_asked: false,
    };
    if pr.number == 0 {
        // A hold names no number; the number of the task's last opened
        // pull request still links the page.
        if let Some(open) = events
            .iter()
            .filter(mine)
            .filter(|e| e.action == "land.pr")
            .max_by_key(|e| e.seq)
        {
            pr.number = field(&open.detail, "number").parse().unwrap_or(0);
            pr.url = field(&open.detail, "url");
        }
    }
    if pr.number != 0 {
        let number = pr.number.to_string();
        let mut of_pr: Vec<LandEvent> = events
            .iter()
            .filter(|e| PR_ACTIONS.contains(&e.action.as_str()))
            .filter(|e| field(&e.detail, "number") == number)
            .cloned()
            .collect();
        of_pr.sort_by_key(|e| e.seq);
        pr.changes_asked = of_pr.iter().any(|e| e.action == "review.changes");
        let st = pr_state_of(&of_pr);
        pr.opened_at = st.opened_at;
        if pr.last == "land.pr" {
            pr.state = st.state;
            if !st.at.is_empty() {
                pr.at = st.at;
            }
            if !st.url.is_empty() {
                pr.url = st.url;
            }
            pr.note = st.note;
            pr.follow_up = st.follow_up;
        }
    }
    Some(pr)
}

/// Derive where a task stands from its rows: `jobs` are the jobs whose
/// source names the task (`task:<id>` or `task:<id>:<label>`, and the
/// task's own `job_id`); `events` are the task's `task.*` events (subject
/// the task) and the `land.*`/`review.*` events of its pull requests, in
/// any order. Pure: no clock, no store.
pub fn derive_lifecycle(
    task: &crate::Task,
    jobs: &[LifecycleJob],
    events: &[LandEvent],
) -> TaskLifecycle {
    let mut jobs: Vec<&LifecycleJob> = jobs.iter().filter(|j| j.status != "cancelled").collect();
    jobs.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    let pr = task_pr(&task.id, events);
    let leads: Vec<&LandEvent> = {
        let mut v: Vec<&LandEvent> = events.iter().filter(|e| e.action == "task.lead").collect();
        v.sort_by_key(|e| e.seq);
        v
    };
    let taken_up = !leads.is_empty() || task.status != "open" || !jobs.is_empty();

    // A lead's own job planning the task is Shape work, not a step of it.
    let planning: Vec<&LifecycleJob> = jobs
        .iter()
        .copied()
        .filter(|j| j.kind == "lead" && j.active())
        .collect();
    let active: Vec<&LifecycleJob> = jobs
        .iter()
        .copied()
        .filter(|j| j.kind != "lead" && j.active())
        .collect();
    let delivering = jobs.iter().any(|j| j.delivers());
    let failed: Vec<&LifecycleJob> = jobs
        .iter()
        .copied()
        .filter(|j| j.status == "failed")
        .collect();
    let pr_open_at_first = pr.as_ref().map(|p| p.opened_at.clone()).unwrap_or_default();
    let rework = pr.as_ref().is_some_and(|p| p.changes_asked)
        || (!pr_open_at_first.is_empty() && jobs.iter().any(|j| j.created_at > pr_open_at_first));

    // The phase whose gate is next, with who it waits on and why.
    let (mut phase, mut waits_on, mut note) = if let Some(p) = pr
        .as_ref()
        .filter(|p| p.last == "land.merged" || p.state == "merged")
    {
        (
            Phase::Done,
            WaitsOn::Nobody,
            format!("pull request #{} merged", p.number),
        )
    } else if task.status == "done" {
        (Phase::Done, WaitsOn::Nobody, "closed".to_string())
    } else if !active.is_empty() {
        let building: Vec<&LifecycleJob> = active
            .iter()
            .copied()
            .filter(|j| j.status != "done")
            .collect();
        if building.is_empty() {
            (
                Phase::Verify,
                WaitsOn::Runner,
                format!(
                    "the runner checks the commits of {}",
                    ids(active.iter().map(|j| j.id.as_str()))
                ),
            )
        } else if let Some(j) = building.iter().find(|j| j.status == "leased") {
            (Phase::Build, WaitsOn::Agent, format!("{} is at work", j.id))
        } else if let Some(j) = building.iter().find(|j| j.status == "blocked") {
            let why = if j.result.is_empty() {
                String::new()
            } else {
                format!(": {}", j.result)
            };
            (
                Phase::Build,
                WaitsOn::Runner,
                format!("{} is blocked{why}", j.id),
            )
        } else {
            (
                Phase::Build,
                WaitsOn::Runner,
                format!(
                    "{} waits for a free slot",
                    ids(building.iter().map(|j| j.id.as_str()))
                ),
            )
        }
    } else if let Some(p) = pr.as_ref().filter(|p| p.last == "land.pr") {
        let n = p.number;
        match p.state.as_str() {
            "merge_waiting" => (
                Phase::Deliver,
                WaitsOn::Github,
                format!("pull request #{n}: the merge waits for its checks or GitHub"),
            ),
            "merge_asked" => (
                Phase::Deliver,
                WaitsOn::Runner,
                format!("pull request #{n}: the owner asked for the merge; the runner merges it"),
            ),
            "needs_owner" | "review_failed" => (
                Phase::Review,
                WaitsOn::Owner,
                format!("pull request #{n} waits for the owner"),
            ),
            "changes_requested" => (
                Phase::Review,
                WaitsOn::Lead,
                format!("pull request #{n}: the reviewer asked for changes"),
            ),
            "in_review" => (
                Phase::Review,
                WaitsOn::Reviewer,
                format!("pull request #{n} is under review"),
            ),
            _ => (
                Phase::Review,
                WaitsOn::Reviewer,
                format!("pull request #{n} is open for review"),
            ),
        }
    } else if let Some(p) = pr.as_ref().filter(|p| {
        p.last == "land.pr-closed"
            && !jobs
                .iter()
                .any(|j| j.delivers() && j.finished_at.as_str() > p.at.as_str())
    }) {
        (
            Phase::Shape,
            WaitsOn::Lead,
            format!(
                "pull request #{} was closed unmerged; the lead plans the next step",
                p.number
            ),
        )
    } else if delivering {
        match pr.as_ref().filter(|p| p.last == "land.pr-held") {
            Some(p) => (
                Phase::Review,
                WaitsOn::Owner,
                if p.note.is_empty() {
                    "the pull request could not be opened".to_string()
                } else {
                    format!("the pull request could not be opened: {}", p.note)
                },
            ),
            None => (
                Phase::Review,
                WaitsOn::Runner,
                "the runner opens the task's pull request".to_string(),
            ),
        }
    } else if let Some(j) = planning.first() {
        (
            Phase::Shape,
            WaitsOn::Lead,
            format!("the lead plans it in {}", j.id),
        )
    } else if !taken_up {
        (
            Phase::Intake,
            WaitsOn::Lead,
            "waits for the lead to take it up".to_string(),
        )
    } else if let Some(j) = failed.last() {
        (
            Phase::Shape,
            WaitsOn::Lead,
            format!("{} failed; the lead plans the next step", j.id),
        )
    } else {
        (
            Phase::Shape,
            WaitsOn::Lead,
            "the lead plans its jobs".to_string(),
        )
    };
    // A task in `review` status is the lead's question to the owner: the
    // task waits on the owner whatever its jobs do.
    if task.status == "review" && phase != Phase::Done {
        waits_on = WaitsOn::Owner;
        note = "the lead asked the owner".to_string();
    }
    let mut stopped_in = None;
    if task.status == "cancelled" && phase != Phase::Done {
        stopped_in = Some(phase);
        phase = Phase::Cancelled;
        waits_on = WaitsOn::Nobody;
        note = "cancelled".to_string();
    }

    // What each phase left behind.
    let mut left: BTreeMap<Phase, (String, Vec<Mark>)> = BTreeMap::new();
    let mut intake = vec![Mark::new("task", &task.id).at(&task.created_at)];
    if !task.origin.is_empty() {
        intake.push(Mark::new("origin", &task.origin));
    }
    left.insert(Phase::Intake, (task.created_at.clone(), intake));
    // `task.lead` names the lead's sitting, and again the step it queued:
    // a step is Build's, so Shape keeps only the sittings.
    let mut shape: Vec<Mark> = leads
        .iter()
        .map(|e| Mark::new("lead", field(&e.detail, "job_id")).at(&e.at))
        .filter(|m| !m.id.is_empty() && !jobs.iter().any(|j| j.id == m.id && j.kind != "lead"))
        .collect();
    for j in jobs.iter().filter(|j| j.kind == "lead") {
        if !shape.iter().any(|m| m.id == j.id) {
            shape.push(Mark::new("lead", &j.id).note(&j.status).at(&j.created_at));
        }
    }
    left.insert(
        Phase::Shape,
        (
            leads.first().map(|e| e.at.clone()).unwrap_or_default(),
            shape,
        ),
    );
    let steps_built: Vec<&LifecycleJob> =
        jobs.iter().copied().filter(|j| j.kind != "lead").collect();
    left.insert(
        Phase::Build,
        (
            steps_built
                .first()
                .map(|j| j.created_at.clone())
                .unwrap_or_default(),
            steps_built
                .iter()
                .map(|j| Mark::new("job", &j.id).note(&j.status).at(&j.created_at))
                .collect(),
        ),
    );
    let verified: Vec<&LifecycleJob> = jobs.iter().copied().filter(|j| j.delivers()).collect();
    left.insert(
        Phase::Verify,
        (
            verified
                .iter()
                .map(|j| j.finished_at.clone())
                .filter(|a| !a.is_empty())
                .min()
                .unwrap_or_default(),
            verified
                .iter()
                .map(|j| {
                    Mark::new("branch", &j.branch)
                        .note(&j.id)
                        .at(&j.finished_at)
                })
                .collect(),
        ),
    );
    if let Some(p) = pr.as_ref() {
        let mut review = Vec::new();
        if p.number != 0 {
            review.push(
                Mark::new("pr", p.number.to_string())
                    .url(&p.url)
                    .at(&p.opened_at),
            );
        }
        if !p.state.is_empty() && p.state != "pr_open" {
            review.push(Mark::new("verdict", &p.state).note(&p.note).at(&p.at));
        } else if p.last == "land.merged" {
            review.push(Mark::new("verdict", "merged").at(&p.at));
        }
        if !p.follow_up.is_empty() {
            review.push(Mark::new("job", &p.follow_up).note("follow-up"));
        }
        if p.last == "land.pr-held" || p.last == "land.pr-closed" {
            let word = if p.last == "land.pr-held" {
                "held"
            } else {
                "closed"
            };
            review.push(Mark::new("verdict", word).note(&p.note).at(&p.at));
        }
        left.insert(Phase::Review, (p.opened_at.clone(), review));
        if p.last == "land.merged" || p.state == "merged" {
            left.insert(
                Phase::Deliver,
                (
                    p.at.clone(),
                    vec![
                        Mark::new("merge", p.number.to_string())
                            .url(&p.url)
                            .at(&p.at),
                    ],
                ),
            );
        } else if matches!(p.state.as_str(), "merge_waiting" | "merge_asked") {
            left.insert(Phase::Deliver, (p.at.clone(), Vec::new()));
        }
    }

    let reached = match phase {
        Phase::Cancelled => stopped_in.and_then(Phase::path_index).unwrap_or(0),
        Phase::Done => Phase::TASK_PATH.len(),
        p => p.path_index().unwrap_or(0),
    };
    let steps = Phase::TASK_PATH
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let (at, marks) = left.remove(p).unwrap_or_default();
            let evidence = !marks.is_empty();
            let state = if phase == Phase::Cancelled {
                if i < reached {
                    StepState::Passed
                } else {
                    StepState::Skipped
                }
            } else if i < reached {
                // A task closed without code passed its later phases by.
                if *p >= Phase::Build && !evidence {
                    StepState::Skipped
                } else {
                    StepState::Passed
                }
            } else if i == reached {
                StepState::Current
            } else {
                StepState::Ahead
            };
            PhaseStep {
                phase: *p,
                state,
                at,
                left: marks,
                gate: p.gate(),
            }
        })
        .collect();
    TaskLifecycle {
        task: task.id.clone(),
        phase,
        waits_on,
        note,
        rework,
        steps,
    }
}

fn ids<'a>(it: impl Iterator<Item = &'a str>) -> String {
    it.collect::<Vec<_>>().join(", ")
}

/// A project's tasks grouped by phase, in phase order, ids as given.
pub fn by_phase(lifecycles: &[TaskLifecycle]) -> BTreeMap<Phase, Vec<String>> {
    let mut out: BTreeMap<Phase, Vec<String>> = BTreeMap::new();
    for l in lifecycles {
        out.entry(l.phase).or_default().push(l.task.clone());
    }
    out
}

/// The rows [`derive_lifecycle`] reads, for one task.
fn rows(
    conn: &rusqlite::Connection,
    task: &crate::Task,
) -> Result<(Vec<LifecycleJob>, Vec<LandEvent>)> {
    let (exact, like) = hive_store::task_source_params(&task.id);
    let mut stmt = conn.prepare(&format!(
        "SELECT j.id, j.kind, j.status, \
                COALESCE(NULLIF(j.branch, ''), (SELECT r.branch FROM runs r WHERE r.job_id = j.id AND r.branch != '' ORDER BY r.started_at DESC LIMIT 1), ''), \
                j.created_at, j.finished_at, \
                EXISTS (SELECT 1 FROM runs r WHERE r.job_id = j.id AND r.status = 'running'), \
                j.result \
         FROM jobs j WHERE j.project = ? AND ({} OR (? != '' AND j.id = ?)) ORDER BY j.created_at, j.id",
        hive_store::task_source_clause("j.source")
    ))?;
    let jobs = stmt
        .query_map(
            rusqlite::params![task.project, exact, like, task.job_id, task.job_id],
            |r| {
                Ok(LifecycleJob {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    status: r.get(2)?,
                    branch: r.get(3)?,
                    created_at: r.get(4)?,
                    finished_at: r.get(5)?,
                    running: r.get::<_, i64>(6)? != 0,
                    result: r.get(7)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let scan = |r: &rusqlite::Row<'_>| {
        Ok(LandEvent {
            seq: r.get(0)?,
            at: r.get(1)?,
            actor: r.get(2)?,
            action: r.get(3)?,
            detail: r.get(4)?,
        })
    };
    let mut events = conn
        .prepare(
            "SELECT seq, at, actor, action, detail FROM events \
             WHERE subject = ? AND action LIKE 'task.%' ORDER BY seq",
        )?
        .query_map([&task.id], scan)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let task_prs = conn
        .prepare(
            "SELECT seq, at, actor, action, detail FROM events WHERE subject = ? \
             AND action IN ('land.pr', 'land.merged', 'land.pr-closed', 'land.pr-held') \
             AND json_valid(detail) AND json_extract(detail, '$.task') = ? ORDER BY seq",
        )?
        .query_map(rusqlite::params![task.project, task.id], scan)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut numbers: Vec<String> = task_prs
        .iter()
        .map(|e| field(&e.detail, "number"))
        .filter(|n| !n.is_empty() && n != "0")
        .collect();
    numbers.dedup();
    if let Some(number) = numbers.last() {
        let marks = vec!["?"; PR_ACTIONS.len()].join(",");
        let mut params: Vec<rusqlite::types::Value> =
            vec![rusqlite::types::Value::Text(task.project.clone())];
        params.extend(
            PR_ACTIONS
                .iter()
                .map(|a| rusqlite::types::Value::Text((*a).to_string())),
        );
        params.push(rusqlite::types::Value::Integer(number.parse().unwrap_or(0)));
        let of_pr = conn
            .prepare(&format!(
                "SELECT seq, at, actor, action, detail FROM events WHERE subject = ? AND action IN ({marks}) \
                 AND json_valid(detail) AND json_extract(detail, '$.number') = ? ORDER BY seq"
            ))?
            .query_map(rusqlite::params_from_iter(params), scan)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        events.extend(of_pr);
    }
    for e in task_prs {
        if !events.iter().any(|x| x.seq == e.seq) {
            events.push(e);
        }
    }
    events.sort_by_key(|e| e.seq);
    Ok((jobs, events))
}

/// Where one task stands in the lifecycle, read from the store.
pub fn lifecycle_for(s: &hive_store::Store, task_id: &str) -> Result<TaskLifecycle> {
    let (task, _) = crate::get(s, task_id)?;
    let conn = s.conn()?;
    let (jobs, events) = rows(&conn, &task)?;
    Ok(derive_lifecycle(&task, &jobs, &events))
}

/// Where every task of a project stands, newest task first; group them
/// with [`by_phase`].
pub fn lifecycles_for(s: &hive_store::Store, project: &str) -> Result<Vec<TaskLifecycle>> {
    let tasks = crate::list(s, project, "")?;
    let conn = s.conn()?;
    let mut out = Vec::with_capacity(tasks.len());
    for task in &tasks {
        let (jobs, events) = rows(&conn, task)?;
        out.push(derive_lifecycle(task, &jobs, &events));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(status: &str) -> crate::Task {
        crate::Task {
            id: "TASK-007".into(),
            project: "demo".into(),
            title: "Footer".into(),
            status: status.into(),
            created_at: "2026-11-26T09:00:00Z".into(),
            ..crate::Task::default()
        }
    }

    fn job(id: &str, status: &str) -> LifecycleJob {
        LifecycleJob {
            id: id.into(),
            kind: "task".into(),
            status: status.into(),
            created_at: format!("2026-11-26T10:00:00Z-{id}"),
            finished_at: if matches!(status, "done" | "failed") {
                "2026-11-26T11:00:00Z".into()
            } else {
                String::new()
            },
            ..LifecycleJob::default()
        }
    }

    fn with_branch(mut j: LifecycleJob) -> LifecycleJob {
        j.branch = format!("hive/w/{}", j.id);
        j
    }

    fn ev(seq: i64, action: &str, detail: &str) -> LandEvent {
        LandEvent {
            seq,
            at: format!("2026-11-26T12:{:02}:00Z", seq % 60),
            actor: "daemon".into(),
            action: action.into(),
            detail: detail.into(),
        }
    }

    fn lead_event(seq: i64) -> LandEvent {
        ev(
            seq,
            "task.lead",
            r#"{"job_id": "job-lead-1", "lead": "demo-lead", "project": "demo"}"#,
        )
    }

    const OPENED: &str = r#"{"task": "TASK-007", "number": 41, "url": "https://github.com/o/r/pull/41", "kind": "task", "head": "hive/task-TASK-007", "base": "main"}"#;

    fn states(l: &TaskLifecycle) -> Vec<(Phase, StepState)> {
        l.steps.iter().map(|s| (s.phase, s.state)).collect()
    }

    fn step(l: &TaskLifecycle, p: Phase) -> &PhaseStep {
        l.steps.iter().find(|s| s.phase == p).unwrap()
    }

    #[test]
    fn a_new_task_is_in_intake_and_waits_for_the_lead() {
        let l = derive_lifecycle(&task("open"), &[], &[]);
        assert_eq!((l.phase, l.waits_on), (Phase::Intake, WaitsOn::Lead));
        assert_eq!(states(&l)[0], (Phase::Intake, StepState::Current));
        assert!(
            l.steps[1..].iter().all(|s| s.state == StepState::Ahead),
            "{l:?}"
        );
        // Intake left the task itself behind.
        assert_eq!(step(&l, Phase::Intake).left[0].id, "TASK-007");
        assert!(!l.rework);
    }

    #[test]
    fn a_task_the_lead_took_up_is_in_shape_until_a_job_exists() {
        // The intake gate: a lead takes it up (task.lead), with no job yet.
        let l = derive_lifecycle(&task("open"), &[], &[lead_event(1)]);
        assert_eq!((l.phase, l.waits_on), (Phase::Shape, WaitsOn::Lead));
        assert_eq!(step(&l, Phase::Intake).state, StepState::Passed);
        assert_eq!(step(&l, Phase::Shape).left[0].id, "job-lead-1");
        // A planned task with no job is in Shape too.
        let l = derive_lifecycle(&task("planned"), &[], &[]);
        assert_eq!(l.phase, Phase::Shape);
        // A lead's own job planning it is Shape work, not a Build step.
        let mut lead = job("job-lead-2", "leased");
        lead.kind = "lead".into();
        let l = derive_lifecycle(&task("open"), &[lead], &[]);
        assert_eq!((l.phase, l.waits_on), (Phase::Shape, WaitsOn::Lead));
        assert!(step(&l, Phase::Build).left.is_empty());
        assert!(
            step(&l, Phase::Shape)
                .left
                .iter()
                .any(|m| m.id == "job-lead-2")
        );
    }

    #[test]
    fn a_queued_job_passes_the_shape_gate_and_waits_for_a_slot() {
        let l = derive_lifecycle(&task("planned"), &[job("J1", "queued")], &[lead_event(1)]);
        assert_eq!((l.phase, l.waits_on), (Phase::Build, WaitsOn::Runner));
        assert!(l.note.contains("free slot"), "{}", l.note);
        assert_eq!(
            states(&l)[..3],
            [
                (Phase::Intake, StepState::Passed),
                (Phase::Shape, StepState::Passed),
                (Phase::Build, StepState::Current)
            ]
        );
        assert_eq!(step(&l, Phase::Build).left[0].id, "J1");
    }

    #[test]
    fn a_leased_job_is_build_at_the_agent_and_a_blocked_one_says_why() {
        let l = derive_lifecycle(&task("in_progress"), &[job("J1", "leased")], &[]);
        assert_eq!((l.phase, l.waits_on), (Phase::Build, WaitsOn::Agent));
        let mut b = job("J2", "blocked");
        b.result = "environment: missing-mise".into();
        let l = derive_lifecycle(&task("in_progress"), &[b], &[]);
        assert_eq!((l.phase, l.waits_on), (Phase::Build, WaitsOn::Runner));
        assert!(l.note.contains("missing-mise"), "{}", l.note);
    }

    #[test]
    fn a_done_job_whose_run_still_goes_is_in_verify() {
        // The build gate: the worker said done; the runner's end check of
        // its commits is still running.
        let mut j = with_branch(job("J1", "done"));
        j.running = true;
        let l = derive_lifecycle(&task("in_progress"), &[j], &[]);
        assert_eq!((l.phase, l.waits_on), (Phase::Verify, WaitsOn::Runner));
        assert_eq!(step(&l, Phase::Build).state, StepState::Passed);
        // A second step still at work keeps the task in Build.
        let mut j = with_branch(job("J1", "done"));
        j.running = true;
        let l = derive_lifecycle(&task("in_progress"), &[j, job("J2", "leased")], &[]);
        assert_eq!(l.phase, Phase::Build);
    }

    #[test]
    fn verified_branches_wait_in_review_for_the_runner_to_open_the_pull_request() {
        // The verify gate: every step finished with its check passed and
        // nothing queued, running or blocked.
        let l = derive_lifecycle(
            &task("in_progress"),
            &[
                with_branch(job("J1", "done")),
                with_branch(job("J2", "done")),
            ],
            &[lead_event(1)],
        );
        assert_eq!((l.phase, l.waits_on), (Phase::Review, WaitsOn::Runner));
        let verify = step(&l, Phase::Verify);
        assert_eq!(verify.state, StepState::Passed);
        assert_eq!(verify.left.len(), 2);
        assert_eq!(verify.left[0].id, "hive/w/J1");
        // A failed step with no branch delivers nothing: back to the lead.
        let l = derive_lifecycle(&task("in_progress"), &[job("J1", "failed")], &[]);
        assert_eq!((l.phase, l.waits_on), (Phase::Shape, WaitsOn::Lead));
        assert!(l.note.contains("J1 failed"), "{}", l.note);
    }

    #[test]
    fn a_pull_request_that_could_not_be_opened_waits_on_the_owner() {
        let l = derive_lifecycle(
            &task("in_progress"),
            &[with_branch(job("J1", "done"))],
            &[ev(
                2,
                "land.pr-held",
                r#"{"task": "TASK-007", "note": "does not merge cleanly"}"#,
            )],
        );
        assert_eq!((l.phase, l.waits_on), (Phase::Review, WaitsOn::Owner));
        assert!(l.note.contains("does not merge cleanly"), "{}", l.note);
        assert!(
            step(&l, Phase::Review)
                .left
                .iter()
                .any(|m| m.kind == "verdict" && m.id == "held")
        );
    }

    #[test]
    fn an_open_pull_request_is_in_review_by_the_reviewer() {
        let jobs = [with_branch(job("J1", "done"))];
        let l = derive_lifecycle(&task("in_progress"), &jobs, &[ev(2, "land.pr", OPENED)]);
        assert_eq!((l.phase, l.waits_on), (Phase::Review, WaitsOn::Reviewer));
        let review = step(&l, Phase::Review);
        assert_eq!(review.state, StepState::Current);
        assert_eq!(review.left[0].kind, "pr");
        assert_eq!(review.left[0].id, "41");
        assert_eq!(review.left[0].url, "https://github.com/o/r/pull/41");
        let l = derive_lifecycle(
            &task("in_progress"),
            &jobs,
            &[
                ev(2, "land.pr", OPENED),
                ev(3, "review.queued", r#"{"number": 41}"#),
            ],
        );
        assert_eq!(
            (l.phase, l.note.as_str()),
            (Phase::Review, "pull request #41 is under review")
        );
    }

    #[test]
    fn a_hand_over_waits_on_the_owner_and_a_request_for_changes_on_the_lead() {
        let jobs = [with_branch(job("J1", "done"))];
        let l = derive_lifecycle(
            &task("in_progress"),
            &jobs,
            &[
                ev(2, "land.pr", OPENED),
                ev(
                    3,
                    "review.owner",
                    r#"{"number": 41, "reasons": ["touches .github/"]}"#,
                ),
            ],
        );
        assert_eq!((l.phase, l.waits_on), (Phase::Review, WaitsOn::Owner));
        let l = derive_lifecycle(
            &task("in_progress"),
            &jobs,
            &[
                ev(2, "land.pr", OPENED),
                ev(
                    3,
                    "review.changes",
                    r#"{"number": 41, "note": "add the test", "follow_up": "J9"}"#,
                ),
            ],
        );
        assert_eq!((l.phase, l.waits_on), (Phase::Review, WaitsOn::Lead));
        assert!(l.rework);
        let review = step(&l, Phase::Review);
        assert!(
            review
                .left
                .iter()
                .any(|m| m.kind == "verdict" && m.id == "changes_requested")
        );
        assert!(review.left.iter().any(|m| m.kind == "job" && m.id == "J9"));
    }

    #[test]
    fn a_follow_up_job_after_the_pull_request_opened_is_rework_in_build() {
        let mut again = job("J2", "queued");
        again.created_at = "2026-11-26T13:00:00Z".into();
        let l = derive_lifecycle(
            &task("in_progress"),
            &[with_branch(job("J1", "done")), again],
            &[ev(2, "land.pr", OPENED)],
        );
        assert_eq!(l.phase, Phase::Build);
        assert!(l.rework);
    }

    #[test]
    fn a_merge_verdict_waiting_on_checks_is_in_deliver() {
        // The review gate: a verdict of merge; the merge itself waits.
        let jobs = [with_branch(job("J1", "done"))];
        let l = derive_lifecycle(
            &task("in_progress"),
            &jobs,
            &[
                ev(2, "land.pr", OPENED),
                ev(
                    3,
                    "review.merge-wait",
                    r#"{"number": 41, "note": "checks run"}"#,
                ),
            ],
        );
        assert_eq!((l.phase, l.waits_on), (Phase::Deliver, WaitsOn::Github));
        assert_eq!(step(&l, Phase::Review).state, StepState::Passed);
        assert_eq!(step(&l, Phase::Deliver).state, StepState::Current);
        // The owner clicked Merge: the runner merges it.
        let l = derive_lifecycle(
            &task("in_progress"),
            &jobs,
            &[
                ev(2, "land.pr", OPENED),
                ev(3, "review.owner", r#"{"number": 41}"#),
                ev(4, "review.ask", r#"{"number": 41}"#),
            ],
        );
        assert_eq!((l.phase, l.waits_on), (Phase::Deliver, WaitsOn::Runner));
    }

    #[test]
    fn a_merged_pull_request_is_done_with_every_phase_passed() {
        // The deliver gate: merged on the upstream.
        let l = derive_lifecycle(
            &task("in_progress"),
            &[with_branch(job("J1", "done"))],
            &[
                lead_event(1),
                ev(2, "land.pr", OPENED),
                ev(3, "review.merged", r#"{"number": 41, "by": "reviewer"}"#),
            ],
        );
        assert_eq!((l.phase, l.waits_on), (Phase::Done, WaitsOn::Nobody));
        assert!(
            l.steps.iter().all(|s| s.state == StepState::Passed),
            "{:?}",
            states(&l)
        );
        let deliver = step(&l, Phase::Deliver);
        assert_eq!(
            (deliver.left[0].kind.as_str(), deliver.left[0].id.as_str()),
            ("merge", "41")
        );
        // The task's own land.merged says the same.
        let l = derive_lifecycle(
            &task("done"),
            &[with_branch(job("J1", "done"))],
            &[ev(2, "land.pr", OPENED), ev(3, "land.merged", OPENED)],
        );
        assert_eq!(l.phase, Phase::Done);
        assert_eq!(l.note, "pull request #41 merged");
    }

    #[test]
    fn a_task_closed_without_code_skips_the_code_phases() {
        let l = derive_lifecycle(&task("done"), &[], &[lead_event(1)]);
        assert_eq!((l.phase, l.note.as_str()), (Phase::Done, "closed"));
        assert_eq!(
            states(&l),
            vec![
                (Phase::Intake, StepState::Passed),
                (Phase::Shape, StepState::Passed),
                (Phase::Build, StepState::Skipped),
                (Phase::Verify, StepState::Skipped),
                (Phase::Review, StepState::Skipped),
                (Phase::Deliver, StepState::Skipped),
            ]
        );
    }

    #[test]
    fn a_cancelled_task_keeps_the_phases_it_passed() {
        let l = derive_lifecycle(
            &task("cancelled"),
            &[job("J1", "cancelled")],
            &[lead_event(1)],
        );
        assert_eq!((l.phase, l.waits_on), (Phase::Cancelled, WaitsOn::Nobody));
        assert_eq!(
            states(&l)[..3],
            [
                (Phase::Intake, StepState::Passed),
                (Phase::Shape, StepState::Skipped),
                (Phase::Build, StepState::Skipped),
            ]
        );
        // Cancelled jobs are not steps.
        assert!(step(&l, Phase::Build).left.is_empty());
    }

    #[test]
    fn a_task_in_review_status_waits_on_the_owner() {
        let l = derive_lifecycle(&task("review"), &[job("J1", "failed")], &[lead_event(1)]);
        assert_eq!((l.phase, l.waits_on), (Phase::Shape, WaitsOn::Owner));
        assert_eq!(l.note, "the lead asked the owner");
    }

    #[test]
    fn a_pull_request_closed_unmerged_goes_back_to_the_lead() {
        let l = derive_lifecycle(
            &task("in_progress"),
            &[with_branch(job("J1", "done"))],
            &[ev(2, "land.pr", OPENED), ev(3, "land.pr-closed", OPENED)],
        );
        assert_eq!((l.phase, l.waits_on), (Phase::Shape, WaitsOn::Lead));
        assert!(l.note.contains("closed unmerged"), "{}", l.note);
    }

    #[test]
    fn another_pull_requests_events_are_not_the_tasks() {
        let l = derive_lifecycle(
            &task("in_progress"),
            &[with_branch(job("J1", "done"))],
            &[
                ev(2, "land.pr", OPENED),
                ev(3, "review.merged", r#"{"number": 99}"#),
                ev(4, "land.merged", r#"{"task": "TASK-008", "number": 99}"#),
            ],
        );
        assert_eq!(l.phase, Phase::Review);
    }

    #[test]
    fn every_phase_has_a_name_and_the_task_path_a_gate() {
        assert_eq!(Phase::ALL.len(), 10);
        for p in Phase::ALL {
            assert!(!p.label().is_empty());
            assert_eq!(
                serde_json::to_value(p).unwrap(),
                serde_json::Value::String(p.as_str().into())
            );
        }
        for p in Phase::TASK_PATH {
            assert!(!p.gate().is_empty(), "{p:?}");
        }
        assert!(Phase::Release.gate().is_empty());
    }

    #[test]
    fn by_phase_groups_tasks_in_phase_order() {
        let mut a = derive_lifecycle(&task("open"), &[], &[]);
        a.task = "TASK-001".into();
        let b = derive_lifecycle(&task("in_progress"), &[job("J1", "leased")], &[]);
        let g = by_phase(&[b, a]);
        let keys: Vec<Phase> = g.keys().copied().collect();
        assert_eq!(keys, vec![Phase::Intake, Phase::Build]);
        assert_eq!(g[&Phase::Intake], vec!["TASK-001".to_string()]);
    }
}
