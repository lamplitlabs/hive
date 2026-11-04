use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, LazyLock, RwLock};
use std::time::Duration;

type Row = crate::util::Row;

pub const DEFAULT_DAEMON_INTERVAL_SECONDS: i32 = 300;
pub(crate) const DISPATCH_TICK_SECONDS: i32 = 15;
pub(crate) const HANDS_WANTED_MINUTES: i32 = 30;
/// Minutes a job whose needs no active machine meets waits before the
/// owner is asked for such a machine.
pub(crate) const NEEDS_WANTED_MINUTES: i32 = 10;
pub(crate) const DISK_FLOOR_WANTED_MINUTES: i32 = 15;
/// A disk that hovers around the floor clears and re-crosses it many times
/// an hour; each crossing used to ask the owner again, and every post became
/// another "answer the disk question" lead job. One post per this window
/// is enough: the condition and the remedy have not changed.
pub(crate) const DISK_FLOOR_REPOST_MINUTES: i32 = 6 * 60;
pub(crate) static BINARY_STAMP: LazyLock<RwLock<fn() -> String>> =
    LazyLock::new(|| RwLock::new(pulse_cli::binary_stamp));
/// When this daemon's loop began, ISO; the heartbeat carries it so the
/// console can tell an ask for a restart this daemon has yet to act on from
/// the one that restarted the daemon before it.
static DAEMON_STARTED_AT: LazyLock<RwLock<String>> = LazyLock::new(|| RwLock::new(String::new()));
#[cfg(test)]
pub(crate) fn set_binary_stamp(f: fn() -> String) {
    *BINARY_STAMP.write().expect("binary stamp lock") = f;
}
/// daemonWake is what wakes the loop before its interval is over: the owner wrote (message.sent, task.*), a
/// compute request was approved, a setting changed (the stop switch), a job was queued by a lead or a CLI.
pub(crate) static DAEMON_WAKE: &[&str] = &[
    "message.sent",
    "post.question",
    "post.handoff",
    "post.vote",
    "post.result",
    "task.created",
    "task.amended",
    "task.status",
    "compute.approved",
    "setting",
    "job.add",
    "job.queued",
    "job.requeue",
    "job.recover",
    "schedule.run",
    RESTART_ACTION,
];

/// The owner's own writes among [`DAEMON_WAKE`]: a task filed, amended or
/// answered, a message sent. These do not wait out `daemon.pass_seconds`:
/// the tick they wake runs the planning pass, so the lead takes the work
/// up at once - the way a chat message starts its agent at once - instead
/// of up to a minute later. The event's actor must be the owner
/// ([`pulse_store::HUMAN_AGENT`]): an agent's own task or message is
/// ordinary work for the next pass.
pub(crate) static OWNER_WAKE: &[&str] = &[
    "message.sent",
    "task.created",
    "task.amended",
    "task.status",
];

/// Whether the owner wrote (a task, a message) after the event `after`.
/// Read once per tick, so a wake that came through the bus is seen even
/// when the bridge dropped it (`sleep` seams) or the owner wrote while the
/// previous tick ran.
pub(crate) fn owner_wrote_after(s: &pulse_store::Store, after: i64) -> bool {
    let filter = pulse_bus::Filter {
        actions: OWNER_WAKE.iter().map(|a| a.to_string()).collect(),
        ..Default::default()
    };
    pulse_bus::since(s, after, filter, 0)
        .map(|events| events.iter().any(|e| e.actor == pulse_store::HUMAN_AGENT))
        .unwrap_or(false)
}

/// The event the owner writes (Settings, Running, "Restart") to have the
/// daemon restart. By default it drains like a self-update: new runs stop
/// starting and the daemon exits with code 3 once the runs in flight have
/// ended or `daemon.restart_drain_minutes` is up, and the restart waits for
/// a landing after them the same `RESTART_DRAIN`. The hub asks first and
/// adds `"now": true` only when the operator confirms the fast path: each
/// run in flight is asked to hold - the harness saves its conversation and
/// exits `held`, the job returns to the queue with its clone, and the job's
/// next run carries on from there (run.rs) - and the daemon exits once they
/// have held, or after `daemon.hold_seconds`, when the runs still going are
/// stopped and their jobs start over.
pub const RESTART_ACTION: &str = "daemon.restart";

/// Writes the owner's ask; the daemon wakes on it within a second.
pub fn ask_restart(
    s: &pulse_store::Store,
    actor: &str,
    detail: serde_json::Value,
) -> anyhow::Result<()> {
    pulse_bus::emit(s, actor, RESTART_ACTION, "daemon", detail)?;
    Ok(())
}

/// An ask for a restart written after event `after`: the daemon reads only
/// the asks of its own lifetime, so the ask that restarted the daemon before
/// it does not restart it too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RestartAsk {
    pub(crate) seq: i64,
    pub(crate) at: String,
    pub(crate) actor: String,
    // `now: true` on the ask: the operator confirmed the fast path (hold the
    // runs in flight) after seeing how many there are; absent or false drains.
    pub(crate) force: bool,
}

pub(crate) fn restart_asked(s: &pulse_store::Store, after: i64) -> Option<RestartAsk> {
    let conn = s.conn().ok()?;
    conn.query_row(
        "SELECT seq, at, actor, detail FROM events WHERE action = ? AND seq > ? ORDER BY seq DESC LIMIT 1",
        rusqlite::params![RESTART_ACTION, after],
        |r| {
            let detail: String = r.get(3)?;
            let force = serde_json::from_str::<serde_json::Value>(&detail)
                .ok()
                .and_then(|d| d.get("now").and_then(serde_json::Value::as_bool))
                .unwrap_or(false);
            Ok(RestartAsk {
                seq: r.get(0)?,
                at: r.get(1)?,
                actor: r.get(2)?,
                force,
            })
        },
    )
    .ok()
}

/// How long a daemon asked to restart now waits for its runs to hold.
pub(crate) fn hold_cap(s: &pulse_store::Store) -> Duration {
    Duration::from_secs(s.setting_int("daemon.hold_seconds").max(0) as u64)
}

/// Asks every run still `running` to hold (request_harness_hold_for_run);
/// returns the run ids asked and, per run, how many harness sessions heard
/// it (none when the worker is past its harness - in the check - or is a
/// custom command: those run to the cap and are stopped).
pub(crate) fn hold_runs_in_flight(s: &pulse_store::Store) -> Vec<(String, usize)> {
    let Ok(conn) = s.conn() else {
        return Vec::new();
    };
    let runs = crate::util::must_rows(
        &conn,
        "SELECT id FROM runs WHERE status = 'running' ORDER BY started_at",
        &[],
    );
    runs.iter()
        .map(|r| {
            let id = r.str("id");
            let asked = s.request_harness_hold_for_run(&id).unwrap_or(0);
            (id, asked)
        })
        .collect()
}

#[derive(Debug, Clone)]
struct PlannerSnapshot {
    watermark: String,
    tally: pulse_tally::TallyResult,
    suggestion_idle: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Dispatch {
    pub(crate) slot: String,
    pub(crate) worker: String,
    pub(crate) job: Row,
    pub(crate) run: Row,
    pub(crate) reason: String,
    /// The tick that started it, ISO; the restart window counts the budget
    /// of the runs in flight from here.
    pub(crate) started_at: String,
}

/// Shared with worker/maintenance threads, so it sits behind a
/// mutex; None writes to stdout.
pub(crate) type DaemonOut = std::sync::Arc<std::sync::Mutex<dyn std::io::Write + Send>>;
pub(crate) type DaemonSleep = std::sync::Arc<dyn Fn(Duration) -> bool + Send + Sync>;

#[derive(Clone, Default)]
pub(crate) struct DaemonOpts {
    pub agent: String,
    pub interval: i32,
    pub dry_run: bool,
    pub worker_cmd: String,
    pub max_loops: i32,
    pub restart_on_change: bool,
    pub grace: std::time::Duration,
    pub out: Option<DaemonOut>,
    pub sleep: Option<DaemonSleep>,
    pub hub_home: String,
    pub self_update: Option<Box<crate::selfupdate::SelfUpdateEnv>>,
    pub no_watchdog: bool,
    /// `--max-jobs`: local runs at once; 0 defers to the `capacity` setting and
    /// then the machine's default (see `daemon_load`).
    pub max_jobs: i32,
}

impl std::fmt::Debug for DaemonOpts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DaemonOpts")
            .field("agent", &self.agent)
            .field("interval", &self.interval)
            .field("dry_run", &self.dry_run)
            .field("worker_cmd", &self.worker_cmd)
            .field("max_loops", &self.max_loops)
            .field("restart_on_change", &self.restart_on_change)
            .field("grace", &self.grace)
            .field("out", &self.out.as_ref().map(|_| "writer"))
            .field("sleep", &self.sleep.as_ref().map(|_| "fn"))
            .field("hub_home", &self.hub_home)
            .field("self_update", &self.self_update)
            .field("no_watchdog", &self.no_watchdog)
            .field("max_jobs", &self.max_jobs)
            .finish()
    }
}

/// How long the loop's bodies take, from the top of the loop to the wait
/// that ends it. Until this was measured the daemon had no number for its
/// own time: the heartbeat said when the loop last beat and how often, so a
/// body that spent the dispatch cadence planning looked like a loop that
/// merely beat late. The heartbeat carries the last body, the slowest and
/// how many ran over the cadence (`tick`); `pulse status` and the hub show
/// them. A body over the cadence is said once and again only after one fits,
/// so a slow planning pass is one line, not one per tick. The line and the
/// heartbeat also name where the time went (`StageClock`): the number alone
/// said that seven passes after one restart took 29-33 s each and nothing
/// about which of the twenty tenders took them.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct TickTiming {
    last: Option<Duration>,
    max: Duration,
    slow: u32,
    over: bool,
    /// The longest stages of the slowest body, for the heartbeat.
    longest: Vec<(&'static str, Duration)>,
}

/// Where a loop body's time goes, by stage. The loop marks the end of each
/// stage and the time since the previous mark is charged to it; a stage
/// that runs once per project (the tenders) is charged every time, so its
/// total across projects is what shows.
#[derive(Debug, Default)]
pub(crate) struct StageClock {
    since: Option<std::time::Instant>,
    spent: BTreeMap<&'static str, Duration>,
}

impl StageClock {
    pub(crate) fn start() -> Self {
        Self {
            since: Some(std::time::Instant::now()),
            ..Self::default()
        }
    }

    /// Charges the time since the last mark (or the start) to `stage`.
    pub(crate) fn mark(&mut self, stage: &'static str) {
        let now = std::time::Instant::now();
        if let Some(since) = self.since.replace(now) {
            self.charge(stage, now.saturating_duration_since(since));
        }
    }

    pub(crate) fn charge(&mut self, stage: &'static str, took: Duration) {
        *self.spent.entry(stage).or_default() += took;
    }

    /// The `n` longest stages of a second or more, longest first.
    pub(crate) fn longest(&self, n: usize) -> Vec<(&'static str, Duration)> {
        let mut all: Vec<_> = self
            .spent
            .iter()
            .filter(|(_, took)| **took >= Duration::from_secs(1))
            .map(|(stage, took)| (*stage, *took))
            .collect();
        all.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        all.truncate(n);
        all
    }
}

/// `plan 14s, maintenance 9s, pipeline 3s`.
fn stages_phrase(stages: &[(&'static str, Duration)]) -> String {
    stages
        .iter()
        .map(|(stage, took)| format!("{stage} {}", pulse_cli::format_duration(*took)))
        .collect::<Vec<_>>()
        .join(", ")
}

impl TickTiming {
    /// Records one body and returns the line to say when it runs over
    /// `bound` after a body that fit (or as the first body). `stages` is
    /// the body's clock; its three longest stages end the line and ride
    /// the heartbeat while the body is the slowest seen.
    pub(crate) fn record(
        &mut self,
        loops: i32,
        took: Duration,
        bound: Duration,
        pass: bool,
        stages: &StageClock,
    ) -> Option<String> {
        self.last = Some(took);
        if took >= self.max {
            self.max = took;
            self.longest = stages.longest(3);
        }
        if took <= bound {
            self.over = false;
            return None;
        }
        self.slow += 1;
        if std::mem::replace(&mut self.over, true) {
            return None;
        }
        let kind = if pass { "planning pass" } else { "refill tick" };
        let longest = stages.longest(3);
        let went = if longest.is_empty() {
            String::new()
        } else {
            format!("; longest: {}", stages_phrase(&longest))
        };
        Some(format!(
            "tick {loops} took {}, over the {} dispatch cadence ({kind}); queued work waited that long for a slot, said again once a tick fits{went}",
            pulse_cli::format_duration(took),
            pulse_cli::format_duration(bound)
        ))
    }

    /// The heartbeat's `tick` object; nothing before the first body ends.
    /// `longest` names the slowest body's longest stages, so `pulse status`
    /// says what a slow pass spent its time on.
    pub(crate) fn beat(&self) -> Option<serde_json::Value> {
        let last = self.last?;
        Some(serde_json::json!({
            "ms": last.as_millis() as u64,
            "max_ms": self.max.as_millis() as u64,
            "slow": self.slow,
            "longest": self
                .longest
                .iter()
                .map(|(stage, took)| serde_json::json!({"stage": stage, "ms": took.as_millis() as u64}))
                .collect::<Vec<_>>(),
        }))
    }
}

/// Writes the daemon's heartbeat row.
/// `passes` counts the planning passes among the `loops`
/// (`daemon.pass_seconds`); the rest were ticks that only reaped, refilled
/// slots and watched the stop switch.
/// `restart` is the pending handover (`Restart::beat`, restart.rs) while a
/// newer binary waits for the runs in flight; the hub reads it to say what
/// the daemon waits for instead of a bare "restarting".
/// `tick` is the loop body's timing (`TickTiming::beat`), absent before
/// the first body ends and in the watchdog's stalled note.
pub(crate) fn daemon_heartbeat(
    s: &pulse_store::Store,
    loops: i32,
    passes: i32,
    interval: i32,
    note: String,
    restart: Option<serde_json::Value>,
    tick: Option<serde_json::Value>,
) {
    let b = pulse_bus::default();
    let mut beat = serde_json::json!({
        "pid": std::process::id(),
        "at": s.now(),
        "loops": loops,
        "passes": passes,
        "interval": interval,
        "note": note,
        "started_at": DAEMON_STARTED_AT.read().expect("daemon start").clone(),
        "version": pulse_cli::version(),
        "revision": pulse_cli::revision(),
        "bus": {"subscribers": b.subscribers(), "dropped": b.dropped(), "deduped": b.deduped()},
        "planner_cache": pulse_tally::cache::cache_stats(),
    });
    if let Some(restart) = restart {
        beat["restart"] = restart;
    }
    if let Some(tick) = tick {
        beat["tick"] = tick;
    }
    // The keep-awake hold and the last sleep noticed (awake.rs), once the
    // loop has ticked; the console reads both from here.
    let awake = crate::awake::beat();
    if let Some(fields) = awake.as_object() {
        for (key, value) in fields {
            beat[key] = value.clone();
        }
    }
    meta_upsert(s, "daemon", &beat);
}

/// Upsert one daemon meta row (the console's heartbeat and capacity notes).
/// A failed write is logged instead of dropped: a silently stale console
/// row would otherwise look like a healthy daemon that merely stopped
/// updating.
pub(crate) fn meta_upsert(s: &pulse_store::Store, key: &str, value: &serde_json::Value) {
    if let Err(e) = meta_upsert_result(s, key, value) {
        eprintln!("daemon: meta '{key}' write failed: {e}");
    }
}

fn meta_upsert_result(
    s: &pulse_store::Store,
    key: &str,
    value: &serde_json::Value,
) -> Result<(), String> {
    // A value that cannot be serialized is an error the caller logs, never an
    // empty string stored where the console expects a JSON document.
    let encoded = pulse_store::gojson::to_string(value).map_err(|e| e.to_string())?;
    let conn = s.conn().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [key.to_string(), encoded],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn capacity_note(
    s: &pulse_store::Store,
    queued: i32,
    slots: i32,
    inflight: i32,
    starved_since: String,
    held: String,
    disk: String,
    bulk: String,
    now: String,
) {
    let note = serde_json::json!({
        "at": now,
        "queued": queued,
        "slots": slots,
        "inflight": inflight,
        "starved_since": starved_since,
        "held": held,
        "disk": disk,
        "bulk": bulk,
    });
    meta_upsert(s, "capacity", &note);
}

pub(crate) fn hands_wanted(
    s: &pulse_store::Store,
    brain: Row,
    queued: i32,
    slots: i32,
    minutes: i32,
    now: String,
    actor: String,
) -> Row {
    if brain.0.is_empty() {
        return Row::default();
    }
    let slug = brain.str("slug");
    let summary = crate::util::trunc(
        &format!(
            "hands wanted: {queued} job(s) waited {minutes} min with every slot busy or held ({slots} slot(s)); pairing another machine (pulse pair) or compute.auto_approve would shorten the queue"
        ),
        230,
    );
    let body = format!(
        "The daemon on this machine found {queued} queued job(s) waiting for {minutes} minutes while every executor slot ({slots}) was busy or held by this machine's load or disk ({now}). More hands would shorten the queue: pair another machine with `pulse pair` (pulse record show WF-executors), set `compute.auto_approve` with `compute.max_vms` so the daemon rents one within your caps or a bigger machine (each machine sizes its own slots from its cores and memory, and the fleet adds them up). The human owns machines and money; this post is a measurement, not a request to act.\n"
    );
    let Ok(Some(post)) = crate::hygiene::post_daemon_owner_question(
        s,
        crate::hygiene::DaemonQuestionAsk {
            project: slug.clone(),
            ask_kind: "hands".into(),
            area: "runner".into(),
            thread: "hands-wanted".into(),
            summary,
            body,
            actor: actor.clone(),
            now: now.clone(),
        },
    ) else {
        return Row::default();
    };
    if let Ok(conn) = s.conn() {
        crate::util::event_at(
            &conn,
            &now,
            &actor,
            "capacity.hands-wanted",
            &slug,
            serde_json::json!({"queued": queued, "slots": slots, "minutes": minutes, "post": post.str("id")}),
        );
    }
    post
}

/// The order the free slots are offered work in: the reserved machines
/// first (they take only the jobs that need them), then the ones kept for
/// certain projects, then the machines that offer the least (a plain Linux
/// hand before the Mac with Xcode, so the scarce machine stays free for the
/// work that needs it), then the least loaded machine with its n-th free
/// slot counting as load, and the hands before this machine on ties (the
/// list has them first).
pub(crate) fn slot_order(slots: &[String], rows: &BTreeMap<String, Row>) -> Vec<usize> {
    let mut nth: BTreeMap<&str, i64> = BTreeMap::new();
    let mut keys = Vec::with_capacity(slots.len());
    for (i, name) in slots.iter().enumerate() {
        let n = nth.entry(name.as_str()).or_insert(0);
        let k = *n;
        *n += 1;
        let row = rows.get(name);
        let reserved = row.is_some_and(crate::executor::reserved);
        let unrestricted = row.is_none_or(|r| r.str("projects") == "*");
        let offers = row.map_or(0, |r| crate::executor::caps(r).len());
        let workers = i64::from(row.map_or(1, |r| r.int("workers").max(1)));
        let free = slots.iter().filter(|s| *s == name).count() as i64;
        // The machine's load once this slot is filled, in thousandths.
        let load = ((workers - free).max(0) + k + 1) * 1000 / workers;
        keys.push(((!reserved, unrestricted, offers, load, i), i));
    }
    keys.sort();
    keys.into_iter().map(|(_, i)| i).collect()
}

/// A kind of machine the queue waits for and no active machine is: the
/// needs the jobs share, the project, how many, and how long the oldest
/// has waited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnmetNeeds {
    pub(crate) project: String,
    pub(crate) needs: String,
    pub(crate) jobs: i32,
    pub(crate) minutes: i32,
}

/// The queued work no active machine can take because of what it needs,
/// grouped by project and needs, oldest first.
pub(crate) fn unmet_needs(s: &pulse_store::Store, now: &str) -> Vec<UnmetNeeds> {
    let Ok(conn) = s.conn() else {
        return Vec::new();
    };
    let machines: Vec<std::collections::BTreeSet<String>> = crate::executor::executor_rows(s, true)
        .iter()
        .filter(|r| r.str("status") == "active")
        .map(crate::executor::caps)
        .collect();
    let rows = crate::util::must_rows(
        &conn,
        "SELECT j.project, j.needs, COUNT(*) AS n, MIN(j.created_at) AS oldest FROM jobs j JOIN projects p ON p.slug = j.project WHERE j.status = 'queued' AND p.status = 'active' AND j.needs != '' GROUP BY j.project, j.needs ORDER BY oldest",
        &[],
    );
    rows.into_iter()
        .filter(|r| {
            let needs = r.str("needs");
            !machines
                .iter()
                .any(|caps| pulse_store::needs::met(&needs, caps))
        })
        .map(|r| {
            let oldest = r.str("oldest");
            let minutes = if oldest.is_empty() {
                0
            } else {
                (crate::util::must_iso(now) - crate::util::must_iso(&oldest)).num_minutes() as i32
            };
            UnmetNeeds {
                project: r.str("project"),
                needs: r.str("needs"),
                jobs: r.int("n"),
                minutes,
            }
        })
        .collect()
}

/// Tells the owner that queued work needs a kind of machine none of the
/// paired ones is, once per project and needs while the question is open:
/// the daemon can rent Linux, not a Mac, and only the owner pairs or labels
/// a machine. Returns the posts made.
pub(crate) fn needs_wanted(
    s: &pulse_store::Store,
    brain: Row,
    unmet: &[UnmetNeeds],
    now: String,
    actor: String,
) -> Vec<Row> {
    if brain.0.is_empty() || unmet.is_empty() {
        return Vec::new();
    }
    let slug = brain.str("slug");
    let machines = crate::executor::executor_rows(s, true)
        .iter()
        .map(|r| {
            format!(
                "{} ({}): {}",
                r.str("name"),
                r.str("status"),
                crate::executor::caps_text(r)
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    let mut posts = Vec::new();
    for u in unmet {
        let summary = crate::util::trunc(
            &format!(
                "machine wanted: {} job(s) of {} need {}, which no active machine offers",
                u.jobs, u.project, u.needs
            ),
            230,
        );
        let body = format!(
            "{} queued job(s) of {} need a machine that offers `{}`; none of the machines paired here does, so they wait ({} min so far, {now}). The machines and what they offer: {machines}. A machine that has it can be paired with `pulse pair` (a Linux machine) or added by `pulse executor add`; a paired machine that does have it can be told so with `pulse executor label NAME +word` (this machine: the setting `machine.labels`); or the project's needs can change in its `.pulse/pulse.yaml` (`needs:`) or with `pulse job needs JOB_ID -`. The daemon rents Linux machines only, so a need such as `macos` is the owner's to meet. This post is a measurement, not a request to act.\n",
            u.jobs, u.project, u.needs, u.minutes
        );
        let Ok(Some(post)) = crate::hygiene::post_daemon_owner_question(
            s,
            crate::hygiene::DaemonQuestionAsk {
                project: slug.clone(),
                ask_kind: "needs".into(),
                area: "runner".into(),
                thread: "machines".into(),
                summary,
                body,
                actor: actor.clone(),
                now: now.clone(),
            },
        ) else {
            continue;
        };
        if let Ok(conn) = s.conn() {
            crate::util::event_at(
                &conn,
                &now,
                &actor,
                "capacity.needs-wanted",
                &u.project,
                serde_json::json!({"needs": u.needs, "jobs": u.jobs, "minutes": u.minutes, "post": post.str("id")}),
            );
        }
        posts.push(post);
    }
    posts
}

/// Hours a machine the daemon rents on its own is asked for: enough for
/// the queued job and its verify, short enough that a machine the queue no
/// longer needs costs an afternoon, not a day (the compute worker releases
/// an idle one earlier).
pub(crate) const AUTO_COMPUTE_HOURS: i32 = 4;

/// Why `auto_compute` rented nothing: a request is open already (a machine
/// is coming, or `pending` waits for the owner), or the caps or the queue
/// said no.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum NoAutoCompute {
    Open { id: String, status: String },
    Why(String),
}

impl std::fmt::Display for NoAutoCompute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open { id, status } => write!(f, "compute request {id} is {status}"),
            Self::Why(why) => f.write_str(why),
        }
    }
}

/// Rents a machine for a starved queue when the owner pre-authorised it:
/// `compute.auto_approve` within `compute.max_vms` and
/// `compute.max_hours_per_day`, one open request at a time, and only for a
/// work kind (improve, task) that has itself waited `HANDS_WANTED_MINUTES`.
/// Until now the daemon only posted "hands wanted" to an owner who may be
/// away for the day while the caps said the machine could be rented.
pub(crate) fn auto_compute(
    s: &pulse_store::Store,
    queued: i32,
    waited: i32,
    now: &str,
    actor: &str,
) -> Result<pulse_tasks::compute::ComputeRequest, NoAutoCompute> {
    let why = |w: String| NoAutoCompute::Why(w);
    let caps = pulse_tasks::compute::get_caps(s).map_err(|e| why(e.to_string()))?;
    if !caps.auto_approve {
        return Err(why("compute.auto_approve is off".into()));
    }
    if caps.vms_now >= caps.max_vms {
        return Err(why(format!(
            "compute.max_vms {} reached ({} in use)",
            caps.max_vms, caps.vms_now
        )));
    }
    if caps.max_hours_per_day > 0 && caps.hours_today + AUTO_COMPUTE_HOURS > caps.max_hours_per_day
    {
        return Err(why(format!(
            "compute.max_hours_per_day {} would be passed ({} h today, {AUTO_COMPUTE_HOURS} h asked)",
            caps.max_hours_per_day, caps.hours_today
        )));
    }
    let conn = s.conn().map_err(|e| why(e.to_string()))?;
    let open = crate::util::must_one(
        &conn,
        "SELECT id, status FROM compute_requests WHERE status IN ('pending', 'approved', 'provisioning', 'ready') ORDER BY id DESC LIMIT 1",
        &[],
    );
    if !open.0.is_empty() {
        return Err(NoAutoCompute::Open {
            id: open.str("id"),
            status: open.str("status"),
        });
    }
    let since = crate::util::plus_minutes(now, -HANDS_WANTED_MINUTES);
    // The machine the daemon rents is a Linux hand of the provider's default
    // size: only a job such a machine can run is worth renting one for.
    let size_setting = pulse_store::compute_size_setting(&s.setting("compute.provider", ""));
    let rented = pulse_store::needs::caps_of(
        "linux",
        &pulse_tasks::compute::default_hands_arch(&s.setting(&size_setting, "")),
        "",
    );
    let (fit, words) = pulse_store::needs::sql_fit("j.needs", &rented);
    let mut vals: Vec<&dyn rusqlite::ToSql> = vec![&since];
    for w in &words {
        vals.push(w);
    }
    let job = crate::util::must_one(
        &conn,
        &format!(
            "SELECT j.id, j.kind, j.project FROM jobs j JOIN projects p ON p.slug = j.project WHERE j.status = 'queued' AND p.status = 'active' AND j.kind IN ({}) AND j.created_at <= ? AND {fit} ORDER BY j.created_at LIMIT 1",
            crate::jobs::work_kinds_sql()
        ),
        &vals,
    );
    if job.0.is_empty() {
        return Err(why(format!(
            "no improve or task job that a rented Linux machine could run has waited {HANDS_WANTED_MINUTES} min"
        )));
    }
    let reason = format!(
        "{queued} job(s) waited {waited} min with every slot on this machine busy or held; {} ({}) is first in line",
        job.str("id"),
        job.str("kind")
    );
    pulse_tasks::compute::request_compute(
        s,
        &job.str("project"),
        actor,
        "",
        AUTO_COMPUTE_HOURS,
        &reason,
    )
    .map_err(|e| why(e.to_string()))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn disk_floor_wanted(
    s: &pulse_store::Store,
    brain: Row,
    path: String,
    free: i32,
    floor: i32,
    minutes: i32,
    now: String,
    actor: String,
) -> Row {
    if brain.0.is_empty() {
        return Row::default();
    }
    let slug = brain.str("slug");
    let summary = crate::util::trunc(
        &format!(
            "disk under floor: {free} MB free on {path} for {minutes} min; daemon.min_free_mb is {floor}"
        ),
        230,
    );
    // What the reclaim ladder already took back since this crossing, so the
    // owner is not asked to free what the daemon could and told what is left.
    let since = crate::util::plus_minutes(&now, -f64::from(minutes.max(0)));
    let reclaimed = crate::reclaim::last_reclaim_since(s, &since)
        .map(|r| format!(" {}", r.note()))
        .unwrap_or_default();
    let body = format!(
        "The daemon paused new leases because the work filesystem `{path}` had {free} MB free for {minutes} minutes, under `daemon.min_free_mb` ({floor}) at {now}.{reclaimed} Remedy it by freeing space on that filesystem, or adjust `daemon.min_free_mb` up or down to the floor this machine should enforce. Existing runs keep draining; no new job or experiment measurement starts while the floor is crossed.\n"
    );
    let Ok(post) = crate::board::create_post(
        s,
        now.clone(),
        crate::board::PostOpts {
            kind: "finding".into(),
            project: slug.clone(),
            area: "runner".into(),
            thread: "disk-floor".into(),
            summary,
            body,
            agent: crate::board::actor_agent(s, actor.clone()),
            date: now.clone(),
            ..Default::default()
        },
    ) else {
        return Row::default();
    };
    if let Ok(conn) = s.conn() {
        crate::util::event_at(
            &conn,
            &now,
            &actor,
            "capacity.disk-floor",
            &slug,
            serde_json::json!({"free_mb": free, "floor_mb": floor, "path": path, "minutes": minutes, "post": post.str("id")}),
        );
    }
    post
}

pub(crate) fn run_budget(s: &pulse_store::Store) -> std::time::Duration {
    let minutes = s.setting_int("minutes_per_job").max(1);
    Duration::from_secs(((2 * minutes + i64::from(crate::recipe::SETUP_MINUTES) + 15) as u64) * 60)
}

/// Say why the project list is empty when the store connection fails, so a
/// DB hiccup no longer looks like "zero projects" with no explanation.
pub(crate) fn warn_store_unavailable(o: &DaemonOpts, e: &anyhow::Error) {
    say(
        o,
        format!(
            "store unavailable: {}; skipping projects this loop",
            crate::run::err_message(e)
        ),
    );
}

/// Set once the daemon log sink has failed, so the warning below is printed a
/// single time instead of once per lost line.
static SAY_FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn say(o: &DaemonOpts, line: impl AsRef<str>) {
    use std::io::Write;
    let line = line.as_ref();
    let result = match &o.out {
        Some(out) => {
            let mut w = out.lock().unwrap_or_else(|e| e.into_inner());
            writeln!(w, "{line}").and_then(|()| w.flush())
        }
        None => {
            let stdout = std::io::stdout();
            let mut w = stdout.lock();
            writeln!(w, "{line}").and_then(|()| w.flush())
        }
    };
    if let Err(e) = result {
        // A closed pipe or full disk used to swallow every later log line
        // without a trace; report the first failure once on stderr so the
        // owner learns why the daemon log went quiet.
        if !SAY_FAILED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            eprintln!("pulse daemon: log write failed ({e}); later log lines may be lost");
        }
    }
}

/// The least time between two planning passes (`daemon.pass_seconds`). The
/// loop wakes every `DISPATCH_TICK_SECONDS` while runs are in flight and
/// whenever a run ends or the bus has news, and each wake used to run the
/// whole pass: the leads, tasks, amendments, recoveries and lessons of every
/// project, the evolution and schedules, the planner, the idle notes, the
/// executors and a maintenance thread. With four agents at work that was a
/// pass every fifteen seconds, a quarter of a core, for jobs that only queued
/// behind slots already full. A tick inside the spacing reaps, watches the
/// stop switch and the restart drain, starts approved compute and refills
/// free slots; the planning pass waits for the spacing, however often the
/// loop wakes. Zero plans on every tick as before; the loop caps the spacing
/// at its interval, so `--interval 30` still plans every half minute.
pub(crate) fn pass_spacing(s: &pulse_store::Store) -> Duration {
    Duration::from_secs(s.setting_int("daemon.pass_seconds").max(0) as u64)
}

/// Whether the governor would admit work now: read between the polls of an
/// idle sleep under a hold, so the queue moves when the load does.
pub(crate) fn hold_lifted(
    s: &pulse_store::Store,
    machine: &crate::daemon_load::Machine,
    flag_max_jobs: i32,
) -> bool {
    let limits = crate::daemon_load::limits(s, machine, flag_max_jobs);
    crate::daemon_load::hold_reason(&limits, &crate::daemon_load::pressure()).is_none()
}

/// What an idle sleep watches between its polls.
pub(crate) struct IdleWatch<'a> {
    pub(crate) s: &'a pulse_store::Store,
    pub(crate) machine: &'a crate::daemon_load::Machine,
    pub(crate) flag_max_jobs: i32,
    pub(crate) done: crossbeam_channel::Receiver<()>,
    pub(crate) compute_wake: &'a crossbeam_channel::Receiver<()>,
    /// Time between polls: `DISPATCH_TICK_SECONDS` in the daemon.
    pub(crate) poll: Duration,
}

