use crate::lists::join_counts;
use crate::register::{json_output, no_args, py_repr, store};
use crate::settings::agent_setting_value;
use anyhow::{Result, anyhow};
use chrono::{DateTime, Duration, Utc};
use hive_cli::Context;
use hive_cli::flags::FlagSet;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusCount {
    pub status: String,
    pub n: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StatusReport {
    pub store: String,
    pub schema_version: String,
    pub projects: Vec<hive_store::Project>,
    pub agents: Vec<StatusCount>,
    pub posts: i64,
    pub jobs: Vec<StatusCount>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schedules: Vec<StatusCount>,
    pub runs_today: i64,
    pub daily_cap: i64,
    /// What today's finished runs cost, per job kind (runs.prompt_tokens,
    /// completion_tokens and cost, summed onto each run as it ends).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cost_today: Vec<KindCost>,
    /// How today's finished runs were classified (runs.outcome_source):
    /// `code` from the worker's SUMMARY outcome code, `text` from the regex
    /// fallback, `none` without a summary.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outcomes_today: Vec<StatusCount>,
    pub runner: String,
    pub harness: String,
    pub daemon: String,
    pub bus: BusStatus,
    pub planner_cache: PlannerCacheStatus,
    pub executors: String,
    pub capacity: String,
    /// The fleet's slots as the capacity model works them out:
    /// "slots 18 = this machine 10 + vm-1 8, per project 6".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub slots: String,
    /// Bulk storage on another disk (hive bulk); empty while local.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub bulk: String,
    pub hygiene: String,
    pub planner: Vec<PlannerNote>,
    pub recovery: Vec<RecoveryNote>,
    pub landing: Vec<LandingNote>,
    /// The owner's jobs queued past ten minutes and what held them
    /// (hive_runner::owner_wait): "proj job-x (Owner step) waiting since
    /// 04:40 UTC: lanes full, load 27 > 24".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waiting: Vec<String>,
    pub settings: Vec<hive_store::SettingRow>,
}

/// One job kind's runs of a day and what they cost.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct KindCost {
    pub kind: String,
    pub runs: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    /// The providers' priced cost, in their units; 0 where none reports one.
    pub cost: f64,
}

