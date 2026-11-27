use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use serde_json::json;

pub const STATUSES: &[&str] = &[
    "open",
    "planned",
    "in_progress",
    "review",
    "done",
    "cancelled",
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Task {
    pub id: String,
    pub project: String,
    pub title: String,
    pub status: String,
    pub priority: i32,
    pub lead: String,
    pub job_id: String,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
    pub current_rev: i32,
    pub brief: String,
    /// Where the task came from when an agent's work filed it: `chat:<id>`
    /// for a hub conversation, `job:<id>` for a job's run, `suggestion:<id>`
    /// for an accepted suggestion. Empty when the owner filed it by hand.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub origin: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Revision {
    pub rev: i32,
    pub title: String,
    pub brief: String,
    pub note: String,
    pub author: String,
    pub at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Job {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub status: String,
    pub agent: String,
    pub branch: String,
    pub priority: i32,
    pub created_at: String,
    pub finished_at: String,
    pub result: String,
}

#[derive(Debug)]
pub struct ErrNotFound;

impl std::fmt::Display for ErrNotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("not found")
    }
}

impl std::error::Error for ErrNotFound {}

pub(crate) const TASK_COLS: &str = "t.id, t.project, t.title, t.status, t.priority, t.lead, t.job_id, t.created_by, t.created_at, t.updated_at, t.current_rev, COALESCE(r.brief, ''), t.origin";
pub(crate) const TASK_FROM: &str =
    "FROM tasks t LEFT JOIN task_revisions r ON r.task_id = t.id AND r.rev = t.current_rev";

pub(crate) fn scan_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    Ok(Task {
        id: row.get(0)?,
        project: row.get(1)?,
        title: row.get(2)?,
        status: row.get(3)?,
        priority: row.get(4)?,
        lead: row.get(5)?,
        job_id: row.get(6)?,
        created_by: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
        current_rev: row.get(10)?,
        brief: row.get(11)?,
        origin: row.get(12)?,
    })
}

pub(crate) fn input(
    code: i32,
    what: impl AsRef<str>,
    why: impl AsRef<str>,
    fix: impl AsRef<str>,
) -> anyhow::Error {
    hive_cli::errorf(code, what.as_ref(), why.as_ref(), fix.as_ref()).into()
}

pub(crate) fn check_project(s: &hive_store::Store, project: &str) -> Result<()> {
    if project.is_empty() {
        return Err(input(
            2,
            "A project is needed",
            "tasks, messages and compute requests belong to one project",
            "pass --project SLUG (see hive projects)",
        ));
    }
    if project == hive_store::COMMONS_SLUG {
        return Ok(());
    }
    if s.project(project)?.is_none() {
        return Err(input(
            2,
            format!("No project {project:?}"),
            "it is not registered in this store",
            "hive projects",
        ));
    }
    Ok(())
}

pub(crate) fn next_id(conn: &rusqlite::Connection, table: &str, prefix: &str) -> Result<String> {
    let q = format!(
        "SELECT MAX(CAST(substr(id, {}) AS INTEGER)) FROM {table} WHERE id LIKE ?",
        prefix.len() + 1
    );
    let n: Option<i64> = conn.query_row(&q, [format!("{prefix}%")], |r| r.get(0))?;
    Ok(format!("{prefix}{:03}", n.unwrap_or(0) + 1))
}

pub(crate) fn clean(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// How many tasks an agent (anyone but the owner) may have open at once
/// in one project through [`file`]: a lead plans each one, so a flood of
/// agent-filed tasks is a flood of lead sittings. The owner has no cap.
pub const AGENT_OPEN_CAP: i64 = 10;

/// Where a filed task came from, recorded on the row as `origin`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Origin {
    /// The owner filed it by hand (`hive task new`, the hub form).
    #[default]
    Owner,
    /// The agent of a hub conversation filed it (`chat:<id>`).
    Conversation(String),
    /// A job's run filed it (`job:<id>`).
    Job(String),
    /// An accepted suggestion became it (`suggestion:<id>`).
    Suggestion(String),
}

impl Origin {
    /// The stored form: empty for the owner, `kind:<id>` otherwise.
    pub fn as_stored(&self) -> String {
        match self {
            Origin::Owner => String::new(),
            Origin::Conversation(id) => format!("chat:{id}"),
            Origin::Job(id) => format!("job:{id}"),
            Origin::Suggestion(id) => format!("suggestion:{id}"),
        }
    }