/// Waits `d` while nothing runs, waking early when the queue changes, the
/// bus or compute has news or, with `held` (the governor closed the slots
/// this tick with work queued), when the machine's pressure no longer
/// holds: a hold that lifted two minutes into a five-minute sleep used to
/// cost the rest of it, every time. False once the loop is cancelled.
pub(crate) fn idle_wait(
    w: &IdleWatch<'_>,
    wake_events: &mut Option<crossbeam_channel::Receiver<pulse_bus::Event>>,
    d: Duration,
    held: bool,
) -> bool {
    let never_wake = crossbeam_channel::never::<pulse_bus::Event>();
    let start_queued = active_queued_count(w.s);
    let deadline = std::time::Instant::now() + d;
    loop {
        let now_i = std::time::Instant::now();
        if now_i >= deadline {
            return true;
        }
        let wait = std::cmp::min(w.poll, deadline.saturating_duration_since(now_i));
        let timer = crossbeam_channel::after(wait);
        crossbeam_channel::select! {
            recv(w.done) -> _ => return false,
            recv(timer) -> _ => {
                if active_queued_count(w.s) != start_queued {
                    return true;
                }
                if held && hold_lifted(w.s, w.machine, w.flag_max_jobs) {
                    return true;
                }
            },
            recv(wake_events.as_ref().unwrap_or(&never_wake)) -> msg => {
                if msg.is_err() {
                    *wake_events = None;
                    continue;
                }
                return true;
            },
            recv(w.compute_wake) -> _ => return true,
        }
    }
}

/// How many times one experiment goes back in the queue after a provider
/// outage before its failure stands: an outage that outlasts the retries is
/// left to the loop's own provider-stop rule (pulse-tally).
pub(crate) const OUTAGE_REQUEUE_LIMIT: i32 = 2;
/// The `job.requeue` detail an outage requeue carries, so its count stays
/// apart from the landing requeues (land.rs `LAND_REQUEUE_LIMIT`).
const OUTAGE_REQUEUE_WHY: &str = "provider outage before any command";
/// Words of a provider error that never reached the model: the request did
/// not go through, so the experiment's idea was never tried.
const TRANSPORT_MARKS: &[&str] = &[
    "error sending request",
    "connection reset",
    "connection refused",
    "connection closed",
    "timed out",
    "no such host",
    "dns error",
    "network is unreachable",
    "unexpected eof",
    "temporarily unavailable",
    "service unavailable",
    "bad gateway",
    "gateway timeout",
    " 502",
    " 503",
    " 504",
];

/// Whether a finished run is an experiment cycle lost to a provider outage:
/// the run failed with no commit, its harness stopped on `provider_error`
/// before a single command, and the error is a transport one.
pub(crate) fn outage_lost_cycle(job: &Row, run: &Row) -> bool {
    let summary = run.str("summary").to_lowercase();
    job.str("kind") == "experiment"
        && run.str("status") == "failed"
        && run.int("commits") == 0
        && summary.contains("harness stopped: provider_error after ")
        && summary.contains(" model calls and 0 commands")
        && TRANSPORT_MARKS.iter().any(|m| summary.contains(m))
}

/// Puts an experiment whose cycle a provider outage lost back in the queue,
/// so it is run again instead of counting as a tried card or a failed cycle;
/// returns the line to log. After `OUTAGE_REQUEUE_LIMIT` such requeues the
/// failure stands.
pub(crate) fn requeue_outage_experiment(
    s: &pulse_store::Store,
    run: &Row,
    actor: &str,
) -> Option<String> {
    let conn = s.conn().ok()?;
    let job = crate::jobs::job_row(&conn, run.str("job_id"));
    if job.0.is_empty() || job.str("status") != "failed" || !outage_lost_cycle(&job, run) {
        return None;
    }
    let earlier = crate::util::must_one(
        &conn,
        "SELECT COUNT(*) AS n FROM events WHERE action = 'job.requeue' AND subject = ? AND detail LIKE ?",
        &[&job.str("id"), &format!("%{OUTAGE_REQUEUE_WHY}%")],
    )
    .int("n");
    if earlier >= OUTAGE_REQUEUE_LIMIT {
        return None;
    }
    let now = s.now();
    crate::jobs::finish_job(
        s,
        job.str("id"),
        actor.to_string(),
        "queued".into(),
        format!(
            "requeued: {OUTAGE_REQUEUE_WHY}: {}",
            crate::util::trunc(&run.str("summary"), 300)
        ),
        String::new(),
        -1,
    )
    .ok()?;
    crate::util::exec1(
        &conn,
        "UPDATE jobs SET attempts = 0, branch = '' WHERE id = ?",
        &[&job.str("id")],
    );
    crate::util::event_at(
        &conn,
        &now,
        actor,
        "job.requeue",
        &job.str("id"),
        serde_json::json!({"why": OUTAGE_REQUEUE_WHY, "patch": "", "run": run.str("id")}),
    );
    Some(format!(
        "requeued {}: {OUTAGE_REQUEUE_WHY} ({} of {OUTAGE_REQUEUE_LIMIT}); the cycle runs again instead of counting as tried",
        job.str("id"),
        earlier + 1
    ))
}

fn active_queued_count(s: &pulse_store::Store) -> i32 {
    s.conn()
        .ok()
        .map(|c| {
            crate::util::must_one(
                &c,
                "SELECT COUNT(*) AS n FROM jobs j JOIN projects p ON p.slug = j.project WHERE j.status = 'queued' AND p.status = 'active'",
                &[],
            )
            .int("n")
        })
        .unwrap_or(0)
}

fn planner_snapshot(
    s: &pulse_store::Store,
    cache: &mut BTreeMap<String, PlannerSnapshot>,
    slug: &str,
    now: &str,
) -> Option<PlannerSnapshot> {
    let watermark = pulse_tally::cache::project_watermark(s, slug, "").ok()?;
    if let Some(snapshot) = cache.get(slug)
        && snapshot.watermark == watermark
    {
        return Some(snapshot.clone());
    }
    let tally = pulse_tally::compute_light(
        s,
        slug,
        "",
        pulse_tally::Thresholds {
            now: now.to_string(),
            ..Default::default()
        },
    )
    .ok()?;
    let suggestion_idle = crate::idle::suggestion_idle_part_with_tally(
        s,
        slug.to_string(),
        now.to_string(),
        Some(&tally),
    );
    let snapshot = PlannerSnapshot {
        watermark,
        tally,
        suggestion_idle,
    };
    cache.insert(slug.to_string(), snapshot.clone());
    Some(snapshot)
}

fn dispatch_key(slot: &str, worker: &str, job: &Row) -> String {
    format!("{slot}\0{worker}\0{}", job.str("id"))
}

