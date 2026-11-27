use crate::messages::stamp;
use crate::tasks::{check_project, clean, input, next_id};
use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use serde_json::json;
use std::sync::RwLock;

pub const COMPUTE_STATUSES: &[&str] = &[
    "pending",
    "approved",
    "provisioning",
    "ready",
    "denied",
    "released",
    "failed",
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ComputeRequest {
    pub id: String,
    pub project: String,
    pub requested_by: String,
    pub size: String,
    pub hours: i32,
    pub reason: String,
    pub status: String,
    pub decided_by: String,
    pub executor: String,
    pub words: String,
    pub proposal_post: String,
    /// The daemon's last word on the request: the provisioning step in progress, the reason it
    /// failed, or why its machine was released. Empty while nothing has happened and once it is ready.
    pub note: String,
    pub at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Caps {
    pub provider: String,
    pub max_vms: i32,
    pub max_hours_per_day: i32,
    pub auto_approve: bool,
    pub budget_credits: i32,
    pub vms_now: i32,
    pub hours_today: i32,
    pub login_source: bool,
}

const COMPUTE_COLS: &str = "id, project, requested_by, size, hours, reason, status, decided_by, executor, words, proposal_post, note, at, updated_at";

/// How much of the daemon's note on a request is kept: a probe note and the step that failed,
/// not a transcript.
pub const COMPUTE_NOTE_CHARS: usize = 500;

fn scan_compute(row: &rusqlite::Row<'_>) -> rusqlite::Result<ComputeRequest> {
    Ok(ComputeRequest {
        id: row.get(0)?,
        project: row.get(1)?,
        requested_by: row.get(2)?,
        size: row.get(3)?,
        hours: row.get(4)?,
        reason: row.get(5)?,
        status: row.get(6)?,
        decided_by: row.get(7)?,
        executor: row.get(8)?,
        words: row.get(9)?,
        proposal_post: row.get(10)?,
        note: row.get(11)?,
        at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

pub fn get_compute(s: &hive_store::Store, id: &str) -> Result<ComputeRequest> {
    let conn = s.conn()?;
    conn.query_row(
        &format!("SELECT {COMPUTE_COLS} FROM compute_requests WHERE id = ?"),
        [id],
        scan_compute,
    )
    .optional()?
    .ok_or_else(|| {
        input(
            2,
            format!("No compute request {id:?}"),
            "the id is not in this store",
            "hive compute list",
        )
    })
}

pub fn list_compute(
    s: &hive_store::Store,
    project: &str,
    status: &str,
) -> Result<Vec<ComputeRequest>> {
    let mut q = format!("SELECT {COMPUTE_COLS} FROM compute_requests WHERE 1=1");
    let mut args = Vec::new();
    if !project.is_empty() {
        q.push_str(" AND project = ?");
        args.push(project.to_string());
    }
    if !status.is_empty() {
        q.push_str(" AND status = ?");
        args.push(status.to_string());
    }
    q.push_str(" ORDER BY id DESC");
    let conn = s.conn()?;
    let mut stmt = conn.prepare(&q)?;
    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    Ok(stmt
        .query_map(rusqlite::params_from_iter(refs), scan_compute)?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

pub type LoginSourceCheckFn = fn(&hive_store::Store) -> bool;
pub static LOGIN_SOURCE_CHECK: RwLock<Option<LoginSourceCheckFn>> = RwLock::new(None);

pub const NO_LOGIN_SOURCE: &str = "no executor login source";

pub fn get_caps(s: &hive_store::Store) -> Result<Caps> {
    let login_source = LOGIN_SOURCE_CHECK
        .read()
        .expect("login source lock poisoned")
        .is_none_or(|f| f(s));
    let mut c = Caps {
        provider: s.setting("compute.provider", ""),
        max_vms: s.setting_int("compute.max_vms") as i32,
        max_hours_per_day: s.setting_int("compute.max_hours_per_day") as i32,
        auto_approve: s.setting("compute.auto_approve", "") == "1",
        budget_credits: s.setting_int("compute.budget_credits") as i32,
        login_source,
        ..Caps::default()
    };
    let conn = s.conn()?;
    c.vms_now = conn.query_row(
        "SELECT COUNT(*) FROM compute_requests WHERE status IN ('approved','provisioning','ready')",
        [],
        |r| r.get(0),
    )?;
    c.hours_today = hours_today(&conn, &s.now())?;
    Ok(c)
}

fn parse_iso(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|t| t.with_timezone(&chrono::Utc))
}

/// The machine-hours the UTC day of `now` has spent or holds, rounded up to whole hours:
///
/// - a machine counts the time it really ran that day, from when it was made to when it was
///   released (or `now` while it runs), past the hours it was approved for too: a machine runs
///   on until its last run ends, and two rentals replayed on one hub ran 5.2 and 8.0 hours on
///   4-hour approvals while the day counted 4 each;
/// - a request still open (approved, provisioning, ready) also holds the asked hours it has not
///   run yet, whatever day it was made, so the day cannot approve them twice;
/// - a pending request of the day holds its asked hours, as one with no machine row recorded
///   does once it is released or failed after its machine was ready;
/// - a request that failed before a machine was made, or was denied, consumed nothing, as its
///   own finding says, so it does not block the retry that fixes the cause.
pub fn hours_today(conn: &rusqlite::Connection, now: &str) -> Result<i32> {
    let today = now.get(..10).unwrap_or(now);
    let day_start = format!("{today}T00:00:00Z");
    // A clock that does not read counts nothing it could vouch for: the caller gets an error
    // and rents nothing, as every other failure here does.
    let (Some(now_t), Some(start_t)) = (parse_iso(now), parse_iso(&day_start)) else {
        return Err(input(
            1,
            format!("The store's clock {now:?} does not read as a time"),
            "the day's machine-hours are counted from it, so no cap can be checked",
            "check the store's clock (HIVE_NOW) and try again",
        ));
    };
    let mut stmt = conn.prepare(
        "SELECT r.hours, r.status, r.executor, r.at, COALESCE(m.created_at, ''), COALESCE(m.released_at, ''), r.updated_at FROM compute_requests r LEFT JOIN compute_machines m ON m.request_id = r.id WHERE r.status != 'denied' AND (substr(r.at, 1, 10) = ?1 OR r.status IN ('approved', 'provisioning', 'ready') OR (COALESCE(m.created_at, '') != '' AND COALESCE(NULLIF(m.released_at, ''), r.updated_at) >= ?2))",
    )?;
    let rows = stmt.query_map(params![today, day_start], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, String>(6)?,
        ))
    })?;
    let mut seconds: i64 = 0;
    for row in rows {
        let (hours, status, executor, at, made, released, updated) = row?;
        let asked = hours.max(0) * 3600;
        let open = matches!(status.as_str(), "approved" | "provisioning" | "ready");
        let of_today = at.get(..10) == Some(today);
        let made_t = if made.is_empty() {
            None
        } else {
            parse_iso(&made)
        };
        // Released at its machine's release, else - a closed request whose machine row was
        // never stamped - when the request closed; an open one runs until now.
        let end_t = if !released.is_empty() {
            parse_iso(&released).map(|t| t.min(now_t))
        } else if open {
            Some(now_t)
        } else {
            parse_iso(&updated).map(|t| t.min(now_t))
        };
        match (made_t, end_t) {
            (Some(made_t), Some(end_t)) => {
                seconds =
                    seconds.saturating_add((end_t - made_t.max(start_t)).num_seconds().max(0));
                if open {
                    seconds = seconds
                        .saturating_add((asked - (end_t - made_t).num_seconds().max(0)).max(0));
                }
            }
            // A machine row whose times do not read: the asked hours, as before rows had times.
            (Some(_), None) | (None, _) if !made.is_empty() => {
                seconds = seconds.saturating_add(asked)
            }
            _ => {
                let ran_or_holds =
                    open || (of_today && !(status == "failed" && executor.is_empty()));
                if ran_or_holds {
                    seconds = seconds.saturating_add(asked);
                }
            }
        }
    }
    // Past what an i32 holds is past every cap: it must never wrap round to a small number.
    Ok(i32::try_from(seconds.saturating_add(3599) / 3600).unwrap_or(i32::MAX))
}

/// The most hours one request may ask: `compute.max_hours_per_day` when it is set and no more
/// than a day, else a day. A request past it could never be served inside the cap, and a sum
/// with it could wrap round past `i32::MAX` into a small number the cap lets through.
pub fn max_request_hours(max_hours_per_day: i32) -> i32 {
    if max_hours_per_day > 0 {
        max_hours_per_day.min(24)
    } else {
        24
    }
}

pub fn request_compute(
    s: &hive_store::Store,
    project: &str,
    agent: &str,
    size: &str,
    hours: i32,
    reason: &str,
) -> Result<ComputeRequest> {
    if agent.is_empty() {
        return Err(input(
            2,
            "An agent is needed",
            "compute is requested by an agent",
            "pass --agent NAME or set HIVE_AGENT",
        ));
    }
    let mut size = size.trim().to_string();
    let size_setting = hive_store::compute_size_setting(&s.setting("compute.provider", ""));
    if size.is_empty() {
        size = s.setting(&size_setting, "");
    }
    if size.is_empty() {
        return Err(input(
            2,
            "A machine size is needed",
            format!("{size_setting} is empty and --size was not given"),
            format!(
                "hive compute request --size <size> ... or hive settings set {size_setting} <size>"
            ),
        ));
    }
    if hours <= 0 {
        return Err(input(
            2,
            "Hours must be at least 1",
            "a request names how long the machine is needed",
            "hive compute request --hours 4 ...",
        ));
    }
    let most = max_request_hours(s.setting_int("compute.max_hours_per_day") as i32);
    if hours > most {
        return Err(input(
            2,
            format!("{hours} hours is more than one request may ask ({most})"),
            format!(
                "a request asks for at most compute.max_hours_per_day ({}) and never more than 24",
                s.setting_int("compute.max_hours_per_day")
            ),
            format!("hive compute request --hours {most} ..."),
        ));
    }
    let reason = clean(reason);
    if reason.is_empty() {
        return Err(input(
            2,
            "A reason is needed",
            "the owner reads it before approving",
            "hive compute request --reason 'why' ...",
        ));
    }
    check_project(s, project)?;
    let caps = get_caps(s)?;
    let auto = caps.auto_approve
        && caps.vms_now < caps.max_vms
        && caps.max_hours_per_day > 0
        && caps.hours_today.saturating_add(hours) <= caps.max_hours_per_day
        && caps.login_source;
    let landing = machine_landing_line(s)?;
    let now = s.now();
    let tx = s.begin()?;
    let id = next_id(&tx, "compute_requests", "CR-")?;
    let (status, decided_by) = if auto {
        ("approved", "hive")
    } else {
        ("pending", "")
    };
    tx.execute("INSERT INTO compute_requests (id, project, requested_by, size, hours, reason, status, decided_by, at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![id, project, agent, size, hours, reason, status, decided_by, now, now])?;
    tx.event(
        agent,
        "compute.requested",
        &id,
        hive_store::Detail::json(
            json!({"project": project, "size": size, "hours": hours, "status": status, "machines": landing}),
        )?,
    )?;
    if auto {
        tx.event("hive", "compute.approved", &id, hive_store::Detail::json(json!({"project": project, "auto": true, "vms_now": caps.vms_now + 1, "max_vms": caps.max_vms}))?)?;
    } else {
        let summary = format!("compute request {id}: {size} for {hours}h - {reason}");
        let mut body = format!(
            "{agent} asks for one {size} machine for {hours} hours.\n\nReason: {reason}\n\nCaps: {} of {} machines in use, auto_approve={}.\n\nApprove with `hive compute approve {id} --words '...'` or deny with `hive compute deny {id} --words '...'` (or from the hub's Compute page).\n",
            caps.vms_now, caps.max_vms, caps.auto_approve
        );
        if !landing.is_empty() {
            body.push('\n');
            body.push_str(&landing);
            body.push('\n');
        }
        if !caps.login_source {
            body.push_str("\nHeld: ");
            body.push_str(NO_LOGIN_SOURCE);
            body.push_str(" - a provisioned machine could not log in. Place a Copilot-only fine-grained token with `hive executor token --with-token`; an approval waits for one (ADR-034).\n");
        }
        let post_id = insert_post(
            &tx, s, project, "runner", "compute", "proposal", agent, &summary, &body, "", "",
        )?;
        tx.execute(
            "UPDATE compute_requests SET proposal_post = ? WHERE id = ?",
            params![post_id, id],
        )?;
    }
    tx.commit()?;
    get_compute(s, &id)
}

/// How many of the last runs with commits on rented or paired machines came
/// back to land (TASK-133): the runs counted are the most recent
/// [`MACHINE_RUNS_SEEN`] finished runs with commits on any executor but this
/// machine whose outcome says something about the machine. A run came back
/// when it ended `done` - its commits were fetched to the brain and handed
/// to the landing - and did not when its commits could not be fetched (an
/// `executor.fetch-failed` event names the run, or, before that event, its
/// summary begins "could not land the branch"). A failed check, an identity
/// refusal or a stale lease would have ended the same on this machine, so
/// they are not counted against the machines (TASK-138 item 5). A request
/// for a machine says it, so nobody rents a machine whose work cannot come
/// back. Empty when no machine has run work with commits yet.
pub fn machine_landing_line(s: &hive_store::Store) -> Result<String> {
    let conn = s.conn()?;
    let (seen, landed, last): (i64, i64, String) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(status = 'done'), 0), COALESCE(MAX(CASE WHEN status = 'done' THEN finished_at END), '')
         FROM (SELECT r.status, r.finished_at FROM runs r
               WHERE r.executor NOT IN ('', 'local') AND r.commits > 0 AND r.finished_at != ''
                 AND r.status != 'running'
                 AND (r.status = 'done'
                      OR r.summary LIKE 'could not land the branch%'
                      OR EXISTS (SELECT 1 FROM events e
                                 WHERE e.action = 'executor.fetch-failed' AND e.subject = r.executor
                                   AND json_valid(e.detail) AND json_extract(e.detail, '$.run') = r.id))
               ORDER BY r.finished_at DESC, r.id DESC LIMIT ?)",
        [MACHINE_RUNS_SEEN],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    if seen == 0 {
        return Ok(String::new());
    }
    let mut line = format!(
        "Machines so far: {landed} of the last {seen} runs with commits on rented or paired machines landed"
    );
    if landed == 0 {
        line.push_str(" - their work did not come back; check `hive executor list` before relying on a new machine");
    } else if !last.is_empty() {
        line.push_str(&format!(" (the last at {last})"));
    }
    line.push('.');
    Ok(line)
}

/// The window [`machine_landing_line`] counts.
pub const MACHINE_RUNS_SEEN: i64 = 20;

pub fn approve(
    s: &hive_store::Store,
    id: &str,
    words: &str,
    actor: &str,
) -> Result<ComputeRequest> {
    decide_compute(s, id, words, actor, true)
}

pub fn deny(s: &hive_store::Store, id: &str, words: &str, actor: &str) -> Result<ComputeRequest> {
    decide_compute(s, id, words, actor, false)
}

fn decide_compute(
    s: &hive_store::Store,
    id: &str,
    words: &str,
    actor: &str,
    yes: bool,
) -> Result<ComputeRequest> {
    let actor = if actor.is_empty() {
        hive_store::HUMAN_AGENT
    } else {
        actor
    };
    let cur = get_compute(s, id)?;
    if cur.status != "pending" {
        return Err(input(
            2,
            format!("{id} is {}, not pending", cur.status),
            "only a pending request takes an answer",
            "hive compute list",
        ));
    }
    let (status, action, answer) = if yes {
        ("approved", "compute.approved", "yes")
    } else {
        ("denied", "compute.denied", "no")
    };
    let now = s.now();
    let tx = s.begin()?;
    tx.execute("UPDATE compute_requests SET status = ?, decided_by = ?, words = ?, updated_at = ? WHERE id = ?", params![status, actor, words, now, id])?;
    if !cur.proposal_post.is_empty() {
        let decided: i64 = tx.query_row(
            "SELECT COUNT(*) FROM decisions WHERE post_id = ?",
            [&cur.proposal_post],
            |r| r.get(0),
        )?;
        if decided == 0 {
            let resolution = if yes { "accepted" } else { "declined" };
            let mut summary: String = tx.query_row(
                "SELECT summary FROM posts WHERE id = ?",
                [&cur.proposal_post],
                |r| r.get(0),
            )?;
            if summary.len() > 200 {
                summary.truncate(197);
                summary.push_str("...");
            }
            let text = if words.is_empty() {
                "(no words recorded)"
            } else {
                words
            };
            let body = format!(
                "The human answered **{answer}**.\n\n{text}\n\nProposal: {}\n",
                cur.proposal_post
            );
            let result_id = insert_post(
                &tx,
                s,
                &cur.project,
                "runner",
                "compute",
                "result",
                actor,
                &format!("{resolution}: {summary}"),
                &body,
                &cur.proposal_post,
                resolution,
            )?;
            // Which model and provider decided (ADR-072 part 5): an agent's
            // answer names its own model and the hub's provider; the human's
            // row stays empty.
            let (model, provider) = if hive_store::human_actor(actor) {
                (String::new(), String::new())
            } else {
                let own: String = tx
                    .query_row("SELECT model FROM agents WHERE name = ?", [actor], |r| {
                        r.get(0)
                    })
                    .optional()?
                    .unwrap_or_default();
                let model = if own.trim().is_empty() {
                    s.light_model()
                } else {
                    own.trim().to_string()
                };
                (model, s.setting("provider", ""))
            };
            tx.execute("INSERT INTO decisions (post_id, project, answer, words, decided_at, decided_by, result_post, decided_model, decided_provider) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![cur.proposal_post, cur.project, answer, words, now, actor, result_id, model, provider])?;
            tx.event(
                actor,
                "decide",
                &cur.proposal_post,
                hive_store::Detail::json(json!({"answer": answer, "result": result_id, "decided_model": model, "decided_provider": provider}))?,
            )?;
        }
    }
    tx.event(
        actor,
        action,
        id,
        hive_store::Detail::json(
            json!({"project": cur.project, "words": words, "requested_by": cur.requested_by}),
        )?,
    )?;
    tx.commit()?;
    get_compute(s, id)
}

pub fn set_compute_status(
    s: &hive_store::Store,
    id: &str,
    status: &str,
    executor: &str,
    actor: &str,
) -> Result<ComputeRequest> {
    if !COMPUTE_STATUSES.contains(&status) {
        return Err(input(
            2,
            format!("Unknown compute status {status:?}"),
            format!("statuses are {}", COMPUTE_STATUSES.join(", ")),
            "",
        ));
    }
    let cur = get_compute(s, id)?;
    let now = s.now();
    let tx = s.begin()?;
    tx.execute("UPDATE compute_requests SET status = ?, executor = CASE WHEN ? = '' THEN executor ELSE ? END, updated_at = ? WHERE id = ?", params![status, executor, executor, now, id])?;
    tx.event(
        actor,
        &format!("compute.{status}"),
        id,
        hive_store::Detail::json(
            json!({"project": cur.project, "executor": executor, "from": cur.status}),
        )?,
    )?;
    tx.commit()?;
    get_compute(s, id)
}

/// Records the daemon's last word on a request - the provisioning step in progress, the reason
/// it failed or was released - for the Machines page and `hive compute list --json`. One line,
/// at most `COMPUTE_NOTE_CHARS`; an empty note clears it (a request that came up ready has
/// nothing to explain).
pub fn note_compute(s: &hive_store::Store, id: &str, note: &str) -> Result<()> {
    let mut note = clean(note);
    if note.chars().count() > COMPUTE_NOTE_CHARS {
        note = note
            .chars()
            .take(COMPUTE_NOTE_CHARS - 1)
            .collect::<String>()
            + "~";
    }
    let conn = s.conn()?;
    let changed = conn.execute(
        "UPDATE compute_requests SET note = ? WHERE id = ?",
        params![note, id],
    )?;
    if changed == 0 {
        return Err(input(
            2,
            format!("No compute request {id:?}"),
            "the id is not in this store",
            "hive compute list",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn insert_post(
    tx: &hive_store::Tx,
    s: &hive_store::Store,
    project: &str,
    area: &str,
    thread: &str,
    kind: &str,
    agent: &str,
    summary: &str,
    body: &str,
    on: &str,
    resolution: &str,
) -> Result<String> {
    let mut summary = clean(summary);
    if summary.len() > hive_board::MAX_SUMMARY_CHARS {
        summary.truncate(hive_board::MAX_SUMMARY_CHARS - 3);
        summary.push_str("...");
    }
    let now = s.now();
    let base = format!("{}-{agent}", stamp(&now));
    let mut id = base.clone();
    for n in 2.. {
        let exists: Option<i64> = tx
            .query_row("SELECT 1 FROM posts WHERE id = ?", [&id], |r| r.get(0))
            .optional()?;
        if exists.is_none() {
            break;
        }
        id = format!("{base}-{n}");
    }
    let (runner, model): (String, String) = tx
        .query_row(
            "SELECT runner, model FROM agents WHERE name = ?",
            [agent],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .unwrap_or_default();
    tx.execute("INSERT INTO posts (id, project, area, thread, kind, agent, runner, model, session, date, branch, expires, on_post, vote, text_hash, has_read_line, resolution, summary, body, links, files, name, origin_project, extra, origin, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, '', ?, '', '', ?, '', '', 0, ?, ?, ?, '', '', '', '', '{}', 'store', ?)",
        params![id, project, area, thread, kind, agent, runner, model, now, on, resolution, summary, body, now])
        .context("post")?;
    tx.execute(
        "UPDATE agents SET posts = posts + 1, last_active_at = ? WHERE name = ?",
        params![now, agent],
    )?;
    tx.event(
        agent,
        "post",
        &id,
        hive_store::Detail::json(
            json!({"kind": kind, "project": project, "thread": format!("{area}/{thread}")}),
        )?,
    )?;
    Ok(id)
}

pub type ComputeProvidersFn = fn(&mut hive_cli::Context) -> Result<()>;
pub static COMPUTE_PROVIDERS: RwLock<Option<ComputeProvidersFn>> = RwLock::new(None);

/// The linux hands arch a requested machine size will need. The request CLI
/// has no provider yet (the worker picks one), so this mirrors the ARM
/// patterns the worker's `hands_arch_for_size` matches; the probe error names
/// the arch it checked.
pub fn default_hands_arch(size: &str) -> String {
    let size = size.trim().to_ascii_lowercase();
    let arm = {
        // Azure: Standard_E2ps_v5 style (a letters+digits block, "p", series
        // letters, "_v" digits). AWS: m8g.large / a1.medium style. GCP: t2a-,
        // c4a-.
        let azure = size.starts_with("standard_") && {
            let core = size.trim_start_matches("standard_");
            let digits: String = core.chars().filter(|c| c.is_ascii_digit()).collect();
            core.contains('p') && !digits.is_empty()
        };
        let aws = size
            .split('.')
            .next()
            .is_some_and(|family| family.starts_with('a') || family.ends_with('g'))
            && size.contains('.');
        let gcp = size.starts_with("t2a-") || size.starts_with("c4a-");
        azure || aws || gcp
    };
    if arm { "arm64".into() } else { "amd64".into() }
}
