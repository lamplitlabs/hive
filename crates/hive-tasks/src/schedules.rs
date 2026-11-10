//! Schedules: a standing job any agent can set for a project - "every day at
//! 09:00, run this brief" - which the daemon queues as an ordinary job each
//! time it is due. The worker that takes the job is the scheduled agent.
//!
//! Two kinds run a script from the repo instead of a worker: a `check`
//! (a monitor: exit 0 is green, anything else is red and queues one job to
//! make it pass) and a `watch` (a watcher: whatever it prints is handed to
//! one job; silence means nothing to do). The daemon runs them with no LLM
//! and wakes an agent only when the script says so.

use anyhow::Result;
use chrono::{DateTime, Duration, NaiveTime, Utc};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use serde_json::json;
use std::path::Component;

use crate::tasks::{check_project, clean, input, next_id};

pub const SCHEDULE_STATUSES: &[&str] = &["active", "paused", "ended"];
/// The job kinds a schedule may queue: the ones a worker runs from a brief
/// alone. Experiments, ballots and leads are the planner's.
pub const SCHEDULE_KINDS: &[&str] = &["task", "improve", "clean", "reflect"];
/// The kinds that run a committed script instead of queueing a job every
/// time: `check` (a monitor) and `watch` (a watcher).
pub const SCRIPT_KINDS: &[&str] = &["check", "watch"];
/// The kinds of job a red check or a firing watch may queue.
pub const SCRIPT_JOB_KINDS: &[&str] = &["task", "improve"];
/// Consecutive failures of a watch script after which a job to fix the
/// watcher itself is queued.
pub const WATCH_FIX_AFTER: i32 = 3;
/// Characters of a script's output kept in `last_output` and quoted to the
/// job and the board.
pub const OUTPUT_TAIL: usize = 4000;
/// The `source` of every job a schedule queues: `schedule:SCHED-001`. One
/// open job per schedule at a time - `add_job` dedupes on it.
pub const SOURCE_PREFIX: &str = "schedule:";
/// The board area the daemon posts a check's or watcher's findings under;
/// the thread is the schedule id.
pub const BOARD_AREA: &str = "schedule";
/// Where a check's or watch's script lives: under the project's `.hive/`
/// folder, so everything Hive adds to a repository stays in one place.
/// `script_home` names the usual spot for one.
pub const SCRIPT_ROOT: &str = ".hive/";
const TIME_LAYOUT: &str = "%Y-%m-%dT%H:%M:%SZ";
pub(crate) const MINUTES_PER_DAY: i64 = 24 * 60;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Schedule {
    pub id: String,
    pub project: String,
    pub title: String,
    pub kind: String,
    pub brief: String,
    pub tier: i32,
    pub priority: i32,
    pub every_minutes: i32,
    pub at_time: String,
    pub status: String,
    pub next_at: String,
    pub last_at: String,
    pub last_job: String,
    pub runs: i32,
    pub max_runs: i32,
    pub note: String,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
    /// `check`/`watch`: the script, relative to the repo root, on the
    /// project's default branch. Empty for the kinds that queue a job.
    pub script: String,
    /// The kind of job the schedule queues: its own kind for the job kinds,
    /// task or improve for a check or a watch.
    pub job_kind: String,
    /// `check`: green/red; `watch`: ok/broken; empty until the first run.
    pub state: String,
    pub state_since: String,
    /// Exit code of the last script run; -1 when it timed out.
    pub last_rc: Option<i32>,
    /// Redacted tail of the last script run's output.
    pub last_output: String,
    /// Consecutive non-zero exits.
    pub fails: i32,
    /// The custom agent every job it queues runs as; empty lets the runner
    /// choose by the card.
    pub agent: String,
    /// The skill every job it queues follows (a name of the project's
    /// `hive skill` registry); empty for none.
    pub skill: String,
    /// Empty when set by hand or by an agent; `repo:<path>#<name>` when the
    /// project's own Hive file keeps it (`crate::manifest`).
    pub origin: String,
}

impl Schedule {
    /// Whether a file of the project's repository keeps this schedule.
    pub fn from_repo(&self) -> bool {
        self.origin.starts_with(hive_store::REPO_ORIGIN)
    }
    /// The cadence as written: `6h`, `1d`, `90m`.
    pub fn every(&self) -> String {
        format_every(i64::from(self.every_minutes))
    }
    pub fn source(&self) -> String {
        format!("{SOURCE_PREFIX}{}", self.id)
    }
    /// The `at` time of day that anchors a cadence of whole days, if any. A
    /// stored value that does not parse counts as none.
    pub fn at(&self) -> Option<NaiveTime> {
        parse_at(&self.at_time).ok().flatten()
    }
    /// The time after this run: one cadence on, kept on the `at` time of
    /// day when there is one (`advance_at`).
    pub fn next_after(&self, now: &str) -> Result<String> {
        advance_at(&self.next_at, i64::from(self.every_minutes), self.at(), now)
    }
    /// A check or a watch: the daemon runs its script instead of queueing a
    /// job each time.
    pub fn is_script(&self) -> bool {
        SCRIPT_KINDS.contains(&self.kind.as_str())
    }
    /// The state a failing run puts a script schedule in.
    pub fn bad_state(&self) -> &'static str {
        if self.kind == "watch" {
            "broken"
        } else {
            "red"
        }
    }
    /// The state a passing run puts a script schedule in.
    pub fn good_state(&self) -> &'static str {
        if self.kind == "watch" { "ok" } else { "green" }
    }
    /// A check or a watch whose script sits outside `.hive/`: set before
    /// the rule, it keeps running from where it is, and is flagged until
    /// the script moves.
    pub fn script_outside(&self) -> bool {
        self.is_script() && !script_in_hive(&self.script)
    }
}

/// Whether a script path (as `validate_script_path` returns it) is under
/// the project's `.hive/` folder.
pub fn script_in_hive(path: &str) -> bool {
    path.starts_with(SCRIPT_ROOT) && path.len() > SCRIPT_ROOT.len()
}

/// The place a check's or watch's script belongs: beside its automation
/// file, `.hive/automations/<name>.sh`.
pub fn script_home(name: &str) -> String {
    let stem = name.trim();
    format!(
        "{}/{}.sh",
        crate::manifest::AUTOMATIONS_DIR,
        if stem.is_empty() { "<name>" } else { stem }
    )
}

#[derive(Debug, Clone, Default)]
pub struct ScheduleInput {
    pub project: String,
    pub title: String,
    pub kind: String,
    pub brief: String,
    pub tier: i32,
    pub priority: i32,
    /// `30m`, `6h`, `1d`, `2w`, `hourly`, `daily`, `weekly`, or minutes.
    pub every: String,
    /// `HH:MM` UTC; anchors a cadence of whole days.
    pub at: String,
    /// End after this many jobs; 0 keeps going.
    pub times: i32,
    pub author: String,
    /// `check`/`watch`: the script to run, relative to the repo root.
    pub script: String,
    /// `check`/`watch`: the kind of job to queue (task, improve); empty
    /// means task.
    pub job_kind: String,
    /// The custom agent the jobs run as (`--run-as`); checked against the
    /// project's agent files. Empty lets the runner choose.
    pub agent: String,
    /// The skill the jobs follow (`--skill`); a name the project's skill
    /// registry holds. Empty for none.
    pub skill: String,
    /// `repo:<path>#<name>` when the project's Hive file keeps the
    /// schedule; empty otherwise.
    pub origin: String,
}

/// What one due schedule did on a daemon tick.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Fired {
    pub schedule: Schedule,
    pub job: String,
    /// False when the job of the previous run was still open, so none was
    /// queued; the schedule moved on to its next time all the same.
    pub queued: bool,
    pub ended: bool,
}

const COLS: &str = "id, project, title, kind, brief, tier, priority, every_minutes, at_time, status, next_at, last_at, last_job, runs, max_runs, note, created_by, created_at, updated_at, script, job_kind, state, state_since, last_rc, last_output, fails, agent, skill, origin";