pub(crate) fn daemon(
    cancel: &pulse_bus::CancellationToken,
    s: &pulse_store::Store,
    o: DaemonOpts,
) -> String {
    let mut interval = o.interval;
    if interval <= 0 {
        interval = DEFAULT_DAEMON_INTERVAL_SECONDS;
    }

    let actor = if o.agent.is_empty() {
        "daemon".to_string()
    } else {
        o.agent.clone()
    };
    let mut loops = 0;
    let mut passes = 0;
    let mut last_prune_day = String::new();
    // When the last planning pass began (`pass_spacing`); None before the
    // first, which plans at once.
    let mut last_pass: Option<std::time::Instant> = None;
    // The bus position the last planning pass read up to: an owner's write
    // (OWNER_WAKE) after it makes the next tick a planning pass whatever
    // the spacing says, so a task the owner files is planned at once.
    let mut planned_through = pulse_bus::last_id(s).unwrap_or(0);
    let mut prev_idle: BTreeMap<String, String> = BTreeMap::new();
    let mut idle_log = crate::idle_log::IdleLog::default();
    // The restart handover (restart.rs): armed when the binary changed on
    // disk or the owner asked for a restart while runs were in flight, it
    // says which runs may still start, what the heartbeat waits for and when
    // this daemon exits with code 3 so the service restarts it.
    let mut restart = crate::restart::Restart::default();
    // Asks for a restart are read from here on: the ask that restarted the
    // daemon before this one is behind it.
    let restart_asks_after = pulse_bus::last_id(s).unwrap_or(0);
    let mut maintenance = crate::daemon_maintenance::DaemonMaintenance::default();
    let mut maintenance_wake = false;
    let mut probes = crate::checkout::CheckoutProbes::default();
    let mut refused = crate::amend::AmendRefusals::new();
    let stamp = (BINARY_STAMP.read().expect("binary stamp"))();
    let (results_tx, results_rx) = crossbeam_channel::bounded::<Dispatch>(64);
    let mut inflight: BTreeMap<String, Dispatch> = BTreeMap::new();
    let loop_ctx = cancel.child();
    let run_ctx = pulse_bus::CancellationToken::new();
    let maintenance_ctx = cancel.child();
    let compute_ctx = pulse_bus::CancellationToken::new();
    let (wake_events, cancel_wake) = pulse_bus::default().subscribe(
        pulse_bus::Filter {
            actions: DAEMON_WAKE.iter().map(|s| (*s).to_string()).collect(),
            ..Default::default()
        },
        32,
    );
    pulse_tally::set_log(Some(Arc::new({
        let out = o.clone();
        move |line| say(&out, line)
    })));
    let bridge_done = pulse_bus::bridge(loop_ctx.clone(), s.clone(), Duration::ZERO);
    let mut hooks_runner = pulse_hooks::Runner::new(s.clone());
    hooks_runner.bus = Some(pulse_bus::default());
    let hook_opts = o.clone();
    hooks_runner.log = Some(Arc::new(move |line| {
        say(&hook_opts, format!("hook: {line}"))
    }));
    let (hooks_done_tx, hooks_done_rx) = crossbeam_channel::bounded(1);
    let hooks_ctx = loop_ctx.clone();
    std::thread::Builder::new()
        .name("daemon-hooks".into())
        .spawn(move || {
            hooks_runner.run(hooks_ctx);
            let _ = hooks_done_tx.send(());
        })
        .expect("spawn daemon hooks");
    // The watchdog outlives nothing: a loop that stops beating for daemonStallBound is dumped and restarted.
    // The stalled note reads `loops` from the watchdog thread, so it is shared.
    let loops_shared = Arc::new(std::sync::atomic::AtomicI32::new(0));
    let passes_shared = Arc::new(std::sync::atomic::AtomicI32::new(0));
    let dog_stop = pulse_bus::CancellationToken::new();
    let _dog_stop = CancelOnDrop(dog_stop.clone());
    let dog = (!o.no_watchdog).then(|| {
        let s_note = s.clone();
        let actor_note = actor.clone();
        let loops_note = Arc::clone(&loops_shared);
        let passes_note = Arc::clone(&passes_shared);
        let out = o.clone();
        let kill = run_ctx.clone();
        let dog = Arc::new(crate::daemon_watchdog::new_daemon_watchdog(
            crate::daemon_watchdog::daemon_stall_bound(interval),
            std::path::Path::new(&s.home)
                .join("logs")
                .display()
                .to_string(),
            Arc::new(move |line| say(&out, line)),
            Some(Arc::new(move |note: String, dump: String| {
                let loops = loops_note.load(std::sync::atomic::Ordering::SeqCst);
                let passes = passes_note.load(std::sync::atomic::Ordering::SeqCst);
                daemon_heartbeat(&s_note, loops, passes, interval, note.clone(), None, None);
                if let Ok(conn) = s_note.conn() {
                    crate::util::event_at(
                        &conn,
                        &s_note.now(),
                        &actor_note,
                        "daemon.stalled",
                        "",
                        serde_json::json!({"note": note, "dump": dump, "loops": loops}),
                    );
                }
            })),
            Some(Arc::new(move || kill.cancel())),
        ));
        let dog_run = Arc::clone(&dog);
        // The watchdog outlives the loop: it watches the shutdown wait too
        // (`DaemonWatchdog::enter`) and stops when daemon() returns.
        let dog_ctx = dog_stop.clone();
        std::thread::Builder::new()
            .name("daemon-watchdog".into())
            .spawn(move || dog_run.run(&dog_ctx, Duration::from_secs(60)))
            .expect("spawn daemon watchdog");
        dog
    });
    // The heartbeat row plus the watchdog's beat.
    let heartbeat = |loops: i32,
                     passes: i32,
                     note: &str,
                     restart: Option<serde_json::Value>,
                     tick: Option<serde_json::Value>| {
        daemon_heartbeat(s, loops, passes, interval, note.to_string(), restart, tick);
        if let Some(dog) = &dog {
            dog.beat();
        }
    };
    let lost = crate::run::sweep_lost_runs(s, &s.now());
    let self_env =
        o.self_update
            .as_ref()
            .map(|b| (**b).clone())
            .unwrap_or(crate::selfupdate::SelfUpdateEnv {
                binary: crate::service::pulse_binary(),
                cargo_cmd: "cargo".into(),
                revision: pulse_cli::revision(),
                actor: actor.clone(),
                ..Default::default()
            });
    let dry = if o.dry_run { ", dry run" } else { "" };
    *DAEMON_STARTED_AT.write().expect("daemon start") = s.now();
    say(
        &o,
        format!(
            "daemon started {} (interval {}s{}, pid {})",
            s.now(),
            interval,
            dry,
            std::process::id()
        ),
    );
    // The load governor: the machine's shape is read once; the limits are
    // re-read from the settings each tick so a changed setting takes effect
    // without a restart. Job children run at nice CHILD_NICE from here on,
    // the daemon itself at `daemon.nice`.
    let machine = crate::daemon_load::Machine::detect();
    crate::daemon_load::set_child_nice(crate::daemon_load::CHILD_NICE);
    let limits = crate::daemon_load::limits(s, &machine, o.max_jobs);
    crate::daemon_load::renice_self(limits.own_nice);
    crate::daemon_load::set_build_jobs(limits.build_jobs);
    say(&o, crate::daemon_load::describe(&machine, &limits));
    if lost > 0 {
        say(
            &o,
            format!("closed {lost} run(s) a previous runner left in progress as lost"),
        );
    }
    // ADR-038: the framework's records travel in the binary; a store that lacks them gets them here.
    match pulse_records::ensure_framework(s, &actor) {
        Err(err) => say(&o, format!("framework records: {err:#}")),
        Ok(seeded) if !seeded.written.is_empty() => say(
            &o,
            format!(
                "seeded {} framework record(s) from {}",
                seeded.written.len(),
                seeded.from
            ),
        ),
        Ok(_) => {}
    }
    for line in crate::selfupdate::check_restored(s, self_env.clone()) {
        say(&o, line);
    }
    let compute = crate::compute::new_compute_worker(
        &compute_ctx,
        s,
        crate::compute::ComputeOpts {
            actor: actor.clone(),
            hub_home: o.hub_home.clone(),
        },
    );
    let compute_wake = compute.wake_rx();
    // Cleared once the subscription closes, so a closed channel never wakes the loop again.
    let mut wake_events = Some(wake_events);
    let never_wake = crossbeam_channel::never::<pulse_bus::Event>();
    // Keep the batch outcome until every slot drains, even across dispatch ticks.
    let mut completed = false;
    let mut all_errors = true;
    let mut starved_since = String::new();
    let mut disk_floor_since = String::new();
    let mut disk_floor_posted = false;
    let mut disk_floor_was_under = false;
    // Whether the last pass found the bulk volume away (bulk.rs), so its
    // return is said once.
    let mut bulk_was_away = false;
    // Whether the last tick found the daily cap holding queued work (the
    // owner heard once; daily_cap.rs), so its lifting is said once too;
    // unknown on the first tick, when a question from before a restart may
    // be open.
    let mut cap_was_holding: Option<bool> = None;
    // When the last disk-floor finding was posted; a re-crossing inside
    // DISK_FLOOR_REPOST_MINUTES keeps pausing but does not post again.
    let mut disk_floor_last_post = String::new();
    let mut planner_cache: BTreeMap<String, PlannerSnapshot> = BTreeMap::new();
    fn report(
        o: &DaemonOpts,
        inflight: &mut BTreeMap<String, Dispatch>,
        completed: &mut bool,
        all_errors: &mut bool,
        d: Dispatch,
    ) {
        inflight.remove(&dispatch_key(&d.slot, &d.worker, &d.job));
        *completed = true;
        if !d.run.0.is_empty() && d.run.str("status") != "error" {
            *all_errors = false;
        }
        if !d.run.0.is_empty() {
            say(
                o,
                format!(
                    "{} {} {} on {}: {}",
                    d.run.str("status"),
                    d.run.str("job_id"),
                    d.run.str("agent"),
                    d.slot,
                    d.run.str("summary")
                ),
            );
        } else if !d.reason.is_empty() {
            say(o, format!("idle: {}", d.reason));
        }
    }
    // Waits d, or less when the store wakes the loop; false once the loop is cancelled.
    let sleep_wait = |wake_events: &mut Option<crossbeam_channel::Receiver<pulse_bus::Event>>,
                      d: Duration|
     -> bool {
        if let Some(sleep) = &o.sleep {
            return sleep(d);
        }
        let timer = crossbeam_channel::after(d);
        let done = cancel.done();
        loop {
            crossbeam_channel::select! {
                recv(done) -> _ => return false,
                recv(timer) -> _ => return true,
                recv(wake_events.as_ref().unwrap_or(&never_wake)) -> msg => {
                    if msg.is_err() {
                        *wake_events = None;
                        continue;
                    }
                    return true;
                },
                recv(compute_wake) -> _ => return true,
            }
        }
    };
    let sleep_idle_wait =
        |wake_events: &mut Option<crossbeam_channel::Receiver<pulse_bus::Event>>,
         d: Duration,
         held: bool|
         -> bool {
            if let Some(sleep) = &o.sleep {
                return sleep(d);
            }
            let watch = IdleWatch {
                s,
                machine: &machine,
                flag_max_jobs: o.max_jobs,
                done: cancel.done(),
                compute_wake: &compute_wake,
                poll: Duration::from_secs(DISPATCH_TICK_SECONDS as u64),
            };
            idle_wait(&watch, wake_events, d, held)
        };
    // The loop's exits name their result and the note the say lines carry.
    let leave: (&str, &str);
    // Set when the watchdog asks for the restart: the stuck pass has already
    // had `RESTART_DRAIN` since its cancellation, so the exit does not wait
    // for it again.
    let mut stuck_pass = false;
    // The keep-awake hold (daemon.keep_awake) and the sleep watch; the hold
    // follows the setting each tick and goes with this daemon.
    let mut awake = crate::awake::Awake::new();
    // The loop body's timing; the heartbeat written at the top of a loop
    // carries the body before it.
    let mut tick_timing = TickTiming::default();
    let tick_bound = Duration::from_secs(DISPATCH_TICK_SECONDS as u64);
    loop {
        loops += 1;
        loops_shared.store(loops, std::sync::atomic::Ordering::SeqCst);
        let tick_began = std::time::Instant::now();
        let mut stages = StageClock::start();
        // The wall clock got ahead of the monotonic clock while the loop
        // waited: the machine slept. Said, recorded as an event, and kept in
        // the heartbeat so the console can explain the lost hours.
        if let Some(secs) = awake.notice_sleep(&s.now()) {
            let slept = pulse_cli::format_duration(Duration::from_secs(secs));
            let hint = if awake.mode() == crate::awake::Mode::Off {
                "; daemon.keep_awake=on keeps it awake while PULSE runs"
            } else {
                ""
            };
            say(
                &o,
                format!("the machine slept for about {slept}; runs in flight lost that time{hint}"),
            );
            if let Ok(conn) = s.conn() {
                crate::util::event_at(
                    &conn,
                    &s.now(),
                    &actor,
                    crate::awake::SLEPT_ACTION,
                    "",
                    serde_json::json!({
                        "seconds": secs,
                        "inflight": inflight.len(),
                        "keep_awake": awake.mode().as_str(),
                    }),
                );
            }
        }
        for line in awake.tick(s) {
            say(&o, line);
        }
        // Collect the runs whose threads ended, without waiting.
        while let Ok(d) = results_rx.try_recv() {
            report(&o, &mut inflight, &mut completed, &mut all_errors, d);
        }
        // A console restart asked with runs in flight drains: once they have
        // all ended, go. (The drain-budget stop is below, with the rest.)
        if restart.console_draining() && inflight.is_empty() {
            say(
                &o,
                "restart asked and nothing is in flight; exiting with code 3 so the service restarts it",
            );
            leave = ("restart", "restarting");
            break;
        }
        let only_maintenance_wake = maintenance_wake;
        maintenance_wake = false;
        for line in maintenance.drain(s) {
            say(&o, line);
        }
        match maintenance.stuck() {
            Ok(None) => {}
            Ok(Some(line)) => say(&o, line),
            Err(line) => {
                say(&o, line);
                if o.restart_on_change {
                    stuck_pass = true;
                    leave = ("restart", "restarting");
                    break;
                }
            }
        }
        if completed && inflight.is_empty() {
            if all_errors && !sleep_wait(&mut wake_events, Duration::from_secs(interval as u64)) {
                leave = ("interrupted", "stopping on signal");
                break;
            }
            completed = false;
            all_errors = true;
        }
        if cancel.is_cancelled() {
            leave = ("interrupted", "stopping on signal");
            break;
        }
        if o.max_loops > 0 && inflight.len() == 1 {
            let done = cancel.done();
            crossbeam_channel::select! {
                recv(results_rx) -> msg => if let Ok(d) = msg {
                    report(&o, &mut inflight, &mut completed, &mut all_errors, d);
                },
                recv(done) -> _ => {
                    leave = ("interrupted", "stopping on signal");
                    break;
                },
            }
            continue;
        }
        let why = crate::run::runner_stopped(s);
        if !why.is_empty() {
            say(&o, format!("stopped: {why}"));
            leave = ("stopped", "stopping");
            break;
        }
        // The owner asked for a restart: by default the daemon drains like
        // a self-update (it stops starting runs and leaves when they end or
        // the drain is up, so nothing is held or lost); with "now" the runs
        // in flight hold (the harness saves its conversation, the job's next
        // run carries on) and this daemon leaves at once, its wait after the
        // loop capped by daemon.hold_seconds.
        if o.restart_on_change
            && let Some(ask) = restart_asked(s, restart_asks_after)
        {
            if ask.force {
                let held = hold_runs_in_flight(s);
                let heard = held.iter().filter(|(_, n)| *n > 0).count();
                let cap = hold_cap(s);
                say(
                    &o,
                    format!(
                        "restart asked by {} at {}: {} run(s) in flight, {heard} asked to hold; exiting with code 3 once they have or after {} so the service restarts it",
                        ask.actor,
                        ask.at,
                        inflight.len(),
                        pulse_cli::format_duration(cap)
                    ),
                );
                for (run, n) in &held {
                    say(
                        &o,
                        if *n > 0 {
                            format!("run {run}: {n} harness session(s) asked to hold")
                        } else {
                            format!(
                                "run {run}: no harness session running (in its check, or a custom worker); it runs to the cap"
                            )
                        },
                    );
                }
                restart.force(s, &inflight);
                leave = ("restart", "restarting now, as asked");
                break;
            }
            if let Some(line) = restart.drain_as_asked(s, &ask, &inflight) {
                say(&o, line);
            }
            if inflight.is_empty() {
                leave = ("restart", "restarting, as asked");
                break;
            }
            // The tick goes on with the drain in place: the window closes the
            // slots (`Restart::hold`), the drain's end is judged
            // (`Restart::tend`), the heartbeat is written and the loop waits
            // for a run to end or the dispatch tick. Skipping the rest of the
            // tick here spun the loop instead - the same ask is read every
            // tick - through its queries with no wait and no heartbeat for
            // as long as the runs took, and the drain's own end never came.
        }
        // A newer binary on disk arms the restart or, once its drain is
        // over, decides whether this daemon leaves; the console's drain and
        // a binary changed back are tended here too (`Restart::tend`).
        let changed = o.restart_on_change
            && !stamp.is_empty()
            && (BINARY_STAMP.read().expect("binary stamp"))() != stamp;
        match restart.tend(s, changed, &inflight, &mut idle_log) {
            crate::restart::Tend::Exits(line) => {
                say(&o, line);
                leave = ("restart", "restarting");
                break;
            }
            crate::restart::Tend::Goes(line) => {
                if let Some(line) = line {
                    say(&o, line);
                }
            }
        }
        let now = s.now();
        let (beat_note, restart_record) = restart.heartbeat(&now, &inflight);
        // A bounded daemon (`--once`, the tests' loop limits) plans on every
        // loop: its loops are the passes it was asked for. An interval shorter
        // than the spacing caps it: the owner asked to plan that often.
        let spacing = pass_spacing(s).min(Duration::from_secs(interval as u64));
        // The owner wrote since the last pass: plan now, not when the
        // spacing is up. The lead's job for a fresh task (lead_due) is the
        // whole point of the wake; a tick that only refilled slots would
        // leave the task "waiting for the lead" for up to a minute.
        let bus_now = pulse_bus::last_id(s).unwrap_or(planned_through);
        let owner_wrote = bus_now > planned_through && owner_wrote_after(s, planned_through);
        let full_pass = o.max_loops > 0
            || owner_wrote
            || last_pass.is_none_or(|began| began.elapsed() >= spacing);
        if full_pass {
            last_pass = Some(std::time::Instant::now());
            passes += 1;
            passes_shared.store(passes, std::sync::atomic::Ordering::SeqCst);
        }
        // Read up to here either way: a range with no owner write in it is
        // not read again next tick.
        planned_through = bus_now;
        heartbeat(
            loops,
            passes,
            &beat_note,
            restart_record,
            tick_timing.beat(),
        );
        let mut started: Vec<Dispatch> = Vec::new();
        let mut reason = String::new();
        let mut slots: Vec<String> = Vec::new();
        let mut disk_trimmed = 0usize;
        // Local slots the load governor closed this tick (machine saturated):
        // like disk_trimmed, not offered to the queue.
        let mut load_held = 0usize;
        // Whether the one slot left open is the owner's express slot: the
        // fleet's queued work could not have had it.
        let mut express_slot = false;
        // A panic in the tick body becomes the idle reason, not the daemon's end.
        let tick = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if now[..10.min(now.len())] != last_prune_day {
                match crate::board::prune_agents(
                    s,
                    now.clone(),
                    crate::util::setting_int(s, "prune_after_days"),
                    actor.clone(),
                    false,
                ) {
                    Ok(pruned) => {
                        for pr in pruned {
                            say(&o, format!("pruned {}: {}", pr.name, pr.reason));
                        }
                    }
                    Err(e) => say(
                        &o,
                        format!("prune skipped: {}", crate::run::err_message(&e)),
                    ),
                }
                last_prune_day = now[..10.min(now.len())].to_string();
            }
            if !o.dry_run {
                let inflight_runs = inflight
                    .values()
                    .map(|d| d.run.str("id"))
                    .collect::<BTreeSet<_>>();
                let inflight_jobs = inflight
                    .values()
                    .map(|d| d.job.str("id"))
                    .collect::<BTreeSet<_>>();
                for line in
                    crate::liveness::sweep_dead_agents(s, &now, &inflight_runs, &inflight_jobs)
                {
                    say(&o, line);
                }
                // (The owner's lanes are yielded further down, once the
                // restart window says which runs may still start.)
                // A worker is not kept idle: once its run has ended and
                // nothing waits for it (no lease, no held conversation), it
                // retires for good and the next job gets a fresh name. The
                // workers still in flight here, and the one a `--agent`
                // daemon runs as, stay.
                let mut keep = inflight
                    .values()
                    .map(|d| d.worker.clone())
                    .collect::<Vec<_>>();
                if !o.agent.is_empty() {
                    keep.push(actor.clone());
                }
                match crate::board::retire_idle_workers(s, &now, &actor, &keep) {
                    Ok(retired) => {
                        for name in retired {
                            say(
                                &o,
                                format!("retired {name}: {}", pulse_board::RETIRE_REASON),
                            );
                        }
                    }
                    Err(e) => say(
                        &o,
                        format!("retire skipped: {}", crate::run::err_message(&e)),
                    ),
                }
            }
            let conn = match s.conn() {
                Ok(c) => Some(c),
                Err(e) => {
                    warn_store_unavailable(&o, &e);
                    None
                }
            };
            stages.mark("reap");
            // The planning pass; a tick inside `daemon.pass_seconds` skips to
            // the slots below.
            if full_pass {
                let all_projects = conn
                    .as_ref()
                    .map(|c| crate::board::project_rows(c, false))
                    .unwrap_or_default();
                let (projects, _, hold_lines) =
                    probes.ready(all_projects, crate::checkout::CHECKOUT_PROBE_TIMEOUT);
                for line in hold_lines {
                    say(&o, line);
                }
                stages.mark("probes");
                if maintenance.done.is_none()
                    && (!only_maintenance_wake || maintenance.has_new_pending(s, projects.clone()))
                {
                    let mut binary = self_env.clone();
                    binary.inflight = inflight.len() as i32;
                    binary.local_inflight = inflight
                        .values()
                        .filter(|d| d.slot == crate::executor::LOCAL_EXECUTOR)
                        .count() as i32;
                    let limits = crate::daemon_load::limits(s, &machine, o.max_jobs);
                    // A changed daemon.build_jobs reaches the next child here.
                    crate::daemon_load::set_build_jobs(limits.build_jobs);
                    binary.governor = Some(limits);
                    binary.refused_queued = crate::restart::fits_phrase(
                        &crate::restart::refused_queued_work(s, &inflight, &now),
                    );
                    let inflight_jobs = inflight
                        .values()
                        .map(|d| (d.job.str("id"), true))
                        .collect::<BTreeMap<_, _>>();
                    maintenance.start(
                        &maintenance_ctx,
                        s,
                        projects.clone(),
                        actor.clone(),
                        now.clone(),
                        o.dry_run,
                        inflight_jobs,
                        binary,
                    );
                }
                stages.mark("maintenance");
                for line in crate::auto_adopt::auto_adopt_drafts(s, now.clone(), actor.clone()) {
                    say(&o, line);
                }
                stages.mark("adopt");
                let slugs = projects.iter().map(|p| p.str("slug")).collect::<Vec<_>>();
                let environment_now = crate::recover::live_environment(s);
                for project in &projects {
                    let slug = project.str("slug");
                    for line in crate::amend::auto_apply_amendments(
                        s,
                        slug.clone(),
                        now.clone(),
                        &mut refused,
                    ) {
                        say(&o, line);
                    }
                    stages.mark("amend");
                    for line in crate::manifest::tend_manifest(s, &slug, &now, &actor) {
                        say(&o, line);
                    }
                    stages.mark("manifest");
                    for line in crate::lead::tend_lead(s, slug.clone(), now.clone(), actor.clone())
                    {
                        say(&o, line);
                    }
                    for line in crate::lead_roles::tend_role_leads(s, &slug, &now, &actor) {
                        say(&o, line);
                    }
                    for line in crate::lead_roles::suggest_leads(s, &slug, &now, &actor) {
                        say(&o, line);
                    }
                    for line in crate::lead_roles::seat_decided_leads(s, &slug, &now, &actor) {
                        say(&o, line);
                    }
                    stages.mark("leads");
                    for line in crate::toolkit::suggest_toolkit(s, &slug, &now, &actor) {
                        say(&o, line);
                    }
                    for line in crate::toolkit::add_decided_toolkit(s, &slug, &now, &actor) {
                        say(&o, line);
                    }
                    stages.mark("toolkit");
                    for line in crate::lead::tend_tasks(s, slug.clone(), now.clone(), actor.clone())
                    {
                        say(&o, line);
                    }
                    stages.mark("tasks");
                    for line in crate::suggestions::tend_suggestions(s, &slug, &now, &actor) {
                        say(&o, line);
                    }
                    stages.mark("suggestions");
                    for line in crate::recover::recover_failed_jobs(
                        s,
                        project.clone(),
                        now.clone(),
                        actor.clone(),
                    ) {
                        say(&o, line);
                    }
                    for line in crate::recover::release_healed_environment_blocks(
                        s,
                        slug.clone(),
                        now.clone(),
                        actor.clone(),
                        &environment_now,
                    ) {
                        say(&o, line);
                    }
                    stages.mark("recover");
                    for line in crate::security::tend_dependency_alerts(s, &slug, &now, &actor) {
                        say(&o, line);
                    }
                    stages.mark("security");
                    for line in crate::pipeline::tend_pipeline(s, &slug, &now, &actor) {
                        say(&o, line);
                    }
                    stages.mark("pipeline");
                    for line in crate::lessons::tend_lessons(s, slug, now.clone(), actor.clone()) {
                        say(&o, line);
                    }
                    stages.mark("lessons");
                }
                if !o.dry_run {
                    for line in crate::evolve_plan::tend_evolve(
                        s,
                        now.clone(),
                        actor.clone(),
                        slugs.clone(),
                    ) {
                        say(&o, line);
                    }
                    stages.mark("evolve");
                    if crate::run::runner_stopped(s).is_empty() {
                        let mut scripts_held = false;
                        // When the bulk volume is away, leave every due schedule
                        // untouched so it still falls due on the first pass after
                        // the disk is back, instead of running with a dangling
                        // work dir and moving next_at past the missed tick. The
                        // hold note rides the "schedules" idle-log key, so it
                        // logs once per away spell, like the load hold.
                        if let crate::bulk::State::Missing { target, .. } =
                            crate::bulk::state(std::path::Path::new(&s.home))
                        {
                            let line = crate::bulk::hold_note(&target);
                            // Keep the slot set across passes (skip the clear
                            // below) so the note logs once per away spell; it
                            // clears on the first pass that runs scripts again.
                            scripts_held = true;
                            if let Some(line) =
                                idle_log.line("schedules", &line, std::time::Instant::now())
                            {
                                say(&o, line);
                            }
                        } else {
                            for line in crate::schedule::tend_schedules(
                                s,
                                now.clone(),
                                actor.clone(),
                                &slugs,
                            ) {
                                // A hold on the check and watch scripts stands for
                                // as long as the machine is saturated; it logs
                                // once per change, like the load hold.
                                if crate::schedule_script::is_hold_line(&line) {
                                    scripts_held = true;
                                    if let Some(line) =
                                        idle_log.line("schedules", &line, std::time::Instant::now())
                                    {
                                        say(&o, line);
                                    }
                                } else {
                                    say(&o, line);
                                }
                            }
                        }
                        if !scripts_held {
                            idle_log.clear("schedules");
                        }
                    }
                    stages.mark("schedules");
                }
                let mut planned = BTreeMap::new();
                for slug in &slugs {
                    let before_plan = crate::plan::last_event_seq(s);
                    let snapshot = planner_snapshot(s, &mut planner_cache, slug, &now);
                    let jobs = match crate::plan::plan_jobs_with_tally(
                        s,
                        slug.clone(),
                        now.clone(),
                        actor.clone(),
                        snapshot.as_ref().map(|snap| &snap.tally),
                    ) {
                        Ok(jobs) => jobs,
                        Err(e) => {
                            say(
                                &o,
                                format!("planning {slug}: {}", crate::run::err_message(&e)),
                            );
                            Vec::new()
                        }
                    };
                    for l in crate::plan::plan_deferred_lines_since(s, before_plan) {
                        say(&o, l);
                    }
                    planned.insert(slug.clone(), jobs.len() as i32);
                    for job in jobs {
                        say(
                            &o,
                            format!(
                                "planned {} {}: {}",
                                job.str("kind"),
                                job.str("id"),
                                job.str("title")
                            ),
                        );
                    }
                }
                stages.mark("plan");
                let mut cur_idle = BTreeMap::new();
                if let Some(c) = conn.as_ref() {
                    for slug in &slugs {
                        if *planned.get(slug).unwrap_or(&0) != 0 {
                            continue;
                        }
                        let busy = crate::util::must_one(
                            c,
                            "SELECT 1 AS x FROM jobs WHERE project = ? AND status IN ('queued', 'leased') LIMIT 1",
                            &[slug],
                        );
                        if !busy.0.is_empty() {
                            continue;
                        }
                        let note = if let Some(snapshot) =
                            planner_snapshot(s, &mut planner_cache, slug, &now)
                        {
                            crate::idle::idle_note_from_parts(
                                s,
                                slug.clone(),
                                now.clone(),
                                snapshot.suggestion_idle,
                                snapshot.tally.suggestions,
                                snapshot.tally.amendments,
                            )
                        } else {
                            crate::idle::idle_note(s, slug.clone(), now.clone())
                        };
                        cur_idle.insert(slug.clone(), note);
                    }
                }
                for line in crate::idle::idle_notes_changed(prev_idle.clone(), cur_idle.clone()) {
                    say(&o, line);
                }
                crate::idle::record_idle_notes(s, cur_idle.clone(), now.clone());
                prev_idle = cur_idle;
                stages.mark("idle");
                for line in crate::heldout::tend_heldout(s, now.clone()) {
                    say(&o, line);
                }
                stages.mark("heldout");
                for pr in crate::keys::reprobe_due(s, interval, &now) {
                    say(
                        &o,
                        format!("executor {} is reachable again: {}", pr[0], pr[1]),
                    );
                }
                for line in crate::retire::tend_executors(s, now.clone(), actor.clone()) {
                    say(&o, line);
                }
                stages.mark("executors");
            }
            for line in compute.tick(&compute_ctx, &now) {
                say(&o, line);
            }
            stages.mark("compute");
            let mut busy = BTreeMap::new();
            for d in inflight.values() {
                *busy.entry(d.slot.clone()).or_insert(0) += 1;
            }
            // The login gate: with the native harness and no sign-in here, this machine's slots stay
            // closed rather than launching workers that die at once (see `login_gate`).
            let gate = (crate::login_gate::LOGIN_GATE
                .read()
                .expect("login gate poisoned"))(s);
            slots = crate::run::executor_slots(s, &now, &busy, !gate.is_empty());
            // The restart window: all kinds of run still fit it (no hold),
            // only the short ones and the owner's work do (the line names
            // them and the slots lease nothing else), or the owner's work
            // alone (`restart_fits`). A console drain closes the slots.
            let (restart_hold, restart_kinds) = restart.hold(s, &now, &inflight);
            // The owner's work goes first in its project (yield_lane): a fleet
            // run in the way of a queued owner job is asked to hold in any
            // restart window, since the owner's work starts in any - but not
            // during a console drain, which starts nothing.
            let restart_starts_nothing = restart_hold.is_some() && restart_kinds.is_empty();
            if !o.dry_run && !restart_starts_nothing {
                for line in crate::yield_lane::yield_lanes_to_owner(s, &now, &actor, &restart_kinds)
                {
                    say(&o, line);
                }
            }
            let disk_floor = crate::executor::disk_floor_hold(s);
            // The console's daemon line: empty while the disk pays for runs,
            // otherwise the hold in the copy the Overview prints in red.
            let mut disk_line = String::new();
            // Bulk storage on another disk (bulk.rs): while the volume is
            // away nothing starts, and the hold names the disk, so nobody
            // frees space for a floor that is not the problem.
            let (bulk_state, bulk_lines) = crate::bulk::tend(std::path::Path::new(&s.home));
            for line in bulk_lines {
                say(&o, line);
            }
            let bulk_hold = match &bulk_state {
                crate::bulk::State::Missing { target, .. } => Some(crate::bulk::hold_note(target)),
                _ => None,
            };
            if bulk_hold.is_none() && bulk_was_away {
                say(&o, "bulk storage back; resuming");
            }
            bulk_was_away = bulk_hold.is_some();
            let bulk_line = bulk_hold.clone().unwrap_or_default();
            if let Some(hold) = &restart_hold
                && let Some(line) = idle_log.line("restart", hold, std::time::Instant::now())
            {
                say(&o, line);
            }
            // Only a console drain leaves the window with nothing to start
            // (restart_fits); a binary change keeps the slots open to the
            // owner's work at least.
            if let Some(hold) = restart_hold.clone().filter(|_| restart_kinds.is_empty()) {
                reason = hold;
                slots.clear();
            } else if let Some(hold) = bulk_hold {
                // The floor's clock stops too: a 0 MB reading from a dangling
                // link is the disk's absence, not a floor to ask the owner about.
                disk_floor_was_under = false;
                disk_floor_since.clear();
                disk_floor_posted = false;
                starved_since.clear();
                reason = hold;
                slots.clear();
            } else if let Some((_, _, _)) = disk_floor {
                disk_floor_was_under = true;
                if disk_floor_since.is_empty() {
                    disk_floor_since = now.clone();
                    disk_floor_posted = false;
                    // The maintenance pass answers the crossing with the
                    // reclaim ladder (reclaim.rs) before anyone asks the
                    // owner: a pause that clears itself needs no owner.
                    say(
                        &o,
                        "disk floor crossed: the reclaim ladder runs on the next maintenance pass",
                    );
                }
                starved_since.clear();
                reason = crate::executor::disk_floor_note(s).unwrap_or_default();
                if let Some((free, floor, _)) = &disk_floor {
                    disk_line = crate::diskroom::console_line(*free, *floor, None);
                }
                slots.clear();
            } else {
                if disk_floor_was_under {
                    let free = crate::executor::free_mb(&crate::executor::local_work_root(s));
                    say(&o, format!("disk floor cleared: {free} MB free"));
                }
                disk_floor_was_under = false;
                disk_floor_since.clear();
                disk_floor_posted = false;
                // The floor alone cannot see that ten setups about to start
                // each need a gigabyte: a local slot takes only a project
                // whose footprint the headroom still pays for (diskroom.rs).
                let mut admission = None;
                if slots.iter().any(|n| n == crate::executor::LOCAL_EXECUTOR) {
                    let inflight_local = inflight
                        .values()
                        .filter(|d| d.slot == crate::executor::LOCAL_EXECUTOR)
                        .map(|d| (d.job.str("id"), d.job.str("project"), d.started_at.clone()))
                        .collect::<Vec<_>>();
                    admission = crate::diskroom::local_headroom(s, &inflight_local)
                        .map(crate::diskroom::Admission::new);
                }
                // The load governor (daemon_load.rs): the local slots are capped
                // at the machine's capacity and all close while the machine is saturated.
                // One slot stays free while a self-update build waits for room.
                let mut load_hold: Option<String> = None;
                // The last half of the local slots under the machine's capacity
                // (rounded up) go to the work class first (jobs.rs
                // work_first): with four slots and a queue of reflects and
                // ballots for every project, the fairness between projects
                // kept an improve job waiting all day.
                let mut work_reserve = 0usize;
                // While a soft hold (load, or the kernel's memory warning
                // with free memory above the floor) closes the machine, or
                // the fleet's own work fills the machine, one local slot
                // stays open to the owner's work (jobs.rs OWNER_DRIVEN_SQL):
                // the owner should not wait behind the fleet's own builds.
                if slots.iter().any(|n| n == crate::executor::LOCAL_EXECUTOR) {
                    let local = inflight
                        .values()
                        .filter(|d| d.slot == crate::executor::LOCAL_EXECUTOR)
                        .collect::<Vec<_>>();
                    let running_local = local.len();
                    let owner_running = local
                        .iter()
                        .filter(|d| crate::jobs::is_owner_driven(&d.job))
                        .count();
                    let reserved = usize::from(crate::selfupdate::update_pending());
                    let limits = crate::daemon_load::limits(s, &machine, o.max_jobs);
                    work_reserve = (limits.max_jobs.max(0) as usize).div_ceil(2);
                    let verdict = crate::daemon_load::decide(
                        &limits,
                        &crate::daemon_load::pressure(),
                        running_local,
                        crate::daemon_load::measuring(&s.home),
                        reserved,
                        owner_running,
                    );
                    if let Some(hold) = verdict.hold {
                        // Inside a restart window too: the owner's work
                        // starts in any window, and its run anchors the
                        // window rather than outlast it (anchors_restart).
                        if verdict.express {
                            // The slot kept is restricted, so the tick still
                            // counts as held: an empty lane is not starvation.
                            load_held = crate::daemon_load::trim_local_slots(&mut slots, 1).max(1);
                            express_slot = true;
                            load_hold = Some(format!(
                                "{hold}; one express slot stays open for the owner's work"
                            ));
                        } else {
                            load_held = crate::daemon_load::trim_local_slots(&mut slots, 0);
                            load_hold = Some(hold);
                        }
                    } else if verdict.express {
                        // The cap is full of the fleet's work and none of it
                        // is the owner's: one slot over the capacity takes
                        // owner-driven jobs alone, so an answer to the owner
                        // does not wait out two verifies - nor a restart
                        // window, which the owner's run anchors.
                        load_held = crate::daemon_load::trim_local_slots(&mut slots, 1).max(1);
                        express_slot = true;
                        load_hold = Some(
                            "every slot of this machine is busy with the fleet's work; one express slot stays open for the owner's work"
                                .to_string(),
                        );
                    } else {
                        // Over the cap is capacity the owner chose not to use, not
                        // a hold: those slots are simply not there this tick.
                        crate::daemon_load::trim_local_slots(&mut slots, verdict.room);
                    }
                }
                match &load_hold {
                    Some(hold) => {
                        if let Some(line) = idle_log.line("load", hold, std::time::Instant::now()) {
                            say(&o, line);
                        }
                    }
                    None => idle_log.clear("load"),
                }
                let mut disk_room_note = String::new();
                reason = if slots.is_empty() {
                    if let Some(hold) = load_hold.clone() {
                        hold
                    } else if gate.is_empty() {
                        "every executor slot is busy".to_string()
                    } else {
                        gate.clone()
                    }
                } else {
                    let mut names = inflight
                        .values()
                        .map(|d| d.worker.clone())
                        .collect::<Vec<_>>();
                    let mut reasons = BTreeMap::new();
                    if let Some(hold) = &restart_hold {
                        reasons.insert(hold.clone(), true);
                    }
                    if let Some(hold) = &load_hold {
                        reasons.insert(hold.clone(), true);
                    }
                    let mut rows = BTreeMap::new();
                    for name in &slots {
                        rows.entry(name.clone())
                            .or_insert_with(|| crate::executor::executor_row(s, name));
                    }
                    let order = slot_order(&slots, &rows);
                    let owner_lane = crate::jobs::owner_lane_fits();
                    let local_free = slots
                        .iter()
                        .filter(|n| *n == crate::executor::LOCAL_EXECUTOR)
                        .count();
                    let mut local_leased = 0usize;
                    for i in order {
                        let slot = &slots[i];
                        let fits = if express_slot && slot == crate::executor::LOCAL_EXECUTOR {
                            &owner_lane
                        } else {
                            &restart_kinds
                        };
                        // This and the local slots still to fill this tick
                        // are the last of the cap: the work class goes first.
                        let work_first = slot == crate::executor::LOCAL_EXECUTOR
                            && local_free - local_leased <= work_reserve;
                        let mut held = BTreeMap::new();
                        if slot == crate::executor::LOCAL_EXECUTOR
                            && let Some(adm) = admission.as_ref()
                        {
                            match adm.offer() {
                                Some(h) => held = h,
                                None => {
                                    disk_trimmed += 1;
                                    disk_room_note = adm.note();
                                    reasons.insert(disk_room_note.clone(), true);
                                    continue;
                                }
                            }
                        }
                        let (lead, job, why) = crate::lead::lead_job_for(
                            s,
                            &names,
                            o.dry_run,
                            now.clone(),
                            slot.clone(),
                            started.len() as i32,
                            &held,
                            fits,
                        );
                        let (worker, job) = if !job.0.is_empty() {
                            (lead, job)
                        } else {
                            if !why.is_empty() {
                                reasons.insert(why, true);
                            }
                            let worker = match crate::run::pick_worker(s, &now, &o.agent, &names) {
                                Ok(worker) => worker,
                                Err(e) => {
                                    reasons.insert(crate::run::err_message(&e), true);
                                    continue;
                                }
                            };
                            let (job, _, why) = match crate::run::take_job_first(
                                s,
                                &worker,
                                "",
                                "",
                                o.dry_run,
                                &now,
                                slot,
                                started.len() as i32,
                                &held,
                                fits,
                                work_first,
                            ) {
                                Ok(v) => v,
                                Err(e) => {
                                    reasons.insert(crate::run::err_message(&e), true);
                                    continue;
                                }
                            };
                            if job.0.is_empty() {
                                if !why.is_empty() {
                                    reasons.insert(why, true);
                                }
                                continue;
                            }
                            (worker, job)
                        };
                        if slot == crate::executor::LOCAL_EXECUTOR {
                            local_leased += 1;
                            if let Some(adm) = admission.as_mut() {
                                adm.claim(&job.str("project"));
                            }
                        }
                        names.push(worker.clone());
                        let wait_s = crate::jobs::job_wait_seconds(
                            &*s.conn().expect("lease wait connection"),
                            &job,
                            &now,
                        );
                        say(&o, format!("leased {} after {}s", job.str("id"), wait_s));
                        let d = Dispatch {
                            slot: slot.clone(),
                            worker,
                            job,
                            started_at: now.clone(),
                            ..Default::default()
                        };
                        let key = dispatch_key(&d.slot, &d.worker, &d.job);
                        // The owner's run anchors a pending restart: the
                        // window follows it to its expected end, and the
                        // kinds that fit keep starting while it runs.
                        restart.anchor(&key, &d.job);
                        inflight.insert(key, d.clone());
                        started.push(d.clone());
                        let tx = results_tx.clone();
                        let s_run = s.clone();
                        let o_run = o.clone();
                        let now_run = now.clone();
                        let actor_run = actor.clone();
                        let run_parent = run_ctx.child();
                        std::thread::Builder::new()
                            .name("daemon-run".into())
                            .spawn(move || {
                                let mut d = d;
                                let result =
                                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                        let dctx = run_parent.with_timeout(
                                            crate::baseline::job_budget(&s_run, d.job.clone()),
                                        );
                                        match crate::run::run_job(
                                            &s_run,
                                            d.job.clone(),
                                            &d.worker,
                                            &now_run,
                                            crate::run::RunOpts {
                                                dry_run: o_run.dry_run,
                                                worker_cmd: o_run.worker_cmd.clone(),
                                                executor: d.slot.clone(),
                                                cancel: dctx,
                                            },
                                        ) {
                                            Ok(run) => {
                                                if !o_run.dry_run
                                                    && let Some(line) = requeue_outage_experiment(
                                                        &s_run, &run, &actor_run,
                                                    )
                                                {
                                                    say(&o_run, line);
                                                }
                                                d.run = run;
                                            }
                                            Err(e) => {
                                                d.reason = format!(
                                                    "daemon error on {}: {}",
                                                    d.slot,
                                                    crate::run::err_message(&e)
                                                );
                                            }
                                        }
                                    }));
                                if let Err(panic) = result {
                                    let msg = panic
                                        .downcast_ref::<&str>()
                                        .map(|s| (*s).to_string())
                                        .or_else(|| panic.downcast_ref::<String>().cloned())
                                        .unwrap_or_else(|| "unknown panic".into());
                                    d.reason = format!("daemon error on {}: {}", d.slot, msg);
                                }
                                let _ = tx.send(d);
                            })
                            .expect("spawn daemon run");
                    }
                    let mut reason = crate::proc::sorted_keys(&bool_keys(reasons)).join("; ");
                    if !disk_room_note.is_empty()
                        && let Some(adm) = admission.as_ref()
                    {
                        disk_line = adm.console_line();
                    }
                    if reason.is_empty() && started.is_empty() {
                        reason = "no queued job".into();
                    }
                    reason
                };
                if disk_trimmed > 0 {
                    if let Some(line) =
                        idle_log.line("diskroom", &disk_room_note, std::time::Instant::now())
                    {
                        say(&o, line);
                    }
                } else {
                    idle_log.clear("diskroom");
                }
            }
            let queued = active_queued_count(s);
            // Slots the disk headroom or the load governor held back were not
            // offered this tick; the owner's express slot, unused, was not
            // offered to the fleet's work either.
            let offered = slots.len() - disk_trimmed;
            let total_slots = inflight.len() as i32 + offered as i32 - started.len() as i32;
            let usable = offered - usize::from(express_slot && started.is_empty());
            // Starved: work queued and nothing could start, whether every
            // slot is busy or the governor or the disk held them. Why this
            // machine is short does not change what would help - hands
            // elsewhere - so a held machine counts as short like a busy one
            // (before this a laptop held by its load all day never asked).
            let short = !inflight.is_empty() || load_held > 0 || disk_trimmed > 0;
            let starved = queued > 0 && usable == started.len() && short;
            if starved {
                if starved_since.is_empty() {
                    starved_since = now.clone();
                }
            } else {
                starved_since.clear();
            }
            capacity_note(
                s,
                queued,
                total_slots,
                inflight.len() as i32,
                starved_since.clone(),
                gate.clone(),
                disk_line,
                bulk_line,
                now.clone(),
            );
            // Work that needs a kind of machine none of the paired ones is
            // waits whatever the slots do; the owner hears once per project
            // and needs, after the job has waited long enough not to be a
            // manifest the owner is still editing.
            if queued > 0
                && let Some(c) = conn.as_ref()
            {
                let unmet: Vec<UnmetNeeds> = unmet_needs(s, &now)
                    .into_iter()
                    .filter(|u| u.minutes >= NEEDS_WANTED_MINUTES)
                    .collect();
                if !unmet.is_empty() {
                    let line = unmet
                        .iter()
                        .map(|u| format!("{} job(s) of {} need {}", u.jobs, u.project, u.needs))
                        .collect::<Vec<_>>()
                        .join("; ");
                    if let Some(line) = idle_log.line(
                        "needs",
                        &format!("machine wanted: {line}; no active machine offers that"),
                        std::time::Instant::now(),
                    ) {
                        say(&o, line);
                    }
                    for post in needs_wanted(
                        s,
                        crate::board::brain_project(c),
                        &unmet,
                        now.clone(),
                        actor.clone(),
                    ) {
                        say(&o, format!("machine wanted: {}", post.str("summary")));
                    }
                } else {
                    idle_log.clear("needs");
                }
            }
            if !gate.is_empty()
                && let Some(c) = conn.as_ref()
            {
                let post = crate::login_gate::login_wanted(
                    s,
                    crate::board::brain_project(c),
                    queued,
                    gate.clone(),
                    now.clone(),
                    actor.clone(),
                );
                if !post.0.is_empty() {
                    say(&o, post.str("summary"));
                }
            }
            // The daily cap: the owner hears the first tick it holds queued
            // work, on the board and in the log without the hourly dedupe,
            // and hears again when it lifts (daily_cap.rs).
            if !o.dry_run
                && let Some(c) = conn.as_ref()
            {
                let cap = crate::run::daily_cap(s);
                let done = crate::run::runs_today(s, &now);
                let holding = cap > 0 && done >= cap && queued > 0;
                if holding && cap_was_holding != Some(true) {
                    let post = crate::daily_cap::post_cap_reached(
                        s,
                        crate::board::brain_project(c),
                        done,
                        cap,
                        queued,
                        now.clone(),
                        actor.clone(),
                    );
                    say(
                        &o,
                        format!(
                            "{}; {} job(s) wait; set jobs_per_day higher or to 0 to let them start{}",
                            crate::run::daily_cap_reason(done, cap),
                            queued,
                            if post.0.is_empty() {
                                String::new()
                            } else {
                                format!(" (asked the owner: {})", post.str("id"))
                            }
                        ),
                    );
                } else if !holding && cap_was_holding != Some(false) {
                    let why = crate::daily_cap::lift_reason(s, &now);
                    let closed = crate::daily_cap::close_cap_notices(
                        s,
                        &crate::board::brain_project(c),
                        &why,
                        &now,
                        &actor,
                    );
                    if closed > 0 {
                        say(
                            &o,
                            format!("daily cap lifted: {why}; {closed} owner question(s) answered"),
                        );
                    }
                }
                cap_was_holding = Some(holding);
            }
            if !disk_floor_since.is_empty()
                && !disk_floor_posted
                && let Some(c) = conn.as_ref()
            {
                let waited = (crate::util::must_iso(&now)
                    - crate::util::must_iso(&disk_floor_since))
                .num_minutes() as i32;
                let debounced = !disk_floor_last_post.is_empty()
                    && (crate::util::must_iso(&now) - crate::util::must_iso(&disk_floor_last_post))
                        .num_minutes()
                        < i64::from(DISK_FLOOR_REPOST_MINUTES);
                if waited >= DISK_FLOOR_WANTED_MINUTES && debounced {
                    // Same transient pressure, already asked: no duplicate post.
                    disk_floor_posted = true;
                } else if waited >= DISK_FLOOR_WANTED_MINUTES
                    && let Some((free, floor, path)) = crate::executor::disk_floor_hold(s)
                {
                    let post = disk_floor_wanted(
                        s,
                        crate::board::brain_project(c),
                        path,
                        free,
                        floor,
                        waited,
                        now.clone(),
                        actor.clone(),
                    );
                    disk_floor_posted = true;
                    if !post.0.is_empty() {
                        disk_floor_last_post = now.clone();
                        say(&o, format!("disk floor wanted: {}", post.str("summary")));
                    }
                }
            }
            if !starved_since.is_empty()
                && let Some(c) = conn.as_ref()
            {
                let waited = (crate::util::must_iso(&now) - crate::util::must_iso(&starved_since))
                    .num_minutes() as i32;
                if waited >= HANDS_WANTED_MINUTES {
                    // A machine the owner pre-authorised first; the question
                    // to the owner only when none may be rented.
                    match auto_compute(s, queued, waited, &now, &actor) {
                        Ok(req) => say(
                            &o,
                            format!(
                                "hands wanted: compute request {} ({}, {} for {} h) made for {}: {}",
                                req.id, req.status, req.size, req.hours, req.project, req.reason
                            ),
                        ),
                        // A machine is on its way: no question to the owner.
                        Err(NoAutoCompute::Open { id, status }) if status != "pending" => {
                            if let Some(line) = idle_log.line(
                                "hands",
                                &format!("hands wanted: compute request {id} is {status}; waiting for the machine"),
                                std::time::Instant::now(),
                            ) {
                                say(&o, line);
                            }
                        }
                        Err(why) => {
                            let post = hands_wanted(
                                s,
                                crate::board::brain_project(c),
                                queued,
                                total_slots,
                                waited,
                                now.clone(),
                                actor.clone(),
                            );
                            if !post.0.is_empty() {
                                say(
                                    &o,
                                    format!("hands wanted: {} ({why})", post.str("summary")),
                                );
                            }
                        }
                    }
                }
            }
        }));
        if let Err(panic) = tick {
            reason = format!("daemon error: {}", panic_message(&panic));
        }
        for line in compute.drain() {
            say(&o, line);
        }
        // One guard covers both idle shapes: nothing started or in flight, or fewer starts than slots.
        // The idle log says a reason when it changes and summarises it hourly
        // while it holds; a started run opens the channel again.
        if !started.is_empty() {
            idle_log.clear("daemon");
        }
        let idle_now = !reason.is_empty()
            && ((started.is_empty() && inflight.is_empty())
                || (!started.is_empty() && started.len() < slots.len()));
        if let Some(line) = idle_now
            .then(|| idle_log.line("daemon", &reason, std::time::Instant::now()))
            .flatten()
        {
            say(&o, format!("idle: {line}"));
        }
        stages.mark("dispatch");
        if let Some(line) =
            tick_timing.record(loops, tick_began.elapsed(), tick_bound, full_pass, &stages)
        {
            say(&o, line);
        }
        if o.max_loops > 0 && loops >= o.max_loops {
            leave = ("done", "loop limit reached");
            break;
        }
        let tick = crossbeam_channel::after(Duration::from_secs(DISPATCH_TICK_SECONDS as u64));
        let done = cancel.done();
        if !inflight.is_empty() {
            if o.max_loops > 0 && inflight.len() == 1 {
                crossbeam_channel::select! {
                    recv(results_rx) -> msg => if let Ok(d) = msg {
                        report(&o, &mut inflight, &mut completed, &mut all_errors, d);
                    },
                    recv(compute_wake) -> _ => {},
                    recv(wake_events.as_ref().unwrap_or(&never_wake)) -> msg => {
                        if msg.is_err() {
                            wake_events = None;
                        }
                    },
                    recv(done) -> _ => {
                        leave = ("interrupted", "stopping on signal");
                        break;
                    },
                }
            } else {
                crossbeam_channel::select! {
                    recv(results_rx) -> msg => if let Ok(d) = msg {
                        report(&o, &mut inflight, &mut completed, &mut all_errors, d);
                    },
                    recv(tick) -> _ => {},
                    recv(compute_wake) -> _ => {},
                    recv(wake_events.as_ref().unwrap_or(&never_wake)) -> msg => {
                        if msg.is_err() {
                            wake_events = None;
                        }
                    },
                    recv(done) -> _ => {
                        leave = ("interrupted", "stopping on signal");
                        break;
                    },
                }
            }
        } else if let Some(done_rx) = maintenance.done.clone() {
            // The pass channel carries the pass's result; the wake must keep
            // it, or the pass stays "running" for good and nothing lands
            // until the next daemon start.
            crossbeam_channel::select! {
                recv(done_rx) -> msg => {
                    maintenance.pass_done(msg);
                    maintenance_wake = true;
                },
                recv(tick) -> _ => {},
                recv(compute_wake) -> _ => {},
                recv(wake_events.as_ref().unwrap_or(&never_wake)) -> msg => {
                    if msg.is_err() {
                        wake_events = None;
                    }
                },
                recv(done) -> _ => {
                    leave = ("interrupted", "stopping on signal");
                    break;
                },
            }
        } else {
            // An early wake (a run ended, the bus had news) ran the tick
            // alone: sleep until the planning pass is due, not a whole
            // interval, so what woke the loop is planned for in time.
            let mut wait = Duration::from_secs(interval as u64);
            if !full_pass && let Some(began) = last_pass {
                wait = wait.min(spacing.saturating_sub(began.elapsed()));
            }
            let held = load_held > 0 && active_queued_count(s) > 0;
            if !sleep_idle_wait(&mut wake_events, wait, held) {
                leave = ("interrupted", "stopping on signal");
                break;
            }
        }
    }
    let (mut result, note) = leave;
    loop_ctx.cancel();
    // From here the daemon waits to go; each wait is named to the watchdog,
    // which restarts a daemon wedged in one (exit 3) as it does a loop that
    // stops beating.
    let enter = |phase: String| {
        if let Some(dog) = &dog {
            dog.enter(phase);
        }
    };
    let grace_for = |result: &str| -> Duration {
        if result != "interrupted" && result != "stopped" {
            return Duration::ZERO;
        }
        if o.grace > Duration::ZERO {
            return o.grace;
        }
        Duration::from_secs(((s.setting_int("minutes_per_job") + 5).max(0) as u64) * 60)
    };
    let mut grace = grace_for(result);
    if restart.forced() && result == "restart" {
        grace = hold_cap(s);
    }
    if matches!(result, "interrupted" | "stopped") {
        maintenance_ctx.cancel();
    }
    if !inflight.is_empty() {
        say(
            &o,
            format!(
                "{note}: waiting for {} run(s) in flight to finish",
                inflight.len()
            ),
        );
        // Results are reported as they arrive; the caller's cancellation
        // starts the interrupted grace when none is running; when the grace
        // ends the runs are killed and still awaited.
        let never_tick = crossbeam_channel::never::<std::time::Instant>();
        let never_done = crossbeam_channel::never::<()>();
        let mut timer = (grace > Duration::ZERO).then(|| crossbeam_channel::after(grace));
        let mut caller_done = Some(cancel.done());
        // The wait can outlast the heartbeat's freshness (a brain run takes
        // an hour): beat on, naming the runs, so the console does not call a
        // daemon that waits for its runs silent.
        let beat_tick = crossbeam_channel::tick(Duration::from_secs(interval.max(1) as u64));
        let wait_beat = |inflight: &BTreeMap<String, Dispatch>| {
            heartbeat(
                loops,
                passes,
                &format!(
                    "{note}: waiting for {} run(s) in flight to finish",
                    inflight.len()
                ),
                (result == "restart").then(|| restart.beat(inflight, false)),
                tick_timing.beat(),
            );
        };
        wait_beat(&inflight);
        while !inflight.is_empty() {
            crossbeam_channel::select! {
                recv(results_rx) -> msg => {
                    let Ok(d) = msg else { break };
                    inflight.remove(&dispatch_key(&d.slot, &d.worker, &d.job));
                    if !d.run.0.is_empty() {
                        say(&o, format!("{} {} {} on {}: {}", d.run.str("status"), d.run.str("job_id"), d.run.str("agent"), d.slot, d.run.str("summary")));
                    } else if !d.reason.is_empty() {
                        say(&o, format!("idle: {}", d.reason));
                    }
                },
                recv(beat_tick) -> _ => wait_beat(&inflight),
                recv(caller_done.as_ref().unwrap_or(&never_done)) -> _ => {
                    caller_done = None;
                    if timer.is_none() {
                        grace = grace_for("interrupted");
                        timer = Some(crossbeam_channel::after(grace));
                    }
                },
                recv(timer.as_ref().unwrap_or(&never_tick)) -> _ => {
                    if restart.forced() && result == "restart" {
                        say(&o, format!("hold cap of {} over: stopping {} run(s) still in flight; their jobs return to the queue", pulse_cli::format_duration(grace), inflight.len()));
                    } else {
                        say(&o, format!("grace of {} over: killing {} run(s) still in flight", pulse_cli::format_duration(grace), inflight.len()));
                    }
                    run_ctx.cancel();
                    timer = None;
                },
            }
        }
    }
    // The runs that ended in the drain leave their workers with nothing to
    // do: retire them now rather than at the next daemon's first tick. A
    // holder of held work (a restart's hold) stays for the daemon to come.
    if !o.dry_run {
        let keep = if o.agent.is_empty() {
            Vec::new()
        } else {
            vec![actor.clone()]
        };
        match crate::board::retire_idle_workers(s, &s.now(), &actor, &keep) {
            Ok(retired) => {
                for name in retired {
                    say(
                        &o,
                        format!("retired {name}: {}", pulse_board::RETIRE_REASON),
                    );
                }
            }
            Err(e) => say(
                &o,
                format!("retire skipped: {}", crate::run::err_message(&e)),
            ),
        }
    }
    if maintenance.done.is_some() || maintenance.lane.is_some() {
        let waiting = format!("{note}: waiting for landing maintenance to finish");
        say(&o, &waiting);
        enter(waiting.clone());
        // A restart (the binary changed on disk, or the owner's ask)
        // cancels the experiment under measurement at once - the next daemon
        // measures it again - and gives the plain landings RESTART_DRAIN
        // before cancelling them too, so a restart is never held for a
        // ninety-minute measurement. A restart the owner asked for now gives
        // them the hold cap, like the runs.
        let drain = if stuck_pass {
            Duration::from_secs(30)
        } else if restart.forced() && result == "restart" {
            grace.min(crate::daemon_maintenance::RESTART_DRAIN)
        } else {
            crate::daemon_maintenance::RESTART_DRAIN
        };
        let deadline = (result == "restart").then(|| {
            maintenance.cancel_lane();
            std::time::Instant::now() + drain
        });
        // Once cancelled, what still runs gets this long to stop before the
        // commands it waits in are ended and it is left behind; a restart
        // the owner asked for now gets no more than its hold cap.
        maintenance.cancel_grace = Some(crate::daemon_maintenance::CANCEL_GRACE.min(drain));
        // While maintenance is waited for uncancelled it beats the watchdog
        // and watches for the owner: a restart asked during this wait (the
        // hub's "Restart now", pressed because the daemon seems stuck)
        // cancels the landings at once instead of going unheard. Once
        // cancelled the wait is bounded and the watchdog stands behind it.
        let asks_after = pulse_bus::last_id(s).unwrap_or(0);
        let escalated = std::cell::Cell::new(false);
        let (lines, cut) = maintenance.finish_beating(s, &maintenance_ctx, deadline, &|| {
            enter(waiting.clone());
            if escalated.get() {
                return;
            }
            if let Some(ask) = restart_asked(s, asks_after) {
                escalated.set(true);
                say(
                    &o,
                    format!(
                        "restart asked by {} at {} while landing maintenance drains: cancelling it now; every landing done so far stays landed, the rest run on the next daemon",
                        ask.actor, ask.at
                    ),
                );
                maintenance_ctx.cancel();
            }
        });
        for line in lines {
            say(&o, line);
        }
        if cut {
            say(
                &o,
                format!(
                    "{note}: landing maintenance cancelled after {}",
                    pulse_cli::format_duration(drain)
                ),
            );
        }
    }
    maintenance_ctx.cancel();
    if cancel.is_cancelled() && result != "stopped" {
        result = "interrupted";
        grace = grace_for(result);
    }
    if compute.busy() > 0 {
        say(
            &o,
            format!(
                "{note}: waiting for {} compute operation(s) in flight",
                compute.busy()
            ),
        );
    }
    let computing = format!(
        "{note}: waiting for {} compute operation(s) in flight",
        compute.busy()
    );
    enter(computing.clone());
    compute.wait_beating(grace, &|| enter(computing.clone()));
    compute_ctx.cancel();
    for line in compute.drain() {
        say(&o, line);
    }
    enter(format!(
        "{note}: waiting for the bus bridge and the hooks to stop"
    ));
    let _ = bridge_done.recv();
    let _ = hooks_done_rx.recv();
    let beat = if result == "restart" {
        "restarting"
    } else {
        "exited"
    };
    // The runs in flight are done: the machine may sleep again (a restarted
    // daemon takes its own hold).
    awake.release();
    daemon_heartbeat(
        s,
        loops,
        passes,
        interval,
        beat.to_string(),
        None,
        tick_timing.beat(),
    );
    // The wake subscription outlives the heartbeat, which therefore counts
    // it among the bus subscribers.
    cancel_wake();
    crate::checkout::hold_checkouts(BTreeMap::new());
    result.into()
}