    /// The origin a run's environment names: `HIVE_CONVERSATION` for a
    /// hub conversation's agent, else `HIVE_JOB` for a job's worker; the
    /// owner otherwise.
    pub fn from_env() -> Origin {
        let var = |name: &str| std::env::var(name).unwrap_or_default().trim().to_string();
        let chat = var("HIVE_CONVERSATION");
        if !chat.is_empty() {
            return Origin::Conversation(chat);
        }
        let job = var("HIVE_JOB");
        if !job.is_empty() {
            return Origin::Job(job);
        }
        Origin::Owner
    }
}

/// Whether a stored `origin` names an agent's work rather than the owner's
/// hand: the lead words such a task differently (nobody is waiting to be
/// asked whether what landed is what they wanted).
pub fn agent_origin(origin: &str) -> bool {
    !origin.is_empty()
}

/// Files a task the owner's way: `hive task new` and the hub's form come
/// here, with no origin to record. The guard rails of [`file`] still hold
/// for an agent author.
pub fn new(
    s: &hive_store::Store,
    project: &str,
    title: &str,
    brief: &str,
    priority: i32,
    author: &str,
) -> Result<Task> {
    file(s, project, title, brief, priority, author, &Origin::Owner)
}

/// Files a task and records where it came from. An agent's task (any
/// author but the owner) needs a brief that says why, may not repeat the
/// title of a task still open in the project, and counts against
/// [`AGENT_OPEN_CAP`] open tasks per author and project: the lead plans
/// each one, and an agent that files in a loop would fill its queue. An
/// accepted suggestion skips the cap: another agent's review stood in
/// for it.
pub fn file(
    s: &hive_store::Store,
    project: &str,
    title: &str,
    brief: &str,
    priority: i32,
    author: &str,
    origin: &Origin,
) -> Result<Task> {
    let title = clean(title);
    if title.is_empty() {
        return Err(input(
            2,
            "A task needs a title",
            "the title is what the board and the lead see first",
            "hive task new --project P --title 'what you want' [--brief-file F]",
        ));
    }
    if title.len() > 200 {
        return Err(input(
            2,
            format!("title is {} chars; limit 200", title.len()),
            "keep the title short",
            "move detail into the brief (--brief-file F or stdin)",
        ));
    }
    if author.is_empty() {
        return Err(input(
            2,
            "An author is needed",
            "every task records who gave it",
            format!(
                "pass --agent NAME or set HIVE_AGENT (the owner is {})",
                hive_store::HUMAN_AGENT
            ),
        ));
    }
    let agent = author != hive_store::HUMAN_AGENT;
    if agent && brief.trim().is_empty() {
        return Err(input(
            2,
            "A task an agent files needs a brief",
            "the lead plans from the brief: what to do, why, and how the work is proved",
            "say what and why in the brief, or leave the task to the owner",
        ));
    }
    check_project(s, project)?;
    let now = s.now();
    let tx = s.begin()?;
    if agent {
        let same: Option<String> = tx
            .query_row(
                "SELECT id FROM tasks WHERE project = ? AND lower(title) = lower(?) AND status NOT IN ('done', 'cancelled') LIMIT 1",
                params![project, title],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = same {
            return Err(input(
                2,
                format!("Already a task: {id}"),
                "an open task in this project has the same title",
                format!(
                    "hive task show {id}; add to its thread with hive task reply, or file a different slice"
                ),
            ));
        }
        let open: i64 = tx.query_row(
            "SELECT COUNT(*) FROM tasks WHERE project = ? AND created_by = ? AND status NOT IN ('done', 'cancelled')",
            params![project, author],
            |r| r.get(0),
        )?;
        if open >= AGENT_OPEN_CAP && !matches!(origin, Origin::Suggestion(_)) {
            return Err(input(
                2,
                format!("{author} has {open} tasks open in {project}"),
                format!(
                    "an agent may have {AGENT_OPEN_CAP} open per project; each one is a lead's sitting"
                ),
                format!(
                    "wait for one to finish, or cancel one of yours: hive task list --project {project} --status open"
                ),
            ));
        }
    }
    let origin_text = origin.as_stored();
    let id = next_id(&tx, "tasks", "TASK-")?;
    tx.execute("INSERT INTO tasks (id, project, title, status, priority, created_by, created_at, updated_at, current_rev, origin) VALUES (?, ?, ?, 'open', ?, ?, ?, ?, 1, ?)",
        params![id, project, title, priority, author, now, now, origin_text])?;
    tx.execute(
        "INSERT INTO task_revisions (task_id, rev, title, brief, note, author, at) VALUES (?, 1, ?, ?, '', ?, ?)",
        params![id, title, brief, author, now],
    )?;
    tx.event(
        author,
        "task.created",
        &id,
        hive_store::Detail::json(
            json!({"project": project, "title": title, "priority": priority, "origin": origin_text}),
        )?,
    )?;
    tx.commit()?;
    Ok(get(s, &id)?.0)
}

pub fn get(s: &hive_store::Store, id: &str) -> Result<(Task, Vec<Revision>)> {
    let conn = s.conn()?;
    let q = format!("SELECT {TASK_COLS} {TASK_FROM} WHERE t.id = ?");
    let t = conn
        .query_row(&q, [id], scan_task)
        .optional()?
        .ok_or_else(|| {
            input(
                2,
                format!("No task {id:?}"),
                "the id is not in this store",
                "hive task list",
            )
        })?;
    let mut stmt = conn.prepare("SELECT rev, title, brief, note, author, at FROM task_revisions WHERE task_id = ? ORDER BY rev")?;
    let revs = stmt
        .query_map([id], |r| {
            Ok(Revision {
                rev: r.get(0)?,
                title: r.get(1)?,
                brief: r.get(2)?,
                note: r.get(3)?,
                author: r.get(4)?,
                at: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok((t, revs))
}

pub fn list(s: &hive_store::Store, project: &str, status: &str) -> Result<Vec<Task>> {
    let mut q = format!("SELECT {TASK_COLS} {TASK_FROM} WHERE 1=1");
    let mut args = Vec::new();
    if !project.is_empty() {
        q.push_str(" AND t.project = ?");
        args.push(project.to_string());
    }
    if !status.is_empty() {
        q.push_str(" AND t.status = ?");
        args.push(status.to_string());
    }
    q.push_str(" ORDER BY t.id DESC");
    let conn = s.conn()?;
    let mut stmt = conn.prepare(&q)?;
    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    Ok(stmt
        .query_map(rusqlite::params_from_iter(refs), scan_task)?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn jobs(s: &hive_store::Store, id: &str) -> Result<Vec<Job>> {
    let conn = s.conn()?;
    let (exact, like) = hive_store::task_source_params(id);
    let mut stmt = conn.prepare(&format!(
        "SELECT id, kind, title, status, lease_agent, branch, priority, created_at, finished_at, result FROM jobs WHERE {} ORDER BY created_at",
        hive_store::task_source_clause("source")
    ))?;
    Ok(stmt
        .query_map([exact, like], |r| {
            Ok(Job {
                id: r.get(0)?,
                kind: r.get(1)?,
                title: r.get(2)?,
                status: r.get(3)?,
                agent: r.get(4)?,
                branch: r.get(5)?,
                priority: r.get(6)?,
                created_at: r.get(7)?,
                finished_at: r.get(8)?,
                result: r.get(9)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

pub(crate) fn terminal(status: &str) -> bool {
    status == "done" || status == "cancelled"
}

pub fn amend(
    s: &hive_store::Store,
    id: &str,
    title: &str,
    brief: &str,
    note: &str,
    author: &str,
) -> Result<Task> {
    if author.is_empty() {
        return Err(input(
            2,
            "An author is needed",
            "every revision records who wrote it",
            "pass --agent NAME or set HIVE_AGENT",
        ));
    }
    let (cur, revs) = get(s, id)?;
    if terminal(&cur.status) {
        return Err(input(
            2,
            format!("{id} is {} and cannot be amended", cur.status),
            "finished work is history",
            format!(
                "hive task new --project {} --title '...' (open a new task)",
                cur.project
            ),
        ));
    }
    let mut title = clean(title);
    if title.is_empty() {
        title = cur.title.clone();
    }
    if title.len() > 200 {
        return Err(input(
            2,
            format!("title is {} chars; limit 200", title.len()),
            "keep the title short",
            "move detail into the brief",
        ));
    }
    let last = revs.last().cloned().unwrap_or_default();
    let brief = if brief.is_empty() {
        last.brief
    } else {
        brief.to_string()
    };
    if title == cur.title && brief == cur.brief && clean(note).is_empty() {
        return Err(input(
            2,
            "Nothing changed",
            "the title and brief equal the current revision",
            "pass --title, --brief-file/stdin or --note",
        ));
    }
    let now = s.now();
    let tx = s.begin()?;
    let rev = cur.current_rev + 1;
    tx.execute("INSERT INTO task_revisions (task_id, rev, title, brief, note, author, at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        params![id, rev, title, brief, note, author, now])?;
    tx.execute(
        "UPDATE tasks SET title = ?, current_rev = ?, updated_at = ? WHERE id = ?",
        params![title, rev, now, id],
    )?;
    let (exact, like) = hive_store::task_source_params(id);
    let replanned = tx.execute(
        &format!(
            "UPDATE jobs SET brief = brief || ? WHERE {} AND status = 'queued'",
            hive_store::task_source_clause("source")
        ),
        params![
            format!(
                "\n\n[task amended: rev {rev} - re-read `hive task show {id}` before starting]"
            ),
            exact,
            like
        ],
    )?;
    tx.event(
        author,
        "task.amended",
        id,
        hive_store::Detail::json(
            json!({"project": cur.project, "rev": rev, "note": note, "replanned_jobs": replanned}),
        )?,
    )?;
    tx.commit()?;
    Ok(get(s, id)?.0)
}

pub fn cancel(s: &hive_store::Store, id: &str, reason: &str, actor: &str) -> Result<Task> {
    if actor.is_empty() {
        return Err(input(
            2,
            "An actor is needed",
            "every cancellation records who asked",
            "pass --agent NAME or set HIVE_AGENT",
        ));
    }
    let (cur, _) = get(s, id)?;
    if terminal(&cur.status) {
        return Err(input(
            2,
            format!("{id} is already {}", cur.status),
            "a finished task does not change",
            format!("hive task show {id}"),
        ));
    }
    let now = s.now();
    let tx = s.begin()?;
    tx.execute(
        "UPDATE tasks SET status = 'cancelled', updated_at = ? WHERE id = ?",
        params![now, id],
    )?;
    let (exact, like) = hive_store::task_source_params(id);
    let clause = hive_store::task_source_clause("source");
    // A cancelled task has no open jobs (TASK-140): the queued and the
    // blocked ones end here - a blocked job left behind was once put back
    // in the queue three days later and ran 100 minutes for nothing - and
    // the run of a leased one is asked to stop the way the owner's cancel
    // of a run does (its harness session's cancel request; the runner's
    // watch also stops a run whose task is cancelled, for a run that has
    // no session yet).
    let blocked: i64 = tx.query_row(
        &format!("SELECT COUNT(*) FROM jobs WHERE {clause} AND status = 'blocked'"),
        params![exact, like],
        |r| r.get(0),
    )?;
    let cancelled = tx.execute(
        &format!("UPDATE jobs SET status = 'cancelled', finished_at = ?, result = ?, lease_agent = '', lease_until = '' WHERE {clause} AND status IN ('queued', 'blocked')"),
        params![now, format!("task cancelled: {reason}"), exact, like])?;
    let leased: i64 = tx.query_row(
        &format!("SELECT COUNT(*) FROM jobs WHERE {clause} AND status = 'leased'"),
        params![exact, like],
        |r| r.get(0),
    )?;
    let stopped = tx.execute(
        &format!("UPDATE harness_sessions SET cancel_requested_at = ?, updated_at = ? WHERE status = 'running' AND cancel_requested_at = '' AND id IN (SELECT r.id FROM runs r JOIN jobs j ON j.id = r.job_id WHERE r.status = 'running' AND j.status = 'leased' AND {})", hive_store::task_source_clause("j.source")),
        params![now, now, exact, like],
    )?;
    tx.event(actor, "task.cancelled", id, hive_store::Detail::json(json!({"project": cur.project, "reason": reason, "cancelled_jobs": cancelled, "blocked_jobs": blocked, "leased_jobs": leased, "stopped_runs": stopped, "lead": cur.lead}))?)?;
    tx.commit()?;
    Ok(get(s, id)?.0)
}

pub fn status_index(status: &str) -> isize {
    STATUSES
        .iter()
        .position(|s| *s == status)
        .map_or(-1, |i| i as isize)
}

pub fn set_status(s: &hive_store::Store, id: &str, status: &str, actor: &str) -> Result<Task> {
    if actor.is_empty() {
        return Err(input(
            2,
            "An actor is needed",
            "every status change records who made it",
            "pass --agent NAME or set HIVE_AGENT",
        ));
    }
    let to = status_index(status);
    if to < 0 {
        return Err(input(
            2,
            format!("Unknown status {status:?}"),
            format!("statuses are {}", STATUSES.join(", ")),
            format!("hive task status {id} <status>"),
        ));
    }
    if status == "cancelled" {
        return cancel(s, id, "", actor);
    }
    let (cur, _) = get(s, id)?;
    let from = status_index(&cur.status);
    if terminal(&cur.status) {
        return Err(input(
            2,
            format!("{id} is {}", cur.status),
            "a finished task does not change",
            format!("hive task new --project {} --title '...'", cur.project),
        ));
    }
    // One step back is allowed: a task waiting in review goes on when the
    // owner answers or the lead picks the work up again.
    let back_to_work = cur.status == "review" && status == "in_progress";
    if to <= from && !back_to_work {
        return Err(input(
            2,
            format!("{id} is {}; {status} is not a step forward", cur.status),
            format!(
                "tasks move {} or to cancelled; only review goes back, to in_progress",
                STATUSES[..5].join(" -> ")
            ),
            format!("hive task status {id} {}", STATUSES[from as usize + 1]),
        ));
    }
    let now = s.now();
    let tx = s.begin()?;
    tx.execute(
        "UPDATE tasks SET status = ?, updated_at = ? WHERE id = ?",
        params![status, now, id],
    )?;
    tx.event(
        actor,
        "task.status",
        id,
        hive_store::Detail::json(
            json!({"project": cur.project, "from": cur.status, "to": status}),
        )?,
    )?;
    tx.commit()?;
    Ok(get(s, id)?.0)
}

/// Moves a `planned` task back to `open` so its lead plans it again: the
/// one step back [`set_status`] does not take, for a task marked planned
/// with no job carrying its source (`task:<id>` or `task:<id>:<label>`) -
/// a lead that folded it into another task's job, or set the status before
/// queueing anything. The `task.status` event carries `why`, so the lead's
/// next brief and the hub can say what happened.
pub fn reopen(s: &hive_store::Store, id: &str, why: &str, actor: &str) -> Result<Task> {
    if actor.is_empty() {
        return Err(input(
            2,
            "An actor is needed",
            "every status change records who made it",
            "pass --agent NAME or set HIVE_AGENT",
        ));
    }
    let (cur, _) = get(s, id)?;
    if cur.status != "planned" {
        return Err(input(
            2,
            format!("{id} is {}; only a planned task reopens", cur.status),
            "a task whose work started moves forward or to review",
            format!("hive task show {id}"),
        ));
    }
    let now = s.now();
    let tx = s.begin()?;
    tx.execute(
        "UPDATE tasks SET status = 'open', updated_at = ? WHERE id = ?",
        params![now, id],
    )?;
    tx.event(
        actor,
        "task.status",
        id,
        hive_store::Detail::json(
            json!({"project": cur.project, "from": cur.status, "to": "open", "why": why}),
        )?,
    )?;
    tx.commit()?;
    Ok(get(s, id)?.0)
}

pub fn set_lead(
    s: &hive_store::Store,
    id: &str,
    lead: &str,
    job_id: &str,
    actor: &str,
) -> Result<Task> {
    let (cur, _) = get(s, id)?;
    let now = s.now();
    let tx = s.begin()?;
    tx.execute("UPDATE tasks SET lead = ?, job_id = CASE WHEN ? = '' THEN job_id ELSE ? END, updated_at = ? WHERE id = ?", params![lead, job_id, job_id, now, id])?;
    tx.event(
        actor,
        "task.lead",
        id,
        hive_store::Detail::json(json!({"project": cur.project, "lead": lead, "job_id": job_id}))?,
    )?;
    tx.commit()?;
    Ok(get(s, id)?.0)
}