fn scan(row: &rusqlite::Row<'_>) -> rusqlite::Result<Schedule> {
    Ok(Schedule {
        id: row.get(0)?,
        project: row.get(1)?,
        title: row.get(2)?,
        kind: row.get(3)?,
        brief: row.get(4)?,
        tier: row.get(5)?,
        priority: row.get(6)?,
        every_minutes: row.get(7)?,
        at_time: row.get(8)?,
        status: row.get(9)?,
        next_at: row.get(10)?,
        last_at: row.get(11)?,
        last_job: row.get(12)?,
        runs: row.get(13)?,
        max_runs: row.get(14)?,
        note: row.get(15)?,
        created_by: row.get(16)?,
        created_at: row.get(17)?,
        updated_at: row.get(18)?,
        script: row.get(19)?,
        job_kind: row.get(20)?,
        state: row.get(21)?,
        state_since: row.get(22)?,
        last_rc: row.get(23)?,
        last_output: row.get(24)?,
        fails: row.get(25)?,
        agent: row.get(26)?,
        skill: row.get(27)?,
        origin: row.get(28)?,
    })
}

/// Minutes in a cadence: `30m`, `6h`, `1d`, `2w`, `hourly`, `daily`,
/// `weekly`, or a bare number of minutes.
pub fn parse_every(text: &str) -> Result<i64> {
    let text = text.trim().to_lowercase();
    let bad = || {
        input(
            2,
            format!("Not a cadence: {text:?}"),
            "a cadence is a number with a unit",
            "--every 30m | 6h | 1d | 2w (or hourly, daily, weekly)",
        )
    };
    let minutes = match text.as_str() {
        "hourly" => 60,
        "daily" => MINUTES_PER_DAY,
        "weekly" => 7 * MINUTES_PER_DAY,
        "" => return Err(bad()),
        _ => {
            let digits: String = text.chars().take_while(|c| c.is_ascii_digit()).collect();
            let unit = text[digits.len()..].trim();
            let n: i64 = digits.parse().map_err(|_| bad())?;
            let per = match unit {
                "" | "m" | "min" | "minute" | "minutes" => 1,
                "h" | "hr" | "hour" | "hours" => 60,
                "d" | "day" | "days" => MINUTES_PER_DAY,
                "w" | "wk" | "week" | "weeks" => 7 * MINUTES_PER_DAY,
                _ => return Err(bad()),
            };
            n.checked_mul(per).ok_or_else(bad)?
        }
    };
    if minutes < 1 || minutes > i64::from(i32::MAX) {
        return Err(bad());
    }
    Ok(minutes)
}

/// The cadence in its largest whole unit: `2w`, `1d`, `6h`, `90m`.
pub fn format_every(minutes: i64) -> String {
    if minutes > 0 && minutes % (7 * MINUTES_PER_DAY) == 0 {
        format!("{}w", minutes / (7 * MINUTES_PER_DAY))
    } else if minutes > 0 && minutes % MINUTES_PER_DAY == 0 {
        format!("{}d", minutes / MINUTES_PER_DAY)
    } else if minutes > 0 && minutes % 60 == 0 {
        format!("{}h", minutes / 60)
    } else {
        format!("{minutes}m")
    }
}

pub(crate) fn parse_at(text: &str) -> Result<Option<NaiveTime>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    NaiveTime::parse_from_str(text, "%H:%M")
        .map(Some)
        .map_err(|_| {
            input(
                2,
                format!("Not a time of day: {text:?}"),
                "--at takes hours and minutes in UTC",
                "--at 09:00",
            )
        })
}

fn parse_iso(value: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|_| {
            input(
                2,
                format!("Not a UTC time like 2026-09-05T08:00:00Z: {value:?}"),
                "times are UTC",
                "pass --now 2026-09-05T08:00:00Z",
            )
        })
}

fn iso(t: DateTime<Utc>) -> String {
    t.format(TIME_LAYOUT).to_string()
}

/// When a new schedule first fires: one cadence from now, or the next `at`
/// time of day when one is given.
pub fn first_due(now: &str, every_minutes: i64, at: Option<NaiveTime>) -> Result<String> {
    let now = parse_iso(now)?;
    Ok(iso(match at {
        Some(at) => {
            let today = now.date_naive().and_time(at).and_utc();
            if today > now {
                today
            } else {
                today + Duration::days(1)
            }
        }
        None => now + Duration::minutes(every_minutes),
    }))
}

/// The time after `next_at` that follows `now`: one cadence on, skipping the
/// ticks a stopped daemon missed instead of queueing one job for each.
pub fn advance(next_at: &str, every_minutes: i64, now: &str) -> Result<String> {
    let now = parse_iso(now)?;
    let step = Duration::minutes(every_minutes.max(1));
    let mut next = parse_iso(next_at)? + step;
    if next <= now {
        let behind = (now - next).num_minutes() / every_minutes.max(1) + 1;
        next += Duration::minutes(behind * every_minutes.max(1));
    }
    Ok(iso(next))
}

/// `advance` for a schedule that may carry an `at` time of day. Without one,
/// or on a cadence that is not whole days, it is plain `advance`. With one,
/// the next time keeps that time of day even when `next_at` lost it - a
/// manual `hive schedule run` makes a schedule due at the moment it was
/// run, and a cadence stepped from there would fire at that moment for
/// ever:
///
/// - daily: the next `at` strictly after `now`, as `first_due` picks it (a
///   manual run is one run more; the 05:00 run keeps its place);
/// - every N > 1 days: the first `at` at least one cadence on from
///   `next_at` (the cadence's weekday is not kept anywhere once a manual run
///   replaced `next_at`, so the next run is a whole cadence away, never
///   sooner). On time, `next_at` already sits on `at` and keeps its weekday.
///
/// Missed ticks still collapse into one run, as `advance` does.
pub fn advance_at(
    next_at: &str,
    every_minutes: i64,
    at: Option<NaiveTime>,
    now: &str,
) -> Result<String> {
    let stepped = advance(next_at, every_minutes, now)?;
    let Some(at) = at else {
        return Ok(stepped);
    };
    if every_minutes <= 0 || every_minutes % MINUTES_PER_DAY != 0 {
        return Ok(stepped);
    }
    let from = if every_minutes == MINUTES_PER_DAY {
        parse_iso(now)?
    } else {
        // `stepped` is past `now`; one second back lets an `at` that falls
        // exactly on it count as "at least one cadence on".
        parse_iso(&stepped)? - Duration::seconds(1)
    };
    let same_day = from.date_naive().and_time(at).and_utc();
    Ok(iso(if same_day > from {
        same_day
    } else {
        same_day + Duration::days(1)
    }))
}

fn setting_int(s: &hive_store::Store, name: &str, default: i64) -> i64 {
    s.setting(name, "").trim().parse().unwrap_or(default)
}

/// Checks a check's or watch's script path as written: relative, inside the
/// repo, with no `..`, backslash or absolute root. Returns it trimmed of a
/// leading `./`.
pub fn validate_script_path(path: &str) -> Result<String> {
    let path = path.trim();
    let bad = |why: &str| {
        input(
            2,
            format!("Not a script path: {path:?}"),
            why,
            "--script .hive/automations/check.sh (relative to the repo root, committed on the default branch)",
        )
    };
    if path.is_empty() {
        return Err(input(
            2,
            "A check or watch needs a script",
            "the daemon runs it from the repo root on a cadence",
            "--script .hive/automations/check.sh",
        ));
    }
    if path.contains('\\') {
        return Err(bad("use forward slashes"));
    }
    if path.starts_with('/') {
        return Err(bad("the path is relative to the repo root, not absolute"));
    }
    let mut parts = Vec::new();
    for c in std::path::Path::new(path).components() {
        match c {
            Component::Normal(p) => parts.push(p.to_string_lossy().to_string()),
            Component::CurDir => {}
            Component::ParentDir => return Err(bad("`..` would leave the repo")),
            Component::RootDir | Component::Prefix(_) => {
                return Err(bad("the path is relative to the repo root, not absolute"));
            }
        }
    }
    if parts.is_empty() {
        return Err(bad("it names no file"));
    }
    Ok(parts.join("/"))
}