/// Cancels its token when dropped: the daemon's watchdog thread stops on
/// every way out of daemon().
struct CancelOnDrop(pulse_bus::CancellationToken);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

fn panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".into())
}

pub(crate) fn bool_keys(m: BTreeMap<String, bool>) -> BTreeMap<String, String> {
    m.into_keys().map(|k| (k, String::new())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::*;
    use crate::test_helpers_completion::with_closed_login_gate;
    use crate::test_helpers_daemon::{
        exec1, finished_job_of_kind, finished_run, store_with_projects, test_wait,
    };
    use crate::test_helpers_selfupdate::{head_of, self_update_fixture, wait_self_update_idle};
    use crate::util::file_exists;

    #[test]
    fn warn_store_unavailable_explains_empty_project_list() {
        struct Shared(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
        impl std::io::Write for Shared {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let out: DaemonOut = std::sync::Arc::new(std::sync::Mutex::new(Shared(buf.clone())));
        let opts = DaemonOpts {
            out: Some(out),
            ..Default::default()
        };
        warn_store_unavailable(&opts, &anyhow::anyhow!("database is locked"));
        let log = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert!(
            log.contains("store unavailable") && log.contains("database is locked"),
            "log: {log:?}"
        );
    }

    #[test]
    fn meta_upsert_writes_and_reports_failure() {
        let s = new_store();
        meta_upsert(&s, "capacity", &serde_json::json!({"note": "x"}));
        let row = crate::util::must_one(
            &*s.conn().unwrap(),
            "SELECT value FROM meta WHERE key = 'capacity'",
            &[],
        );
        assert!(row.str("value").contains("\"note\":\"x\""));
        s.conn().unwrap().execute("DROP TABLE meta", []).unwrap();
        let err = meta_upsert_result(&s, "daemon", &serde_json::json!({}))
            .expect_err("a missing meta table must surface as an error");
        assert!(err.contains("meta"), "{err}");
    }

    use std::io::Write;
    use std::time::Duration;

    static TEST_RESTART_CHANGED: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);

    fn test_restart_stamp() -> String {
        if TEST_RESTART_CHANGED.load(std::sync::atomic::Ordering::SeqCst) {
            "new-fake-binary".into()
        } else {
            "old-fake-binary".into()
        }
    }

    #[test]
    fn daemon_disk_floor_pauses_dispatch_before_lease() {
        let _serial = crate::test_helpers::serial();
        let _login = with_closed_login_gate();
        // The host's disk does not decide the hold: the dispatch reads 512 MB.
        let _free = crate::executor::pin_free_mb(512);
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "Needs disk".into(),
            "fake worker only".into(),
            0,
            100,
            "test".into(),
            pulse_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        s.set_setting(
            pulse_store::HUMAN_AGENT,
            "daemon.min_free_mb",
            &i32::MAX.to_string(),
        )
        .unwrap();
        let out = once_daemon(
            &s,
            DaemonOpts {
                worker_cmd: "sh -c 'echo SUMMARY: should not run'".into(),
                ..Default::default()
            },
        );
        assert!(
            out.contains("idle: disk under floor: 512 MB free on "),
            "{out}"
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM runs WHERE project = 'proj'",
                &[]
            ),
            0
        );
    }

    #[test]
    fn daemon_holds_all_work_while_the_bulk_volume_is_away() {
        let _serial = crate::test_helpers::serial();
        let _login = with_closed_login_gate();
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "Needs the disk".into(),
            "fake worker only".into(),
            0,
            100,
            "test".into(),
            pulse_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        // The bulk link points at an unplugged volume; `work` goes through it.
        let home = std::path::Path::new(&s.home);
        let gone = std::path::Path::new("/Volumes/NotPluggedIn/pulse");
        std::fs::write(home.join(crate::bulk::ID_FILE), "abc123\n").unwrap();
        std::os::unix::fs::symlink(gone, home.join(crate::bulk::LINK)).unwrap();
        let work = home.join("work");
        let _ = std::fs::remove_dir_all(&work);
        std::os::unix::fs::symlink(std::path::Path::new(crate::bulk::LINK).join("work"), &work)
            .unwrap();
        let out = once_daemon(
            &s,
            DaemonOpts {
                worker_cmd: "sh -c 'echo SUMMARY: should not run'".into(),
                ..Default::default()
            },
        );
        assert!(out.contains("idle: bulk storage away:"), "{out}");
        assert!(
            !out.contains("disk under floor"),
            "the missing disk is the hold, not a floor: {out}"
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM runs WHERE project = 'proj'",
                &[]
            ),
            0
        );
        let note: serde_json::Value = serde_json::from_str(&s.meta("capacity").unwrap().0).unwrap();
        assert!(
            note["bulk"]
                .as_str()
                .unwrap_or_default()
                .contains("/Volumes/NotPluggedIn/pulse"),
            "{note}"
        );
        assert!(
            std::fs::read_link(&work).is_ok() && !work.exists(),
            "nothing was created on the boot disk and the link stands"
        );
    }

    #[test]
    fn daemon_disk_floor_zero_leases_as_before() {
        let _serial = crate::test_helpers::serial();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        // What a failed `df` reads: with the floor off the run still leases.
        let _free = crate::executor::pin_free_mb(0);
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "Runs with floor off".into(),
            "fake worker only".into(),
            0,
            100,
            "test".into(),
            pulse_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        s.set_setting(pulse_store::HUMAN_AGENT, "daemon.min_free_mb", "0")
            .unwrap();
        let out = once_daemon(
            &s,
            DaemonOpts {
                worker_cmd: "sh -c 'echo SUMMARY: ok'".into(),
                ..Default::default()
            },
        );
        assert!(!out.contains("disk under floor"), "{out}");
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM runs WHERE project = 'proj'",
                &[]
            ),
            1
        );
    }

    #[test]
    fn daemon_disk_headroom_admits_one_local_run_and_measures_its_work_dir() {
        let _serial = crate::test_helpers::serial();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        let _free = crate::executor::pin_free_mb(5000);
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        join(&s, "borealis");
        for title in ["First setup", "Second setup"] {
            crate::jobs::add_job(
                &s,
                "proj".into(),
                "task".into(),
                title.into(),
                "fake worker only".into(),
                0,
                100,
                "test".into(),
                pulse_store::HUMAN_AGENT.into(),
                false,
            )
            .unwrap();
        }
        s.set_setting(pulse_store::HUMAN_AGENT, "capacity", "2")
            .unwrap();
        s.set_setting(pulse_store::HUMAN_AGENT, "daemon.min_free_mb", "0")
            .unwrap();
        // A footprint no disk pays for twice: the first run starts because
        // nothing is in flight; the second local slot waits for headroom.
        s.set_setting(pulse_store::HUMAN_AGENT, "daemon.setup_mb", "2000000000")
            .unwrap();
        let out = once_daemon(
            &s,
            DaemonOpts {
                worker_cmd: "sh -c 'echo SUMMARY: ok'".into(),
                ..Default::default()
            },
        );
        assert!(
            out.contains(
                "disk headroom: 0 MB above daemon.min_free_mb 0 with 1 run(s) setting up; waits: proj (2000000000 MB)"
            ),
            "{out}"
        );
        assert!(!out.contains("hands wanted"), "{out}");
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM runs WHERE project = 'proj'",
                &[]
            ),
            1,
            "{out}"
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM jobs WHERE project = 'proj' AND status = 'queued'",
                &[]
            ),
            1,
            "{out}"
        );
        // The clone was measured before it was removed: the next admission
        // of this project uses the real footprint instead of the setting.
        let row = crate::util::must_one(
            &*s.conn().unwrap(),
            "SELECT work_mb, status FROM runs WHERE project = 'proj'",
            &[],
        );
        assert_eq!(row.str("status"), "done", "{out}");
        assert!(
            row.int("work_mb") >= 1,
            "work_mb = {}; {out}",
            row.int("work_mb")
        );
        assert!(
            crate::diskroom::footprint_mb(&*s.conn().unwrap(), "proj", 2_000_000_000) < 1000,
            "{out}"
        );
    }

    #[test]
    fn daemon_load_governor_holds_every_local_slot_while_the_machine_is_saturated() {
        let _serial = crate::test_helpers::serial();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "Waits for room".into(),
            "fake worker only".into(),
            0,
            100,
            "test".into(),
            pulse_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        s.set_setting(pulse_store::HUMAN_AGENT, "capacity", "2")
            .unwrap();
        s.set_setting(pulse_store::HUMAN_AGENT, "daemon.min_free_mb", "0")
            .unwrap();
        let saturated = crate::daemon_load::pin_pressure(crate::daemon_load::Pressure {
            load1: Some(40.2),
            ..crate::daemon_load::CALM
        });
        let out = once_daemon(
            &s,
            DaemonOpts {
                worker_cmd: "sh -c 'echo SUMMARY: ok'".into(),
                ..Default::default()
            },
        );
        assert!(
            out.contains(
                "load governor: up to 2 local run(s) at once (capacity; 12 cores, 12 CPUs, 12288 MB)"
            ),
            "{out}"
        );
        assert!(
            out.contains("machine saturated: load 40.2 on 12 CPUs is over the line 24"),
            "{out}"
        );
        assert!(!out.contains("leased "), "{out}");
        assert!(!out.contains("hands wanted"), "{out}");
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM jobs WHERE project = 'proj' AND status = 'queued'",
                &[]
            ),
            1,
            "{out}"
        );
        drop(saturated);
        // Memory under the floor holds the same way; a floor of 0 is off and
        // the run starts.
        let tight = crate::daemon_load::pin_pressure(crate::daemon_load::Pressure {
            free_mem_mb: Some(300),
            ..crate::daemon_load::CALM
        });
        let out = once_daemon(
            &s,
            DaemonOpts {
                worker_cmd: "sh -c 'echo SUMMARY: ok'".into(),
                ..Default::default()
            },
        );
        assert!(
            out.contains("machine saturated: 300 MB of memory free, under the floor 1024")
                && !out.contains("leased "),
            "{out}"
        );
        drop(tight);
        let out = once_daemon(
            &s,
            DaemonOpts {
                worker_cmd: "sh -c 'echo SUMMARY: ok'".into(),
                ..Default::default()
            },
        );
        assert!(
            out.contains("leased ") && !out.contains("machine saturated"),
            "{out}"
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM runs WHERE project = 'proj'",
                &[]
            ),
            1,
            "{out}"
        );
    }

    #[test]
    fn a_held_machine_with_queued_work_counts_as_starved() {
        let _serial = crate::test_helpers::serial();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "Waits behind the load".into(),
            "fake worker only".into(),
            0,
            100,
            "test".into(),
            pulse_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        s.set_setting(pulse_store::HUMAN_AGENT, "capacity", "2")
            .unwrap();
        let starved_since = |out: &str| {
            let (raw, _) = s.meta("capacity").unwrap();
            let note: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
            assert_eq!(note["queued"], 1, "{raw}\n{out}");
            note["starved_since"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        };
        // Held by the load with nothing in flight: the queue is short of
        // hands all the same, so the starvation clock starts.
        let saturated = crate::daemon_load::pin_pressure(crate::daemon_load::Pressure {
            load1: Some(40.0),
            ..crate::daemon_load::CALM
        });
        let out = once_daemon(
            &s,
            DaemonOpts {
                worker_cmd: "sh -c 'echo SUMMARY: ok'".into(),
                ..Default::default()
            },
        );
        assert!(!out.contains("leased "), "{out}");
        assert_eq!(starved_since(&out), TEST_NOW, "{out}");
        drop(saturated);
        // Under a soft hold the express slot stays open to the owner alone;
        // the fleet's task could not have had it, so it still waits short.
        let soft = crate::daemon_load::pin_pressure(crate::daemon_load::Pressure {
            load1: Some(30.0),
            ..crate::daemon_load::CALM
        });
        let out = once_daemon(
            &s,
            DaemonOpts {
                worker_cmd: "sh -c 'echo SUMMARY: ok'".into(),
                ..Default::default()
            },
        );
        assert!(
            out.contains("one express slot stays open") && !out.contains("leased "),
            "{out}"
        );
        assert_eq!(starved_since(&out), TEST_NOW, "{out}");
        drop(soft);
        // Thirty minutes of that with nothing queued but a reflection would
        // ask the owner; a task that waited rents a machine within the caps.
        exec1(
            &s,
            "UPDATE jobs SET created_at = ? WHERE project = 'proj'",
            &[&crate::util::plus_minutes(TEST_NOW, -31)],
        );
        let why = |w: &str| Err(NoAutoCompute::Why(w.into()));
        assert_eq!(
            auto_compute(&s, 1, 31, TEST_NOW, "daemon"),
            why("compute.auto_approve is off")
        );
        s.set_setting(pulse_store::HUMAN_AGENT, "compute.auto_approve", "1")
            .unwrap();
        assert_eq!(
            auto_compute(&s, 1, 31, TEST_NOW, "daemon"),
            why("compute.max_vms 0 reached (0 in use)")
        );
        for (k, v) in [("compute.max_vms", "1"), ("compute.max_hours_per_day", "2")] {
            s.set_setting(pulse_store::HUMAN_AGENT, k, v).unwrap();
        }
        assert_eq!(
            auto_compute(&s, 1, 31, TEST_NOW, "daemon"),
            why("compute.max_hours_per_day 2 would be passed (0 h today, 4 h asked)")
        );
        s.set_setting(pulse_store::HUMAN_AGENT, "compute.max_hours_per_day", "0")
            .unwrap();
        exec1(
            &s,
            "UPDATE jobs SET kind = 'reflect' WHERE project = 'proj'",
            &[],
        );
        assert_eq!(
            auto_compute(&s, 1, 31, TEST_NOW, "daemon"),
            why("no improve or task job that a rented Linux machine could run has waited 30 min")
        );
        exec1(
            &s,
            "UPDATE jobs SET kind = 'task' WHERE project = 'proj'",
            &[],
        );
        let req = auto_compute(&s, 1, 31, TEST_NOW, "daemon").expect("no machine rented");
        assert_eq!(
            (req.project.as_str(), req.hours),
            ("proj", AUTO_COMPUTE_HOURS)
        );
        assert_eq!(req.requested_by, "daemon");
        assert!(req.reason.contains("waited 31 min"), "{}", req.reason);
        // Approved on the spot where a login source exists; a proposal to
        // the owner where none does (the machine could not log in).
        let login = pulse_tasks::get_caps(&s).unwrap().login_source;
        assert_eq!(req.status, if login { "approved" } else { "pending" });
        let open = NoAutoCompute::Open {
            id: req.id.clone(),
            status: req.status.clone(),
        };
        assert_eq!(
            open.to_string(),
            format!("compute request {} is {}", req.id, req.status)
        );
        assert_eq!(
            auto_compute(&s, 1, 31, TEST_NOW, "daemon"),
            Err(open),
            "a second machine while the first is open"
        );
    }

    #[test]
    fn an_idle_sleep_under_a_hold_wakes_when_the_load_drops() {
        let _serial = crate::test_helpers::serial();
        let s = store_with_projects(&["proj"]);
        let machine = crate::daemon_load::Machine::detect();
        let cancel = pulse_bus::CancellationToken::new();
        let (_compute_tx, compute_rx) = crossbeam_channel::bounded::<()>(1);
        let watch = IdleWatch {
            s: &s,
            machine: &machine,
            flag_max_jobs: 0,
            done: cancel.done(),
            compute_wake: &compute_rx,
            poll: Duration::from_millis(20),
        };
        let saturated = crate::daemon_load::Pressure {
            load1: Some(40.0),
            ..crate::daemon_load::CALM
        };
        let sleep = Duration::from_millis(400);
        let mut no_bus = None;

        // Still saturated: the sleep runs its course, one poll at a time.
        let pin = crate::daemon_load::pin_pressure(saturated);
        let began = std::time::Instant::now();
        assert!(idle_wait(&watch, &mut no_bus, sleep, true));
        assert!(began.elapsed() >= sleep, "woke with the hold still on");

        // The hold lifts: the next poll wakes the loop, not the deadline.
        drop(pin);
        let began = std::time::Instant::now();
        assert!(idle_wait(&watch, &mut no_bus, sleep, true));
        assert!(
            began.elapsed() < sleep,
            "slept the whole interval after the hold lifted"
        );

        // Without a hold the load is nobody's business: the sleep runs out.
        let began = std::time::Instant::now();
        assert!(idle_wait(&watch, &mut no_bus, sleep, false));
        assert!(began.elapsed() >= sleep);

        // A queued job wakes it as before.
        let began = std::time::Instant::now();
        let s_add = s.clone();
        let adder = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(60));
            crate::util::exec1(
                &*s_add.conn().unwrap(),
                "INSERT INTO jobs (id, project, kind, title, brief, status, created_at) VALUES ('queued-1', 'proj', 'task', 'queued', '', 'queued', ?)",
                &[&TEST_NOW],
            );
        });
        assert!(idle_wait(&watch, &mut no_bus, sleep, false));
        adder.join().unwrap();
        assert!(began.elapsed() < sleep, "slept through a queued job");

        // Cancelled: false, so the loop stops.
        cancel.cancel();
        assert!(!idle_wait(&watch, &mut no_bus, sleep, true));
    }

    #[test]
    fn daemon_counts_a_measurement_in_flight_as_a_local_run() {
        let _serial = crate::test_helpers::serial();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "Fleet work waits".into(),
            "fake worker only".into(),
            0,
            100,
            "test".into(),
            pulse_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        for (k, v) in [("capacity", "1"), ("daemon.min_free_mb", "0")] {
            s.set_setting(pulse_store::HUMAN_AGENT, k, v).unwrap();
        }
        let _calm = crate::daemon_load::pin_pressure(crate::daemon_load::CALM);
        // The experiment lane is measuring (two builds) in another process or
        // thread: its mark takes the one slot under the cap.
        let mark = crate::daemon_load::MeasuringMark::new(&s.home);
        let out = once_daemon(
            &s,
            DaemonOpts {
                worker_cmd: "sh -c 'echo SUMMARY: ok'".into(),
                ..Default::default()
            },
        );
        assert!(!out.contains("leased "), "{out}");
        assert!(
            out.contains("every slot of this machine is busy with the fleet's work"),
            "{out}"
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM jobs WHERE project = 'proj' AND status = 'queued'",
                &[]
            ),
            1,
            "{out}"
        );
        drop(mark);
        let out = once_daemon(
            &s,
            DaemonOpts {
                worker_cmd: "sh -c 'echo SUMMARY: ok'".into(),
                ..Default::default()
            },
        );
        assert_eq!(out.matches("leased ").count(), 1, "{out}");
    }

    #[test]
    fn daemon_load_governor_caps_the_local_slots_at_max_jobs() {
        let _serial = crate::test_helpers::serial();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        join(&s, "borealis");
        join(&s, "cirrus");
        for title in ["First", "Second", "Third"] {
            crate::jobs::add_job(
                &s,
                "proj".into(),
                "task".into(),
                title.into(),
                "fake worker only".into(),
                0,
                100,
                "test".into(),
                pulse_store::HUMAN_AGENT.into(),
                false,
            )
            .unwrap();
        }
        s.set_setting(pulse_store::HUMAN_AGENT, "capacity", "3")
            .unwrap();
        crate::capacity::pin_per_project(&s, 3);
        s.set_setting(pulse_store::HUMAN_AGENT, "daemon.min_free_mb", "0")
            .unwrap();
        s.set_setting(pulse_store::HUMAN_AGENT, "daemon.setup_mb", "0")
            .unwrap();
        s.set_setting(pulse_store::HUMAN_AGENT, "jobs_per_day", "none")
            .unwrap();
        // The flag beats the setting, the setting beats the machine.
        s.set_setting(pulse_store::HUMAN_AGENT, "capacity", "2")
            .unwrap();
        let out = once_daemon(
            &s,
            DaemonOpts {
                worker_cmd: "sh -c 'echo SUMMARY: ok'".into(),
                max_jobs: 1,
                ..Default::default()
            },
        );
        assert!(
            out.contains("load governor: up to 1 local run(s) at once (--max-jobs;"),
            "{out}"
        );
        assert_eq!(out.matches("leased ").count(), 1, "{out}");
        let out = once_daemon(
            &s,
            DaemonOpts {
                worker_cmd: "sh -c 'echo SUMMARY: ok'".into(),
                ..Default::default()
            },
        );
        assert!(
            out.contains("load governor: up to 2 local run(s) at once (capacity;"),
            "{out}"
        );
        assert_eq!(out.matches("leased ").count(), 2, "{out}");
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM runs WHERE project = 'proj'",
                &[]
            ),
            3,
            "{out}"
        );
    }

    #[test]
    fn daemon_lends_the_owner_one_slot_over_a_cap_full_of_fleet_work() {
        let _serial = crate::test_helpers::serial();
        pulse_bus::reset_default();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        join(&s, "borealis");
        join(&s, "cirrus");
        join(&s, "drizzle");
        for (k, v) in [
            ("capacity", "2"),
            ("daemon.pass_seconds", "3600"),
            ("daemon.min_free_mb", "0"),
            ("daemon.setup_mb", "0"),
            ("jobs_per_day", "none"),
        ] {
            s.set_setting(pulse_store::HUMAN_AGENT, k, v).unwrap();
        }
        crate::capacity::pin_per_project(&s, 4);
        let _calm = crate::daemon_load::pin_pressure(crate::daemon_load::CALM);
        let add = |title: &str, source: &str| {
            crate::jobs::add_job(
                &s,
                "proj".into(),
                "task".into(),
                title.into(),
                "fake worker only".into(),
                0,
                100,
                source.into(),
                pulse_store::HUMAN_AGENT.into(),
                false,
            )
            .unwrap()
            .0
            .str("id")
        };
        add("Fleet one", "test");
        add("Fleet two", "test");
        let out = OutBuf::new();
        let opts = DaemonOpts {
            interval: 300,
            out: Some(out.writer()),
            worker_cmd: "sh -c 'sleep 8; echo SUMMARY: ok'".into(),
            ..Default::default()
        };
        let s_daemon = s.clone();
        let cancel = pulse_bus::CancellationToken::new();
        let cancel_daemon = cancel.clone();
        let (done_tx, done_rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = done_tx.send(daemon(&cancel_daemon, &s_daemon, opts));
        });
        let wait_for = |want: &dyn Fn(&str) -> bool, what: &str| {
            let since = std::time::Instant::now();
            loop {
                let text = out.string();
                if want(&text) {
                    return text;
                }
                assert!(
                    since.elapsed() < test_wait(Duration::from_secs(60)),
                    "{what} never came:\n{text}"
                );
                std::thread::sleep(Duration::from_millis(50));
            }
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            wait_for(
                &|t| t.matches("leased job-").count() >= 2,
                "the fleet's two runs",
            );
            // The cap is full of the fleet's work. The owner's job and one
            // more of the fleet's arrive: job.add wakes the loop for a tick.
            let fleet_three = add("Fleet three", "test");
            let owners = add("Owner's step", "task:TASK-001");
            let text = wait_for(&|t| t.matches("leased job-").count() >= 3, "a third lease");
            assert!(
                text.contains(
                    "every slot of this machine is busy with the fleet's work; one express slot stays open for the owner's work"
                ),
                "{text}"
            );
            assert!(
                text.contains(&format!("leased {owners}"))
                    && !text.contains(&format!("leased {fleet_three}")),
                "the slot over the cap is the owner's alone:\n{text}"
            );
            assert_eq!(
                count(
                    &s,
                    "SELECT COUNT(*) AS n FROM jobs WHERE status = 'queued' AND source = 'test'",
                    &[]
                ),
                1,
                "{text}"
            );
            // No second one while the owner's run is in flight.
            let owners_next = add("Owner's next step", "task:TASK-001:next");
            std::thread::sleep(Duration::from_millis(1500));
            let text = out.string();
            assert!(!text.contains(&format!("leased {owners_next}")), "{text}");
        }));
        cancel.cancel();
        let _ = done_rx.recv_timeout(test_wait(Duration::from_secs(30)));
        if let Err(panic) = result {
            panic!("{}\n{}", panic_message(&panic), out.string());
        }
    }

    #[test]
    fn daemon_disk_headroom_starts_the_project_that_fits_while_the_larger_one_waits() {
        let _serial = crate::test_helpers::serial();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        // Room for the 1 MB project, not the large one, whatever the host has
        // free: a failed `df` (0 MB) used to start the large one instead.
        let _free = crate::executor::pin_free_mb(5000);
        let s = new_store();
        new_bare_repo(&s);
        let small = std::path::Path::new(&s.home).join("small");
        crate::test_helpers_selfupdate::copy_dir(std::path::Path::new(&template_repo()), &small);
        crate::board::add_project(
            &s,
            small.display().to_string(),
            "Small".into(),
            String::new(),
            false,
            String::new(),
            pulse_store::HUMAN_AGENT.into(),
            "small".into(),
        )
        .unwrap();
        join(&s, "aurora");
        join(&s, "borealis");
        // The large project's job is older: first in the queue, yet it waits.
        for (slug, title) in [("proj", "Large setup"), ("small", "Small setup")] {
            crate::jobs::add_job(
                &s,
                slug.into(),
                "task".into(),
                title.into(),
                "fake worker only".into(),
                0,
                100,
                "test".into(),
                pulse_store::HUMAN_AGENT.into(),
                false,
            )
            .unwrap();
        }
        // Measured footprints from yesterday (outside today's cap): one no
        // disk pays for, one of a megabyte.
        let yesterday = crate::util::plus_hours(TEST_NOW, -24.0);
        for (id, slug, mb) in [("r-large", "proj", 2_000_000_000), ("r-small", "small", 1)] {
            exec1(
                &s,
                "INSERT INTO runs (id, job_id, agent, project, started_at, finished_at, status, executor, work_mb)
                 VALUES (?, 'j-old', 'aurora', ?, ?, ?, 'done', 'local', ?)",
                &[&id, &slug, &yesterday, &yesterday, &mb],
            );
            exec1(
                &s,
                "INSERT INTO briefs (run_id, job_id, agent, text, at) VALUES (?, 'j-old', 'aurora', 'b', ?)",
                &[&id, &yesterday],
            );
        }
        s.set_setting(pulse_store::HUMAN_AGENT, "capacity", "2")
            .unwrap();
        s.set_setting(pulse_store::HUMAN_AGENT, "daemon.min_free_mb", "0")
            .unwrap();
        s.set_setting(pulse_store::HUMAN_AGENT, "daemon.setup_mb", "1")
            .unwrap();
        let out = once_daemon(
            &s,
            DaemonOpts {
                worker_cmd: "sh -c 'echo SUMMARY: ok'".into(),
                ..Default::default()
            },
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM runs WHERE project = 'small' AND job_id != 'j-old'",
                &[]
            ),
            1,
            "the project that fits starts: {out}"
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM jobs WHERE project = 'proj' AND status = 'queued'",
                &[]
            ),
            1,
            "the project no disk pays for waits: {out}"
        );
        assert!(
            out.contains("1 for proj (local needs 2000000000 MB of disk)"),
            "the idle reason names the held project: {out}"
        );
        assert!(!out.contains("hands wanted"), "{out}");
    }

    struct SignalWriter {
        out: OutBuf,
        waiting: crossbeam_channel::Sender<()>,
        needle: String,
        sent: bool,
    }

    /// Starts a daemon that restarts on a binary change with one slow task
    /// in flight, flips the binary stamp once the run is leased and returns
    /// the daemon's output and result once it exits. `history` seeds the
    /// runs `run_fits` measures: none, `fast` (ten runs of ten seconds, one
    /// measure for every kind), `kinds` (ten tasks of ten seconds and ten
    /// improve runs of ten minutes, so only a task fits a short window) or
    /// `owner` (none, so no fleet kind fits a short window, and the owner's
    /// `task:TASK-001` job is queued after the flip beside the fleet's).
    fn restart_with_run_in_flight(
        drain_minutes: &str,
        queue_after_flip: bool,
        history: &str,
    ) -> (crate::test_helpers::TestStore, String, String, String) {
        pulse_bus::reset_default();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        join(&s, "borealis");
        if !history.is_empty() {
            // Room for the slow run, the daily reflect the planner adds over
            // a day-old history and the job queued after the flip.
            crate::capacity::pin_per_project(&s, 3);
            s.set_setting(pulse_store::HUMAN_AGENT, "jobs_per_day", "none")
                .unwrap();
            // Ten real runs of ten seconds, two days ago so the daily job cap
            // does not count them: run_fits measures 10s + 5m. `owner` seeds
            // none: every kind is expected to run to the budget.
            let two_days = 2.0 * 24.0 * 60.0;
            if history != "owner" {
                for n in 0..10 {
                    if history == "kinds" {
                        finished_job_of_kind(&s, n, "task");
                    }
                    finished_run(&s, n, two_days + n as f64, "done", "real", 10.0);
                }
            }
            if history == "kinds" {
                for n in 10..20 {
                    finished_job_of_kind(&s, n, "improve");
                    finished_run(&s, n, two_days + n as f64, "done", "real", 600.0);
                }
            }
        }
        let workers = if history.is_empty() { "2" } else { "3" };
        s.set_setting(pulse_store::HUMAN_AGENT, "capacity", workers)
            .unwrap();
        s.set_setting(
            pulse_store::HUMAN_AGENT,
            "daemon.restart_drain_minutes",
            drain_minutes,
        )
        .unwrap();
        let add_from = |title: &str, source: &str| {
            crate::jobs::add_job(
                &s,
                "proj".into(),
                "task".into(),
                title.into(),
                "fake worker only".into(),
                0,
                100,
                source.into(),
                pulse_store::HUMAN_AGENT.into(),
                false,
            )
            .unwrap()
        };
        let add = |title: &str| add_from(title, "test");
        let (slow, _) = add("Slow run");
        let slow_id = slow.str("id");
        let old_stamp = *BINARY_STAMP.read().unwrap();
        TEST_RESTART_CHANGED.store(false, std::sync::atomic::Ordering::SeqCst);
        set_binary_stamp(test_restart_stamp);
        let out = OutBuf::new();
        let (leased_tx, leased_rx) = crossbeam_channel::bounded(1);
        let writer: DaemonOut = std::sync::Arc::new(std::sync::Mutex::new(SignalWriter {
            out: out.clone(),
            waiting: leased_tx,
            needle: "leased job-".into(),
            sent: false,
        }));
        let opts = DaemonOpts {
            interval: 300,
            out: Some(writer),
            worker_cmd: "sh -c 'sleep 3; echo SUMMARY: ok'".into(),
            restart_on_change: true,
            ..Default::default()
        };
        let s_daemon = s.clone();
        let cancel = pulse_bus::CancellationToken::new();
        let cancel_daemon = cancel.clone();
        let (done_tx, done_rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = done_tx.send(daemon(&cancel_daemon, &s_daemon, opts));
        });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            leased_rx
                .recv_timeout(test_wait(Duration::from_secs(60)))
                .expect("the slow run was never leased");
            TEST_RESTART_CHANGED.store(true, std::sync::atomic::Ordering::SeqCst);
            if queue_after_flip {
                // Wake the loop so it sees the change while the slow run is
                // still in flight: without an event it re-checks the binary
                // only on the next tick or when the run ends, and under host
                // load that is after the run finished, when nothing is in
                // flight and the daemon exits without ever holding the drain.
                pulse_bus::emit(
                    &s,
                    "human",
                    "post.question",
                    "wake-restart",
                    serde_json::json!({"project": "proj"}),
                )
                .unwrap();
                // Queue once the daemon has armed the restart, so the jobs
                // meet the window rather than the tick the change fell into.
                let armed = std::time::Instant::now();
                while !out.string().contains("pulse changed on disk") {
                    assert!(
                        armed.elapsed() < test_wait(Duration::from_secs(60)),
                        "the daemon never noticed the binary change:\n{}",
                        out.string()
                    );
                    std::thread::sleep(Duration::from_millis(50));
                }
                if history == "kinds" {
                    crate::jobs::add_job(
                        &s,
                        "proj".into(),
                        "improve".into(),
                        "Improve run queued after the binary changed".into(),
                        "fake worker only".into(),
                        0,
                        200,
                        "test".into(),
                        pulse_store::HUMAN_AGENT.into(),
                        false,
                    )
                    .unwrap();
                }
                if history == "owner" {
                    add_from(
                        "Owner's task queued after the binary changed",
                        "task:TASK-001",
                    );
                }
                add("Queued after the binary changed");
            } else {
                pulse_bus::emit(
                    &s,
                    "human",
                    "post.question",
                    "wake-restart",
                    serde_json::json!({"project": "proj"}),
                )
                .unwrap();
            }
            done_rx
                .recv_timeout(test_wait(Duration::from_secs(90)))
                .expect("the daemon did not exit after the run in flight finished")
        }));
        cancel.cancel();
        set_binary_stamp(old_stamp);
        TEST_RESTART_CHANGED.store(false, std::sync::atomic::Ordering::SeqCst);
        let result = match result {
            Ok(result) => result,
            Err(panic) => panic!("{}\n{}", panic_message(&panic), out.string()),
        };
        (s, out.string(), result, slow_id)
    }

    #[test]
    fn a_restart_asked_now_holds_the_runs_in_flight_and_requeues_what_did_not_hold() {
        let _serial = crate::test_helpers::serial();
        pulse_bus::reset_default();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        s.set_setting(pulse_store::HUMAN_AGENT, "capacity", "1")
            .unwrap();
        s.set_setting(pulse_store::HUMAN_AGENT, "daemon.hold_seconds", "1")
            .unwrap();
        // An ask from before this daemon started is the one that restarted
        // the daemon before it: not read again.
        ask_restart(&s, "owner", serde_json::json!({"from": "earlier"})).unwrap();
        let (job, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "Slow run".into(),
            "fake worker only".into(),
            0,
            100,
            "test".into(),
            pulse_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let out = OutBuf::new();
        let (leased_tx, leased_rx) = crossbeam_channel::bounded(1);
        let writer: DaemonOut = std::sync::Arc::new(std::sync::Mutex::new(SignalWriter {
            out: out.clone(),
            waiting: leased_tx,
            needle: "leased job-".into(),
            sent: false,
        }));
        let opts = DaemonOpts {
            interval: 300,
            out: Some(writer),
            worker_cmd: "sh -c 'sleep 60; echo SUMMARY: ok'".into(),
            restart_on_change: true,
            ..Default::default()
        };
        let s_daemon = s.clone();
        let cancel = pulse_bus::CancellationToken::new();
        let cancel_daemon = cancel.clone();
        let (done_tx, done_rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = done_tx.send(daemon(&cancel_daemon, &s_daemon, opts));
        });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            leased_rx
                .recv_timeout(test_wait(Duration::from_secs(60)))
                .expect("the slow run was never leased");
            // The run row follows the lease by a moment; the ask waits for it
            // so the hold has a run to address.
            let began = std::time::Instant::now();
            while count(
                &s,
                "SELECT COUNT(*) AS n FROM runs WHERE status = 'running'",
                &[],
            ) == 0
            {
                assert!(
                    began.elapsed() < test_wait(Duration::from_secs(30)),
                    "the run never started:\n{}",
                    out.string()
                );
                std::thread::sleep(Duration::from_millis(50));
            }
            ask_restart(
                &s,
                "owner",
                serde_json::json!({"from": "settings", "now": true}),
            )
            .unwrap();
            done_rx
                .recv_timeout(test_wait(Duration::from_secs(90)))
                .expect("the daemon did not exit after the ask")
        }));
        cancel.cancel();
        let result = match result {
            Ok(result) => result,
            Err(panic) => panic!("{}\n{}", panic_message(&panic), out.string()),
        };
        let out = out.string();
        assert_eq!(result, "restart", "{out}");
        assert!(
            out.contains("restart asked by owner at ")
                && out.contains("1 run(s) in flight, 0 asked to hold; exiting with code 3 once they have or after 1s"),
            "{out}"
        );
        assert!(
            out.contains(
                "no harness session running (in its check, or a custom worker); it runs to the cap"
            ),
            "{out}"
        );
        assert!(
            out.contains("hold cap of 1s over: stopping 1 run(s) still in flight; their jobs return to the queue"),
            "{out}"
        );
        assert!(
            out.contains("restarting now, as asked: waiting for 1 run(s) in flight"),
            "{out}"
        );
        let conn = s.conn().unwrap();
        let runs = crate::util::must_rows(
            &conn,
            "SELECT status, summary FROM runs WHERE job_id = ?",
            &[&job.str("id")],
        );
        assert_eq!(runs.len(), 1, "{runs:?}");
        assert_eq!(runs[0].str("status"), "lost", "{runs:?}\n{out}");
        assert_eq!(
            runs[0].str("summary"),
            crate::run::LOST_RUN_SUMMARY,
            "{runs:?}"
        );
        let j = crate::jobs::job_row(&conn, job.str("id"));
        assert_eq!(
            j.str("status"),
            "queued",
            "job after the forced restart: {j:?}\n{out}"
        );
        // The cut run posts no result to the board: the job is not over.
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM posts WHERE kind = 'result'",
                &[]
            ),
            0,
            "{out}"
        );
        let left = crate::test_helpers::work_leftovers(&s.home);
        assert!(left.is_empty(), "clone left behind: {left:?}");
    }

    #[test]
    fn a_restart_asked_without_now_drains_the_runs_in_flight_and_holds_nothing() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        s.set_setting(pulse_store::HUMAN_AGENT, "capacity", "1")
            .unwrap();
        s.set_setting(pulse_store::HUMAN_AGENT, "daemon.hold_seconds", "1")
            .unwrap();
        // The default ask (no "now") drains: the run in flight is left alone
        // and the daemon goes once it ends, inside the drain budget.
        let (out, result) =
            restart_with_ask_and_run_in_flight(&s, serde_json::json!({"from": "settings"}));
        assert_eq!(result, "restart", "{out}");
        assert!(
            out.contains("draining - nothing new starts and the daemon exits with code 3"),
            "{out}"
        );
        assert!(
            out.contains("nothing is in flight; exiting with code 3"),
            "{out}"
        );
        assert!(
            !out.contains("asked to hold"),
            "a drained restart holds nothing: {out}"
        );
        assert!(
            !out.contains("requeued"),
            "a drained restart requeues nothing: {out}"
        );
        // The drain waits like any other tick: the loop that read the ask
        // once spun through its queries without a wait or a heartbeat for as
        // long as the run took, which the loop count would show in the
        // thousands; drained properly it beats, says the hold, and blocks
        // until the run ends.
        let beat = crate::util::must_one(
            &*s.conn().unwrap(),
            "SELECT value FROM meta WHERE key = 'daemon'",
            &[],
        );
        let beat: serde_json::Value = serde_json::from_str(&beat.str("value")).unwrap();
        let loops = beat["loops"].as_i64().unwrap_or(-1);
        assert!(
            (1..50).contains(&loops),
            "the drain spun the loop {loops} times:\n{out}"
        );
        assert!(
            out.contains("restarting as asked: no new runs, draining 1 in flight with"),
            "{out}"
        );
    }

    /// A console restart ask answered while a run is in flight: the daemon's
    /// whole output and its exit line, so the caller asserts how it went.
    fn restart_with_ask_and_run_in_flight(
        s: &crate::test_helpers::TestStore,
        detail: serde_json::Value,
    ) -> (String, String) {
        pulse_bus::reset_default();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        let (job, _) = crate::jobs::add_job(
            s,
            "proj".into(),
            "task".into(),
            "Slow run".into(),
            "fake worker only".into(),
            0,
            100,
            "test".into(),
            pulse_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let _ = job;
        let out = OutBuf::new();
        let (leased_tx, leased_rx) = crossbeam_channel::bounded(1);
        let writer: DaemonOut = std::sync::Arc::new(std::sync::Mutex::new(SignalWriter {
            out: out.clone(),
            waiting: leased_tx,
            needle: "leased job-".into(),
            sent: false,
        }));
        let opts = DaemonOpts {
            interval: 1,
            out: Some(writer),
            worker_cmd: "sh -c 'sleep 3; echo SUMMARY: ok'".into(),
            restart_on_change: true,
            ..Default::default()
        };

        let s_daemon = s.clone();
        let cancel = pulse_bus::CancellationToken::new();
        let cancel_daemon = cancel.clone();
        let (done_tx, done_rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = done_tx.send(daemon(&cancel_daemon, &s_daemon, opts));
        });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            leased_rx
                .recv_timeout(test_wait(Duration::from_secs(60)))
                .expect("the run was never leased");
            ask_restart(s, "owner", detail).unwrap();
            done_rx
                .recv_timeout(test_wait(Duration::from_secs(90)))
                .expect("the daemon did not exit after the ask")
        }));
        cancel.cancel();
        let result = match result {
            Ok(result) => result,
            Err(panic) => panic!("{}\n{}", panic_message(&panic), out.string()),
        };
        (out.string(), result)
    }

    #[test]
    fn restart_hands_over_once_the_runs_in_flight_finish() {
        let _serial = crate::test_helpers::serial();
        let (s, out, result, slow_id) = restart_with_run_in_flight("30", false, "");
        assert_eq!(result, "restart", "{out}");
        assert!(
            out.contains("1 run(s) in flight finish first and new runs may start for up to 30m"),
            "{out}"
        );
        assert!(
            out.contains("nothing is in flight; exiting with code 3"),
            "{out}"
        );
        assert!(
            !out.contains("restarting: waiting for 1 run(s) in flight"),
            "the handover still stopped dispatch to wait:\n{out}"
        );
        let beat = crate::util::must_one(
            &*s.conn().unwrap(),
            "SELECT value FROM meta WHERE key = 'daemon'",
            &[],
        );
        assert!(
            beat.str("value").contains("\"note\":\"restarting\""),
            "{}",
            beat.str("value")
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM runs WHERE job_id = ? AND status = 'done'",
                &[&slow_id]
            ),
            1,
            "{out}"
        );
    }

    #[test]
    fn tick_timing_says_a_slow_body_once_and_carries_it_in_the_heartbeat() {
        let bound = Duration::from_secs(15);
        let mut timing = TickTiming::default();
        let quiet = StageClock::default();
        assert_eq!(
            timing.beat(),
            None,
            "nothing measured before the first body"
        );
        assert_eq!(
            timing.record(1, Duration::from_secs(2), bound, true, &quiet),
            None
        );
        // The slow pass's clock: four tenders over a second, one under it
        // (left out), so the line names the three longest, longest first.
        let mut stages = StageClock::default();
        stages.charge("plan", Duration::from_secs(14));
        stages.charge("pipeline", Duration::from_secs(3));
        stages.charge("maintenance", Duration::from_secs(9));
        stages.charge("leads", Duration::from_millis(1500));
        stages.charge("adopt", Duration::from_millis(200));
        let line = timing
            .record(2, Duration::from_secs(40), bound, true, &stages)
            .expect("the first body over the cadence is said");
        assert_eq!(
            line,
            "tick 2 took 40s, over the 15s dispatch cadence (planning pass); queued work waited that long for a slot, said again once a tick fits; longest: plan 14s, maintenance 9s, pipeline 3s"
        );
        // Still over: quiet, so a slow pass is one line and not one per tick.
        assert_eq!(
            timing.record(3, Duration::from_secs(16), bound, false, &quiet),
            None
        );
        let beat = timing.beat().unwrap();
        assert_eq!(beat["ms"], 16_000);
        assert_eq!(beat["max_ms"], 40_000);
        assert_eq!(beat["slow"], 2);
        // The heartbeat keeps the slowest body's stages, not the last body's.
        assert_eq!(
            beat["longest"],
            serde_json::json!([
                {"stage": "plan", "ms": 14_000},
                {"stage": "maintenance", "ms": 9_000},
                {"stage": "pipeline", "ms": 3_000}
            ])
        );
        // A body that fits reopens the line for the next crossing, and a
        // refill tick is named as such; a clock with nothing over a second
        // adds no phrase.
        assert_eq!(
            timing.record(4, Duration::from_millis(900), bound, false, &quiet),
            None
        );
        let again = timing
            .record(5, Duration::from_secs(20), bound, false, &quiet)
            .unwrap();
        assert!(
            again.ends_with("(refill tick); queued work waited that long for a slot, said again once a tick fits"),
            "{again}"
        );
        assert_eq!(timing.beat().unwrap()["slow"], 3);
    }

    #[test]
    fn stage_clock_charges_the_time_since_the_previous_mark() {
        let mut clock = StageClock::start();
        std::thread::sleep(Duration::from_millis(20));
        clock.mark("reap");
        clock.mark("plan");
        assert!(
            clock.spent["reap"] >= Duration::from_millis(20),
            "{:?}",
            clock.spent
        );
        assert!(clock.spent["plan"] < Duration::from_millis(20));
        // A stage marked once per project adds up across the projects.
        clock.charge("pipeline", Duration::from_secs(2));
        clock.charge("pipeline", Duration::from_secs(3));
        assert_eq!(clock.longest(3), vec![("pipeline", Duration::from_secs(5))]);
    }

    #[test]
    fn pass_spacing_reads_the_setting_and_refuses_negatives() {
        let s = new_store();
        assert_eq!(pass_spacing(&s), Duration::from_secs(60));
        s.set_setting(pulse_store::HUMAN_AGENT, "daemon.pass_seconds", "0")
            .unwrap();
        assert_eq!(pass_spacing(&s), Duration::ZERO);
        assert!(
            s.set_setting(pulse_store::HUMAN_AGENT, "daemon.pass_seconds", "-5")
                .is_err(),
            "the catalogue floors the spacing at zero"
        );
    }

    /// Inside `daemon.pass_seconds` the loop's wakes are ticks: a job queued
    /// while a run is in flight is still leased at once, the runs that end
    /// are reaped, but the planning pass does not run again.
    #[test]
    fn ticks_inside_the_pass_spacing_refill_slots_without_planning() {
        let _serial = crate::test_helpers::serial();
        pulse_bus::reset_default();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        join(&s, "borealis");
        s.set_setting(pulse_store::HUMAN_AGENT, "capacity", "2")
            .unwrap();
        s.set_setting(pulse_store::HUMAN_AGENT, "daemon.pass_seconds", "3600")
            .unwrap();
        let add = |title: &str| {
            crate::jobs::add_job(
                &s,
                "proj".into(),
                "task".into(),
                title.into(),
                "fake worker only".into(),
                0,
                100,
                "test".into(),
                pulse_store::HUMAN_AGENT.into(),
                false,
            )
            .unwrap()
        };
        add("First run");
        let out = OutBuf::new();
        let (leased_tx, leased_rx) = crossbeam_channel::bounded(1);
        let writer: DaemonOut = std::sync::Arc::new(std::sync::Mutex::new(SignalWriter {
            out: out.clone(),
            waiting: leased_tx,
            needle: "leased job-".into(),
            sent: false,
        }));
        let opts = DaemonOpts {
            interval: 300,
            out: Some(writer),
            worker_cmd: "sh -c 'sleep 1; echo SUMMARY: ok'".into(),
            ..Default::default()
        };
        let s_daemon = s.clone();
        let cancel = pulse_bus::CancellationToken::new();
        let cancel_daemon = cancel.clone();
        let (done_tx, done_rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = done_tx.send(daemon(&cancel_daemon, &s_daemon, opts));
        });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            leased_rx
                .recv_timeout(test_wait(Duration::from_secs(60)))
                .expect("the first run was never leased");
            // Queued inside the spacing: job.add wakes the loop for a tick.
            add("Second run");
            let since = std::time::Instant::now();
            loop {
                let text = out.string();
                let loops = s
                    .meta("daemon")
                    .ok()
                    .and_then(|(raw, _)| serde_json::from_str::<serde_json::Value>(&raw).ok())
                    .and_then(|beat| beat.get("loops").and_then(|v| v.as_i64()))
                    .unwrap_or(0);
                if text.matches("leased job-").count() >= 2
                    && text.matches("done job-").count() >= 2
                    && loops >= 4
                {
                    break;
                }
                assert!(
                    since.elapsed() < test_wait(Duration::from_secs(60)),
                    "two leases, two finished runs and four loops never came:\n{text}"
                );
                std::thread::sleep(Duration::from_millis(50));
            }
        }));
        cancel.cancel();
        let _ = done_rx.recv_timeout(test_wait(Duration::from_secs(30)));
        if let Err(panic) = result {
            panic!("{}\n{}", panic_message(&panic), out.string());
        }
        let text = out.string();
        let beat: serde_json::Value = serde_json::from_str(&s.meta("daemon").unwrap().0).unwrap();
        assert!(beat["loops"].as_i64().unwrap() >= 4, "{beat}\n{text}");
        assert_eq!(
            beat["passes"].as_i64(),
            Some(1),
            "one planning pass, the first; the ticks after it only reaped and refilled: {beat}\n{text}"
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM jobs WHERE status = 'done'",
                &[]
            ),
            2,
            "{text}"
        );
    }

    /// A task the owner files inside `daemon.pass_seconds` does not wait
    /// the spacing out: the wake it causes is a planning pass, so the lead's
    /// job for it is queued at once - the task starts the way a chat
    /// message does. An agent's own write stays a tick.
    #[test]
    fn owner_task_inside_the_pass_spacing_plans_at_once() {
        let _serial = crate::test_helpers::serial();
        pulse_bus::reset_default();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        s.set_setting(pulse_store::HUMAN_AGENT, "daemon.pass_seconds", "3600")
            .unwrap();
        s.set_setting(pulse_store::HUMAN_AGENT, "daemon.sweep", "off")
            .unwrap();
        let out = OutBuf::new();
        let opts = DaemonOpts {
            interval: 300,
            out: Some(out.writer()),
            // No run ever finishes on its own: the loop's wakes are the bus's.
            worker_cmd: "sh -c 'sleep 30; echo SUMMARY: ok'".into(),
            ..Default::default()
        };
        let s_daemon = s.clone();
        let cancel = pulse_bus::CancellationToken::new();
        let cancel_daemon = cancel.clone();
        let (done_tx, done_rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = done_tx.send(daemon(&cancel_daemon, &s_daemon, opts));
        });
        let passes = || -> i64 {
            s.meta("daemon")
                .ok()
                .and_then(|(raw, _)| serde_json::from_str::<serde_json::Value>(&raw).ok())
                .and_then(|beat| beat.get("passes").and_then(|v| v.as_i64()))
                .unwrap_or(0)
        };
        let wait_for = |what: &str, ok: &dyn Fn() -> bool| {
            let since = std::time::Instant::now();
            while !ok() {
                assert!(
                    since.elapsed() < test_wait(Duration::from_secs(30)),
                    "{what} never came:\n{}",
                    out.string()
                );
                std::thread::sleep(Duration::from_millis(50));
            }
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // The first pass: nothing to plan, the loop sleeps the interval.
            wait_for("the first planning pass", &|| passes() >= 1);
            // An agent's job inside the spacing wakes a tick, not a pass.
            crate::jobs::add_job(
                &s,
                "proj".into(),
                "task".into(),
                "An agent's job".into(),
                "fake worker only".into(),
                0,
                100,
                "test".into(),
                "aurora".into(),
                false,
            )
            .unwrap();
            wait_for("the tick for the agent's job", &|| {
                out.string().contains("leased job-")
            });
            assert_eq!(
                passes(),
                1,
                "an agent's write planned early:\n{}",
                out.string()
            );
            // The owner files a task: the wake is a planning pass and the
            // lead's job for the task is queued by it.
            let task = pulse_tasks::new(
                &s,
                "proj",
                "Fix the footer",
                "the brief",
                50,
                pulse_store::HUMAN_AGENT,
            )
            .unwrap();
            wait_for("the planning pass for the owner's task", &|| passes() >= 2);
            wait_for("the lead job for the owner's task", &|| {
                count(
                    &s,
                    "SELECT COUNT(*) AS n FROM jobs WHERE kind = 'lead' AND project = 'proj'",
                    &[],
                ) >= 1
            });
            let lead_jobs = crate::util::must_rows(
                &*s.conn().unwrap(),
                "SELECT brief FROM jobs WHERE kind = 'lead' AND project = 'proj'",
                &[],
            );
            assert!(
                lead_jobs.iter().any(|j| j.str("brief").contains(&task.id)),
                "the lead job does not name {}: {lead_jobs:?}\n{}",
                task.id,
                out.string()
            );
        }));
        cancel.cancel();
        let _ = done_rx.recv_timeout(test_wait(Duration::from_secs(60)));
        if let Err(panic) = result {
            panic!("{}\n{}", panic_message(&panic), out.string());
        }
    }

    #[test]
    fn restart_drain_zero_stops_dispatch_at_once() {
        let _serial = crate::test_helpers::serial();
        let (_s, out, result, _) = restart_with_run_in_flight("0", false, "");
        assert_eq!(result, "restart", "{out}");
        assert!(
            out.contains("pulse changed on disk; exiting with code 3 so the service restarts it")
                && out.contains("restarting: waiting for 1 run(s) in flight"),
            "{out}"
        );
        assert!(
            !out.contains("finish first and new runs may start"),
            "{out}"
        );
    }

    #[test]
    fn restart_drain_holds_runs_that_would_not_fit() {
        let _serial = crate::test_helpers::serial();
        // One minute of drain is less than minutes_per_job + 5: the fleet's
        // job (source `test`, not the owner's) queued after the binary
        // changed waits for the next daemon, while the window stays open to
        // the owner's work.
        let (s, out, result, slow_id) = restart_with_run_in_flight("1", true, "");
        assert_eq!(result, "restart", "{out}");
        assert!(
            out.contains(
                "only the owner's work starts (one fleet run may take up to 15m0s and would not fit the 1m0s drain)"
            ),
            "{out}"
        );
        assert!(
            out.contains("restarting: only the owner's work starts with"),
            "{out}"
        );
        assert_eq!(
            out.matches("restarting: only the owner's work starts with")
                .count(),
            1,
            "the hold line should be said once, not per tick: {out}"
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM jobs WHERE title = 'Queued after the binary changed' AND status = 'queued'",
                &[]
            ),
            1,
            "{out}"
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM runs WHERE project = 'proj' AND job_id <> ?",
                &[&slow_id]
            ),
            0,
            "a run started inside the last minute of the drain:\n{out}"
        );
    }

    #[test]
    fn restart_keeps_dispatching_runs_that_end_before_the_slowest_run_must() {
        let _serial = crate::test_helpers::serial();
        // The same one-minute drain, but the fleet's runs take seconds: the
        // run in flight is expected to go on for five more minutes, so the job
        // queued after the binary changed runs now, inside the time the
        // restart waits anyway, instead of idling until the next daemon.
        let (s, out, result, _) = restart_with_run_in_flight("1", true, "fast");
        assert_eq!(result, "restart", "{out}");
        assert!(
            out.contains("new runs may start for up to ")
                && out.contains(" while the slowest run in flight is expected to go on"),
            "{out}"
        );
        // A hold may follow once the slow run is done and only the drain is
        // left, never before the queued job was leased.
        let leased_twice = out.match_indices("leased job-").nth(1).map(|(i, _)| i);
        assert!(
            leased_twice.is_some(),
            "the queued job was never leased:\n{out}"
        );
        if let Some(hold) = out.find("restarting: only the owner's work starts with") {
            assert!(hold > leased_twice.unwrap(), "{out}");
        }
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM runs r JOIN jobs j ON j.id = r.job_id WHERE j.title = 'Queued after the binary changed' AND r.status = 'done'",
                &[]
            ),
            1,
            "the queued job did not run before the handover:\n{out}"
        );
        assert!(
            out.contains("nothing is in flight; exiting with code 3"),
            "{out}"
        );
    }

    #[test]
    fn restart_window_keeps_the_short_kinds_running_while_the_long_ones_wait() {
        let _serial = crate::test_helpers::serial();
        // A task takes seconds and an improve run ten minutes: inside the
        // window the slow task leaves, the task queued after the binary
        // changed runs while the improve run queued beside it waits for the
        // next daemon, and the notice names the kind that may start beside
        // the owner's work, which starts in any window.
        let (s, out, result, _) = restart_with_run_in_flight("1", true, "kinds");
        assert_eq!(result, "restart", "{out}");
        assert!(
            out.contains("the owner's work and task runs on proj may start for up to ")
                && out.contains(" while the slowest run in flight is expected to go on"),
            "{out}"
        );
        assert!(
            out.contains("restarting: the owner's work and task runs on proj start with "),
            "{out}"
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM runs r JOIN jobs j ON j.id = r.job_id WHERE j.title = 'Queued after the binary changed' AND r.status = 'done'",
                &[]
            ),
            1,
            "the task did not run before the handover:\n{out}"
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM jobs WHERE title = 'Improve run queued after the binary changed' AND status = 'queued'",
                &[]
            ),
            1,
            "the improve run started inside a window it does not fit:\n{out}"
        );
    }

    #[test]
    fn restart_window_starts_the_owner_s_work_while_the_fleet_s_waits() {
        let _serial = crate::test_helpers::serial();
        // No history: every kind is expected to run to the budget and none
        // fits the one-minute drain. The fleet's task queued after the
        // binary changed waits for the next daemon as before; the owner's
        // task (`task:TASK-001`), queued beside it, starts inside the
        // window all the same - on the brain one waited 13 hours through
        // restart windows that refused its kind.
        let (s, out, result, _) = restart_with_run_in_flight("1", true, "owner");
        assert_eq!(result, "restart", "{out}");
        assert!(
            out.contains(
                "only the owner's work starts (one fleet run may take up to 15m0s and would not fit the 1m0s drain)"
            ),
            "{out}"
        );
        assert!(
            out.contains("restarting: only the owner's work starts with"),
            "{out}"
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM runs r JOIN jobs j ON j.id = r.job_id WHERE j.source = 'task:TASK-001' AND r.status = 'done'",
                &[]
            ),
            1,
            "the owner's task did not run before the handover:\n{out}"
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM jobs WHERE title = 'Queued after the binary changed' AND status = 'queued'",
                &[]
            ),
            1,
            "the fleet's task started inside a window it does not fit:\n{out}"
        );
    }

    impl Write for SignalWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let text = String::from_utf8_lossy(buf);
            if !self.sent && text.contains(&self.needle) {
                self.sent = true;
                let _ = self.waiting.send(());
            }
            self.out
                .0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn daemon_interval_flag_defaults() {
        let _serial = crate::test_helpers::serial();
        crate::commands::register();
        for name in ["daemon", "service"] {
            let cmd = pulse_cli::lookup(name).unwrap_or_else(|| panic!("{name} not registered"));
            let mut fs = pulse_cli::FlagSet::new(name);
            (cmd.flags.expect("flags"))(&mut fs);
            let f = fs
                .lookup("interval")
                .unwrap_or_else(|| panic!("{name} interval flag"));
            assert_eq!(
                crate::commands::int_flag(&fs, "interval"),
                DEFAULT_DAEMON_INTERVAL_SECONDS,
                "interval flag = {f:?}, want default {DEFAULT_DAEMON_INTERVAL_SECONDS}"
            );
            fs.parse(&["--interval".into(), "7".into()]).unwrap();
            assert_eq!(crate::commands::int_flag(&fs, "interval"), 7);
        }
    }

    pub(crate) fn daemon_retry_fixture() -> (crate::test_helpers::TestStore, crate::util::Row) {
        pulse_bus::reset_default();
        let s = new_store();
        let (_project, root) = new_bare_repo(&s);
        std::process::Command::new("git")
            .args(["checkout", "-q", "-b", "evolve/slow-measurement"])
            .current_dir(&root)
            .status()
            .unwrap();
        std::fs::write(std::path::Path::new(&root).join("slow.txt"), "slow\n").unwrap();
        std::process::Command::new("git")
            .args(["add", "slow.txt"])
            .current_dir(&root)
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "gpg.format=openpgp",
                "commit",
                "-q",
                "-m",
                "slow",
            ])
            .current_dir(&root)
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["checkout", "-q", "main"])
            .current_dir(&root)
            .status()
            .unwrap();
        std::fs::write(
            std::path::Path::new(&root).join("dirty.txt"),
            "hold landing\n",
        )
        .unwrap();
        join(&s, "aurora");
        let (job, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "retry".into(),
            "exercise retry timing".into(),
            1,
            100,
            "retry-test".into(),
            pulse_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        crate::plan::plan_jobs(&s, "proj".into(), s.now(), pulse_store::HUMAN_AGENT.into())
            .unwrap();
        (s, job)
    }

    fn normalized_interval(interval: i32) -> i32 {
        if interval <= 0 {
            DEFAULT_DAEMON_INTERVAL_SECONDS
        } else {
            interval
        }
    }

    #[test]
    fn daemon_runner_error_wait() {
        let _serial = crate::test_helpers::serial();
        for (name, interval, want) in [
            ("omitted", 0, DEFAULT_DAEMON_INTERVAL_SECONDS),
            (
                "explicit-default",
                DEFAULT_DAEMON_INTERVAL_SECONDS,
                DEFAULT_DAEMON_INTERVAL_SECONDS,
            ),
            ("negative", -1, DEFAULT_DAEMON_INTERVAL_SECONDS),
            ("custom", 7, 7),
        ] {
            let (s, job) = daemon_retry_fixture();
            assert_eq!(normalized_interval(interval), want, "{name}");
            let out = OutBuf::new();
            let waits = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let attempts_at_wait = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let waits_for_sleep = waits.clone();
            let attempts_for_sleep = attempts_at_wait.clone();
            let s_for_sleep = s.clone();
            let job_id = job.str("id");
            let result = daemon(
                &pulse_bus::CancellationToken::new(),
                &s,
                DaemonOpts {
                    interval,
                    max_loops: 2,
                    out: Some(out.writer()),
                    worker_cmd: format!(
                        "{:?}",
                        std::path::Path::new(&s.home)
                            .join("missing-worker")
                            .display()
                            .to_string()
                    ),
                    sleep: Some(std::sync::Arc::new(move |d| {
                        waits_for_sleep.lock().unwrap().push(d);
                        attempts_for_sleep.lock().unwrap().push(count(
                            &s_for_sleep,
                            "SELECT COUNT(*) AS n FROM runs WHERE job_id = ?",
                            &[&job_id],
                        ));
                        true
                    })),
                    ..Default::default()
                },
            );
            let waits = waits.lock().unwrap().clone();
            assert!(
                result == "done"
                    && waits.len() == 1
                    && waits[0] == Duration::from_secs(want as u64),
                "daemon = {result}, waits = {waits:?}, want [{want}s]\n{}",
                out.string()
            );
            assert_eq!(attempts_at_wait.lock().unwrap()[0], 1);
            let attempts = count(
                &s,
                "SELECT COUNT(*) AS n FROM runs WHERE job_id = ? AND status = 'error'",
                &[&job.str("id")],
            );
            assert_eq!(
                attempts,
                2,
                "{name}: error attempts = {attempts}, want 2\n{}",
                out.string()
            );
            assert!(
                out.string().contains(&format!("interval {want}s")),
                "{name}: configured interval missing:\n{}",
                out.string()
            );
        }
    }

    #[test]
    fn daemon_runner_error_wait_interrupted() {
        let _serial = crate::test_helpers::serial();
        let (s, job) = daemon_retry_fixture();
        let cancel = pulse_bus::CancellationToken::new();
        let out = OutBuf::new();
        let waits = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let waits_for_sleep = waits.clone();
        let cancel_for_sleep = cancel.clone();
        let result = daemon(
            &cancel,
            &s,
            DaemonOpts {
                interval: 7,
                max_loops: 2,
                out: Some(out.writer()),
                worker_cmd: format!(
                    "{:?}",
                    std::path::Path::new(&s.home)
                        .join("missing-worker")
                        .display()
                        .to_string()
                ),
                sleep: Some(std::sync::Arc::new(move |d| {
                    waits_for_sleep.lock().unwrap().push(d);
                    cancel_for_sleep.cancel();
                    false
                })),
                ..Default::default()
            },
        );
        let waits = waits.lock().unwrap().clone();
        assert!(
            result == "interrupted" && waits == vec![Duration::from_secs(7)],
            "daemon = {result}, waits = {waits:?}\n{}",
            out.string()
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM runs WHERE job_id = ?",
                &[&job.str("id")]
            ),
            1
        );
    }

    const OUTAGE: &str = "worker exited 3: harness stopped: provider_error after 1 model calls and 0 commands; copilot: error sending request for url (https://api.business.githubcopilot.com/chat/completions)";

    fn outage_run(job: &Row, summary: &str, commits: i64) -> Row {
        let mut run = Row::new();
        for (k, v) in [
            ("id", "run-outage".to_string()),
            ("job_id", job.str("id")),
            ("status", "failed".to_string()),
            ("summary", summary.to_string()),
        ] {
            run.insert(k, serde_json::Value::String(v));
        }
        run.insert("commits", serde_json::json!(commits));
        run
    }

    /// Only an experiment that failed on a provider transport error before
    /// any command, with no commit, is a cycle the outage lost.
    #[test]
    fn outage_lost_cycle_names_only_transport_stops_before_any_command() {
        let mut job = Row::new();
        job.insert("id", serde_json::json!("job-x"));
        job.insert("kind", serde_json::json!("experiment"));
        assert!(outage_lost_cycle(&job, &outage_run(&job, OUTAGE, 0)));
        // A commit means the cycle ran: its failure is the experiment's.
        assert!(!outage_lost_cycle(&job, &outage_run(&job, OUTAGE, 1)));
        // Commands ran: the provider stopped mid-work, the idea was tried.
        let mid = OUTAGE.replace("0 commands", "4 commands");
        assert!(!outage_lost_cycle(&job, &outage_run(&job, &mid, 0)));
        // A refused model setting is not an outage: retrying it changes nothing.
        let refused = "worker exited 3: harness stopped: provider_error after 1 model calls and 0 commands; error: model gpt-x does not support reasoning";
        assert!(!outage_lost_cycle(&job, &outage_run(&job, refused, 0)));
        let mut task = job.clone();
        task.insert("kind", serde_json::json!("task"));
        assert!(!outage_lost_cycle(&task, &outage_run(&task, OUTAGE, 0)));
    }

    /// The daemon puts an experiment the provider outage stopped back in the
    /// queue (twice at most) instead of leaving it failed, where the loop's
    /// tally would score its card tried.
    #[test]
    fn daemon_requeues_an_experiment_lost_to_a_provider_outage() {
        let _serial = crate::test_helpers::serial();
        pulse_bus::reset_default();
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        crate::test_helpers_daemon::set_setting(&s, "daemon.sweep", "off");
        crate::test_helpers_daemon::set_setting(&s, "jobs_per_day", "10");
        let (job, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "experiment".into(),
            "Experiment: outage".into(),
            "Experiment card\nHypothesis: x\nBranch: evolve/outage\n".into(),
            1,
            100,
            "suggestion:S-outage".into(),
            pulse_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let worker = format!("sh -c 'echo \"SUMMARY: {OUTAGE}\"; exit 3'");
        let status = |s: &pulse_store::Store| {
            crate::jobs::job_row(&*s.conn().unwrap(), job.str("id")).str("status")
        };
        for n in 1..=OUTAGE_REQUEUE_LIMIT {
            let out = once_daemon(
                &s,
                DaemonOpts {
                    worker_cmd: worker.clone(),
                    ..Default::default()
                },
            );
            assert!(
                out.contains(&format!(
                    "requeued {}: {OUTAGE_REQUEUE_WHY} ({n} of {OUTAGE_REQUEUE_LIMIT})",
                    job.str("id")
                )),
                "pass {n}: {out}"
            );
            assert_eq!(status(&s), "queued", "pass {n}: {out}");
        }
        // The outage outlasts the retries: the failure stands.
        let out = once_daemon(
            &s,
            DaemonOpts {
                worker_cmd: worker,
                ..Default::default()
            },
        );
        assert!(!out.contains("requeued "), "{out}");
        assert_eq!(status(&s), "failed", "{out}");
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM runs WHERE job_id = ?",
                &[&job.str("id")]
            ),
            OUTAGE_REQUEUE_LIMIT + 1
        );
    }

    #[test]
    fn daemon_successful_runs_do_not_back_off() {
        let _serial = crate::test_helpers::serial();
        let (s, first) = daemon_retry_fixture();
        let (second, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "next".into(),
            "exercise successful dispatch".into(),
            1,
            90,
            "next-test".into(),
            pulse_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let out = OutBuf::new();
        let waits = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let waits_for_sleep = waits.clone();
        let result = daemon(
            &pulse_bus::CancellationToken::new(),
            &s,
            DaemonOpts {
                interval: 7,
                max_loops: 2,
                out: Some(out.writer()),
                worker_cmd: "sh -c 'echo SUMMARY: ok'".into(),
                sleep: Some(std::sync::Arc::new(move |d| {
                    waits_for_sleep.lock().unwrap().push(d);
                    true
                })),
                ..Default::default()
            },
        );
        assert!(
            result == "done" && waits.lock().unwrap().is_empty(),
            "daemon = {result}, waits = {:?}\n{}",
            waits.lock().unwrap(),
            out.string()
        );
        for job in [first, second] {
            let n = count(
                &s,
                "SELECT COUNT(*) AS n FROM runs WHERE job_id = ? AND status = 'done'",
                &[&job.str("id")],
            );
            assert_eq!(
                n,
                1,
                "successful attempts for {} = {n}, want 1\n{}",
                job.str("id"),
                out.string()
            );
        }
    }

    /// A worker is not kept idle: the tick after its run ends it retires for
    /// good (`agent.retire`, status pruned, name never reused) and the next
    /// job runs under a fresh name.
    #[test]
    fn daemon_retires_a_worker_once_its_run_ends_and_spawns_a_fresh_one() {
        let _serial = crate::test_helpers::serial();
        let (s, first) = daemon_retry_fixture();
        s.set_setting(pulse_store::HUMAN_AGENT, "capacity", "1")
            .unwrap();
        let (second, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "next".into(),
            "exercise a fresh worker".into(),
            1,
            90,
            "next-test".into(),
            pulse_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let out = OutBuf::new();
        let result = daemon(
            &pulse_bus::CancellationToken::new(),
            &s,
            DaemonOpts {
                interval: 7,
                max_loops: 3,
                out: Some(out.writer()),
                worker_cmd: "sh -c 'echo SUMMARY: ok'".into(),
                sleep: Some(std::sync::Arc::new(|_| true)),
                ..Default::default()
            },
        );
        assert_eq!(result, "done", "{}", out.string());
        let conn = s.conn().unwrap();
        let run_agent = |job: &crate::util::Row| {
            crate::util::must_one(
                &conn,
                "SELECT agent FROM runs WHERE job_id = ? AND status = 'done'",
                &[&job.str("id")],
            )
            .str("agent")
        };
        let (a, b) = (run_agent(&first), run_agent(&second));
        assert!(
            !a.is_empty() && !b.is_empty(),
            "runs: {a:?} {b:?}\n{}",
            out.string()
        );
        assert_ne!(a, b, "the second job reused {a}\n{}", out.string());
        for name in [&a, &b] {
            let row = s.agent(name).unwrap().expect(name);
            assert_eq!(row.status, "pruned", "{name}\n{}", out.string());
            assert_eq!(row.pruned_reason, pulse_board::RETIRE_REASON, "{name}");
        }
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM events WHERE action = 'agent.retire' AND actor = 'daemon'",
                &[]
            ),
            2,
            "{}",
            out.string()
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM agents WHERE kind = 'worker' AND status = 'active'",
                &[]
            ),
            0,
            "{}",
            out.string()
        );
        assert!(
            out.string()
                .contains(&format!("retired {a}: {}", pulse_board::RETIRE_REASON)),
            "{}",
            out.string()
        );
    }

    #[test]
    fn daemon_retry_uses_post_wait_time() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let (fixed, job) = daemon_retry_fixture();
        let _now = pulse_testutil::setenv("PULSE_NOW", TEST_NOW);
        let s = pulse_store::Store::open(pulse_store::Options {
            home: fixed.home.clone(),
            create: false,
            read_only: false,
            skip_records: false,
            now: String::new(),
        })
        .unwrap();
        let later = crate::util::plus_minutes(TEST_NOW, 7.0 / 60.0);
        assert_eq!(s.now(), TEST_NOW);
        assert_eq!(later, "2026-09-06T10:00:07Z");
        let out = OutBuf::new();
        let later_for_sleep = later.clone();
        let result = daemon(
            &pulse_bus::CancellationToken::new(),
            &s,
            DaemonOpts {
                interval: 7,
                max_loops: 2,
                out: Some(out.writer()),
                worker_cmd: format!(
                    "{:?}",
                    std::path::Path::new(&s.home)
                        .join("missing-worker")
                        .display()
                        .to_string()
                ),
                sleep: Some(std::sync::Arc::new(move |_| {
                    // The test holds pulse_testutil::env_lock for the whole body.
                    unsafe {
                        std::env::set_var("PULSE_NOW", &later_for_sleep);
                    }
                    true
                })),
                ..Default::default()
            },
        );
        let runs = crate::util::must_rows(
            &*s.conn().unwrap(),
            "SELECT started_at FROM runs WHERE job_id = ? ORDER BY started_at, id",
            &[&job.str("id")],
        );
        assert_eq!(
            (
                result.as_str(),
                runs.len(),
                runs.first().map(|r| r.str("started_at")),
                runs.get(1).map(|r| r.str("started_at"))
            ),
            ("done", 2, Some(TEST_NOW.to_string()), Some(later.clone())),
            "daemon = {result}, starts = {runs:?}, want {TEST_NOW} then {later}\n{}",
            out.string()
        );
    }

    #[test]
    fn daemon_wakes_on_post_question() {
        let _serial = crate::test_helpers::serial();
        pulse_bus::reset_default();
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        let cancel = pulse_bus::CancellationToken::new();
        let out = OutBuf::new();
        let (done_tx, done_rx) = crossbeam_channel::bounded(1);
        let s_daemon = s.clone();
        let cancel_daemon = cancel.clone();
        let out_daemon = out.clone();
        std::thread::spawn(move || {
            let result = daemon(
                &cancel_daemon,
                &s_daemon,
                DaemonOpts {
                    interval: 30,
                    max_loops: 2,
                    worker_cmd: "sh -c 'echo SUMMARY: ok'".into(),
                    out: Some(out_daemon.writer()),
                    ..Default::default()
                },
            );
            let _ = done_tx.send(result);
        });
        let deadline = std::time::Instant::now() + test_wait(Duration::from_secs(30));
        while std::time::Instant::now() < deadline {
            let beat = crate::util::must_one(
                &*s.conn().unwrap(),
                "SELECT value FROM meta WHERE key = 'daemon'",
                &[],
            );
            if beat.str("value").contains("\"loops\":1") {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        pulse_bus::emit(
            &s,
            "human",
            "post.question",
            "q1",
            serde_json::json!({"project": "proj"}),
        )
        .unwrap();
        let result = done_rx
            .recv_timeout(test_wait(Duration::from_secs(30)))
            .unwrap_or_else(|_| {
                cancel.cancel();
                panic!(
                    "the daemon did not wake on post.question:\n{}",
                    out.string()
                )
            });
        assert_eq!(result, "done", "daemon = {result}\n{}", out.string());
        let raw = crate::util::must_one(
            &*s.conn().unwrap(),
            "SELECT value FROM meta WHERE key = 'daemon'",
            &[],
        )
        .str("value");
        assert!(!raw.is_empty(), "daemon = {raw}");
        assert!(raw.contains("\"loops\""), "daemon = {raw}");
        assert!(DAEMON_WAKE.contains(&"post.question"));
        assert!(DAEMON_WAKE.contains(&"job.add"));
        assert!(DAEMON_WAKE.contains(&"job.queued"));
        assert!(DAEMON_WAKE.contains(&"job.requeue"));
        assert!(DAEMON_WAKE.contains(&"job.recover"));
    }

    #[test]
    fn daemon_cancellation_during_maintenance_drain() {
        let _serial = crate::test_helpers::serial();
        for exit in ["done", "restart"] {
            pulse_bus::reset_default();
            let _env_guard = pulse_testutil::env_lock();
            let (s, root, binary, _fixture) = self_update_fixture();
            let _login = with_closed_login_gate();
            crate::test_helpers_daemon::set_setting(&s, "landing", "agents");
            run(&root, &["git", "branch", "evolve/draining"]);
            exec1(&s,
                "INSERT INTO jobs (id, project, kind, title, status, branch, created_at, finished_at)
                 VALUES ('draining', 'proj', 'experiment', 'Fake pending measurement', 'done', 'evolve/draining', ?, ?)",
                &[&TEST_NOW, &TEST_NOW],
            );
            let (started_tx, started_rx) = crossbeam_channel::bounded::<()>(1);
            let (cancelled_tx, cancelled_rx) = crossbeam_channel::bounded::<()>(1);
            let (release_tx, release_rx) = crossbeam_channel::bounded::<()>(1);
            let old_measure = pulse_evolve::set_measure(Box::new(
                move |ctx, _s, _project, _repo, _ref_name, _opts| {
                    let _ = started_tx.send(());
                    loop {
                        if ctx.is_cancelled() {
                            let _ = cancelled_tx.send(());
                            anyhow::bail!("context canceled");
                        }
                        if release_rx.try_recv().is_ok() {
                            anyhow::bail!("fake measurement released by test cleanup");
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                },
            ));
            let old_stamp = *BINARY_STAMP.read().unwrap();
            TEST_RESTART_CHANGED.store(false, std::sync::atomic::Ordering::SeqCst);
            set_binary_stamp(test_restart_stamp);
            if exit == "restart" {
                TEST_RESTART_CHANGED.store(false, std::sync::atomic::Ordering::SeqCst);
            }
            let cancel = pulse_bus::CancellationToken::new();
            let out = OutBuf::new();
            let (waiting_tx, waiting_rx) = crossbeam_channel::bounded(1);
            let writer: DaemonOut = std::sync::Arc::new(std::sync::Mutex::new(SignalWriter {
                out: out.clone(),
                waiting: waiting_tx,
                needle: "waiting for landing maintenance to finish".into(),
                sent: false,
            }));
            let opts = DaemonOpts {
                interval: 300,
                out: Some(writer),
                self_update: Some(Box::new(binary.clone())),
                max_loops: if exit == "done" { 1 } else { 0 },
                restart_on_change: exit == "restart",
                ..Default::default()
            };
            let s_daemon = s.clone();
            let cancel_daemon = cancel.clone();
            let (done_tx, done_rx) = crossbeam_channel::bounded(1);
            std::thread::spawn(move || {
                let _ = done_tx.send(daemon(&cancel_daemon, &s_daemon, opts));
            });
            let mut joined = false;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                started_rx
                    .recv_timeout(test_wait(Duration::from_secs(20)))
                    .expect("fake measurement never started");
                if exit == "restart" {
                    TEST_RESTART_CHANGED.store(true, std::sync::atomic::Ordering::SeqCst);
                    pulse_bus::emit(
                        &s,
                        "human",
                        "post.question",
                        "wake-restart",
                        serde_json::json!({"project": "proj"}),
                    )
                    .unwrap();
                }
                waiting_rx
                    .recv_timeout(test_wait(Duration::from_secs(20)))
                    .expect("daemon did not enter its maintenance drain");
                if exit == "restart" {
                    // A restart cancels the experiment lane at once instead of
                    // holding the new binary for the whole measurement.
                    cancelled_rx
                        .recv_timeout(test_wait(Duration::from_secs(30)))
                        .expect("restart drain kept waiting for the measurement");
                    let result = done_rx
                        .recv_timeout(test_wait(Duration::from_secs(30)))
                        .expect("restart drain did not finish once the lane was cancelled");
                    joined = true;
                    assert_eq!(result, "restart", "restart drain returned {result}");
                    let deferred = count(
                        &s,
                        "SELECT COUNT(*) AS n FROM events WHERE action = 'land.deferred' AND json_extract(detail, '$.job') = 'draining'",
                        &[],
                    );
                    assert_eq!(
                        deferred, 1,
                        "cancelled measurement was not recorded as deferred"
                    );
                    assert!(
                        out.string().contains("measuring on the experiment lane"),
                        "daemon did not announce the experiment lane: {}",
                        out.string()
                    );
                } else {
                    assert!(
                        cancelled_rx.try_recv().is_err(),
                        "normal drain cancelled the lane before the caller cancelled"
                    );
                    cancel.cancel();
                    let result = done_rx
                        .recv_timeout(test_wait(Duration::from_secs(30)))
                        .expect("caller cancellation was ignored after drain began");
                    joined = true;
                    assert_eq!(
                        result, "interrupted",
                        "late cancellation returned {result} instead of interrupted"
                    );
                    cancelled_rx
                        .recv_timeout(test_wait(Duration::from_secs(30)))
                        .expect("daemon returned without cancelling and joining its measurement");
                }
                let job = crate::jobs::job_row(&*s.conn().unwrap(), "draining".into());
                assert_eq!(
                    job.str("branch"),
                    "evolve/draining",
                    "cancelled drain removed the unmeasured branch"
                );
                let building = crate::selfupdate::SELF_UPDATE_STATE
                    .read()
                    .unwrap()
                    .inflight;
                assert!(
                    !building
                        && count(
                            &s,
                            "SELECT COUNT(*) AS n FROM events WHERE action = ?",
                            &[&crate::selfupdate::SELF_UPDATE_EVENT]
                        ) == 0,
                    "self-update ran before the cancelled measurement finished"
                );
            }));
            cancel.cancel();
            let _ = release_tx.send(());
            if !joined {
                let _ = done_rx.recv_timeout(test_wait(Duration::from_secs(30)));
            }
            let _ = pulse_evolve::set_measure(old_measure);
            set_binary_stamp(old_stamp);
            wait_self_update_idle();
            if let Err(panic) = result {
                std::panic::resume_unwind(panic);
            }
        }
    }

    #[test]
    fn daemon_slow_measurement_does_not_block_dispatch_or_stop() {
        let _serial = crate::test_helpers::serial();
        pulse_bus::reset_default();
        let _env_guard = pulse_testutil::env_lock();
        let (s, root, self_env, _fixture) = self_update_fixture();
        let other = std::path::Path::new(&s.home).join("other");
        crate::test_helpers_selfupdate::copy_dir(std::path::Path::new(&template_repo()), &other);
        let _ = crate::board::add_project(
            &s,
            other.display().to_string(),
            "Other".into(),
            String::new(),
            false,
            String::new(),
            pulse_store::HUMAN_AGENT.into(),
            "other".into(),
        )
        .unwrap();
        join(&s, "aurora");
        crate::test_helpers_daemon::set_setting(&s, "landing", "agents");
        crate::test_helpers_daemon::set_setting(&s, "daemon.sweep", "off");
        crate::test_helpers_daemon::set_setting(&s, "jobs_per_day", "10");
        crate::test_helpers_daemon::set_setting(&s, "minutes_per_job", "1000");
        // Room for the measurement and the other project past the reserved half.
        crate::test_helpers_daemon::set_setting(&s, "capacity", "4");
        run(&root, &["git", "branch", "evolve/slow-measurement"]);
        exec1(&s,
            "INSERT INTO jobs (id, project, kind, title, status, branch, created_at, finished_at)
             VALUES ('slow-measurement', 'proj', 'experiment', 'Fake measurement', 'done', 'evolve/slow-measurement', ?, ?)",
            &[&TEST_NOW, &TEST_NOW],
        );
        exec1(
            &s,
            "INSERT INTO runs (id, job_id, agent, project, started_at, status)
             VALUES ('slow-run', 'slow-measurement', 'aurora', 'proj', ?, 'done')",
            &[&TEST_NOW],
        );
        let (fast, _) = crate::jobs::add_job(
            &s,
            "other".into(),
            "task".into(),
            "Other project can work".into(),
            "fake worker only".into(),
            0,
            100,
            "fast-test".into(),
            pulse_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        for slug in ["proj", "other"] {
            let id = format!("reflected-{slug}");
            let source = format!("reflect:{slug}:{}", &TEST_NOW[..10]);
            exec1(
                &s,
                "INSERT INTO jobs (id, project, kind, title, status, source, created_at)
                 VALUES (?, ?, 'reflect', 'Already reflected', 'done', ?, ?)",
                &[&id, &slug, &source, &TEST_NOW],
            );
        }
        let (started_tx, started_rx) = crossbeam_channel::bounded::<()>(1);
        let (release_tx, release_rx) = crossbeam_channel::bounded::<()>(1);
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0));
        let calls_for_measure = calls.clone();
        let once = std::sync::Arc::new(std::sync::Once::new());
        let once_for_measure = once.clone();
        let old = pulse_evolve::set_measure(Box::new(
            move |ctx, _s, _project, _repo, _ref_name, _opts| {
                calls_for_measure.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                once_for_measure.call_once(|| {
                    let _ = started_tx.send(());
                });
                loop {
                    if ctx.is_cancelled() {
                        anyhow::bail!("context canceled");
                    }
                    if release_rx.try_recv().is_ok() {
                        anyhow::bail!("fake measurement intentionally left unmeasured");
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            },
        ));
        let cancel = pulse_bus::CancellationToken::new();
        let s_daemon = s.clone();
        let cancel_daemon = cancel.clone();
        let self_env_daemon = self_env.clone();
        let out = OutBuf::new();
        let out_daemon = out.writer();
        let (done_tx, done_rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let result = daemon(
                &cancel_daemon,
                &s_daemon,
                DaemonOpts {
                    interval: 300,
                    worker_cmd: "sh -c 'echo SUMMARY: fake work complete'".into(),
                    out: Some(out_daemon),
                    self_update: Some(Box::new(self_env_daemon)),
                    ..Default::default()
                },
            );
            let _ = done_tx.send(result);
        });
        let release_measurement = || {
            let _ = release_tx.send(());
        };
        let mut joined = false;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            started_rx
                .recv_timeout(test_wait(Duration::from_secs(10)))
                .expect("fake measurement did not start");
            pulse_bus::emit(
                &s,
                pulse_store::HUMAN_AGENT,
                "post.question",
                "wake-control",
                serde_json::json!({"project": "other"}),
            )
            .unwrap();
            let deadline = std::time::Instant::now() + test_wait(Duration::from_secs(10));
            let mut progress = false;
            while std::time::Instant::now() < deadline {
                let raw = crate::util::must_one(
                    &*s.conn().unwrap(),
                    "SELECT value FROM meta WHERE key = 'daemon'",
                    &[],
                )
                .str("value");
                let loops = serde_json::from_str::<serde_json::Value>(&raw)
                    .ok()
                    .and_then(|v| v.get("loops").and_then(|v| v.as_i64()))
                    .unwrap_or(0);
                if loops > 1
                    && count(
                        &s,
                        "SELECT COUNT(*) AS n FROM runs WHERE job_id = ?",
                        &[&fast.str("id")],
                    ) > 0
                {
                    progress = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(
                progress,
                "a blocked measurement prevented another project's dispatch or a supervisor heartbeat"
            );
            assert_eq!(
                calls.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "started measurements while the first was still pending"
            );
            // The due self-update waits for the lane: its restart would throw
            // the running measurement away.
            let deadline = std::time::Instant::now() + test_wait(Duration::from_secs(10));
            while std::time::Instant::now() < deadline
                && !out
                    .string()
                    .contains("building once the experiment lane is idle")
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            let data = std::fs::read(&self_env.binary).unwrap_or_default();
            assert_eq!(
                String::from_utf8_lossy(&data),
                "old binary",
                "self-update installed while the experiment lane was measuring"
            );
            let (building, lane_wait) = {
                let st = crate::selfupdate::SELF_UPDATE_STATE.read().unwrap();
                (st.inflight, st.lane_wait)
            };
            assert!(
                !building && !file_exists(&format!("{}.new", self_env.binary)),
                "self-update started while the experiment lane was measuring"
            );
            assert!(
                lane_wait && crate::selfupdate::update_pending(),
                "due self-update did not record that it waits for the lane"
            );
            assert!(
                out.string()
                    .contains("building once the experiment lane is idle"),
                "daemon did not say the self-update waits for the lane: {}",
                out.string()
            );
            crate::test_helpers_daemon::set_setting(&s, "stop", "1");
            match done_rx.recv_timeout(test_wait(Duration::from_secs(60))) {
                Ok(result) => {
                    joined = true;
                    assert_eq!(result, "stopped", "STOP result = {result}");
                    let job = crate::jobs::job_row(&*s.conn().unwrap(), "slow-measurement".into());
                    assert_eq!(
                        job.str("branch"),
                        "evolve/slow-measurement",
                        "STOP discarded the unmeasured branch: {job:?}"
                    );
                }
                Err(_) => panic!("STOP could not cancel a fake blocked measurement"),
            }
        }));
        release_measurement();
        cancel.cancel();
        if !joined {
            let _ = done_rx.recv_timeout(test_wait(Duration::from_secs(10)));
        }
        let _ = pulse_evolve::set_measure(old);
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }

    /// A pass that ends while the daemon sits in its idle wait: the wait
    /// selects on the pass channel, and before the fix it swallowed the
    /// result, so the pass stayed "running" for good and no later pass ran
    /// until the daemon restarted. Stale cards added after the first pass
    /// must be retired by a second one.
    #[test]
    fn daemon_idle_wait_keeps_the_pass_result_so_the_next_pass_runs() {
        let _serial = crate::test_helpers::serial();
        pulse_bus::reset_default();
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        s.set_setting(pulse_store::HUMAN_AGENT, "jobs_per_day", "1")
            .unwrap();
        // Nothing may be planned: the daily reflect is done and a ballot was
        // just held, so the daemon's only work is the maintenance pass.
        let reflect = format!("reflect:proj:{}", &TEST_NOW[..10]);
        let ballot = format!("ballot:proj:{}", crate::util::stamp_of(TEST_NOW));
        crate::util::exec1(
            &*s.conn().unwrap(),
            "INSERT INTO jobs (id, project, kind, title, status, source, created_at) VALUES ('daily-reflect', 'proj', 'reflect', 'Already reflected', 'done', ?, ?), ('held-ballot', 'proj', 'ballot', 'Already voted', 'done', ?, ?)",
            &[&reflect, &TEST_NOW, &ballot, &TEST_NOW],
        );
        let cancel = pulse_bus::CancellationToken::new();
        let out = OutBuf::new();
        let (done_tx, done_rx) = crossbeam_channel::bounded(1);
        {
            let (s, cancel, writer) = (s.clone(), cancel.clone(), out.writer());
            std::thread::spawn(move || {
                let result = daemon(
                    &cancel,
                    &s,
                    DaemonOpts {
                        interval: 1,
                        worker_cmd: "sh -c 'echo SUMMARY: ok'".into(),
                        out: Some(writer),
                        ..Default::default()
                    },
                );
                let _ = done_tx.send(result);
            });
        }
        // The first pass writes the sweep note on its way to the finish; a
        // moment later it has ended and the daemon is back in its idle wait.
        // The sweep walks the machine's runner session dirs and asks each
        // runner for its version (up to 8s), so the note gets the same 30s
        // as the other tests that wait on it, not 10s.
        let deadline = std::time::Instant::now() + test_wait(Duration::from_secs(30));
        loop {
            let (_, exists) = s.meta("hygiene").unwrap();
            if exists {
                break;
            }
            assert!(
                std::time::Instant::now() <= deadline,
                "initial maintenance did not run"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        // Wait for the idle line itself rather than a fixed pause: under
        // full-workspace test load the first pass can outlast any sleep, and
        // posts created mid-pass are retired by it without the second pass
        // this test is about.
        let deadline = std::time::Instant::now() + test_wait(Duration::from_secs(20));
        while !out.string().contains("idle: ") {
            assert!(
                std::time::Instant::now() <= deadline,
                "the daemon did not reach its idle wait:\n{}",
                out.string()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let old = crate::util::plus_hours(TEST_NOW, -8.0 * 24.0);
        for n in 0..5 {
            crate::board::create_post(
                &s,
                old.clone(),
                crate::board::PostOpts {
                    kind: "suggestion".into(),
                    project: "proj".into(),
                    area: "evolution".into(),
                    thread: format!("gone-{n}"),
                    summary: format!("Fix gone tree {n}"),
                    body: format!(
                        "Class: product\nFile: internal/gone/file{n}.go\nVariable: x\nHypothesis: y\nSignal: check:z\n"
                    ),
                    agent: crate::board::actor_agent(&s, "aurora".into()),
                    ..Default::default()
                },
            )
            .expect("suggestion");
        }
        let dropped =
            "SELECT COUNT(*) AS n FROM posts WHERE kind = 'result' AND resolution = 'dropped'";
        let deadline = std::time::Instant::now() + test_wait(Duration::from_secs(20));
        // The pass retires the cards in the store before its lines reach the
        // daemon log, so the wait covers both: the count and the log lines.
        // The cards go in one at a time while the daemon is idle, so a pass
        // can start between two of them and retire them in two lines ("2"
        // then "3"); the lines must add up to all five, whatever the split.
        let logged = |out: &str| -> i64 {
            out.lines()
                .filter_map(|l| {
                    l.strip_prefix("retired ")?
                        .strip_suffix(" stale suggestion(s) for proj")?
                        .parse::<i64>()
                        .ok()
                })
                .sum()
        };
        let retired = loop {
            let n = crate::test_helpers::count(&s, dropped, &[]);
            if (n >= 5 && logged(&out.string()) >= 5) || std::time::Instant::now() > deadline {
                break n;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        cancel.cancel();
        let result = done_rx
            .recv_timeout(test_wait(Duration::from_secs(30)))
            .expect("the daemon did not stop after cancellation");
        let out = out.string();
        let tail: Vec<&str> = out.lines().rev().take(12).collect();
        assert_eq!(
            retired, 5,
            "no second maintenance pass ran while the fleet was idle ({result}); last lines: {tail:#?}"
        );
        assert_eq!(logged(&out), 5, "pass lines were lost:\n{out}");
    }

    #[test]
    fn daemon_maintenance_preserves_self_update() {
        let _serial = crate::test_helpers::serial();
        let _env_guard = pulse_testutil::env_lock();
        let (s, root, env, _fixture) = self_update_fixture();
        let _login = with_closed_login_gate();
        let out = once_daemon(
            &s,
            DaemonOpts {
                self_update: Some(Box::new(env.clone())),
                ..Default::default()
            },
        );
        wait_self_update_idle();
        assert!(
            out.contains("self-update: brain HEAD"),
            "a completed maintenance pass omitted the self-update phase:\n{out}"
        );
        let data = std::fs::read(&env.binary).unwrap_or_default();
        assert!(
            String::from_utf8_lossy(&data).contains(&format!("dev-{}", &head_of(&root)[..12])),
            "fake self-update did not finish: {data:?}"
        );
    }

    #[test]
    fn daemon_cancellation_during_worker_drain_keeps_grace() {
        let _serial = crate::test_helpers::serial();
        pulse_bus::reset_default();
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        s.set_setting(pulse_store::HUMAN_AGENT, "minutes_per_job", "1000")
            .unwrap();
        s.set_setting(pulse_store::HUMAN_AGENT, "jobs_per_day", "1")
            .unwrap();
        let (job, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "Fake draining worker".into(),
            "fake worker only".into(),
            0,
            100,
            "drain-test".into(),
            pulse_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let dir = std::path::Path::new(&s.home).join("worker-drain");
        std::fs::create_dir_all(&dir).unwrap();
        let fifo = dir.join("worker");
        let marker = dir.join("started");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo");
        assert!(status.success(), "mkfifo failed");
        let cmd = format!(
            "sh -c {:?}",
            format!(
                "printf ready > {}; read line < {}",
                crate::util::shell_quote(&marker.display().to_string()),
                crate::util::shell_quote(&fifo.display().to_string())
            )
        );
        let out = OutBuf::new();
        let (waiting_tx, waiting_rx) = crossbeam_channel::bounded(1);
        let writer: DaemonOut = std::sync::Arc::new(std::sync::Mutex::new(SignalWriter {
            out: out.clone(),
            waiting: waiting_tx,
            needle: "run(s) in flight to finish".into(),
            sent: false,
        }));
        let cancel = pulse_bus::CancellationToken::new();
        let cancel_for_thread = cancel.clone();
        let s_for_thread = s.clone();
        let (done_tx, done_rx) = crossbeam_channel::bounded(1);
        const GRACE: Duration = Duration::from_millis(100);
        std::thread::spawn(move || {
            let result = daemon(
                &cancel_for_thread,
                &s_for_thread,
                DaemonOpts {
                    interval: 300,
                    max_loops: 1,
                    grace: GRACE,
                    worker_cmd: cmd,
                    out: Some(writer),
                    ..Default::default()
                },
            );
            if let Err(crossbeam_channel::SendError(result)) = done_tx.send(result) {
                // The test thread already gave up (an earlier assertion panicked),
                // so surface the daemon's exit state instead of silently dropping it.
                eprintln!(
                    "late-cancellation daemon finished after the test stopped waiting: {result}"
                );
            }
        });
        waiting_rx
            .recv_timeout(test_wait(Duration::from_secs(60)))
            .expect("daemon did not enter its normal worker drain");
        let deadline = std::time::Instant::now() + test_wait(Duration::from_secs(30));
        while !marker.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            marker.exists(),
            "fake worker did not reach its blocking read"
        );
        let cancelled_at = std::time::Instant::now();
        cancel.cancel();
        let result = done_rx
            .recv_timeout(test_wait(Duration::from_secs(30)))
            .expect("late cancellation failed to bound the previously unbounded worker drain");
        assert!(
            result == "interrupted" && cancelled_at.elapsed() >= GRACE,
            "late cancellation lost its grace or exit state: {result}, elapsed {:?}",
            cancelled_at.elapsed()
        );
        assert_ne!(
            crate::jobs::job_row(&*s.conn().unwrap(), job.str("id")).str("status"),
            "leased"
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM runs WHERE status = 'running'",
                &[]
            ),
            0,
            "drain returned without joining the cancelled worker"
        );
        assert_eq!(s.setting("minutes_per_job", ""), "1000");
        assert!(out.string().contains("run(s) in flight to finish"));
    }

    fn machine(name: &str, workers: i64, projects: &str, caps: &str, reserved: bool) -> Row {
        let mut r = Row::new();
        r.insert("name", serde_json::json!(name));
        r.insert("kind", serde_json::json!("ssh"));
        r.insert("workers", serde_json::json!(workers));
        r.insert("projects", serde_json::json!(projects));
        r.insert("os", serde_json::json!("linux"));
        r.insert("labels", serde_json::json!(caps));
        r.insert("reserved", serde_json::json!(i64::from(reserved)));
        r
    }

    #[test]
    fn slots_go_to_the_reserved_the_restricted_the_plainest_and_the_least_loaded_first() {
        let mut rows = BTreeMap::new();
        rows.insert("mac".to_string(), machine("mac", 2, "*", "xcode", false));
        rows.insert("plain".to_string(), machine("plain", 2, "*", "", false));
        rows.insert("kept".to_string(), machine("kept", 1, "*", "gpu", true));
        rows.insert(
            "ios-only".to_string(),
            machine("ios-only", 1, "app", "", false),
        );
        let mut local = machine("local", 2, "*", "xcode,docker", false);
        local.insert("kind", serde_json::json!("local"));
        local.insert("os", serde_json::json!("macos"));
        local.insert("arch", serde_json::json!("arm64"));
        rows.insert("local".to_string(), local);
        let slots: Vec<String> = ["mac", "mac", "plain", "ios-only", "kept", "local"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let order: Vec<String> = slot_order(&slots, &rows)
            .into_iter()
            .map(|i| slots[i].clone())
            .collect();
        // The reserved machine, then the one kept for a project, then by what
        // they offer: the plain hand (linux) before the hand with Xcode (linux,
        // xcode) before this Mac (macos, arm64, xcode, docker); the Mac hand's
        // first free slot (half full once filled) before its second (full).
        assert_eq!(order, ["kept", "ios-only", "plain", "mac", "mac", "local"]);
        // Two plain hands, one already busier: the emptier one first.
        let mut rows = BTreeMap::new();
        rows.insert("big".to_string(), machine("big", 4, "*", "", false));
        rows.insert("small".to_string(), machine("small", 1, "*", "", false));
        let slots: Vec<String> = ["small", "big", "big"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let order: Vec<String> = slot_order(&slots, &rows)
            .into_iter()
            .map(|i| slots[i].clone())
            .collect();
        // big: 2 of 4 busy, its next slot makes it 3/4 full; small: its only
        // slot makes it full, as big's second does - the earlier in the list
        // wins that tie.
        assert_eq!(order, ["big", "small", "big"]);
    }

    #[test]
    fn unmet_needs_groups_the_queued_work_no_active_machine_can_take() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        new_bare_repo(&s);
        let now = crate::test_helpers::TEST_NOW;
        let add = |title: &str, needs: &str, minutes_ago: f64| {
            let (job, _) = crate::jobs::add_job(
                &s,
                "proj".into(),
                "task".into(),
                title.into(),
                "b".into(),
                1,
                5,
                format!("test:{title}"),
                pulse_store::HUMAN_AGENT.into(),
                false,
            )
            .unwrap();
            let id = job.str("id");
            if !needs.is_empty() {
                pulse_board::set_job_needs(&s, &id, needs, "human").unwrap();
            }
            s.conn()
                .unwrap()
                .execute(
                    "UPDATE jobs SET created_at = ? WHERE id = ?",
                    rusqlite::params![crate::util::plus_minutes(now, -minutes_ago), id],
                )
                .unwrap();
        };
        add("plain", "", 30.0);
        // Words no machine running this test offers: this one may be a Mac
        // with Xcode or a Linux box, so the groups wait for Windows and a GPU.
        add("win", "windows,visualstudio", 25.0);
        add("win-2", "windows,visualstudio", 5.0);
        add("gpu", "linux,gpu", 12.0);
        add("here", pulse_store::needs::this_os(), 40.0);
        let unmet = unmet_needs(&s, now);
        assert_eq!(
            unmet,
            vec![
                UnmetNeeds {
                    project: "proj".into(),
                    needs: "windows,visualstudio".into(),
                    jobs: 2,
                    minutes: 25
                },
                UnmetNeeds {
                    project: "proj".into(),
                    needs: "linux,gpu".into(),
                    jobs: 1,
                    minutes: 12
                },
            ],
            "this machine meets `here`; the plain job needs nothing"
        );
        // A paired Linux hand with a gpu label meets the gpu group once it is active.
        crate::executor::add_executor(&s, now, "box", "pulse@box", "/srv", 1, "*", "human")
            .unwrap();
        crate::executor::set_labels(&s, now, "box", "gpu", "human").unwrap();
        assert_eq!(
            unmet_needs(&s, now).len(),
            2,
            "a drained hand does not count"
        );
        s.conn()
            .unwrap()
            .execute(
                "UPDATE executors SET status = 'active' WHERE name = 'box'",
                [],
            )
            .unwrap();
        let unmet = unmet_needs(&s, now);
        assert_eq!(unmet.len(), 1, "{unmet:?}");
        assert_eq!(unmet[0].needs, "windows,visualstudio");
    }
}