/// Runs that finished on `day` (YYYY-MM-DD, UTC) and what they cost, per
/// job kind, the dearest kind first.
pub fn cost_by_kind(conn: &rusqlite::Connection, day: &str) -> Result<Vec<KindCost>> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(j.kind, '?') AS kind, COUNT(*), COALESCE(SUM(r.prompt_tokens), 0), \
                COALESCE(SUM(r.completion_tokens), 0), COALESCE(SUM(r.cost), 0) \
         FROM runs r LEFT JOIN jobs j ON j.id = r.job_id \
         WHERE substr(r.finished_at, 1, 10) = ?1 AND r.status != 'running' \
         GROUP BY kind ORDER BY 5 DESC, 3 + 4 DESC, kind",
    )?;
    let rows = stmt.query_map([day], |r| {
        Ok(KindCost {
            kind: r.get(0)?,
            runs: r.get(1)?,
            prompt_tokens: r.get(2)?,
            completion_tokens: r.get(3)?,
            cost: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// A token count for the status line: 980, 12.3k, 4.1M.
fn tokens_text(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// The `cost` row of the text status: one kind per entry.
fn kind_cost_text(k: &KindCost) -> String {
    let runs = if k.runs == 1 { "run" } else { "runs" };
    let mut text = format!(
        "{} {} {}, {} prompt + {} completion tokens",
        k.kind,
        k.runs,
        runs,
        tokens_text(k.prompt_tokens),
        tokens_text(k.completion_tokens)
    );
    if k.cost > 0.0 {
        text.push_str(&format!(", cost {:.2}", k.cost));
    }
    text
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusStatus {
    pub subscribers: i64,
    pub dropped: i64,
    pub deduped: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannerCacheStatus {
    pub hits: u64,
    pub misses: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannerNote {
    pub project: String,
    pub at: String,
    pub note: String,
    pub stale: bool,
}

pub type RecoveryNote = PlannerNote;
pub type LandingNote = PlannerNote;

pub fn status_counts(s: &hive_store::Store, query: &str) -> Result<Vec<StatusCount>> {
    let conn = s.conn()?;
    let mut stmt = conn.prepare(query)?;
    let rows = stmt.query_map([], |r| {
        Ok(StatusCount {
            status: r.get(0)?,
            n: r.get(1)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn parse_utc(text: &str) -> Result<DateTime<Utc>> {
    let t = DateTime::parse_from_rfc3339(text).map_err(|_| {
        hive_cli::errorf(
            2,
            &format!(
                "hive: Not a UTC time like 2026-09-05T08:00:00Z: {}",
                py_repr(text)
            ),
            "",
            "",
        )
    })?;
    if t.offset().local_minus_utc() != 0 || text.len() != 20 || !text.ends_with('Z') {
        return Err(hive_cli::errorf(
            2,
            &format!(
                "hive: Not a UTC time like 2026-09-05T08:00:00Z: {}",
                py_repr(text)
            ),
            "",
            "",
        )
        .into());
    }
    Ok(t.with_timezone(&Utc))
}

pub fn setting_int(values: &BTreeMap<String, String>, name: &str) -> i64 {
    let raw = values
        .get(name)
        .cloned()
        .or_else(|| hive_store::DEFAULT_SETTINGS.get(name).cloned())
        .unwrap_or_default();
    raw.trim().parse().unwrap_or_else(|_| {
        hive_store::DEFAULT_SETTINGS
            .get(name)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    })
}

/// The daily cap as the runner reads it (`hive_store::daily_cap_of`): 0
/// for none; an unparsable value falls back to the shipped default.
pub fn daily_cap(values: &BTreeMap<String, String>) -> i64 {
    let raw = values
        .get("jobs_per_day")
        .cloned()
        .or_else(|| hive_store::DEFAULT_SETTINGS.get("jobs_per_day").cloned())
        .unwrap_or_default();
    let fallback = hive_store::DEFAULT_SETTINGS
        .get("jobs_per_day")
        .and_then(|d| d.parse::<i64>().ok())
        .unwrap_or(0);
    hive_store::daily_cap_of(&raw, fallback)
}

pub fn runner_stopped(s: &hive_store::Store) -> Result<String> {
    let values = s.settings()?;
    if values.get("stop").is_some_and(|v| v == "1") {
        return Ok("the runner is stopped in the store (hive start to resume)".into());
    }
    if let Some(brain) = s.brain()? {
        let stop = Path::new(&brain.path).join("docs/evolution/STOP");
        match std::fs::metadata(&stop) {
            Ok(_) => {
                return Ok(format!(
                    "docs/evolution/STOP is present in the brain ({}); a human removes it",
                    brain.path
                ));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(String::new())
}

/// How long `hive status` waits for the provider's login check. The check
/// may ask the vendor (GitHub Copilot asks api.github.com about the token it
/// found) through an HTTP client whose whole-request timeout is 120 s, and
/// the token exchange that follows is a second such request; a status screen
/// must not hang for minutes on a slow or unreachable host.
pub const HARNESS_STATUS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

pub fn harness_line(s: &hive_store::Store) -> String {
    harness_line_within(s, HARNESS_STATUS_TIMEOUT)
}

/// [`harness_line`] with the wait for the provider's answer bounded by
/// `timeout`. The check runs on its own thread with a cancel token that
/// expires at the same deadline, so a provider that looks at the token stops
/// early; one blocked inside a request is left to finish on its own while
/// this line says the status is unknown.
pub fn harness_line_within(s: &hive_store::Store, timeout: std::time::Duration) -> String {
    let name = s.setting("provider", hive_provider::DEFAULT);
    let mut label = hive_provider::label_of(&name);
    if label.is_empty() {
        label = name.clone();
    }
    let cancel = hive_provider::CancelToken::new().with_timeout(timeout);
    let p = match hive_provider::open(
        &name,
        hive_provider::Options {
            home: s.home.clone(),
            ..Default::default()
        },
    ) {
        Ok(p) => p,
        Err(err) => {
            return format!(
                "native · {label} · status unknown ({})",
                one_status_line(&err.to_string(), 200)
            );
        }
    };
    let label = p.label().to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("hive-status-harness".into())
        .spawn(move || {
            // The receiver is gone once the wait below gave up; nobody
            // needs this answer any more.
            let _ = tx.send(p.status(&cancel));
        });
    if let Err(err) = spawned {
        return format!(
            "native · {label} · status unknown (could not start the login check: {})",
            one_status_line(&err.to_string(), 200)
        );
    }
    let answer = match rx.recv_timeout(timeout) {
        Ok(answer) => answer,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            return format!(
                "native · {label} · status unknown (no answer within {}; the provider may be slow or unreachable - hive harness status waits for it)",
                hive_provider::http::format_duration(timeout)
            );
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            return format!(
                "native · {label} · status unknown (the login check stopped without an answer)"
            );
        }
    };
    match answer {
        Ok(login) if login.logged_in => {
            format!("native · {label} · signed in as {}", login.user)
        }
        Ok(login) if !login.detail.is_empty() => format!(
            "native · {label} · not signed in - {}",
            one_status_line(&login.detail, 240)
        ),
        Ok(_) => format!(
            "native · {label} · not signed in - open Settings → Harness or run hive harness login"
        ),
        Err(err) => format!(
            "native · {label} · status unknown ({})",
            one_status_line(&err.to_string(), 200)
        ),
    }
}

pub fn one_status_line(s: &str, limit: usize) -> String {
    let text = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.len() > limit {
        text[..text.floor_char_boundary(limit)].to_string()
    } else {
        text
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DaemonBeat {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<i64>,
    pub at: String,
    pub loops: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval: Option<i64>,
    pub note: String,
    pub bus: BusStatus,
    pub planner_cache: PlannerCacheStatus,
    /// The loop body's timing (daemon.rs `TickTiming`); absent from
    /// heartbeats older than the measurement and before the first body ends.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tick: Option<TickBeat>,
    /// The pending restart (restart.rs `restart_beat`): since when, the
    /// runs the new build waits for and when its bound
    /// (daemon.restart_max_minutes) is up; absent when none is pending.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restart: Option<RestartBeat>,
}

/// A pending restart as the heartbeat records it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RestartBeat {
    pub since: String,
    pub inflight: u64,
    pub dispatching: Option<bool>,
    pub waits_for: Vec<RestartWait>,
    /// When the bound is up and nothing new starts; absent without one
    /// and from heartbeats older than the bound.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub until: String,
}

/// One run a pending restart waits for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RestartWait {
    pub agent: String,
    pub kind: String,
    pub project: String,
    pub job: String,
    pub started_at: String,
}

/// The status line's clause for a pending restart: since when (and how
/// long that is), what the new build waits for and when the bound closes
/// the slots - so a restart held open by the owner's queue reads as such,
/// not as a daemon that forgot to restart.
fn restart_clause(r: &RestartBeat, now: chrono::DateTime<chrono::Utc>) -> String {
    let ago = |at: &str| {
        parse_utc(at).ok().map(|t| {
            hive_cli::format_duration(std::time::Duration::from_secs(
                (now - t).num_seconds().max(0) as u64,
            ))
        })
    };
    let since = match ago(&r.since) {
        Some(d) => format!("since {} ({d})", r.since),
        None => "pending".to_string(),
    };
    let names: Vec<String> = r
        .waits_for
        .iter()
        .take(3)
        .map(|w| {
            let mut who = if w.agent.is_empty() {
                w.kind.clone()
            } else {
                format!("{}'s {}", w.agent, w.kind)
            };
            if !w.project.is_empty() {
                who.push_str(&format!(" on {}", w.project));
            }
            who
        })
        .collect();
    let waits = if names.is_empty() {
        format!("{} run(s) in flight", r.inflight)
    } else if r.waits_for.len() > names.len() {
        format!(
            "{} and {} more",
            names.join(", "),
            r.waits_for.len() - names.len()
        )
    } else {
        names.join(", ")
    };
    let starts = if r.dispatching == Some(false) {
        "; nothing new starts".to_string()
    } else if r.until.is_empty() {
        String::new()
    } else {
        match parse_utc(&r.until) {
            Ok(t) if t <= now => {
                "; nothing new starts (daemon.restart_max_minutes is up)".to_string()
            }
            _ => format!(
                "; nothing new starts after {} (daemon.restart_max_minutes)",
                r.until
            ),
        }
    };
    format!(", restart {since}, waits for {waits}{starts}")
}

/// The last loop body in milliseconds, the slowest since the daemon started
/// and how many ran over the dispatch cadence; `longest` names the slowest
/// body's longest stages (daemon.rs `StageClock`), empty from a daemon older
/// than the measurement.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TickBeat {
    pub ms: u64,
    pub max_ms: u64,
    pub slow: u64,
    pub longest: Vec<StageBeat>,
}

/// One stage of the slowest loop body and its milliseconds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StageBeat {
    pub stage: String,
    pub ms: u64,
}

pub fn read_daemon_beat(s: &hive_store::Store) -> Result<(DaemonBeat, bool)> {
    let (raw, exists) = s.meta("daemon")?;
    if !exists {
        return Ok((DaemonBeat::default(), false));
    }
    Ok((serde_json::from_str(&raw)?, true))
}

pub fn daemon_interval_seconds(s: &hive_store::Store) -> Result<i64> {
    let Ok((beat, _)) = read_daemon_beat(s) else {
        return Ok(hive_runner::DEFAULT_DAEMON_INTERVAL_SECONDS as i64);
    };
    Ok(beat
        .interval
        .filter(|n| *n != 0)
        .unwrap_or(hive_runner::DEFAULT_DAEMON_INTERVAL_SECONDS as i64))
}

pub fn daemon_status(s: &hive_store::Store) -> Result<String> {
    let (beat, exists) = match read_daemon_beat(s) {
        Ok(v) => v,
        Err(_) => return Ok("unreadable heartbeat".into()),
    };
    if !exists {
        return Ok("never started".into());
    }
    let pid = beat
        .pid
        .map(|n| n.to_string())
        .unwrap_or_else(|| "None".into());
    let interval_text = beat
        .interval
        .map(|n| n.to_string())
        .unwrap_or_else(|| "None".into());
    if beat.note == "exited" {
        return Ok(format!("exited {} (pid {pid})", beat.at));
    }
    let at = parse_utc(&beat.at)?;
    let now = parse_utc(&s.now())?;
    let age = (now - at).num_seconds();
    let seconds = beat
        .interval
        .filter(|n| *n != 0)
        .unwrap_or(hive_runner::DEFAULT_DAEMON_INTERVAL_SECONDS as i64);
    let state = if age > 2 * seconds + 60 {
        "silent"
    } else {
        "alive"
    };
    // The body's time is the daemon's own: a loop that beats on time but
    // spends its cadence in the body delays dispatch just the same.
    let tick = beat
        .tick
        .as_ref()
        .map(|t| {
            let slow = if t.slow > 0 {
                format!(", {} over the cadence", t.slow)
            } else {
                String::new()
            };
            // What the slowest body spent its time on, so a daemon over
            // its cadence names the tender to look at.
            let longest = if t.longest.is_empty() {
                String::new()
            } else {
                format!(
                    ": {}",
                    t.longest
                        .iter()
                        .map(|st| format!("{} {}ms", st.stage, st.ms))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            format!(
                ", last tick {}ms (slowest {}ms{longest}{slow})",
                t.ms, t.max_ms
            )
        })
        .unwrap_or_default();
    let restart = beat
        .restart
        .as_ref()
        .filter(|_| state == "alive")
        .map(|r| restart_clause(r, now))
        .unwrap_or_default();
    Ok(format!(
        "{state} pid {pid}, last loop {age}s ago, {} loop(s), interval {interval_text}s{tick}{restart}",
        beat.loops
    ))
}

pub fn meta_notes(s: &hive_store::Store, key: &str) -> Result<Vec<PlannerNote>> {
    let (raw, exists) = s.meta(key)?;
    if !exists {
        return Ok(vec![]);
    }
    #[derive(Deserialize)]
    struct Stored {
        at: String,
        note: String,
    }
    let stored: BTreeMap<String, Stored> =
        serde_json::from_str(&raw).map_err(|e| anyhow!("read {key} notes: {e}"))?;
    let interval = daemon_interval_seconds(s)?;
    let now = parse_utc(&s.now())?;
    let mut out = Vec::with_capacity(stored.len());
    for (project, item) in stored {
        let at = parse_utc(&item.at)?;
        out.push(PlannerNote {
            project,
            at: item.at,
            note: item.note,
            stale: now - at > Duration::seconds(2 * interval),
        });
    }
    Ok(out)
}

pub fn planner_notes(s: &hive_store::Store) -> Result<Vec<PlannerNote>> {
    meta_notes(s, "planner")
}
pub fn recovery_notes(s: &hive_store::Store) -> Result<Vec<RecoveryNote>> {
    meta_notes(s, "recovery")
}
pub fn landing_notes(s: &hive_store::Store) -> Result<Vec<LandingNote>> {
    meta_notes(s, "landing")
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct CapacityNote {
    queued: i64,
    slots: i64,
    inflight: i64,
    starved_since: String,
    held: String,
}

fn read_capacity_note(s: &hive_store::Store) -> Result<(CapacityNote, bool)> {
    let (raw, exists) = s.meta("capacity")?;
    if !exists {
        return Ok((CapacityNote::default(), false));
    }
    Ok((
        serde_json::from_str(&raw).map_err(|e| anyhow!("read capacity heartbeat: {e}"))?,
        true,
    ))
}

pub fn capacity_status(s: &hive_store::Store) -> Result<String> {
    let (note, exists) = read_capacity_note(s)?;
    if !exists || note.queued == 0 {
        return Ok(String::new());
    }
    let mut line = format!(
        "{} job(s) queued, {} slot(s), {} in flight",
        note.queued, note.slots, note.inflight
    );
    if !note.held.is_empty() {
        line.push_str("; ");
        line.push_str(&note.held);
    }
    if !note.starved_since.is_empty() {
        let start = parse_utc(&note.starved_since)?;
        let now = parse_utc(&s.now())?;
        line.push_str(&format!(
            "; every slot busy for {} min - another machine would help (hive pair; hive record show WF-executors)",
            (now - start).num_minutes()
        ));
    }
    Ok(line)
}

/// The disk hold the daemon wrote with its last capacity note, or empty.
pub fn capacity_disk(s: &hive_store::Store) -> Result<String> {
    let (raw, exists) = s.meta("capacity")?;
    if !exists || raw.is_empty() {
        return Ok(String::new());
    }
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
    Ok(v.get("disk")
        .and_then(|d| d.as_str())
        .unwrap_or("")
        .to_string())
}

/// The bulk-storage hold the daemon wrote with its last capacity note, or
/// empty: the volume carrying work, cache and worktrees is not mounted.
pub fn capacity_bulk(s: &hive_store::Store) -> Result<String> {
    let (raw, exists) = s.meta("capacity")?;
    if !exists || raw.is_empty() {
        return Ok(String::new());
    }
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
    Ok(v.get("bulk")
        .and_then(|d| d.as_str())
        .unwrap_or("")
        .to_string())
}

pub fn capacity_hold(s: &hive_store::Store) -> Result<String> {
    let (note, exists) = read_capacity_note(s)?;
    Ok(if exists { note.held } else { String::new() })
}

pub fn hygiene_status(s: &hive_store::Store) -> Result<String> {
    let (raw, exists) = s.meta("hygiene")?;
    if !exists {
        return Ok(String::new());
    }
    #[derive(Deserialize)]
    struct Note {
        at: String,
        removed: BTreeMap<String, i64>,
        held: BTreeMap<String, i64>,
        bytes: i64,
        errors: i64,
        #[serde(default)]
        cargo_target: Vec<CargoTargetNote>,
    }
    #[derive(Deserialize)]
    struct CargoTargetNote {
        units: i64,
        incremental: i64,
        #[serde(default)]
        objects: i64,
        bytes: i64,
        #[serde(default)]
        skipped: String,
    }
    let note: Note = serde_json::from_str(&raw).map_err(|e| anyhow!("read hygiene note: {e}"))?;
    let counts = |m: &BTreeMap<String, i64>| {
        m.iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut line = format!("last sweep {}: ", note.at);
    if note.removed.is_empty() {
        line.push_str("nothing to remove");
    } else {
        line.push_str("removed ");
        line.push_str(&counts(&note.removed));
        if note.bytes > 0 {
            line.push_str(&format!(" ({} freed)", size_text(note.bytes)));
        }
    }
    if !note.held.is_empty() {
        line.push_str("; held ");
        line.push_str(&counts(&note.held));
    }
    if note.errors > 0 {
        line.push_str(&format!(
            "; {} error(s) - hive sweep --dry-run shows them",
            note.errors
        ));
    }
    let cargo_units: i64 = note.cargo_target.iter().map(|x| x.units).sum();
    let cargo_incremental: i64 = note.cargo_target.iter().map(|x| x.incremental).sum();
    let cargo_objects: i64 = note.cargo_target.iter().map(|x| x.objects).sum();
    let cargo_bytes: i64 = note.cargo_target.iter().map(|x| x.bytes).sum();
    let cargo_skipped = note
        .cargo_target
        .iter()
        .filter(|x| !x.skipped.is_empty())
        .count();
    if cargo_bytes > 0 || cargo_units > 0 || cargo_incremental > 0 || cargo_objects > 0 {
        line.push_str(&format!(
            "; cargo target: freed {} ({} dead clone {}, {} stale incremental {}, {} stale debug {})",
            size_text(cargo_bytes),
            cargo_units,
            if cargo_units == 1 { "unit" } else { "units" },
            cargo_incremental,
            if cargo_incremental == 1 {
                "dir"
            } else {
                "dirs"
            },
            cargo_objects,
            if cargo_objects == 1 {
                "object"
            } else {
                "objects"
            },
        ));
    } else if !note.cargo_target.is_empty() && cargo_skipped == 0 {
        line.push_str("; cargo target: nothing to prune");
    } else if cargo_skipped > 0 {
        line.push_str(&format!(
            "; cargo target: skipped {cargo_skipped} busy or unsafe target(s)"
        ));
    }
    Ok(line)
}

pub fn size_text(n: i64) -> String {
    if n >= 1 << 30 {
        format!("{:.1} GB", n as f64 / (1 << 30) as f64)
    } else if n >= 1 << 20 {
        format!("{:.1} MB", n as f64 / (1 << 20) as f64)
    } else if n >= 1 << 10 {
        format!("{:.0} KB", n as f64 / (1 << 10) as f64)
    } else {
        format!("{n} B")
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct UnfitMark {
    tool: String,
    at: String,
}

fn unfit_marks(s: &hive_store::Store) -> Result<BTreeMap<String, BTreeMap<String, UnfitMark>>> {
    let (raw, exists) = s.meta("unfit")?;
    if !exists {
        return Ok(BTreeMap::new());
    }
    serde_json::from_str(&raw).map_err(|e| anyhow!("read executor unfit marks: {e}"))
}

pub fn unfit_phrase(tool: &str) -> String {
    match tool {
        "services" => "needs services".into(),
        "hands outdated" => tool.into(),
        _ if tool.starts_with("setup:") => format!("cannot set up {}", &tool[6..]),
        _ => format!("lacks {}", tool.strip_prefix("tool:").unwrap_or(tool)),
    }
}

/// The `executors` row: this machine and every paired one, with its state,
/// its busy slots and the projects it cannot run. A removed machine keeps
/// its row in the store, and every rented machine leaves one, so those are
/// counted at the end of the row instead of listed, and no note is worked
/// out for them.
pub fn executors_line(s: &hive_store::Store) -> Result<String> {
    let values = s.settings()?;
    let now = parse_utc(&s.now())?;
    let horizon = (now - Duration::minutes(2 * setting_int(&values, "minutes_per_job") + 15))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    #[derive(Clone)]
    struct Executor {
        name: String,
        status: String,
        workers: i64,
    }
    let mut executors = vec![Executor {
        name: "local".into(),
        status: "active".into(),
        workers: i64::from(hive_runner::capacity::local_slots(s)),
    }];
    let conn = s.conn()?;
    let mut stmt = conn.prepare(
        "SELECT name, status, workers FROM executors WHERE status != 'removed' ORDER BY name",
    )?;
    for row in stmt.query_map([], |r| {
        Ok(Executor {
            name: r.get(0)?,
            status: r.get(1)?,
            workers: r.get(2)?,
        })
    })? {
        executors.push(row?);
    }
    let marks = unfit_marks(s)?;
    let mut parts = Vec::new();
    for e in executors {
        let busy: i64 = conn.query_row(
                "SELECT COUNT(*) FROM runs WHERE status = 'running' AND executor = ? AND started_at > ?",
                (&e.name, &horizon),
                |r| r.get(0),
            )?;
        let mut part = format!("{} {} ({}/{} busy)", e.name, e.status, busy, e.workers);
        let mut unfit: BTreeMap<String, String> = BTreeMap::new();
        if let Some(mm) = marks.get(&e.name) {
            for (project, mark) in mm {
                if now - parse_utc(&mark.at)? < Duration::hours(24) {
                    unfit.insert(project.clone(), mark.tool.clone());
                }
            }
        }
        for (project, reason) in crate::recipes::recipe_blocks(s, &e.name)? {
            unfit.entry(project).or_insert(reason);
        }
        if !unfit.is_empty() {
            part.push_str("; ");
            part.push_str(
                &unfit
                    .iter()
                    .map(|(project, reason)| format!("{} for {}", unfit_phrase(reason), project))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
        parts.push(part);
    }
    let removed: i64 = conn.query_row(
        "SELECT COUNT(*) FROM executors WHERE status = 'removed'",
        [],
        |r| r.get(0),
    )?;
    if removed > 0 {
        parts.push(format!("{removed} removed"));
    }
    Ok(parts.join(", "))
}

pub fn status(s: &hive_store::Store) -> Result<StatusReport> {
    let (schema_version, exists) = s.meta("schema_version")?;
    if !exists {
        return Err(anyhow!("store has no schema_version"));
    }
    let mut r = StatusReport {
        store: s.path.clone(),
        schema_version,
        projects: s.projects(true)?,
        agents: status_counts(s, "SELECT status, COUNT(*) FROM agents GROUP BY status")?,
        jobs: status_counts(s, "SELECT status, COUNT(*) FROM jobs GROUP BY status")?,
        schedules: status_counts(s, "SELECT status, COUNT(*) FROM schedules GROUP BY status")?,
        ..Default::default()
    };
    let conn = s.conn()?;
    r.posts = conn.query_row("SELECT COUNT(*) FROM posts", [], |row| row.get(0))?;
    let today = parse_utc(&s.now())?.format("%Y-%m-%d").to_string();
    // What the daily cap counts (hive_store::DAILY_CAP_COUNT_SQL): the same
    // number the runner holds against jobs_per_day, so "N of M" here is the
    // runner's own arithmetic.
    r.runs_today = conn.query_row(hive_store::DAILY_CAP_COUNT_SQL, [&today], |row| row.get(0))?;
    r.cost_today = cost_by_kind(&conn, &today)?;
    {
        let mut stmt = conn.prepare(
            "SELECT outcome_source, COUNT(*) FROM runs \
             WHERE substr(finished_at, 1, 10) = ?1 AND status != 'running' AND outcome_source != '' \
             GROUP BY outcome_source ORDER BY outcome_source",
        )?;
        let rows = stmt.query_map([&today], |row| {
            Ok(StatusCount {
                status: row.get(0)?,
                n: row.get(1)?,
            })
        })?;
        r.outcomes_today = rows.collect::<rusqlite::Result<_>>()?;
    }
    r.settings = s.setting_rows()?;
    let mut values = BTreeMap::new();
    for setting in &mut r.settings {
        values.insert(setting.name.clone(), setting.value.clone());
        setting.value = agent_setting_value(&setting.name, &setting.value);
    }
    r.daily_cap = daily_cap(&values);
    r.runner = runner_stopped(s)?;
    if r.runner.is_empty() {
        r.runner = "running allowed".into();
    }
    r.harness = harness_line(s);
    r.daemon = daemon_status(s)?;
    if let Ok((beat, true)) = read_daemon_beat(s) {
        r.bus = beat.bus;
        r.planner_cache = beat.planner_cache;
    }
    r.executors = executors_line(s)?;
    r.capacity = capacity_status(s)?;
    r.slots = hive_runner::capacity::fleet(s).line();
    r.bulk = hive_runner::bulk::status_line(std::path::Path::new(&s.home));
    r.hygiene = hygiene_status(s)?;
    r.planner = planner_notes(s)?;
    r.recovery = recovery_notes(s)?;
    r.landing = landing_notes(s)?;
    r.waiting = hive_runner::owner_wait::status_lines(s);
    Ok(r)
}

impl StatusReport {
    pub fn text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "store      {} (schema {})\nprojects   {} active\n",
            self.store,
            self.schema_version,
            self.projects.len()
        ));
        for p in &self.projects {
            let brain = if p.brain { "  (brain)" } else { "" };
            out.push_str(&format!("  {:<18} {}{}\n", p.slug, p.path, brain));
        }
        out.push_str(&format!(
            "agents     {}\nposts      {}\n",
            join_counts(&self.agents),
            self.posts
        ));
        let jobs = {
            let j = join_counts(&self.jobs);
            if j.is_empty() { "none".into() } else { j }
        };
        out.push_str(&format!("jobs       {jobs}\n"));
        if !self.schedules.is_empty() {
            out.push_str(&format!("schedules  {}\n", join_counts(&self.schedules)));
        }
        out.push_str(&format!("runs today {}", self.runs_today));
        if self.daily_cap > 0 {
            out.push_str(&format!(" of {} (jobs_per_day)\n", self.daily_cap));
        } else {
            out.push_str(" (no daily cap; jobs_per_day=0)\n");
        }
        for (i, k) in self.cost_today.iter().enumerate() {
            let label = if i == 0 { "cost today" } else { "" };
            out.push_str(&format!("{label:<10} {}\n", kind_cost_text(k)));
        }
        if !self.outcomes_today.is_empty() {
            out.push_str(&format!(
                "outcomes   {} today (code = the worker's SUMMARY outcome code, text = regex fallback)\n",
                join_counts(&self.outcomes_today)
            ));
        }
        out.push_str(&format!(
            "runner     {}\nharness    {}\ndaemon     {}\n",
            self.runner, self.harness, self.daemon
        ));
        if self.bus.dropped > 0 {
            out.push_str(&format!(
                "events     {} event(s) dropped for slow subscribers since the daemon started\n",
                self.bus.dropped
            ));
        }
        if self.planner_cache.hits > 0 || self.planner_cache.misses > 0 {
            out.push_str(&format!(
                "planner    cache {} hit(s), {} miss(es) since the daemon started\n",
                self.planner_cache.hits, self.planner_cache.misses
            ));
        }
        out.push_str(&format!("executors  {}\n", self.executors));
        if !self.slots.is_empty() {
            out.push_str(&format!(
                "slots      {}\n",
                self.slots.trim_start_matches("slots ")
            ));
        }
        if !self.capacity.is_empty() {
            out.push_str(&format!("capacity   {}\n", self.capacity));
        }
        if !self.bulk.is_empty() {
            out.push_str(&format!("bulk       {}\n", self.bulk));
        }
        if !self.hygiene.is_empty() {
            out.push_str(&format!("hygiene    {}\n", self.hygiene));
        }
        for (label, notes) in [
            ("planner", &self.planner),
            ("recovery", &self.recovery),
            ("landing", &self.landing),
        ] {
            for note in notes {
                let stale = if note.stale { " (stale)" } else { "" };
                out.push_str(&format!(
                    "{:<10} {}: {}{}\n",
                    label, note.project, note.note, stale
                ));
            }
        }
        for line in &self.waiting {
            out.push_str(&format!("waiting    {line}\n"));
        }
        out.push_str("settings   ");
        out.push_str(
            &self
                .settings
                .iter()
                .map(|s| format!("{}={}", s.name, setting_text_value(&s.name, &s.value)))
                .collect::<Vec<_>>()
                .join(", "),
        );
        out.push('\n');
        out
    }
}

/// The value a setting shows on the one-line `settings` row of the text
/// status. A project goal is a paragraph (or several) stored as JSON; printed
/// inline it swamps the row, so the text view names its source and points at
/// `hive goal --project <slug>`. The JSON report keeps the full value.
fn setting_text_value(name: &str, value: &str) -> String {
    let Some(slug) = name.strip_prefix("project.goal.") else {
        return value.to_string();
    };
    let source = serde_json::from_str::<serde_json::Value>(value)
        .ok()
        .and_then(|v| v.get("source").and_then(|s| s.as_str()).map(str::to_string))
        .unwrap_or_else(|| "unknown".into());
    format!("<{source} goal; hive goal --project {slug}>")
}

pub fn status_text(s: &hive_store::Store) -> Result<String> {
    Ok(status(s)?.text())
}

pub fn run_status(c: &mut Context, _fs: &FlagSet, args: &[String]) -> Result<()> {
    no_args(args)?;
    let report = status(store(c)?)?;
    if c.json {
        return json_output(c, &report);
    }
    c.printf(format_args!("{}", report.text()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::*;

    /// A provider whose login check stands in for api.github.com: `slow`
    /// blocks like a request stuck inside its 120 s client timeout and never
    /// looks at the cancel token; otherwise it answers at once.
    struct StatusProbe {
        slow: bool,
    }

    impl hive_provider::Provider for StatusProbe {
        fn name(&self) -> &str {
            if self.slow {
                "inspectslow"
            } else {
                "inspectquick"
            }
        }
        fn label(&self) -> &str {
            "Probe"
        }
        fn chat(
            &self,
            _cancel: &hive_provider::CancelToken,
            _req: hive_provider::Request,
        ) -> Result<hive_provider::Response> {
            Ok(hive_provider::Response::default())
        }
        fn models(
            &self,
            _cancel: &hive_provider::CancelToken,
        ) -> Result<Vec<hive_provider::Model>> {
            Ok(vec![])
        }
        fn status(&self, _cancel: &hive_provider::CancelToken) -> Result<hive_provider::Login> {
            if self.slow {
                std::thread::sleep(std::time::Duration::from_secs(120));
            }
            Ok(hive_provider::Login {
                logged_in: true,
                user: "octocat".into(),
                ..hive_provider::Login::default()
            })
        }
    }

    fn use_status_probe(s: &hive_store::Store, name: &str) {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            hive_provider::register("inspectslow", "Probe", |_| {
                Ok(Box::new(StatusProbe { slow: true }))
            });
            hive_provider::register("inspectquick", "Probe", |_| {
                Ok(Box::new(StatusProbe { slow: false }))
            });
        });
        exec_sql(
            s,
            &format!(
                "INSERT INTO settings (name, value, updated_at, updated_by) VALUES ('provider', '{name}', '{TEST_NOW}', 'human') ON CONFLICT(name) DO UPDATE SET value = excluded.value"
            ),
        );
    }

    #[test]
    fn harness_line_gives_up_on_a_provider_that_does_not_answer() {
        let (_dir, s) = temporary_store();
        use_status_probe(&s, "inspectslow");
        let started = std::time::Instant::now();
        let line = harness_line_within(&s, std::time::Duration::from_millis(200));
        let waited = started.elapsed();
        assert_eq!(
            line,
            "native · Probe · status unknown (no answer within 0.200s; the provider may be slow or unreachable - hive harness status waits for it)"
        );
        // The provider blocks for 120 s, as a stuck request would; the line
        // comes back after the bound, not after the provider.
        assert!(
            waited < std::time::Duration::from_secs(10),
            "harness_line waited {waited:?}"
        );
    }

    #[test]
    fn harness_line_reports_a_prompt_answer_within_the_bound() {
        let (_dir, s) = temporary_store();
        use_status_probe(&s, "inspectquick");
        assert_eq!(
            harness_line_within(&s, HARNESS_STATUS_TIMEOUT),
            "native · Probe · signed in as octocat"
        );
    }

    #[test]
    fn one_status_line_cuts_on_char_boundaries() {
        assert_eq!(one_status_line("a \n b", 80), "a b");
        assert_eq!(one_status_line("abcdef", 3), "abc");
        // A multi-byte char straddling the byte limit is dropped, not split.
        let s = "ab—cd";
        assert_eq!(one_status_line(s, 3), "ab");
        assert_eq!(one_status_line(s, 4), "ab");
        assert_eq!(one_status_line(s, 5), "ab—");
        assert_eq!(one_status_line("عميل عميل", 3), "ع");
    }

    #[test]
    fn daemon_interval_fallback() {
        for (name, raw, want) in [
            (
                "no-heartbeat",
                "",
                hive_runner::DEFAULT_DAEMON_INTERVAL_SECONDS as i64,
            ),
            (
                "unreadable",
                "not json",
                hive_runner::DEFAULT_DAEMON_INTERVAL_SECONDS as i64,
            ),
            (
                "missing-interval",
                "{}",
                hive_runner::DEFAULT_DAEMON_INTERVAL_SECONDS as i64,
            ),
            (
                "null-interval",
                r#"{"interval":null}"#,
                hive_runner::DEFAULT_DAEMON_INTERVAL_SECONDS as i64,
            ),
            (
                "zero-interval",
                r#"{"interval":0}"#,
                hive_runner::DEFAULT_DAEMON_INTERVAL_SECONDS as i64,
            ),
            ("custom-interval", r#"{"interval":7}"#, 7),
        ] {
            let (_dir, s) = temporary_store();
            if !raw.is_empty() {
                s.conn()
                    .unwrap()
                    .execute("INSERT INTO meta (key, value) VALUES ('daemon', ?)", [raw])
                    .unwrap();
            }
            assert_eq!(daemon_interval_seconds(&s).unwrap(), want, "{name}");
        }
    }

    #[test]
    fn daemon_heartbeat_default_interval() {
        let (_dir, s) = temporary_store();
        s.conn()
            .unwrap()
            .execute(
                "INSERT INTO meta(key,value) VALUES('daemon', '{\"at\":\"2026-09-06T18:48:59Z\"}')",
                [],
            )
            .unwrap();
        assert!(daemon_status(&s).unwrap().starts_with("silent "));
    }

    #[test]
    fn status_runner_counts_and_capacity() {
        let (_dir, s) = temporary_store();
        exec_sql(
            &s,
            "INSERT INTO projects(slug,name,path,status,brain,added_at) VALUES ('active','Active','/missing-active','active',0,'2026-09-06T19:00:00Z'),('inactive','Inactive','/missing-inactive','inactive',0,'2026-09-06T19:00:00Z')",
        );
        // Each run its own job but the two of `again`: the cap counts
        // distinct jobs (a held run carried on later is the same work).
        for (id, job, mode, status, at) in [
            ("today", "job-today", "real", "done", TEST_NOW),
            ("dry", "job-dry", "dry-run", "done", TEST_NOW),
            ("error", "job-error", "real", "error", TEST_NOW),
            (
                "yesterday",
                "job-yesterday",
                "real",
                "done",
                "2026-09-05T23:59:59Z",
            ),
            (
                "running",
                "job-running",
                "real",
                "running",
                "2026-09-06T18:50:00Z",
            ),
            (
                "stale-running",
                "job-stale",
                "real",
                "running",
                "2026-09-06T01:00:00Z",
            ),
            ("held", "job-today", "real", "held", "2026-09-06T00:30:00Z"),
            ("again", "job-today", "real", "done", "2026-09-06T00:40:00Z"),
            ("lost", "job-lost", "real", "lost", "2026-09-06T00:50:00Z"),
        ] {
            s.conn()
                .unwrap()
                .execute(
                    "INSERT INTO runs(id,job_id,agent,project,started_at,status,mode) VALUES (?,?,'worker','active',?,?,?)",
                    (id, job, at, status, mode),
                )
                .unwrap();
        }
        exec_sql(
            &s,
            "INSERT INTO meta(key,value) VALUES ('capacity','{\"queued\":3,\"slots\":2,\"inflight\":2,\"starved_since\":\"2026-09-06T18:29:01Z\"}'),('daemon','{\"pid\":123,\"at\":\"2026-09-06T18:59:00Z\",\"loops\":4,\"interval\":30,\"note\":\"\"}')",
        );
        s.set_setting("human", "capacity", "1").unwrap();
        let report = status(&s).unwrap();
        assert_eq!(report.slots, "slots 1 = this machine 1, per project 1");
        assert_eq!(report.projects.len(), 1);
        assert_eq!(report.projects[0].slug, "active");
        assert_eq!(report.runs_today, 3);
        assert_eq!(report.daily_cap, 2);
        assert_eq!(report.executors, "local active (1/1 busy)");
        assert_eq!(
            report.daemon,
            "alive pid 123, last loop 60s ago, 4 loop(s), interval 30s"
        );
        assert_eq!(
            report.capacity,
            "3 job(s) queued, 2 slot(s), 2 in flight; every slot busy for 30 min - another machine would help (hive pair; hive record show WF-executors)"
        );
        exec_sql(
            &s,
            "UPDATE meta SET value = '{\"queued\":3,\"slots\":0,\"inflight\":0,\"starved_since\":\"\",\"held\":\"workers on this machine wait for a GitHub Copilot login: sign in with GitHub (Settings -> Harness)\"}' WHERE key = 'capacity'",
        );
        assert_eq!(
            capacity_status(&s).unwrap(),
            "3 job(s) queued, 0 slot(s), 0 in flight; workers on this machine wait for a GitHub Copilot login: sign in with GitHub (Settings -> Harness)"
        );
        assert_eq!(
            capacity_hold(&s).unwrap(),
            "workers on this machine wait for a GitHub Copilot login: sign in with GitHub (Settings -> Harness)"
        );
        exec_sql(
            &s,
            "UPDATE meta SET value = '{\"queued\":3,\"slots\":2,\"inflight\":2,\"starved_since\":\"2026-09-06T18:29:01Z\"}' WHERE key = 'capacity'",
        );
        assert_eq!(capacity_hold(&s).unwrap(), "");
        assert_eq!(capacity_disk(&s).unwrap(), "");
        exec_sql(
            &s,
            "UPDATE meta SET value = '{\"queued\":3,\"slots\":2,\"inflight\":0,\"starved_since\":\"\",\"held\":\"\",\"disk\":\"disk: 252 MB free, needs 1024; 6 projects waiting\"}' WHERE key = 'capacity'",
        );
        assert_eq!(
            capacity_disk(&s).unwrap(),
            "disk: 252 MB free, needs 1024; 6 projects waiting"
        );
        // Back to the starved note so the CLI check below still sees it.
        exec_sql(
            &s,
            "UPDATE meta SET value = '{\"queued\":3,\"slots\":2,\"inflight\":2,\"starved_since\":\"2026-09-06T18:29:01Z\"}' WHERE key = 'capacity'",
        );
        assert_eq!(capacity_disk(&s).unwrap(), "");
        // temporary_store() hides this machine's GitHub logins, so the
        // harness line is decided without the network and is the same on
        // every machine; a probe that reaches api.github.com again fails
        // here instead of flaking snapshot_json_stable.
        assert_eq!(
            report.harness,
            "native · GitHub Copilot · not signed in - Sign in with GitHub (Settings -> Harness) or run hive harness login"
        );
        assert!(
            report
                .text()
                .contains(&format!("\nharness    {}\n", report.harness))
        );
        let as_json = invoke(Some(s.clone()), true, "", "status", &[]);
        let decoded: StatusReport = serde_json::from_str(&as_json.out).unwrap();
        assert_eq!(decoded.text(), report.text(), "{as_json:?}");
    }

    #[test]
    fn daemon_heartbeat_states() {
        let (_dir, s) = temporary_store();
        for (raw, want) in [
            ("not json", "unreadable heartbeat"),
            (
                r#"{"pid":42,"at":"2026-09-06T18:00:00Z","note":"exited"}"#,
                "exited 2026-09-06T18:00:00Z (pid 42)",
            ),
            (
                r#"{"at":"2026-09-06T19:00:00Z"}"#,
                "alive pid None, last loop 0s ago, 0 loop(s), interval Nones",
            ),
            (
                r#"{"pid":7,"at":"2026-09-06T19:00:00Z","loops":9,"interval":300,"tick":{"ms":1200,"max_ms":41000,"slow":2}}"#,
                "alive pid 7, last loop 0s ago, 9 loop(s), interval 300s, last tick 1200ms (slowest 41000ms, 2 over the cadence)",
            ),
            (
                r#"{"pid":7,"at":"2026-09-06T19:00:00Z","loops":9,"interval":300,"tick":{"ms":80,"max_ms":950,"slow":0}}"#,
                "alive pid 7, last loop 0s ago, 9 loop(s), interval 300s, last tick 80ms (slowest 950ms)",
            ),
            (
                r#"{"pid":7,"at":"2026-09-06T19:00:00Z","loops":9,"interval":300,"restart":{"since":"2026-09-06T12:35:00Z","inflight":2,"dispatching":true,"until":"2026-09-06T14:05:00Z","waits_for":[{"agent":"fjord","kind":"task","project":"brain","job":"j1","started_at":"2026-09-06T18:20:00Z"}]}}"#,
                "alive pid 7, last loop 0s ago, 9 loop(s), interval 300s, restart since 2026-09-06T12:35:00Z (6h25m0s), waits for fjord's task on brain; nothing new starts (daemon.restart_max_minutes is up)",
            ),
            (
                r#"{"pid":7,"at":"2026-09-06T19:00:00Z","loops":9,"interval":300,"restart":{"since":"2026-09-06T18:30:00Z","inflight":1,"dispatching":true,"until":"2026-09-06T20:00:00Z","waits_for":[]}}"#,
                "alive pid 7, last loop 0s ago, 9 loop(s), interval 300s, restart since 2026-09-06T18:30:00Z (30m0s), waits for 1 run(s) in flight; nothing new starts after 2026-09-06T20:00:00Z (daemon.restart_max_minutes)",
            ),
            (
                r#"{"pid":7,"at":"2026-09-06T19:00:00Z","loops":9,"interval":300,"restart":{"since":"2026-09-06T18:30:00Z","inflight":4,"dispatching":false,"waits_for":[{"agent":"a","kind":"task","project":"p","job":"1","started_at":""},{"agent":"b","kind":"review","project":"p","job":"2","started_at":""},{"agent":"c","kind":"lead","project":"q","job":"3","started_at":""},{"agent":"d","kind":"task","project":"q","job":"4","started_at":""}]}}"#,
                "alive pid 7, last loop 0s ago, 9 loop(s), interval 300s, restart since 2026-09-06T18:30:00Z (30m0s), waits for a's task on p, b's review on p, c's lead on q and 1 more; nothing new starts",
            ),
            (
                r#"{"pid":7,"at":"2026-09-06T19:00:00Z","loops":9,"interval":300,"tick":{"ms":80,"max_ms":31000,"slow":7,"longest":[{"stage":"plan","ms":14000},{"stage":"pipeline","ms":9000}]}}"#,
                "alive pid 7, last loop 0s ago, 9 loop(s), interval 300s, last tick 80ms (slowest 31000ms: plan 14000ms, pipeline 9000ms, 7 over the cadence)",
            ),
        ] {
            s.conn().unwrap().execute("INSERT INTO meta(key,value) VALUES ('daemon',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [raw]).unwrap();
            assert_eq!(daemon_status(&s).unwrap(), want);
        }
    }

    #[test]
    fn status_shows_cost_per_job_kind_and_how_runs_were_classified() {
        let (_dir, s) = temporary_store();
        let before = status(&s).unwrap();
        assert!(before.cost_today.is_empty());
        assert!(!before.text().contains("cost today"), "{}", before.text());
        exec_sql(
            &s,
            "INSERT INTO jobs(id,project,kind,title,created_at) VALUES \
             ('j-task','p','task','t','2026-09-06T01:00:00Z'), \
             ('j-lead','p','lead','l','2026-09-06T01:00:00Z'), \
             ('j-old','p','task','o','2026-09-05T01:00:00Z')",
        );
        exec_sql(
            &s,
            "INSERT INTO runs(id,job_id,agent,project,started_at,finished_at,status,prompt_tokens,completion_tokens,cost,outcome,outcome_source) VALUES \
             ('r1','j-task','a','p','2026-09-06T02:00:00Z','2026-09-06T02:30:00Z','done',120000,4000,1.5,'done','code'), \
             ('r2','j-task','a','p','2026-09-06T03:00:00Z','2026-09-06T03:30:00Z','failed',30000,1000,0.5,'done','text'), \
             ('r3','j-lead','a','p','2026-09-06T04:00:00Z','2026-09-06T04:10:00Z','done',900,50,0,'','none'), \
             ('r4','j-old','a','p','2026-09-05T04:00:00Z','2026-09-05T04:10:00Z','done',999999,9,9,'done','code'), \
             ('r5','j-task','a','p','2026-09-06T18:00:00Z','','running',5,5,5,'','')",
        );
        let report = status(&s).unwrap();
        assert_eq!(
            report.cost_today,
            vec![
                KindCost {
                    kind: "task".into(),
                    runs: 2,
                    prompt_tokens: 150_000,
                    completion_tokens: 5_000,
                    cost: 2.0,
                },
                KindCost {
                    kind: "lead".into(),
                    runs: 1,
                    prompt_tokens: 900,
                    completion_tokens: 50,
                    cost: 0.0,
                },
            ]
        );
        let text = report.text();
        assert!(
            text.contains(
                "cost today task 2 runs, 150.0k prompt + 5.0k completion tokens, cost 2.00\n           lead 1 run, 900 prompt + 50 completion tokens\n"
            ),
            "{text}"
        );
        assert!(
            text.contains("outcomes   1 code, 1 none, 1 text today (code = the worker's SUMMARY outcome code, text = regex fallback)\n"),
            "{text}"
        );
        // The JSON report carries both and reads back the same.
        let json = serde_json::to_string(&report).unwrap();
        let decoded: StatusReport = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.cost_today, report.cost_today);
        assert_eq!(decoded.outcomes_today, report.outcomes_today);
    }

    #[test]
    fn status_prints_dropped_bus_events() {
        let (_dir, s) = temporary_store();
        exec_sql(
            &s,
            "INSERT INTO meta(key,value) VALUES ('daemon','{\"pid\":123,\"at\":\"2026-09-06T18:59:00Z\",\"loops\":4,\"interval\":30,\"note\":\"\",\"bus\":{\"subscribers\":2,\"dropped\":7,\"deduped\":3}}')",
        );
        let report = status(&s).unwrap();
        assert_eq!(report.bus.subscribers, 2);
        assert_eq!(report.bus.dropped, 7);
        assert_eq!(report.bus.deduped, 3);
        assert!(
            report.text().contains(
                "events     7 event(s) dropped for slow subscribers since the daemon started\n"
            ),
            "dropped line missing:\n{}",
            report.text()
        );
    }

    #[test]
    fn planner_notes_text_and_stale() {
        let (_dir, s) = temporary_store();
        exec_sql(
            &s,
            "INSERT INTO meta(key,value) VALUES ('daemon','{\"pid\":123,\"at\":\"2026-09-06T18:59:00Z\",\"loops\":4,\"interval\":30,\"note\":\"\"}'), ('planner','{\"fresh\":{\"at\":\"2026-09-06T18:59:31Z\",\"note\":\"no suggestions; reflect due\"},\"old\":{\"at\":\"2026-09-06T18:58:59Z\",\"note\":\"no eligible suggestion (1 rejected)\"}}')",
        );
        let notes = planner_notes(&s).unwrap();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].project, "fresh");
        assert!(!notes[0].stale);
        assert_eq!(notes[1].project, "old");
        assert!(notes[1].stale);
        let text = status(&s).unwrap().text();
        assert!(
            text.contains("planner    fresh: no suggestions; reflect due\n"),
            "fresh planner line missing:\n{text}"
        );
        assert!(
            text.contains("planner    old: no eligible suggestion (1 rejected) (stale)\n"),
            "stale planner line missing:\n{text}"
        );
    }

    #[test]
    fn hygiene_status_line() {
        let (_dir, s) = temporary_store();
        let mut report = status(&s).unwrap();
        assert_eq!(report.hygiene, "");
        assert!(!report.text().contains("hygiene"));
        exec_sql(
            &s,
            r#"INSERT INTO meta(key,value) VALUES ('hygiene','{"at":"2026-09-06T18:59:00Z","removed":{"clone":2,"scratch":5},"held":{"branch":20,"clone":8},"bytes":1503238553,"errors":1}')"#,
        );
        report = status(&s).unwrap();
        let want = "last sweep 2026-09-06T18:59:00Z: removed clone 2, scratch 5 (1.4 GB freed); held branch 20, clone 8; 1 error(s) - hive sweep --dry-run shows them";
        assert_eq!(report.hygiene, want, "hygiene = want");
        assert!(
            report.text().contains(&format!("hygiene    {want}\n")),
            "hygiene line missing:\n{}",
            report.text()
        );
        exec_sql(
            &s,
            r#"UPDATE meta SET value = '{"at":"2026-09-06T19:09:00Z","removed":{},"held":{},"bytes":0,"errors":0}' WHERE key = 'hygiene'"#,
        );
        assert_eq!(
            hygiene_status(&s).unwrap(),
            "last sweep 2026-09-06T19:09:00Z: nothing to remove"
        );
    }

    #[test]
    fn unfit_marks_are_read_only() {
        let (_dir, s) = temporary_store();
        let raw = r#"{"local":{"fresh":{"tool":"tool:node@22","at":"2026-09-05T19:00:01Z"},"expired":{"tool":"uv","at":"2026-09-05T19:00:00Z"},"services":{"tool":"services","at":"2026-09-06T18:00:00Z"},"setup":{"tool":"setup:Gemfile.lock","at":"2026-09-06T18:30:00Z"}}}"#;
        s.conn()
            .unwrap()
            .execute("INSERT INTO meta(key,value) VALUES ('unfit',?)", [raw])
            .unwrap();
        s.set_setting("human", "capacity", "1").unwrap();
        let before = event_count(&s);
        assert_eq!(
            executors_line(&s).unwrap(),
            "local active (0/1 busy); lacks node@22 for fresh, needs services for services, cannot set up Gemfile.lock for setup"
        );
        assert_eq!(event_count(&s), before);
    }

    /// The store keeps the row of every machine it ever had. A removed one
    /// takes no work: the line counts it, and works out no note for it.
    #[test]
    fn executors_line_counts_removed_executors_instead_of_listing_them() {
        let (_dir, s) = temporary_store();
        s.set_setting("human", "capacity", "1").unwrap();
        exec_sql(
            &s,
            "INSERT INTO projects(slug,name,path,added_at) VALUES ('pinned','Pinned','/does-not-exist','2026-09-06T19:00:00Z')",
        );
        // A recipe that pins a tool: hands that never answered a probe with
        // `recipe=1` read as outdated for the project.
        let record = hive_store::Record {
            id: "CTX-environment".into(),
            project: "pinned".into(),
            kind: "context".into(),
            slug: "environment".into(),
            title: "Environment".into(),
            created_at: TEST_NOW.into(),
            updated_at: TEST_NOW.into(),
            origin: "store".into(),
            ..Default::default()
        };
        let mut data = std::collections::BTreeMap::new();
        data.insert("recipe".to_string(), serde_json::json!("1"));
        data.insert("tools".to_string(), serde_json::json!(["node@22"]));
        s.put_record(record, data, "The environment.", "human", "")
            .unwrap();
        exec_sql(
            &s,
            "INSERT INTO executors(name,status,workers,created_at) VALUES \
             ('hands-drained','drained',1,'2026-09-01T10:00:00Z'), \
             ('hands-gone-1','removed',1,'2026-09-01T10:00:00Z'), \
             ('hands-gone-2','removed',1,'2026-09-02T10:00:00Z'), \
             ('hands-kept','active',2,'2026-09-03T10:00:00Z')",
        );
        // A mark a failed setup left for a machine that is gone since.
        exec_sql(
            &s,
            r#"INSERT INTO meta(key,value) VALUES ('unfit','{"hands-gone-1":{"pinned":{"tool":"cargo","at":"2026-09-06T18:30:00Z"}}}')"#,
        );
        assert_eq!(
            executors_line(&s).unwrap(),
            "local active (0/1 busy), hands-drained drained (0/1 busy); hands outdated for pinned, hands-kept active (0/2 busy); hands outdated for pinned, 2 removed"
        );
        let report = status(&s).unwrap();
        assert!(
            report.text().contains(
                "\nexecutors  local active (0/1 busy), hands-drained drained (0/1 busy); hands outdated for pinned, hands-kept active (0/2 busy); hands outdated for pinned, 2 removed\n"
            ),
            "{}",
            report.text()
        );
        // Without a removed machine the line says nothing about them.
        exec_sql(&s, "DELETE FROM executors WHERE status = 'removed'");
        assert_eq!(
            executors_line(&s).unwrap(),
            "local active (0/1 busy), hands-drained drained (0/1 busy); hands outdated for pinned, hands-kept active (0/2 busy); hands outdated for pinned"
        );
    }

    #[test]
    fn daily_cap_spellings() {
        let (_dir, s) = temporary_store();
        for value in [
            "none",
            "off",
            "unlimited",
            "0",
            "3",
            "invalid-old-value",
            "-2",
        ] {
            s.conn()
                .unwrap()
                .execute(
                    "UPDATE settings SET value=? WHERE name='jobs_per_day'",
                    [value],
                )
                .unwrap();
            let report = status(&s).unwrap();
            if value == "3" {
                assert_eq!(report.daily_cap, 3);
            }
            if value != "3" {
                assert!(report.daily_cap >= 0);
            }
        }
    }

    #[test]
    fn recovery_notes_missing_key_is_empty_slice() {
        let (_dir, s) = temporary_store();
        let notes = recovery_notes(&s).unwrap();
        assert!(notes.is_empty());
    }

    #[test]
    fn recovery_notes_sorted_stale_and_malformed() {
        let (_dir, s) = temporary_store();
        exec_sql(
            &s,
            "INSERT INTO meta(key,value) VALUES ('daemon','{\"pid\":123,\"at\":\"2026-09-06T18:59:00Z\",\"loops\":4,\"interval\":30,\"note\":\"\"}'), ('recovery','{\"zeta\":{\"at\":\"2026-09-06T18:59:31Z\",\"note\":\"next retry at 2026-09-06T19:05:00Z\"},\"alpha\":{\"at\":\"2026-09-06T18:58:59Z\",\"note\":\"held (structural: missing npm)\"}}')",
        );
        let notes = recovery_notes(&s).unwrap();
        assert_eq!(notes[0].project, "alpha");
        assert_eq!(notes[0].note, "held (structural: missing npm)");
        assert!(notes[0].stale);
        assert_eq!(notes[1].project, "zeta");
        assert!(!notes[1].stale);
        s.conn()
            .unwrap()
            .execute("INSERT INTO meta(key,value) VALUES ('recovery','not json') ON CONFLICT(key) DO UPDATE SET value=excluded.value", [])
            .unwrap();
        assert!(
            recovery_notes(&s)
                .unwrap_err()
                .to_string()
                .contains("recovery notes")
        );
    }

    #[test]
    fn status_recovery_text_and_json() {
        let (_dir, s) = temporary_store();
        exec_sql(
            &s,
            "INSERT INTO meta(key,value) VALUES ('daemon',''), ('planner','{\"proj\":{\"at\":\"2026-09-06T18:59:31Z\",\"note\":\"planner note\"}}'),('recovery','{\"proj\":{\"at\":\"2026-09-06T18:59:31Z\",\"note\":\"3 failed in 7d: 1 run again, 1 re-planned (job-r), 1 held\"}}')",
        );
        let report = status(&s).unwrap();
        assert_eq!(report.recovery.len(), 1);
        let text = report.text();
        let planner = "planner    proj: planner note\n";
        let recovery =
            "recovery   proj: 3 failed in 7d: 1 run again, 1 re-planned (job-r), 1 held\n";
        let planner_at = text.find(planner);
        let recovery_at = text.find(recovery);
        assert!(
            planner_at.is_some() && recovery_at.is_some(),
            "planner/recovery lines missing:\n{text}"
        );
        assert!(
            recovery_at > planner_at,
            "recovery line should follow planner line:\n{text}"
        );
        assert!(
            serde_json::to_string(&report)
                .unwrap()
                .contains("\"recovery\":[")
        );
        let (_empty_dir, empty) = temporary_store();
        let empty_report = status(&empty).unwrap();
        assert!(empty_report.recovery.is_empty());
        assert!(
            serde_json::to_string(&empty_report)
                .unwrap()
                .contains("\"recovery\":[]")
        );
    }

    #[test]
    fn status_text_elides_project_goals() {
        let (_dir, s) = temporary_store();
        let goal = "{\"text\":\"Make zeta useful.\\n\\nSecond paragraph.\",\"source\":\"owner\"}";
        exec_sql(
            &s,
            &format!(
                "INSERT INTO settings(name,value,updated_at,updated_by) VALUES ('project.goal.zeta','{}','2026-09-06T18:59:00Z','owner')",
                goal.replace('\'', "''")
            ),
        );
        let report = status(&s).unwrap();
        let text = report.text();
        assert!(
            text.contains("project.goal.zeta=<owner goal; hive goal --project zeta>"),
            "goal should be elided on the settings row:\n{text}"
        );
        assert!(!text.contains("Second paragraph"), "{text}");
        let json = serde_json::to_string(&report).unwrap();
        assert!(
            json.contains("Second paragraph"),
            "json keeps the full goal:\n{json}"
        );
        assert_eq!(
            setting_text_value("project.goal.q", "not json"),
            "<unknown goal; hive goal --project q>"
        );
        assert_eq!(setting_text_value("quorum", "2"), "2");
    }

    #[test]
    fn status_lists_the_owners_open_waits_with_their_holds() {
        let (_dir, s) = temporary_store();
        exec_sql(
            &s,
            "INSERT INTO projects (slug, name, path, added_at) VALUES ('proj', 'Proj', '/w/proj', '2026-09-06T00:00:00Z')",
        );
        exec_sql(
            &s,
            "INSERT INTO jobs (id, project, kind, title, source, status, created_at) VALUES ('job-owner', 'proj', 'task', 'Owner step', 'task:TASK-055', 'queued', '2026-09-06T04:40:00Z')",
        );
        let before = status(&s).unwrap();
        assert!(before.waiting.is_empty());
        assert!(!before.text().contains("waiting    "));
        assert!(
            !serde_json::to_string(&before)
                .unwrap()
                .contains("\"waiting\"")
        );
        exec_sql(
            &s,
            "INSERT INTO events (at, actor, action, subject, detail) VALUES ('2026-09-06T04:51:00Z', 'daemon', 'job.owner-wait', 'job-owner', '{\"since\":\"2026-09-06T04:40:00Z\",\"waited_s\":660,\"reasons\":[\"lanes full\",\"load 27 > 24\"]}')",
        );
        let report = status(&s).unwrap();
        let text = report.text();
        assert!(
            text.contains(
                "waiting    proj job-owner (Owner step) waiting since 04:40 UTC: lanes full, load 27 > 24\n"
            ),
            "{text}"
        );
        assert_eq!(report.waiting.len(), 1);
    }

    #[test]
    fn status_landing_hold_text_and_json() {
        let (_dir, s) = temporary_store();
        exec_sql(
            &s,
            "INSERT INTO meta(key,value) VALUES ('daemon','{\"pid\":123,\"at\":\"2026-09-06T18:59:00Z\",\"loops\":4,\"interval\":30,\"note\":\"\"}'),('recovery','{\"proj\":{\"at\":\"2026-09-06T18:59:31Z\",\"note\":\"1 held\"}}'),('landing','{\"zeta\":{\"at\":\"2026-09-06T18:59:31Z\",\"note\":\"2 branches waiting; deferred: the checkout of /w/zeta has uncommitted changes\"},\"alpha\":{\"at\":\"2026-09-06T18:58:59Z\",\"note\":\"1 branch waiting; locked: another landing holds /w/alpha/.git/hive-land.lock\"}}')",
        );
        let report = status(&s).unwrap();
        assert_eq!(report.landing.len(), 2);
        assert_eq!(report.landing[0].project, "alpha");
        assert!(report.landing[0].stale);
        assert_eq!(report.landing[1].project, "zeta");
        assert!(!report.landing[1].stale);
        let text = report.text();
        let recovery = "recovery   proj: 1 held\n";
        let landing = "landing    zeta: 2 branches waiting; deferred: the checkout of /w/zeta has uncommitted changes\n";
        let recovery_at = text.find(recovery);
        let landing_at = text.find(landing);
        assert!(
            recovery_at.is_some() && landing_at.is_some() && landing_at > recovery_at,
            "recovery then landing lines expected:\n{text}"
        );
        assert!(text.contains("landing    alpha: 1 branch waiting; locked: another landing holds /w/alpha/.git/hive-land.lock (stale)\n"), "stale landing line:\n{text}");
        exec_sql(
            &s,
            "INSERT INTO meta(key,value) VALUES ('landing','not json') ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        );
        assert!(
            landing_notes(&s)
                .unwrap_err()
                .to_string()
                .contains("landing notes")
        );
        let (_empty_dir, empty) = temporary_store();
        let empty_report = status(&empty).unwrap();
        assert!(empty_report.landing.is_empty());
        assert!(!empty_report.text().contains("landing    "));
        assert!(
            empty_report.landing.is_empty(),
            "empty status landing: {:?}",
            empty_report.landing
        );
        assert!(
            serde_json::to_string(&empty_report)
                .unwrap()
                .contains("\"landing\":[]")
        );
    }
}