/// The rule a new or changed script path must meet: it lives under
/// `.hive/`. `kept` is the script the schedule already runs, when there is
/// one: an automation set before the rule keeps its script where it is
/// (`Schedule::script_outside` flags it) until the script changes.
pub fn check_script_place(script: &str, kept: &str) -> Result<()> {
    if script_in_hive(script) || (!kept.is_empty() && script == kept.trim()) {
        return Ok(());
    }
    Err(input(
        2,
        format!("The script {script:?} is outside {SCRIPT_ROOT}"),
        "a check's or watch's script lives under the project's .hive/ folder, so everything Hive adds to the repository stays in one place",
        format!(
            "move it to {} (or anywhere under {SCRIPT_ROOT}), commit on the default branch, and name that path",
            script_home("")
        ),
    ))
}

/// Whether `path` is a file on `branch` of the repo at `repo`: `Some(true)`
/// when `git cat-file` finds a blob there, `Some(false)` when the repo answers
/// and it is not, `None` when the repo cannot be asked (no such directory, no
/// git, not a repo).
pub fn script_on_branch(repo: &str, branch: &str, path: &str) -> Option<bool> {
    if repo.trim().is_empty() || !std::path::Path::new(repo).is_dir() {
        return None;
    }
    let git = |args: &[&str]| -> Option<(bool, String)> {
        let mut cmd = std::process::Command::new(hive_gitenv::git_program());
        cmd.arg("-C").arg(repo).args(args);
        cmd.env_clear();
        for kv in hive_gitenv::env() {
            if let Some((k, v)) = kv.split_once('=') {
                cmd.env(k, v);
            }
        }
        let out = cmd.output().ok()?;
        Some((
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
        ))
    };
    let (is_repo, _) = git(&["rev-parse", "--git-dir"])?;
    if !is_repo {
        return None;
    }
    let spec = format!("{branch}:{path}");
    Some(matches!(git(&["cat-file", "-t", &spec]), Some((true, kind)) if kind == "blob"))
}

/// Adds a schedule for `input.project`; the first job is due one cadence from
/// now (or at the next `at`). A check or watch whose repo cannot be asked for
/// the script is accepted all the same; see `add_checked` for the warning.
pub fn add(s: &hive_store::Store, input: ScheduleInput) -> Result<Schedule> {
    add_checked(s, input).map(|(sched, _)| sched)
}

/// `add`, returning also a warning for the caller to print: non-empty when
/// the script of a check or watch could not be looked for on the default
/// branch because the repo was out of reach.
pub fn add_checked(s: &hive_store::Store, input: ScheduleInput) -> Result<(Schedule, String)> {
    let v = validate(s, &input, "")?;
    let cap = setting_int(s, "schedule.max_per_project", 10);
    let now = s.now();
    let tx = s.begin()?;
    if cap > 0 {
        let held: i64 = tx.query_row(
            "SELECT COUNT(*) FROM schedules WHERE project = ? AND status IN ('active', 'paused')",
            [&input.project],
            |r| r.get(0),
        )?;
        if held >= cap {
            return Err(self::input(
                2,
                format!("{} already holds {held} schedules", input.project),
                "the setting schedule.max_per_project caps them",
                format!(
                    "end one first: hive schedule list --project {}",
                    input.project
                ),
            ));
        }
    }
    let id = next_id(&tx, "schedules", "SCHED-")?;
    let next_at = first_due(&now, v.every, v.at)?;
    tx.execute(
        "INSERT INTO schedules (id, project, title, kind, brief, tier, priority, every_minutes, at_time, status, next_at, max_runs, created_by, created_at, updated_at, script, job_kind, agent, skill, origin)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'active', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            id,
            input.project,
            v.title,
            v.kind,
            v.brief,
            input.tier,
            input.priority,
            v.every,
            v.at_text(),
            next_at,
            input.times,
            input.author,
            now,
            now,
            v.script,
            v.job_kind,
            v.agent,
            v.skill,
            v.origin
        ],
    )?;
    tx.event(
        &input.author,
        "schedule.added",
        &id,
        hive_store::Detail::json(json!({
            "project": input.project,
            "title": v.title,
            "kind": v.kind,
            "script": v.script,
            "job_kind": v.job_kind,
            "agent": v.agent,
            "skill": v.skill,
            "origin": v.origin,
            "every": format_every(v.every),
            "next_at": next_at,
            "times": input.times,
        }))?,
    )?;
    tx.commit()?;
    Ok((get(s, &id)?, v.warning))
}

/// The fields of a schedule that `update` may change, in the order
/// `schedule.updated` lists them.
pub const UPDATABLE: &[&str] = &[
    "title", "kind", "brief", "tier", "priority", "every", "at", "times", "script", "job_kind",
    "agent", "skill",
];

/// Changes a schedule's definition in place, keeping its id, project,
/// author, origin, status and history: every other field of `input`
/// replaces the row's after the checks `add` makes. The cadence starts over
/// from now only when `every` or `at` changed; a change of kind forgets the
/// script state. Returns the schedule, the names of the fields that changed
/// (`UPDATABLE` order; empty when nothing did, and then nothing is written)
/// and the same warning `add_checked` gives.
pub fn update(
    s: &hive_store::Store,
    id: &str,
    input: &ScheduleInput,
    author: &str,
) -> Result<(Schedule, Vec<String>, String)> {
    update_as(s, id, input, author, true)
}

/// `update` for the project's Hive file (`crate::manifest`): the file owns
/// the row, so the daemon that applies it needs no ownership.
pub(crate) fn update_from_file(
    s: &hive_store::Store,
    id: &str,
    input: &ScheduleInput,
    author: &str,
) -> Result<(Schedule, Vec<String>, String)> {
    update_as(s, id, input, author, false)
}

fn update_as(
    s: &hive_store::Store,
    id: &str,
    input: &ScheduleInput,
    author: &str,
    check_owner: bool,
) -> Result<(Schedule, Vec<String>, String)> {
    let sched = get(s, id)?;
    if check_owner {
        owned_by(&sched, author)?;
    } else if author.is_empty() {
        return Err(self::input(
            2,
            "An author is needed",
            "changes to a schedule record who made them",
            "pass --agent NAME or set HIVE_AGENT",
        ));
    }
    if sched.status == "ended" {
        return Err(self::input(
            2,
            format!("Schedule {id} has ended"),
            "an ended schedule stays in the list for its history and never changes",
            "hive schedule add ... for a new one",
        ));
    }
    if !input.project.is_empty() && input.project != sched.project {
        return Err(self::input(
            2,
            format!("Schedule {id} belongs to {}", sched.project),
            "a schedule keeps its project",
            "end it and add one for the other project",
        ));
    }
    // The file that keeps a row may move it to another path (the list file
    // to its own file); by hand the origin never changes.
    let origin =
        if !check_owner && sched.from_repo() && input.origin.starts_with(hive_store::REPO_ORIGIN) {
            input.origin.trim().to_string()
        } else {
            sched.origin.clone()
        };
    let own = ScheduleInput {
        project: sched.project.clone(),
        author: author.to_string(),
        origin,
        ..input.clone()
    };
    let v = validate(s, &own, &sched.script)?;
    let mut changed = Vec::new();
    let differs: [(&str, bool); 13] = [
        ("origin", v.origin != sched.origin),
        ("title", v.title != sched.title),
        ("kind", v.kind != sched.kind),
        ("brief", v.brief != sched.brief),
        ("tier", own.tier != sched.tier),
        ("priority", own.priority != sched.priority),
        ("every", v.every != i64::from(sched.every_minutes)),
        ("at", v.at_text() != sched.at_time),
        ("times", own.times != sched.max_runs),
        ("script", v.script != sched.script),
        ("job_kind", v.job_kind != sched.job_kind),
        ("agent", v.agent != sched.agent),
        ("skill", v.skill != sched.skill),
    ];
    for (name, differ) in differs {
        if differ {
            changed.push(name.to_string());
        }
    }
    if changed.is_empty() {
        return Ok((sched, changed, v.warning));
    }
    let now = s.now();
    let cadence = changed.iter().any(|c| c == "every" || c == "at");
    let next_at = if cadence {
        first_due(&now, v.every, v.at)?
    } else {
        sched.next_at.clone()
    };
    let tx = s.begin()?;
    tx.execute(
        "UPDATE schedules SET origin = ?, title = ?, kind = ?, brief = ?, tier = ?, priority = ?, every_minutes = ?, at_time = ?, max_runs = ?, script = ?, job_kind = ?, agent = ?, skill = ?, next_at = ?, updated_at = ? WHERE id = ?",
        params![
            v.origin,
            v.title,
            v.kind,
            v.brief,
            own.tier,
            own.priority,
            v.every,
            v.at_text(),
            own.times,
            v.script,
            v.job_kind,
            v.agent,
            v.skill,
            next_at,
            now,
            id
        ],
    )?;
    if changed.iter().any(|c| c == "kind") {
        tx.execute(
            "UPDATE schedules SET state = '', state_since = '', last_rc = NULL, last_output = '', fails = 0 WHERE id = ?",
            [id],
        )?;
    }
    tx.event(
        author,
        "schedule.updated",
        id,
        hive_store::Detail::json(json!({
            "project": sched.project,
            "changed": changed,
            "title": v.title,
            "kind": v.kind,
            "every": format_every(v.every),
            "next_at": next_at,
            "origin": v.origin,
        }))?,
    )?;
    tx.commit()?;
    Ok((get(s, id)?, changed, v.warning))
}

