use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, LazyLock, RwLock};
use std::time::Duration;

type Row = crate::util::Row;

pub const DEFAULT_DAEMON_INTERVAL_SECONDS: i32 = 300;
pub(crate) const DISPATCH_TICK_SECONDS: i32 = 15;
/// How long a stopping daemon waits for the chores pass it asked last
/// (daemon_chores.rs) before cancelling it between two units; every stage of
/// it already has the daemon.stage_budget_seconds budget.
pub(crate) const CHORES_CLOSE_WAIT: Duration = Duration::from_secs(5 * 60);
/// How long a cancelled chores pass gets to reach the end of its unit before
/// the daemon leaves it behind.
pub(crate) const CHORES_CANCEL_GRACE: Duration = Duration::from_secs(30);
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
    LazyLock::new(|| RwLock::new(hive_cli::binary_stamp));
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
/// ([`hive_store::HUMAN_AGENT`]): an agent's own task or message is
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
pub(crate) fn owner_wrote_after(s: &hive_store::Store, after: i64) -> bool {
    let filter = hive_bus::Filter {
        actions: OWNER_WAKE.iter().map(|a| a.to_string()).collect(),
        ..Default::default()
    };
    hive_bus::since(s, after, filter, 0)
        .map(|events| events.iter().any(|e| e.actor == hive_store::HUMAN_AGENT))
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
    s: &hive_store::Store,
    actor: &str,
    detail: serde_json::Value,
) -> anyhow::Result<()> {
    hive_bus::emit(s, actor, RESTART_ACTION, "daemon", detail)?;
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

pub(crate) fn restart_asked(s: &hive_store::Store, after: i64) -> Option<RestartAsk> {
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
pub(crate) fn hold_cap(s: &hive_store::Store) -> Duration {
    Duration::from_secs(s.setting_int("daemon.hold_seconds").max(0) as u64)
}

/// Asks every run still `running` to hold (request_harness_hold_for_run);
/// returns the run ids asked and, per run, how many harness sessions heard
/// it (none when the worker is past its harness - in the check - or is a
/// custom command: those run to the cap and are stopped).
pub(crate) fn hold_runs_in_flight(s: &hive_store::Store) -> Vec<(String, usize)> {
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

/// Whether a restart whose drain ends with runs still in flight asks them
/// to hold rather than wait them out (`daemon.restart_hold`, on by default).
pub(crate) fn restart_hold_on(s: &hive_store::Store) -> bool {
    s.setting("daemon.restart_hold", "on") != "off"
}

/// The restart's drain is over and runs are still in flight: asks them to
/// hold as a restart asked for now does, and says what each was asked. A
/// run whose harness heard it stops at its current step within moments
/// (the harness saves its conversation and exits held), its job returns to
/// the queue with the attempt refunded and the next daemon carries it on in
/// the same clone; a run past its harness - in its run-end check, or a
/// custom worker - runs to its end, since a check stopped midway loses the
/// run and its job is done over. Before this (to 2026-11-04) the daemon
/// waited for every run to end: on the brain, where a task run takes
/// hours, ten self-updates in three days waited 0.2 to 4.3 hours each with
/// nothing dispatched, twelve hours in all, and the owner's asks to restart
/// now went unheard in that wait.
pub(crate) fn hold_for_restart(s: &hive_store::Store, inflight: usize) -> Vec<String> {
    let held = hold_runs_in_flight(s);
    let heard = held.iter().filter(|(_, n)| *n > 0).count();
    let mut lines = vec![format!(
        "restart: {inflight} run(s) in flight, {heard} asked to hold (daemon.restart_hold); a held run's job returns to the queue and the next daemon carries it on, the rest run to their end"
    )];
    for (run, n) in &held {
        lines.push(if *n > 0 {
            format!("run {run}: {n} harness session(s) asked to hold")
        } else {
            format!(
                "run {run}: no harness session running (in its check, or a custom worker); it runs to its end"
            )
        });
    }
    lines
}

/// How often the wait after the loop looks for the owner's ask to restart
/// now, so a daemon waiting for its runs hears it (the harness polls its
/// hold at the same pace).
const WAIT_ASK_POLL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
struct PlannerSnapshot {
    watermark: String,
    tally: hive_tally::TallyResult,
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

/// Set when a daemon run thread ends, however it ends (a return, an error
/// before the run row, a panic), so the lease line never waits on a run
/// that will not write its row.
struct ReturnedFlag(Arc<std::sync::atomic::AtomicBool>);

impl Drop for ReturnedFlag {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// How long the lease line waits for its run row at most; run_job writes
/// it within milliseconds of starting, so this only bounds a stuck store.
const RUN_ROW_WAIT: Duration = Duration::from_secs(10);

/// Waits until the run the daemon just started for `job_id` at `started_at`
/// has its row in `runs`, or the run thread has ended without one, or
/// RUN_ROW_WAIT passed - so "leased <job>" is printed once the run exists.
fn wait_for_run_row(
    s: &hive_store::Store,
    job_id: &str,
    started_at: &str,
    returned: &std::sync::atomic::AtomicBool,
) {
    let since = std::time::Instant::now();
    loop {
        let found = s.conn().ok().is_some_and(|c| {
            crate::util::query_one(
                &*c,
                "SELECT 1 AS x FROM runs WHERE job_id = ? AND started_at = ? LIMIT 1",
                &[&job_id, &started_at],
            )
            .ok()
            .flatten()
            .is_some()
        });
        if found
            || returned.load(std::sync::atomic::Ordering::SeqCst)
            || since.elapsed() >= RUN_ROW_WAIT
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
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
/// how many ran over the cadence (`tick`); `hive status` and the hub show
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
        .map(|(stage, took)| format!("{stage} {}", hive_cli::format_duration(*took)))
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
            hive_cli::format_duration(took),
            hive_cli::format_duration(bound)
        ))
    }

    /// The heartbeat's `tick` object; nothing before the first body ends.
    /// `longest` names the slowest body's longest stages, so `hive status`
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
    s: &hive_store::Store,
    loops: i32,
    passes: i32,
    interval: i32,
    note: String,
    restart: Option<serde_json::Value>,
    tick: Option<serde_json::Value>,
) {
    let b = hive_bus::default();
    let mut beat = serde_json::json!({
        "pid": std::process::id(),
        "at": s.now(),
        "loops": loops,
        "passes": passes,
        "interval": interval,
        "note": note,
        "started_at": DAEMON_STARTED_AT.read().expect("daemon start").clone(),
        "version": hive_cli::version(),
        "revision": hive_cli::revision(),
        "bus": {"subscribers": b.subscribers(), "dropped": b.dropped(), "deduped": b.deduped()},
        "planner_cache": hive_tally::cache::cache_stats(),
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
pub(crate) fn meta_upsert(s: &hive_store::Store, key: &str, value: &serde_json::Value) {
    if let Err(e) = meta_upsert_result(s, key, value) {
        eprintln!("daemon: meta '{key}' write failed: {e}");
    }
}

fn meta_upsert_result(
    s: &hive_store::Store,
    key: &str,
    value: &serde_json::Value,
) -> Result<(), String> {
    // A value that cannot be serialized is an error the caller logs, never an
    // empty string stored where the console expects a JSON document.
    let encoded = hive_store::gojson::to_string(value).map_err(|e| e.to_string())?;
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
    s: &hive_store::Store,
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
    s: &hive_store::Store,
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
            "hands wanted: {queued} job(s) waited {minutes} min with every slot busy or held ({slots} slot(s)); pairing another machine (hive pair) or compute.auto_approve would shorten the queue"
        ),
        230,
    );
    let body = format!(
        "The daemon on this machine found {queued} queued job(s) waiting for {minutes} minutes while every executor slot ({slots}) was busy or held by this machine's load or disk ({now}). More hands would shorten the queue: pair another machine with `hive pair` (hive record show WF-executors), set `compute.auto_approve` with `compute.max_vms` so the daemon rents one within your caps or a bigger machine (each machine sizes its own slots from its cores and memory, and the fleet adds them up). The human owns machines and money; this post is a measurement, not a request to act.\n"
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
pub(crate) fn unmet_needs(s: &hive_store::Store, now: &str) -> Vec<UnmetNeeds> {
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
                .any(|caps| hive_store::needs::met(&needs, caps))
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
    s: &hive_store::Store,
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
            "{} queued job(s) of {} need a machine that offers `{}`; none of the machines paired here does, so they wait ({} min so far, {now}). The machines and what they offer: {machines}. A machine that has it can be paired with `hive pair` (a Linux machine) or added by `hive executor add`; a paired machine that does have it can be told so with `hive executor label NAME +word` (this machine: the setting `machine.labels`); or the project's needs can change in its `.hive/hive.yaml` (`needs:`) or with `hive job needs JOB_ID -`. The daemon rents Linux machines only, so a need such as `macos` is the owner's to meet. This post is a measurement, not a request to act.\n",
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
/// is coming, or `pending` waits for the owner), the last request failed and
/// the daemon waits before the next (it asks again by itself, so the owner
/// is not asked), or the caps or the queue said no.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum NoAutoCompute {
    Open { id: String, status: String },
    Pause(String),
    Why(String),
}

impl std::fmt::Display for NoAutoCompute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open { id, status } => write!(f, "compute request {id} is {status}"),
            Self::Pause(why) | Self::Why(why) => f.write_str(why),
        }
    }
}

/// What the machine the daemon rents on its own offers: a Linux hand of the
/// provider's default size, so its OS word and the arch that size runs.
pub(crate) fn rented_caps(s: &hive_store::Store) -> std::collections::BTreeSet<String> {
    let size_setting = hive_store::compute_size_setting(&s.setting("compute.provider", ""));
    hive_store::needs::caps_of(
        "linux",
        &hive_tasks::compute::default_hands_arch(&s.setting(&size_setting, "")),
        "",
    )
}

/// Why renting again for `project` would only repeat the last rental: the
/// rented machine the daemon last gave it was marked unfit for a tool the
/// image lacks, and the project's recipe - the one thing that puts a tool on
/// a rented machine - has not changed since. Measured on 2026-11-05: CR-064
/// came up, lacked `cargo` for the brain, sat idle for 30 minutes because
/// nothing else needed Linux, was released, and the needs path rented CR-065
/// for the same job within the minute; up to three such machines a day fit
/// under `compute.max_hours_per_day`, each one as useless as the last. The
/// line returned goes into the idle note and names the fix.
pub(crate) fn rental_repeats_lack(
    s: &hive_store::Store,
    project: &str,
    now: &str,
) -> Option<String> {
    let conn = s.conn().ok()?;
    let since = crate::util::plus_hours(now, -f64::from(crate::recipe::UNFIT_HOURS));
    let rows = crate::util::must_rows(
        &conn,
        "SELECT e.at, e.subject, e.detail FROM events e WHERE e.action = 'executor.unfit' AND e.at >= ? AND EXISTS (SELECT 1 FROM compute_requests c WHERE c.executor = e.subject AND c.executor != '') ORDER BY e.seq DESC LIMIT 200",
        &[&since],
    );
    let (at, executor, tool) = rows.iter().find_map(|r| {
        let detail: serde_json::Value = serde_json::from_str(&r.str("detail")).ok()?;
        (detail["project"].as_str() == Some(project)).then(|| {
            (
                r.str("at"),
                r.str("subject"),
                detail["tool"].as_str().unwrap_or_default().to_string(),
            )
        })
    })?;
    // Only a missing tool repeats on the next machine for sure: a setup that
    // fails or a disk that is short may not, and the repair job and the
    // bigger size are other paths.
    let bare = tool.strip_prefix("tool:").unwrap_or(&tool);
    if bare.is_empty() || bare.contains(':') {
        return None;
    }
    let row = crate::util::must_one(&conn, "SELECT * FROM projects WHERE slug = ?", &[&project]);
    if recipe_changed_since(s, &row, &at) {
        return None;
    }
    Some(format!(
        "renting again would meet the same lack: {executor} {} for {project} ({at}) and the recipe of {project} has not changed since; a rented machine starts from the image, so pin the tool in `tools:` of {} (`hive env set --project {project}`), and the daemon rents again",
        crate::recipe::unfit_phrase(&tool),
        crate::recipe::RECIPE_PATH
    ))
}

/// Whether the recipe the next job of `project` would read is newer than
/// `at`: the store's copy by its revision time, else the committed file by
/// its last commit on the default branch.
fn recipe_changed_since(s: &hive_store::Store, project: &Row, at: &str) -> bool {
    let slug = project.str("slug");
    if let (_, Some(rev)) = crate::recipe::recipe_record(s, &slug) {
        return rev.at.as_str() > at;
    }
    let root = project.str("path");
    if root.is_empty() {
        return false;
    }
    let mut def = project.str("default_branch");
    if def.is_empty() {
        def = "main".to_string();
    }
    let spec = format!("refs/heads/{def}");
    let out = crate::util::git(
        &root,
        &[
            "log",
            "-1",
            "--format=%cI",
            &spec,
            "--",
            crate::recipe::RECIPE_PATH,
        ],
    );
    if !out.ok() {
        return false;
    }
    chrono::DateTime::parse_from_rfc3339(out.stdout.trim())
        .map(|t| t.with_timezone(&chrono::Utc) > crate::util::must_iso(at))
        .unwrap_or(false)
}

/// How far back from the newest failure the failures in a row count. An
/// older one says nothing about today, and a cause that still stands has
/// eight of the pause's six-hour steps inside this, so its run never looks
/// shorter than it is.
pub(crate) const FAILED_RENTALS_HOURS: i64 = 48;

/// Why the daemon does not ask for a machine again yet: the last request
/// failed before a machine was ready. First the pause a failed job's
/// recovery waits (`failure::recover_backoff`: 5 minutes, then 15, 45, 135,
/// then 6 hours), counted from the last of the failures in a row; the
/// daemon ends it by itself. Then the worker's own checks, which need no
/// request to be asked: while they would fail the next one (no hands
/// binary, a cap that allows nothing) none is made, and that is the
/// owner's to change. The failures count whoever asked and whatever each
/// said, back to `FAILED_RENTALS_HOURS` before the newest: what stops a
/// machine before it is ready (the toolchain, the provider's login, a
/// pairing step) is the hub's, not one project's, and the reasons carry
/// each request's own names. Measured on one hub: 11,034 requests in under
/// two hours, up to 243 a minute, each failed for want of a hands binary
/// and replaced on the next tick, which an approved request wakes at once;
/// before that, 56 back to back that each bought a machine, failed 5.6
/// minutes later on average and released it, 2.8 machine-hours in all. A
/// failed request gives its hours back, so no cap held either.
pub(crate) fn rental_failed_lately(s: &hive_store::Store, now: &str) -> Option<NoAutoCompute> {
    let conn = s.conn().ok()?;
    // Newest first, and more of them than the pause has steps. A request
    // the owner denied says nothing about whether a machine can be had, so
    // it ends neither the run of failures nor the pause.
    let rows = crate::util::must_rows(
        &conn,
        "SELECT r.id, r.status, r.executor, r.note, r.hours, r.updated_at, COALESCE(m.created_at, '') AS made_at, COALESCE(m.released_at, '') AS released_at, COALESCE((SELECT MIN(e.at) FROM events e WHERE e.action = ?1 AND e.subject = r.id), '') AS release_began FROM compute_requests r LEFT JOIN compute_machines m ON m.request_id = r.id WHERE r.status != 'denied' ORDER BY r.rowid DESC LIMIT 8",
        &[&crate::compute::RELEASE_BEGAN_ACTION],
    );
    let last = rows.first()?;
    let since = crate::util::must_iso(&last.str("updated_at"))
        - chrono::Duration::hours(FAILED_RENTALS_HOURS);
    let failed_before_ready = |r: &Row| r.str("status") == "failed" && r.str("executor").is_empty();
    let failed = rows
        .iter()
        .take_while(|r| {
            (failed_before_ready(r) || ran_past_hours(r).is_some())
                && crate::util::must_iso(&r.str("updated_at")) >= since
        })
        .count();
    if failed == 0 {
        return None;
    }
    let id = last.str("id");
    let due = crate::failure::recover_due(last.str("updated_at"), failed as i32 - 1);
    let overran = rows
        .iter()
        .take(failed)
        .filter(|r| ran_past_hours(r).is_some())
        .count();
    if crate::util::must_iso(now) < crate::util::must_iso(&due) {
        let what = if overran == 0 {
            format!(
                "the last {failed} request(s) failed before a machine was ready ({id}: {})",
                crate::util::trunc(&last.str("note"), 200)
            )
        } else if let Some(ran) = ran_past_hours(last) {
            format!(
                "{id}'s machine ran {ran:.1} hour(s) on {} approved and was released ({} of the last {failed} request(s) ran past their hours, the rest failed before a machine was ready)",
                last.int("hours"),
                overran
            )
        } else {
            format!(
                "the last {failed} request(s) failed before a machine was ready or ran past their hours ({id}: {})",
                crate::util::trunc(&last.str("note"), 200)
            )
        };
        return Some(NoAutoCompute::Pause(format!(
            "{what}; the daemon asks again after {due}"
        )));
    }
    if !failed_before_ready(last) {
        return None;
    }
    crate::compute::would_fail_before_provider(s, AUTO_COMPUTE_HOURS, now).map(|why| {
        NoAutoCompute::Why(format!(
            "{id} failed before a machine was ready and the next request would fail too: {why}"
        ))
    })
}

/// How long past its approved hours a machine may go and still count as released on time: the
/// compute worker lets a machine whose hours are up go on its next tick, within one daemon pass.
pub(crate) const OVERRUN_GRACE_MINUTES: i64 = 5;

/// The hours a released request's machine ran when that was more than it was approved for (by
/// more than `OVERRUN_GRACE_MINUTES`), from when it was made to when its release began (the
/// provider's delete that follows may take up to `compute::COMPUTE_RELEASE_BUDGET`; a release
/// with no such mark, as the sweep's, counts to when it was released). A machine runs on past its hours until its last
/// run ends (`compute::rentals_past_hours`); one hub replayed two that ran 5.2 and 8.0 hours on
/// 4-hour approvals, and the daemon asked for the next machine at once after each.
fn ran_past_hours(r: &Row) -> Option<f64> {
    if r.str("status") != "released"
        || r.str("made_at").is_empty()
        || r.str("released_at").is_empty()
    {
        return None;
    }
    let began = r.str("release_began");
    let until = if began.is_empty() || began > r.str("released_at") {
        r.str("released_at")
    } else {
        began
    };
    let ran = crate::util::must_iso(&until) - crate::util::must_iso(&r.str("made_at"));
    let asked = r.int("hours");
    (asked > 0
        && ran
            > chrono::Duration::hours(asked as i64)
                + chrono::Duration::minutes(OVERRUN_GRACE_MINUTES))
    .then(|| ran.num_seconds() as f64 / 3600.0)
}

/// Rents a machine for a waiting queue when the owner pre-authorised it:
/// `compute.auto_approve` within `compute.max_vms` and
/// `compute.max_hours_per_day`, one open request at a time, and only for a
/// work kind (improve, task) that has itself waited `min_wait` minutes and
/// that the rented machine could run. `waiting` says why the queue waits
/// (every slot here busy, or a need no paired machine meets) and opens the
/// request's reason. Until now the daemon only posted "hands wanted" to an
/// owner who may be away for the day while the caps said the machine could
/// be rented. A request that failed before a machine was ready is not
/// followed by the next at once (`rental_failed_lately`).
pub(crate) fn auto_compute(
    s: &hive_store::Store,
    waiting: &str,
    min_wait: i32,
    now: &str,
    actor: &str,
) -> Result<hive_tasks::compute::ComputeRequest, NoAutoCompute> {
    let why = |w: String| NoAutoCompute::Why(w);
    let caps = hive_tasks::compute::get_caps(s).map_err(|e| why(e.to_string()))?;
    if !caps.auto_approve {
        return Err(why("compute.auto_approve is off".into()));
    }
    if caps.vms_now >= caps.max_vms {
        return Err(why(format!(
            "compute.max_vms {} reached ({} in use)",
            caps.max_vms, caps.vms_now
        )));
    }
    // 0 means none, as the setting's description says and the compute worker reads it.
    if caps.hours_today.saturating_add(AUTO_COMPUTE_HOURS) > caps.max_hours_per_day {
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
    let since = crate::util::plus_minutes(now, -min_wait);
    // Only a job the rented machine can run is worth renting one for, and
    // not one whose project the last rented machine already proved too
    // bare for (`rental_repeats_lack`): the next waiting project gets the
    // machine instead.
    let (fit, words) = hive_store::needs::sql_fit("j.needs", &rented_caps(s));
    let mut vals: Vec<&dyn rusqlite::ToSql> = vec![&since];
    for w in &words {
        vals.push(w);
    }
    let candidates = crate::util::must_rows(
        &conn,
        &format!(
            "SELECT j.id, j.kind, j.project FROM jobs j JOIN projects p ON p.slug = j.project WHERE j.status = 'queued' AND p.status = 'active' AND j.kind IN ({}) AND j.created_at <= ? AND {fit} ORDER BY j.created_at LIMIT 50",
            crate::jobs::work_kinds_sql()
        ),
        &vals,
    );
    if candidates.is_empty() {
        return Err(why(format!(
            "no improve or task job that a rented Linux machine could run has waited {min_wait} min"
        )));
    }
    let mut repeats: Vec<String> = Vec::new();
    let mut seen_projects = std::collections::BTreeSet::new();
    let mut job = Row::default();
    for c in candidates {
        let project = c.str("project");
        if !seen_projects.insert(project.clone()) {
            continue;
        }
        match rental_repeats_lack(s, &project, now) {
            Some(line) => repeats.push(line),
            None => {
                job = c;
                break;
            }
        }
    }
    if job.0.is_empty() {
        return Err(why(repeats.join("; ")));
    }
    if let Some(no) = rental_failed_lately(s, now) {
        return Err(no);
    }
    let reason = format!(
        "{waiting}; {} ({}) is first in line",
        job.str("id"),
        job.str("kind")
    );
    hive_tasks::compute::request_compute(
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
    s: &hive_store::Store,
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

pub(crate) fn run_budget(s: &hive_store::Store) -> std::time::Duration {
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
            eprintln!("hive daemon: log write failed ({e}); later log lines may be lost");
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
pub(crate) fn pass_spacing(s: &hive_store::Store) -> Duration {
    Duration::from_secs(s.setting_int("daemon.pass_seconds").max(0) as u64)
}

/// Whether the governor would admit work now: read between the polls of an
/// idle sleep under a hold, so the queue moves when the load does.
pub(crate) fn hold_lifted(
    s: &hive_store::Store,
    machine: &crate::daemon_load::Machine,
    flag_max_jobs: i32,
) -> bool {
    let limits = crate::daemon_load::limits(s, machine, flag_max_jobs);
    crate::daemon_load::hold_reason(&limits, &crate::daemon_load::pressure()).is_none()
}

/// What an idle sleep watches between its polls.
pub(crate) struct IdleWatch<'a> {
    pub(crate) s: &'a hive_store::Store,
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
    wake_events: &mut Option<crossbeam_channel::Receiver<hive_bus::Event>>,
    d: Duration,
    held: bool,
) -> bool {
    let never_wake = crossbeam_channel::never::<hive_bus::Event>();
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
/// left to the loop's own provider-stop rule (hive-tally).
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
    s: &hive_store::Store,
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

/// The job id the dispatch loop's idle probe leases by: no job carries it,
/// so `take_job_first` gives its reason and leases nothing.
const NO_LEASE_PROBE: &str = "(no job: idle reason probe)";

/// Whether a worker's lease could take a job this tick: one of an active
/// project that is not a lead sitting (the leads lease their own), queued
/// or with a lapsed lease the lease puts back first - and, while the daily
/// cap is full, one of the owner's own (`OWNER_DRIVEN_SQL`). `taken` and
/// `dry_run` count the cap as `take_job_first` does. When the store cannot
/// say, the lease is tried.
fn worker_lease_possible(
    s: &hive_store::Store,
    now: &str,
    taken: &[String],
    dry_run: bool,
) -> bool {
    let Ok(c) = s.conn() else {
        return true;
    };
    let mut sql = "SELECT COUNT(*) AS n FROM jobs j JOIN projects p ON p.slug = j.project WHERE p.status = 'active' AND j.kind != ? AND (j.status = 'queued' OR (j.status = 'leased' AND j.lease_until != '' AND j.lease_until < ?))".to_string();
    if !dry_run {
        let cap = crate::run::daily_cap(s);
        if cap > 0 && crate::run::jobs_today_with(s, now, taken) >= cap {
            sql.push_str(&format!(" AND {}", crate::jobs::OWNER_DRIVEN_SQL));
        }
    }
    let lead = crate::lead::LEAD_KIND.to_string();
    let now = now.to_string();
    match crate::util::query_rows(&*c, &sql, &[&lead, &now]) {
        Ok(rows) => rows.first().is_none_or(|r| r.int("n") > 0),
        Err(_) => true,
    }
}

fn active_queued_count(s: &hive_store::Store) -> i32 {
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
    s: &hive_store::Store,
    cache: &mut BTreeMap<String, PlannerSnapshot>,
    slug: &str,
    now: &str,
) -> Option<PlannerSnapshot> {
    let watermark = hive_tally::cache::project_watermark(s, slug, "").ok()?;
    if let Some(snapshot) = cache.get(slug)
        && snapshot.watermark == watermark
    {
        return Some(snapshot.clone());
    }
    let tally = hive_tally::compute_light(
        s,
        slug,
        "",
        hive_tally::Thresholds {
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
    cancel: &hive_bus::CancellationToken,
    s: &hive_store::Store,
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
    let stale_sockets = crate::executor::ssh_sockets_reset();
    if stale_sockets > 0 {
        say(
            &o,
            format!(
                "removed {stale_sockets} shared ssh connection socket(s) left by an earlier daemon; sessions reconnect once"
            ),
        );
    }
    let mut loops = 0;
    let mut passes = 0;
    let mut last_prune_day = String::new();
    // When the last planning pass began (`pass_spacing`); None before the
    // first, which plans at once.
    let mut last_pass: Option<std::time::Instant> = None;
    // The bus position the last planning pass read up to: an owner's write
    // (OWNER_WAKE) after it makes the next tick a planning pass whatever
    // the spacing says, so a task the owner files is planned at once.
    let mut planned_through = hive_bus::last_id(s).unwrap_or(0);
    let mut prev_idle: BTreeMap<String, String> = BTreeMap::new();
    let mut idle_log = crate::idle_log::IdleLog::default();
    // The restart handover (restart.rs): armed when the binary changed on
    // disk or the owner asked for a restart while runs were in flight, it
    // says which runs may still start, what the heartbeat waits for and when
    // this daemon exits with code 3 so the service restarts it.
    let mut restart = crate::restart::Restart::default();
    // Asks for a restart are read from here on: the ask that restarted the
    // daemon before this one is behind it.
    let restart_asks_after = hive_bus::last_id(s).unwrap_or(0);
    let mut maintenance = crate::daemon_maintenance::DaemonMaintenance::default();
    let mut maintenance_wake = false;
    let mut probes = crate::checkout::CheckoutProbes::default();
    let mut refused = crate::amend::AmendRefusals::new();
    let stamp = (BINARY_STAMP.read().expect("binary stamp"))();
    let (results_tx, results_rx) = crossbeam_channel::bounded::<Dispatch>(64);
    let mut inflight: BTreeMap<String, Dispatch> = BTreeMap::new();
    let loop_ctx = cancel.child();
    let run_ctx = hive_bus::CancellationToken::new();
    let maintenance_ctx = cancel.child();
    let compute_ctx = hive_bus::CancellationToken::new();
    let (wake_events, cancel_wake) = hive_bus::default().subscribe(
        hive_bus::Filter {
            actions: DAEMON_WAKE.iter().map(|s| (*s).to_string()).collect(),
            ..Default::default()
        },
        32,
    );
    hive_tally::set_log(Some(Arc::new({
        let out = o.clone();
        move |line| say(&out, line)
    })));
    let bridge_done = hive_bus::bridge(loop_ctx.clone(), s.clone(), Duration::ZERO);
    let mut hooks_runner = hive_hooks::Runner::new(s.clone());
    hooks_runner.bus = Some(hive_bus::default());
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
    let dog_stop = hive_bus::CancellationToken::new();
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
                binary: crate::service::hive_binary(),
                cargo_cmd: "cargo".into(),
                revision: hive_cli::revision(),
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
    match hive_records::ensure_framework(s, &actor) {
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
    // PR #278 review, point 5: once per store, the open asks that name no
    // answers to pick go to their leads (counted in the log).
    if !o.dry_run {
        let slugs = s
            .conn()
            .map(|c| crate::util::must_rows(&c, "SELECT slug FROM projects ORDER BY slug", &[]))
            .unwrap_or_default()
            .into_iter()
            .map(|r| r.str("slug"))
            .collect::<Vec<_>>();
        for line in crate::lead::hand_bare_asks_once(s, &slugs, &s.now(), &actor) {
            say(&o, line);
        }
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
    // The chores off the tick (daemon_chores.rs): the Dependabot alert
    // watch, the pipeline verdicts and the compute listing run on their own
    // thread, each stage under daemon.stage_budget_seconds; the tick only
    // asks for a pass and goes on leasing.
    let mut chores = crate::daemon_chores::start(s, &actor, compute.clone(), compute_ctx.clone());
    // Cleared once the subscription closes, so a closed channel never wakes the loop again.
    let mut wake_events = Some(wake_events);
    let never_wake = crossbeam_channel::never::<hive_bus::Event>();
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
    // The other daemon-wide holds (idle_hold.rs): each tells the owner once
    // when it keeps the owner's queued work waiting past one pass, and
    // answers when it lifts.
    let mut idle_holds = crate::idle_hold::IdleHolds::default();
    // The owner's queued jobs and the holds each has waited behind: one
    // job.owner-wait event once a job has waited ten minutes (owner_wait.rs).
    let mut owner_waits = crate::owner_wait::OwnerWaits::default();
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
    let sleep_wait = |wake_events: &mut Option<crossbeam_channel::Receiver<hive_bus::Event>>,
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
        |wake_events: &mut Option<crossbeam_channel::Receiver<hive_bus::Event>>,
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
            let slept = hive_cli::format_duration(Duration::from_secs(secs));
            let hint = if awake.mode() == crate::awake::Mode::Off {
                "; daemon.keep_awake=on keeps it awake while Hive runs"
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
                        hive_cli::format_duration(cap)
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
                // A self-update's drain that ends with runs in flight holds
                // them; the console's drain without "now" waits them out, as
                // the owner chose (`hold_for_restart`).
                if !inflight.is_empty() && !restart.console_draining() && restart_hold_on(s) {
                    for line in hold_for_restart(s, inflight.len()) {
                        say(&o, line);
                    }
                }
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
        let bus_now = hive_bus::last_id(s).unwrap_or(planned_through);
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
        // Whether that express slot is under the cap (a soft hold with
        // room below it): it is then the machine's last free slot, and kept
        // back from the owner's work past its share like any other.
        let mut express_under_cap = false;
        // While owner-driven work is queued and the cap has room, the one
        // local slot reserved for it (docs/owner-reservation.md); and
        // whether that slot leased nothing this tick, so the fleet's queue
        // could not have had it either.
        let mut owner_reserved = false;
        let mut owner_slot_idle = false;
        // The daemon-wide holds this tick's dispatch saw (idle_hold.rs).
        let mut holds_seen: Vec<crate::idle_hold::Hold> = Vec::new();
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
                            say(&o, format!("retired {name}: {}", hive_board::RETIRE_REASON));
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
            // The projects whose security and pipeline stages the chores
            // pass runs: set by a planning pass, none on the ticks between.
            let mut chore_projects: Option<Vec<String>> = None;
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
                    // The build's wait for that work is bounded from the
                    // arming of a restart already pending (selfupdate gate).
                    binary.restart_armed = restart.armed_at().to_string();
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
                chore_projects = Some(slugs.clone());
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
                    for line in crate::records_sync::tend_records(s, &slug, &now, &actor) {
                        say(&o, line);
                    }
                    stages.mark("records");
                    // The decide role (ADR-072) answers the routine asks
                    // it can before the lead's pass reads what is left.
                    // Both work from one reading of the project's asks:
                    // it reads every marked post's thread.
                    let asks = crate::lead::owner_addressed_asks(s, slug.clone());
                    if !o.dry_run {
                        for line in crate::decide::tend_decide_with(s, &slug, &now, &asks) {
                            say(&o, line);
                        }
                    }
                    stages.mark("decide");
                    for line in crate::lead::tend_lead_with(
                        s,
                        slug.clone(),
                        now.clone(),
                        actor.clone(),
                        asks,
                    ) {
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
            // Security and pipeline (on a planning pass) and the compute
            // listing (every tick) run on the chores thread, off this tick.
            chores.ask(crate::daemon_chores::ChoresAsk {
                now: now.clone(),
                projects: chore_projects.take(),
            });
            stages.mark("chores");
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
            // The machine as the governor sees it this tick, read once: the
            // yield below asks only what the governor would then admit.
            let pressure = crate::daemon_load::pressure();
            let limits = crate::daemon_load::limits(s, &machine, o.max_jobs);
            let owner_running_local = inflight
                .values()
                .filter(|d| {
                    d.slot == crate::executor::LOCAL_EXECUTOR
                        && crate::jobs::is_owner_driven(&d.job)
                })
                .count();
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
            // A fleet run in the way of the owner's queued job is asked to
            // hold only when that job could start once its lane is free
            // (docs/owner-reservation.md): not during a console drain, which
            // starts nothing, nor while the disk, the login or the machine
            // would refuse it anyway - a yield then only throws away the
            // fleet run's step and leases it straight back.
            if !o.dry_run {
                let everywhere = if restart_starts_nothing {
                    restart_hold.clone()
                } else if let Some(hold) = &bulk_hold {
                    Some(hold.clone())
                } else if disk_floor.is_some() {
                    crate::executor::disk_floor_note(s)
                } else {
                    None
                };
                let running_local = inflight
                    .values()
                    .filter(|d| d.slot == crate::executor::LOCAL_EXECUTOR)
                    .count();
                let measuring = crate::daemon_load::measuring(&s.home);
                let update_waits = usize::from(crate::selfupdate::update_pending());
                let local = if !gate.is_empty() {
                    Some(gate.clone())
                } else {
                    crate::daemon_load::owner_refusal(
                        &limits,
                        &pressure,
                        running_local,
                        measuring,
                        update_waits,
                        owner_running_local,
                    )
                };
                // The slots the dispatch below would offer the owner's job
                // with no run asked to hold: the paired machines' free
                // slots, and this machine's as far as the governor's room
                // goes (none behind the login gate, which closed them).
                let local_offered = slots
                    .iter()
                    .filter(|n| *n == crate::executor::LOCAL_EXECUTOR)
                    .count();
                let local_room = crate::daemon_load::owner_room(
                    &limits,
                    &pressure,
                    running_local,
                    measuring,
                    update_waits,
                    owner_running_local,
                );
                let free_slots = slots.len() - local_offered + local_offered.min(local_room);
                // Whether the slot a local run frees would be the machine's
                // last free one under the cap, which the dispatch keeps back
                // from the owner's work past its share (keep_slot below):
                // the yield asks no local run to hold for such a job, since
                // the freed slot would refuse it (review of PR 276, item B).
                let after = running_local.saturating_sub(1);
                let local_kept = crate::daemon_load::owner_room(
                    &limits,
                    &pressure,
                    after,
                    measuring,
                    update_waits,
                    owner_running_local,
                ) <= 1
                    && crate::daemon_load::under_cap(
                        &limits,
                        &pressure,
                        after,
                        measuring,
                        update_waits,
                    );
                let pass = crate::yield_lane::yield_lanes_if_admitted(
                    s,
                    &now,
                    &actor,
                    &crate::yield_lane::OwnerAdmission {
                        everywhere,
                        local,
                        free_slots: Some(free_slots),
                        local_kept,
                    },
                );
                for line in pass.asked {
                    say(&o, line);
                }
                // Said once per change, not per tick: the hold that refuses
                // the owner's job stands for hours at a time.
                if pass.refused.is_empty() {
                    idle_log.clear("yield");
                } else if let Some(line) =
                    idle_log.line("yield", &pass.refused.join("; "), std::time::Instant::now())
                {
                    say(&o, line);
                }
            }
            if let Some(hold) = &restart_hold
                && let Some(line) = idle_log.line("restart", hold, std::time::Instant::now())
            {
                say(&o, line);
            }
            // Only a console drain leaves the window with nothing to start
            // (restart_fits); a binary change keeps the slots open to the
            // owner's work at least.
            if let Some(hold) = restart_hold.clone().filter(|_| restart_kinds.is_empty()) {
                holds_seen.push(crate::idle_hold::Hold::new(
                    &crate::idle_hold::RESTART_DRAIN,
                    &hold,
                ));
                reason = hold;
                slots.clear();
            } else if let Some(hold) = bulk_hold {
                holds_seen.push(crate::idle_hold::Hold::new(
                    &crate::idle_hold::BULK_AWAY,
                    &hold,
                ));
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
                holds_seen.push(crate::idle_hold::Hold::new(
                    &crate::idle_hold::DISK_FLOOR,
                    &reason,
                ));
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
                // And while owner-driven work this machine could run is
                // queued, one local slot is the owner's reservation
                // (docs/owner-reservation.md): under the cap the fleet
                // cannot take it, over the cap or the load or memory line
                // the owner's one run starts there all the same.
                if slots.iter().any(|n| n == crate::executor::LOCAL_EXECUTOR) {
                    let running_local = inflight
                        .values()
                        .filter(|d| d.slot == crate::executor::LOCAL_EXECUTOR)
                        .count();
                    let owner_running = owner_running_local;
                    let reserved = usize::from(crate::selfupdate::update_pending());
                    let measuring_local = crate::daemon_load::measuring(&s.home);
                    work_reserve = (limits.max_jobs.max(0) as usize).div_ceil(2);
                    let verdict = crate::daemon_load::decide(
                        &limits,
                        &pressure,
                        running_local,
                        measuring_local,
                        reserved,
                        owner_running,
                    );
                    let reservation = crate::daemon_load::reserve_for_owner(
                        &verdict,
                        &pressure,
                        crate::jobs::owner_work_queued_here(s, &now),
                        owner_running,
                    );
                    if let Some(hold) = verdict.hold {
                        holds_seen.push(crate::idle_hold::Hold::new(
                            crate::idle_hold::governor_kind(&hold),
                            &hold,
                        ));
                        // Inside a restart window too: the owner's work
                        // starts in any window, and its run anchors the
                        // window rather than outlast it (anchors_restart).
                        if verdict.express || reservation.is_some() {
                            // The slot kept is restricted, so the tick still
                            // counts as held: an empty lane is not starvation.
                            load_held = crate::daemon_load::trim_local_slots(&mut slots, 1).max(1);
                            express_slot = true;
                            express_under_cap = crate::daemon_load::under_cap(
                                &limits,
                                &pressure,
                                running_local,
                                measuring_local,
                                reserved,
                            );
                            load_hold = Some(format!(
                                "{hold}; one express slot stays open for the owner's work"
                            ));
                        } else {
                            load_held = crate::daemon_load::trim_local_slots(&mut slots, 0);
                            load_hold = Some(hold);
                        }
                    } else if verdict.express || (verdict.room == 0 && reservation.is_some()) {
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
                        holds_seen.push(crate::idle_hold::Hold::new(
                            &crate::idle_hold::FLEET_FULL,
                            load_hold.clone().unwrap_or_default(),
                        ));
                    } else {
                        // Over the cap is capacity the owner chose not to use, not
                        // a hold: those slots are simply not there this tick.
                        crate::daemon_load::trim_local_slots(&mut slots, verdict.room);
                        // The owner's reservation under the cap: the first
                        // local slot offered takes owner-driven work alone.
                        owner_reserved = reservation.is_some();
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
                        // Every slot is taken; when the fleet's own work
                        // holds all of them, a long wait of the owner's is
                        // the owner's question (idle_hold.rs FLEET_FULL).
                        let busy = "every executor slot is busy".to_string();
                        if crate::idle_hold::fleet_holds_every_slot(s) {
                            holds_seen.push(crate::idle_hold::Hold::new(
                                &crate::idle_hold::FLEET_FULL,
                                format!("{busy} with the fleet's own work"),
                            ));
                        }
                        busy
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
                    // The owner's reservation: the first local slot offered
                    // leases owner-driven work alone (lease_job takes it
                    // first anyway); the fleet's work takes the rest.
                    let mut owner_slot_pending = owner_reserved;
                    for i in order {
                        let slot = &slots[i];
                        let owner_only = slot == crate::executor::LOCAL_EXECUTOR
                            && (express_slot || owner_slot_pending);
                        let fits = if owner_only {
                            &owner_lane
                        } else {
                            &restart_kinds
                        };
                        // This and the local slots still to fill this tick
                        // are the last of the cap: the work class goes first.
                        let work_first = slot == crate::executor::LOCAL_EXECUTOR
                            && local_free - local_leased <= work_reserve;
                        // The last local slot free is kept back from the
                        // owner's work past its project's share (jobs.rs
                        // LeaseOpts::keep_slot): with every slot on the
                        // owner's tasks, the lead's sitting for the owner's
                        // message, an answer or a review would wait for a
                        // task to end. The express slot over a full cap is
                        // one past it and the owner's already; under a soft
                        // hold with room below the cap it is the machine's
                        // last free slot and kept like any other (review of
                        // PR 276, item D).
                        let keep_slot = slot == crate::executor::LOCAL_EXECUTOR
                            && (!express_slot || express_under_cap)
                            && local_free - local_leased <= 1;
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
                        // The disk paid for this slot: it is the owner's
                        // reservation, offered to owner-driven work alone.
                        if slot == crate::executor::LOCAL_EXECUTOR && owner_slot_pending {
                            owner_slot_pending = false;
                            owner_slot_idle = true;
                        }
                        // The jobs this tick leased already: their run rows
                        // land from their own threads, so the cap takes
                        // the ids and counts each job once.
                        let taken = started.iter().map(|d| d.job.str("id")).collect::<Vec<_>>();
                        let (lead, job, why) = crate::lead::lead_job_for(
                            s,
                            &names,
                            o.dry_run,
                            now.clone(),
                            slot.clone(),
                            &taken,
                            &held,
                            fits,
                        );
                        let (worker, job) = if !job.0.is_empty() {
                            (lead, job)
                        } else {
                            if !why.is_empty() {
                                reasons.insert(why, true);
                            }
                            // A lease that can take nothing (an empty queue,
                            // or a full daily cap with none of the owner's
                            // work queued) gets no worker: `pick_worker`
                            // would spawn a fresh one that never runs and so
                            // is never retired. The lease is still asked,
                            // for a job no row carries, so the idle line
                            // keeps its reason.
                            if o.agent.is_empty()
                                && !worker_lease_possible(s, &now, &taken, o.dry_run)
                            {
                                match crate::run::take_job_first(
                                    s,
                                    "",
                                    "",
                                    NO_LEASE_PROBE,
                                    o.dry_run,
                                    &now,
                                    slot,
                                    &taken,
                                    &held,
                                    fits,
                                    work_first,
                                    keep_slot,
                                ) {
                                    Ok((_, _, why)) if !why.is_empty() => {
                                        reasons.insert(why, true);
                                    }
                                    Ok(_) => {}
                                    Err(e) => {
                                        reasons.insert(crate::run::err_message(&e), true);
                                    }
                                }
                                continue;
                            }
                            let worker = match crate::run::pick_worker_on(s, &o.agent, &names, slot)
                            {
                                Ok(worker) => worker,
                                Err(e) => {
                                    reasons.insert(crate::run::err_message(&e), true);
                                    continue;
                                }
                            };
                            let (job, _, why) = match crate::run::take_job_first(
                                s, &worker, "", "", o.dry_run, &now, slot, &taken, &held, fits,
                                work_first, keep_slot,
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
                            if owner_only {
                                owner_slot_idle = false;
                            }
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
                        let leased_line = format!("leased {} after {}s", job.str("id"), wait_s);
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
                        let run_returned = Arc::new(std::sync::atomic::AtomicBool::new(false));
                        let returned_run = Arc::clone(&run_returned);
                        let (lease_job, lease_at) = (d.job.str("id"), d.started_at.clone());
                        std::thread::Builder::new()
                            .name("daemon-run".into())
                            .spawn(move || {
                                let mut d = d;
                                let _returned = ReturnedFlag(returned_run);
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
                        // The lease line means the run exists: whoever reacts
                        // to it (the hub, a test reading the runs table) finds
                        // the run's row, not a gap before run_job writes it.
                        wait_for_run_row(s, &lease_job, &lease_at, &run_returned);
                        say(&o, leased_line);
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
            let usable = offered
                .saturating_sub(usize::from(express_slot && started.is_empty()))
                .saturating_sub(usize::from(owner_slot_idle));
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
            // waits whatever the slots do. A need a rented Linux hand meets
            // rents one when the owner pre-authorised it (before this only a
            // queue with every slot busy rented, so a Mac with free slots
            // and a `linux` job told the owner "machine wanted" and left
            // the job queued for good); the owner hears once per project
            // and needs, after the job has waited long enough not to be a
            // manifest the owner is still editing, for the needs the daemon
            // cannot rent or may not.
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
                    let rented = rented_caps(s);
                    let (rentable, owners): (Vec<UnmetNeeds>, Vec<UnmetNeeds>) = unmet
                        .into_iter()
                        .partition(|u| hive_store::needs::met(&u.needs, &rented));
                    let mut ask = owners;
                    if !rentable.is_empty() {
                        let waiting = rentable
                            .iter()
                            .map(|u| {
                                format!(
                                    "{} job(s) of {} waited {} min for a machine that offers {}, which no paired machine does",
                                    u.jobs, u.project, u.minutes, u.needs
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("; ");
                        match auto_compute(s, &waiting, NEEDS_WANTED_MINUTES, &now, &actor) {
                            Ok(req) => say(
                                &o,
                                format!(
                                    "machine wanted: compute request {} ({}, {} for {} h) made for {}: {}",
                                    req.id, req.status, req.size, req.hours, req.project, req.reason
                                ),
                            ),
                            // A machine is on its way: no question to the owner.
                            Err(NoAutoCompute::Open { id, status }) if status != "pending" => {
                                if let Some(line) = idle_log.line(
                                    "needs-rent",
                                    &format!("machine wanted: compute request {id} is {status}; waiting for the machine"),
                                    std::time::Instant::now(),
                                ) {
                                    say(&o, line);
                                }
                            }
                            // The daemon asks again by itself: no question either.
                            Err(NoAutoCompute::Pause(why)) => {
                                if let Some(line) = idle_log.line(
                                    "needs-rent",
                                    &format!("machine wanted: {why}"),
                                    std::time::Instant::now(),
                                ) {
                                    say(&o, line);
                                }
                            }
                            Err(why) => {
                                if let Some(line) = idle_log.line(
                                    "needs-rent",
                                    &format!("machine wanted: the daemon rents none ({why})"),
                                    std::time::Instant::now(),
                                ) {
                                    say(&o, line);
                                }
                                ask.extend(rentable);
                            }
                        }
                    }
                    for post in needs_wanted(
                        s,
                        crate::board::brain_project(c),
                        &ask,
                        now.clone(),
                        actor.clone(),
                    ) {
                        say(&o, format!("machine wanted: {}", post.str("summary")));
                    }
                } else {
                    idle_log.clear("needs");
                    idle_log.clear("needs-rent");
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
            // and hears again when it lifts (daily_cap.rs). The jobs this
            // tick started count whether or not their run rows are in yet,
            // as the lease counted them.
            if !o.dry_run
                && let Some(c) = conn.as_ref()
            {
                let cap = crate::run::daily_cap(s);
                let started_ids = started.iter().map(|d| d.job.str("id")).collect::<Vec<_>>();
                let done = crate::run::jobs_today_with(s, &now, &started_ids);
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
            // The other daemon-wide holds, where the idle log said them
            // above: one owner question once a hold has kept the owner's
            // queued work waiting past one pass, answered when it lifts.
            if !o.dry_run
                && let Some(c) = conn.as_ref()
            {
                let owner = crate::idle_hold::owner_waiting(s, &now);
                // The machine as the governor sees it after this tick's
                // dispatch: a load or memory hold is the owner's question
                // only where the governor itself would refuse the owner.
                let local_runs = inflight
                    .values()
                    .filter(|d| d.slot == crate::executor::LOCAL_EXECUTOR);
                let governor = crate::idle_hold::GovernorTick {
                    limits,
                    pressure,
                    running: local_runs.clone().count(),
                    measuring: crate::daemon_load::measuring(&s.home),
                    reserved: usize::from(crate::selfupdate::update_pending()),
                    owner_running: local_runs
                        .filter(|d| crate::jobs::is_owner_driven(&d.job))
                        .count(),
                };
                for line in idle_holds.tend(
                    s,
                    &crate::board::brain_project(c),
                    &holds_seen,
                    &governor,
                    &owner,
                    spacing,
                    &now,
                    &actor,
                    std::time::Instant::now(),
                ) {
                    say(&o, line);
                }
                let reasons = crate::owner_wait::tick_reasons(
                    &holds_seen,
                    cap_was_holding == Some(true),
                    &reason,
                );
                for line in owner_waits.tend(s, &reasons, &now, &actor) {
                    say(&o, line);
                }
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
                    let waiting = format!(
                        "{queued} job(s) waited {waited} min with every slot on this machine busy or held"
                    );
                    match auto_compute(s, &waiting, HANDS_WANTED_MINUTES, &now, &actor) {
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
                        // The daemon asks again by itself: no question either.
                        Err(NoAutoCompute::Pause(why)) => {
                            if let Some(line) = idle_log.line(
                                "hands",
                                &format!("hands wanted: {why}"),
                                std::time::Instant::now(),
                            ) {
                                say(&o, line);
                            }
                        }
                        Err(why) => {
                            // Said whether or not the question below is
                            // new: with one open already nothing else
                            // names the reason.
                            if let Some(line) = idle_log.line(
                                "hands",
                                &format!("hands wanted: the daemon rents none ({why})"),
                                std::time::Instant::now(),
                            ) {
                                say(&o, line);
                            }
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
        for line in chores.drain() {
            say(&o, line);
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
        // The owner's ask to restart now, written while this daemon waits
        // for its runs, is heard here: before this it was read only by the
        // loop, so a daemon in this wait sat on the ask for as long as its
        // runs took (on 2026-11-04, seven asks in a day, one unheard for
        // hours while a task run was in its check). The asks that count are
        // the ones after this point, and the point is fixed before the line
        // below says that the daemon waits: an ask written the moment that
        // line appears (the console shows it) must not fall behind it.
        let asks_after = hive_bus::last_id(s).unwrap_or(0);
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
        let ask_tick = if result == "restart" && !restart.forced() {
            crossbeam_channel::tick(WAIT_ASK_POLL)
        } else {
            crossbeam_channel::never()
        };
        let wait_beat = |inflight: &BTreeMap<String, Dispatch>,
                         restart: &crate::restart::Restart| {
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
        wait_beat(&inflight, &restart);
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
                recv(beat_tick) -> _ => wait_beat(&inflight, &restart),
                recv(ask_tick) -> _ => {
                    if !restart.forced()
                        && let Some(ask) = restart_asked(s, asks_after)
                        && ask.force
                    {
                        let held = hold_runs_in_flight(s);
                        let heard = held.iter().filter(|(_, n)| *n > 0).count();
                        // The cap then stops the runs as a restart asked for
                        // now before the loop ended does, and the heartbeat
                        // says the restart is now forced.
                        restart.force(s, &inflight);
                        grace = hold_cap(s);
                        timer = Some(crossbeam_channel::after(grace));
                        say(&o, format!(
                            "restart asked by {} at {} while this daemon waits for {} run(s) in flight: {heard} asked to hold; stopping the rest after {}, their jobs return to the queue",
                            ask.actor,
                            ask.at,
                            inflight.len(),
                            hive_cli::format_duration(grace)
                        ));
                    }
                },
                recv(caller_done.as_ref().unwrap_or(&never_done)) -> _ => {
                    caller_done = None;
                    if timer.is_none() {
                        grace = grace_for("interrupted");
                        timer = Some(crossbeam_channel::after(grace));
                    }
                },
                recv(timer.as_ref().unwrap_or(&never_tick)) -> _ => {
                    if restart.forced() && result == "restart" {
                        say(&o, format!("hold cap of {} over: stopping {} run(s) still in flight; their jobs return to the queue", hive_cli::format_duration(grace), inflight.len()));
                    } else {
                        say(&o, format!("grace of {} over: killing {} run(s) still in flight", hive_cli::format_duration(grace), inflight.len()));
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
                    say(&o, format!("retired {name}: {}", hive_board::RETIRE_REASON));
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
        let asks_after = hive_bus::last_id(s).unwrap_or(0);
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
                    hive_cli::format_duration(drain)
                ),
            );
        }
    }
    maintenance_ctx.cancel();
    if cancel.is_cancelled() && result != "stopped" {
        result = "interrupted";
        grace = grace_for(result);
    }
    // The chores pass asked last still runs - its compute stage starts what
    // the last tick saw approved - within CHORES_CLOSE_WAIT.
    if chores.busy() {
        let waiting = format!(
            "{note}: waiting for the daemon chores pass ({})",
            chores.stage()
        );
        say(&o, &waiting);
        enter(waiting.clone());
    }
    for line in chores.close(CHORES_CLOSE_WAIT, CHORES_CANCEL_GRACE, &|| {
        enter(format!("{note}: waiting for the daemon chores pass"));
    }) {
        say(&o, line);
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
struct CancelOnDrop(hive_bus::CancellationToken);

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
        exec1, finished_job_of_kind, finished_run, queued_job, store_with_projects, test_wait,
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

    /// `brain.code none` (ADR-070 rule 6): after a full pass the brain's
    /// work dir holds no checkout of the project. The reflection, which
    /// plans from the board, ran in an empty workspace; the task, which
    /// needs the code, still waits for a hand.
    #[test]
    fn brain_code_none_leaves_no_project_checkout_after_a_pass() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        new_bare_repo(&s);
        s.set_setting(hive_store::HUMAN_AGENT, "brain.code", "none")
            .unwrap();
        let add = |kind: &str, title: &str| {
            crate::jobs::add_job(
                &s,
                "proj".into(),
                kind.into(),
                title.into(),
                "fake worker only".into(),
                0,
                100,
                "test".into(),
                hive_store::HUMAN_AGENT.into(),
                false,
            )
            .unwrap()
            .0
        };
        let task = add("task", "Needs the code");
        let reflect = add(crate::suggestions::REVIEW_KIND, "Plans from the board");
        let seen = tempfile::tempdir().unwrap();
        let listing = seen.path().join("listing.txt");
        let out = once_daemon(
            &s,
            DaemonOpts {
                worker_cmd: format!(
                    "sh -c {}",
                    crate::util::shell_quote(&format!(
                        "ls -A > {}; echo SUMMARY: planned",
                        crate::util::shell_quote(&listing.display().to_string())
                    ))
                ),
                ..Default::default()
            },
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM runs WHERE job_id = ? AND status = 'done'",
                &[&reflect.str("id")]
            ),
            1,
            "the reflection runs on the brain\n{out}"
        );
        let status = crate::util::must_one(
            &s.conn().unwrap(),
            "SELECT status FROM jobs WHERE id = ?",
            &[&task.str("id")],
        )
        .str("status");
        assert_eq!(status, "queued", "the task waits for a hand\n{out}");
        // The worker's workspace held nothing of the project.
        let seen_files = std::fs::read_to_string(&listing).unwrap();
        assert_eq!(seen_files.trim(), ".git", "{seen_files}\n{out}");
        // No checkout of the project anywhere under the work dir.
        fn readmes(dir: &std::path::Path, found: &mut Vec<String>) {
            for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() && !p.is_symlink() {
                    readmes(&p, found);
                } else if p.file_name().is_some_and(|n| n == "README.md") {
                    found.push(p.display().to_string());
                }
            }
        }
        let mut found = Vec::new();
        readmes(&std::path::Path::new(&s.home).join("work"), &mut found);
        assert!(found.is_empty(), "checkouts under work: {found:?}\n{out}");
        // A run asked for by hand is refused the same way.
        let err = crate::exec::make_executor(
            crate::executor::executor_row(&s, crate::executor::LOCAL_EXECUTOR),
            &s,
            &s.home,
            task.clone(),
            crate::board::project_by_slug(&s.conn().unwrap(), "proj".into()),
            "aurora",
            &mut crate::exec::RunLog::default(),
        )
        .err()
        .expect("a task is refused on a brain without code");
        assert!(err.to_string().contains("brain.code is none"), "{err}");
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
            hive_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        s.set_setting(
            hive_store::HUMAN_AGENT,
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

    #[cfg(unix)]
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
            hive_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        // The bulk link points at an unplugged volume; `work` goes through it.
        let home = std::path::Path::new(&s.home);
        let gone = std::path::Path::new("/Volumes/NotPluggedIn/hive");
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
                .contains("/Volumes/NotPluggedIn/hive"),
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
            hive_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        s.set_setting(hive_store::HUMAN_AGENT, "daemon.min_free_mb", "0")
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
                hive_store::HUMAN_AGENT.into(),
                false,
            )
            .unwrap();
        }
        s.set_setting(hive_store::HUMAN_AGENT, "capacity", "2")
            .unwrap();
        s.set_setting(hive_store::HUMAN_AGENT, "daemon.min_free_mb", "0")
            .unwrap();
        // A footprint no disk pays for twice: the first run starts because
        // nothing is in flight; the second local slot waits for headroom.
        s.set_setting(hive_store::HUMAN_AGENT, "daemon.setup_mb", "2000000000")
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

    /// TASK-067 (c): the owner's job held past ten minutes gets one
    /// job.owner-wait event naming the hold, through a real tick. Past twice
    /// the load line the reservation (a) still starts the owner's one run;
    /// the owner's next step, queued twenty minutes ago by the store's
    /// clock, waits behind that run and the load, and the event says so.
    #[test]
    fn daemon_names_the_holds_of_an_owner_job_queued_past_ten_minutes() {
        let _serial = crate::test_helpers::serial();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        for (k, v) in [
            ("daemon.sweep", "off"),
            ("capacity", "2"),
            ("daemon.min_free_mb", "0"),
        ] {
            s.set_setting(hive_store::HUMAN_AGENT, k, v).unwrap();
        }
        for (id, title, at) in [
            ("job-owner-first", "Owner's task", "2026-09-06T09:30:00Z"),
            ("job-owner", "Owner step", "2026-09-06T09:40:00Z"),
        ] {
            crate::util::exec1(
                &s.conn().unwrap(),
                "INSERT INTO jobs (id, project, kind, title, brief, source, status, created_at) VALUES (?, 'proj', 'task', ?, 'fake worker only', 'task:TASK-055', 'queued', ?)",
                &[&id, &title, &at],
            );
        }
        let saturated = crate::daemon_load::pin_pressure(crate::daemon_load::Pressure {
            load1: Some(60.2),
            ..crate::daemon_load::CALM
        });
        let out = once_daemon(
            &s,
            DaemonOpts {
                worker_cmd: "sh -c 'echo SUMMARY: ok'".into(),
                ..Default::default()
            },
        );
        drop(saturated);
        assert!(out.contains("leased job-owner-first after"), "{out}");
        assert!(!out.contains("leased job-owner after"), "{out}");
        assert!(
            out.contains("owner job job-owner (proj) waiting since 09:40 UTC: load 60 > 24"),
            "{out}"
        );
        let row = crate::util::query_one(
            &s.conn().unwrap(),
            "SELECT detail FROM events WHERE action = 'job.owner-wait' AND subject = 'job-owner'",
            &[],
        )
        .unwrap()
        .expect("one job.owner-wait event");
        let d: serde_json::Value = serde_json::from_str(&row.str("detail")).unwrap();
        assert_eq!(d["reasons"], serde_json::json!(["load 60 > 24"]), "{d}");
        assert_eq!(d["waited_s"], serde_json::json!(1200), "{d}");
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM events WHERE action = 'job.owner-wait'",
                &[]
            ),
            1,
            "the run that started gets no wait event"
        );
        let lines = crate::owner_wait::status_lines(&s);
        assert_eq!(
            lines,
            vec!["proj job-owner (Owner step) waiting since 09:40 UTC: load 60 > 24".to_string()]
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
            hive_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        s.set_setting(hive_store::HUMAN_AGENT, "capacity", "2")
            .unwrap();
        s.set_setting(hive_store::HUMAN_AGENT, "daemon.min_free_mb", "0")
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
            hive_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        s.set_setting(hive_store::HUMAN_AGENT, "capacity", "2")
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
        const STARVED: &str = "1 job(s) waited 31 min with every slot on this machine busy or held";
        let why = |w: &str| Err(NoAutoCompute::Why(w.into()));
        assert_eq!(
            auto_compute(&s, STARVED, HANDS_WANTED_MINUTES, TEST_NOW, "daemon"),
            why("compute.auto_approve is off")
        );
        s.set_setting(hive_store::HUMAN_AGENT, "compute.auto_approve", "1")
            .unwrap();
        assert_eq!(
            auto_compute(&s, STARVED, HANDS_WANTED_MINUTES, TEST_NOW, "daemon"),
            why("compute.max_vms 0 reached (0 in use)")
        );
        for (k, v) in [("compute.max_vms", "1"), ("compute.max_hours_per_day", "2")] {
            s.set_setting(hive_store::HUMAN_AGENT, k, v).unwrap();
        }
        assert_eq!(
            auto_compute(&s, STARVED, HANDS_WANTED_MINUTES, TEST_NOW, "daemon"),
            why("compute.max_hours_per_day 2 would be passed (0 h today, 4 h asked)")
        );
        // 0 means none, as the setting says: no machine is rented.
        s.set_setting(hive_store::HUMAN_AGENT, "compute.max_hours_per_day", "0")
            .unwrap();
        assert_eq!(
            auto_compute(&s, STARVED, HANDS_WANTED_MINUTES, TEST_NOW, "daemon"),
            why("compute.max_hours_per_day 0 would be passed (0 h today, 4 h asked)")
        );
        s.set_setting(hive_store::HUMAN_AGENT, "compute.max_hours_per_day", "8")
            .unwrap();
        exec1(
            &s,
            "UPDATE jobs SET kind = 'reflect' WHERE project = 'proj'",
            &[],
        );
        assert_eq!(
            auto_compute(&s, STARVED, HANDS_WANTED_MINUTES, TEST_NOW, "daemon"),
            why("no improve or task job that a rented Linux machine could run has waited 30 min")
        );
        exec1(
            &s,
            "UPDATE jobs SET kind = 'task' WHERE project = 'proj'",
            &[],
        );
        let req = auto_compute(&s, STARVED, HANDS_WANTED_MINUTES, TEST_NOW, "daemon")
            .expect("no machine rented");
        assert_eq!(
            (req.project.as_str(), req.hours),
            ("proj", AUTO_COMPUTE_HOURS)
        );
        assert_eq!(req.requested_by, "daemon");
        assert!(req.reason.contains("waited 31 min"), "{}", req.reason);
        // Approved on the spot where a login source exists; a proposal to
        // the owner where none does (the machine could not log in).
        let login = hive_tasks::get_caps(&s).unwrap().login_source;
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
            auto_compute(&s, STARVED, HANDS_WANTED_MINUTES, TEST_NOW, "daemon"),
            Err(open),
            "a second machine while the first is open"
        );
    }

    #[test]
    fn a_lack_the_last_rented_machine_had_is_not_rented_for_again_until_the_recipe_changes() {
        let _serial = crate::test_helpers::serial();
        let s = store_with_projects(&["proj", "other"]);
        for (k, v) in [
            ("compute.auto_approve", "1"),
            ("compute.max_vms", "1"),
            ("compute.max_hours_per_day", "24"),
        ] {
            s.set_setting(hive_store::HUMAN_AGENT, k, v).unwrap();
        }
        const WAITED: &str = "1 job(s) of proj waited 31 min for a machine that offers linux";
        queued_job(&s, "j-proj", "proj", "task", "owner");
        exec1(
            &s,
            "UPDATE jobs SET created_at = ?",
            &[&crate::util::plus_minutes(TEST_NOW, -31)],
        );
        // The rental before this tick: made for proj, paired as hands-cr-1,
        // marked unfit for lacking cargo, released for idling (CR-064).
        let first = auto_compute(&s, WAITED, HANDS_WANTED_MINUTES, TEST_NOW, "daemon")
            .expect("the first rental");
        exec1(
            &s,
            "UPDATE compute_requests SET status = 'released', executor = 'hands-cr-1' WHERE id = ?",
            &[&first.id],
        );
        crate::recipe::mark_unfit(
            &*s.conn().unwrap(),
            &crate::util::plus_minutes(TEST_NOW, -40),
            "hands-cr-1",
            "proj",
            "cargo",
        );
        let err = auto_compute(&s, WAITED, HANDS_WANTED_MINUTES, TEST_NOW, "daemon")
            .expect_err("the same lack again");
        let NoAutoCompute::Why(why) = &err else {
            panic!("{err}");
        };
        assert!(
            why.starts_with(
                "renting again would meet the same lack: hands-cr-1 lacks cargo for proj ("
            ) && why.contains("hive env set --project proj"),
            "{why}"
        );
        // A project that never had the machine gets it instead.
        queued_job(&s, "j-other", "other", "task", "owner");
        exec1(
            &s,
            "UPDATE jobs SET created_at = ?",
            &[&crate::util::plus_minutes(TEST_NOW, -31)],
        );
        let second = auto_compute(&s, WAITED, HANDS_WANTED_MINUTES, TEST_NOW, "daemon")
            .expect("the other project rents");
        assert_eq!(second.project, "other");
        exec1(
            &s,
            "UPDATE compute_requests SET status = 'released' WHERE id = ?",
            &[&second.id],
        );
        exec1(&s, "DELETE FROM jobs WHERE id = 'j-other'", &[]);
        // A recipe that pins the tool makes the next machine a different
        // one, so proj is rented for again.
        let project = crate::util::must_one(
            &*s.conn().unwrap(),
            "SELECT * FROM projects WHERE slug = 'proj'",
            &[],
        );
        crate::env::env_set(
            &s,
            project,
            "---\nrecipe: 1\ntools:\n  - rust@1.95\n---\n".into(),
            "owner".into(),
            String::new(),
            String::new(),
        )
        .unwrap();
        let third = auto_compute(&s, WAITED, HANDS_WANTED_MINUTES, TEST_NOW, "daemon")
            .expect("rents once the recipe changed");
        assert_eq!(third.project, "proj");
    }

    /// A hub whose daemon rents on its own from a provider the test fakes,
    /// with one task that has waited long enough to rent for.
    fn renting_hub(s: &hive_store::Store) -> crate::test_helpers_provider::ComputeHarness {
        let h = crate::test_helpers_provider::new_compute_harness(
            s,
            &[
                ("compute.provider", "azure"),
                ("compute.azure.resource_group", "hive-hands"),
                ("compute.azure.location", "westeurope"),
                ("compute.azure.vm_size", "Standard_B2ms"),
                ("compute.auto_approve", "1"),
            ],
        );
        queued_job(s, "j-waits", "proj", "task", "owner");
        exec1(
            s,
            "UPDATE jobs SET created_at = ? WHERE id = 'j-waits'",
            &[&crate::util::plus_minutes(TEST_NOW, -31)],
        );
        h
    }

    /// The renting half of one daemon tick at `now`: the daemon asks for a
    /// machine when it may, and the compute worker takes that request to
    /// its end. The request as the worker left it, or why none was made.
    fn renting_tick(
        s: &hive_store::Store,
        h: &crate::test_helpers_provider::ComputeHarness,
        now: &str,
    ) -> Result<hive_tasks::ComputeRequest, NoAutoCompute> {
        let at = hive_store::Store::open(hive_store::Options {
            home: s.home.clone(),
            now: now.into(),
            ..Default::default()
        })
        .unwrap();
        let req = auto_compute(
            &at,
            "1 job(s) waited 31 min with every slot on this machine busy or held",
            HANDS_WANTED_MINUTES,
            now,
            "daemon",
        )?;
        let cancel = hive_bus::CancellationToken::new();
        let worker = crate::compute::new_compute_worker(
            &cancel,
            &at,
            crate::compute::ComputeOpts {
                actor: "daemon".into(),
                hub_home: h.hub_home.clone(),
            },
        );
        worker.tick(&cancel, now);
        worker.wait(Duration::from_secs(60));
        cancel.cancel();
        Ok(hive_tasks::get_compute(&at, &req.id).unwrap())
    }

    /// One request a tick is what a hub measured: 11,034 in under two hours,
    /// each failed before a machine was ready and replaced on the next tick.
    #[test]
    fn a_request_that_fails_before_a_machine_is_ready_is_asked_again_after_a_growing_pause() {
        let _serial = crate::test_helpers::serial();
        let _lock = crate::test_helpers_provider::provider_test_lock();
        let s = new_store();
        let h = renting_hub(&s);
        // The provider fails the same way every time it is asked.
        let _fake = crate::test_helpers_provider::install_fake(
            crate::azure::set_run_az,
            Box::new(|_, _, _| Err(anyhow::anyhow!("the provider's sign-in has expired"))),
        );
        // A tick every fifteen seconds for an hour.
        let mut asked = Vec::new();
        for tick in 0..240 {
            let minutes = f64::from(tick) / 4.0;
            let now = crate::util::plus_minutes(TEST_NOW, minutes);
            if let Ok(req) = renting_tick(&s, &h, &now) {
                assert_eq!(req.status, "failed", "{req:?}");
                asked.push(minutes);
            }
        }
        assert_eq!(
            asked,
            [0.0, 5.0, 20.0],
            "the minutes at which the daemon asked: one request per pause (5 min, then 15, then 45), not one per tick"
        );
        // The rest of the ladder, looked at a tick before each step and on
        // it: 135 minutes, then six hours for as long as the cause stands,
        // the third day included.
        let mut due = 20.0 + 45.0;
        for pause in [135.0].into_iter().chain([360.0; 11]) {
            let tick =
                |minutes: f64| renting_tick(&s, &h, &crate::util::plus_minutes(TEST_NOW, minutes));
            let early = tick(due - 0.25);
            assert!(
                matches!(early, Err(NoAutoCompute::Pause(_))),
                "asked before minute {due}: {early:?}"
            );
            let req = tick(due).unwrap_or_else(|why| panic!("not asked at minute {due}: {why}"));
            assert_eq!(req.status, "failed", "{req:?}");
            due += pause;
        }
        assert!(due > 48.0 * 60.0, "{due}");
    }

    #[test]
    fn a_request_the_workers_own_checks_would_fail_again_is_not_asked_for() {
        let _serial = crate::test_helpers::serial();
        let _lock = crate::test_helpers_provider::provider_test_lock();
        let s = new_store();
        let h = renting_hub(&s);
        let az = crate::test_helpers_provider::new_fake_az();
        let _fake = crate::test_helpers_provider::install_fake(crate::azure::set_run_az, {
            let az = az.clone();
            Box::new(move |ctx, args, timeout| az.run(ctx, args, timeout))
        });
        // No linux hive to pair a machine with: the worker fails the request
        // before it asks the provider for anything.
        let had = crate::pair::set_hands_binary(Box::new(|arch| {
            Err(anyhow::anyhow!("no linux/{arch} hive binary to ship"))
        }));
        let first = renting_tick(&s, &h, TEST_NOW).expect("the first request");
        assert_eq!(first.status, "failed", "{first:?}");
        assert!(first.note.ends_with("; no machine was bought"), "{first:?}");
        // On the next tick the pause holds the daemon; once it is over, and
        // a day later still, the worker's own check does. No request is
        // made, and the line says why.
        let tick =
            |minutes: f64| renting_tick(&s, &h, &crate::util::plus_minutes(TEST_NOW, minutes));
        assert_eq!(
            tick(0.25),
            Err(NoAutoCompute::Pause(
                "the last 1 request(s) failed before a machine was ready (CR-001: no linux/amd64 hive binary to ship; no machine was bought); the daemon asks again after 2026-09-06T10:05:00Z".into()
            ))
        );
        for minutes in [5.0, 20.0, 24.0 * 60.0] {
            assert_eq!(
                tick(minutes),
                Err(NoAutoCompute::Why(
                    "CR-001 failed before a machine was ready and the next request would fail too: no linux/amd64 hive binary to ship; no machine was bought".into()
                )),
                "after {minutes} min"
            );
        }
        assert_eq!(
            count(&s, "SELECT COUNT(*) AS n FROM compute_requests", &[]),
            1
        );
        assert!(
            !az.has("vm create"),
            "the provider was asked: {:?}",
            az.calls()
        );
        // The binary is there: the daemon asks on its next tick.
        let _ = crate::pair::set_hands_binary(had);
        let second = tick(24.0 * 60.0 + 0.25).expect("asked once the binary is there");
        assert_eq!(
            (second.status.as_str(), second.executor.as_str()),
            ("ready", "hands-cr-002"),
            "{second:?}"
        );
    }

    /// The pause line for the `failed` requests in a row that ended with
    /// `last`, asked again at `due`.
    fn pause_line(failed: usize, last: &hive_tasks::ComputeRequest, due: &str) -> NoAutoCompute {
        NoAutoCompute::Pause(format!(
            "the last {failed} request(s) failed before a machine was ready ({}: {}); the daemon asks again after {due}",
            last.id,
            crate::util::trunc(&last.note, 200)
        ))
    }

    /// The costly failure: the machine is bought, a later step fails and it
    /// is released. A hub measured 56 of those back to back.
    #[test]
    fn a_request_that_fails_after_its_machine_was_made_is_followed_by_the_pause_too() {
        let _serial = crate::test_helpers::serial();
        let _lock = crate::test_helpers_provider::provider_test_lock();
        let s = new_store();
        let h = renting_hub(&s);
        let az = crate::test_helpers_provider::new_fake_az();
        let _fake = crate::test_helpers_provider::install_fake(crate::azure::set_run_az, {
            let az = az.clone();
            Box::new(move |ctx, args, timeout| {
                if args.join(" ").starts_with("vm run-command invoke") {
                    return Ok((1, String::new(), "RunCommand is disabled here".into()));
                }
                az.run(ctx, args, timeout)
            })
        });
        let tick =
            |minutes: f64| renting_tick(&s, &h, &crate::util::plus_minutes(TEST_NOW, minutes));
        let first = tick(0.0).expect("the first request");
        assert_eq!(first.status, "failed", "{first:?}");
        assert!(
            first.note.ends_with("; what was created is released"),
            "{first:?}"
        );
        assert_eq!(az.deleted(), vec!["hive-cr-001"]);
        assert_eq!(
            tick(0.25),
            Err(pause_line(1, &first, "2026-09-06T10:05:00Z"))
        );
        assert_eq!(
            az.deleted(),
            vec!["hive-cr-001"],
            "a second machine was bought inside the pause"
        );
    }

    /// A released rental: requested at `at` for `hours`, its machine made at `made` and released
    /// at `released`.
    fn released_rental(
        s: &hive_store::Store,
        id: &str,
        hours: i32,
        at: &str,
        made: &str,
        released: &str,
    ) {
        exec1(
            s,
            "INSERT INTO compute_requests(id,project,requested_by,size,hours,reason,status,decided_by,executor,note,at,updated_at) VALUES(?,'proj','daemon','Standard_B2ms',?,'more hands','released','hive',?,'its hours are up',?,?)",
            &[
                &id,
                &hours,
                &format!("hands-{}", id.to_lowercase()),
                &at,
                &released,
            ],
        );
        exec1(
            s,
            "INSERT INTO compute_machines(request_id,provider,name,executor,created_at,released_at) VALUES(?,'azure',?,?,?,?)",
            &[
                &id,
                &format!("hive-{}", id.to_lowercase()),
                &format!("hands-{}", id.to_lowercase()),
                &made,
                &released,
            ],
        );
    }

    /// Two rentals replayed on one hub ran 5.2 and 8.0 hours on 4-hour approvals, and each time
    /// the daemon asked for the next machine as soon as the last was released. A release for
    /// running past its hours now waits like a failed request does (`rental_failed_lately`).
    #[test]
    fn a_rental_released_past_its_hours_is_followed_by_the_pause() {
        let _serial = crate::test_helpers::serial();
        let _lock = crate::test_helpers_provider::provider_test_lock();
        let s = new_store();
        let _h = renting_hub(&s);
        // Approved for 4 hours, ran 8 and released two minutes before TEST_NOW.
        released_rental(
            &s,
            "CR-001",
            4,
            "2026-09-06T01:50:00Z",
            "2026-09-06T01:58:00Z",
            "2026-09-06T09:58:00Z",
        );
        const STARVED: &str = "1 job(s) waited 31 min with every slot on this machine busy or held";
        let ask = |now: &str| auto_compute(&s, STARVED, HANDS_WANTED_MINUTES, now, "daemon");
        assert_eq!(
            ask(TEST_NOW),
            Err(NoAutoCompute::Pause(
                "CR-001's machine ran 8.0 hour(s) on 4 approved and was released (1 of the last 1 request(s) ran past their hours, the rest failed before a machine was ready); the daemon asks again after 2026-09-06T10:03:00Z".into()
            ))
        );
        let next = ask(&crate::util::plus_minutes(TEST_NOW, 4.0)).expect("asked after the pause");
        assert_eq!(next.hours, AUTO_COMPUTE_HOURS, "{next:?}");
    }

    /// A rental released inside its hours (or on the tick they ran out) is no overrun: the next
    /// machine is asked for at once, as before.
    #[test]
    fn a_rental_released_on_time_is_followed_at_once() {
        let _serial = crate::test_helpers::serial();
        let _lock = crate::test_helpers_provider::provider_test_lock();
        let s = new_store();
        let _h = renting_hub(&s);
        released_rental(
            &s,
            "CR-001",
            4,
            "2026-09-06T05:50:00Z",
            "2026-09-06T05:57:00Z",
            "2026-09-06T09:58:00Z",
        );
        let req = auto_compute(
            &s,
            "1 job(s) waited 31 min with every slot on this machine busy or held",
            HANDS_WANTED_MINUTES,
            TEST_NOW,
            "daemon",
        )
        .expect("asked at once");
        assert_eq!(req.hours, AUTO_COMPUTE_HOURS, "{req:?}");
    }

    /// A release that began on the tick the hours ran out but whose provider delete took 15
    /// minutes (the budget is 20) is no overrun: the grace is measured to when the release
    /// began, not to the `released_at` stamped after the delete returned.
    #[test]
    fn a_slow_teardown_is_no_overrun() {
        let _serial = crate::test_helpers::serial();
        let _lock = crate::test_helpers_provider::provider_test_lock();
        let s = new_store();
        let _h = renting_hub(&s);
        released_rental(
            &s,
            "CR-001",
            4,
            "2026-09-06T05:30:00Z",
            "2026-09-06T05:40:00Z",
            "2026-09-06T09:55:00Z",
        );
        crate::util::event_at(
            &s.conn().unwrap(),
            "2026-09-06T09:41:00Z",
            "daemon",
            crate::compute::RELEASE_BEGAN_ACTION,
            "CR-001",
            serde_json::Value::Null,
        );
        let req = auto_compute(
            &s,
            "1 job(s) waited 31 min with every slot on this machine busy or held",
            HANDS_WANTED_MINUTES,
            TEST_NOW,
            "daemon",
        )
        .expect("asked at once");
        assert_eq!(req.hours, AUTO_COMPUTE_HOURS, "{req:?}");
    }

    #[test]
    fn a_denied_request_ends_neither_the_pause_nor_the_run_of_failures() {
        let _serial = crate::test_helpers::serial();
        let _lock = crate::test_helpers_provider::provider_test_lock();
        let s = new_store();
        let h = renting_hub(&s);
        let _fake = crate::test_helpers_provider::install_fake(
            crate::azure::set_run_az,
            Box::new(|_, _, _| Err(anyhow::anyhow!("the provider's sign-in has expired"))),
        );
        let tick =
            |minutes: f64| renting_tick(&s, &h, &crate::util::plus_minutes(TEST_NOW, minutes));
        let first = tick(0.0).expect("the first request");
        assert_eq!(first.status, "failed", "{first:?}");
        // Inside the pause an agent asks by hand for a machine the owner
        // has to approve, and the owner says no.
        s.set_setting(hive_store::HUMAN_AGENT, "compute.auto_approve", "0")
            .unwrap();
        let by_hand =
            hive_tasks::request_compute(&s, "proj", "aurora", "", 2, "asked by hand").unwrap();
        assert_eq!(by_hand.status, "pending", "{by_hand:?}");
        hive_tasks::deny(&s, &by_hand.id, "not now", hive_store::HUMAN_AGENT).unwrap();
        s.set_setting(hive_store::HUMAN_AGENT, "compute.auto_approve", "1")
            .unwrap();
        // The pause stands, and the next failure is the second in a row.
        assert_eq!(
            tick(2.0),
            Err(pause_line(1, &first, "2026-09-06T10:05:00Z"))
        );
        let second = tick(5.0).expect("asked again when the pause was over");
        assert_eq!(second.status, "failed", "{second:?}");
        assert_eq!(
            tick(5.25),
            Err(pause_line(2, &second, "2026-09-06T10:20:00Z"))
        );
    }

    #[test]
    fn failures_older_than_two_days_do_not_lengthen_the_pause() {
        let _serial = crate::test_helpers::serial();
        let _lock = crate::test_helpers_provider::provider_test_lock();
        let s = new_store();
        let h = renting_hub(&s);
        let _fake = crate::test_helpers_provider::install_fake(
            crate::azure::set_run_az,
            Box::new(|_, _, _| Err(anyhow::anyhow!("the provider's sign-in has expired"))),
        );
        // Five requests failed one after another two months ago, and none
        // has been made since.
        let long_ago = hive_store::Store::open(hive_store::Options {
            home: s.home.clone(),
            now: crate::util::plus_hours(TEST_NOW, -60.0 * 24.0),
            ..Default::default()
        })
        .unwrap();
        for _ in 0..5 {
            let old =
                hive_tasks::request_compute(&long_ago, "proj", "aurora", "", 2, "an old wait")
                    .unwrap();
            hive_tasks::set_compute_status(&long_ago, &old.id, "failed", "", "daemon").unwrap();
        }
        let tick =
            |minutes: f64| renting_tick(&s, &h, &crate::util::plus_minutes(TEST_NOW, minutes));
        let first = tick(0.0).expect("asked: failures that old hold nothing");
        assert_eq!(first.status, "failed", "{first:?}");
        // Today's failure is the first of its run: five minutes, not six hours.
        assert_eq!(
            tick(0.25),
            Err(pause_line(1, &first, "2026-09-06T10:05:00Z"))
        );
    }

    /// The every-slot-busy path with the owner's question open already: the
    /// reason the daemon rents nothing used to ride only on the line of a
    /// new question, so with one open nothing said it.
    #[test]
    fn a_starved_queue_says_why_no_machine_is_rented_when_the_owner_was_asked_already() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        let fixed = new_store();
        new_bare_repo(&fixed);
        join(&fixed, "aurora");
        exec1(
            &fixed,
            "UPDATE projects SET brain = 1 WHERE slug = 'proj'",
            &[],
        );
        for (k, v) in [("capacity", "2"), ("daemon.sweep", "off")] {
            fixed.set_setting(hive_store::HUMAN_AGENT, k, v).unwrap();
        }
        crate::jobs::add_job(
            &fixed,
            "proj".into(),
            "task".into(),
            "Waits behind the load".into(),
            "fake worker only".into(),
            0,
            100,
            "test".into(),
            hive_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        // Saturated for the whole test: nothing is leased, the queue is
        // short of hands.
        let _saturated = crate::daemon_load::pin_pressure(crate::daemon_load::Pressure {
            load1: Some(40.0),
            ..crate::daemon_load::CALM
        });
        let _now = hive_testutil::setenv("HIVE_NOW", TEST_NOW);
        let s = hive_store::Store::open(hive_store::Options {
            home: fixed.home.clone(),
            create: false,
            read_only: false,
            skip_records: false,
            now: String::new(),
        })
        .unwrap();
        // One daemon whose clock jumps 31 minutes the first time it idles,
        // so its queue has starved for half an hour on the loops after.
        let starved_half_an_hour = || {
            let later = crate::util::plus_minutes(&s.now(), 31);
            let out = OutBuf::new();
            let result = daemon(
                &hive_bus::CancellationToken::new(),
                &s,
                DaemonOpts {
                    interval: 1,
                    max_loops: 6,
                    out: Some(out.writer()),
                    worker_cmd: "sh -c 'echo SUMMARY: ok'".into(),
                    sleep: Some(std::sync::Arc::new(move |_| {
                        // The test holds hive_testutil::env_lock for the whole body.
                        unsafe {
                            std::env::set_var("HIVE_NOW", &later);
                        }
                        true
                    })),
                    ..Default::default()
                },
            );
            assert_eq!(result, "done", "{}", out.string());
            out.string()
        };
        let questions = || {
            count(
                &fixed,
                "SELECT COUNT(*) AS n FROM posts WHERE kind = 'question' AND thread = 'hands-wanted'",
                &[],
            )
        };
        // Not pre-authorised: the owner is asked.
        let out = starved_half_an_hour();
        assert!(
            out.contains(" job(s) waited 31 min with every slot busy or held"),
            "{out}"
        );
        assert_eq!(questions(), 1, "{out}");
        // Pre-authorised now, but no machine-hours a day (0 means none, as
        // the worker reads it too): no new question, and the log says why.
        for (k, v) in [("compute.auto_approve", "1"), ("compute.max_vms", "1")] {
            fixed.set_setting(hive_store::HUMAN_AGENT, k, v).unwrap();
        }
        let failed =
            hive_tasks::request_compute(&fixed, "proj", "aurora", "", 4, "an earlier wait")
                .unwrap();
        hive_tasks::set_compute_status(&fixed, &failed.id, "failed", "", "daemon").unwrap();
        let out = starved_half_an_hour();
        assert_eq!(questions(), 1, "{out}");
        assert!(
            out.contains(&format!(
                "hands wanted: the daemon rents none (compute.max_hours_per_day 0 would be passed (0 h today, {AUTO_COMPUTE_HOURS} h asked))"
            )) && !out.contains(&failed.id),
            "{out}"
        );
    }

    #[test]
    fn an_idle_sleep_under_a_hold_wakes_when_the_load_drops() {
        let _serial = crate::test_helpers::serial();
        let s = store_with_projects(&["proj"]);
        let machine = crate::daemon_load::Machine::detect();
        let cancel = hive_bus::CancellationToken::new();
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

        // A queued job wakes it as before. The adder queues a job every poll
        // until the test signals that idle_wait returned, so whenever the
        // wait takes its snapshot of the queue a later job still changes it;
        // a timed sleep before one insert raced that snapshot under load.
        // The wait gets an interval no loaded machine reaches, so only the
        // wake - never the deadline - can end it in time.
        let wake_sleep = Duration::from_secs(120);
        let began = std::time::Instant::now();
        let s_add = s.clone();
        let (stop_tx, stop_rx) = crossbeam_channel::bounded::<()>(1);
        let adder = std::thread::spawn(move || {
            let mut n = 0;
            while let Err(crossbeam_channel::RecvTimeoutError::Timeout) =
                stop_rx.recv_timeout(Duration::from_millis(20))
            {
                n += 1;
                crate::util::exec1(
                    &*s_add.conn().unwrap(),
                    "INSERT INTO jobs (id, project, kind, title, brief, status, created_at) VALUES (?, 'proj', 'task', 'queued', '', 'queued', ?)",
                    &[&format!("queued-{n}"), &TEST_NOW],
                );
            }
        });
        let woke = idle_wait(&watch, &mut no_bus, wake_sleep, false);
        let _ = stop_tx.send(());
        adder.join().unwrap();
        assert!(woke);
        assert!(began.elapsed() < wake_sleep, "slept through a queued job");

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
            hive_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        for (k, v) in [("capacity", "1"), ("daemon.min_free_mb", "0")] {
            s.set_setting(hive_store::HUMAN_AGENT, k, v).unwrap();
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
                hive_store::HUMAN_AGENT.into(),
                false,
            )
            .unwrap();
        }
        s.set_setting(hive_store::HUMAN_AGENT, "capacity", "3")
            .unwrap();
        crate::capacity::pin_per_project(&s, 3);
        s.set_setting(hive_store::HUMAN_AGENT, "daemon.min_free_mb", "0")
            .unwrap();
        s.set_setting(hive_store::HUMAN_AGENT, "daemon.setup_mb", "0")
            .unwrap();
        s.set_setting(hive_store::HUMAN_AGENT, "jobs_per_day", "none")
            .unwrap();
        // The flag beats the setting, the setting beats the machine.
        s.set_setting(hive_store::HUMAN_AGENT, "capacity", "2")
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
        hive_bus::reset_default();
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
            s.set_setting(hive_store::HUMAN_AGENT, k, v).unwrap();
        }
        crate::capacity::pin_per_project(&s, 4);
        let _calm = crate::daemon_load::pin_pressure(crate::daemon_load::CALM);
        let add = |title: &str, source: &str| {
            crate::test_helpers::task_for_source(&s, "proj", source);
            crate::jobs::add_job(
                &s,
                "proj".into(),
                "task".into(),
                title.into(),
                "fake worker only".into(),
                0,
                100,
                source.into(),
                hive_store::HUMAN_AGENT.into(),
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
        let cancel = hive_bus::CancellationToken::new();
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

    /// A daemon left running on `s` with capacity 2 and calm pressure until
    /// the test pins another; each queued job wakes it for one tick. The
    /// fake worker runs `seconds` long.
    struct LiveGovernor {
        s: crate::test_helpers::TestStore,
        out: OutBuf,
        cancel: hive_bus::CancellationToken,
        done: crossbeam_channel::Receiver<String>,
        /// The jobs whose "leased" line was written while their run had no
        /// row yet (LeaseLineWriter).
        rowless: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    /// The daemon's output, checked the way the hub reads it: the moment a
    /// "leased <job>" line is written, the job's run row is looked up and a
    /// job without one is noted in `rowless`.
    struct LeaseLineWriter {
        out: OutBuf,
        s: hive_store::Store,
        rowless: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl Write for LeaseLineWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let text = String::from_utf8_lossy(buf);
            for line in text.lines() {
                let Some(rest) = line.strip_prefix("leased ") else {
                    continue;
                };
                let job = rest.split_whitespace().next().unwrap_or_default();
                let conn = self.s.conn().unwrap();
                let found = crate::util::query_one(
                    &*conn,
                    "SELECT 1 AS x FROM runs WHERE job_id = ? LIMIT 1",
                    &[&job],
                )
                .unwrap()
                .is_some();
                if !found {
                    self.rowless
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(job.to_string());
                }
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

    impl LiveGovernor {
        /// The fake worker runs until the test ends (`finish` drops the gate
        /// file), five minutes at most.
        fn start() -> Self {
            Self::start_with(|_| {})
        }

        /// [`Self::start`] with `seed` run on the store before the daemon's
        /// first tick.
        fn start_with(seed: impl FnOnce(&hive_store::Store)) -> Self {
            hive_bus::reset_default();
            let s = new_store();
            new_bare_repo(&s);
            for name in ["aurora", "borealis", "cirrus", "drizzle"] {
                join(&s, name);
            }
            for (k, v) in [
                ("capacity", "2"),
                ("daemon.pass_seconds", "3600"),
                ("daemon.min_free_mb", "0"),
                ("daemon.setup_mb", "0"),
                ("daemon.sweep", "off"),
                ("jobs_per_day", "none"),
            ] {
                s.set_setting(hive_store::HUMAN_AGENT, k, v).unwrap();
            }
            crate::capacity::pin_per_project(&s, 4);
            seed(&s);
            let out = OutBuf::new();
            let rowless = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let writer: DaemonOut = std::sync::Arc::new(std::sync::Mutex::new(LeaseLineWriter {
                out: out.clone(),
                s: (*s).clone(),
                rowless: std::sync::Arc::clone(&rowless),
            }));
            let opts = DaemonOpts {
                interval: 300,
                out: Some(writer),
                worker_cmd: format!(
                    "sh -c 'i=0; while [ ! -f {gate} ] && [ $i -lt 1500 ]; do sleep 0.2; i=$((i+1)); done; echo SUMMARY: ok'",
                    gate = Self::gate_of(&s).display()
                ),
                ..Default::default()
            };
            let s_daemon = s.clone();
            let cancel = hive_bus::CancellationToken::new();
            let cancel_daemon = cancel.clone();
            let (done_tx, done) = crossbeam_channel::bounded(1);
            std::thread::spawn(move || {
                let _ = done_tx.send(daemon(&cancel_daemon, &s_daemon, opts));
            });
            Self {
                s,
                out,
                cancel,
                done,
                rowless,
            }
        }

        fn gate_of(s: &hive_store::Store) -> std::path::PathBuf {
            std::path::Path::new(&s.home).join("live-governor-gate")
        }

        fn add(&self, title: &str, source: &str) -> String {
            crate::test_helpers::task_for_source(&self.s, "proj", source);
            crate::jobs::add_job(
                &self.s,
                "proj".into(),
                "task".into(),
                title.into(),
                "fake worker only".into(),
                0,
                100,
                source.into(),
                hive_store::HUMAN_AGENT.into(),
                false,
            )
            .unwrap()
            .0
            .str("id")
        }

        fn wait_for(&self, want: &dyn Fn(&str) -> bool, what: &str) -> String {
            let since = std::time::Instant::now();
            loop {
                let text = self.out.string();
                if want(&text) {
                    return text;
                }
                assert!(
                    since.elapsed() < test_wait(Duration::from_secs(60)),
                    "{what} never came:\n{text}"
                );
                std::thread::sleep(Duration::from_millis(50));
            }
        }

        fn yields(&self) -> i32 {
            count(
                &self.s,
                "SELECT COUNT(*) AS n FROM events WHERE action = ?",
                &[&crate::yield_lane::YIELD_ACTION],
            )
        }

        /// Stops the daemon; re-raises the test's panic with what it said.
        fn finish(self, result: std::thread::Result<()>) {
            let _ = std::fs::write(Self::gate_of(&self.s), "");
            self.cancel.cancel();
            let _ = self.done.recv_timeout(test_wait(Duration::from_secs(30)));
            if let Err(panic) = result {
                panic!("{}\n{}", panic_message(&panic), self.out.string());
            }
        }
    }

    #[test]
    fn daemon_starts_the_owners_job_over_the_load_line_while_the_fleets_stays_queued() {
        let _serial = crate::test_helpers::serial();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        let _calm = crate::daemon_load::pin_pressure(crate::daemon_load::CALM);
        let live = LiveGovernor::start();
        live.add("Fleet one", "test");
        live.add("Fleet two", "test");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            live.wait_for(
                &|t| t.matches("leased job-").count() >= 2,
                "the fleet's two runs",
            );
            // The fleet fills every slot and pushes the machine past twice
            // daemon.max_load (24 on the test machine's 12 CPUs), where
            // even the express lane used to shut. The owner's task and one
            // more of the fleet's arrive.
            let _loaded = crate::daemon_load::pin_pressure(crate::daemon_load::Pressure {
                load1: Some(60.0),
                ..crate::daemon_load::CALM
            });
            let fleet_three = live.add("Fleet three", "test");
            let owners = live.add("Owner's task", "task:TASK-001");
            let text = live.wait_for(
                &|t| t.contains(&format!("leased {owners}")),
                "the owner's job on its reserved slot",
            );
            assert!(
                text.contains("one express slot stays open for the owner's work"),
                "{text}"
            );
            assert!(!text.contains(&format!("leased {fleet_three}")), "{text}");
            assert_eq!(
                count(
                    &live.s,
                    "SELECT COUNT(*) AS n FROM jobs WHERE status = 'queued' AND source = 'test'",
                    &[]
                ),
                1,
                "the fleet's stays queued:\n{text}"
            );
            // One run over the line, not two: the owner's next step waits
            // while the first is in flight, as does the fleet's.
            let owners_next = live.add("Owner's next step", "task:TASK-001:next");
            std::thread::sleep(Duration::from_millis(1500));
            let text = live.out.string();
            assert!(!text.contains(&format!("leased {owners_next}")), "{text}");
            assert!(!text.contains(&format!("leased {fleet_three}")), "{text}");
        }));
        live.finish(result);
    }

    #[test]
    fn daemon_asks_no_fleet_run_to_yield_while_the_load_would_refuse_the_owners_job() {
        let _serial = crate::test_helpers::serial();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        let _calm = crate::daemon_load::pin_pressure(crate::daemon_load::CALM);
        let live = LiveGovernor::start();
        // The owner's first task and the fleet's run fill both slots.
        let owners = live.add("Owner's task", "task:TASK-001");
        let fleet = live.add("Fleet one", "test");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            live.wait_for(
                &|t| {
                    t.contains(&format!("leased {owners}"))
                        && t.contains(&format!("leased {fleet}"))
                },
                "the owner's and the fleet's runs",
            );
            // The lease line means the run exists: each run's row was there
            // the moment its "leased" line was written.
            assert_eq!(
                *live.rowless.lock().unwrap(),
                Vec::<String>::new(),
                "a leased line came before its run row:\n{}",
                live.out.string()
            );
            // Then the project's checks take turns and one lane fits the
            // commit cap: the fleet's run holds the lane the owner's next
            // task needs, inside its harness, so it could be asked to hold.
            live.s
                .set_setting(
                    hive_store::HUMAN_AGENT,
                    crate::verify_lock::SETTING,
                    crate::verify_lock::PROJECT,
                )
                .unwrap();
            for (k, v) in [
                ("harness.commit_timeout", "900"),
                ("harness.step_timeout", "900"),
            ] {
                live.s.set_setting(hive_store::HUMAN_AGENT, k, v).unwrap();
            }
            {
                let conn = live.s.conn().unwrap();
                crate::util::event_at(
                    &*conn,
                    "2026-09-01T00:00:00Z",
                    "daemon",
                    "land.landed",
                    "hive/x/1",
                    serde_json::json!({"project": "proj", "sha": "abc", "verify": "verify exit 0", "verify_seconds": 600}),
                );
            }
            assert_eq!(crate::jobs::project_lanes(&live.s, "proj").lanes, 1);
            let run = crate::util::must_one(
                &*live.s.conn().unwrap(),
                "SELECT id, agent FROM runs WHERE job_id = ? AND status = 'running'",
                &[&fleet],
            );
            live.s
                .begin_harness_session(hive_store::HarnessSession {
                    id: format!("h-{fleet}"),
                    run_id: run.str("id"),
                    job_id: fleet.clone(),
                    agent: run.str("agent"),
                    project: "proj".into(),
                    ..Default::default()
                })
                .unwrap();
            // Past twice the load line with the owner's one run over it in
            // flight, the governor would refuse the owner's next task even
            // with the fleet's lane free: no run is asked to hold.
            let loaded = crate::daemon_load::pin_pressure(crate::daemon_load::Pressure {
                load1: Some(60.0),
                ..crate::daemon_load::CALM
            });
            let next = live.add("Owner's next task", "task:TASK-002");
            let text = live.wait_for(
                &|t| t.contains("no fleet run is asked to hold"),
                "the refused yield",
            );
            assert!(
                text.contains(&format!("the owner's {next} waits for a lane"))
                    && text.contains("load 60.0 on 12 CPUs is over the line 24, and the owner's one run over the line is already in flight"),
                "{text}"
            );
            assert_eq!(live.yields(), 0, "no job.yield:\n{text}");
            assert!(!text.contains("holds after its current step"), "{text}");
            assert!(!text.contains(&format!("leased {next}")), "{text}");
            // Once the machine would admit the owner's job (the load back under
            // the line: the fleet's slot would go to the owner's), the
            // fleet's run is asked to hold, once.
            drop(loaded);
            let _calm = crate::daemon_load::pin_pressure(crate::daemon_load::CALM);
            live.add("Fleet wake", "test");
            let text = live.wait_for(
                &|t| t.contains("holds after its current step"),
                "the yield once the owner's job could start",
            );
            assert!(
                text.contains(&format!(
                    "{fleet} (task) holds after its current step so the owner's {next} can start"
                )),
                "{text}"
            );
            assert_eq!(live.yields(), 1, "{text}");
        }));
        live.finish(result);
    }

    /// Today's reflect of `proj` is done, so the planner adds no run beside
    /// the ones a test counts.
    fn reflected_today(s: &hive_store::Store) {
        exec1(
            s,
            "INSERT INTO jobs (id, project, kind, title, tier, priority, source, status, created_by, created_at, finished_at) VALUES ('job-reflected', 'proj', 'reflect', 'Daily reflect', 0, 1, ?, 'done', 'daemon', ?, ?)",
            &[
                &format!("reflect:proj:{}", &TEST_NOW[..10]),
                &TEST_NOW,
                &TEST_NOW,
            ],
        );
    }

    #[test]
    fn daemon_asks_a_fleet_run_to_yield_when_the_halved_cap_is_full_under_the_fleet_total() {
        // Review of PR 276: at the plain share the yield's one trigger is a
        // full fleet, and it compared the jobs in flight with the fleet's
        // nominal total. With the cap halved by the kernel's memory warning
        // two runs fill a machine of four slots: the owner's next task
        // waited beside them and no fleet run was asked to hold.
        let _serial = crate::test_helpers::serial();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        let calm = crate::daemon_load::pin_pressure(crate::daemon_load::CALM);
        let live = LiveGovernor::start_with(|s| {
            s.set_setting(hive_store::HUMAN_AGENT, "capacity", "4")
                .unwrap();
            reflected_today(s);
        });
        let fleet = live.add("Fleet one", "test");
        let owner = live.add("Owner's step", "task:TASK-001");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            live.wait_for(
                &|t| {
                    t.contains(&format!("leased {fleet}")) && t.contains(&format!("leased {owner}"))
                },
                "the fleet's run and the owner's",
            );
            let run = crate::util::must_one(
                &*live.s.conn().unwrap(),
                "SELECT id, agent FROM runs WHERE job_id = ? AND status = 'running'",
                &[&fleet],
            );
            live.s
                .begin_harness_session(hive_store::HarnessSession {
                    id: format!("h-{fleet}"),
                    run_id: run.str("id"),
                    job_id: fleet.clone(),
                    agent: run.str("agent"),
                    project: "proj".into(),
                    ..Default::default()
                })
                .unwrap();
            // The kernel's memory warning halves the cap of four to two, and
            // two runs fill it - one the owner's, so no express slot - while
            // the fleet's nominal total is still four.
            drop(calm);
            let _warned = crate::daemon_load::pin_pressure(crate::daemon_load::Pressure {
                mem_warning: true,
                ..crate::daemon_load::CALM
            });
            assert_eq!(crate::capacity::fleet(&live.s).total, 4);
            let next = live.add("Owner's next step", "task:TASK-001:next");
            let text = live.wait_for(
                &|t| t.contains("holds after its current step"),
                "the yield of the fleet run over the halved cap",
            );
            assert!(
                text.contains(&format!(
                    "{fleet} (task) holds after its current step so the owner's {next} can start"
                )),
                "{text}"
            );
            assert_eq!(live.yields(), 1, "{text}");
            assert!(!text.contains(&format!("leased {next}")), "{text}");
        }));
        live.finish(result);
    }

    #[test]
    fn daemon_runs_a_locked_projects_owner_work_to_its_share_plus_one() {
        // Review of PR 276: with `verify.lock` on, the owner's work past
        // the share stops at the share plus one (jobs.rs owner_work_bound),
        // since a run's commit cap is fixed when it starts. Under the real
        // dispatch loop at a share of one, with four slots free: two of the
        // owner's three tasks start, the third waits, and the fleet's job
        // of the project waits behind the owner's.
        let _serial = crate::test_helpers::serial();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        let _calm = crate::daemon_load::pin_pressure(crate::daemon_load::CALM);
        let live = LiveGovernor::start_with(|s| {
            for (k, v) in [
                ("capacity", "4"),
                (crate::verify_lock::SETTING, crate::verify_lock::PROJECT),
            ] {
                s.set_setting(hive_store::HUMAN_AGENT, k, v).unwrap();
            }
            crate::capacity::pin_per_project(s, 1);
            reflected_today(s);
        });
        assert_eq!(crate::jobs::owner_work_bound(&live.s), Some(2));
        let tasks = (1..=3)
            .map(|n| live.add(&format!("Owner's task {n}"), &format!("task:TASK-00{n}")))
            .collect::<Vec<_>>();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let text = live.wait_for(
                &|t| t.matches("leased job-").count() >= 2,
                "the owner's two runs",
            );
            assert!(
                text.contains(&format!("leased {}", tasks[0]))
                    && text.contains(&format!("leased {}", tasks[1])),
                "{text}"
            );
            // More ticks: a fleet job of the project wakes the loop, and
            // neither it nor the owner's third task starts, with two of
            // the four slots still free.
            let fleet = live.add("Fleet wake", "test");
            std::thread::sleep(Duration::from_millis(1500));
            live.add("Fleet wake two", "test");
            std::thread::sleep(Duration::from_millis(1500));
            let text = live.out.string();
            assert_eq!(text.matches("leased job-").count(), 2, "{text}");
            assert!(!text.contains(&format!("leased {}", tasks[2])), "{text}");
            assert!(!text.contains(&format!("leased {fleet}")), "{text}");
            assert_eq!(crate::jobs::occupied_work_jobs(&live.s, "proj"), 2);
        }));
        live.finish(result);
    }

    #[test]
    fn daemon_keeps_the_last_slot_from_the_owners_work_past_its_share_under_a_soft_hold_too() {
        // Review of PR 276, items D and G: keep_slot at the daemon's level.
        // Two slots, a share of one: the owner's first task runs, and its
        // second, past the share, is refused the last free slot, which an
        // answer to the owner then takes. Under a soft load hold with room
        // below the cap the one slot left is the express slot; it was
        // never kept, so the owner's second task took it and the answer
        // waited.
        let _serial = crate::test_helpers::serial();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        let calm = crate::daemon_load::pin_pressure(crate::daemon_load::CALM);
        let live = LiveGovernor::start_with(|s| {
            crate::capacity::pin_per_project(s, 1);
            reflected_today(s);
        });
        let first = live.add("Owner's task 1", "task:TASK-001");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            live.wait_for(
                &|t| t.contains(&format!("leased {first}")),
                "the owner's first task",
            );
            // Calm: the second task is past the share and the free slot is
            // the last one.
            let second = live.add("Owner's task 2", "task:TASK-002");
            std::thread::sleep(Duration::from_millis(1500));
            let text = live.out.string();
            assert!(!text.contains(&format!("leased {second}")), "{text}");
            // A soft hold: load 30 on 12 CPUs, over the line 24 and under
            // twice it, one run of two in flight. The express slot is the
            // last one under the cap, and kept the same.
            drop(calm);
            let _soft = crate::daemon_load::pin_pressure(crate::daemon_load::Pressure {
                load1: Some(30.0),
                ..crate::daemon_load::CALM
            });
            live.add("Fleet wake", "test");
            live.wait_for(
                &|t| t.contains("one express slot stays open"),
                "the soft hold's express slot",
            );
            std::thread::sleep(Duration::from_millis(1500));
            let text = live.out.string();
            assert!(!text.contains(&format!("leased {second}")), "{text}");
            // The owner's answer is not work past a share: it takes the slot.
            let answer = crate::jobs::add_job(
                &live.s,
                "proj".into(),
                "answer".into(),
                "Answer the owner".into(),
                "fake worker only".into(),
                0,
                100,
                "question:abc".into(),
                hive_store::HUMAN_AGENT.into(),
                false,
            )
            .unwrap()
            .0
            .str("id");
            let text = live.wait_for(
                &|t| t.contains(&format!("leased {answer}")),
                "the owner's answer on the kept slot",
            );
            assert!(!text.contains(&format!("leased {second}")), "{text}");
        }));
        live.finish(result);
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
            hive_store::HUMAN_AGENT.into(),
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
                hive_store::HUMAN_AGENT.into(),
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
        s.set_setting(hive_store::HUMAN_AGENT, "capacity", "2")
            .unwrap();
        s.set_setting(hive_store::HUMAN_AGENT, "daemon.min_free_mb", "0")
            .unwrap();
        s.set_setting(hive_store::HUMAN_AGENT, "daemon.setup_mb", "1")
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
        hive_bus::reset_default();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        join(&s, "borealis");
        if !history.is_empty() {
            // Room for the slow run, the daily reflect the planner adds over
            // a day-old history and the job queued after the flip.
            crate::capacity::pin_per_project(&s, 3);
            s.set_setting(hive_store::HUMAN_AGENT, "jobs_per_day", "none")
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
        s.set_setting(hive_store::HUMAN_AGENT, "capacity", workers)
            .unwrap();
        s.set_setting(
            hive_store::HUMAN_AGENT,
            "daemon.restart_drain_minutes",
            drain_minutes,
        )
        .unwrap();
        let add_from = |title: &str, source: &str| {
            crate::test_helpers::task_for_source(&s, "proj", source);
            crate::jobs::add_job(
                &s,
                "proj".into(),
                "task".into(),
                title.into(),
                "fake worker only".into(),
                0,
                100,
                source.into(),
                hive_store::HUMAN_AGENT.into(),
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
        let cancel = hive_bus::CancellationToken::new();
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
                hive_bus::emit(
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
                while !out.string().contains("hive changed on disk") {
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
                        hive_store::HUMAN_AGENT.into(),
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
                hive_bus::emit(
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
        hive_bus::reset_default();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        s.set_setting(hive_store::HUMAN_AGENT, "capacity", "1")
            .unwrap();
        s.set_setting(hive_store::HUMAN_AGENT, "daemon.hold_seconds", "1")
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
            hive_store::HUMAN_AGENT.into(),
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
        let cancel = hive_bus::CancellationToken::new();
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
    fn hold_for_restart_asks_harness_runs_to_hold_and_lets_checks_run_on() {
        let s = new_store();
        let conn = s.conn().unwrap();
        for (run, agent) in [("run-harness", "aurora"), ("run-check", "borealis")] {
            conn.execute(
                "INSERT INTO runs (id, job_id, agent, project, started_at, status) VALUES (?, ?, ?, 'proj', '2026-11-04T10:00:00Z', 'running')",
                rusqlite::params![run, format!("job-{run}"), agent],
            )
            .unwrap();
        }
        // The first run is in its harness; the second is past it (its
        // check): no session to hear a hold.
        s.begin_harness_session(hive_store::HarnessSession {
            id: "h-harness".into(),
            run_id: "run-harness".into(),
            job_id: "job-run-harness".into(),
            agent: "aurora".into(),
            project: "proj".into(),
            ..Default::default()
        })
        .unwrap();
        drop(conn);
        assert!(restart_hold_on(&s), "the hold is on by default");
        let lines = hold_for_restart(&s, 2);
        assert_eq!(
            lines[0],
            "restart: 2 run(s) in flight, 1 asked to hold (daemon.restart_hold); a held run's job returns to the queue and the next daemon carries it on, the rest run to their end",
            "{lines:?}"
        );
        assert!(
            lines.contains(&"run run-harness: 1 harness session(s) asked to hold".to_string()),
            "{lines:?}"
        );
        assert!(
            lines.contains(&"run run-check: no harness session running (in its check, or a custom worker); it runs to its end".to_string()),
            "{lines:?}"
        );
        assert!(
            !s.harness_session_head("h-harness")
                .unwrap()
                .unwrap()
                .hold_requested_at
                .is_empty(),
            "the harness session was not asked to hold"
        );
        s.set_setting(hive_store::HUMAN_AGENT, "daemon.restart_hold", "off")
            .unwrap();
        assert!(!restart_hold_on(&s));
    }

    /// A daemon whose binary changed with the drain at zero exits at once:
    /// its run in flight is asked to hold, and the owner's ask to restart
    /// now written while it waits for that run is heard, stopping the run
    /// at the hold cap rather than after the hour the run would take.
    #[test]
    fn a_restart_asked_now_during_the_wait_for_runs_in_flight_is_heard() {
        let _serial = crate::test_helpers::serial();
        hive_bus::reset_default();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        s.set_setting(hive_store::HUMAN_AGENT, "capacity", "1")
            .unwrap();
        s.set_setting(hive_store::HUMAN_AGENT, "daemon.hold_seconds", "1")
            .unwrap();
        s.set_setting(hive_store::HUMAN_AGENT, "daemon.restart_drain_minutes", "0")
            .unwrap();
        let (job, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "Slow run".into(),
            "fake worker only".into(),
            0,
            100,
            "test".into(),
            hive_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let old_stamp = *BINARY_STAMP.read().unwrap();
        TEST_RESTART_CHANGED.store(false, std::sync::atomic::Ordering::SeqCst);
        set_binary_stamp(test_restart_stamp);
        let out = OutBuf::new();
        let (waiting_tx, waiting_rx) = crossbeam_channel::bounded(1);
        let writer: DaemonOut = std::sync::Arc::new(std::sync::Mutex::new(SignalWriter {
            out: out.clone(),
            waiting: waiting_tx,
            needle: "run(s) in flight to finish".into(),
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
        let cancel = hive_bus::CancellationToken::new();
        let cancel_daemon = cancel.clone();
        let (done_tx, done_rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = done_tx.send(daemon(&cancel_daemon, &s_daemon, opts));
        });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let began = std::time::Instant::now();
            while count(
                &s,
                "SELECT COUNT(*) AS n FROM runs WHERE status = 'running'",
                &[],
            ) == 0
            {
                assert!(
                    began.elapsed() < test_wait(Duration::from_secs(60)),
                    "the run never started:\n{}",
                    out.string()
                );
                std::thread::sleep(Duration::from_millis(50));
            }
            TEST_RESTART_CHANGED.store(true, std::sync::atomic::Ordering::SeqCst);
            hive_bus::emit(
                &s,
                "human",
                "post.question",
                "wake-restart",
                serde_json::json!({"project": "proj"}),
            )
            .unwrap();
            waiting_rx
                .recv_timeout(test_wait(Duration::from_secs(60)))
                .expect("the daemon never waited for its run in flight");
            ask_restart(
                &s,
                "owner",
                serde_json::json!({"from": "settings", "now": true}),
            )
            .unwrap();
            let asked = std::time::Instant::now();
            let result = done_rx
                .recv_timeout(test_wait(Duration::from_secs(60)))
                .expect("the daemon did not exit after the ask");
            (result, asked.elapsed())
        }));
        cancel.cancel();
        set_binary_stamp(old_stamp);
        TEST_RESTART_CHANGED.store(false, std::sync::atomic::Ordering::SeqCst);
        let (result, took) = match result {
            Ok(result) => result,
            Err(panic) => panic!("{}\n{}", panic_message(&panic), out.string()),
        };
        let out = out.string();
        assert_eq!(result, "restart", "{out}");
        assert!(
            took < Duration::from_secs(50),
            "the ask was not heard until the run ended ({took:?}):\n{out}"
        );
        assert!(
            out.contains("restart: 1 run(s) in flight, 0 asked to hold (daemon.restart_hold)")
                && out.contains(
                    "no harness session running (in its check, or a custom worker); it runs to its end"
                ),
            "{out}"
        );
        assert!(
            out.contains("restart asked by owner at ")
                && out.contains(
                    "while this daemon waits for 1 run(s) in flight: 0 asked to hold; stopping the rest after 1s"
                ),
            "{out}"
        );
        assert!(
            out.contains("hold cap of 1s over: stopping 1 run(s) still in flight; their jobs return to the queue"),
            "{out}"
        );
        let conn = s.conn().unwrap();
        let runs = crate::util::must_rows(
            &conn,
            "SELECT status FROM runs WHERE job_id = ?",
            &[&job.str("id")],
        );
        assert_eq!(runs.len(), 1, "{runs:?}");
        assert_eq!(runs[0].str("status"), "lost", "{runs:?}\n{out}");
        assert_eq!(
            crate::jobs::job_row(&conn, job.str("id")).str("status"),
            "queued",
            "{out}"
        );
    }

    #[test]
    fn a_restart_asked_without_now_drains_the_runs_in_flight_and_holds_nothing() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        s.set_setting(hive_store::HUMAN_AGENT, "capacity", "1")
            .unwrap();
        s.set_setting(hive_store::HUMAN_AGENT, "daemon.hold_seconds", "1")
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
        hive_bus::reset_default();
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
            hive_store::HUMAN_AGENT.into(),
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
        let cancel = hive_bus::CancellationToken::new();
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
        s.set_setting(hive_store::HUMAN_AGENT, "daemon.pass_seconds", "0")
            .unwrap();
        assert_eq!(pass_spacing(&s), Duration::ZERO);
        assert!(
            s.set_setting(hive_store::HUMAN_AGENT, "daemon.pass_seconds", "-5")
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
        hive_bus::reset_default();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        join(&s, "borealis");
        s.set_setting(hive_store::HUMAN_AGENT, "capacity", "2")
            .unwrap();
        s.set_setting(hive_store::HUMAN_AGENT, "daemon.pass_seconds", "3600")
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
                hive_store::HUMAN_AGENT.into(),
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
        let cancel = hive_bus::CancellationToken::new();
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
        hive_bus::reset_default();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        s.set_setting(hive_store::HUMAN_AGENT, "daemon.pass_seconds", "3600")
            .unwrap();
        s.set_setting(hive_store::HUMAN_AGENT, "daemon.sweep", "off")
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
        let cancel = hive_bus::CancellationToken::new();
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
            let task = hive_tasks::new(
                &s,
                "proj",
                "Fix the footer",
                "the brief",
                50,
                hive_store::HUMAN_AGENT,
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
            out.contains("hive changed on disk; exiting with code 3 so the service restarts it")
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

    /// Resets the restart bound's minute to sixty when the test ends,
    /// passed or not.
    struct MaxMinuteSecs;

    impl Drop for MaxMinuteSecs {
        fn drop(&mut self) {
            crate::restart::TEST_MAX_MINUTE_SECS.store(60, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[test]
    fn a_restart_comes_within_its_bound_while_the_owner_s_work_keeps_arriving() {
        let _serial = crate::test_helpers::serial();
        // On 2026-11-26 a restart armed at 05:42Z had not come by 12:07Z:
        // every owner's run started while it was pending anchored the window
        // again and the owner's queue never emptied. Here the drain is half
        // an hour, no fleet kind fits it, and the owner's tasks are queued
        // every quarter second for as long as the daemon lives; the bound
        // (daemon.restart_max_minutes, one minute made a second here) still
        // ends the handover, starting nothing more, the owner's work included.
        let bound = Duration::from_secs(8);
        let _minute = MaxMinuteSecs;
        crate::restart::TEST_MAX_MINUTE_SECS.store(1, std::sync::atomic::Ordering::SeqCst);
        hive_bus::reset_default();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        join(&s, "borealis");
        crate::capacity::pin_per_project(&s, 3);
        for (key, value) in [
            ("jobs_per_day", "none"),
            ("capacity", "3"),
            ("daemon.restart_drain_minutes", "30"),
            ("daemon.restart_max_minutes", &bound.as_secs().to_string()),
        ] {
            s.set_setting(hive_store::HUMAN_AGENT, key, value).unwrap();
        }
        let add = |s: &hive_store::Store, title: &str, source: &str| {
            crate::test_helpers::task_for_source(s, "proj", source);
            crate::jobs::add_job(
                s,
                "proj".into(),
                "task".into(),
                title.into(),
                "fake worker only".into(),
                0,
                100,
                source.into(),
                hive_store::HUMAN_AGENT.into(),
                false,
            )
            .unwrap();
        };
        add(&s, "Slow run", "test");
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
            worker_cmd: "sh -c 'sleep 2; echo SUMMARY: ok'".into(),
            restart_on_change: true,
            ..Default::default()
        };
        let s_daemon = s.clone();
        let cancel = hive_bus::CancellationToken::new();
        let cancel_daemon = cancel.clone();
        let (done_tx, done_rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = done_tx.send(daemon(&cancel_daemon, &s_daemon, opts));
        });
        let stop_feeding = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            leased_rx
                .recv_timeout(test_wait(Duration::from_secs(60)))
                .expect("the slow run was never leased");
            TEST_RESTART_CHANGED.store(true, std::sync::atomic::Ordering::SeqCst);
            hive_bus::emit(
                &s,
                "human",
                "post.question",
                "wake-restart",
                serde_json::json!({"project": "proj"}),
            )
            .unwrap();
            let waited = std::time::Instant::now();
            while !out.string().contains("hive changed on disk") {
                assert!(
                    waited.elapsed() < test_wait(Duration::from_secs(60)),
                    "the daemon never noticed the binary change:\n{}",
                    out.string()
                );
                std::thread::sleep(Duration::from_millis(20));
            }
            let armed = std::time::Instant::now();
            // The owner's work keeps arriving until the daemon has gone.
            let feeder = {
                let s = s.clone();
                let stop = stop_feeding.clone();
                std::thread::spawn(move || {
                    let mut n = 0;
                    while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                        n += 1;
                        add(&s, &format!("Owner's task {n}"), &format!("task:TASK-{n}"));
                        let _ = hive_bus::emit(
                            &s,
                            "human",
                            "post.question",
                            "wake-owner",
                            serde_json::json!({"project": "proj"}),
                        );
                        std::thread::sleep(Duration::from_millis(250));
                    }
                    n
                })
            };
            let result = done_rx.recv_timeout(test_wait(bound + Duration::from_secs(60)));
            let took = armed.elapsed();
            stop_feeding.store(true, std::sync::atomic::Ordering::SeqCst);
            let fed = feeder.join().unwrap();
            let result = result.unwrap_or_else(|_| {
                panic!(
                    "the daemon did not leave within the bound while the owner's work kept arriving:\n{}",
                    out.string()
                )
            });
            (result, took, fed)
        }));
        stop_feeding.store(true, std::sync::atomic::Ordering::SeqCst);
        cancel.cancel();
        set_binary_stamp(old_stamp);
        TEST_RESTART_CHANGED.store(false, std::sync::atomic::Ordering::SeqCst);
        let (result, took, fed) = match result {
            Ok(v) => v,
            Err(panic) => panic!("{}\n{}", panic_message(&panic), out.string()),
        };
        let out = out.string();
        assert_eq!(result, "restart", "{out}");
        // The bound, then the owner's runs it let start (two seconds each)
        // finishing; never the hours of a queue that does not empty.
        assert!(
            took < bound + test_wait(Duration::from_secs(20)),
            "left {took:?} after the restart was armed:\n{out}"
        );
        let said = out
            .find("(daemon.restart_max_minutes): nothing new starts, the owner's work included")
            .unwrap_or_else(|| panic!("the bound never ended the handover:\n{out}"));
        assert!(
            !out[said..].contains("leased job-"),
            "a run started after the bound was up:\n{out}"
        );
        // The owner's work did start inside the bound - it is the starting
        // that kept the handover open - and more of it was queued than ran.
        let owner_ran = count(
            &s,
            "SELECT COUNT(*) AS n FROM runs r JOIN jobs j ON j.id = r.job_id WHERE j.source LIKE 'task:TASK-%'",
            &[],
        );
        assert!(
            owner_ran >= 1,
            "no owner's run started in the window:\n{out}"
        );
        assert!(fed > owner_ran, "fed {fed}, ran {owner_ran}:\n{out}");
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
            let cmd = hive_cli::lookup(name).unwrap_or_else(|| panic!("{name} not registered"));
            let mut fs = hive_cli::FlagSet::new(name);
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
        hive_bus::reset_default();
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
            hive_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        crate::plan::plan_jobs(&s, "proj".into(), s.now(), hive_store::HUMAN_AGENT.into()).unwrap();
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
                &hive_bus::CancellationToken::new(),
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
        let cancel = hive_bus::CancellationToken::new();
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
        hive_bus::reset_default();
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
            hive_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let worker = format!("sh -c 'echo \"SUMMARY: {OUTAGE}\"; exit 3'");
        let status = |s: &hive_store::Store| {
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
            hive_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let out = OutBuf::new();
        let waits = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let waits_for_sleep = waits.clone();
        let result = daemon(
            &hive_bus::CancellationToken::new(),
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
        s.set_setting(hive_store::HUMAN_AGENT, "capacity", "1")
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
            hive_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let out = OutBuf::new();
        // Two loops, whichever way the first run's end reaches the loop.
        // Either the bus wakes loop 1 before the result lands (loop 2 then
        // waits for the run at its head, and loop 3 retires the first
        // worker and leases the second job), or the result itself ends
        // loop 1 (loop 2 retires and leases). The limit is checked only at a
        // loop's foot, so both orders stop right after the second lease and
        // the drain retires the second worker. A spare third loop plans a
        // Daily reflect under load that the daily cap (2) refuses; the
        // dispatch loop no longer spawns a worker for a lease that can take
        // nothing (`worker_lease_possible`), so it holds either way.
        let result = daemon(
            &hive_bus::CancellationToken::new(),
            &s,
            DaemonOpts {
                interval: 7,
                max_loops: 2,
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
            assert_eq!(row.pruned_reason, hive_board::RETIRE_REASON, "{name}");
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
                .contains(&format!("retired {a}: {}", hive_board::RETIRE_REASON)),
            "{}",
            out.string()
        );
    }

    /// A tick whose lease can take nothing - here the daily cap is full and
    /// none of the owner's work waits - spawns no worker: before, the
    /// dispatch loop's `pick_worker` added a fresh active worker that never
    /// ran, so the retire sweep (which needs a finished run) never retired
    /// it. The idle line still names the cap.
    #[test]
    fn daemon_tick_that_leases_nothing_spawns_no_worker() {
        let _serial = crate::test_helpers::serial();
        hive_bus::reset_default();
        let s = new_store();
        new_bare_repo(&s);
        crate::test_helpers_daemon::set_setting(&s, "daemon.sweep", "off");
        crate::test_helpers_daemon::set_setting(&s, "jobs_per_day", "1");
        crate::test_helpers_daemon::exec1(
            &s,
            "INSERT INTO jobs (id, project, kind, title, status, created_at, finished_at)
             VALUES ('ran-today', 'proj', 'task', 'Ran today', 'done', ?, ?)",
            &[&TEST_NOW, &TEST_NOW],
        );
        crate::test_helpers_daemon::exec1(
            &s,
            "INSERT INTO runs (id, job_id, agent, project, started_at, finished_at, status)
             VALUES ('ran-today-run', 'ran-today', 'gone', 'proj', ?, ?, 'done')",
            &[&TEST_NOW, &TEST_NOW],
        );
        crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "Waits for the cap".into(),
            "fleet work, not the owner's".into(),
            1,
            50,
            "cap-test".into(),
            hive_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let workers = || {
            count(
                &s,
                "SELECT COUNT(*) AS n FROM agents WHERE kind = 'worker'",
                &[],
            )
        };
        assert_eq!(workers(), 0);
        let out = OutBuf::new();
        let result = daemon(
            &hive_bus::CancellationToken::new(),
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
        assert_eq!(workers(), 0, "a worker was spawned\n{}", out.string());
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM jobs WHERE status = 'queued' AND source = 'cap-test'",
                &[]
            ),
            1,
            "{}",
            out.string()
        );
        assert!(
            out.string().contains(crate::run::DAILY_CAP_REASON),
            "{}",
            out.string()
        );
    }

    #[test]
    fn daemon_retry_uses_post_wait_time() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (fixed, job) = daemon_retry_fixture();
        let _now = hive_testutil::setenv("HIVE_NOW", TEST_NOW);
        let s = hive_store::Store::open(hive_store::Options {
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
            &hive_bus::CancellationToken::new(),
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
                    // The test holds hive_testutil::env_lock for the whole body.
                    unsafe {
                        std::env::set_var("HIVE_NOW", &later_for_sleep);
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
        hive_bus::reset_default();
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        let cancel = hive_bus::CancellationToken::new();
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
        hive_bus::emit(
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
            hive_bus::reset_default();
            let _env_guard = hive_testutil::env_lock();
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
            let old_measure = hive_evolve::set_measure(Box::new(
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
            let cancel = hive_bus::CancellationToken::new();
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
                    hive_bus::emit(
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
            let _ = hive_evolve::set_measure(old_measure);
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
        hive_bus::reset_default();
        let _env_guard = hive_testutil::env_lock();
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
            hive_store::HUMAN_AGENT.into(),
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
            hive_store::HUMAN_AGENT.into(),
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
        let old = hive_evolve::set_measure(Box::new(
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
        let cancel = hive_bus::CancellationToken::new();
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
            hive_bus::emit(
                &s,
                hive_store::HUMAN_AGENT,
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
        let _ = hive_evolve::set_measure(old);
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
        hive_bus::reset_default();
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        s.set_setting(hive_store::HUMAN_AGENT, "jobs_per_day", "1")
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
        let cancel = hive_bus::CancellationToken::new();
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
        let _env_guard = hive_testutil::env_lock();
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
        hive_bus::reset_default();
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        s.set_setting(hive_store::HUMAN_AGENT, "minutes_per_job", "1000")
            .unwrap();
        s.set_setting(hive_store::HUMAN_AGENT, "jobs_per_day", "1")
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
            hive_store::HUMAN_AGENT.into(),
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
        let cancel = hive_bus::CancellationToken::new();
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
                hive_store::HUMAN_AGENT.into(),
                false,
            )
            .unwrap();
            let id = job.str("id");
            if !needs.is_empty() {
                hive_board::set_job_needs(&s, &id, needs, "human").unwrap();
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
        add("here", hive_store::needs::this_os(), 40.0);
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
        crate::executor::add_executor(&s, now, "box", "hive@box", "/srv", 1, "*", "human").unwrap();
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

    #[test]
    fn a_job_that_needs_linux_rents_a_machine_without_a_starved_queue() {
        let _serial = crate::test_helpers::serial();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        exec1(&s, "UPDATE projects SET brain = 1 WHERE slug = 'proj'", &[]);
        // The need is what the rented machine offers and this machine does
        // not: on a Linux box of the default arch the test rents an ARM size.
        let local = crate::executor::caps(&crate::executor::local_executor_row(&s));
        let mut rented = rented_caps(&s);
        if hive_store::needs::met(
            &rented.iter().cloned().collect::<Vec<_>>().join(","),
            &local,
        ) {
            s.set_setting(
                hive_store::HUMAN_AGENT,
                "compute.azure.vm_size",
                "Standard_E2ps_v5",
            )
            .unwrap();
            rented = rented_caps(&s);
        }
        let needs = rented.iter().cloned().collect::<Vec<_>>().join(",");
        assert!(
            !hive_store::needs::met(&needs, &local),
            "{needs} vs {local:?}"
        );
        let (job, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "Builds on Linux".into(),
            "fake worker only".into(),
            0,
            100,
            "test".into(),
            hive_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let id = job.str("id");
        hive_board::set_job_needs(&s, &id, &needs, "human").unwrap();
        exec1(
            &s,
            "UPDATE jobs SET created_at = ? WHERE id = ?",
            &[
                &crate::util::plus_minutes(TEST_NOW, -(NEEDS_WANTED_MINUTES + 1)),
                &id,
            ],
        );
        let opts = || DaemonOpts {
            worker_cmd: "sh -c 'echo SUMMARY: ok'".into(),
            ..Default::default()
        };
        let requests = || {
            crate::util::must_rows(
                &s.conn().unwrap(),
                "SELECT id, status, requested_by, reason FROM compute_requests ORDER BY id",
                &[],
            )
        };
        // Not pre-authorised: the owner hears, nothing is rented, the job waits.
        let out = once_daemon(&s, opts());
        assert!(
            out.contains("machine wanted: the daemon rents none (compute.auto_approve is off)")
                && out.contains(&format!(
                    "1 job(s) of proj need {needs}, which no active machine offers"
                ))
                && !out.contains("leased "),
            "{out}"
        );
        assert!(requests().is_empty(), "{out}");
        // Pre-authorised within the caps: the machine is rented on the spot,
        // slots free or not, for the job that waited; the owner is not asked.
        for (k, v) in [
            ("compute.auto_approve", "1"),
            ("compute.max_vms", "1"),
            ("compute.max_hours_per_day", "24"),
        ] {
            s.set_setting(hive_store::HUMAN_AGENT, k, v).unwrap();
        }
        let out = once_daemon(&s, opts());
        let reqs = requests();
        assert_eq!(reqs.len(), 1, "{out}");
        assert_eq!(reqs[0].str("requested_by"), "daemon", "{out}");
        assert!(
            reqs[0].str("reason").contains(&format!(
                "1 job(s) of proj waited {} min for a machine that offers {needs}, which no paired machine does; {id} (task) is first in line",
                NEEDS_WANTED_MINUTES + 1
            )),
            "{}\n{out}",
            reqs[0].str("reason")
        );
        assert!(
            out.contains(&format!(
                "machine wanted: compute request {} (",
                reqs[0].str("id")
            )) && !out.contains("which no active machine offers\nmachine wanted: machine wanted"),
            "{out}"
        );
        // While that request is open no second machine is asked for.
        let out = once_daemon(&s, opts());
        assert_eq!(requests().len(), 1, "{out}");
        assert!(
            out.contains(&format!(
                "machine wanted: compute request {} is {}; waiting for the machine",
                reqs[0].str("id"),
                reqs[0].str("status")
            )) || reqs[0].str("status") == "pending",
            "{out}"
        );
        // The request fails before a machine is ready: the daemon does not
        // ask again on the next tick, and says until when it waits.
        let id = reqs[0].str("id");
        hive_tasks::set_compute_status(&s, &id, "failed", "", "daemon").unwrap();
        hive_tasks::note_compute(&s, &id, "the provider said no").unwrap();
        let out = once_daemon(&s, opts());
        assert_eq!(requests().len(), 1, "{out}");
        assert!(
            out.contains(&format!(
                "machine wanted: the last 1 request(s) failed before a machine was ready ({id}: the provider said no); the daemon asks again after {}",
                crate::util::plus_minutes(TEST_NOW, 5)
            )),
            "{out}"
        );
    }
}