/// A schedule input after every check `add` and `update` make, with the
/// fields in the form the row keeps.
struct Validated {
    title: String,
    kind: String,
    brief: String,
    script: String,
    job_kind: String,
    agent: String,
    skill: String,
    origin: String,
    every: i64,
    at: Option<NaiveTime>,
    /// Non-empty when the script could not be looked for on the default
    /// branch (the repo was out of reach).
    warning: String,
}

impl Validated {
    fn at_text(&self) -> String {
        self.at
            .map(|t| t.format("%H:%M").to_string())
            .unwrap_or_default()
    }
}

/// The checks every schedule passes before it is written, as an error for
/// the first that fails; `hive manifest check --project P` runs them on a
/// file's entries without writing anything.
pub fn check(s: &hive_store::Store, input: &ScheduleInput) -> Result<String> {
    validate(s, input, "").map(|v| v.warning)
}

/// `check` for a change to a schedule that already runs `kept_script`:
/// that script may stay where it is (see `check_script_place`).
pub fn check_update(
    s: &hive_store::Store,
    input: &ScheduleInput,
    kept_script: &str,
) -> Result<String> {
    validate(s, input, kept_script).map(|v| v.warning)
}

/// The checks every schedule passes before it is written, whether added or
/// updated: the cap on schedules per project is the one check `add` alone
/// makes. `kept_script` is the script the schedule already runs (empty for
/// a new one).
fn validate(s: &hive_store::Store, input: &ScheduleInput, kept_script: &str) -> Result<Validated> {
    let title = clean(&input.title);
    if title.is_empty() {
        return Err(self::input(
            2,
            "A schedule needs a title",
            "the title names the job it queues",
            "hive schedule add --project P --title 'what to do' --every 1d --brief '...'",
        ));
    }
    if title.len() > 200 {
        return Err(self::input(
            2,
            format!("title is {} chars; limit 200", title.len()),
            "keep the title short",
            "move detail into the brief (--brief TEXT or --brief-file F)",
        ));
    }
    let kind = if input.kind.is_empty() {
        "task".to_string()
    } else {
        input.kind.clone()
    };
    let script_kind = SCRIPT_KINDS.contains(&kind.as_str());
    if !script_kind && !SCHEDULE_KINDS.contains(&kind.as_str()) {
        return Err(self::input(
            2,
            format!("A schedule cannot queue {kind:?} jobs"),
            format!(
                "its kinds are {} (jobs) and {} (scripts)",
                SCHEDULE_KINDS.join(", "),
                SCRIPT_KINDS.join(", ")
            ),
            "--kind task",
        ));
    }
    let skill = input.skill.trim().to_string();
    let mut warning = String::new();
    let (script, job_kind) = if script_kind {
        let script = validate_script_path(&input.script)?;
        check_script_place(&script, kept_script)?;
        let job_kind = if input.job_kind.trim().is_empty() {
            "task".to_string()
        } else {
            input.job_kind.trim().to_string()
        };
        if !SCRIPT_JOB_KINDS.contains(&job_kind.as_str()) {
            return Err(self::input(
                2,
                format!("A {kind} cannot queue {job_kind:?} jobs"),
                format!("--job-kind is one of {}", SCRIPT_JOB_KINDS.join(", ")),
                "--job-kind task",
            ));
        }
        (script, job_kind)
    } else {
        if !input.script.trim().is_empty() {
            return Err(self::input(
                2,
                format!("--script is for a check or a watch, not a {kind}"),
                "a job kind queues its brief to a worker; a script kind runs the script and queues a job only when it says so",
                "--kind check --script PATH, or drop --script",
            ));
        }
        if !input.job_kind.trim().is_empty() {
            return Err(self::input(
                2,
                format!("--job-kind is for a check or a watch, not a {kind}"),
                "a job kind queues jobs of its own kind",
                "--kind check --job-kind improve, or drop --job-kind",
            ));
        }
        if input.brief.trim().is_empty() && skill.is_empty() {
            return Err(self::input(
                2,
                "A schedule needs a brief",
                "the worker that takes each job has nothing else to go on",
                "--brief 'what to do, how to know it is done', --brief-file F, or --skill NAME to run a skill",
            ));
        }
        (String::new(), kind.clone())
    };
    if input.tier != 0 && input.tier != 1 {
        return Err(self::input(
            2,
            "The runner takes Tier 0 and Tier 1 jobs only",
            "Tier 2 needs an ADR and the human",
            "--tier 0 or --tier 1",
        ));
    }
    if input.times < 0 {
        return Err(self::input(
            2,
            "--times counts jobs",
            "0 means no end",
            "--times 3",
        ));
    }
    if input.author.is_empty() {
        return Err(self::input(
            2,
            "An author is needed",
            "every schedule records who set it",
            format!(
                "pass --agent NAME or set HIVE_AGENT (the owner is {})",
                hive_store::HUMAN_AGENT
            ),
        ));
    }
    check_project(s, &input.project)?;
    let agent = hive_board::check_run_as(s, &input.project, &input.agent)?;
    if !skill.is_empty() {
        check_skill(s, &input.project, &skill)?;
    }
    if script_kind {
        let Some(project) = s.project(&input.project)? else {
            return Err(self::input(
                2,
                format!("A {kind} needs a repo; {} has none", input.project),
                "the daemon runs the script in a worktree of the project's default branch",
                "pick a project from hive projects",
            ));
        };
        let branch = if project.default_branch.trim().is_empty() {
            "main".to_string()
        } else {
            project.default_branch.trim().to_string()
        };
        match script_on_branch(&project.path, &branch, &script) {
            Some(true) => {}
            Some(false) => {
                return Err(self::input(
                    2,
                    format!("No file {script:?} on {branch} of {}", input.project),
                    format!(
                        "the daemon runs the script from a worktree of {branch}, so it must be committed there (`git -C {} cat-file -e {branch}:{script}` fails)",
                        project.path
                    ),
                    format!("commit it on {branch} first, or check the path"),
                ));
            }
            None => {
                warning = format!(
                    "warning: could not look for {script} on {branch}: the repo at {} is out of reach from here; the daemon will tell on the first run",
                    if project.path.is_empty() {
                        "(no path)".to_string()
                    } else {
                        project.path.clone()
                    }
                );
            }
        }
    }
    let every = parse_every(&input.every)?;
    let min = setting_int(s, "schedule.min_minutes", 30).max(1);
    if every < min {
        return Err(self::input(
            2,
            format!(
                "{} is shorter than the {} floor",
                format_every(every),
                format_every(min)
            ),
            "the setting schedule.min_minutes keeps standing jobs from turning into a loop",
            format!("--every {}", format_every(min)),
        ));
    }
    let at = parse_at(&input.at)?;
    if at.is_some() && every % MINUTES_PER_DAY != 0 {
        return Err(self::input(
            2,
            "--at needs a cadence of whole days",
            "a time of day anchors daily and weekly schedules",
            "--every 1d --at 09:00",
        ));
    }
    Ok(Validated {
        title,
        kind,
        brief: input.brief.clone(),
        script,
        job_kind,
        agent,
        skill,
        origin: input.origin.trim().to_string(),
        every,
        at,
        warning,
    })
}

/// A schedule's `--skill` names an enabled skill of the project's registry
/// (its own or a global one), so the worker can load it.
fn check_skill(s: &hive_store::Store, project: &str, skill: &str) -> Result<()> {
    let known = s.skills_for(project)?;
    if known.iter().any(|k| k.name == skill) {
        return Ok(());
    }
    let hint = if let Some(k) = s.skill_in(project, skill)?.or(s.skill_named(skill)?) {
        if k.status != "enabled" {
            format!("it is disabled: hive skill enable {}", k.id)
        } else {
            format!(
                "it belongs to {}: hive skill update {} --project '' shares it",
                k.project, k.id
            )
        }
    } else {
        format!(
            "hive skill list --project {project} shows them; hive skill add, or a SKILL.md in the repo's .hive/skills folder, adds one"
        )
    };
    Err(self::input(
        2,
        format!("No enabled skill {skill:?} for {project}"),
        "a scheduled job loads the skill by name from the hive skill registry",
        hint,
    ))
}

pub fn get(s: &hive_store::Store, id: &str) -> Result<Schedule> {
    let conn = s.conn()?;
    conn.query_row(
        &format!("SELECT {COLS} FROM schedules WHERE id = ?"),
        [id],
        scan,
    )
    .optional()?
    .ok_or_else(|| {
        input(
            2,
            format!("No schedule {id:?}"),
            "the id is not in this store",
            "hive schedule list",
        )
    })
}

pub fn list(s: &hive_store::Store, project: &str, status: &str) -> Result<Vec<Schedule>> {
    let mut q = format!("SELECT {COLS} FROM schedules WHERE 1=1");
    let mut args = Vec::new();
    if !project.is_empty() {
        q.push_str(" AND project = ?");
        args.push(project.to_string());
    }
    if !status.is_empty() {
        q.push_str(" AND status = ?");
        args.push(status.to_string());
    }
    q.push_str(
        " ORDER BY CASE status WHEN 'active' THEN 0 WHEN 'paused' THEN 1 ELSE 2 END, next_at, id",
    );
    let conn = s.conn()?;
    let mut stmt = conn.prepare(&q)?;
    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    Ok(stmt
        .query_map(rusqlite::params_from_iter(refs), scan)?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// One line of a schedule's history, newest first in `history`: what the
/// daemon did with it (a run of its script, a job it queued or skipped, a
/// state change) and what the owner or an agent did to it (added, changed,
/// paused, resumed, ended, asked to run now).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct HistoryEntry {
    /// The event's sequence number; a cursor for "older than".
    pub seq: i64,
    pub at: String,
    pub actor: String,
    /// The event's action without its `schedule.` prefix: `ran`, `fired`,
    /// `skipped`, `red`, `green`, `added`, `updated`, `paused`, `resumed`,
    /// `ended`, `run`.
    pub what: String,
    /// The run count after a `ran`/`fired`/`skipped`; 0 when not a run.
    pub run: i32,
    /// `ran`: the exit code (None for a run that could not start).
    pub rc: Option<i32>,
    pub timeout: bool,
    /// `ran`: seconds the script took.
    pub seconds: f64,
    /// `ran`/`red`/`green`: the state after the run.
    pub state: String,
    /// `red`/`green`: the state before.
    pub previous: String,
    /// The job queued or found open, with its status now when it is known.
    pub job: String,
    pub job_status: String,
    pub job_agent: String,
    pub job_finished_at: String,
    /// `fired`/`skipped`: whether a new job was queued.
    pub queued: bool,
    /// `fired`/`skipped`: why a job was wanted (`verdict.job_why`).
    pub why: String,
    /// `paused`/`ended`: the note; `red`/`green`: the finding posted.
    pub note: String,
    /// `updated`: the fields that changed.
    pub changed: Vec<String>,
    /// `added`/`updated`: the file that holds the schedule
    /// (`repo:<path>#<name>`), "" for one set by hand.
    pub origin: String,
}

/// The actions `history` reads, every one written with the schedule id as
/// its subject.
pub const HISTORY_ACTIONS: &[&str] = &[
    "schedule.added",
    "schedule.updated",
    "schedule.paused",
    "schedule.resumed",
    "schedule.ended",
    "schedule.run",
    "schedule.ran",
    "schedule.fired",
    "schedule.skipped",
    "schedule.red",
    "schedule.green",
];

/// What happened to a schedule, newest first: at most `limit` entries with
/// a sequence number below `before` (0 for the newest). A job named by an
/// entry carries its status as it is now, so the line says whether the
/// work it started is done.
pub fn history(
    s: &hive_store::Store,
    id: &str,
    before: i64,
    limit: usize,
) -> Result<Vec<HistoryEntry>> {
    let conn = s.conn()?;
    let limit = limit.clamp(1, 500);
    let before = if before <= 0 { i64::MAX } else { before };
    let mut stmt = conn.prepare(
        "SELECT seq, at, actor, action, detail FROM events WHERE subject = ? AND seq < ? AND action LIKE 'schedule.%' ORDER BY seq DESC LIMIT ?",
    )?;
    let rows = stmt
        .query_map(params![id, before, limit as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::with_capacity(rows.len());
    for (seq, at, actor, action, detail) in rows {
        let Some(what) = action.strip_prefix("schedule.") else {
            continue;
        };
        if !HISTORY_ACTIONS.contains(&action.as_str()) {
            continue;
        }
        let d: serde_json::Value = serde_json::from_str(&detail).unwrap_or_default();
        let text = |k: &str| d.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let int = |k: &str| d.get(k).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let flag = |k: &str| d.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
        let note = if what == "red" || what == "green" {
            text("post")
        } else {
            text("note")
        };
        out.push(HistoryEntry {
            seq,
            at,
            actor,
            what: what.to_string(),
            run: int("run"),
            rc: d.get("rc").and_then(|v| v.as_i64()).map(|v| v as i32),
            timeout: flag("timeout"),
            seconds: d.get("seconds").and_then(|v| v.as_f64()).unwrap_or(0.0),
            state: text("state"),
            previous: text("previous"),
            job: text("job"),
            queued: match what {
                "fired" => true,
                "skipped" => false,
                _ => flag("queued"),
            },
            why: text("why"),
            origin: text("origin"),
            note,
            changed: d
                .get("changed")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            ..Default::default()
        });
    }
    // The jobs as they stand now, one lookup each (a history page names a
    // few dozen at most).
    let mut job_stmt =
        conn.prepare("SELECT status, lease_agent, finished_at FROM jobs WHERE id = ?")?;
    for e in &mut out {
        if e.job.is_empty() {
            continue;
        }
        if let Some((status, agent, finished)) = job_stmt
            .query_row([&e.job], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .optional()?
        {
            e.job_status = status;
            e.job_agent = agent;
            e.job_finished_at = finished;
        }
    }
    Ok(out)
}

/// The jobs a schedule queued, newest first.
pub fn jobs(s: &hive_store::Store, id: &str) -> Result<Vec<crate::tasks::Job>> {
    let conn = s.conn()?;
    let mut stmt = conn.prepare("SELECT id, kind, title, status, lease_agent, branch, priority, created_at, finished_at, result FROM jobs WHERE source = ? ORDER BY created_at DESC")?;
    Ok(stmt
        .query_map([format!("{SOURCE_PREFIX}{id}")], |r| {
            Ok(crate::tasks::Job {
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

fn owned_by(sched: &Schedule, author: &str) -> Result<()> {
    if author.is_empty() {
        return Err(input(
            2,
            "An author is needed",
            "changes to a schedule record who made them",
            "pass --agent NAME or set HIVE_AGENT",
        ));
    }
    if author != sched.created_by && author != hive_store::HUMAN_AGENT {
        return Err(input(
            2,
            format!("Schedule {} belongs to {}", sched.id, sched.created_by),
            "only the agent that set a schedule, or the owner, changes it",
            format!(
                "post a question for {} on the board, or ask the owner",
                sched.created_by
            ),
        ));
    }
    Ok(())
}

fn set_status(
    s: &hive_store::Store,
    id: &str,
    status: &str,
    note: &str,
    author: &str,
    action: &str,
) -> Result<Schedule> {
    let sched = get(s, id)?;
    owned_by(&sched, author)?;
    set_status_of(s, sched, status, note, author, action)
}

/// `pause`, `resume` or `end` for the project's Hive file: the file owns
/// the row, so the daemon that applies it needs no ownership.
pub(crate) fn set_status_from_file(
    s: &hive_store::Store,
    id: &str,
    status: &str,
    note: &str,
    author: &str,
) -> Result<Schedule> {
    let sched = get(s, id)?;
    let action = match status {
        "paused" => "schedule.paused",
        "active" => "schedule.resumed",
        _ => "schedule.ended",
    };
    set_status_of(s, sched, status, note, author, action)
}

fn set_status_of(
    s: &hive_store::Store,
    sched: Schedule,
    status: &str,
    note: &str,
    author: &str,
    action: &str,
) -> Result<Schedule> {
    let id = sched.id.clone();
    let id = id.as_str();
    if sched.status == "ended" {
        return Err(input(
            2,
            format!("Schedule {id} has ended"),
            "an ended schedule stays in the list for its history and never fires again",
            "hive schedule add ... for a new one",
        ));
    }
    if sched.status == status {
        // Already there: a repeated pause or resume changes nothing, so it
        // writes no event and keeps the note from the change that counted.
        return Ok(sched);
    }
    let now = s.now();
    let tx = s.begin()?;
    tx.execute(
        "UPDATE schedules SET status = ?, note = ?, updated_at = ? WHERE id = ?",
        params![status, clean(note), now, id],
    )?;
    tx.event(
        author,
        action,
        id,
        hive_store::Detail::json(json!({"project": sched.project, "note": clean(note)}))?,
    )?;
    tx.commit()?;
    get(s, id)
}

/// Pauses a schedule: it keeps its place and fires again after `resume`.
pub fn pause(s: &hive_store::Store, id: &str, note: &str, author: &str) -> Result<Schedule> {
    set_status(s, id, "paused", note, author, "schedule.paused")
}

/// Resumes a paused schedule; a due time that passed while paused fires on
/// the next tick, once.
pub fn resume(s: &hive_store::Store, id: &str, author: &str) -> Result<Schedule> {
    set_status(s, id, "active", "", author, "schedule.resumed")
}

/// Ends a schedule for good; its history stays.
pub fn end(s: &hive_store::Store, id: &str, note: &str, author: &str) -> Result<Schedule> {
    set_status(s, id, "ended", note, author, "schedule.ended")
}

/// Makes the schedule due now; the daemon queues its job on the next tick.
/// A schedule with an `at` time of day comes back to it after this run
/// (`advance_at`).
pub fn run_now(s: &hive_store::Store, id: &str, author: &str) -> Result<Schedule> {
    let sched = get(s, id)?;
    owned_by(&sched, author)?;
    if sched.status != "active" {
        return Err(input(
            2,
            format!("Schedule {id} is {}", sched.status),
            "only an active schedule fires",
            format!("hive schedule resume {id}"),
        ));
    }
    let now = s.now();
    let tx = s.begin()?;
    tx.execute(
        "UPDATE schedules SET next_at = ?, updated_at = ? WHERE id = ?",
        params![now, now, id],
    )?;
    tx.event(
        author,
        "schedule.run",
        id,
        hive_store::Detail::json(json!({"project": sched.project}))?,
    )?;
    tx.commit()?;
    get(s, id)
}

/// The brief a scheduled job carries: the schedule's brief and where it came
/// from, so the worker knows the job recurs and how to change it.
pub fn job_brief(sched: &Schedule, run: i32) -> String {
    let mut brief = sched.brief.trim_end().to_string();
    if !sched.skill.is_empty() {
        let line = format!(
            "Load the skill `{}` first (the skill tool, or `hive skill show {}`) and follow it{}",
            sched.skill,
            sched.skill,
            if brief.is_empty() {
                "; it is the whole brief."
            } else {
                " for the work below."
            }
        );
        brief = if brief.is_empty() {
            line
        } else {
            format!("{line}\n\n{brief}")
        };
    }
    brief.push_str(&format!(
        "\n\n---\nThis job is run {run} of schedule {} (every {}), set by {}. Do the work above and finish as usual; the schedule queues the next run by itself. `hive schedule show {}` has its history; only {} or the owner pause or end it.",
        sched.id,
        sched.every(),
        sched.created_by,
        sched.id,
        sched.created_by
    ));
    if sched.max_runs > 0 {
        brief.push_str(&format!(" It ends after {} runs.", sched.max_runs));
    }
    brief
}

/// What one run of a check or watch script decided, from its exit code and
/// output alone; the daemon acts on it (queues the job, posts the finding)
/// and then records it with `record_script_run`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Verdict {
    /// The state after this run: green/red for a check, ok/broken for a
    /// watch.
    pub state: String,
    /// Consecutive non-zero exits after this run.
    pub fails: i32,
    /// The state changed: a check went red or came back green, a watch broke
    /// or recovered. A first run that passes is not a transition.
    pub transition: bool,
    /// Why a job is queued this run, if one is: `red` (a failing check),
    /// `trigger` (a watch that printed something), `fix` (a watch broken
    /// `WATCH_FIX_AFTER` runs in a row); empty for none.
    pub job_why: String,
}

/// Reads one script run: `rc` is the exit code (-1 for a timeout), `stdout`
/// what the script printed.
pub fn verdict(sched: &Schedule, rc: i32, stdout: &str) -> Verdict {
    let good = sched.good_state();
    let bad = sched.bad_state();
    let passed = rc == 0;
    let state = if passed { good } else { bad }.to_string();
    let fails = if passed { 0 } else { sched.fails + 1 };
    let transition = if passed {
        sched.state == bad
    } else {
        sched.state != bad
    };
    let job_why = match (sched.kind.as_str(), passed) {
        ("check", false) => "red",
        ("watch", true) if !stdout.trim().is_empty() => "trigger",
        ("watch", false) if fails >= WATCH_FIX_AFTER => "fix",
        _ => "",
    }
    .to_string();
    Verdict {
        state,
        fails,
        transition,
        job_why,
    }
}

fn exit_text(rc: i32) -> String {
    if rc < 0 {
        "timed out and was killed".to_string()
    } else {
        format!("exit {rc}")
    }
}

fn quoted(tail: &str) -> String {
    let tail = tail.trim();
    if tail.is_empty() {
        return "    (no output)".to_string();
    }
    tail.lines()
        .map(|l| format!("    {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The brief of a job a check or watch queues: the schedule's own brief (when
/// it has one), what the script said, and where the schedule lives.
pub fn script_job_brief(sched: &Schedule, run: i32, body: &str) -> String {
    let mut brief = sched.brief.trim().to_string();
    if !brief.is_empty() {
        brief.push_str("\n\n");
    }
    brief.push_str(body.trim_end());
    brief.push_str(&format!(
        "\n\n---\nThis job comes from run {run} of schedule {} (a {} of `{}`, every {}), set by {}. `hive schedule show {}` has its history and `hive schedule log {}` the last output; only {} or the owner pause or end it.",
        sched.id,
        sched.kind,
        sched.script,
        sched.every(),
        sched.created_by,
        sched.id,
        sched.id,
        sched.created_by
    ));
    if sched.max_runs > 0 {
        brief.push_str(&format!(" It ends after {} runs.", sched.max_runs));
    }
    brief
}

/// Where a script ran: the default branch and its commit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tree {
    pub branch: String,
    pub sha: String,
}

impl Tree {
    pub fn at(&self) -> String {
        let short: String = self.sha.chars().take(12).collect();
        if short.is_empty() {
            self.branch.clone()
        } else {
            format!("{}@{short}", self.branch)
        }
    }
}

/// The brief of the job a red check queues.
pub fn check_job_brief(
    sched: &Schedule,
    run: i32,
    rc: i32,
    now: &str,
    tree: &Tree,
    tail: &str,
) -> String {
    script_job_brief(
        sched,
        run,
        &format!(
            "The check `{}` failed ({}) at {now} on {}. {}\n\nMake it pass on the default branch. If the check itself is wrong, fix the script, or end the schedule with `hive schedule end {} --note WHY` and say why.",
            sched.script,
            exit_text(rc),
            tree.at(),
            check_output_quote(tail),
            sched.id
        ),
    )
}

/// The line `scripts/verify-check.sh` prints on stderr before its summary of
/// a red run: which chunks failed and which tests.
const FAILED_CHUNKS_MARKER: &str = "verify-check: failed chunks:";

/// What a red check's brief quotes from its output. When the output holds a
/// `verify-check: failed chunks:` summary, the lines from the last one on
/// come first - they name the failing tests - followed by the output before
/// them, so a long run of compiler or test noise can never push the failing
/// test's name out of the brief. Each part keeps at most `OUTPUT_TAIL` chars.
fn check_output_quote(output: &str) -> String {
    let at = output
        .rmatch_indices(FAILED_CHUNKS_MARKER)
        .map(|(i, _)| i)
        .find(|&i| {
            let rest = &output[i + FAILED_CHUNKS_MARKER.len()..];
            (i == 0 || output[..i].ends_with('\n'))
                && (rest.is_empty() || rest.starts_with('\n') || rest.starts_with("\r\n"))
        });
    match at {
        Some(i) => {
            let summary: String = output[i..].chars().take(OUTPUT_TAIL).collect();
            format!(
                "Failing chunks and tests:\n\n{}\n\nOutput tail before them:\n\n{}",
                quoted(&summary),
                quoted(&last_chars(&output[..i], OUTPUT_TAIL))
            )
        }
        None => format!(
            "Output tail:\n\n{}",
            quoted(&last_chars(output, OUTPUT_TAIL))
        ),
    }
}

/// The last `n` chars of `s`.
fn last_chars(s: &str, n: usize) -> String {
    let skip = s.chars().count().saturating_sub(n);
    s.chars().skip(skip).collect()
}

/// The brief of the job a watch queues when its script printed something.
pub fn watch_job_brief(sched: &Schedule, run: i32, now: &str, stdout: &str) -> String {
    script_job_brief(
        sched,
        run,
        &format!(
            "The watcher `{}` reported at {now}:\n\n{}\n\nHandle what it reports; when done, the next run of the watcher will see your changes.",
            sched.script,
            quoted(stdout)
        ),
    )
}

/// The brief of the job that fixes a watch script failing run after run.
pub fn watch_fix_brief(
    sched: &Schedule,
    run: i32,
    rc: i32,
    fails: i32,
    now: &str,
    tree: &Tree,
    tail: &str,
) -> String {
    script_job_brief(
        sched,
        run,
        &format!(
            "The watcher `{}` itself is broken: it failed ({}) {fails} runs in a row, last at {now} on {}. Output tail:\n\n{}\n\nFix the watcher so it exits 0 again: print nothing when there is nothing to do, print what needs handling otherwise. If it is no longer wanted, end the schedule with `hive schedule end {} --note WHY` and say why.",
            sched.script,
            exit_text(rc),
            tree.at(),
            quoted(tail),
            sched.id
        ),
    )
}

/// The title of the job a script schedule queues.
pub fn script_job_title(sched: &Schedule, why: &str) -> String {
    let title = match why {
        "red" => format!("Make check `{}` pass: {}", sched.script, sched.title),
        "fix" => format!("Fix watcher `{}`: {}", sched.script, sched.title),
        _ => sched.title.clone(),
    };
    clean(&title).chars().take(200).collect()
}

/// The active check and watch schedules of `projects` whose time has come,
/// the longest overdue first.
pub fn due_scripts(s: &hive_store::Store, now: &str, projects: &[String]) -> Result<Vec<Schedule>> {
    let mut due = list(s, "", "active")?
        .into_iter()
        .filter(|sc| sc.is_script())
        .filter(|sc| sc.next_at.as_str() <= now)
        .filter(|sc| projects.is_empty() || projects.contains(&sc.project))
        .collect::<Vec<_>>();
    due.sort_by(|a, b| a.next_at.cmp(&b.next_at).then_with(|| a.id.cmp(&b.id)));
    Ok(due)
}

/// One finished run of a check or watch script, as the daemon records it.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ScriptRun {
    /// Exit code; -1 for a timeout; `None` when the script could not be
    /// started at all (no worktree), which counts as a run but changes no
    /// state and queues nothing.
    pub rc: Option<i32>,
    pub timeout: bool,
    pub seconds: f64,
    /// The redacted tail of stdout and stderr, at most `OUTPUT_TAIL` chars.
    pub output: String,
    /// What `verdict` said; ignored when `rc` is `None`.
    pub verdict: Verdict,
    /// The job queued or found open this run, and whether it was new.
    pub job: String,
    pub queued: bool,
    /// The finding posted this run, if any.
    pub post: String,
}

/// What `record_script_run` left behind.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Recorded {
    pub schedule: Schedule,
    /// The state before this run.
    pub previous: String,
    pub ended: bool,
}

/// Writes one script run to the schedule - state, exit code, output tail,
/// run count, next time - with its events: `schedule.ran` always,
/// `schedule.red`/`schedule.green` on a transition, `schedule.fired` or
/// `schedule.skipped` when a job was wanted, `schedule.ended` when the run
/// count is up.
pub fn record_script_run(
    s: &hive_store::Store,
    sched: &Schedule,
    now: &str,
    actor: &str,
    run: &ScriptRun,
) -> Result<Recorded> {
    let runs = sched.runs + 1;
    let ended = sched.max_runs > 0 && runs >= sched.max_runs;
    let next_at = sched.next_after(now)?;
    let note = if ended {
        format!("ended after {runs} runs")
    } else {
        sched.note.clone()
    };
    let started = run.rc.is_some();
    let (state, fails, transition) = if started {
        (
            run.verdict.state.clone(),
            run.verdict.fails,
            run.verdict.transition,
        )
    } else {
        (sched.state.clone(), sched.fails, false)
    };
    let state_since = if started && (transition || sched.state_since.is_empty()) {
        now.to_string()
    } else {
        sched.state_since.clone()
    };
    let output: String = run
        .output
        .chars()
        .rev()
        .take(OUTPUT_TAIL)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let tx = s.begin()?;
    tx.execute(
        "UPDATE schedules SET runs = ?, last_at = ?, last_job = CASE WHEN ? THEN ? ELSE last_job END,
         next_at = ?, status = CASE WHEN ? THEN 'ended' ELSE status END, note = ?, updated_at = ?,
         state = ?, state_since = ?, last_rc = ?, last_output = ?, fails = ? WHERE id = ?",
        params![
            runs,
            now,
            run.queued,
            run.job,
            next_at,
            ended,
            note,
            now,
            state,
            state_since,
            run.rc,
            output,
            fails,
            sched.id
        ],
    )?;
    tx.event(
        actor,
        "schedule.ran",
        &sched.id,
        hive_store::Detail::json(json!({
            "project": sched.project,
            "kind": sched.kind,
            "script": sched.script,
            "rc": run.rc,
            "timeout": run.timeout,
            "seconds": (run.seconds * 10.0).round() / 10.0,
            "state": state,
            "fails": fails,
            "run": runs,
            "next_at": next_at,
            "job": run.job,
            "queued": run.queued,
            "ended": ended,
        }))?,
    )?;
    if started && transition {
        tx.event(
            actor,
            if state == sched.bad_state() {
                "schedule.red"
            } else {
                "schedule.green"
            },
            &sched.id,
            hive_store::Detail::json(json!({
                "project": sched.project,
                "kind": sched.kind,
                "script": sched.script,
                "rc": run.rc,
                "state": state,
                "previous": sched.state,
                "job": run.job,
                "post": run.post,
            }))?,
        )?;
    }
    if !run.job.is_empty() {
        tx.event(
            actor,
            if run.queued {
                "schedule.fired"
            } else {
                "schedule.skipped"
            },
            &sched.id,
            hive_store::Detail::json(json!({
                "project": sched.project,
                "job": run.job,
                "why": run.verdict.job_why,
                "run": runs,
                "next_at": next_at,
                "ended": ended,
            }))?,
        )?;
    }
    if ended {
        tx.event(
            actor,
            "schedule.ended",
            &sched.id,
            hive_store::Detail::json(json!({"project": sched.project, "note": note}))?,
        )?;
    }
    tx.commit()?;
    Ok(Recorded {
        schedule: get(s, &sched.id)?,
        previous: sched.state.clone(),
        ended,
    })
}

/// Queues a job for every active schedule of `projects` whose time has come
/// and moves each on to its next time. One open job per schedule: when the
/// previous run's job is still queued or leased, nothing new is queued and
/// the tick is skipped. Called by the daemon once per loop. Checks and
/// watches are not fired here: the daemon runs their scripts (`due_scripts`,
/// `record_script_run`).
pub fn fire_due(
    s: &hive_store::Store,
    now: &str,
    projects: &[String],
    actor: &str,
) -> Result<Vec<Fired>> {
    let due = list(s, "", "active")?
        .into_iter()
        .filter(|sc| !sc.is_script())
        .filter(|sc| sc.next_at.as_str() <= now)
        .filter(|sc| projects.is_empty() || projects.contains(&sc.project))
        .collect::<Vec<_>>();
    let mut out = Vec::new();
    for sched in due {
        let run = sched.runs + 1;
        let (job, queued) = hive_board::add_job(
            s,
            hive_board::JobInput {
                project: sched.project.clone(),
                kind: sched.kind.clone(),
                title: sched.title.clone(),
                brief: job_brief(&sched, run),
                tier: i64::from(sched.tier),
                priority: i64::from(sched.priority),
                source: sched.source(),
                created_by: sched.created_by.clone(),
                agent: sched.agent.clone(),
                ..Default::default()
            },
        )?;
        let runs = if queued { run } else { sched.runs };
        let ended = sched.max_runs > 0 && runs >= sched.max_runs;
        let next_at = sched.next_after(now)?;
        let note = if ended {
            format!("ended after {runs} runs")
        } else {
            sched.note.clone()
        };
        let tx = s.begin()?;
        tx.execute(
            "UPDATE schedules SET runs = ?, last_at = CASE WHEN ? THEN ? ELSE last_at END,
             last_job = CASE WHEN ? THEN ? ELSE last_job END, next_at = ?,
             status = CASE WHEN ? THEN 'ended' ELSE status END, note = ?, updated_at = ? WHERE id = ?",
            params![runs, queued, now, queued, job.id, next_at, ended, note, now, sched.id],
        )?;
        tx.event(
            actor,
            if queued {
                "schedule.fired"
            } else {
                "schedule.skipped"
            },
            &sched.id,
            hive_store::Detail::json(json!({
                "project": sched.project,
                "job": job.id,
                "run": runs,
                "next_at": next_at,
                "ended": ended,
            }))?,
        )?;
        if ended {
            tx.event(
                actor,
                "schedule.ended",
                &sched.id,
                hive_store::Detail::json(json!({"project": sched.project, "note": note}))?,
            )?;
        }
        tx.commit()?;
        out.push(Fired {
            schedule: get(s, &sched.id)?,
            job: job.id,
            queued,
            ended,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod check_brief_tests {
    use super::{OUTPUT_TAIL, Schedule, Tree, check_job_brief};

    /// A red verify run whose cargo noise alone is longer than the output
    /// tail still yields a brief that names the failing test, ahead of the
    /// noise, and the brief stays bounded.
    #[test]
    fn check_brief_names_the_failing_test_past_long_noise() {
        let sched = Schedule {
            id: "SCHED-009".into(),
            script: "scripts/verify-check.sh".into(),
            ..Schedule::default()
        };
        let tree = Tree {
            branch: "main".into(),
            sha: "0123456789abcdef".into(),
        };
        let noise = "   Compiling hive-noise v0.1.0 (/x/y/z)\n".repeat(OUTPUT_TAIL / 20);
        let output = format!(
            "verify-check: failed chunks:\n  stale summary chunk 1: fail\n{noise}[stderr]\n{noise}verify-check: failed chunks:\n  hive_store chunk 2: fail (exit 101, 1 failed)\nverify-check: failing tests:\n  store::tests::keeps_rows\nverify-check: 1 chunk(s) failed\n"
        );
        assert!(output.len() > 2 * OUTPUT_TAIL);
        let brief = check_job_brief(&sched, 1, 1, "t", &tree, &output);
        let named = brief.find("    store::tests::keeps_rows").expect(&brief);
        let noise_at = brief.find("Compiling hive-noise").expect(&brief);
        assert!(named < noise_at, "{brief}");
        assert!(brief.contains("Failing chunks and tests:\n\n    verify-check: failed chunks:\n      hive_store chunk 2"), "{brief}");
        assert!(!brief.contains("stale summary"), "{brief}");
        assert!(brief.len() < 3 * OUTPUT_TAIL, "{}", brief.len());

        let plain = check_job_brief(
            &sched,
            1,
            1,
            "t",
            &tree,
            "abc\nverify-check: failed chunks: inline only\n",
        );
        assert!(
            plain.contains("Output tail:\n\n    abc\n    verify-check: failed chunks: inline only"),
            "{plain}"
        );
    }
}

#[cfg(test)]
mod every_tests {
    use super::{MINUTES_PER_DAY, format_every, parse_every};

    /// Representative minute values at every unit boundary: the display an
    /// owner sees in `hive schedule show` must parse back to the same cadence.
    #[test]
    fn format_every_round_trips_across_unit_boundaries() {
        let cases = [
            1,
            59,
            60,
            61,
            90,
            MINUTES_PER_DAY - 1,
            MINUTES_PER_DAY,
            MINUTES_PER_DAY + 1,
            2 * MINUTES_PER_DAY,
            6 * MINUTES_PER_DAY,
            7 * MINUTES_PER_DAY - 1,
            7 * MINUTES_PER_DAY,
            7 * MINUTES_PER_DAY + 60,
            14 * MINUTES_PER_DAY,
            i64::from(i32::MAX),
        ];
        for minutes in cases {
            let text = format_every(minutes);
            assert_eq!(
                parse_every(&text).ok(),
                Some(minutes),
                "format_every({minutes}) = {text:?} did not round-trip"
            );
        }
    }

    /// At each exact boundary the largest whole unit wins.
    #[test]
    fn format_every_uses_largest_whole_unit() {
        assert_eq!(format_every(1), "1m");
        assert_eq!(format_every(59), "59m");
        assert_eq!(format_every(60), "1h");
        assert_eq!(format_every(90), "90m");
        assert_eq!(format_every(MINUTES_PER_DAY - 1), "1439m");
        assert_eq!(format_every(MINUTES_PER_DAY), "1d");
        assert_eq!(format_every(2 * MINUTES_PER_DAY), "2d");
        assert_eq!(format_every(6 * MINUTES_PER_DAY), "6d");
        assert_eq!(format_every(7 * MINUTES_PER_DAY), "1w");
        assert_eq!(format_every(14 * MINUTES_PER_DAY), "2w");
        assert_eq!(format_every(7 * MINUTES_PER_DAY + 60), "169h");
    }
}
