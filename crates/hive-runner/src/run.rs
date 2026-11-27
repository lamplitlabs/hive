use crate::brief::{BallotItem, Report, Vote};
use crate::exec::Executor;
pub(crate) use crate::exec::RunLog;
use crate::util::Row;
use anyhow::Result;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{
    Arc, LazyLock, RwLock,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, SystemTime};

pub(crate) const LAUNCH_SECONDS: i32 = 30;
pub(crate) const MAX_ATTEMPTS: i32 = 3;
/// The summary of a run the runner stopped before the worker could finish:
/// one the runner died under (sweep_lost_runs), or one its restart could
/// not hold and had to cut when the hold cap ran out.
pub(crate) const LOST_RUN_SUMMARY: &str = "the runner stopped while this run was in progress";

/// The note on a harness session held because its run stopped at the step
/// budget with work in the clone; `continuation_message` reads it back to
/// tell the carried-on run why it stopped.
pub(crate) const STEP_BUDGET_HELD_REASON: &str = "held: stopped at the step budget with work in the clone; the next run carries on from here with a fresh budget";
/// The opening words of a held run's summary after a step-budget stop; a
/// job gets that hold once, and the count of such runs says whether it had it.
const STEP_BUDGET_HELD_SUMMARY: &str = "stopped at the step budget";

/// What a run carries on from when its job's earlier run was held for the
/// runner's restart: the held harness session (its trajectory holds the
/// conversation) and the base the clone was made from.
#[derive(Debug, Clone)]
struct HeldResume {
    session: hive_store::HarnessSession,
    base: String,
}

/// The held session the run of `job` by `agent` can carry on, when one is
/// there and everything it needs still is: the same agent (its commits name
/// the agent and the landing check holds them to it), the trajectory file,
/// the clone with the worker's edits, the signing key the commits carry and
/// the held run's base. Anything missing is written to the log and the held
/// session is settled so the run starts over once, not every time.
fn resumable_held_run(
    s: &hive_store::Store,
    job_id: &str,
    agent: &str,
    run_id: &str,
    ex: &dyn crate::exec::Executor,
    custom_worker: bool,
    log: &mut RunLog,
) -> Option<HeldResume> {
    let held = s.held_harness_session_for_job(job_id).ok().flatten()?;
    let mut why = String::new();
    if custom_worker {
        why = "a custom worker command cannot carry a harness session on".into();
    } else if ex.remote() {
        why = format!("executor {} keeps no clone between runs", ex.name());
    } else if held.agent != agent {
        why = format!(
            "its worker was {}, this run's is {agent}, and the landing check holds commits to one agent",
            held.agent
        );
    } else if held.trajectory.is_empty() || !Path::new(&held.trajectory).is_file() {
        why = format!("its trajectory {} is gone", held.trajectory);
    } else if !Path::new(&ex.clone()).join(".git").exists() {
        why = format!("its clone {} is gone", ex.clone());
    } else if !crate::util::file_exists(&ex.key_path())
        || !crate::util::file_exists(&format!("{}.pub", ex.key_path()))
    {
        why = "its signing key is gone".into();
    }
    let base = if why.is_empty() {
        let base = crate::util::must_one(
            &conn(s),
            "SELECT base FROM runs WHERE id = ?",
            &[&held.run_id],
        )
        .str("base");
        if base.is_empty() {
            why = format!("run {} recorded no base", held.run_id);
        }
        base
    } else {
        String::new()
    };
    if !why.is_empty() {
        log.add(format!(
            "held harness session {} ({} steps) is not carried on: {why}; this run starts over",
            held.id, held.steps
        ));
        let _ = s.mark_harness_session_resumed(
            &held.id,
            &format!("not carried on: run {run_id} started over ({why})"),
        );
        return None;
    }
    Some(HeldResume {
        session: held,
        base,
    })
}

/// The opening message of a run that carries a held run on; the brief is
/// already in the carried conversation, so this is what changed since.
fn continuation_message(run_id: &str, held: &HeldResume, clone: &str) -> String {
    if held.session.error.starts_with(STEP_BUDGET_HELD_REASON) {
        return format!(
            "Your earlier session {} ({} steps), whose conversation is above, stopped at the step budget before the task was finished; this is run {run_id}, carrying it on with a fresh step budget - the only continuation this job gets. The checkout at {clone} is the same one, with the commits you made and your uncommitted edits; a command that was running when you stopped did not finish, so run it again. Read a file again before relying on what you saw of it, do not start over and do not widen the scope: finish the task from where it stands with the same report rules - the targeted check, the commit, the SUMMARY line - and commit what passes before the budget notice comes, so finished work lands even if the rest does not.",
            held.session.id, held.session.steps
        );
    }
    format!(
        "The runner restarted for an update while you worked; this is run {run_id}, carrying on your earlier session {} ({} steps) whose conversation is above. The checkout at {clone} is the same one, with your uncommitted edits and the commits you made; a command that was running when you stopped did not finish, so run it again. Read a file again before relying on what you saw of it, do not start over, and finish the task with the same report rules. This run has a fresh step budget.",
        held.session.id, held.session.steps
    )
}

/// Whether the worker's exit is the harness stopping at the step budget
/// (`harness.max_steps`): exit code 1 with the harness's own stop report,
/// not a timeout, a cancel or a verdict of the worker's.
pub(crate) fn step_budget_stop(timeout: bool, exit_code: i32, summary: &str) -> bool {
    !timeout
        && exit_code == 1
        && summary.trim_start().starts_with(&format!(
            "harness stopped: {}",
            hive_store::HARNESS_STEP_LIMIT
        ))
}

/// Whether a run of `job_id` already held at the step budget: the hold is
/// given once per job, so a task two budgets cannot finish is judged and
/// recovered as before instead of being carried on without end.
fn step_budget_carried_on(c: &(impl crate::util::Querier + ?Sized), job_id: &str) -> bool {
    crate::util::must_one(
        c,
        "SELECT COUNT(*) AS n FROM runs WHERE job_id = ? AND status = 'held' AND summary LIKE ?",
        &[&job_id, &format!("{STEP_BUDGET_HELD_SUMMARY}%")],
    )
    .int("n")
        > 0
}

/// How long the report turn may take.
pub(crate) const REPORT_TURN_MINUTES: i32 = 20;

/// What the report turn needs of the run it carries on.
struct ReportTurn<'a> {
    job: &'a Row,
    project: &'a Row,
    run_id: &'a str,
    base: &'a str,
    clone: &'a str,
    changed: &'a [String],
    model_class: &'a str,
    spec: Option<&'a hive_harness::agents::AgentDef>,
    pins: &'a [String],
    env: &'a BTreeMap<String, String>,
}

/// The report turn (`landing.ui_report = require`): a verified run whose
/// change has a face and whose artifacts folder shows no change before and
/// after carries the worker's session on once more, for the before/after
/// report alone. The branch was taken from the verified clone already, so
/// nothing the turn commits lands; what it leaves in the folder is what the
/// landing reads. A session with no trajectory to carry on, or a project
/// that does not require the report, gets no turn.
fn report_turn(
    s: &hive_store::Store,
    ex: &mut dyn crate::exec::Executor,
    cancel: &hive_bus::CancellationToken,
    t: ReportTurn<'_>,
    log: &mut RunLog,
) {
    let home = s.home.clone();
    let job_id = t.job.str("id");
    let mode = crate::ui_evidence::Mode::of(s, &t.project.str("slug"));
    let before = crate::ui_evidence::UiEvidence::collect(&home, &job_id, t.changed, mode);
    if !before.wants_report() {
        return;
    }
    let trajectory = match s.harness_session_head(t.run_id) {
        Ok(Some(h)) if !h.trajectory.is_empty() && Path::new(&h.trajectory).is_file() => {
            h.trajectory
        }
        _ => {
            log.add(format!(
                "report turn: the change has a face and {}, but the worker's session {} left no trajectory to carry on; the landing waits for the report",
                before.report_trouble(),
                t.run_id
            ));
            return;
        }
    };
    let session = format!("{}-report", t.run_id);
    let message = crate::ui_evidence::report_turn_message(&before, t.base);
    let (cmd, _) = crate::exec::worker_command_as(
        s,
        &t.job.str("kind"),
        t.model_class,
        &message,
        t.clone,
        "",
        false,
        t.spec,
    );
    let cmd = match crate::exec::brief_off_argv(&*ex, &session, &message, cmd) {
        Ok(cmd) => cmd,
        Err(e) => {
            log.add(format!(
                "report turn could not run: {}; the landing waits for the report",
                err_message(&e)
            ));
            return;
        }
    };
    let mut cmd = crate::exec::with_resume(cmd, &trajectory);
    cmd.extend(["--session".to_string(), session.clone()]);
    if !t.pins.is_empty() {
        let mut prefix = crate::recipe::mise_exec_argv(t.pins);
        prefix.extend(cmd);
        cmd = prefix;
    }
    log.add(format!(
        "report turn: {} changed and {}; session {} carries on as {session} for the before/after report (up to {REPORT_TURN_MINUTES} min)",
        before.ui_paths.len(),
        before.report_trouble(),
        t.run_id
    ));
    match ex.run_worker(cancel, &cmd, t.env, REPORT_TURN_MINUTES) {
        Ok(out) => {
            log.add(tail(&out.stdout, 4000));
            log.add(tail(&out.stderr, 4000));
        }
        Err(e) => log.add(format!("report turn could not run: {}", err_message(&e))),
    }
    // A hold for the runner's restart has nothing to carry on: the job's
    // run is over, and the landing reads the folder as it stands.
    if let Ok(Some(h)) = s.held_harness_session_for_job(&job_id)
        && h.id == session
    {
        let _ = s.mark_harness_session_resumed(
            &h.id,
            "not carried on: the report turn of a finished run",
        );
    }
    let after = crate::ui_evidence::UiEvidence::collect(&home, &job_id, t.changed, mode);
    if after.reported() {
        log.add(format!(
            "report turn: {} shows {}",
            hive_harness::report::PAGE,
            after.report.counts()
        ));
    } else {
        log.add(format!(
            "report turn ended without a report ({}); the landing waits for one",
            after.report_trouble()
        ));
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RunOpts {
    pub(crate) dry_run: bool,
    pub(crate) worker_cmd: String,
    pub(crate) executor: String,
    pub(crate) cancel: hive_bus::CancellationToken,
    /// Why the daemon stopped this run, written before it cancels `cancel`
    /// (the restart's wait for runs on other machines, `daemon.rs`): a run
    /// on another machine stopped so has its commits fetched as kept work
    /// and its job queued again at no cost of an attempt.
    pub(crate) stop_why: StopWhy,
}

/// The daemon's reason for stopping one run, shared with the run.
#[derive(Debug, Clone, Default)]
pub(crate) struct StopWhy(Arc<std::sync::Mutex<String>>);

impl StopWhy {
    pub(crate) fn set(&self, why: &str) {
        if let Ok(mut w) = self.0.lock() {
            *w = why.to_string();
        }
    }
    pub(crate) fn get(&self) -> String {
        self.0.lock().map(|w| w.clone()).unwrap_or_default()
    }
}

pub(crate) type WallClockFn = fn() -> SystemTime;
pub(crate) static WALL_CLOCK: LazyLock<RwLock<WallClockFn>> =
    LazyLock::new(|| RwLock::new(SystemTime::now));

fn is_nil(r: &Row) -> bool {
    r.0.is_empty()
}
fn conn(s: &hive_store::Store) -> hive_store::PooledConn {
    s.conn().expect("store connection")
}
pub(crate) fn wall_clock() -> SystemTime {
    (*WALL_CLOCK.read().expect("wall clock"))()
}
fn since_seconds(start: SystemTime) -> f64 {
    wall_clock()
        .duration_since(start)
        .unwrap_or_default()
        .as_secs_f64()
}
fn tail(s: &str, n: usize) -> String {
    crate::executor::tail(s, n)
}
fn sorted_keys(m: &BTreeMap<String, String>) -> Vec<String> {
    m.keys().cloned().collect()
}
fn contains_str(list: &[String], s: &str) -> bool {
    list.iter().any(|x| x == s)
}
fn run_err(msg: impl Into<String>) -> anyhow::Error {
    crate::util::fail(1, msg.into()).into()
}
/// The runner stopped the run before its worker started (a restart the owner
/// asked for now, the stop grace); `phase` names what was under way. The run
/// settles `lost`, not as a setup that timed out.
#[derive(Debug, Clone)]
struct RunLost {
    phase: String,
}
impl std::fmt::Display for RunLost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{LOST_RUN_SUMMARY} (during {}; the worker had not started)",
            self.phase
        )
    }
}
impl std::error::Error for RunLost {}
fn stopped_by_runner(cancel: &hive_bus::CancellationToken) -> bool {
    cancel.err() == Some(hive_bus::ContextError::Canceled)
}
fn cli_err(code: i32, msg: impl Into<String>) -> anyhow::Error {
    crate::util::fail(code, msg.into()).into()
}
fn cancel_child(parent: &hive_bus::CancellationToken) -> hive_bus::CancellationToken {
    parent.child()
}

static GIT_COMMIT_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    // `git commit`, also behind global options with or without a value
    // (`git -C dir commit`, `git -c x=y commit`, `git --no-pager commit`).
    regex::Regex::new(r"\bgit\s+(?:-\S+\s+(?:[^-\s]\S*\s+)?)*commit\b").unwrap()
});
static COMMIT_MESSAGE_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r#"(?s)(?:^|\s)(?:-[a-zA-Z]*m|--message)[\s=]*(?:"((?:[^"\\]|\\.)*)"|'([^']*)'|([^\s;&|]+))"#,
    )
    .unwrap()
});

/// The worker's `git commit` that the harness killed at the step deadline,
/// when this run had one: the observation step with exit -1 (timed out)
/// whose command ran `git commit`. The pre-commit hook runs the project's
/// whole verification behind a shared lock, so the deadline lands on the
/// commit, not on the change. Sessions are matched by this run's id, or by
/// the job when they started during this run (a custom worker command may
/// not pass `HIVE_SESSION` on).
pub(crate) fn killed_commit_command(
    q: &(impl crate::util::Querier + ?Sized),
    run_id: &str,
    job_id: &str,
    run_started: &str,
) -> Option<String> {
    crate::util::must_rows(
        q,
        "SELECT st.command AS command FROM harness_steps st JOIN harness_sessions h ON h.id = st.session_id WHERE (h.run_id = ? OR (h.job_id = ? AND h.job_id != '' AND h.started_at >= ?)) AND st.kind = 'observation' AND st.exit_code = -1 ORDER BY h.started_at DESC, st.n DESC",
        &[&run_id, &job_id, &run_started],
    )
    .into_iter()
    .map(|r| r.str("command"))
    .find(|cmd| GIT_COMMIT_RE.is_match(cmd))
}

/// The message the worker gave its killed `git commit` (`-m`, `-qm`,
/// `--message=`), so the rescued commit keeps the worker's words; none when
/// it used an editor or `-F`.
pub(crate) fn commit_message_from_command(command: &str) -> Option<String> {
    let caps = COMMIT_MESSAGE_RE.captures(command)?;
    let text = caps
        .get(1)
        .map(|m| m.as_str().replace("\\\"", "\""))
        .or_else(|| caps.get(2).map(|m| m.as_str().to_string()))
        .or_else(|| caps.get(3).map(|m| m.as_str().to_string()))?;
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// The token cost of one run: what the harness sessions it ran
/// (`harness_sessions.run_id`, the `HIVE_SESSION` the runner hands the
/// worker) spent - prompt and completion tokens summed from their steps,
/// and the provider's priced cost where the provider reports one.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct RunCost {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cost: f64,
}

/// The model a run is started with (`--model`): the agent file's when the
/// job runs as one and `models.allowed` lets it through, else the kind's
/// and class's; empty for the provider's default or a custom worker
/// command, whose model is its own.
pub(crate) fn started_model(
    s: &hive_store::Store,
    kind: &str,
    model_class: &str,
    spec: Option<&hive_harness::agents::AgentDef>,
    worker_cmd: &str,
) -> String {
    if !worker_cmd.is_empty() {
        return String::new();
    }
    crate::exec::worker_model(s, kind, model_class, spec)
}

/// The model a finished run ran on: `ran_model` when it was started with
/// one, else the model its harness session recorded (the provider's
/// default, resolved by the harness), else empty (a worker that kept no
/// session).
pub(crate) fn resolved_run_model(
    q: &(impl crate::util::Querier + ?Sized),
    ran_model: &str,
    run_id: &str,
    job_id: &str,
    since: &str,
) -> String {
    if !ran_model.trim().is_empty() {
        return ran_model.to_string();
    }
    q.conn()
        .query_row(
            "SELECT model FROM harness_sessions WHERE (run_id = ?1 OR (run_id = '' AND job_id = ?2 AND job_id != '' AND started_at >= ?3)) AND model != '' ORDER BY started_at DESC LIMIT 1",
            rusqlite::params![run_id, job_id, since],
            |r| r.get::<_, String>(0),
        )
        .unwrap_or_default()
}

/// Sums the run's harness sessions: the ones that name the run, and a
/// session of the job started during the run that names none (a worker
/// started without `HIVE_SESSION`); zero for a worker that kept none (a
/// custom worker command).
pub(crate) fn run_cost(
    q: &(impl crate::util::Querier + ?Sized),
    run_id: &str,
    job_id: &str,
    since: &str,
) -> RunCost {
    q.conn()
        .query_row(
            "SELECT COALESCE(SUM(prompt_tokens), 0), COALESCE(SUM(completion_tokens), 0), COALESCE(SUM(cost), 0) FROM harness_sessions WHERE run_id = ?1 OR (run_id = '' AND job_id = ?2 AND job_id != '' AND started_at >= ?3)",
            rusqlite::params![run_id, job_id, since],
            |r| {
                Ok(RunCost {
                    prompt_tokens: r.get(0)?,
                    completion_tokens: r.get(1)?,
                    cost: r.get(2)?,
                })
            },
        )
        .unwrap_or_default()
}

/// A cost for the log: two decimals, or four under a cent.
pub(crate) fn trim_cost(cost: f64) -> String {
    if cost > 0.0 && cost < 0.01 {
        format!("{cost:.4}")
    } else {
        format!("{cost:.2}")
    }
}

pub(crate) fn runner_stopped(s: &hive_store::Store) -> String {
    if s.setting("stop", "0") == "1" {
        return "the runner is stopped in the store (hive start to resume)".to_string();
    }
    let c = conn(s);
    let brain = crate::board::brain_project(&c);
    if !is_nil(&brain) {
        let stop = Path::new(&brain.str("path"))
            .join("docs")
            .join("evolution")
            .join("STOP");
        if stop.exists() {
            return format!(
                "docs/evolution/STOP is present in the brain ({}); a human removes it",
                brain.str("path")
            );
        }
    }
    String::new()
}

/// What the daily cap (`jobs_per_day`) has counted today: the distinct
/// jobs that ran, not the runs (`hive_store::DAILY_CAP_COUNT_SQL`). A run
/// the daemon asked to hold - for the owner's work (yield_lane) or its own
/// restart - is the same job carried on later, and a run lost to a restart
/// never started its work; on 2026-11-03 sixty-six held runs of three jobs
/// filled a cap of ten by noon and the fleet stood until the owner found
/// out.
pub(crate) fn runs_today(s: &hive_store::Store, now: &str) -> i32 {
    let c = conn(s);
    let day = now.get(..10).unwrap_or(now);
    c.query_row(hive_store::DAILY_CAP_COUNT_SQL, [day], |r| {
        r.get::<_, i64>(0)
    })
    .unwrap_or(0) as i32
}

/// [`runs_today`] with the jobs a lease pass has taken so far (`taken`)
/// counted in - once each. A taken job's run row goes in from the run's
/// own thread, so by the time the daemon fills its next slot the row may
/// or may not be there; adding the pass's count to the table's count made
/// the first job of a tick count twice whenever the thread was quick, and
/// a cap of two then held the second slot with "2 of 2 jobs today" after
/// one job had started.
pub(crate) fn jobs_today_with(s: &hive_store::Store, now: &str, taken: &[String]) -> i32 {
    let c = conn(s);
    let day = now.get(..10).unwrap_or(now);
    let mut jobs: BTreeSet<String> = c
        .prepare(hive_store::DAILY_CAP_JOBS_SQL)
        .and_then(|mut st| {
            st.query_map([day], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<BTreeSet<_>>>()
        })
        .unwrap_or_default();
    jobs.extend(taken.iter().cloned());
    jobs.len() as i32
}

pub(crate) fn daily_cap(s: &hive_store::Store) -> i32 {
    hive_store::daily_cap_of(&s.setting("jobs_per_day", ""), 2) as i32
}

/// The idle reason a full cap leaves; the daemon reads it back
/// (`daily_cap_hold`) to tell the owner once.
pub(crate) const DAILY_CAP_REASON: &str = "daily cap reached";

/// What a full cap stops: the fleet's own work. The owner's jobs
/// (`OWNER_DRIVEN_SQL`: the lead's sittings, answers, task jobs, security
/// updates) go on past it - the cap is the owner's budget for the fleet,
/// not a lock on the owner's own asks - so the slot leases those alone.
pub(crate) fn daily_cap_reason(done: i32, cap: i32) -> String {
    format!(
        "{DAILY_CAP_REASON}: {done} of {cap} jobs today (setting jobs_per_day; 0 lifts it); only the owner's own tasks, answers and lead sittings start until midnight UTC"
    )
}

/// `taken` names the jobs this lease pass has taken already (the daemon's
/// tick, one slot after another): the cap counts them with today's
/// (`jobs_today_with`). `held` names projects this call may not lease, with the unfit reason each
/// carries - the daemon's disk headroom for the local executor (diskroom.rs);
/// a project's own marks and blocks come first. `fits` names the project and
/// kind pairs the call may lease - the daemon's restart window, which short
/// runs still fit when long ones do not; empty leases any, and the owner's
/// work (`OWNER_DRIVEN_SQL`) fits every window.
#[allow(clippy::too_many_arguments)]
pub(crate) fn take_job(
    s: &hive_store::Store,
    agent: &str,
    project: &str,
    job_id: &str,
    dry_run: bool,
    now: &str,
    executor: &str,
    taken: &[String],
    held: &BTreeMap<String, String>,
    fits: &[(String, String)],
) -> Result<(Row, Row, String)> {
    take_job_first(
        s, agent, project, job_id, dry_run, now, executor, taken, held, fits, false, false,
    )
}

/// [`take_job`] for a slot the daemon keeps for the work class
/// (`work_first`, jobs.rs `LeaseOpts`): queued improve, task and experiment
/// jobs go before the meta kinds. `keep_slot` marks the machine's last free
/// slot, which the owner's work past its project's share does not take.
#[allow(clippy::too_many_arguments)]
pub(crate) fn take_job_first(
    s: &hive_store::Store,
    agent: &str,
    project: &str,
    job_id: &str,
    dry_run: bool,
    now: &str,
    executor: &str,
    taken: &[String],
    held: &BTreeMap<String, String>,
    fits: &[(String, String)],
    work_first: bool,
    keep_slot: bool,
) -> Result<(Row, Row, String)> {
    if let why = runner_stopped(s)
        && !why.is_empty()
    {
        return Ok((Row::new(), Row::new(), why));
    }
    // Past the cap the slot still takes the owner's own work (the owner
    // lane, jobs.rs OWNER_DRIVEN_SQL); the fleet's waits for midnight. With
    // the marker there the lease is owner-only whatever a restart window's
    // kinds beside it say: the owner's work fits every window.
    let mut fits = fits.to_vec();
    let mut cap_note = String::new();
    if !dry_run {
        let cap = daily_cap(s);
        let done = jobs_today_with(s, now, taken);
        if cap > 0 && done >= cap {
            cap_note = daily_cap_reason(done, cap);
            if !crate::jobs::is_owner_lane(&fits) {
                fits.extend(crate::jobs::owner_lane_fits());
            }
        }
    }
    let fits = fits.as_slice();
    let mut ex_name = executor.to_string();
    if ex_name.is_empty() {
        ex_name = crate::executor::LOCAL_EXECUTOR.to_string();
    }
    let exrow = crate::executor::executor_row(s, &ex_name);
    if is_nil(&exrow) {
        return Err(cli_err(
            2,
            format!("unknown executor {} (hive executor list)", executor),
        ));
    }
    if exrow.str("status") != "active" {
        return Ok((
            Row::new(),
            exrow.clone(),
            format!("executor {} is {}", exrow.str("name"), exrow.str("status")),
        ));
    }
    let mut projects = Vec::new();
    let mut any_proj = exrow.str("projects") == "*";
    if !any_proj {
        projects = exrow
            .str("projects")
            .split(',')
            .map(str::to_string)
            .collect();
    }
    if !project.is_empty() && !any_proj && !contains_str(&projects, project) {
        return Ok((
            Row::new(),
            exrow.clone(),
            format!(
                "executor {} does not serve {} (projects {})",
                exrow.str("name"),
                project,
                exrow.str("projects")
            ),
        ));
    }
    let c = conn(s);
    let mut unfit = crate::recipe::unfit_projects(&c, &exrow.str("name"), now);
    let only = if project.is_empty() {
        Vec::new()
    } else {
        vec![project.to_string()]
    };
    for (slug, why) in crate::recipe::recipe_blocks(s, &exrow.str("name"), &only) {
        unfit.entry(slug).or_insert(why);
    }
    for slug in crate::checkout::held_checkouts().keys() {
        unfit.insert(
            slug.clone(),
            crate::recipe::RECIPE_UNFIT_CHECKOUT.to_string(),
        );
    }
    for (slug, why) in held {
        unfit.entry(slug.clone()).or_insert_with(|| why.clone());
    }
    let check_projects = if !project.is_empty() {
        vec![project.to_string()]
    } else {
        projects.clone()
    };
    for slug in check_projects {
        if unfit.contains_key(&slug) {
            continue;
        }
        if let (reason, true) = crate::recover::environment_health_block(
            &c,
            exrow.str("name"),
            slug.clone(),
            now.to_string(),
        ) {
            unfit.insert(slug, reason);
        }
    }
    // A project a repeated setup failure froze on this machine still gets its
    // repair job here, the one job that can lift the mark (setup_repair.rs);
    // nothing else of the project runs until it does.
    let only_source: Vec<(String, String)> = unfit
        .iter()
        .filter(|(_, why)| crate::setup_repair::unfit_is_setup(why))
        .map(|(slug, _)| (slug.clone(), crate::setup_repair::repair_source(slug)))
        .collect();
    let repairable = |slug: &str| only_source.iter().any(|(s, _)| s == slug);
    if !unfit.is_empty() {
        if !project.is_empty() && unfit.contains_key(project) && !repairable(project) {
            let why = unfit.get(project).cloned().unwrap_or_default();
            return Ok((
                Row::new(),
                exrow.clone(),
                format!(
                    "executor {} {} for {} ({})",
                    exrow.str("name"),
                    crate::recipe::unfit_phrase(&why),
                    project,
                    crate::recipe::tool_hint(&why)
                ),
            ));
        }
        if any_proj {
            projects = crate::util::must_rows(
                &c,
                "SELECT slug FROM projects WHERE status = 'active'",
                &[],
            )
            .into_iter()
            .map(|r| r.str("slug"))
            .collect();
            any_proj = false;
        }
        projects.retain(|p| !unfit.contains_key(p) || repairable(p));
        if projects.is_empty() {
            let slugs = sorted_keys(&unfit);
            let reasons = slugs
                .iter()
                .map(|slug| format!("{} for {}", crate::recipe::unfit_phrase(&unfit[slug]), slug))
                .collect::<Vec<_>>();
            let first = slugs
                .first()
                .and_then(|s| unfit.get(s))
                .cloned()
                .unwrap_or_default();
            return Ok((
                Row::new(),
                exrow.clone(),
                format!(
                    "executor {} {} ({})",
                    exrow.str("name"),
                    reasons.join("; "),
                    crate::recipe::tool_hint(&first)
                ),
            ));
        }
    }
    let caps = crate::executor::caps(&exrow);
    let reserved = crate::executor::reserved(&exrow);
    let job = crate::jobs::lease_job(
        s,
        agent.to_string(),
        now.to_string(),
        crate::jobs::LeaseOpts {
            project: project.to_string(),
            job_id: job_id.to_string(),
            projects: projects.clone(),
            any_proj,
            fits: fits.to_vec(),
            only_source,
            work_first,
            keep_slot,
            caps: Some(caps.clone()),
            reserved,
            executor: exrow.str("name"),
            ..Default::default()
        },
    )?;
    if is_nil(&job) {
        // The cap is full and no owner work waits: the reason is the cap,
        // whatever else the queue holds.
        if !cap_note.is_empty() {
            return Ok((Row::new(), exrow, cap_note));
        }
        // A project frozen here by a repeated setup failure passed the gate
        // above for its repair job alone; with none queued the wait reason
        // still names the mark, not an empty queue.
        if !project.is_empty()
            && let Some(why) = unfit.get(project)
        {
            return Ok((
                Row::new(),
                exrow.clone(),
                format!(
                    "executor {} {} for {} ({}); only its setup repair job runs here and none is queued",
                    exrow.str("name"),
                    crate::recipe::unfit_phrase(why),
                    project,
                    crate::recipe::tool_hint(why)
                ),
            ));
        }
        let mut why = "no queued job".to_string();
        if !project.is_empty() {
            why.push_str(&format!(" for {project}"));
        }
        if exrow.str("name") != crate::executor::LOCAL_EXECUTOR {
            why.push_str(&format!(" on {}", exrow.str("name")));
        }
        if project.is_empty() {
            why.push_str(&held_queue_note(&c, &exrow.str("name"), &unfit));
        }
        why.push_str(&needs_queue_note(
            &c, &exrow, &caps, reserved, &projects, any_proj, project,
        ));
        if exrow.str("name") != crate::executor::LOCAL_EXECUTOR {
            why.push_str(&crate::class_red::kept_off_note(&c, &exrow, now, project));
        }
        return Ok((Row::new(), exrow, why));
    }
    Ok((job, exrow, String::new()))
}

/// Names the queued jobs an executor cannot take because they need what it
/// does not offer, or because it is reserved and they need nothing, so an
/// idle line never says "no queued job" over a queue the machine cannot
/// serve.
fn needs_queue_note(
    c: &(impl crate::util::Querier + ?Sized),
    exrow: &Row,
    caps: &BTreeSet<String>,
    reserved: bool,
    projects: &[String],
    any_proj: bool,
    project: &str,
) -> String {
    let mut sql = "SELECT j.needs FROM jobs j JOIN projects p ON p.slug = j.project WHERE j.status = 'queued' AND p.status = 'active' AND j.kind != ?".to_string();
    let lead = crate::lead::LEAD_KIND.to_string();
    let mut vals: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(lead)];
    if !project.is_empty() {
        sql.push_str(" AND j.project = ?");
        vals.push(Box::new(project.to_string()));
    } else if !any_proj && !projects.is_empty() {
        let marks = vec!["?"; projects.len()].join(",");
        sql.push_str(&format!(" AND j.project IN ({marks})"));
        for p in projects {
            vals.push(Box::new(p.clone()));
        }
    }
    let refs: Vec<&dyn rusqlite::ToSql> = vals.iter().map(|v| &**v).collect();
    let rows = crate::util::query_rows(c, &sql, &refs).unwrap_or_default();
    let mut unmet: BTreeMap<String, i32> = BTreeMap::new();
    let mut plain = 0;
    for r in &rows {
        let needs = r.str("needs");
        if needs.is_empty() {
            plain += 1;
            continue;
        }
        let missing = hive_store::needs::missing(&needs, caps);
        if !missing.is_empty() {
            *unmet.entry(missing.join(",")).or_insert(0) += 1;
        }
    }
    let name = exrow.str("name");
    let mut parts = Vec::new();
    if !unmet.is_empty() {
        let list = unmet
            .iter()
            .map(|(m, n)| format!("{n} need {m}"))
            .collect::<Vec<_>>()
            .join(", ");
        parts.push(format!(
            "{list}, which {name} does not offer (it offers {}; hive executor list)",
            crate::executor::caps_text(exrow)
        ));
    }
    if reserved && plain > 0 {
        parts.push(format!(
            "{plain} need nothing in particular and {name} is reserved for the work that needs it"
        ));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("; queued: {}", parts.join("; "))
    }
}

/// Names the queued jobs an executor cannot take because their project is
/// unfit for it, so an idle line never says "no queued job" over a queue the
/// executor is holding; the hint is the same one the unfit mark posted.
fn held_queue_note(
    c: &(impl crate::util::Querier + ?Sized),
    executor: &str,
    unfit: &BTreeMap<String, String>,
) -> String {
    if unfit.is_empty() {
        return String::new();
    }
    let slugs = sorted_keys(unfit);
    let marks = vec!["?"; slugs.len()].join(",");
    let sql = format!(
        "SELECT j.project, COUNT(*) AS n FROM jobs j JOIN projects p ON p.slug = j.project WHERE j.status = 'queued' AND p.status = 'active' AND j.kind != ? AND j.project IN ({marks}) GROUP BY j.project ORDER BY j.project"
    );
    let lead = crate::lead::LEAD_KIND.to_string();
    let mut vals: Vec<&dyn rusqlite::ToSql> = vec![&lead];
    for slug in &slugs {
        vals.push(slug);
    }
    let held = crate::util::query_rows(c, &sql, &vals).unwrap_or_default();
    if held.is_empty() {
        return String::new();
    }
    let total: i32 = held.iter().map(|r| r.int("n")).sum();
    let parts = held
        .iter()
        .map(|r| {
            let slug = r.str("project");
            format!(
                "{} for {slug} ({executor} {})",
                r.int("n"),
                crate::recipe::unfit_phrase(&unfit[&slug])
            )
        })
        .collect::<Vec<_>>();
    let first = held
        .first()
        .map(|r| r.str("project"))
        .and_then(|slug| unfit.get(&slug).cloned())
        .unwrap_or_default();
    format!(
        "; {total} queued wait: {}; {}",
        parts.join(", "),
        crate::recipe::tool_hint(&first)
    )
}

pub(crate) fn work_once(
    s: &hive_store::Store,
    agent: &str,
    project: &str,
    job_id: &str,
    now: &str,
    mut o: RunOpts,
) -> Result<(Row, String)> {
    let (job, exrow, reason) = take_job(
        s,
        agent,
        project,
        job_id,
        o.dry_run,
        now,
        &o.executor,
        &[],
        &BTreeMap::new(),
        &[],
    )?;
    if is_nil(&job) {
        return Ok((Row::new(), reason));
    }
    o.executor = exrow.str("name");
    let run = run_job(s, job, agent, now, o)?;
    Ok((run, String::new()))
}

pub(crate) fn squeeze(s: &str, n: usize) -> String {
    let t = tail(&s.split_whitespace().collect::<Vec<_>>().join(" "), n);
    if t.is_empty() {
        "no output".to_string()
    } else {
        t
    }
}

fn row_to_ballot(v: Value) -> BallotItem {
    BallotItem {
        id: v
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        key: v
            .get("key")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        summary: v
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        body: v
            .get("body")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        up: v.get("up").and_then(Value::as_i64).unwrap_or(0) as i32,
        down: v.get("down").and_then(Value::as_i64).unwrap_or(0) as i32,
        kind: v
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        hash: v
            .get("hash")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    }
}

/// What `prepare_setup` learned: the parsed recipe (if any), whether a recipe
/// was found at all, where it came from, the setup lines shown in the brief
/// and the tool pins installed.
struct PreparedSetup {
    recipe: Option<crate::recipe::Recipe>,
    found: bool,
    source: Option<crate::recipe::RecipeSource>,
    setup_shown: String,
    pins: Vec<String>,
}

/// `repair` is a setup repair job (setup_repair.rs): the recipe's setup lines
/// are what it fixes, so they are not run in its clone and their failure
/// plans nothing.
#[allow(clippy::too_many_arguments)]
fn prepare_setup(
    s: &hive_store::Store,
    project: &Row,
    root: &str,
    base: &str,
    clone: &str,
    ex: &dyn Executor,
    cancel: &hive_bus::CancellationToken,
    dry_run: bool,
    repair: bool,
    log: &mut RunLog,
) -> Result<PreparedSetup> {
    let slug = project.str("slug");
    let (recipe, problems, found, source) = crate::recipe::read_recipe_from(s, &slug, root, base);
    if !problems.is_empty() {
        let now = s.now();
        crate::recipe::recipe_blocker(s, project.clone(), &problems, base, source.as_ref(), &now);
        return Err(
            crate::recipe::RecipeInvalid(crate::recipe::recipe_problem_line(
                &slug,
                &problems,
                base,
                source.as_ref(),
            ))
            .into(),
        );
    }
    if found {
        crate::recipe::recipe_unblocked(s, project, base, source.as_ref(), &s.now());
    }
    let Some(r) = recipe.clone() else {
        return Ok(PreparedSetup {
            recipe: None,
            found,
            source,
            setup_shown: String::new(),
            pins: Vec::new(),
        });
    };
    let lack_recipe = crate::recipe::hands_lack_recipe(&s.conn()?, &ex.name());
    match crate::recipe::recipe_block(&r, lack_recipe).as_str() {
        crate::recipe::RECIPE_UNFIT_SERVICES => return Err(crate::executor::ExecutorUnfit { msg: format!("{} lists services ({}) that no machine starts; {} waits until an owner clears the list", crate::recipe::RECIPE_PATH, r.services.iter().take(4).cloned().collect::<Vec<_>>().join(", "), project.str("slug")), tool: crate::recipe::RECIPE_UNFIT_SERVICES.to_string(), project: project.str("slug"), detail: String::new() }.into()),
        crate::recipe::RECIPE_UNFIT_HANDS => return Err(crate::executor::ExecutorUnfit { msg: format!("{} pins tools or env that the hands on {} predate (their probe does not answer recipe=1); {} jobs run on the hub until the hands are refreshed", crate::recipe::RECIPE_PATH, ex.name(), project.str("slug")), tool: crate::recipe::RECIPE_UNFIT_HANDS.to_string(), project: project.str("slug"), detail: String::new() }.into()),
        _ => {}
    }

    let pins = r.tools.clone();
    let setup_budget = crate::recipe::SETUP_MINUTES;
    if !pins.is_empty() && !dry_run {
        let cmd = crate::recipe::mise_install_command(&pins, clone);
        let (rc, mout) = ex.verify(cancel, &cmd, setup_budget);
        log.add(format!(
            "$ mise install {}  # tools from {}\n{}",
            pins.join(" "),
            crate::recipe::RECIPE_PATH,
            tail(&mout, 20000)
        ));
        match rc {
            None => {
                if stopped_by_runner(cancel) {
                    return Err(RunLost {
                        phase: "mise install".into(),
                    }
                    .into());
                }
                return Err(run_err(format!(
                    "mise install on {} timed out after {} minutes ({}); the job goes back to the queue",
                    ex.name(),
                    setup_budget,
                    pins.join(", ")
                )));
            }
            Some(0) => {}
            Some(code) => {
                let t = squeeze(&mout, 300);
                if code == 127
                    || regex::Regex::new(
                        r"mise: (command )?not found|mise: No such file or directory",
                    )
                    .unwrap()
                    .is_match(&mout)
                {
                    return Err(crate::executor::ExecutorUnfit { msg: format!("{} pins tools but `mise` is not installed on {} ({}); {} jobs run elsewhere until it is and `hive executor probe {}` passes", crate::recipe::RECIPE_PATH, ex.name(), t, project.str("slug"), ex.name()), tool: crate::recipe::RECIPE_UNFIT_MISE.to_string(), project: project.str("slug"), detail: t }.into());
                }
                let failed = crate::recipe::mise_failed_pin(&pins, &mout);
                return Err(crate::executor::ExecutorUnfit { msg: format!("mise could not install {} on {} ({}); {} jobs run elsewhere until it can and `hive executor probe {}` passes", failed, ex.name(), t, project.str("slug"), ex.name()), tool: format!("tool:{failed}"), project: project.str("slug"), detail: t }.into());
            }
        }
    }
    let (setup_cmd, setup_files, setup_shown) = crate::recipe::project_setup(root, base, &r, clone);
    if repair && !setup_cmd.is_empty() {
        log.add(format!(
            "setup from {} not run: this job repairs it (source {}{})",
            setup_files.join(", "),
            crate::setup_repair::SOURCE_PREFIX,
            project.str("slug")
        ));
    }
    if !setup_cmd.is_empty() && !dry_run && !repair {
        let (rc, sout) = ex.verify(cancel, &setup_cmd, setup_budget);
        log.add(format!(
            "$ {}  # setup from {}\n{}",
            setup_cmd,
            setup_files.join(", "),
            tail(&sout, 20000)
        ));
        match rc {
            None => {
                if stopped_by_runner(cancel) {
                    return Err(RunLost {
                        phase: "setup".into(),
                    }
                    .into());
                }
                return Err(run_err(format!(
                    "setup on {} timed out after {} minutes ({}); the job goes back to the queue",
                    ex.name(),
                    setup_budget,
                    setup_files.join(", ")
                )));
            }
            Some(0) => {}
            Some(code) => {
                let tool = crate::recipe::missing_tool(&setup_cmd, code, &sout);
                let t = squeeze(&sout, 300);
                if !tool.is_empty() {
                    return Err(crate::executor::ExecutorUnfit { msg: format!("setup on {} needs `{}`, which is not installed or usable there ({}); {} jobs run elsewhere until it is fixed and `hive executor probe {}` passes", ex.name(), tool, t, project.str("slug"), ex.name()), tool, project: project.str("slug"), detail: t }.into());
                }
                let summary = crate::recipe::setup_failure_summary(&ex.name(), code, &t);
                let c = conn(s);
                if crate::recipe::repeated_setup_failure(
                    &c,
                    &s.now(),
                    &project.str("slug"),
                    &ex.name(),
                    &summary,
                ) {
                    let tool = crate::recipe::setup_unfit_tool(&setup_files);
                    // The same failure twice at the project's own commit is
                    // the project's fault: the repair job fixes it while the
                    // unfit mark keeps the other jobs off this machine.
                    let planned = crate::setup_repair::plan_repair(
                        s,
                        project,
                        &crate::setup_repair::SetupFailure {
                            base: base.to_string(),
                            files: setup_files.join(", "),
                            cmd: setup_cmd.clone(),
                            rc: Some(code),
                            tail: squeeze(&sout, 2500),
                            executor: ex.name(),
                            seen: "run",
                            sync: false,
                        },
                        "daemon",
                        &s.now(),
                    );
                    if !planned.note.is_empty() {
                        log.add(planned.note.clone());
                    }
                    let repair_note = if planned.job.is_empty() {
                        String::new()
                    } else {
                        format!("; repair job {} planned", planned.job)
                    };
                    return Err(crate::executor::ExecutorUnfit { msg: format!("setup on {} for {} failed the same way again ({}); {} jobs run elsewhere until the machine is fixed and `hive executor probe {}` passes{repair_note}", ex.name(), setup_files.join(", "), t, project.str("slug"), ex.name()), tool, project: project.str("slug"), detail: t }.into());
                }
                return Err(run_err(summary));
            }
        }
    }
    Ok(PreparedSetup {
        recipe: Some(r),
        found,
        source,
        setup_shown,
        pins,
    })
}

pub(crate) fn run_job(
    s: &hive_store::Store,
    job: Row,
    agent: &str,
    now: &str,
    o: RunOpts,
) -> Result<Row> {
    let c = conn(s);
    let mut project = crate::board::project_by_slug(&c, job.str("project"));
    if is_nil(&project) {
        return Err(cli_err(
            2,
            format!(
                "Job {} names an unknown project {}",
                job.str("id"),
                job.str("project")
            ),
        ));
    }
    let root = project.str("path");
    // A brain without code (`brain.code none`) may hold no checkout of the
    // project at all; its runs work in an empty workspace.
    if !Path::new(&root).join(".git").exists() && !crate::jobs::brain_code_none(s) {
        return Err(cli_err(
            2,
            format!(
                "Project {} at {} is not a git checkout",
                project.str("slug"),
                root
            ),
        ));
    }
    let minutes = crate::util::setting_int(s, "minutes_per_job");
    let home = s.home.clone();
    let work_dir = Path::new(&home).join("work");
    let logs_dir = Path::new(&home).join("logs");
    std::fs::create_dir_all(&work_dir).ok();
    std::fs::create_dir_all(&logs_dir).ok();
    let ex_name = if o.executor.is_empty() {
        crate::executor::LOCAL_EXECUTOR.to_string()
    } else {
        o.executor.clone()
    };
    let exrow = crate::executor::executor_row(s, &ex_name);
    if is_nil(&exrow) {
        return Err(cli_err(
            2,
            format!("unknown executor {} (hive executor list)", o.executor),
        ));
    }
    let run_id = crate::util::new_id("run", now);
    let mode = if o.dry_run {
        "dry-run"
    } else if !o.worker_cmd.is_empty() {
        "custom"
    } else {
        "real"
    };
    let log_path = logs_dir.join(format!("{run_id}.log")).display().to_string();
    let mut log = RunLog::default();
    let mut ex = crate::exec::make_executor(
        exrow,
        s,
        &home,
        job.clone(),
        project.clone(),
        agent,
        &mut log,
    )?;
    let clone = ex.clone();
    crate::util::exec1(
        &c,
        "INSERT INTO runs (id, job_id, agent, project, started_at, status, clone, log, mode, executor) VALUES (?, ?, ?, ?, ?, 'running', ?, ?, ?, ?)",
        &[
            &run_id,
            &job.str("id"),
            &agent,
            &project.str("slug"),
            &now,
            &clone,
            &log_path,
            &mode,
            &ex.name(),
        ],
    );
    crate::util::event_at(
        &c,
        now,
        agent,
        "run.start",
        &run_id,
        json!({"job": job.str("id"), "mode": mode, "executor": ex.name()}),
    );
    let cancel = cancel_child(&o.cancel);
    let owner_cancelled = Arc::new(AtomicBool::new(false));
    let stop_watch = watch_run_harness_cancel(&cancel, s, &run_id, Arc::clone(&owner_cancelled));
    // A run on a hand: the machine is probed while the run goes, and one
    // that stops answering ends the run instead of holding its slot, its
    // lease and a restart for the run's whole budget (hand_watch.rs).
    let mut hand_watch = crate::hand_watch::watch_hand(
        &cancel,
        s,
        &crate::executor::executor_row(s, &ex.name()),
        &run_id,
        &job.str("id"),
        log.clone(),
    );
    if job.str("kind") == "baseline" {
        // ADR-031: the runner measures the default branch itself; no clone, brief, worker or landing.
        let r = crate::baseline::run_baseline(
            s,
            job,
            project,
            agent.to_string(),
            now.to_string(),
            run_id,
            log_path,
            &*ex,
            o,
        );
        stop_watch();
        hand_watch.stop();
        return r;
    }
    let model_class = crate::lead_roles::model_class_of_job(s, &job);
    // The agent file the job is pinned to, else the one whose kinds and
    // select fit the card, if any: the worker runs as it (`--agent NAME`),
    // the clone being made from the checkout whose files are read here.
    let chosen = crate::agent_specs::spec_for_job(s, &job, &project.str("slug"), &root);
    if let Some(w) = &chosen.warning {
        eprintln!("run {run_id}: {w}");
        log.add(w.clone());
    }
    let agent_spec = chosen.spec;
    if let Some(a) = &agent_spec {
        let travels = crate::exec::agent_flag_travels(a, ex.remote());
        log.add(format!(
            "runs as the {} agent ({}){}",
            a.name,
            a.source,
            if travels {
                String::new()
            } else {
                "; a remote hand cannot read that layer, so only its model, step cap and read-only flag travel".to_string()
            }
        ));
    }
    // The model this run is started with (`--model`), empty for the
    // provider's default or a custom worker command: the lead's `DECIDE:`
    // lines record it, not the setting (ADR-072 part 5).
    let mut ran_model = String::new();
    if agent != crate::util::HUMAN_AGENT {
        let model = started_model(
            s,
            &job.str("kind"),
            &model_class,
            agent_spec.as_ref(),
            &o.worker_cmd,
        );
        ran_model = model.clone();
        let a = crate::board::agent_row(&c, agent.to_string());
        let kind = if is_nil(&a) {
            "worker".to_string()
        } else {
            a.str("kind")
        };
        crate::board::join_agent(
            s,
            agent.to_string(),
            String::new(),
            String::new(),
            run_id.clone(),
            String::new(),
            kind,
            String::new(),
        )?;
        if !model.is_empty() {
            crate::util::exec1(
                &c,
                "UPDATE agents SET model = ? WHERE name = ?",
                &[&model, &agent],
            );
        }
    }
    let started = wall_clock();
    let mut status = "failed".to_string();
    let mut commits = 0;
    let mut branch = String::new();
    let mut verify_note = String::new();
    let mut summary = String::new();
    let mut exit_code: Option<i32> = None;
    let mut parsed = crate::brief::parse_result_text(None);
    let mut outcome = crate::brief::Outcome::default();
    let mut transport_error = false;
    let mut fault = false;
    // The run's commits are on the machine and could not be fetched: its
    // clone stays there (Executor::keep_work) instead of being cleaned.
    let mut work_not_fetched = false;
    // The run's check passed and its commits were fetched to the checkout:
    // a job requeued under it for a lapsed lease takes this result instead
    // of starting again from nothing (TASK-179).
    let mut checked_and_fetched = false;
    // The runner stopped the run's check of finished work (a forced
    // restart): the base the commits stand on, so they can be kept on a
    // branch for the job's next run to check (unchecked.rs).
    let mut check_stopped: Option<String> = None;
    // The worker's report lines, kept with that work for the next run.
    let mut worker_report = String::new();
    // This run checks the work an earlier run's check left unfinished
    // instead of starting a worker (unchecked.rs).
    let mut unchecked: Option<crate::unchecked::Unchecked> = None;
    // The stopped work is kept on its branch and written down: the job goes
    // back to the queue at no cost of an attempt.
    let mut unchecked_kept = false;
    // The commits a stopped run left in its clone could not be fetched
    // again: the run ends and that clone stays, where they are.
    let mut stranded_clone_kept = false;
    let mut unfit: Option<crate::executor::ExecutorUnfit> = None;
    // The recipe did not parse: the project's fault, not the run's, so the
    // job keeps its attempt and waits for dispatch to release the project.
    let mut recipe_invalid = false;
    let mut pubkey = String::new();
    let mut worker_reported = String::new();
    // The files the worker's commits changed, read once the commits are
    // counted: the evidence line of the result post tells a change with a
    // face (a page, a view) from one without.
    let mut changed_files: Vec<String> = Vec::new();
    // The ballot the brief showed; `VOTE:` lines may only name these cards.
    let mut run_ballot: Vec<BallotItem> = Vec::new();
    // The shared Cargo build-cache lane this run builds into, held until the
    // clone is gone so no concurrent run of the project shares it.
    let mut build_cache: Option<crate::buildcache::Lane> = None;
    // A run on a hand: the target dir of its own its worker and gate build
    // into, which the run-end check on the hand is given too.
    let mut hand_target_dir = String::new();

    // The vault secrets granted to this run, if any; wiped when dropped.
    let mut secret_grant = crate::secrets::SecretGrant::default();
    let body_result: Result<()> = (|| {
        let mut resume = resumable_held_run(
            s,
            &job.str("id"),
            agent,
            &run_id,
            &*ex,
            !o.worker_cmd.is_empty(),
            &mut log,
        );
        // A run that carries on signs with the held run's key, so the
        // commits of both pass the landing check together.
        pubkey = match resume
            .as_ref()
            .map(|_| crate::keys::reuse_run_key(&ex.key_path()))
        {
            Some(Ok(key)) => key,
            Some(Err(e)) => {
                log.add(err_message(&e));
                if let Some(r) = resume.take() {
                    let _ = s.mark_harness_session_resumed(
                        &r.session.id,
                        &format!("not carried on: run {run_id} started over (the signing key was unreadable)"),
                    );
                }
                crate::keys::make_run_key(&ex.key_path(), agent, &run_id)?
            }
            None => crate::keys::make_run_key(&ex.key_path(), agent, &run_id)?,
        };
        crate::exec::preflight_run_key(&ex.key_path())?;
        crate::util::exec1(
            &c,
            "UPDATE runs SET signing_pubkey = ? WHERE id = ?",
            &[&pubkey, &run_id],
        );
        if let Some(r) = &resume {
            ex.set_resume(&r.base);
        }
        // An earlier run was stopped in its check and its commits could not
        // be fetched onto the kept branch: they are only in that run's
        // clone, which this run's own clone would remove. Fetch them first;
        // when that fails again the run ends and the clone stays (TASK-152
        // item 1).
        if resume.is_none()
            && !ex.remote()
            && !o.dry_run
            && let Some(mut u) = crate::unchecked::stranded(s, &job.str("id"), &root)
        {
            match crate::unchecked::fetch_kept_clone(&root, &u) {
                Ok(()) => {
                    log.add(format!(
                        "run {} was stopped in its check and its {} commit(s) stayed in {}; fetched onto {}",
                        u.run, u.commits, u.clone, u.branch
                    ));
                    u.clone.clear();
                    crate::unchecked::record(&*c, &s.now(), &job.str("id"), &u);
                }
                Err(why) => {
                    // A runner error: the job is queued again until its
                    // attempts run out, and the work stays waiting (an
                    // error does not judge it); the clone is not touched.
                    status = "error".into();
                    summary = format!(
                        "the {} commit(s) run {} left unchecked in {} could not be fetched onto {} ({}); the clone is kept: git -C {} fetch {} +HEAD:refs/heads/{}",
                        u.commits,
                        u.run,
                        u.clone,
                        u.branch,
                        crate::util::trunc(&why, 200),
                        crate::util::shell_quote(&root),
                        crate::util::shell_quote(&u.clone),
                        u.branch
                    );
                    log.put(&summary);
                    stranded_clone_kept = true;
                    return Ok(());
                }
            }
        }
        let mut base = ex.prepare(&cancel)?;
        // An earlier run of the job was stopped in its check with its
        // worker's commits kept on a branch: this run checks them and lands
        // them, rather than doing the task again (unchecked.rs).
        if resume.is_none() && !ex.remote() && !o.dry_run {
            unchecked = crate::unchecked::pending(s, &job.str("id"), &root).and_then(|u| {
                match crate::unchecked::check_out(&clone, &root, &u) {
                    Ok(()) => {
                        log.add(format!(
                            "run {} was stopped in its check with {} commit(s) of {} kept on {}: this run checks and lands them; no worker starts",
                            u.run, u.commits, u.agent, u.branch
                        ));
                        base = u.base.clone();
                        // The task changed after the work was kept: the
                        // commits predate the amendment, and the run says so.
                        for amended in crate::unchecked::amended_since_kept(
                            s,
                            &job.str("id"),
                            &job.str("source"),
                        ) {
                            log.add(format!(
                                "the task was amended after this work was kept ({amended}); the kept commits predate it - re-read `hive task show {}`",
                                crate::compute::task_of_source(&job.str("source"))
                            ));
                        }
                        Some(u)
                    }
                    Err(why) => {
                        log.add(format!(
                            "the work run {} left unchecked on {} is not taken ({why}); this run starts over",
                            u.run, u.branch
                        ));
                        let _ = crate::util::git(&clone, &["reset", "-q", "--hard", &base]);
                        None
                    }
                }
            });
        }
        // Record the base sha now so every exit path (red base, verify failure, cancel)
        // leaves `runs.base` set; retries and the hub compare bases against it.
        crate::util::exec1(
            &c,
            "UPDATE runs SET base = ? WHERE id = ?",
            &[&base, &run_id],
        );
        let PreparedSetup {
            recipe,
            found,
            source: recipe_source,
            setup_shown,
            pins,
        } = prepare_setup(
            s,
            &project,
            &root,
            &base,
            &clone,
            &*ex,
            &cancel,
            o.dry_run,
            crate::setup_repair::is_repair_job(&job),
            &mut log,
        )?;
        // Vault secrets open on the brain only until hands hold device keys
        // (ADR-070 step 2): refuse here and record it; the job keeps its
        // attempt and the unfit mark sends it to the brain's own executor.
        if ex.remote()
            && let Some(r) = recipe.as_ref().filter(|r| !r.secrets.is_empty())
        {
            let why = crate::secrets::refuse_remote(
                s,
                r,
                &crate::secrets::GrantFor {
                    job: &job.str("id"),
                    run: &run_id,
                    project: &project.str("slug"),
                    machine: &ex.name(),
                    agent,
                },
            );
            return Err(crate::executor::ExecutorUnfit {
                msg: format!(
                    "{} declares vault secrets ({}): {why}",
                    crate::recipe::RECIPE_PATH,
                    r.secrets.join(", ")
                ),
                tool: crate::recipe::RECIPE_UNFIT_SECRETS.to_string(),
                project: project.str("slug"),
                detail: String::new(),
            }
            .into());
        }
        // A project without a verify command takes the recipe's
        // `suggested_check` here, so this run is already checked by it.
        if let Some(r) = &recipe
            && let Some(cmd) =
                crate::recipe::adopt_suggested_check(s, &project, r, recipe_source.as_ref(), now)
        {
            log.add(format!(
                "verify command adopted from {}: {cmd}",
                crate::recipe::RECIPE_PATH
            ));
            project = crate::board::project_by_slug(&c, job.str("project"));
        }
        let mut ballot = Vec::new();
        for v in crate::tally::ballot_items_for(
            s,
            project.str("slug"),
            agent.to_string(),
            now.to_string(),
            run_id.clone(),
        ) {
            ballot.push(row_to_ballot(v));
        }
        if job.str("kind") == "ballot" {
            for item in &mut ballot {
                let p = crate::board::post_row(&c, item.id.clone());
                if !is_nil(&p) {
                    item.body = p.str("body");
                }
            }
        }
        run_ballot = ballot.clone();
        // The job's artifacts folder outlives the clone: the hub shows what
        // the worker leaves there (screenshots, reports) beside the job.
        let artifacts_dir = if ex.remote() {
            None
        } else {
            crate::ui_evidence::prepare_dir(&home, &job.str("id"))
        };
        let brief_text = crate::brief::write_brief(crate::brief::BriefOpts {
            job: job.clone(),
            project: project.clone(),
            agent: agent.to_string(),
            clone: clone.clone(),
            minutes,
            ballot: ballot.clone(),
            has_ball: !ballot.is_empty(),
            remote: ex.remote(),
            setup: setup_shown,
            recipe: recipe.clone(),
            found,
            lessons: crate::lessons::recent_failure_lessons(
                s,
                project.str("slug"),
                now.to_string(),
            ),
            store: Some(s.clone()),
            now: now.to_string(),
            artifacts_dir: artifacts_dir
                .as_ref()
                .map(|d| d.display().to_string())
                .unwrap_or_default(),
        })?;
        crate::util::exec1(
            &c,
            "INSERT OR REPLACE INTO briefs (run_id, job_id, agent, text, at) VALUES (?, ?, ?, ?, ?)",
            &[&run_id, &job.str("id"), &agent, &brief_text, &now],
        );
        if o.dry_run {
            status = "dry-run".to_string();
            exit_code = Some(0);
            summary = format!(
                "dry run: clone prepared at {} on {}, brief written, no worker started",
                clone,
                ex.name()
            );
            log.put(&summary);
            return Ok(());
        }
        let (mut cmd, custom_worker) = match &resume {
            // The carried conversation already holds the brief; the worker
            // opens with what changed since and resumes its trajectory.
            Some(r) => {
                let (cmd, custom) = crate::exec::worker_command_as(
                    s,
                    &job.str("kind"),
                    &model_class,
                    &continuation_message(&run_id, r, &clone),
                    &clone,
                    &o.worker_cmd,
                    ex.remote(),
                    agent_spec.as_ref(),
                );
                (crate::exec::with_resume(cmd, &r.session.trajectory), custom)
            }
            None => crate::exec::worker_command_as(
                s,
                &job.str("kind"),
                &model_class,
                &brief_text,
                &clone,
                &o.worker_cmd,
                ex.remote(),
                agent_spec.as_ref(),
            ),
        };
        // The brief goes to the worker in a file, never on its command
        // line: another agent's `pkill -f <pattern>` matched workers by the
        // words of their briefs and stopped them all (TASK-151).
        if !custom_worker {
            let opening = match &resume {
                Some(r) => continuation_message(&run_id, r, &clone),
                None => brief_text.clone(),
            };
            cmd = crate::exec::brief_off_argv(&*ex, &run_id, &opening, cmd)?;
        }
        // The owner's work job starts past its project's narrowed lanes and
        // share (jobs.rs lease_job), up to the lock's bound, so on this
        // machine its gated commit may wait for every other check in the
        // project before its own - those of the runs that start after it
        // too, since its cap is fixed now: the harness command gets a
        // commit cap that covers the bound, the first run included. A
        // custom worker's command is its own, and a hand's checks take
        // turns on the hand.
        if !custom_worker
            && !ex.remote()
            && crate::jobs::is_owner_driven(&job)
            && crate::jobs::WORK_KINDS.contains(&job.str("kind").as_str())
            && let Some(cap) = crate::jobs::owner_commit_cap(s, &job.str("project"))
        {
            let lanes = crate::jobs::project_lanes(s, &job.str("project"));
            let checks = crate::jobs::owner_work_bound(s).unwrap_or(1);
            log.add(format!(
                "commit cap widened to {} min: up to {checks} checks share the verify lock on {} (verify ~{} min each)",
                crate::jobs::minutes_up(cap),
                job.str("project"),
                crate::jobs::minutes_up(lanes.verify_seconds.unwrap_or(0))
            ));
            cmd = crate::exec::with_commit_cap(cmd, cap);
        }
        if let Some(r) = &resume {
            log.add(format!(
                "carries on held harness session {} ({} steps) from run {}",
                r.session.id, r.session.steps, r.session.run_id
            ));
            if !s
                .mark_harness_session_resumed(&r.session.id, &format!("carried on by run {run_id}"))
                .unwrap_or(false)
            {
                log.add(format!(
                    "held session {} was already settled by another run",
                    r.session.id
                ));
            }
        }
        let mut env = BTreeMap::from([
            ("HIVE_JOB".to_string(), job.str("id")),
            ("HIVE_BRIEF".to_string(), run_id.clone()),
            ("HIVE_CLONE".to_string(), clone.clone()),
            ("HIVE_AGENT".to_string(), agent.to_string()),
            ("HIVE_SESSION".to_string(), run_id.clone()),
            ("HIVE_PROJECT".to_string(), project.str("slug")),
            ("GIT_TERMINAL_PROMPT".to_string(), "0".to_string()),
            (
                "HIVE_GENERATION".to_string(),
                job.int("generation").to_string(),
            ),
            ("HIVE_EXECUTOR".to_string(), ex.name()),
        ]);
        if let Some(dir) = &artifacts_dir {
            env.insert(
                hive_harness::artifacts::ARTIFACTS_ENV.to_string(),
                dir.display().to_string(),
            );
        }
        if ex.remote()
            && !custom_worker
            && let Some(dir) = ex.worker_dirs().first()
        {
            env.insert(
                "HIVE_HARNESS_STEPS".into(),
                Path::new(dir)
                    .join(format!("harness-steps-{run_id}.jsonl"))
                    .display()
                    .to_string(),
            );
        }
        if !ex.remote() {
            worker_cli_env(&mut env, &crate::exec::local_cli(&home));
            env.insert("HIVE_HOME".into(), home.clone());
            if let Some(r) = &recipe {
                for (k, v) in &r.env {
                    env.insert(k.clone(), v.clone());
                }
            }
            // The run's own TMPDIR beside the clone (work/<job>.tmp); a
            // recipe that sets one itself wins, like CARGO_TARGET_DIR.
            let run_tmp = ex.run_tmp_dir();
            if !run_tmp.is_empty() {
                env.entry("TMPDIR".into()).or_insert(run_tmp);
            }
            if !o.dry_run {
                let (prepared, lane) = crate::buildcache::prepare(
                    s.setting("daemon.build_cache", "on") != "off",
                    Path::new(&home),
                    &project.str("slug"),
                    Path::new(&clone),
                    &mut env,
                );
                let line = prepared.line();
                if !line.is_empty() {
                    log.add(line);
                }
                build_cache = lane;
            }
            if !pins.is_empty() {
                for kv in crate::recipe::mise_env(&clone) {
                    env.insert(kv[0].clone(), kv[1].clone());
                }
                let mut prefix = crate::recipe::mise_exec_argv(&pins);
                prefix.extend(cmd);
                cmd = prefix;
            }
        } else {
            // prepare_setup let a recipe with tools or env through only because
            // these hands take them: the worker gets what the hub's gets.
            if let Some(r) = &recipe {
                for (k, v) in &r.env {
                    env.insert(k.clone(), v.clone());
                }
            }
            // Each run on a hand builds into its own target dir, as a local
            // run takes a lane of its own (`buildcache::prepare_hand`).
            if !o.dry_run && !custom_worker {
                let prepared = crate::buildcache::prepare_hand(
                    crate::buildcache::is_cargo_project(Path::new(&root)),
                    ex.worker_dirs().first().map_or("", String::as_str),
                    &mut env,
                );
                let line = prepared.line();
                if !line.is_empty() {
                    log.add(line);
                }
                if let crate::buildcache::Prepared::HandRun(dir) = prepared {
                    hand_target_dir = dir;
                }
            }
            // A local hive-hand runs on this machine: its worker uses this
            // hive, unless the recipe names one.
            let cli = if ex.pushes()
                && recipe
                    .as_ref()
                    .is_none_or(|r| crate::recipe::recipe_hive_cli(r).is_empty())
            {
                crate::exec::local_cli(&home)
            } else {
                remote_hive_cli(recipe.as_ref())
            };
            env.insert("HIVE_CLI".into(), cli);
            if !pins.is_empty() {
                for kv in crate::recipe::mise_env(&clone) {
                    env.insert(kv[0].clone(), kv[1].clone());
                }
                let mut prefix = crate::recipe::mise_exec_argv(&pins);
                prefix.extend(cmd);
                cmd = prefix;
            }
        }
        if !custom_worker && !project.str("verify_cmd").is_empty() {
            // The hook applies the recipe's exports and pins itself, so it
            // is given the bare check inside the lock, touch included, and
            // the memo key beside it.
            let hook_check = crate::verify_lock::wrap(
                s,
                &project.str("slug"),
                &check_with_freshen(&project, recipe.as_ref(), &clone, "the clone's"),
            );
            ex.install_pre_commit_hook(
                &hook_check,
                &verify_memo_key(s, &project),
                recipe.as_ref(),
                &pins,
                &home,
                &project.str("slug"),
                agent,
                &base,
            )?;
        }
        let shown = cmd
            .iter()
            .map(|x| {
                if x == &brief_text || (resume.is_some() && x.starts_with("The runner restarted")) {
                    "<brief>".to_string()
                } else {
                    crate::util::shell_quote(x)
                }
            })
            .collect::<Vec<_>>();
        if !ex.remote() {
            let found_cli = crate::preflight::hive_on_path(&crate::proc::env_with(
                &crate::proc::environ(),
                &env,
            ));
            log.add(crate::preflight::preflight_line(
                found_cli.clone(),
                env.get("HIVE_CLI").cloned().unwrap_or_default(),
            ));
            crate::preflight::preflight_hive_cli(
                found_cli,
                env.get("HIVE_CLI").cloned().unwrap_or_default(),
            )?;
        }
        log.add(format!("$ {}", shown.join(" ")));
        // Team vault secrets (ADR-070 rule 4): granted for this run and
        // opened in memory on the brain; only the worker's env carries the
        // values, wiped when it ends, and its output is redacted before any
        // of it reaches the log, the result or an event.
        if !ex.remote() && !o.dry_run {
            let (grant, lines) = crate::secrets::grant_local(
                s,
                recipe.as_ref(),
                &job.str("brief"),
                &crate::secrets::GrantFor {
                    job: &job.str("id"),
                    run: &run_id,
                    project: &project.str("slug"),
                    machine: &ex.name(),
                    agent,
                },
            );
            for line in lines {
                log.add(line);
            }
            secret_grant = grant;
        }
        let launched = wall_clock();
        let mut out = if let Some(u) = &unchecked {
            // The worker finished in the stopped run; its report stands.
            crate::proc::Outcome {
                timeout: false,
                exit_code: 0,
                stdout: u.report.clone(),
                stderr: String::new(),
            }
        } else if secret_grant.is_empty() {
            ex.run_worker(&cancel, &cmd, &env, minutes)?
        } else {
            let worker_env = secret_grant.worker_env(&env);
            ex.run_worker(&cancel, &cmd, &worker_env, minutes)?
        };
        out.stdout = secret_grant.redact(&out.stdout);
        out.stderr = secret_grant.redact(&out.stderr);
        if !out.timeout {
            exit_code = Some(out.exit_code);
        }
        log.add(tail(&out.stdout, 20000));
        log.add(tail(&out.stderr, 20000));
        // The worker finished the job itself (`hive job finish`) under this
        // run's lease before the worker command ended: the job's status is
        // its verdict, whatever stopped the harness afterwards.
        let finished_by_worker = |c: &rusqlite::Connection| {
            let cur = crate::jobs::job_row(c, job.str("id"));
            !is_nil(&cur)
                && cur.int("generation") == job.int("generation")
                && cur.str("lease_agent") == agent
                && ["done", "failed", "cancelled"].contains(&cur.str("status").as_str())
        };
        // A held exit that came after the finish is a stop notice on a job
        // that is already done, whichever worker gave it: the finish stands,
        // the session is settled, and the run is judged below like any
        // other. Taken as a hold, the job was requeued over its `done`
        // (2026-11-05: an owner's task finished at 06:13:37Z, verified, was
        // requeued at 06:13:50Z by a restart's hold and its 161-step work
        // was lost to a fresh clone).
        let mut finished_before_hold = false;
        if !out.timeout
            && out.exit_code == hive_store::HARNESS_HELD_EXIT_CODE
            && finished_by_worker(&c)
        {
            finished_before_hold = true;
            if let Some(h) = s
                .held_harness_session_for_job(&job.str("id"))
                .ok()
                .flatten()
                .filter(|h| h.run_id == run_id)
            {
                let _ = s.mark_harness_session_resumed(
                    &h.id,
                    "not carried on: the worker had finished the job before the hold; nothing is left to do",
                );
            }
            log.add(format!(
                "the harness exited {} (held) after the worker had finished job {}; the finish stands and the hold is not taken",
                out.exit_code,
                job.str("id")
            ));
        }
        if !out.timeout
            && out.exit_code == hive_store::HARNESS_HELD_EXIT_CODE
            && !custom_worker
            && !finished_before_hold
        {
            // The harness held for the runner's restart: its trajectory is
            // written and its session is `held`. Nothing is judged; the job
            // goes back to the queue with its clone and the next run carries
            // on (resumable_held_run). The harness's own report line is a
            // stop notice, not a result, so it is not parsed.
            status = "held".into();
            let held = s
                .held_harness_session_for_job(&job.str("id"))
                .ok()
                .flatten()
                .filter(|h| h.run_id == run_id);
            let why = if crate::yield_lane::yielded(s, &job.str("id"), &run_id) {
                "held so the owner's work could start first"
            } else {
                "held for the runner's restart"
            };
            summary = match &held {
                Some(h) => format!(
                    "{why} after {} steps; the next run carries on from harness session {}",
                    h.steps, h.id
                ),
                None => {
                    format!("{why}; no harness session to carry on, so the next run starts over")
                }
            };
            log.put(&summary);
            return Ok(());
        }
        // Ended from outside (TASK-151 item 3): a SIGTERM or SIGKILL with no
        // hold, cancel or timeout on record is not the worker's doing, so the
        // run is not judged. The job goes back to the queue without using an
        // attempt, the clone stays, the commits are kept on the job's
        // `-failed` branch and as a patch the next brief starts from, and the
        // daemon posts one finding for the runs that ended together. A
        // worker that had already finished the job keeps its finish.
        if let Some(signal) = crate::signal_stop::outside_signal(out.timeout, out.exit_code)
            && !finished_by_worker(&c)
            && !owner_cancelled.load(Ordering::SeqCst)
            && !cancel.is_cancelled()
        {
            let on_record = crate::signal_stop::stop_on_record(s, &job.str("id"), &run_id);
            let earlier = crate::signal_stop::earlier_requeues(&*c, &job.str("id"));
            if !on_record.is_empty() {
                log.add(format!(
                    "the worker exited {} ({signal}), but {on_record}; judged as it stands",
                    out.exit_code
                ));
            } else if earlier >= crate::signal_stop::SIGNAL_REQUEUE_LIMIT {
                log.add(format!(
                    "the worker exited {} ({signal}) from outside, and this job was requeued {earlier} times that way already; judged as it stands",
                    out.exit_code
                ));
            } else {
                let mut stop = crate::signal_stop::Signalled {
                    run: run_id.clone(),
                    job: job.str("id"),
                    project: project.str("slug"),
                    agent: agent.to_string(),
                    executor: ex.name(),
                    exit: out.exit_code,
                    signal: signal.into(),
                    commits: ex.commit_count(&base),
                    ..Default::default()
                };
                // How to get commits the fetch could not bring back from the
                // machine; the clone there stays for it (as after a failed
                // check, below).
                let mut how = String::new();
                if stop.commits > 0 && !root.is_empty() {
                    let kept_branch = crate::land::failed_work_branch(agent, &job.str("id"));
                    let mut fetch_failed = false;
                    let kept = crate::land::keep_failed_work(
                        &root,
                        |dst| {
                            let ok = matches!(
                                fetch_run_work(s, &cancel, ex.as_ref(), &job.str("id"), &root, dst, &log),
                                Ok(o) if o.exit_code == 0
                            );
                            fetch_failed = !ok;
                            ok
                        },
                        &base,
                        &kept_branch,
                        &home,
                        &project.str("slug"),
                    );
                    if !kept.is_empty() {
                        stop.branch = kept_branch;
                        if let Some((_, patch)) = kept.split_once(" and in ") {
                            stop.patch = patch.trim().to_string();
                        }
                    } else if fetch_failed {
                        // The commits are only on the machine: its clone
                        // is not cleaned, and the machine is marked.
                        how = ex.fetch_by_hand(&root, &kept_branch);
                        if !how.is_empty() {
                            work_not_fetched = true;
                        }
                    }
                }
                commits = stop.commits;
                status = "held".into();
                summary = stop.summary();
                if !how.is_empty() {
                    summary.push_str(&format!("; {how}"));
                }
                stop.record(s, &s.now(), "runner");
                log.put(&summary);
                return Ok(());
            }
        }
        let result_raw = match &unchecked {
            Some(u) => Some(u.report.clone()),
            None => ex.result_text().map(|t| secret_grant.redact(&t)),
        };
        parsed = crate::brief::parse_result_text(result_raw.clone());
        let current_job = crate::jobs::job_row(&*c, job.str("id"));
        if !is_nil(&current_job)
            && current_job.int("generation") == job.int("generation")
            && current_job.str("lease_agent") == agent
            && ["done", "failed", "cancelled"].contains(&current_job.str("status").as_str())
        {
            worker_reported = current_job.str("status");
            let result = current_job.str("result");
            let mut stored = crate::brief::parse_result_text(Some(result.clone()));
            if parsed.summary.is_empty() {
                parsed.summary = if !stored.reported() {
                    result.trim().to_string()
                } else {
                    stored.summary
                };
            }
            parsed.found = true;
            for q in stored.questions {
                if !parsed.questions.contains(&q) {
                    parsed.questions.push(q);
                }
            }
            log.add(format!(
                "the worker finished job {} itself ({}): {}",
                job.str("id"),
                current_job.str("status"),
                current_job.str("result")
            ));
        }
        // The worker's outcome code is read before any of its words; a
        // summary without one falls back to the text. Which path classified
        // the run goes in the log and on the run row (runs.outcome_source),
        // so the fallback rate can be measured.
        outcome = parsed.outcome();
        log.add(format!(
            "outcome: {} (from the {})",
            if outcome.code.is_empty() {
                "none"
            } else {
                outcome.code.as_str()
            },
            match outcome.source {
                "code" => "worker's SUMMARY outcome code",
                "text" => "summary text: no outcome code, regex fallback",
                _ => "absence of a summary",
            }
        ));
        // The step budget stopped the worker with work in the clone: the
        // run is not judged on a task cut off mid-way. Like one the runner's
        // restart held, it holds, and the next run of the job - the same
        // agent, the same clone, the conversation carried on - gets a fresh
        // budget to finish. Once per job: a second stop is judged and
        // recovered as before. A custom worker or a remote executor keeps
        // no session or clone to carry on, so it is judged as it stands.
        if step_budget_stop(out.timeout, out.exit_code, &parsed.summary)
            && !custom_worker
            && !ex.remote()
            && worker_reported.is_empty()
            && !owner_cancelled.load(Ordering::SeqCst)
            && !step_budget_carried_on(&*c, &job.str("id"))
        {
            let made = ex.commit_count(&base);
            let left = ex.dirty();
            let new = ex.untracked();
            if made > 0 || !left.trim().is_empty() || !new.trim().is_empty() {
                match s.hold_step_limit_harness_session(
                    &run_id,
                    &job.str("id"),
                    STEP_BUDGET_HELD_REASON,
                ) {
                    Ok(Some(h)) => {
                        let mut kept = vec![format!("{made} commit(s)")];
                        if !left.trim().is_empty() {
                            kept.push("uncommitted edits".into());
                        }
                        if !new.trim().is_empty() {
                            kept.push("new files".into());
                        }
                        status = "held".into();
                        summary = format!(
                            "{STEP_BUDGET_HELD_SUMMARY} after {} steps with {} in the clone; the next run carries on from harness session {} with a fresh budget",
                            h.steps,
                            kept.join(", "),
                            h.id
                        );
                        log.put(&summary);
                        return Ok(());
                    }
                    Ok(None) => log.add(
                        "stopped at the step budget with work in the clone, but no harness session with a trajectory to carry on; judged as it stands",
                    ),
                    Err(e) => log.add(format!(
                        "stopped at the step budget with work in the clone; the hold failed ({}); judged as it stands",
                        err_message(&e)
                    )),
                }
            } else {
                log.add("stopped at the step budget with nothing in the clone: no hold, judged as it stands");
            }
        }
        let lease_ok = |mins: i32| -> bool {
            if !worker_reported.is_empty() {
                let j = crate::jobs::job_row(&*c, job.str("id"));
                !is_nil(&j)
                    && j.int("generation") == job.int("generation")
                    && j.str("lease_agent") == agent
            } else {
                // Renew from the store's clock at the call, not the run's
                // start (`now`): a run longer than the lease renewed from its
                // start set lease_until in the past, and the next lease_job
                // anywhere requeued the job under the running run (TASK-179).
                crate::jobs::renew_lease(
                    &*c,
                    s.now(),
                    job.str("id"),
                    agent.to_string(),
                    job.int("generation"),
                    mins,
                )
            }
        };
        let mut rescued = false;
        if out.timeout {
            if owner_cancelled.load(Ordering::SeqCst) {
                status = "failed".into();
                summary = "cancelled by the owner".into();
                exit_code = Some(5);
                log.add(format!("timeout after {minutes} minutes"));
            } else if stopped_by_runner(&cancel) {
                // The runner stopped the worker before its cap (a restart
                // the owner asked for now, a stall, the stop grace, a hand
                // that stopped answering) - a deadline would read
                // DeadlineExceeded. The run is lost like one the runner died
                // under, and the job goes back to the queue instead of
                // failing on work it never got to finish.
                status = "lost".into();
                summary = hand_watch
                    .why()
                    .unwrap_or_else(|| LOST_RUN_SUMMARY.to_string());
                exit_code = None;
                log.add("stopped by the runner before the cap; the job returns to the queue");
            } else {
                status = "timeout".into();
                exit_code = None;
                log.add(format!("timeout after {minutes} minutes"));
            }
            if status == "timeout" && ex.commit_count(&base) > 0 {
                rescued = true;
                exit_code = Some(0);
                if !parsed.found {
                    parsed.found = true;
                    parsed.summary = format!(
                        "hit the {minutes}-minute cap before reporting; the commits it made are kept only because the check passed - the work may be partial"
                    );
                } else {
                    let sm = if parsed.summary.is_empty() {
                        "no summary".to_string()
                    } else {
                        parsed.summary.clone()
                    };
                    parsed.summary = format!("hit the {minutes}-minute cap after committing: {sm}");
                }
                log.add("commits found after the timeout: judged like a finished run");
            }
        }
        // A run the runner stopped (`lost`) is not judged either: nothing it
        // did is held against the job, which is done again.
        if (status != "timeout" && status != "lost") || rescued {
            commits = ex.commit_count(&base);
            // A review's clone stands on the pull request's head once the
            // agent ran `hive review checkout`: the commits past the base
            // are the author's, signed by the author's run key, and the
            // identity check refused them as this run's (eighteen review
            // runs failed that way on 2026-11-04 after reviews that had
            // merged their pull request). A review ends in its verdict
            // through `hive review`; nothing of the clone lands.
            if commits > 0 && crate::review::is_review_job(&job) {
                log.add(format!(
                    "{commits} commit(s) in the clone are the pull request's, checked out for the review; a review lands nothing of its own"
                ));
                commits = 0;
            }
            if commits > 0 {
                changed_files = ex.changed_files(&base);
            }
            let dirty = ex.dirty();
            // The worker finished its change and ran `git commit`, and the
            // harness killed the commit at the step deadline while the
            // pre-commit verification ran: the change is still in the tree.
            // The runner commits it and lets the run's own verification
            // judge it, instead of discarding finished work.
            let mut rescued_commit = false;
            // A worker that gave up for another reason keeps its own verdict;
            // one that reported the failed commit is the case this is for.
            let worker_gave_up_on_commit =
                !parsed.incomplete() || parsed.summary.to_lowercase().contains("commit");
            if commits == 0
                && !dirty.is_empty()
                && exit_code == Some(0)
                && !owner_cancelled.load(Ordering::SeqCst)
                && (worker_reported.is_empty() || worker_reported == "done")
                && worker_gave_up_on_commit
                && let Some(killed) = killed_commit_command(&c, &run_id, &job.str("id"), now)
            {
                let title = commit_message_from_command(&killed)
                    .unwrap_or_else(|| job.str("title").trim().to_string());
                let message = format!(
                    "{title}\n\nThe worker's `git commit` was killed at the harness step deadline while the pre-commit verification ran; the runner committed the work it left in the tree so the run's own verification could judge it instead of discarding it (run {run_id}).",
                );
                match ex.commit_left_work(&message) {
                    Ok(()) => {
                        commits = ex.commit_count(&base);
                        rescued_commit = commits > 0;
                    }
                    Err(e) => log.add(format!(
                        "the worker's `git commit` was killed at the step deadline and the runner could not commit its work either: {e}"
                    )),
                }
            }
            if rescued_commit {
                log.add(format!(
                    "the worker's `git commit` was killed at the step deadline; the runner committed its work for verification:\n{dirty}"
                ));
                let worker_said = parsed.summary.trim().to_string();
                parsed.found = true;
                parsed.summary = format!(
                    "the worker's `git commit` was killed at the step deadline; the runner committed its work for verification{}",
                    if worker_said.is_empty() {
                        String::new()
                    } else {
                        format!("; worker report: {worker_said}")
                    }
                );
            } else if !dirty.is_empty() {
                log.add(format!(
                    "uncommitted changes left in the clone were discarded:\n{dirty}"
                ));
            }
            let elapsed = wall_clock()
                .duration_since(launched)
                .unwrap_or_default()
                .as_secs();
            // The held exit after a finish is a stop notice, not a verdict:
            // the worker's own finish (worker_reported) judges the run.
            let code = if finished_before_hold {
                0
            } else {
                exit_code.unwrap_or(-1)
            };
            if code == 5 || owner_cancelled.load(Ordering::SeqCst) || worker_reported == "cancelled"
            {
                status = "failed".into();
                summary = "cancelled by the owner".into();
                if !parsed.summary.is_empty() {
                    summary = if crate::brief::plain_cancel_summary(parsed.summary.clone())
                        && !owner_cancelled.load(Ordering::SeqCst)
                        && worker_reported != "cancelled"
                    {
                        parsed.summary.clone()
                    } else {
                        format!("cancelled by the owner: {}", parsed.summary)
                    };
                }
            } else if code != 0
                && !parsed.reported()
                && worker_reported.is_empty()
                && commits == 0
                && ((elapsed as i32) < LAUNCH_SECONDS
                    || (code == 4
                        && crate::executor::login_fault(
                            &(out.stderr.clone() + "\n" + &out.stdout),
                        )))
            {
                status = "error".into();
                let src = if out.stderr.is_empty() {
                    out.stdout.clone()
                } else {
                    out.stderr.clone()
                };
                summary = format!(
                    "worker rejected at launch (exit {code} after {}s): {}",
                    elapsed,
                    squeeze(&src, 300)
                );
                if crate::executor::login_fault(&squeeze(&src, 300)) {
                    if ex.remote() {
                        fault = true;
                        summary.push_str(&format!(" - the login on {}, not the job; run `hive executor login {} --admin <admin@host> --with-token`", ex.name(), ex.name()));
                    } else {
                        fault = true;
                        summary.push_str(" - the provider login on this machine, not the job; sign in with GitHub in the console (Settings -> Harness) or run `hive harness login`");
                    }
                }
            } else if code != 0 {
                status = "failed".into();
                summary = if parsed.summary.is_empty() {
                    format!("worker exited {code}")
                } else {
                    format!("worker exited {code}: {}", parsed.summary)
                };
            } else if worker_reported == "failed" {
                status = "failed".into();
                summary = if parsed.summary.is_empty() {
                    "the worker reported failure".into()
                } else {
                    parsed.summary.clone()
                };
            } else if parsed.incomplete() {
                status = "failed".into();
                summary = parsed.summary.clone();
            } else if !parsed.reported() && worker_reported.is_empty() {
                status = "failed".into();
                summary = "worker wrote no report (no `hive job finish`, no SUMMARY line); nothing to land".into();
            } else if commits == 0 {
                status = "done".into();
                summary = if parsed.summary.is_empty() {
                    "no commits; nothing to land".into()
                } else {
                    parsed.summary.clone()
                };
            } else if !lease_ok(minutes + 5) {
                status = "stale".into();
                summary = format!(
                    "lease generation {} is no longer current; {} commit(s) discarded",
                    job.int("generation"),
                    commits
                );
            } else if !ex.owner_paths_touched(&base).is_empty() {
                status = "failed".into();
                summary = format!(
                    "{} commit(s) touch owner-owned paths ({}); nothing landed - propose the change instead",
                    commits,
                    ex.owner_paths_touched(&base).join(", ")
                );
            } else if let Some(why) =
                crate::card_proof::card_unproven(s, &job, &base, &log, &|cmd: &str| {
                    ex.verify(&cancel, cmd, minutes)
                })
            {
                if stopped_by_runner(&cancel) {
                    // The runner stopped the card's proof (a forced
                    // restart): the check was cut, not red, so the work is
                    // kept for the job's next run to prove and check, as a
                    // stopped end check is (TASK-152 item 6).
                    status = "lost".into();
                    summary = format!(
                        "{LOST_RUN_SUMMARY} (during the check; {commits} commit(s) not judged, nothing landed)"
                    );
                    exit_code = None;
                    log.add(format!(
                        "the card's proof was stopped by the runner ({why}); the job returns to the queue"
                    ));
                    check_stopped = Some(base.clone());
                    worker_report = crate::unchecked::report_lines(
                        result_raw.as_deref().unwrap_or_default(),
                        &parsed.summary,
                    );
                } else {
                    // A job from a voted card lands only on its card's
                    // proof: the check red on the default branch, green
                    // with the change.
                    status = "failed".into();
                    summary = format!("{commits} commit(s) but {why}; nothing landed");
                }
            } else {
                let mut ok = true;
                let mut check_tool = String::new();
                let mut base_check_cmd = String::new();
                let mut verify_rc: Option<i32> = None;
                if let Some((tree, memo)) = (!project.str("verify_cmd").is_empty())
                    .then(|| hook_verified_tree(s, &project, &*ex, &clone))
                    .flatten()
                {
                    verify_rc = Some(0);
                    crate::jobs::record_check_ran(
                        s,
                        &project.str("slug"),
                        &job.str("id"),
                        memo.seconds,
                        &memo.by,
                    );
                    verify_note = crate::verify_memo::note(&memo, &tree);
                    log.add(format!(
                        "$ {}\nhive: verify skipped: tree {tree} already passed this check on this store ({verify_note})",
                        project.str("verify_cmd")
                    ));
                } else if !project.str("verify_cmd").is_empty() {
                    let mut check_cmd = wrapped_check(s, &project, recipe.as_ref(), &pins, &clone);
                    if !hand_target_dir.is_empty() {
                        // The hand's verify verb passes no env: the check
                        // builds where the run's worker and gate did.
                        check_cmd = format!(
                            "export CARGO_TARGET_DIR={}\n{check_cmd}",
                            crate::util::shell_quote(&hand_target_dir)
                        );
                    }
                    base_check_cmd = check_cmd.clone();
                    let (mut rc, mut vout) = ex.verify(&cancel, &check_cmd, minutes);
                    if rc == Some(crate::verify_lock::DISK_FULL_EXIT) && ex.name() == "local" {
                        // Leftover clones from finished or lost jobs are the
                        // usual reason the host is under min_free_mb: sweep
                        // the runner's own work dir once and retry before
                        // calling the disk full.
                        let (freed, paths) =
                            crate::sweep::sweep_stale_clones(s, &job.str("id"), s.now());
                        if paths.is_empty() {
                            log.add(
                                "verify refused for lack of disk room; no stale clone under work/ to sweep"
                                    .to_string(),
                            );
                        } else {
                            log.add(format!(
                                "verify refused for lack of disk room; swept {} stale clone item(s), {} MB freed; retrying once",
                                paths.len(),
                                freed / (1024 * 1024)
                            ));
                            let (rc2, vout2) = ex.verify(&cancel, &check_cmd, minutes);
                            rc = rc2;
                            vout = vout2;
                        }
                    }
                    verify_rc = rc;
                    // A check that passed on a paired or rented machine
                    // lifts a red mark on its class (class_red.rs) - on a
                    // tree with this job's commits, once a landing names
                    // the job (TASK-169 item 6).
                    if rc == Some(0) && ex.remote() {
                        crate::class_red::mark_green(
                            &c,
                            &s.now(),
                            &project.str("slug"),
                            &crate::executor::executor_row(s, &ex.name()),
                            &job.str("id"),
                            &run_id,
                            &base,
                            commits > 0,
                        );
                    }
                    if rc == Some(0) && !ex.remote() {
                        // Only this machine's checks take turns on its lock.
                        crate::jobs::record_check_ran(
                            s,
                            &project.str("slug"),
                            &job.str("id"),
                            crate::verify_lock::ran_seconds(&vout),
                            "run",
                        );
                    }
                    if rc == Some(0) && ex.dirty().is_empty() {
                        // The tree the check judged is the committed one, so
                        // a landing or a later base check on it can skip.
                        record_verified_tree(s, &project, &*ex, &clone, "HEAD", "run", agent);
                    }
                    if let Some(rc) = rc {
                        verify_note = crate::verify_lock::exit_note(rc);
                        ok = rc == 0;
                        if !ok {
                            check_tool = crate::recipe::check_tool_missing(
                                &project.str("verify_cmd"),
                                rc,
                                &vout,
                            );
                        }
                    } else {
                        verify_note = "verify timed out".into();
                        ok = false;
                    }
                    log.add(format!(
                        "$ {}\n{}",
                        project.str("verify_cmd"),
                        tail(&vout, 40000)
                    ));
                }
                if !check_tool.is_empty() {
                    status = "failed".into();
                    summary = format!(
                        "{commits} commit(s) but the check could not start on {}: `{}` is not installed there ({}); nothing landed",
                        ex.name(),
                        check_tool,
                        verify_note
                    );
                    unfit = Some(crate::executor::ExecutorUnfit {
                        msg: summary.clone(),
                        tool: check_tool,
                        project: project.str("slug"),
                        detail: verify_note.clone(),
                    });
                } else if verify_rc.is_none() && stopped_by_runner(&cancel) {
                    // The runner stopped the check before its cap (a restart
                    // the owner asked for now): the commits were not judged,
                    // so the run is lost and the job is done again, rather
                    // than failed on a check that never finished.
                    status = "lost".into();
                    summary = format!(
                        "{LOST_RUN_SUMMARY} (during the check; {commits} commit(s) not judged, nothing landed)"
                    );
                    exit_code = None;
                    log.add("the check was stopped by the runner before the cap; the job returns to the queue");
                    // The worker's work is finished: it is kept for the
                    // job's next run to check, not done again.
                    check_stopped = Some(base.clone());
                    worker_report = crate::unchecked::report_lines(
                        result_raw.as_deref().unwrap_or_default(),
                        &parsed.summary,
                    );
                } else if !ok {
                    status = "failed".into();
                    summary = format!("{commits} commit(s) but {verify_note}; nothing landed");
                    // The clone goes when the run ends: keep its commits so a
                    // retry can start from them instead of from the run record.
                    let kept_branch = crate::land::failed_work_branch(agent, &job.str("id"));
                    let mut fetch_failed = false;
                    // How to get commits that stayed on the machine; a red
                    // base verdict below rewrites the summary and keeps it.
                    let mut how_clause = String::new();
                    let kept = crate::land::keep_failed_work(
                        &root,
                        |dst| {
                            let ok = matches!(
                                fetch_run_work(s, &cancel, ex.as_ref(), &job.str("id"), &root, dst, &log),
                                Ok(o) if o.exit_code == 0
                            );
                            fetch_failed = !ok;
                            ok
                        },
                        &base,
                        &kept_branch,
                        &home,
                        &project.str("slug"),
                    );
                    if !kept.is_empty() {
                        log.add(format!("the failed run's work: {kept}"));
                        summary.push_str(&format!("; {kept}"));
                    } else if fetch_failed && !root.is_empty() {
                        // The commits are only on the machine: its clone
                        // stays, and the summary says how to get them.
                        let how = ex.fetch_by_hand(&root, &kept_branch);
                        if !how.is_empty() {
                            work_not_fetched = true;
                            how_clause = format!("its commits could not be fetched: {how}");
                            summary.push_str(&format!("; {how_clause}"));
                        }
                    }
                    // A check that did not run (the disk under the floor, the
                    // lock given up) says nothing about the base either; its
                    // prelude would stop the same way, after the same wait.
                    let verdict = if verify_rc.is_some_and(crate::verify_lock::did_not_run) {
                        log.add(
                            "the check did not run, so the base tree is not checked either; the base's state is unknown",
                        );
                        BaseVerdict::Unknown
                    } else {
                        base_tree_verdict(
                            s,
                            &project,
                            ex.as_ref(),
                            &cancel,
                            &clone,
                            &base,
                            &base_check_cmd,
                            minutes,
                            &log,
                            "the job's verify failed; was the base already red?",
                            agent,
                        )
                    };
                    if let BaseVerdict::Red { rc: base_rc, .. } = verdict {
                        // Red on the base on a paired or rented machine: the
                        // project's code jobs keep off its class until a
                        // check there passes (class_red.rs, TASK-144).
                        if ex.remote() {
                            crate::class_red::mark_red(
                                &c,
                                &s.now(),
                                &project.str("slug"),
                                &crate::executor::executor_row(s, &ex.name()),
                                &job.str("id"),
                                &run_id,
                                &base,
                            );
                            log.add(format!(
                                "the base is red on {} ({}): {}'s code jobs keep off that class of machine until a check there passes",
                                ex.name(),
                                crate::class_red::class_of(&crate::executor::executor_row(s, &ex.name())),
                                project.str("slug")
                            ));
                        }
                        summary = format!(
                            "{commits} commit(s) but {verify_note}; the base {} was already red before this job (verify exit {base_rc} on the base tree), so this is not a regression the job introduced; nothing landed",
                            crate::recipe::short(&base, 12)
                        );
                        if !kept.is_empty() {
                            summary.push_str(&format!("; {kept}"));
                        }
                        if !how_clause.is_empty() {
                            summary.push_str(&format!("; {how_clause}"));
                        }
                    }
                } else if !lease_ok(5) {
                    status = "stale".into();
                    summary = format!(
                        "lease generation {} is no longer current after verify; {} commit(s) discarded",
                        job.int("generation"),
                        commits
                    );
                } else {
                    // Kept work lands under the worker that made it: its
                    // commits carry that worker's name and run key.
                    let author = unchecked.as_ref().map_or(agent, |u| u.agent.as_str());
                    branch = job.str("branch");
                    if branch.is_empty() {
                        branch = format!("hive/{}/{}", author, job.str("id"));
                    }
                    if crate::util::git(
                        &root,
                        &[
                            "rev-parse",
                            "--verify",
                            "--quiet",
                            &format!("refs/heads/{branch}"),
                        ],
                    )
                    .ok()
                    {
                        status = "failed".into();
                        summary =
                            format!("branch {branch} already exists in {root}; nothing landed");
                        branch.clear();
                    } else {
                        let holding = format!("refs/hive/landing/{}", job.str("id"));
                        let fetched = fetch_run_work(
                            s,
                            &cancel,
                            ex.as_ref(),
                            &job.str("id"),
                            &root,
                            &holding,
                            &log,
                        )?;
                        if fetched.exit_code != 0 && stopped_by_runner(&cancel) {
                            // The runner stopped the landing (a forced
                            // restart): the checked work is kept below for
                            // the job's next run, not failed and removed.
                            status = "lost".into();
                            branch.clear();
                            summary = format!(
                                "{LOST_RUN_SUMMARY} (during the landing; {commits} commit(s) not landed)"
                            );
                            exit_code = None;
                            check_stopped = Some(base.clone());
                            worker_report = crate::unchecked::report_lines(
                                result_raw.as_deref().unwrap_or_default(),
                                &parsed.summary,
                            );
                        } else if fetched.exit_code != 0 {
                            status = "failed".into();
                            let how = ex.fetch_by_hand(&root, &branch);
                            branch.clear();
                            summary =
                                format!("could not land the branch: {}", fetched.stderr.trim());
                            if !how.is_empty() {
                                work_not_fetched = true;
                                summary.push_str(&format!("; {how}"));
                            }
                        } else {
                            let signers = work_dir
                                .join(format!("{}.signers", job.str("id")))
                                .display()
                                .to_string();
                            // Commits already on the default branch or on
                            // another of Hive's branches are not this
                            // run's to answer for: the worker built on a
                            // sibling's unmerged work, which the delivery
                            // stacks the pull request on.
                            let mut accepted = crate::keys::accepted_refs(
                                &root,
                                &crate::baseline::default_branch(project.clone()),
                                &holding,
                            );
                            // The kept branch holds the very commits under
                            // check: they answer to the identity check.
                            if let Some(u) = &unchecked {
                                let kept = format!("refs/heads/{}", u.branch);
                                accepted.retain(|r| *r != kept);
                            }
                            let signer = unchecked
                                .as_ref()
                                .map_or(pubkey.as_str(), |u| u.pubkey.as_str());
                            let problems = crate::keys::landing_problems(
                                &root,
                                &base,
                                &holding,
                                author,
                                &job.str("id"),
                                signer,
                                &signers,
                                &accepted,
                            );
                            if !problems.is_empty() {
                                status = "failed".into();
                                branch.clear();
                                summary = format!(
                                    "{commits} commit(s) refused: {}; nothing landed",
                                    problems
                                        .iter()
                                        .take(3)
                                        .cloned()
                                        .collect::<Vec<_>>()
                                        .join("; ")
                                );
                                log.add(format!(
                                    "identity check (ADR-022):\n{}",
                                    problems.join("\n")
                                ));
                                // The refused commits are the worker's work all
                                // the same (a lost trailer refuses as surely as a
                                // forged one): keep them as after a failed check,
                                // so a retry or the owner starts from them.
                                let kept = crate::land::keep_failed_work(
                                    &root,
                                    |dst| {
                                        crate::util::git(&root, &["update-ref", dst, &holding]).ok()
                                    },
                                    &base,
                                    &crate::land::failed_work_branch(author, &job.str("id")),
                                    &home,
                                    &project.str("slug"),
                                );
                                if !kept.is_empty() {
                                    log.add(format!("the refused run's work: {kept}"));
                                    summary.push_str(&format!("; {kept}"));
                                }
                            } else {
                                let own = crate::keys::own_commit_count(
                                    &root, &base, &holding, &accepted,
                                );
                                if own < commits {
                                    log.add(format!(
                                        "identity check (ADR-022): {} of the {commits} commit(s) are already on the project's own branches, which the worker built on; the {own} this run added carry its identity",
                                        commits - own
                                    ));
                                }
                                crate::util::git_check(
                                    &root,
                                    &["update-ref", &format!("refs/heads/{branch}"), &holding],
                                )?;
                                if ex.pushes() {
                                    log.add(push_from_hand(
                                        s,
                                        ex.as_ref(),
                                        &project,
                                        &root,
                                        &branch,
                                        &holding,
                                        agent,
                                        now,
                                    ));
                                }
                                status = "done".into();
                                checked_and_fetched = verify_rc == Some(0);
                                // The kept work landed on the job's branch.
                                if let Some(u) = &unchecked {
                                    crate::unchecked::drop_branch(&root, &u.branch);
                                }
                                summary = if parsed.summary.is_empty() {
                                    format!("{commits} commit(s) landed on {branch}")
                                } else {
                                    parsed.summary.clone()
                                };
                                if !verify_note.is_empty() {
                                    summary.push_str(&format!(" ({verify_note})"));
                                }
                                // The change has a face and its folder shows
                                // no change before and after: the worker's
                                // session carries on once more, for the
                                // report alone, while the clone is here.
                                if !custom_worker
                                    && unchecked.is_none()
                                    && !ex.remote()
                                    && !owner_cancelled.load(Ordering::SeqCst)
                                    && lease_ok(REPORT_TURN_MINUTES + 5)
                                {
                                    report_turn(
                                        s,
                                        &mut *ex,
                                        &cancel,
                                        ReportTurn {
                                            job: &job,
                                            project: &project,
                                            run_id: &run_id,
                                            base: &base,
                                            clone: &clone,
                                            changed: &changed_files,
                                            model_class: &model_class,
                                            spec: agent_spec.as_ref(),
                                            pins: &pins,
                                            env: &env,
                                        },
                                        &mut log,
                                    );
                                }
                            }
                            let _ = crate::util::git(&root, &["update-ref", "-d", &holding]);
                        }
                    }
                }
            }
        }
        Ok(())
    })();
    if let Err(err) = body_result {
        if let Some(e) = err.downcast_ref::<crate::executor::ExecutorUnreachable>() {
            transport_error = true;
            let when = if ex.started() { "after" } else { "before" };
            status = if ex.started() { "failed" } else { "error" }.into();
            summary = format!(
                "executor {} unreachable {} worker start: {}",
                ex.name(),
                when,
                e
            );
            log.put(&summary);
        } else if let Some(e) = err.downcast_ref::<crate::executor::ExecutorFault>() {
            fault = true;
            status = "error".into();
            summary = e.to_string();
            log.put(&summary);
        } else if let Some(e) = err.downcast_ref::<crate::executor::ExecutorUnfit>() {
            unfit = Some(e.clone());
            status = "error".into();
            summary = e.to_string();
            log.put(&summary);
        } else if let Some(e) = err.downcast_ref::<crate::recipe::RecipeInvalid>() {
            recipe_invalid = true;
            status = "error".into();
            summary = e.to_string();
            log.put(&summary);
        } else if let Some(e) = err.downcast_ref::<RunLost>() {
            status = "lost".into();
            summary = e.to_string();
            exit_code = None;
            log.put(&summary);
        } else {
            status = "error".into();
            summary = err_message(&err);
            log.put(&summary);
        }
    }
    // The bulk volume (bulk.rs) went away under the run: whatever the worker
    // or the verify reported is the disk's doing, not the job's. An executor
    // fault keeps the job's attempt and queues it again; the daemon holds
    // dispatch until the disk is back.
    if status != "done"
        && status != "dry-run"
        && status != "stale"
        && status != "queued"
        && !ex.remote()
        && let crate::bulk::State::Missing { target, .. } =
            crate::bulk::state(std::path::Path::new(&s.home))
    {
        fault = true;
        unfit = None;
        status = "error".into();
        summary = crate::bulk::run_fault_note(&target);
        log.put(&summary);
    }
    stop_watch();
    hand_watch.stop();
    // `hive job stop` without `--requeue` cut the run (owner_stop): in its
    // setup, end check or landing that reads as a run the runner stopped,
    // which keeps the commits and requeues the job free so its next run
    // lands them - the opposite of the stop the owner asked for. It ends
    // as the owner's cancel does in the worker (TASK-169 item 4).
    if status == "lost" && owner_cancelled.load(Ordering::SeqCst) {
        status = "failed".into();
        summary = match summary.strip_prefix(LOST_RUN_SUMMARY) {
            Some(rest) => format!("cancelled by the owner{rest}"),
            None => "cancelled by the owner".into(),
        };
        check_stopped = None;
        exit_code = Some(5);
        log.put(&summary);
    }
    // The worker is gone: close any session it left `running` with a reason,
    // instead of waiting hours for the sweep to mark it `lost` with no error.
    let closed = crate::steps_mirror::close_running_sessions(
        s,
        &run_id,
        &status,
        exit_code,
        status == "timeout",
    );
    if !closed.is_empty() {
        log.add(format!(
            "closed {} harness session(s) left running by the worker: {}",
            closed.len(),
            closed.join(", ")
        ));
    }
    if status != "done" {
        branch.clear();
    }
    let seconds = (since_seconds(started) * 10.0).round() / 10.0;
    // The work directory's size before it goes: the footprint the daemon's
    // disk admission counts for the next runs of this project (diskroom.rs).
    let work_mb = if ex.remote() {
        0
    } else {
        crate::diskroom::dir_mb(Path::new(&ex.clone()))
    };
    // The stop came from the owner's cancel - of the run, or of the task it
    // works for (the watch cancels the run's token for both) - not from a
    // restart: nobody wants the work any more, so it is neither kept nor
    // queued again (TASK-152 item 7).
    // The job itself cancelled while it was leased (`hive job finish ...
    // cancelled` from another shell, the console's stop) is the same: the
    // board refuses to queue it again (hive-board finish_run_job).
    let job_cancelled = crate::jobs::job_row(&*c, job.str("id")).str("status") == "cancelled";
    let owner_stopped =
        owner_cancelled.load(Ordering::SeqCst) || run_task_cancelled(s, &run_id) || job_cancelled;
    if status == "lost" && check_stopped.is_some() && owner_stopped {
        check_stopped = None;
        status = "failed".into();
        summary =
            format!("cancelled by the owner during the check; {commits} commit(s) not landed");
        if let Some(u) = &unchecked {
            crate::unchecked::drop_branch(&root, &u.branch);
        }
        log.put(&summary);
    }
    // A machine that stopped answering after the worker started may still
    // hold the run's commits: they are fetched before anything cleans the
    // clone (TASK-156 item 5). On 2026-11-27 a rented machine whose hours
    // were up dropped one `verify` call (ssh exit 255) 3 h 50 min into a run;
    // the `clean` a moment later answered and removed the clone with the
    // work in it, and the machine was released. Now the commits go to the
    // failed-work branch like a failed check's, and when they cannot be
    // fetched the clone stays and is marked (`work_not_fetched`): the
    // machine is not released idle inside its hours while it holds them,
    // and when they are up the loss is said on the job's task
    // (`compute::say_lost_work`). Nothing here rents or extends a machine.
    if transport_error && ex.started() && ex.remote() && !o.dry_run && !root.is_empty() {
        let run_base =
            crate::util::must_one(&c, "SELECT base FROM runs WHERE id = ?", &[&run_id]).str("base");
        let made = if run_base.is_empty() {
            0
        } else {
            ex.commit_count(&run_base)
        };
        if made > 0 {
            commits = made;
            let kept_branch = crate::land::failed_work_branch(agent, &job.str("id"));
            let mut fetch_failed = false;
            let kept = crate::land::keep_failed_work(
                &root,
                |dst| {
                    // The run's own token may be cancelled: the fetch gets one of its own.
                    let ok = matches!(
                        fetch_run_work(
                            s,
                            &hive_bus::CancellationToken::new(),
                            ex.as_ref(),
                            &job.str("id"),
                            &root,
                            dst,
                            &log,
                        ),
                        Ok(o) if o.exit_code == 0
                    );
                    fetch_failed = !ok;
                    ok
                },
                &run_base,
                &kept_branch,
                &home,
                &project.str("slug"),
            );
            if !kept.is_empty() {
                summary.push_str(&format!(
                    "; {made} commit(s) fetched before the clone went: {kept}"
                ));
            } else if fetch_failed {
                work_not_fetched = true;
                let how = ex.fetch_by_hand(&root, &kept_branch);
                summary.push_str(&format!(
                    "; its {made} commit(s) could not be fetched, so the clone is kept{}",
                    if how.is_empty() {
                        String::new()
                    } else {
                        format!(": {how}")
                    }
                ));
            }
            log.put(&summary);
        }
    }
    // The runner stopped the check of finished work: its commits are kept on
    // a branch of the project's checkout before the clone goes, and the
    // job's next run checks them instead of doing the task again. A clone
    // whose commits could not be fetched is not removed.
    let mut stopped_clone_kept = false;
    if status == "lost"
        && let Some(kept_base) = check_stopped.clone()
    {
        let author = unchecked.as_ref().map_or(agent, |u| u.agent.as_str());
        let kept_branch = crate::unchecked::unchecked_branch(author, &job.str("id"));
        // The run's own token is cancelled: the fetch gets one of its own.
        let fetched = !root.is_empty()
            && matches!(
                fetch_run_work(
                    s,
                    &hive_bus::CancellationToken::new(),
                    ex.as_ref(),
                    &job.str("id"),
                    &root,
                    &format!("refs/heads/{kept_branch}"),
                    &log,
                ),
                Ok(o) if o.exit_code == 0
            );
        if fetched {
            let kept = crate::unchecked::Unchecked {
                run: run_id.clone(),
                agent: author.to_string(),
                pubkey: unchecked
                    .as_ref()
                    .map_or_else(|| pubkey.clone(), |u| u.pubkey.clone()),
                branch: kept_branch.clone(),
                base: kept_base,
                report: unchecked
                    .as_ref()
                    .map_or_else(|| worker_report.clone(), |u| u.report.clone()),
                commits: commits as i64,
                clone: String::new(),
            };
            crate::unchecked::record(&c, &s.now(), &job.str("id"), &kept);
            unchecked_kept = true;
            summary = format!(
                "{LOST_RUN_SUMMARY} ({}; {commits} commit(s) kept on branch {kept_branch}; the job's next run checks and lands them)",
                if summary.contains("during the landing") {
                    "during the landing"
                } else {
                    "during the check"
                }
            );
        } else if ex.remote() {
            work_not_fetched = true;
            summary.push_str("; its commits could not be fetched, so the clone is kept");
        } else {
            // Written down with the clone the commits are in: the job's
            // next run fetches them from there before its own clone would
            // remove it (TASK-152 item 1).
            let kept = crate::unchecked::Unchecked {
                run: run_id.clone(),
                agent: author.to_string(),
                pubkey: unchecked
                    .as_ref()
                    .map_or_else(|| pubkey.clone(), |u| u.pubkey.clone()),
                branch: kept_branch.clone(),
                base: kept_base,
                report: unchecked
                    .as_ref()
                    .map_or_else(|| worker_report.clone(), |u| u.report.clone()),
                commits: commits as i64,
                clone: ex.clone(),
            };
            crate::unchecked::record(&c, &s.now(), &job.str("id"), &kept);
            stopped_clone_kept = true;
            summary.push_str(&format!(
                "; its commits could not be kept on {kept_branch}, so the clone {} is kept for the job's next run",
                ex.clone()
            ));
        }
        log.put(&summary);
    }
    // A run on another machine the daemon stopped while its worker went on
    // (the restart's wait for other machines, daemon.restart_remote_minutes):
    // what it committed is fetched into the project's checkout as kept work
    // before its clone goes, and the job's next run starts from it - the
    // restart did not wait for it, and nothing it finished is lost.
    let mut daemon_stopped_remote = false;
    let stop_why = o.stop_why.get();
    if status == "lost"
        && ex.remote()
        && check_stopped.is_none()
        && !owner_stopped
        && !stop_why.is_empty()
        && !root.is_empty()
        && ex.started()
    {
        let run_base = crate::util::query_one(&c, "SELECT base FROM runs WHERE id = ?", &[&run_id])
            .ok()
            .flatten()
            .map(|r| r.str("base"))
            .unwrap_or_default();
        let made = if run_base.is_empty() {
            0
        } else {
            ex.commit_count(&run_base)
        };
        summary = format!("stopped on {}: {stop_why}", ex.name());
        // The daemon stopped it, not the job: its job goes back to the
        // queue at no cost of an attempt whatever it had committed.
        daemon_stopped_remote = true;
        if made > 0 {
            commits = made;
            let kept_branch = crate::land::failed_work_branch(agent, &job.str("id"));
            let mut fetch_failed = false;
            // The run's own token is cancelled: the fetch gets one of its own.
            let kept = crate::land::keep_failed_work(
                &root,
                |dst| {
                    let ok = matches!(
                        fetch_run_work(
                            s,
                            &hive_bus::CancellationToken::new(),
                            ex.as_ref(),
                            &job.str("id"),
                            &root,
                            dst,
                            &log,
                        ),
                        Ok(o) if o.exit_code == 0
                    );
                    fetch_failed = !ok;
                    ok
                },
                &run_base,
                &kept_branch,
                &home,
                &project.str("slug"),
            );
            if !kept.is_empty() {
                summary.push_str(&format!(
                    "; {made} commit(s) fetched, {kept}; the job's next run starts from them"
                ));
                let patch = kept
                    .split(" and in ")
                    .nth(1)
                    .unwrap_or_default()
                    .to_string();
                crate::util::event_at(
                    &c,
                    &s.now(),
                    "runner",
                    "job.requeue",
                    &job.str("id"),
                    json!({"why": format!("the runner stopped its run on {}: {stop_why}", ex.name()), "patch": patch, "run": run_id, "branch": kept_branch}),
                );
            } else if fetch_failed {
                // The commits stay on the machine with its clone; the
                // maintenance pass fetches them from there (kept_work.rs).
                work_not_fetched = true;
                summary.push_str(&format!(
                    "; its {made} commit(s) could not be fetched, so the clone on {} is kept and fetched from there",
                    ex.name()
                ));
            }
        } else {
            summary.push_str("; it had committed nothing, so the job starts over");
        }
        log.put(&summary);
    }
    // The agent was told to remove what it added (brief rule 9); whatever is
    // still registered on the clone goes now and is written down against it.
    // A held run keeps its clone for the run that carries on.
    let held_clone =
        (status == "held" || stopped_clone_kept || stranded_clone_kept) && !ex.remote();
    let left = if held_clone {
        ex.hold()
    } else if work_not_fetched {
        // Written down against the machine: the lease keeps code-changing
        // work off it until a fetch from it works again (TASK-133).
        crate::util::event_at(
            &s.conn()?,
            &s.now(),
            "runner",
            crate::executor::FETCH_FAILED_EVENT,
            &ex.name(),
            json!({"job": job.str("id"), "run": run_id, "clone": ex.clone()}),
        );
        ex.keep_work()
    } else {
        ex.cleanup()
    };
    if !ex.remote() {
        // Evidence stays for the hub; a folder with nothing in it goes with
        // the clone (a held run's next pass makes it again).
        crate::ui_evidence::drop_empty_dir(&home, &job.str("id"));
    }
    drop(build_cache);
    if ex.started() {
        let clone_text = if held_clone {
            format!("clone kept for the next run ({work_mb} MB)")
        } else if work_not_fetched {
            format!("clone kept on {}: its commits were not fetched", ex.name())
        } else if ex.remote() {
            format!("clone removed on {}", ex.name())
        } else {
            format!("clone removed ({work_mb} MB)")
        };
        let text = format!("{clone_text}; {}", crate::tidy::leftover_summary(&left));
        log.put(&format!("cleanup: {text}"));
        let mut d = serde_json::Map::new();
        d.insert("job".into(), json!(job.str("id")));
        d.insert("project".into(), json!(project.str("slug")));
        d.insert("executor".into(), json!(ex.name()));
        d.insert("status".into(), json!(status));
        d.insert("clone_mb".into(), json!(work_mb));
        d.insert("left".into(), json!(left));
        d.insert("text".into(), json!(text));
        crate::util::event_at(
            &c,
            &s.now(),
            agent,
            "agent.cleanup",
            &run_id,
            Value::Object(d),
        );
    }
    // What the run cost: the harness sessions it ran, summed onto the run.
    let spent = run_cost(&c, &run_id, &job.str("id"), now);
    if spent.prompt_tokens > 0 || spent.completion_tokens > 0 || spent.cost > 0.0 {
        log.add(format!(
            "cost: {} prompt + {} completion tokens, {} (provider units)",
            spent.prompt_tokens,
            spent.completion_tokens,
            trim_cost(spent.cost)
        ));
    }
    write_run_log(&run_id, &log_path, &log.text());
    let exit_val: Option<i32> = exit_code;
    let record_run = |status: &str,
                      summary: &str,
                      branch: &str,
                      commits: i32,
                      verify_note: &str| {
        crate::util::exec1(
            &c,
            "UPDATE runs SET finished_at = ?, status = ?, exit_code = ?, seconds = ?, branch = ?, commits = ?, summary = ?, verify = ?, work_mb = ?, prompt_tokens = ?, completion_tokens = ?, cost = ?, outcome = ?, outcome_source = ? WHERE id = ?",
            &[
                &s.now(),
                &status,
                &exit_val,
                &seconds,
                &branch,
                &commits,
                &summary,
                &verify_note,
                &work_mb,
                &spent.prompt_tokens,
                &spent.completion_tokens,
                &spent.cost,
                &outcome.code,
                &outcome.source,
                &run_id,
            ],
        );
    };
    crate::executor::note_executor_run(
        s,
        now,
        &ex.name(),
        &status,
        &summary,
        transport_error,
        fault,
    );
    if let Some(u) = &unfit {
        crate::recipe::mark_unfit(&c, now, &ex.name(), &u.project, &u.tool);
        crate::recipe::tools_wanted(
            s,
            crate::board::brain_project(&c),
            &ex.name(),
            &u.tool,
            &u.project,
            &u.detail,
            now,
            agent,
        );
    }
    let run_row = || crate::util::must_one(&c, "SELECT * FROM runs WHERE id = ?", &[&run_id]);
    let finish_event = |st: &str, br: &str| {
        let mut d = serde_json::Map::new();
        d.insert("status".into(), json!(st));
        d.insert("commits".into(), json!(commits));
        d.insert("branch".into(), json!(br));
        d.insert("executor".into(), json!(ex.name()));
        if fault {
            d.insert("fault".into(), json!(true));
        }
        if !outcome.source.is_empty() {
            d.insert("outcome".into(), json!(outcome.code));
            d.insert("outcome_source".into(), json!(outcome.source));
        }
        d.insert("prompt_tokens".into(), json!(spent.prompt_tokens));
        d.insert("completion_tokens".into(), json!(spent.completion_tokens));
        d.insert("cost".into(), json!(spent.cost));
        crate::util::event_at(&c, now, agent, "run.finish", &run_id, Value::Object(d));
    };
    if status == "stale" {
        record_run(&status, &summary, "", commits, &verify_note);
        finish_event(&status, "");
        return Ok(run_row());
    }
    let mut job_status = if status == "done" || status == "dry-run" {
        "done".to_string()
    } else {
        "failed".to_string()
    };
    match status.as_str() {
        "dry-run" => job_status = "queued".into(),
        // Held for the runner's restart: the job waits in the queue for the
        // run that carries it on, and the hold costs it no attempt.
        "held" => {
            job_status = "queued".into();
            crate::util::exec1(
                &c,
                "UPDATE jobs SET attempts = MAX(attempts - 1, 0) WHERE id = ?",
                &[&job.str("id")],
            );
        }
        // Stopped by the owner to run again (`hive job stop --requeue`):
        // back to the queue whatever the run ended as, and the stop costs
        // the job no attempt.
        _ if status != "done" && owner_stop(&*c, &run_id) == Some(true) => {
            job_status = "queued".into();
            summary = OWNER_REQUEUE_SUMMARY.to_string();
            crate::util::exec1(
                &c,
                "UPDATE jobs SET attempts = MAX(attempts - 1, 0) WHERE id = ?",
                &[&job.str("id")],
            );
        }
        // The runner stopped the run before its cap: back to the queue, as
        // after a runner that died under it, until the attempts run out.
        // Finished work kept for the next run's check: back to the queue
        // at no cost of an attempt, like a hold - the work is not redone.
        "lost" if unchecked_kept || stopped_clone_kept || daemon_stopped_remote => {
            job_status = "queued".into();
            crate::util::exec1(
                &c,
                "UPDATE jobs SET attempts = MAX(attempts - 1, 0) WHERE id = ?",
                &[&job.str("id")],
            );
        }
        "lost" => {
            if job.int("attempts") < MAX_ATTEMPTS {
                job_status = "queued".into();
            } else {
                job_status = "failed".into();
                summary = format!(
                    "failed: the runner stopped while this job was in progress {} times",
                    job.int("attempts")
                );
            }
        }
        "error" if fault || unfit.is_some() || recipe_invalid => {
            job_status = "queued".into();
            crate::util::exec1(
                &c,
                "UPDATE jobs SET attempts = MAX(attempts - 1, 0) WHERE id = ?",
                &[&job.str("id")],
            );
        }
        "failed" if unfit.is_some() && commits > 0 && job.int("attempts") < MAX_ATTEMPTS => {
            job_status = "queued".into()
        }
        "error" => {
            if job.int("attempts") < MAX_ATTEMPTS {
                job_status = "queued".into();
            } else {
                job_status = "failed".into();
                summary = format!(
                    "failed after {} runner errors; last: {}",
                    job.int("attempts"),
                    summary
                );
            }
        }
        _ => {}
    }
    // A job whose task the owner cancelled while it ran is not put back in
    // the queue by the run's end, whatever stopped it: the cancel ends the
    // task's open jobs, and this one was leased then (TASK-152 item 7).
    // A job itself cancelled while it ran stays cancelled: the board
    // refuses to queue it again.
    if job_status == "queued" && status != "dry-run" && owner_stopped {
        job_status = if job_cancelled { "cancelled" } else { "failed" }.into();
    }
    let final_branch = if job_status == "queued" {
        job.str("branch")
    } else {
        branch.clone()
    };
    let finish = hive_board::RunJobResult {
        run_id: run_id.clone(),
        job_id: job.str("id"),
        agent: agent.to_string(),
        status: job_status.clone(),
        result: summary.clone(),
        branch: final_branch,
        generation: job.int("generation") as i64,
    };
    let finished = match hive_board::finish_run_job(s, finish.clone()) {
        Err(e) if status == "done" && checked_and_fetched && commits > 0 => {
            // The lease lapsed under a run that finished its work: the
            // check passed and the commits are in the checkout. Unless
            // another run took the job meanwhile, the job takes this result
            // rather than being done again from nothing (TASK-179).
            match hive_board::adopt_stale_done_run(s, finish) {
                Ok(Some(_)) => {
                    log.add(format!(
                        "the job's lease lapsed under this run ({}); its check passed and its commits were fetched, so the job takes this result",
                        err_message(&e)
                    ));
                    Ok(())
                }
                Ok(None) => Err(e),
                Err(adopt) => {
                    log.add(format!(
                        "the job could not take this run's result: {}",
                        err_message(&adopt)
                    ));
                    Err(e)
                }
            }
        }
        other => other.map(|_| ()),
    };
    if let Err(e) = finished {
        log.put(&err_message(&e));
        eprintln!("{}", err_message(&e));
        write_run_log(&run_id, &log_path, &log.text());
        status = "stale".into();
        summary = format!("{} ({})", summary, err_message(&e));
        record_run(&status, &summary, &branch, commits, &verify_note);
        finish_event("stale", &branch);
        return Ok(run_row());
    }
    record_run(&status, &summary, &branch, commits, &verify_note);
    if job_status == "done" {
        crate::util::exec1(
            &c,
            "UPDATE agents SET jobs_done = jobs_done + 1 WHERE name = ?",
            &[&agent],
        );
        crate::board::touch_agent(s, agent.to_string(), commits > 0, now.to_string());
    } else if job_status == "failed" && status != "error" {
        crate::util::exec1(
            &c,
            "UPDATE agents SET jobs_failed = jobs_failed + 1 WHERE name = ?",
            &[&agent],
        );
    }
    let quiet_requeue =
        job_status == "queued" && matches!(status.as_str(), "error" | "held" | "lost");
    if status != "dry-run" && !quiet_requeue {
        let mut lines = vec![
            format!(
                "Run `{}` of job `{}` ({}, Tier {}) by {} on executor `{}`: **{}** in {}s.",
                run_id,
                job.str("id"),
                job.str("kind"),
                job.int("tier"),
                agent,
                ex.name(),
                status,
                seconds
            ),
            String::new(),
        ];
        let br = if branch.is_empty() {
            String::new()
        } else {
            format!(
                "; branch `{branch}` (local; lands on the default branch at the next landing pass, ADR-024)"
            )
        };
        lines.push(format!("- Commits: {commits}{br}"));
        if !verify_note.is_empty() {
            lines.push(format!("- Verify: {verify_note}"));
        }
        lines.push(format!("- Log: {log_path}"));
        // What there is to look at: the files the worker left in the job's
        // artifacts folder, or the plain word that a UI change left none.
        if !ex.remote() {
            let evidence = crate::ui_evidence::UiEvidence::collect(
                &home,
                &job.str("id"),
                &changed_files,
                crate::ui_evidence::Mode::of(s, &project.str("slug")),
            );
            lines.extend(evidence.result_lines());
        }
        for f in &parsed.findings {
            lines.push(format!("- Finding: {f}"));
        }
        for f in &parsed.shared {
            lines.push(format!("- Finding (shared board): {}", f.card));
        }
        for l in &parsed.lessons {
            lines.push(format!("- Lesson: {l}"));
        }
        let arow = crate::board::actor_agent(s, agent.to_string());
        let cast = cast_result_votes(
            s,
            &project.str("slug"),
            arow.clone().map(|b| *b),
            agent,
            now,
            &run_id,
            parsed.votes.clone(),
            ballot_items_to_values(&run_ballot),
            &mut log,
        );
        if !parsed.votes.is_empty() {
            lines.push(format!(
                "- Votes: {cast} cast of {} `VOTE:` line(s)",
                parsed.votes.len()
            ));
        }
        // No `--model`: the harness resolved the provider's default and
        // its session names it; the decision records that, not nothing.
        let decided_model = resolved_run_model(&c, &ran_model, &run_id, &job.str("id"), now);
        let decided =
            apply_result_decisions_ran(s, job.clone(), parsed.clone(), &mut log, &decided_model);
        if !parsed.decisions.is_empty()
            && job.str("kind") == crate::lead::LEAD_KIND
            && !job
                .str("source")
                .starts_with(hive_tasks::leads::SOURCE_PREFIX)
        {
            lines.push(format!(
                "- Decisions: {decided} recorded of {} DECIDE: line(s)",
                parsed.decisions.len()
            ));
        }
        if !parsed.verdicts.is_empty() {
            let recorded = crate::suggestions::apply_result_verdicts(
                s,
                &job,
                &parsed.verdicts,
                agent,
                &mut log,
            );
            lines.push(format!(
                "- Verdicts: {recorded} recorded of {} VERDICT: line(s)",
                parsed.verdicts.len()
            ));
        }
        if !parsed.work.is_empty() {
            let filed = crate::suggestions::file_result_work(
                s,
                &project.str("slug"),
                &parsed.work,
                agent,
                &mut log,
            );
            lines.push(format!(
                "- Suggested work: {} filed of {} `SUGGESTION: work:` line(s){}",
                filed.len(),
                parsed.work.len(),
                if filed.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", filed.join(", "))
                }
            ));
        }
        let res = if job_status == "done" { "tested" } else { "" };
        let branch_box = Box::new(branch.clone());
        let failures = PostFailures {
            c: &c,
            now,
            agent,
            job_id: job.str("id"),
            run_id: &run_id,
            log: &log,
        };
        if let Err(e) = crate::board::create_post(
            s,
            now.to_string(),
            crate::board::PostOpts {
                kind: "result".into(),
                project: project.str("slug"),
                area: "jobs".into(),
                thread: job.str("id"),
                summary: crate::util::trunc(&format!("{}: {}", status, summary), 230),
                body: lines.join("\n") + "\n",
                agent: arow.clone(),
                session: run_id.clone(),
                branch: Some(branch_box),
                resolution: res.into(),
                ..Default::default()
            },
        ) {
            failures.note("result", &e);
        }
        if job_status == "done"
            && let Err(e) =
                post_result_answer(s, job.clone(), agent, now, &run_id, &branch, parsed.clone())
        {
            failures.note("answer", &e);
        }
        for line in &parsed.questions {
            // TASK-129: the answers ride on the line (`question | recommended
            // | alternative`); a question without them goes to the lead.
            let (q, answers) = hive_board::split_question_line(line);
            if let Err(e) = crate::board::create_post(
                s,
                now.to_string(),
                crate::board::PostOpts {
                    kind: "question".into(),
                    project: project.str("slug"),
                    area: "jobs".into(),
                    thread: job.str("id"),
                    summary: q.clone(),
                    body: format!(
                        "{}

{q}

{answers}",
                        hive_board::WORKER_QUESTION_PREAMBLE
                    ),
                    agent: arow.clone(),
                    session: run_id.clone(),
                    for_owner: true,
                    ..Default::default()
                },
            ) {
                failures.note("question", &e);
            }
        }
        // Teacher mode: each lesson the worker reported becomes a `lesson`
        // post on the project board, in the job's thread, so a junior finds
        // the why of the run's choices beside its result.
        for (i, l) in parsed.lessons.iter().enumerate() {
            if let Err(e) = crate::board::create_post(
                s,
                now.to_string(),
                crate::board::PostOpts {
                    kind: "lesson".into(),
                    project: project.str("slug"),
                    area: "lessons".into(),
                    thread: job.str("id"),
                    summary: crate::util::trunc(l, 230),
                    body: format!(
                        "{l}\n\nLesson {} of {} from the worker in run `{}` of job `{}` ({}), with teacher mode on: why it made this design or coding choice, for a developer learning from the run.\n",
                        i + 1,
                        parsed.lessons.len(),
                        run_id,
                        job.str("id"),
                        job.str("title")
                    ),
                    agent: arow.clone(),
                    session: run_id.clone(),
                    ..Default::default()
                },
            ) {
                failures.note("lesson", &e);
            }
        }
        // A card about Hive itself (`Class: hive`) from any project but the
        // brain goes to the shared board, where the brain reads it at every
        // reflect; on the brain it is a card like any other.
        let brain = s.brain().ok().flatten().map(|b| b.slug).unwrap_or_default();
        let slug = project.str("slug");
        let to_commons = |card: &str| {
            slug != brain
                && slug != hive_store::COMMONS_SLUG
                && crate::brief::card_for_commons(card)
        };
        for f in &parsed.shared {
            if slug == hive_store::COMMONS_SLUG {
                break;
            }
            let area = if f.area.is_empty() {
                "patterns".to_string()
            } else {
                f.area.clone()
            };
            let thread = if f.thread.is_empty() {
                job.str("id")
            } else {
                f.thread.clone()
            };
            if let Err(e) = crate::board::create_post(
                s,
                now.to_string(),
                crate::board::PostOpts {
                    kind: "finding".into(),
                    project: hive_store::COMMONS_SLUG.into(),
                    area,
                    thread,
                    summary: crate::util::trunc(&f.card, 230),
                    body: format!(
                        "{}\n\nSeen on project `{}` by the worker in run `{}` of job `{}` ({}); shared here for every project on this machine.\n",
                        f.card,
                        slug,
                        run_id,
                        job.str("id"),
                        job.str("title")
                    ),
                    agent: arow.clone(),
                    session: run_id.clone(),
                    origin_project: slug.clone(),
                    ..Default::default()
                },
            ) {
                failures.note("shared finding", &e);
            }
        }
        for sg in &parsed.suggestions {
            let shared = to_commons(&sg.card);
            let area = if !sg.area.is_empty() {
                sg.area.clone()
            } else if shared {
                "patterns".to_string()
            } else {
                "evolution".to_string()
            };
            let thread = if sg.thread.is_empty() {
                job.str("id")
            } else {
                sg.thread.clone()
            };
            let (card, sum, ok) = crate::brief::card_body(sg.card.clone());
            let (card, sum) = if ok {
                (card, sum)
            } else {
                (sg.card.clone(), sg.card.clone())
            };
            let (post_project, origin_project, body) = if shared {
                (
                    hive_store::COMMONS_SLUG.to_string(),
                    slug.clone(),
                    format!(
                        "A wish about Hive itself, seen on project `{}` by the worker in run `{}` of job `{}` ({}) and not made there; the brain reads the shared board at every reflect.\n\n{}\n",
                        slug,
                        run_id,
                        job.str("id"),
                        job.str("title"),
                        card
                    ),
                )
            } else {
                (
                    slug.clone(),
                    String::new(),
                    format!(
                        "One-variable improvement seen by the worker in run `{}` of job `{}` ({}), not made there.\n\n{}\n",
                        run_id,
                        job.str("id"),
                        job.str("title"),
                        card
                    ),
                )
            };
            if let Err(e) = crate::board::create_post(
                s,
                now.to_string(),
                crate::board::PostOpts {
                    kind: "suggestion".into(),
                    project: post_project,
                    area,
                    thread,
                    summary: sum,
                    body,
                    agent: arow.clone(),
                    session: run_id.clone(),
                    origin_project,
                    ..Default::default()
                },
            ) {
                failures.note("suggestion", &e);
            }
        }
        write_run_log(&run_id, &log_path, &log.text());
    }
    finish_event(&status, &branch);
    Ok(run_row())
}

fn ballot_items_to_values(items: &[BallotItem]) -> Vec<Value> {
    items.iter().map(|it| json!({"id": it.id, "key": it.key, "summary": it.summary, "body": it.body, "up": it.up, "down": it.down, "kind": it.kind, "hash": it.hash})).collect()
}

pub(crate) fn post_result_answer(
    s: &hive_store::Store,
    job: Row,
    agent: &str,
    now: &str,
    run_id: &str,
    branch: &str,
    rep: Report,
) -> Result<()> {
    if job.str("kind") != "answer" || !rep.questions.is_empty() || rep.summary.trim().is_empty() {
        return Ok(());
    }
    if crate::evolve_plan::hard_stop_text(rep.summary.clone()) {
        return Err(cli_err(
            2,
            format!("job {} reported a hard stop", job.str("id")),
        ));
    }
    let Some(id) = job
        .str("source")
        .strip_prefix("question:")
        .filter(|s| !s.is_empty())
        .map(str::to_string)
    else {
        return Err(cli_err(
            2,
            format!("job {} has no question source", job.str("id")),
        ));
    };
    let c = conn(s);
    let q = crate::board::post_row(&c, id.clone());
    // A question of another project may be answered here only when it was
    // addressed to this project (`--to` or `@slug`); the answer then goes
    // back on the asker's thread and names this project as its origin.
    let asked_here = !is_nil(&q)
        && q.str("project") != job.str("project")
        && hive_board::extra_projects(&q.str("extra")).contains(&job.str("project"));
    if is_nil(&q)
        || q.str("kind") != "question"
        || (q.str("project") != job.str("project") && !asked_here)
    {
        return Err(cli_err(
            2,
            format!(
                "job {} does not name a question in its project",
                job.str("id")
            ),
        ));
    }
    if !crate::plan::answerable(q.clone(), job.str("created_by"))
        || !hive_board::owner_only_proposal(&q.str("summary"), &q.str("body")).is_empty()
        || crate::evolve_plan::hard_stop_text(format!("{}\n{}", q.str("summary"), q.str("body")))
    {
        return Err(cli_err(2, format!("question {} requires the owner", id)));
    }
    let still_open = if asked_here {
        hive_board::open_asks_to(s, &job.str("project"))?
            .iter()
            .any(|a| a.post.id == id)
    } else {
        crate::plan::open_questions(s, job.str("project"), now.to_string())
            .iter()
            .any(|r| r.str("id") == id)
    };
    if !still_open {
        return Ok(());
    }
    if !crate::util::must_one(&c, "SELECT 1 FROM posts a JOIN posts q ON q.id = ? WHERE a.project = q.project AND a.kind = 'answer' AND a.area = q.area AND a.thread = q.thread AND a.date = q.date AND a.rowid > q.rowid LIMIT 1", &[&id]).0.is_empty() { return Ok(()); }
    crate::board::create_post(
        s,
        now.to_string(),
        crate::board::PostOpts {
            kind: "answer".into(),
            project: q.str("project"),
            area: q.str("area"),
            thread: q.str("thread"),
            summary: crate::util::trunc(&rep.summary, 230),
            body: rep.summary.trim().to_string() + "\n",
            on: id,
            agent: crate::board::actor_agent(s, agent.to_string()),
            session: run_id.to_string(),
            branch: Some(Box::new(branch.to_string())),
            origin_project: if asked_here {
                job.str("project")
            } else {
                String::new()
            },
            ..Default::default()
        },
    )?;
    Ok(())
}

/// Whether run `run_id` works for a task the owner cancelled (TASK-140): the
/// task's cancel asks the run's harness session to stop, and this covers a
/// run that has no session yet (still setting up its clone) or a worker
/// that keeps none. The run then ends as the owner's cancel of a run does.
pub(crate) fn run_task_cancelled(s: &hive_store::Store, run_id: &str) -> bool {
    let Ok(conn) = s.conn() else {
        return false;
    };
    conn.query_row(
        "SELECT 1 FROM runs r JOIN jobs j ON j.id = r.job_id JOIN tasks t ON (j.source = 'task:' || t.id OR j.source LIKE 'task:' || t.id || ':%') WHERE r.id = ? AND j.source LIKE 'task:%' AND t.status = 'cancelled' LIMIT 1",
        [run_id],
        |_| Ok(()),
    )
    .is_ok()
}

/// Whether `hive job stop` asked `run_id` to stop
/// ([`hive_board::RUN_STOP_ACTION`]), and if so whether its job goes back to
/// the queue (`Some(true)`) or ends cancelled (`Some(false)`).
pub(crate) fn owner_stop(q: &(impl crate::util::Querier + ?Sized), run_id: &str) -> Option<bool> {
    let r = crate::util::must_one(
        q,
        "SELECT detail FROM events WHERE action = ? AND subject = ? ORDER BY seq DESC LIMIT 1",
        &[&hive_board::RUN_STOP_ACTION, &run_id],
    );
    if r.0.is_empty() {
        return None;
    }
    let detail: Value = serde_json::from_str(&r.str("detail")).unwrap_or(Value::Null);
    Some(
        detail
            .get("requeue")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    )
}

/// The summary of a run the owner stopped with `hive job stop --requeue`.
pub(crate) const OWNER_REQUEUE_SUMMARY: &str =
    "stopped by the owner (hive job stop --requeue); the job is back in the queue";

pub(crate) fn watch_run_harness_cancel(
    cancel: &hive_bus::CancellationToken,
    s: &hive_store::Store,
    run_id: &str,
    owner_cancelled: Arc<AtomicBool>,
) -> Box<dyn FnOnce() + Send> {
    watch_run_harness_cancel_every(cancel, s, run_id, owner_cancelled, Duration::from_secs(3))
}

pub(crate) fn watch_run_harness_cancel_every(
    cancel: &hive_bus::CancellationToken,
    s: &hive_store::Store,
    run_id: &str,
    owner_cancelled: Arc<AtomicBool>,
    interval: Duration,
) -> Box<dyn FnOnce() + Send> {
    let watch = cancel.child();
    let watch_thread = watch.clone();
    let run_id = run_id.to_string();
    let s = s.clone();
    let child_cancel = cancel.clone();
    let done = Arc::new(AtomicBool::new(false));
    let done_t = Arc::clone(&done);
    let handle = std::thread::spawn(move || {
        let check = || -> bool {
            let asked = matches!(
                s.harness_session_head(&run_id),
                Ok(Some(session)) if !session.cancel_requested_at.is_empty()
            );
            if asked || run_task_cancelled(&s, &run_id) {
                owner_cancelled.store(true, Ordering::SeqCst);
                child_cancel.cancel();
                return true;
            }
            // `hive job stop` (owner_stop): a run on any machine, with or
            // without a harness session here. Requeued, it ends as a run
            // the runner stopped; else as the owner's cancel.
            if let Some(requeue) = s.conn().ok().and_then(|c| owner_stop(&*c, &run_id)) {
                if !requeue {
                    owner_cancelled.store(true, Ordering::SeqCst);
                }
                child_cancel.cancel();
                return true;
            }
            false
        };
        if check() {
            done_t.store(true, Ordering::SeqCst);
            return;
        }
        // Check every interval until the watch is cancelled.
        let stopped = watch_thread.done();
        while let Err(crossbeam_channel::RecvTimeoutError::Timeout) = stopped.recv_timeout(interval)
        {
            if check() {
                break;
            }
        }
        done_t.store(true, Ordering::SeqCst);
    });
    Box::new(move || {
        watch.cancel();
        while !done.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        let _ = handle.join();
    })
}

/// Warning text written to stderr and the run log when a job's result (or
/// answer) post cannot be saved, so owners see which job lost its report and why.
/// Returns `None` when the error text is empty or whitespace-only: an error
/// with no message is almost certainly a spurious post failure (the post
/// actually landed), and a warning with empty error text tells owners nothing.
pub(crate) fn result_post_failure_message(job_id: &str, what: &str, err: &str) -> Option<String> {
    if err.trim().is_empty() {
        return None;
    }
    Some(format!(
        "warning: job {job_id}: {what} post failed, owners will not see this report on the board: {err}"
    ))
}

/// The event a lost post leaves behind: `post.failed` on the job, with the
/// kind of post, the error and the run, so the hub and `hive status` can
/// show what the stderr line and the run log alone could not.
pub(crate) fn post_failed_detail(what: &str, err: &str, run_id: &str) -> Value {
    json!({"kind": what, "error": err.trim(), "run": run_id})
}

/// Where a run's lost posts are noted: a `what` post (result, answer,
/// question, lesson, finding, suggestion) that could not be saved sends the
/// warning to stderr and the run log as before, and a `post.failed` event on
/// the job reaches the owner. A blank error is no failure (see
/// [`result_post_failure_message`]).
struct PostFailures<'a> {
    c: &'a rusqlite::Connection,
    now: &'a str,
    agent: &'a str,
    job_id: String,
    run_id: &'a str,
    log: &'a RunLog,
}

impl PostFailures<'_> {
    fn note(&self, what: &str, err: &anyhow::Error) {
        let err = err_message(err);
        let Some(msg) = result_post_failure_message(&self.job_id, what, &err) else {
            return;
        };
        eprintln!("run {}: {msg}", self.run_id);
        self.log.add(msg);
        crate::util::event_at(
            self.c,
            self.now,
            self.agent,
            "post.failed",
            &self.job_id,
            post_failed_detail(what, &err, self.run_id),
        );
    }
}

/// Warning text for a run log that could not be written: the result post
/// points owners at `log_path`, so a silent failure sends them to a missing
/// or stale file.
pub(crate) fn run_log_write_failure_message(run_id: &str, log_path: &str, err: &str) -> String {
    format!(
        "warning: run {run_id}: could not write log {log_path}, the result post's Log line will be missing or stale: {err}"
    )
}

/// Writes the run log to `log_path`; a failure (full disk, missing directory)
/// is reported on stderr instead of being dropped.
fn write_run_log(run_id: &str, log_path: &str, text: &str) {
    if let Err(e) = std::fs::write(log_path, text) {
        eprintln!(
            "{}",
            run_log_write_failure_message(run_id, log_path, &e.to_string())
        );
    }
}

pub(crate) fn err_message(err: &anyhow::Error) -> String {
    err.to_string()
}

#[cfg(test)]
pub(crate) fn apply_result_decisions(
    s: &hive_store::Store,
    job: Row,
    rep: Report,
    log: &mut RunLog,
) -> i32 {
    apply_result_decisions_ran(s, job, rep, log, &s.run_model(crate::lead::LEAD_KIND))
}

/// The lead's `DECIDE:` lines of a finished run, recorded under the lead's
/// name with `ran_model`, the model the run was started with, and the hub's
/// provider.
pub(crate) fn apply_result_decisions_ran(
    s: &hive_store::Store,
    job: Row,
    rep: Report,
    log: &mut RunLog,
    ran_model: &str,
) -> i32 {
    if rep.decisions.is_empty()
        || is_nil(&job)
        || job.str("kind") != crate::lead::LEAD_KIND
        || job
            .str("source")
            .starts_with(hive_tasks::leads::SOURCE_PREFIX)
    {
        // Whatever owner.presence says: nothing waits on the owner by
        // default, so the lead's DECIDE lines count when the owner is here
        // too; the hard stops are skipped below (TASK-129).
        return 0;
    }
    let slug = job.str("project");
    let lead = crate::lead::lead_name(slug.clone());
    let c = conn(s);
    let mut recorded = 0;
    let mut seen = BTreeMap::<String, bool>::new();
    for d in &rep.decisions {
        if seen.contains_key(&d.post_id) {
            log.add(format!(
                "decision skipped: {} was already named by this report",
                d.post_id
            ));
            continue;
        }
        seen.insert(d.post_id.clone(), true);
        if d.reason.trim().is_empty() {
            log.add(format!("decision skipped: {} has no reason", d.post_id));
            continue;
        }
        let post = crate::util::must_one(
            &c,
            "SELECT id, project, kind, resolution, summary, body, extra FROM posts WHERE id = ?",
            &[&d.post_id],
        );
        if is_nil(&post) {
            log.add(format!("decision skipped: {} does not exist", d.post_id));
            continue;
        }
        if post.str("project") != slug {
            log.add(format!(
                "decision skipped: {} belongs to {}, not {}",
                d.post_id,
                post.str("project"),
                slug
            ));
            continue;
        }
        // An agreed proposal, or a finding, handoff or suggestion addressed
        // to the owner (the hub's "For you"): both are the lead's call,
        // whether the owner is here or away, unless at a hard stop.
        let proposal = post.str("resolution") == "agreed";
        let ask = !proposal
            && crate::lead::OWNER_ASK_DECIDE_KINDS.contains(&post.str("kind").as_str())
            && crate::plan::is_owner_addressed_post(post.clone());
        if !proposal && !ask {
            log.add(format!(
                "decision skipped: {} is neither an agreed proposal nor a finding, handoff or suggestion addressed to the owner",
                d.post_id
            ));
            continue;
        }
        if !crate::util::must_one(
            &c,
            "SELECT 1 AS x FROM decisions WHERE post_id = ?",
            &[&d.post_id],
        )
        .0
        .is_empty()
        {
            log.add(format!(
                "decision skipped: {} was already decided",
                d.post_id
            ));
            continue;
        }
        if !crate::util::must_one(
            &c,
            "SELECT 1 AS x FROM compute_requests WHERE proposal_post = ?",
            &[&d.post_id],
        )
        .0
        .is_empty()
        {
            log.add(format!(
                "decision skipped: {} is a compute proposal",
                d.post_id
            ));
            continue;
        }
        if crate::lead::LEAD_HUMAN_ONLY_MARKER_RE.is_match(&post.str("body")) {
            log.add(format!(
                "decision skipped: {} has a pair/onboard marker",
                d.post_id
            ));
            continue;
        }
        let mut why = hive_board::owner_only_proposal(&post.str("summary"), &post.str("body"));
        if why.is_empty() {
            why = hive_board::hard_stop_ask(
                &post.str("kind"),
                &post.str("summary"),
                &post.str("body"),
            );
        }
        if !why.is_empty() {
            log.add(format!(
                "decision skipped: {} {}: it waits for the owner",
                d.post_id, why
            ));
            continue;
        }
        if proposal {
            if crate::util::must_one(&c, "SELECT resolution FROM posts WHERE project = ? AND area = (SELECT area FROM posts WHERE id = ?) AND thread = (SELECT thread FROM posts WHERE id = ?) AND resolution <> '' ORDER BY id DESC LIMIT 1", &[&slug, &d.post_id, &d.post_id]).str("resolution") != "agreed" { log.add(format!("decision skipped: {} is no longer awaiting", d.post_id)); continue; }
        } else {
            if crate::lead::is_security_major_finding(&c, &d.post_id) {
                log.add(format!(
                    "decision skipped: {} is a major dependency upgrade: the owner's call, or dependencies.major_updates's, never the lead's",
                    d.post_id
                ));
                continue;
            }
            if !crate::lead::owner_ask_is_open(&c, &d.post_id) {
                log.add(format!(
                    "decision skipped: {} was answered or its thread closed",
                    d.post_id
                ));
                continue;
            }
        }
        let by = hive_board::DecidedBy {
            model: ran_model.to_string(),
            ..hive_board::DecidedBy::lead_run(s)
        };
        if let Err(e) = hive_board::decide_with(s, &d.post_id, &d.answer, &d.reason, &lead, by) {
            log.add(format!(
                "decision skipped: {}: {}",
                d.post_id,
                err_message(&e)
            ));
            continue;
        }
        recorded += 1;
    }
    log.add(format!(
        "- Decisions: {recorded} recorded of {} DECIDE: line(s)",
        rep.decisions.len()
    ));
    recorded
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn cast_result_votes(
    s: &hive_store::Store,
    slug: &str,
    arow: Option<hive_store::Agent>,
    agent: &str,
    now: &str,
    run_id: &str,
    votes: Vec<Vote>,
    ballot: Vec<Value>,
    log: &mut RunLog,
) -> i32 {
    let allowed: BTreeMap<String, BallotItem> = ballot
        .into_iter()
        .map(row_to_ballot)
        .map(|it| (it.id.clone(), it))
        .collect();
    let c = conn(s);
    let mut cast = 0;
    let mut seen = BTreeMap::<String, bool>::new();
    for v in votes {
        if seen.contains_key(&v.on) {
            continue;
        }
        seen.insert(v.on.clone(), true);
        let Some(item) = allowed.get(&v.on) else {
            log.add(format!(
                "vote skipped: {} was not on this run's ballot",
                v.on
            ));
            continue;
        };
        if v.reason.trim().is_empty() {
            log.add(format!("vote skipped: {} has no reason", v.on));
            continue;
        }
        if !crate::util::must_one(
            &c,
            "SELECT 1 AS x FROM posts WHERE kind = 'vote' AND agent = ? AND on_post = ? LIMIT 1",
            &[&agent, &v.on],
        )
        .0
        .is_empty()
        {
            log.add(format!("vote skipped: {}: already voted", v.on));
            continue;
        }
        let key = if item.key.is_empty() {
            "evolution/".to_string()
        } else {
            item.key.clone()
        };
        let (area, thread) = key.split_once('/').unwrap_or(("evolution", &v.on));
        let area = if area.is_empty() { "evolution" } else { area };
        let thread = if thread.is_empty() {
            v.on.as_str()
        } else {
            thread
        };
        // An amendment vote counts only when it carries the proposal hash and
        // a `Read:` line; the agent's reason is its restatement of the change.
        let mut body = format!("Cast by the hub from a `VOTE:` line in run `{run_id}`.\n");
        let mut text_hash = String::new();
        if item.is_amendment() {
            body.push_str(&format!("\nRead: {}\n", v.reason.trim()));
            text_hash = item.hash.clone();
        }
        let res = crate::board::create_post(
            s,
            now.to_string(),
            crate::board::PostOpts {
                kind: "vote".into(),
                project: slug.to_string(),
                area: area.into(),
                thread: thread.into(),
                summary: crate::util::trunc(&v.reason, 230),
                body,
                agent: arow.clone().map(Box::new),
                session: run_id.to_string(),
                on: v.on.clone(),
                vote: v.vote.clone(),
                text_hash,
                extra: BTreeMap::from([
                    ("cast_by".into(), json!("hub-from-result")),
                    ("run".into(), json!(run_id)),
                ]),
                ..Default::default()
            },
        );
        if let Err(e) = res {
            log.add(format!("vote skipped: {}: {}", v.on, err_message(&e)));
            continue;
        }
        cast += 1;
    }
    cast
}

/// The worker a job runs as: the `--agent` name when given; else a free
/// active worker - the holder of held work waiting in the queue first, then
/// the one that leased least recently - and when none is free, a fresh name
/// from `spawn_agent`. Workers are not kept in a pool: a worker whose run is
/// over retires (`hive_board::retire_idle_workers`, each daemon tick), so a
/// free one here is a worker between its dispatch and its run, a holder
/// waiting for its held work, or a worker joined by hand.
pub(crate) fn pick_worker(
    s: &hive_store::Store,
    _now: &str,
    agent: &str,
    exclude: &[String],
) -> Result<String> {
    pick_worker_on(s, agent, exclude, "")
}

/// [`pick_worker`] for a slot on `executor` (empty: any machine). A hand's
/// slot passes the holder of work held on this machine, so the holder stays
/// free for the local slot that can carry its clone and conversation on:
/// the hands are offered work before this machine (daemon `slot_order`),
/// and a holder busy with the fleet's work on a hand frees its held job for
/// anyone, who starts it over.
pub(crate) fn pick_worker_on(
    s: &hive_store::Store,
    agent: &str,
    exclude: &[String],
    executor: &str,
) -> Result<String> {
    if !agent.is_empty() {
        return Ok(crate::board::resolve_agent(s, agent.to_string(), true)?
            .map(|a| a.name)
            .unwrap_or_default());
    }
    let runner = hive_store::HARNESS_RUNNER.to_string();
    let model = s.setting("worker_model", "");
    let c = conn(s);
    let mut q = format!(
        "SELECT a.name FROM agents a WHERE a.status = 'active' AND a.kind = 'worker' AND NOT EXISTS (SELECT 1 FROM jobs k WHERE k.lease_agent = a.name AND {}) ",
        crate::jobs::OCCUPIED_JOB_SQL
    );
    let mut vals: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if !exclude.is_empty() {
        q.push_str(&format!(
            "AND a.name NOT IN ({}) ",
            vec!["?"; exclude.len()].join(",")
        ));
        for x in exclude {
            vals.push(Box::new(x.clone()));
        }
    }
    if !executor.is_empty() && executor != crate::executor::LOCAL_EXECUTOR {
        q.push_str(&format!(
            "AND NOT EXISTS (SELECT 1 FROM harness_sessions h JOIN runs r ON r.id = h.run_id JOIN jobs j ON j.id = h.job_id JOIN projects p ON p.slug = j.project WHERE h.agent = a.name AND h.status = 'held' AND j.status = 'queued' AND p.status = 'active' AND r.executor = '{}') ",
            crate::executor::LOCAL_EXECUTOR
        ));
    }
    // A worker with held work waiting in the queue goes first: only it can
    // carry that work on, and nobody else may take it while it is free -
    // unless that work waits behind the owner's queued work in its project
    // (yield_lane), when another worker takes the owner's job and this one
    // stays free to carry its own on. Then the worker that leased least
    // recently.
    q.push_str(&format!(
        "ORDER BY CASE WHEN EXISTS (SELECT 1 FROM harness_sessions h JOIN jobs j ON j.id = h.job_id JOIN projects p ON p.slug = j.project WHERE h.agent = a.name AND h.status = 'held' AND j.status = 'queued' AND p.status = 'active' AND {}) THEN 0 ELSE 1 END, COALESCE((SELECT MAX(e.seq) FROM events e WHERE e.actor = a.name AND e.action = 'job.lease'), 0), a.name LIMIT 1",
        crate::yield_lane::fleet_waits_sql()
    ));
    let refs: Vec<&dyn rusqlite::ToSql> = vals.iter().map(|v| &**v).collect();
    let r = crate::util::must_one(&c, &q, &refs);
    if !is_nil(&r) {
        return Ok(r.str("name"));
    }
    Ok(crate::board::spawn_agent(s, runner, model, "worker".into())?.str("name"))
}

#[allow(dead_code)]
pub(crate) fn executor_slots(
    s: &hive_store::Store,
    now: &str,
    busy: &BTreeMap<String, i32>,
    local_closed: bool,
) -> Vec<String> {
    let mut slots = Vec::new();
    let remotes = crate::executor::executor_rows(s, false);
    // A rented machine whose hours are up takes no new work: the run in
    // flight on it is its last, and the compute worker releases it then.
    let past_hours = crate::compute::rentals_past_hours(s, now);
    for r in &remotes {
        if past_hours.contains(&r.str("name")) {
            continue;
        }
        for _ in 0..crate::executor::executor_free_slots(
            s,
            r,
            now,
            *busy.get(&r.str("name")).unwrap_or(&0),
        ) {
            slots.push(r.str("name"));
        }
    }
    if local_closed {
        return slots;
    }
    let local = crate::executor::local_executor_row(s);
    for _ in 0..crate::executor::executor_free_slots(
        s,
        &local,
        now,
        *busy.get(crate::executor::LOCAL_EXECUTOR).unwrap_or(&0),
    ) {
        slots.push(crate::executor::LOCAL_EXECUTOR.to_string());
    }
    if slots.is_empty()
        && remotes.is_empty()
        && crate::executor::executor_running(s, crate::executor::LOCAL_EXECUTOR, now) == 0
        && *busy.get(crate::executor::LOCAL_EXECUTOR).unwrap_or(&0) == 0
    {
        slots.push(crate::executor::LOCAL_EXECUTOR.to_string());
    }
    slots
}

#[allow(dead_code)]
pub(crate) fn sweep_lost_runs(s: &hive_store::Store, now: &str) -> i32 {
    let c = conn(s);
    let rows = crate::util::must_rows(
        &c,
        "SELECT id, job_id, agent FROM runs WHERE status = 'running'",
        &[],
    );
    for r in &rows {
        crate::util::exec1(
            &c,
            "UPDATE runs SET status = 'lost', finished_at = ?, summary = ? WHERE id = ?",
            &[&now, &LOST_RUN_SUMMARY, &r.str("id")],
        );
        crate::util::event_at(
            &c,
            now,
            "runner",
            "run.lost",
            &r.str("id"),
            Value::String(String::new()),
        );
        release_lost_job(s, now, &r.str("job_id"), &r.str("agent"));
    }
    rows.len() as i32
}

#[allow(dead_code)]
pub(crate) fn release_lost_job(s: &hive_store::Store, now: &str, job_id: &str, agent: &str) {
    let c = conn(s);
    let job = crate::util::must_one(
        &c,
        "SELECT * FROM jobs WHERE id = ? AND status = 'leased' AND lease_agent = ?",
        &[&job_id, &agent],
    );
    if is_nil(&job) {
        return;
    }
    if job.int("attempts") >= MAX_ATTEMPTS {
        let result = format!(
            "failed: the runner stopped while this job was in progress {} times",
            job.int("attempts")
        );
        crate::util::exec1(
            &c,
            "UPDATE jobs SET status = 'failed', lease_agent = '', lease_until = '', finished_at = ?, result = ? WHERE id = ?",
            &[&now, &result, &job_id],
        );
        crate::util::event_at(
            &c,
            now,
            "runner",
            "job.failed",
            job_id,
            Value::String(result),
        );
    } else {
        crate::util::exec1(
            &c,
            "UPDATE jobs SET status = 'queued', lease_agent = '', lease_until = '' WHERE id = ?",
            &[&job_id],
        );
        crate::util::event_at(
            &c,
            now,
            "runner",
            "job.requeue",
            job_id,
            Value::String("run lost: the runner stopped while it was in progress".into()),
        );
    }
}

pub(crate) fn worker_cli_env(env: &mut BTreeMap<String, String>, exe: &str) {
    if exe.is_empty() {
        return;
    }
    env.insert("HIVE_CLI".into(), exe.to_string());
    let path = std::env::var("PATH").unwrap_or_default();
    let p = cli_on_path(&path, exe);
    if !p.is_empty() {
        env.insert("PATH".into(), p);
    }
}

pub(crate) fn cli_on_path(path_env: &str, exe: &str) -> String {
    if exe.is_empty() {
        return String::new();
    }
    let name = Path::new(exe)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    for dir in std::env::split_paths(path_env) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let cand = dir.join(&name);
        if let Ok(st) = std::fs::metadata(&cand)
            && st.is_file()
        {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if st.permissions().mode() & 0o111 != 0 {
                    return String::new();
                }
            }
            #[cfg(not(unix))]
            {
                return String::new();
            }
        }
    }
    let dir = Path::new(exe)
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    if path_env.is_empty() {
        dir
    } else {
        format!(
            "{}{}{}",
            dir,
            if cfg!(windows) { ";" } else { ":" },
            path_env
        )
    }
}

/// A local `hive-hand` pushes the commits the identity check accepted,
/// from its own clone, with the push credential the brain leases it for
/// that one push (ADR-070 step 1b): `push.credential` resolved here and
/// handed over on the verb's stdin, never written into the run's env. A
/// brain without a checkout of the project (`brain.code none`) has no git
/// remote to read, so the upstream is the url the project was registered
/// with (`projects.remote`) and the branch reaches the forge, where its
/// pull request lands it. A project without an upstream lands locally and
/// nothing is pushed; a push
/// that fails is left to the delivery pass, which pushes the same branch
/// from the checkout. Returns the log line.
#[allow(clippy::too_many_arguments)]
fn push_from_hand(
    s: &hive_store::Store,
    ex: &dyn crate::exec::Executor,
    project: &Row,
    root: &str,
    branch: &str,
    holding: &str,
    agent: &str,
    now: &str,
) -> String {
    let def = crate::baseline::default_branch(project.clone());
    let Some(up) =
        crate::forge::upstream_of(root, &def).or_else(|| registered_upstream(project, root, &def))
    else {
        return "hand push skipped: the project has no upstream; it lands locally".into();
    };
    let sha = crate::util::git(root, &["rev-parse", "--verify", "-q", holding])
        .stdout
        .trim()
        .to_string();
    // A GitHub App installation token for this one repository is leased
    // first (ADR-070 step 3): the hand pushes it over https through a
    // credential helper beside its clone; else `push.credential`'s key.
    let mut via = "the leased push credential";
    let mut url = up.url.clone();
    let mut key = Vec::new();
    match crate::gitapp::token_for_url(s, &up.url) {
        Some(Ok((repo, token))) => {
            key = hive_hand_core::gitcred::token_lease(token.secret());
            url = crate::gitapp::https_url(&repo);
            via = "the leased GitHub App token";
        }
        Some(Err(why)) => {
            let _ = s.event(
                agent,
                "gitapp.fallback",
                branch,
                json!({"why": hive_cred::redact(&crate::executor::redact_tokens(&why), &[])}),
            );
        }
        None => {}
    }
    if key.is_empty() {
        key = match crate::forge::push_credential_key(s) {
            Ok(k) => k,
            Err(why) => {
                return format!(
                    "hand push skipped: push.credential could not be read ({why}); the delivery pass pushes {branch}"
                );
            }
        };
    }
    let leased = !key.is_empty();
    let (line, ok) = match ex.push_from_hand(&url, &sha, branch, &key) {
        Ok(out) => (
            format!(
                "{} pushed {branch} to {}{}: {}",
                ex.name(),
                up.name(),
                if leased {
                    format!(" with {via}")
                } else {
                    String::new()
                },
                crate::executor::redact_tokens(&out)
            ),
            true,
        ),
        Err(e) => (
            format!(
                "hand push failed, the delivery pass pushes {branch}: {}",
                crate::executor::redact_tokens(&e.to_string())
            ),
            false,
        ),
    };
    if let Ok(c) = s.conn() {
        crate::util::event_at(
            &c,
            now,
            agent,
            if ok { "hand.push" } else { "hand.push_failed" },
            branch,
            json!({"executor": ex.name(), "sha": sha, "remote": up.name(), "leased": leased, "via": if leased { via } else { "" }}),
        );
    }
    line
}

/// The upstream a brain with no checkout of the project pushes to: the
/// project's registered remote url on its default branch. None while the
/// project path is a checkout (its own remotes decide) or no remote was
/// registered.
fn registered_upstream(project: &Row, root: &str, def: &str) -> Option<crate::forge::Upstream> {
    if Path::new(root).join(".git").exists() {
        return None;
    }
    let url = project.str("remote").trim().to_string();
    if url.is_empty() {
        return None;
    }
    Some(crate::forge::Upstream {
        remote: "origin".into(),
        remote_branch: def.to_string(),
        upstream_ref: format!("refs/remotes/origin/{def}"),
        repo: crate::landing_pr::repo_slug(&url).unwrap_or_default(),
        url,
    })
}

pub(crate) fn remote_hive_cli(recipe: Option<&crate::recipe::Recipe>) -> String {
    if let Some(r) = recipe {
        let cli = crate::recipe::recipe_hive_cli(r);
        if !cli.is_empty() {
            return cli;
        }
    }
    if let Some(f) = *STAGED_HIVE_PATH.read().expect("staged hive path") {
        return f();
    }
    crate::pair::PAIR_HIVE_PATH.to_string()
}

type StagedHivePathFn = Option<fn() -> String>;

pub(crate) static STAGED_HIVE_PATH: LazyLock<RwLock<StagedHivePathFn>> =
    LazyLock::new(|| RwLock::new(None));

/// The key under which this project's verify memos are written and found,
/// whoever verified: the check as the pre-commit hook runs it, inside the
/// verify lock (`install_pre_commit_hook` is given exactly this string), and
/// the key a landing uses too (`land.rs`, `vc_run`). Recipe exports and tool
/// pins wrap the command that runs but never the key, so a memo written by
/// one verifier is good for every other.
fn verify_memo_key(s: &hive_store::Store, project: &Row) -> String {
    crate::verify_lock::wrap(s, &project.str("slug"), &project.str("verify_cmd"))
}

/// The tree `rev` names in the clone and its fresh verify memo, when this
/// user already passed this project's check on that very tree on this store
/// (`landing.verify_memo`, `landing.verify_memo_hours`). Only a local
/// executor qualifies: a remote executor's hook writes its memo on the
/// remote store, where this daemon cannot read it.
fn verified_tree(
    s: &hive_store::Store,
    project: &Row,
    ex: &dyn Executor,
    clone: &str,
    rev: &str,
) -> Option<(String, crate::verify_memo::VerifyMemo)> {
    if ex.remote() || project.str("verify_cmd").is_empty() {
        return None;
    }
    let tree = crate::land::git_tree(clone, rev);
    if tree.is_empty() {
        return None;
    }
    let key = verify_memo_key(s, project);
    let memo = crate::land::find_verify_memo(s, &project.str("slug"), &tree, &key, &s.now())?;
    Some((tree, memo))
}

/// The clone's committed tree when the pre-commit hook's memo says it already
/// passed this project's check: the run's own check then has nothing to add,
/// since the hook ran the identical command on the identical tree moments
/// before, and it is skipped. Only a clean clone qualifies - uncommitted
/// tracked changes mean the check would judge a tree the hook did not. The
/// measured reason: on the brain, 42 run checks in the three days to
/// 2026-11-04 ran 720 minutes and waited 462 for the project's verify lock,
/// while the hook had written 46 memos for those trees in the same days -
/// twenty hours of lock time, a third of the project's, spent passing the
/// same trees twice.
fn hook_verified_tree(
    s: &hive_store::Store,
    project: &Row,
    ex: &dyn Executor,
    clone: &str,
) -> Option<(String, crate::verify_memo::VerifyMemo)> {
    if !ex.dirty().is_empty() {
        return None;
    }
    verified_tree(s, project, ex, clone, "HEAD")
}

/// Leaves the memo a passed check earns: `rev`'s tree in a local clone, keyed
/// as every other verifier keys it, so the next run, landing or base check on
/// the same tree can skip. A remote executor's memo belongs on the remote
/// store and is not written here.
fn record_verified_tree(
    s: &hive_store::Store,
    project: &Row,
    ex: &dyn Executor,
    clone: &str,
    rev: &str,
    by: &str,
    agent: &str,
) {
    if ex.remote() || project.str("verify_cmd").is_empty() {
        return;
    }
    let tree = crate::land::git_tree(clone, rev);
    if tree.is_empty() {
        return;
    }
    let _ = crate::verify_memo::write(
        &s.home,
        &project.str("slug"),
        &tree,
        &verify_memo_key(s, project),
        by,
        agent,
        clone,
    );
}

/// Fetches the run's `HEAD` from the executor into `refspec_dst` of the
/// owner's checkout at `root`: from [`Executor::fetch_source`] first, and
/// when that fails from [`Executor::fetch_fallback`] - for an ssh machine a
/// bundle its `bundle` verb made, which reaches the commits on a hand whose
/// `git-upload-pack` does not answer (every hand before TASK-133: 34 runs on
/// rented machines failed "Could not read from remote repository" between
/// 2026-11-01 and 2026-11-26 while their bundle verb answered). A fetch
/// that failed both ways is written down as an `executor.fetch-failed` event
/// against the executor (the lease keeps code-changing work off it until it
/// fetches again); returns the last attempt's outcome.
pub(crate) fn fetch_run_work(
    s: &hive_store::Store,
    cancel: &hive_bus::CancellationToken,
    ex: &dyn Executor,
    job: &str,
    root: &str,
    dst: &str,
    log: &crate::exec::RunLog,
) -> Result<crate::proc::Outcome> {
    let fetch = |source: String, env: &[String]| {
        crate::proc::run_group(
            cancel,
            &[
                "git".into(),
                "-C".into(),
                root.to_string(),
                "fetch".into(),
                "-q".into(),
                "--no-tags".into(),
                source,
                format!("+HEAD:{dst}"),
            ],
            "",
            env,
            &[],
            Duration::from_secs(300),
        )
    };
    let fetched = fetch(ex.fetch_source(cancel), &ex.fetch_env())?;
    log.add(format!(
        "$ git fetch <{} clone> HEAD:{}\n{}",
        ex.name(),
        dst,
        fetched.stderr
    ));
    if fetched.exit_code == 0 {
        crate::executor::note_fetch_ok(s, &ex.name(), &s.now(), job);
        return Ok(fetched);
    }
    if cancel.is_cancelled() {
        return Ok(fetched);
    }
    let bundle = ex.fetch_fallback(cancel);
    if bundle.is_empty() {
        return Ok(fetched);
    }
    let again = fetch(bundle, &crate::util::git_env(&[]))?;
    log.add(format!(
        "$ git fetch <{} bundle> HEAD:{}\n{}",
        ex.name(),
        dst,
        again.stderr
    ));
    if again.exit_code == 0 {
        log.add(format!(
            "the fetch from {} failed ({}); its commits came back as a bundle instead",
            ex.name(),
            crate::util::trunc(fetched.stderr.trim(), 200)
        ));
        crate::executor::note_fetch_ok(s, &ex.name(), &s.now(), job);
        return Ok(again);
    }
    Ok(fetched)
}

/// `base_tree_verify` behind the verify memo: a base tree this user already
/// passed is green without the check, and a base the check passes here is
/// memoed for the next run that fails on it. The measured reason: the three
/// brain runs that failed their check on 2026-11-04 (fjord-7, ember-7,
/// nimbus-7) each re-checked the same main tree `6cd2724e`, fifteen to
/// twenty-five minutes of the project's verify lock apiece, to learn the same
/// thing.
#[allow(clippy::too_many_arguments)]
fn base_tree_verdict(
    s: &hive_store::Store,
    project: &Row,
    ex: &dyn Executor,
    cancel: &hive_bus::CancellationToken,
    clone: &str,
    base: &str,
    check_cmd: &str,
    minutes: i32,
    log: &RunLog,
    why: &str,
    agent: &str,
) -> BaseVerdict {
    if let Some((tree, memo)) = verified_tree(s, project, ex, clone, base) {
        log.add(format!(
            "$ verify the base tree {} ({why})\nhive: base verify skipped: tree {tree} already passed this check on this store ({}); the base is green",
            crate::recipe::short(base, 12),
            crate::verify_memo::note(&memo, &tree)
        ));
        return BaseVerdict::Green;
    }
    let verdict = base_tree_verify(ex, cancel, base, check_cmd, minutes, log, why);
    if matches!(verdict, BaseVerdict::Green) {
        record_verified_tree(s, project, ex, clone, base, "base", agent);
    }
    verdict
}

/// On the failure path only: re-run the project's check against the base tree
/// (the commit the job branched from) in a throwaway worktree beside the clone.
/// Returns `Some(exit)` when the base itself fails verify, so the job's commits
/// are not blamed for a regression they did not introduce; `None` when the base
/// is green, the check timed out, or the worktree could not be made.
/// The project's verify command as a run's checks execute it: the recipe's
/// env exports first, under the recipe's tool pins when it has any, inside
/// the verify lock and disk preludes.
pub(crate) fn wrapped_check(
    s: &hive_store::Store,
    project: &Row,
    recipe: Option<&crate::recipe::Recipe>,
    pins: &[String],
    clone: &str,
) -> String {
    let mut cmd = recipe
        .map(crate::recipe::recipe_env_exports)
        .unwrap_or_default()
        + &project.str("verify_cmd");
    if !pins.is_empty() {
        cmd = crate::recipe::mise_wrap_shell(pins, &cmd, clone);
    }
    if check_shares_target_dir(project, recipe, clone) {
        // The run-end check runs in the clone and the base check in a
        // worktree beside it; the line is relative to wherever it runs.
        cmd = crate::exec::freshen_before_check(&cmd, "the tree's");
    }
    crate::verify_lock::wrap(s, &project.str("slug"), &cmd)
}

/// True when the project's check builds into a Cargo target dir outside the
/// clone - the recipe's `CARGO_TARGET_DIR` or the one the verify command
/// names - and so shares it with every other clone and landing of the
/// project (`exec::check_shares_cargo_target_dir`).
fn check_shares_target_dir(
    project: &Row,
    recipe: Option<&crate::recipe::Recipe>,
    clone: &str,
) -> bool {
    crate::exec::check_shares_cargo_target_dir(
        recipe
            .and_then(|r| r.env.get("CARGO_TARGET_DIR"))
            .map(String::as_str),
        &project.str("verify_cmd"),
        clone,
    )
}

/// The bare verify command with the shared-target-dir touch in front of it
/// when the check needs one (`exec::freshen_before_check`), for the hook,
/// which wraps the recipe's exports and pins around the check itself.
fn check_with_freshen(
    project: &Row,
    recipe: Option<&crate::recipe::Recipe>,
    clone: &str,
    whose: &str,
) -> String {
    let vc = project.str("verify_cmd");
    if check_shares_target_dir(project, recipe, clone) {
        crate::exec::freshen_before_check(&vc, whose)
    } else {
        vc
    }
}

/// The shell line that runs `check` on a detached worktree of `base` at `wt`
/// (a path relative to the clone) and removes the worktree again whatever
/// the check did. One shell string, so it runs the same way on a local or an
/// ssh executor. Exit 125 marks a worktree that could not be made or entered,
/// which says nothing about the base. The `cd` guards the whole check, not
/// only its first command: a wrapped check is several lines (the disk and
/// lock preludes), and with `cd wt && check` a worktree that vanished before
/// the `cd` (the clone moved to the trash mid-run) let the rest of the check
/// run in the job's own clone and called the base red for it.
pub(crate) fn base_worktree_command(wt: &str, base: &str, check: &str) -> String {
    let q = crate::util::shell_quote;
    format!(
        "git worktree remove --force {wt} >/dev/null 2>&1; git worktree add -q --detach {wt} {b} || exit 125; (cd {wt} || exit 125\n{check}\n); rc=$?; git worktree remove --force {wt} >/dev/null 2>&1; git worktree prune >/dev/null 2>&1; exit $rc",
        wt = q(wt),
        b = q(base),
    )
}

/// What a check of the base tree itself found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BaseVerdict {
    /// The check passes on the base: a failure on top of it is the job's own.
    Green,
    /// The check fails on the base with this exit code and output tail.
    Red { rc: i32, tail: String },
    /// The check did not run to a verdict - no worktree, a timeout, the disk
    /// under `daemon.min_free_mb`, the verify lock given up - so the base's
    /// state is unknown and nothing is charged or recorded for it.
    Unknown,
}

/// Runs the project's check on the base tree itself, in a detached worktree
/// beside the clone; `why` is the log line's reason for looking.
pub(crate) fn base_tree_verify(
    ex: &dyn Executor,
    cancel: &hive_bus::CancellationToken,
    base: &str,
    check_cmd: &str,
    minutes: i32,
    log: &RunLog,
    why: &str,
) -> BaseVerdict {
    if base.trim().is_empty() || check_cmd.trim().is_empty() {
        return BaseVerdict::Unknown;
    }
    let wt = format!("../.hive-base-{}", crate::recipe::short(base, 12));
    let cmd = base_worktree_command(&wt, base, check_cmd);
    log.add(format!(
        "$ verify the base tree {} ({why})",
        crate::recipe::short(base, 12)
    ));
    let (rc, out) = ex.verify(cancel, &cmd, minutes);
    match rc {
        Some(0) => {
            log.add("base tree verify exit 0: the base is green");
            BaseVerdict::Green
        }
        Some(125) => {
            log.add(format!(
                "could not check out or enter the base tree; {}",
                tail(&out, 2000)
            ));
            BaseVerdict::Unknown
        }
        Some(code) if crate::verify_lock::did_not_run(code) => {
            log.add(format!(
                "base tree verify exit {code}: the check did not run ({}); the base's state is unknown\n{}",
                crate::verify_lock::did_not_run_reason(code),
                tail(&out, 2000)
            ));
            BaseVerdict::Unknown
        }
        Some(code) => {
            log.add(format!(
                "base tree verify exit {code}: the base was already red\n{}",
                tail(&out, 20000)
            ));
            BaseVerdict::Red {
                rc: code,
                tail: tail(&out, 4000),
            }
        }
        None => {
            log.add("base tree verify timed out; the base's state is unknown");
            BaseVerdict::Unknown
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn result_post_failure_message_names_job_and_error() {
        let m = super::result_post_failure_message("job-1", "result", "disk I/O error")
            .expect("a real error must produce a warning");
        assert!(m.starts_with("warning: "), "{m}");
        assert!(m.contains("job job-1"), "{m}");
        assert!(m.contains("result post failed"), "{m}");
        assert!(m.ends_with("disk I/O error"), "{m}");
        let a = super::result_post_failure_message("job-2", "answer", "no question")
            .expect("a real error must produce a warning");
        assert!(
            a.contains("job job-2") && a.contains("answer post failed"),
            "{a}"
        );
    }

    /// An empty error message means nothing failed, so no warning may
    /// claim the result post failed on stderr.
    #[test]
    fn result_post_failure_message_is_not_built_for_an_empty_error() {
        let m = super::result_post_failure_message("job-3", "result", "");
        assert!(m.is_none(), "never built for an empty error message: {m:?}");
    }

    /// A whitespace-only error message is no error text either, so it must
    /// never surface as a warning that ends in nothing.
    #[test]
    fn result_post_failure_message_is_not_built_for_a_blank_error() {
        let m = super::result_post_failure_message("job-3", "result", "  \n");
        assert!(m.is_none(), "never built for a blank error message: {m:?}");
    }

    /// A real error must surface as a warning that names the job, the post
    /// kind and the non-empty error text, so the owner can act on it.
    #[test]
    fn result_post_failure_message_is_built_for_a_real_error() {
        let m = super::result_post_failure_message("job-3", "result", "connection refused");
        let m = m.expect("a real error must produce a warning");
        assert!(m.starts_with("warning: "), "{m}");
        assert!(m.contains("job-3"), "{m}");
        assert!(m.contains("result post failed"), "{m}");
        assert!(m.ends_with("connection refused"), "{m}");
    }

    /// The question/lesson/finding/suggestion post-failure paths use the same
    /// gate as the result/answer paths: a real error names the post kind, and
    /// an empty or blank error never becomes a "post skipped" claim.
    #[test]
    fn result_post_failure_message_covers_every_post_kind() {
        for what in ["question", "lesson", "shared finding", "suggestion"] {
            let m = super::result_post_failure_message("job-4", what, "disk I/O error")
                .unwrap_or_else(|| panic!("a real error must produce a warning for {what}"));
            assert!(m.starts_with("warning: "), "{what}: {m}");
            assert!(m.contains("job-4"), "{what}: {m}");
            assert!(m.contains(&format!("{what} post failed")), "{what}: {m}");
            assert!(m.ends_with("disk I/O error"), "{what}: {m}");
            assert!(
                super::result_post_failure_message("job-4", what, "").is_none(),
                "{what} must not warn on an empty error"
            );
            assert!(
                super::result_post_failure_message("job-4", what, "  \n").is_none(),
                "{what} must not warn on a blank error"
            );
        }
    }

    #[test]
    fn run_log_write_failure_message_names_run_path_and_error() {
        let m = super::run_log_write_failure_message("run-1", "/logs/run-1.log", "No space left");
        assert!(m.starts_with("warning: run run-1"), "{m}");
        assert!(m.contains("/logs/run-1.log"), "{m}");
        assert!(m.ends_with("No space left"), "{m}");
    }

    /// A lost post leaves a `post.failed` event on the job naming the post
    /// kind, the error and the run; a blank error leaves nothing, like the
    /// stderr warning.
    #[test]
    fn note_post_failure_records_an_event_on_the_job() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        let c = s.conn().expect("conn");
        let log = RunLog::default();
        let err = anyhow::anyhow!("summary is 600 chars; limit 240 - move detail to --body");
        let failures = super::PostFailures {
            c: &c,
            now: TEST_NOW,
            agent: "aurora",
            job_id: "job-5".into(),
            run_id: "run-5",
            log: &log,
        };
        failures.note("suggestion", &err);
        let events = crate::util::must_rows(
            &c,
            "SELECT at, actor, subject, detail FROM events WHERE action = 'post.failed'",
            &[],
        );
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].str("at"), TEST_NOW);
        assert_eq!(events[0].str("actor"), "aurora");
        assert_eq!(events[0].str("subject"), "job-5");
        let detail: Value = serde_json::from_str(&events[0].str("detail")).expect("json detail");
        assert_eq!(
            detail,
            json!({
                "kind": "suggestion",
                "error": "summary is 600 chars; limit 240 - move detail to --body",
                "run": "run-5",
            })
        );
        assert!(
            log.text().contains("suggestion post failed"),
            "the run log keeps the warning: {}",
            log.text()
        );
        failures.note("result", &anyhow::anyhow!("  \n"));
        let n = crate::util::must_one(
            &c,
            "SELECT COUNT(*) AS n FROM events WHERE action = 'post.failed'",
            &[],
        )
        .int("n");
        assert_eq!(n, 1, "a blank error is no failure");
    }

    /// A suggestion card whose first line runs past the board's summary
    /// limit posts with a clipped summary and the full text in the body,
    /// instead of failing with "summary is N chars; limit 240".
    #[test]
    fn suggestion_card_with_a_long_first_line_posts_clipped() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        new_repo(&s);
        join(&s, "aurora");
        let agent = s.agent("aurora").expect("agent").map(Box::new);
        let first = (0..100)
            .map(|i| format!("word{i:02}"))
            .collect::<Vec<_>>()
            .join(" ");
        let (card, sum, ok) =
            crate::brief::card_body(format!("{first}; File: README.md; Variable: intro length"));
        assert!(ok);
        assert!(sum.ends_with("..."), "{sum}");
        assert!(sum.chars().count() <= hive_board::MAX_SUMMARY_CHARS);
        let post = crate::board::create_post(
            &s,
            TEST_NOW.into(),
            crate::board::PostOpts {
                kind: "suggestion".into(),
                project: "proj".into(),
                area: "evolution".into(),
                thread: "job-6".into(),
                summary: sum.clone(),
                body: format!("One-variable improvement.\n\n{card}\n"),
                agent,
                session: "run-6".into(),
                ..Default::default()
            },
        )
        .expect("the clipped summary passes the board's limit");
        assert_eq!(post.str("summary"), sum);
        assert!(
            post.str("body").contains(&first),
            "the body keeps the full first line"
        );
    }

    use super::*;
    use crate::test_helpers::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[test]
    fn cli_on_path_prepends_when_needed() {
        let _serial = crate::test_helpers::serial();
        let exe = "/opt/hive/bin/hive";
        assert_eq!(cli_on_path("", exe), "/opt/hive/bin");
        let got = cli_on_path("/usr/bin:/bin", exe);
        assert_eq!(got, "/opt/hive/bin:/usr/bin:/bin");
    }

    /// Records the budget each `verify` call gets; setup is its only caller
    /// here, so the recorded minutes are the setup budgets.
    struct BudgetExec(std::cell::RefCell<Vec<i32>>);
    impl Executor for BudgetExec {
        fn name(&self) -> String {
            "local".into()
        }
        fn remote(&self) -> bool {
            false
        }
        fn clone(&self) -> String {
            String::new()
        }
        fn key_path(&self) -> String {
            String::new()
        }
        fn started(&self) -> bool {
            false
        }
        fn worker_dirs(&self) -> Vec<String> {
            Vec::new()
        }
        fn prepare(&mut self, _: &hive_bus::CancellationToken) -> Result<String> {
            unreachable!()
        }
        fn run_worker(
            &mut self,
            _: &hive_bus::CancellationToken,
            _: &[String],
            _: &BTreeMap<String, String>,
            _: i32,
        ) -> Result<crate::proc::Outcome> {
            unreachable!()
        }
        fn result_text(&self) -> Option<String> {
            None
        }
        fn commit_count(&self, _: &str) -> i32 {
            0
        }
        fn dirty(&self) -> String {
            String::new()
        }
        fn owner_paths_touched(&self, _: &str) -> Vec<String> {
            Vec::new()
        }
        fn verify(
            &self,
            _: &hive_bus::CancellationToken,
            _: &str,
            minutes: i32,
        ) -> (Option<i32>, String) {
            self.0.borrow_mut().push(minutes);
            (Some(0), String::new())
        }
        fn install_pre_commit_hook(
            &self,
            _: &str,
            _: &str,
            _: Option<&crate::recipe::Recipe>,
            _: &[String],
            _: &str,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<()> {
            Ok(())
        }
        fn fetch_source(&self, _: &hive_bus::CancellationToken) -> String {
            String::new()
        }
        fn fetch_env(&self) -> Vec<String> {
            Vec::new()
        }
        fn cleanup(&self) -> Vec<crate::tidy::Leftover> {
            Vec::new()
        }
    }

    /// A local `hive-hand` that records the url each push goes to.
    struct PushExec(std::cell::RefCell<Vec<String>>);
    impl Executor for PushExec {
        fn name(&self) -> String {
            "hands".into()
        }
        fn remote(&self) -> bool {
            false
        }
        fn clone(&self) -> String {
            String::new()
        }
        fn key_path(&self) -> String {
            String::new()
        }
        fn started(&self) -> bool {
            false
        }
        fn worker_dirs(&self) -> Vec<String> {
            Vec::new()
        }
        fn prepare(&mut self, _: &hive_bus::CancellationToken) -> Result<String> {
            unreachable!()
        }
        fn run_worker(
            &mut self,
            _: &hive_bus::CancellationToken,
            _: &[String],
            _: &BTreeMap<String, String>,
            _: i32,
        ) -> Result<crate::proc::Outcome> {
            unreachable!()
        }
        fn result_text(&self) -> Option<String> {
            None
        }
        fn commit_count(&self, _: &str) -> i32 {
            0
        }
        fn dirty(&self) -> String {
            String::new()
        }
        fn owner_paths_touched(&self, _: &str) -> Vec<String> {
            Vec::new()
        }
        fn verify(
            &self,
            _: &hive_bus::CancellationToken,
            _: &str,
            _: i32,
        ) -> (Option<i32>, String) {
            (Some(0), String::new())
        }
        fn install_pre_commit_hook(
            &self,
            _: &str,
            _: &str,
            _: Option<&crate::recipe::Recipe>,
            _: &[String],
            _: &str,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<()> {
            Ok(())
        }
        fn fetch_source(&self, _: &hive_bus::CancellationToken) -> String {
            String::new()
        }
        fn fetch_env(&self) -> Vec<String> {
            Vec::new()
        }
        fn pushes(&self) -> bool {
            true
        }
        fn push_from_hand(&self, url: &str, _: &str, head: &str, _: &[u8]) -> Result<String> {
            self.0.borrow_mut().push(url.to_string());
            Ok(format!("pushed {head}"))
        }
        fn cleanup(&self) -> Vec<crate::tidy::Leftover> {
            Vec::new()
        }
    }

    /// `brain.code none`: the project path holds no checkout, so the hand
    /// pushes its finished branch to the url the project was registered
    /// with, and the branch reaches the forge for its pull request; with no
    /// registered url it still lands locally.
    #[test]
    fn hand_push_on_a_codeless_brain_goes_to_the_registered_remote() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        new_bare_repo(&s);
        s.set_setting(crate::util::HUMAN_AGENT, "brain.code", "none")
            .unwrap();
        let gone = tempfile::tempdir().unwrap();
        let root = gone.path().join("no-checkout").display().to_string();
        let url = "git@github.com:acme/widgets.git";
        s.conn()
            .unwrap()
            .execute(
                "UPDATE projects SET path = ?1, remote = ?2 WHERE slug = 'proj'",
                rusqlite::params![root, url],
            )
            .unwrap();
        let project = crate::board::project_by_slug(&s.conn().unwrap(), "proj".into());
        let ex = PushExec(Default::default());
        let line = push_from_hand(
            &s,
            &ex,
            &project,
            &root,
            "hive/aurora/job-1",
            "refs/hive/landing/job-1",
            "aurora",
            TEST_NOW,
        );
        assert_eq!(*ex.0.borrow(), vec![url.to_string()], "{line}");
        assert!(
            line.contains("hands pushed hive/aurora/job-1 to origin/"),
            "{line}"
        );
        // No registered url: nothing to push to, the branch lands locally.
        s.conn()
            .unwrap()
            .execute("UPDATE projects SET remote = '' WHERE slug = 'proj'", [])
            .unwrap();
        let project = crate::board::project_by_slug(&s.conn().unwrap(), "proj".into());
        let ex = PushExec(Default::default());
        let line = push_from_hand(
            &s,
            &ex,
            &project,
            &root,
            "hive/aurora/job-2",
            "refs/hive/landing/job-2",
            "aurora",
            TEST_NOW,
        );
        assert!(ex.0.borrow().is_empty(), "{line}");
        assert!(line.contains("no upstream"), "{line}");
    }

    #[test]
    fn setup_gets_ten_minutes_whatever_minutes_per_job_says() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        let (proj, root) = new_bare_repo(&s);
        s.set_setting(crate::util::HUMAN_AGENT, "minutes_per_job", "120")
            .unwrap();
        crate::env::env_set(
            &s,
            proj.clone(),
            recipe_with(Some(&["node@22.1.0"]), Some(&["true"]), None, None, ""),
            crate::util::HUMAN_AGENT.into(),
            "store".into(),
            "".into(),
        )
        .unwrap();
        let ex = BudgetExec(Default::default());
        let mut log = RunLog::default();
        let cancel = hive_bus::CancellationToken::new();
        let prepared = prepare_setup(
            &s,
            &proj,
            &root,
            "HEAD",
            "/nonexistent/clone",
            &ex,
            &cancel,
            false,
            false,
            &mut log,
        )
        .unwrap();
        assert_eq!(prepared.pins, vec!["node@22.1.0".to_string()]);
        // `mise install` and the recipe's setup lines each got the fixed budget.
        assert_eq!(
            *ex.0.borrow(),
            vec![crate::recipe::SETUP_MINUTES, crate::recipe::SETUP_MINUTES],
            "{}",
            log.text()
        );
    }

    #[test]
    fn run_job_gives_the_worker_an_artifacts_folder_that_outlives_the_clone_and_names_it() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        new_repo(&s);
        join(&s, "aurora");
        let (job, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "evidence".into(),
            "change a screen and show it".into(),
            1,
            0,
            "test".into(),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let job = crate::jobs::lease_job(
            &s,
            "aurora".into(),
            TEST_NOW.into(),
            crate::jobs::LeaseOpts {
                job_id: job.str("id"),
                ..Default::default()
            },
        )
        .unwrap();
        let job_id = job.str("id");
        let want = hive_harness::artifacts::job_artifacts_dir(&s.home, &job_id);
        // The worker changes a page, leaves a screenshot where the brief
        // says, and never commits it.
        let worker = "test -d \"$HIVE_ARTIFACTS\" && printf '%s\\n' \"$HIVE_ARTIFACTS\" > \"$HIVE_ARTIFACTS/where.txt\" \
             && mkdir -p web/src && printf '<p/>\\n' > web/src/page.tsx && git add web/src/page.tsx && git commit -q -m page \
             && printf 'png\\n' > \"$HIVE_ARTIFACTS/after.png\" && echo 'SUMMARY: page changed, after.png shows it'";
        let r = run_job(
            &s,
            job,
            "aurora",
            TEST_NOW,
            RunOpts {
                worker_cmd: format!("sh -c {}", crate::util::shell_quote(worker)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(r.str("status"), "done", "run: {r:?}");
        // The brief named the folder the environment carried.
        let c = s.conn().unwrap();
        let brief =
            crate::util::must_one(&c, "SELECT text FROM briefs WHERE job_id = ?", &[&job_id])
                .str("text");
        assert!(
            brief.contains(&format!("$HIVE_ARTIFACTS ({})", want.display())),
            "{brief}"
        );
        assert_eq!(
            std::fs::read_to_string(want.join("where.txt"))
                .unwrap()
                .trim(),
            want.display().to_string()
        );
        // The clone is gone; the evidence stays for the hub.
        crate::retention::empty_trash(std::path::Path::new(&s.home));
        assert!(
            !std::path::Path::new(&s.home)
                .join("work")
                .join(&job_id)
                .exists()
        );
        assert!(want.join("after.png").is_file());
        let mut names = hive_harness::artifacts::names(&want);
        names.sort();
        assert_eq!(
            names,
            vec!["after.png".to_string(), "where.txt".to_string()]
        );
        // The result post names what there is to look at.
        let body = crate::util::must_one(
            &c,
            "SELECT body FROM posts WHERE project = 'proj' AND kind = 'result' AND agent = 'aurora' ORDER BY created_at DESC LIMIT 1",
            &[],
        )
        .str("body");
        assert!(
            body.contains("- Evidence: ")
                && body.contains("`after.png`")
                && body.contains("`where.txt`"),
            "{body}"
        );
        assert!(body.contains(&want.display().to_string()), "{body}");
    }

    #[test]
    fn run_job_result_says_when_a_ui_change_left_no_evidence() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        new_repo(&s);
        join(&s, "aurora");
        let (job, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "quiet ui".into(),
            "change a screen".into(),
            1,
            0,
            "test".into(),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let job = crate::jobs::lease_job(
            &s,
            "aurora".into(),
            TEST_NOW.into(),
            crate::jobs::LeaseOpts {
                job_id: job.str("id"),
                ..Default::default()
            },
        )
        .unwrap();
        let worker = "mkdir -p web/src && printf 'body{}\\n' > web/src/app.css && git add web/src/app.css && git commit -q -m css && echo 'SUMMARY: css'";
        let r = run_job(
            &s,
            job,
            "aurora",
            TEST_NOW,
            RunOpts {
                worker_cmd: format!("sh -c {}", crate::util::shell_quote(worker)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(r.str("status"), "done", "run: {r:?}");
        let body = crate::util::must_one(
            &s.conn().unwrap(),
            "SELECT body FROM posts WHERE project = 'proj' AND kind = 'result' AND agent = 'aurora' ORDER BY created_at DESC LIMIT 1",
            &[],
        )
        .str("body");
        // A custom worker keeps no session to carry on, so it gets no report
        // turn; the post says the change has no report and what that costs.
        assert!(
            body.contains("- Report: none - 1 UI file changed and there is no report.json in $HIVE_ARTIFACTS (rule 10); the landing waits for one (landing.ui_report)"),
            "{body}"
        );
        assert!(!body.contains("- Evidence:"), "{body}");
    }

    /// The pre-commit hook verified the committed tree and left its memo: the
    /// run's own check, the identical command on the identical tree, is
    /// skipped. A clone the worker left with uncommitted edits is checked as
    /// before, since the hook never saw that tree. A custom worker gets no
    /// hook from the run, so the worker installs the run's own hook script
    /// itself before it commits.
    #[test]
    fn run_job_skips_the_check_the_hook_just_passed_on_the_same_tree() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        let (proj, _) = new_repo(&s);
        join(&s, "aurora");
        let count_path = std::path::Path::new(&s.home).join("checks");
        let vc = format!(
            "printf 'x\\n' >> {}",
            crate::util::shell_quote(&count_path.display().to_string())
        );
        let conn = s.conn().unwrap();
        crate::util::exec1(
            &conn,
            "UPDATE projects SET verify_cmd = ? WHERE slug = ?",
            &[&vc, &proj.str("slug")],
        );
        let run_with = |title: &str, after_hook: &str| {
            let (job, _) = crate::jobs::add_job(
                &s,
                "proj".into(),
                "task".into(),
                title.into(),
                "commit a file".into(),
                1,
                0,
                "test".into(),
                crate::util::HUMAN_AGENT.into(),
                false,
            )
            .unwrap();
            let job = crate::jobs::lease_job(
                &s,
                "aurora".into(),
                TEST_NOW.into(),
                crate::jobs::LeaseOpts {
                    job_id: job.str("id"),
                    ..Default::default()
                },
            )
            .unwrap();
            let clone = std::path::Path::new(&s.home)
                .join("work")
                .join(job.str("id"))
                .display()
                .to_string();
            let hook_path = std::path::Path::new(&s.home).join(format!("hook-{title}"));
            std::fs::write(
                &hook_path,
                crate::keys::pre_commit_hook_script(
                    &crate::verify_lock::wrap(&s, "proj", &vc),
                    &crate::verify_lock::wrap(&s, "proj", &vc),
                    None,
                    &[],
                    &clone,
                    &s.home,
                    "proj",
                    "aurora",
                    "",
                ),
            )
            .unwrap();
            let worker = format!(
                "cp {} .git/hooks/pre-commit && chmod +x .git/hooks/pre-commit && {after_hook}",
                crate::util::shell_quote(&hook_path.display().to_string())
            );
            let r = run_job(
                &s,
                job,
                "aurora",
                TEST_NOW,
                RunOpts {
                    worker_cmd: format!("sh -c {}", crate::util::shell_quote(&worker)),
                    ..Default::default()
                },
            )
            .unwrap();
            let log = std::fs::read_to_string(r.str("log")).unwrap_or_default();
            (r, log)
        };
        let (r, log) = run_with(
            "clean",
            "printf 'a\\n' > a.txt && git add a.txt && git commit -q -m a && echo 'SUMMARY: a'",
        );
        assert_eq!(r.str("status"), "done", "run: {r:?}\n{log}");
        assert!(
            r.str("verify").starts_with("verified earlier by hook at "),
            "run: {r:?}\n{log}"
        );
        assert!(
            log.contains("hive: verify skipped: tree ")
                && log.contains(" already passed this check on this store ("),
            "{log}"
        );
        assert_eq!(
            std::fs::read_to_string(&count_path).unwrap().trim(),
            "x",
            "the check ran once, in the hook"
        );
        // The worker commits, then edits a tracked file without committing:
        // the tree the check would judge is not the one the hook passed.
        let (r, log) = run_with(
            "dirty",
            "printf 'b\\n' > b.txt && git add b.txt && git commit -q -m b && printf 'more\\n' >> b.txt && echo 'SUMMARY: b'",
        );
        assert_eq!(r.str("verify"), "verify exit 0", "run: {r:?}\n{log}");
        assert!(!log.contains("verify skipped"), "{log}");
        assert_eq!(
            std::fs::read_to_string(&count_path).unwrap().trim(),
            "x\nx\nx",
            "the hook's check and the run's"
        );
    }

    /// A check that builds into a target dir shared with other trees carries
    /// its touch inside the verify lock, after the wait, and the memo key is
    /// the check without it: the hook, the run-end check and the base check
    /// all key the same memo whatever they had to touch.
    #[test]
    fn wrapped_check_touches_the_sources_inside_the_lock_and_keys_the_memo_without_it() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        let (mut proj, root) = new_repo(&s);
        s.set_setting("human", "verify.lock", "project").unwrap();
        let shared =
            "env CARGO_TARGET_DIR=${HIVE_TEST_FRESHEN_UNSET:-/tmp/hive-land-shared} cargo test";
        proj.insert("verify_cmd", serde_json::json!(shared));
        let cmd = wrapped_check(&s, &proj, None, &[], &root);
        let lock_taken = cmd.find("hive_t0=$(date +%s)").expect("the lock prelude");
        let touch = cmd.find("xargs -0 -r touch -c").expect("the touch");
        let check = cmd.find(shared).expect("the check");
        assert!(lock_taken < touch && touch < check, "{cmd}");
        assert_eq!(
            verify_memo_key(&s, &proj),
            crate::verify_lock::wrap(&s, "proj", shared),
            "the memo key is the bare check inside the lock"
        );
        assert_ne!(cmd, verify_memo_key(&s, &proj));
        // A target dir inside the clone is this tree's alone: no touch, and
        // the check is its own memo key.
        proj.insert("verify_cmd", serde_json::json!("cargo test"));
        let cmd = wrapped_check(&s, &proj, None, &[], &root);
        assert!(!cmd.contains("touch"), "{cmd}");
        assert_eq!(cmd, verify_memo_key(&s, &proj));
    }

    /// Two runs fail their check on the same base: the first checks the base
    /// tree and memoes it green, the second reads the memo instead of taking
    /// the project's verify lock for the same answer.
    #[test]
    fn run_job_checks_a_base_tree_once_across_runs_that_fail_on_it() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        let (_, root) = new_repo(&s);
        let count_path = std::path::Path::new(&s.home).join("base-checks");
        std::fs::write(
            std::path::Path::new(&root).join("check.sh"),
            format!(
                "printf 'b\\n' >> {}\nexit 0\n",
                crate::util::shell_quote(&count_path.display().to_string())
            ),
        )
        .unwrap();
        run(&root, &["git", "add", "check.sh"]);
        run(&root, &["git", "commit", "-q", "-m", "green base"]);
        let base = run(&root, &["git", "rev-parse", "HEAD"]).trim().to_string();
        s.conn()
            .unwrap()
            .execute(
                "UPDATE projects SET verify_cmd = ? WHERE slug = 'proj'",
                ["sh ./check.sh"],
            )
            .unwrap();
        join(&s, "aurora");
        let run_red = |title: &str| {
            let (job, _) = crate::jobs::add_job(
                &s,
                "proj".into(),
                "task".into(),
                title.into(),
                "break the check".into(),
                1,
                0,
                "test".into(),
                crate::util::HUMAN_AGENT.into(),
                false,
            )
            .unwrap();
            let job = crate::jobs::lease_job(
                &s,
                "aurora".into(),
                TEST_NOW.into(),
                crate::jobs::LeaseOpts {
                    job_id: job.str("id"),
                    ..Default::default()
                },
            )
            .unwrap();
            // The worker's commit turns the check red; the base stays green.
            let worker = format!(
                "printf 'exit 1\\n' > check.sh && git add check.sh && git commit -q -m {title} && echo 'SUMMARY: {title}'"
            );
            let r = run_job(
                &s,
                job,
                "aurora",
                TEST_NOW,
                RunOpts {
                    worker_cmd: format!("sh -c {}", crate::util::shell_quote(&worker)),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(r.str("status"), "failed", "run: {r:?}");
            // The clone goes with the run; its commit stays for a retry.
            let kept = crate::land::failed_work_branch("aurora", &r.str("job_id"));
            let patch = std::path::Path::new(&s.home)
                .join("patches")
                .join("proj")
                .join(format!("{}.patch", kept.replace('/', "__")));
            assert_eq!(
                r.str("summary"),
                format!(
                    "1 commit(s) but verify exit 1; nothing landed; its commits are kept on branch {kept} and in {}",
                    patch.display()
                ),
                "the base is green either way"
            );
            assert_eq!(
                run(&root, &["git", "log", "-1", "--format=%s", &kept]).trim(),
                title,
                "the retry finds the failed run's commit on {kept}"
            );
            assert!(
                std::fs::read_to_string(&patch).unwrap().contains("exit 1"),
                "the patch carries the commit"
            );
            std::fs::read_to_string(r.str("log")).unwrap()
        };
        let log = run_red("first");
        assert!(
            log.contains("base tree verify exit 0: the base is green"),
            "{log}"
        );
        assert_eq!(std::fs::read_to_string(&count_path).unwrap().trim(), "b");
        let base_tree = run(&root, &["git", "rev-parse", &format!("{base}^{{tree}}")])
            .trim()
            .to_string();
        let memo = crate::verify_memo::lookup(
            &s.home,
            "proj",
            &base_tree,
            &crate::verify_lock::wrap(&s, "proj", "sh ./check.sh"),
            24,
            chrono::Utc::now(),
        )
        .expect("the green base is memoed");
        assert_eq!(memo.by, "base");
        let log = run_red("second");
        assert!(
            log.contains(&format!(
                "hive: base verify skipped: tree {base_tree} already passed this check on this store (verified earlier by base at "
            )),
            "{log}"
        );
        assert!(!log.contains("base tree verify exit"), "{log}");
        assert_eq!(
            std::fs::read_to_string(&count_path).unwrap().trim(),
            "b",
            "the base was checked once"
        );
    }

    /// The reviewer's clone stands on the pull request's head after
    /// A commit the identity check refuses (here: no trailers) is still the
    /// worker's work: it is kept on the job's -failed branch, as after a
    /// failed check, and the refusal names that branch.
    #[test]
    fn commits_the_landing_refuses_are_kept_on_the_failed_branch() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        let (_proj, root) = new_repo(&s);
        join(&s, "aurora");
        let (job, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "no trailers".into(),
            "commit without the provenance hook".into(),
            1,
            0,
            "test".into(),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let job = crate::jobs::lease_job(
            &s,
            "aurora".into(),
            TEST_NOW.into(),
            crate::jobs::LeaseOpts {
                job_id: job.str("id"),
                ..Default::default()
            },
        )
        .unwrap();
        // Signed by the run's key, but past the hook: no trailers, as a
        // message whose last paragraph read `H: x` left it under doNothing.
        let worker = "printf 'x\\n' > x.txt && git add x.txt && git -c core.hooksPath=/dev/null commit -q -m 'refused work' && echo 'SUMMARY: committed'";
        let r = run_job(
            &s,
            job,
            "aurora",
            TEST_NOW,
            RunOpts {
                worker_cmd: format!("sh -c {}", crate::util::shell_quote(worker)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(r.str("status"), "failed", "run: {r:?}");
        let kept = crate::land::failed_work_branch("aurora", &r.str("job_id"));
        let summary = r.str("summary");
        assert!(
            summary.starts_with("1 commit(s) refused: commit ")
                && summary.contains("lacks its Hive-Agent trailer")
                && summary.contains(&format!(
                    "nothing landed; its commits are kept on branch {kept}"
                )),
            "the refusal does not name the kept branch: {summary}"
        );
        assert_eq!(
            run(&root, &["git", "log", "-1", "--format=%s", &kept]).trim(),
            "refused work",
            "the refused commit is not on {kept}"
        );
        // The job's own branch is not made: a retry lands there.
        assert!(
            !crate::unchecked::branch_exists(&root, &format!("hive/aurora/{}", r.str("job_id"))),
            "the refused work landed on the job's branch"
        );
    }

    /// `hive review checkout`: the commits past the base are the author's,
    /// signed by another run's key, and the run lands nothing of them.
    /// The review's verdict is its outcome.
    #[test]
    fn a_review_run_lands_nothing_of_the_pull_request_it_checked_out() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        let (_proj, root) = new_repo(&s);
        join(&s, "aurora");
        let (job, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "review".into(),
            "Review PR #7: a change of someone else's".into(),
            "read the diff and give a verdict".into(),
            1,
            0,
            "review:proj:7".into(),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let job = crate::jobs::lease_job(
            &s,
            "aurora".into(),
            TEST_NOW.into(),
            crate::jobs::LeaseOpts {
                job_id: job.str("id"),
                ..Default::default()
            },
        )
        .unwrap();
        // The author's commit, as the pull request's branch brings it into
        // the clone: not this run's key, not this agent.
        let worker = "printf 'theirs\\n' > theirs.txt && git add theirs.txt && git -c user.name=other-1 -c user.email=other-1@hive.local commit -q --no-gpg-sign -m 'theirs' && echo 'SUMMARY: merged #7 after reading the diff'";
        let r = run_job(
            &s,
            job,
            "aurora",
            TEST_NOW,
            RunOpts {
                worker_cmd: format!("sh -c {}", crate::util::shell_quote(worker)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(r.str("status"), "done", "run: {r:?}");
        assert_eq!(r.int("commits"), 0, "run: {r:?}");
        assert_eq!(r.str("summary"), "merged #7 after reading the diff");
        assert_eq!(r.str("branch"), "", "run: {r:?}");
        let log_text = std::fs::read_to_string(r.str("log")).unwrap();
        assert!(
            log_text.contains("1 commit(s) in the clone are the pull request's"),
            "{log_text}"
        );
        assert!(!log_text.contains("identity check"), "{log_text}");
        let branches = run(&root, &["git", "branch", "--list", "hive/*"]);
        assert_eq!(branches.trim(), "", "nothing landed: {branches}");
    }

    #[test]
    fn run_job_gives_the_worker_and_the_check_a_tmpdir_beside_the_clone_and_removes_it() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        let (proj, root) = new_repo(&s);
        join(&s, "aurora");
        let seen_path = std::path::Path::new(&s.home).join("tmpdirs");
        let seen = crate::util::shell_quote(&seen_path.display().to_string());
        // The worker and the check each record their TMPDIR and leave a
        // file in it, as a killed `cargo test` leaves its tempdirs.
        let record = format!("printf '%s\\n' \"$TMPDIR\" >> {seen} && : > \"$TMPDIR/left-behind\"");
        let conn = s.conn().unwrap();
        crate::util::exec1(
            &conn,
            "UPDATE projects SET verify_cmd = ? WHERE slug = ?",
            &[&record, &proj.str("slug")],
        );
        let (job, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "tmpdir".into(),
            "leave a temp file".into(),
            1,
            0,
            "test".into(),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let job = crate::jobs::lease_job(
            &s,
            "aurora".into(),
            TEST_NOW.into(),
            crate::jobs::LeaseOpts {
                job_id: job.str("id"),
                ..Default::default()
            },
        )
        .unwrap();
        let job_id = job.str("id");
        let worker = format!(
            "{record} && printf 'x\\n' > x.txt && git add x.txt && git commit -q -m x && echo 'SUMMARY: left a temp file'"
        );
        let r = run_job(
            &s,
            job,
            "aurora",
            TEST_NOW,
            RunOpts {
                worker_cmd: format!("sh -c {}", crate::util::shell_quote(&worker)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(r.str("status"), "done", "run: {r:?}");
        assert_eq!(r.str("verify"), "verify exit 0", "run: {r:?}");
        // The passed check leaves its memo for the branch's tree, keyed as
        // the hook and a landing key theirs.
        let tree = run(
            &root,
            &["git", "rev-parse", &format!("{}^{{tree}}", r.str("branch"))],
        )
        .trim()
        .to_string();
        let memo = crate::verify_memo::lookup(
            &s.home,
            "proj",
            &tree,
            &crate::verify_lock::wrap(&s, "proj", &record),
            24,
            chrono::Utc::now(),
        )
        .expect("the run's passed check is memoed");
        assert_eq!(memo.by, "run");
        let want = std::path::Path::new(&s.home)
            .join("work")
            .join(format!("{job_id}.tmp"))
            .display()
            .to_string();
        let seen = std::fs::read_to_string(&seen_path).unwrap();
        // Worker, pre-commit check, run-end check: every one beside the clone.
        let lines = seen.lines().collect::<Vec<_>>();
        assert!(lines.len() >= 2, "{seen:?}");
        assert!(lines.iter().all(|l| *l == want), "{seen:?} != {want}");
        crate::retention::empty_trash(std::path::Path::new(&s.home));
        assert!(
            !std::path::Path::new(&want).exists(),
            "the run's TMPDIR goes with the clone"
        );
        assert!(
            !std::path::Path::new(&s.home)
                .join("work")
                .join(&job_id)
                .exists()
        );
    }

    #[test]
    fn run_job_hands_a_declared_vault_secret_to_the_worker_env_only() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        let (proj, _) = new_repo(&s);
        join(&s, "aurora");
        let value = "vault-value-7f3a9c1e";
        s.vault_set("owner", "DEPLOY_TOKEN", value.as_bytes())
            .unwrap();
        s.vault_set("owner", "OTHER_TOKEN", b"never-granted-42")
            .unwrap();
        let mut recipe = recipe_with(None, None, None, None, "");
        recipe = recipe.replacen("recipe: 1\n", "recipe: 1\nsecrets:\n  - DEPLOY_TOKEN\n", 1);
        crate::env::env_set(
            &s,
            proj.clone(),
            recipe,
            crate::util::HUMAN_AGENT.to_string(),
            "store".into(),
            "vault secret".into(),
        )
        .unwrap();
        let seen_path = std::path::Path::new(&s.home).join("secret-seen");
        let seen = crate::util::shell_quote(&seen_path.display().to_string());
        let (job, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "use the deploy token".into(),
            "Secrets: DEPLOY_TOKEN, OTHER_TOKEN".into(),
            1,
            0,
            "test".into(),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let job_id = job.str("id");
        let job = crate::jobs::lease_job(
            &s,
            "aurora".into(),
            TEST_NOW.into(),
            crate::jobs::LeaseOpts {
                job_id: job_id.clone(),
                ..Default::default()
            },
        )
        .unwrap();
        let worker = format!(
            "printf '%s' \"$DEPLOY_TOKEN\" > {seen} &&\nprintf '%s' \"${{OTHER_TOKEN:-unset}}\" >> {seen} &&\necho \"token is $DEPLOY_TOKEN\" &&\necho \"token again $DEPLOY_TOKEN\" >&2 &&\nprintf 'x\\n' > worker.txt &&\ngit add worker.txt &&\ngit commit -q -m 'use the token' &&\necho \"SUMMARY: done - used $DEPLOY_TOKEN\""
        );
        let r = run_job(
            &s,
            job,
            "aurora",
            TEST_NOW,
            RunOpts {
                worker_cmd: format!("sh -c {}", crate::util::shell_quote(&worker)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(r.str("status"), "done", "run: {r:?}");
        // The worker saw the declared value and not the undeclared one.
        let got = std::fs::read_to_string(&seen_path).unwrap();
        assert_eq!(got, format!("{value}unset"));
        // The value is in no log, no result, no event, no file of the clone.
        let log_text = std::fs::read_to_string(r.str("log")).unwrap();
        assert!(!log_text.contains(value), "{log_text}");
        assert!(log_text.contains("[secret DEPLOY_TOKEN]"), "{log_text}");
        assert!(
            log_text.contains("secret OTHER_TOKEN refused"),
            "{log_text}"
        );
        assert!(!r.str("summary").contains(value), "{r:?}");
        let conn = s.conn().unwrap();
        let events: Vec<(String, String)> = conn
            .prepare("SELECT action, detail FROM events")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(|x| x.unwrap())
            .collect();
        assert!(
            events.iter().all(|(_, d)| !d.contains(value)),
            "a value reached an event"
        );
        let jobs_row: String = conn
            .query_row("SELECT result FROM jobs WHERE id = ?", [&job_id], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(!jobs_row.contains(value), "{jobs_row}");
        let granted: Vec<&(String, String)> = events
            .iter()
            .filter(|(a, _)| a == "secret.granted")
            .collect();
        assert_eq!(granted.len(), 1, "{events:?}");
        assert!(granted[0].1.contains("DEPLOY_TOKEN"));
        assert!(granted[0].1.contains(&job_id));
        assert!(!granted[0].1.contains("OTHER_TOKEN"));
        let refused: Vec<&(String, String)> = events
            .iter()
            .filter(|(a, _)| a == "secret.refused")
            .collect();
        assert_eq!(refused.len(), 1, "{events:?}");
        assert!(refused[0].1.contains("OTHER_TOKEN"));
        assert!(refused[0].1.contains("not declared"));
        assert_eq!(
            events.iter().filter(|(a, _)| a == "secret.read").count(),
            1,
            "{events:?}"
        );
        // Every file the run left under the home. A finished run parks its
        // clone in `work/.trash` for the background remover, so a directory
        // queued here may be gone by the time it is read: that is the
        // remover's work, not a leak, and it is skipped.
        let mut stack = vec![std::path::PathBuf::from(&s.home)];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for e in entries.flatten() {
                let p = e.path();
                if p == seen_path {
                    continue;
                }
                if p.is_dir() {
                    stack.push(p);
                } else if let Ok(bytes) = std::fs::read(&p) {
                    assert!(
                        !bytes.windows(value.len()).any(|w| w == value.as_bytes()),
                        "{} holds the secret value",
                        p.display()
                    );
                }
            }
        }
    }

    #[test]
    fn run_job_recipe_setup_runs_once_before_worker_and_verify() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        let (proj, _) = new_repo(&s);
        join(&s, "aurora");

        let marker_path = std::path::Path::new(&s.home).join("setup-order");
        let marker = crate::util::shell_quote(&marker_path.display().to_string());
        let recipe = recipe_with(
            None,
            Some(&[&format!("printf 'setup\\n' >> {marker}")]),
            None,
            None,
            "",
        );
        let (id, _, _) = crate::env::env_set(
            &s,
            proj.clone(),
            recipe,
            crate::util::HUMAN_AGENT.to_string(),
            "store".into(),
            "setup ordering regression".into(),
        )
        .unwrap();
        assert_eq!(id, "CTX-environment");
        let verify_cmd = format!(
            "test \"$(cat {marker})\" = \"$(printf 'setup\\nworker')\" && printf 'verify\\n' >> {marker}"
        );
        let conn = s.conn().unwrap();
        crate::util::exec1(
            &conn,
            "UPDATE projects SET verify_cmd = ? WHERE slug = ?",
            &[&verify_cmd, &proj.str("slug")],
        );
        let (job, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "setup ordering".into(),
            "add a worker marker".into(),
            1,
            0,
            "test".into(),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let job = crate::jobs::lease_job(
            &s,
            "aurora".into(),
            TEST_NOW.into(),
            crate::jobs::LeaseOpts {
                job_id: job.str("id"),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!job.0.is_empty(), "lease returned no job");
        let worker = format!(
            "test \"$(cat {marker})\" = setup &&\nprintf 'worker\\n' >> {marker} &&\nprintf 'worker\\n' > worker.txt &&\ngit add worker.txt &&\ngit commit -q -m 'add worker marker' &&\necho 'SUMMARY: setup ran once before worker'"
        );
        let r = run_job(
            &s,
            job,
            "aurora",
            TEST_NOW,
            RunOpts {
                worker_cmd: format!("sh -c {}", crate::util::shell_quote(&worker)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(r.str("status"), "done", "run: {r:?}");
        assert_eq!(r.int("commits"), 1, "run: {r:?}");
        assert_eq!(r.str("verify"), "verify exit 0", "run: {r:?}");
        let got = std::fs::read_to_string(marker_path).unwrap();
        assert_eq!(got, "setup\nworker\nverify\n");
    }

    #[test]
    fn run_job_setup_failing_twice_plans_the_repair_job_which_runs_without_the_setup() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        let (proj, _) = new_repo(&s);
        join(&s, "aurora");
        let marker_path = std::path::Path::new(&s.home).join("setup-ran");
        let marker = crate::util::shell_quote(&marker_path.display().to_string());
        let recipe = recipe_with(
            None,
            Some(&[&format!(
                "touch {marker}; echo setup broken at this commit >&2; exit 3"
            )]),
            None,
            None,
            "",
        );
        crate::env::env_set(
            &s,
            proj.clone(),
            recipe,
            crate::util::HUMAN_AGENT.to_string(),
            "store".into(),
            "broken setup".into(),
        )
        .unwrap();
        let conn = s.conn().unwrap();
        crate::util::exec1(
            &conn,
            "UPDATE projects SET verify_cmd = 'test -f worker.txt' WHERE slug = ?",
            &[&proj.str("slug")],
        );
        let (plain, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "plain work".into(),
            "add worker.txt".into(),
            1,
            0,
            "test".into(),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let worker = "printf 'worker\\n' > worker.txt && git add worker.txt && git commit -q -m 'worker' && echo 'SUMMARY: done'";
        let opts = || RunOpts {
            worker_cmd: format!("sh -c {}", crate::util::shell_quote(worker)),
            ..Default::default()
        };
        // The first failure is one error; the same failure again freezes the
        // project on this machine and plans the repair.
        for attempt in 0..2 {
            let leased = crate::jobs::lease_job(
                &s,
                "aurora".into(),
                TEST_NOW.into(),
                crate::jobs::LeaseOpts {
                    job_id: plain.str("id"),
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(!leased.0.is_empty(), "lease {attempt}");
            let r = run_job(&s, leased, "aurora", TEST_NOW, opts()).unwrap();
            assert_eq!(r.str("status"), "error", "attempt {attempt}: {r:?}");
            assert!(
                r.str("summary").contains("setup"),
                "attempt {attempt}: {}",
                r.str("summary")
            );
        }
        let marks = crate::recipe::unfit_projects(&conn, crate::executor::LOCAL_EXECUTOR, TEST_NOW);
        assert_eq!(
            marks.get("proj").map(String::as_str),
            Some("setup:docs/context/environment.md"),
            "{marks:?}"
        );
        let repair = crate::util::must_one(
            &conn,
            "SELECT * FROM jobs WHERE project = 'proj' AND source = 'setup:proj'",
            &[],
        );
        assert_eq!(repair.str("status"), "queued", "{repair:?}");
        assert!(
            repair.str("brief").contains("setup broken at this commit"),
            "{}",
            repair.str("brief")
        );
        let ev = crate::util::must_one(
            &conn,
            "SELECT detail FROM events WHERE action = 'setup.repair' AND subject = 'proj'",
            &[],
        );
        assert!(
            ev.str("detail").contains("\"seen\": \"run\""),
            "{}",
            ev.str("detail")
        );
        // Dispatch on the frozen machine takes the repair job and nothing else
        // of the project.
        let (taken, _, why) = take_job(
            &s,
            "aurora",
            "",
            "",
            false,
            TEST_NOW,
            crate::executor::LOCAL_EXECUTOR,
            &[],
            &BTreeMap::new(),
            &[],
        )
        .unwrap();
        assert_eq!(taken.str("id"), repair.str("id"), "why = {why}");
        // The repair job runs without the recipe's setup and lands its work.
        std::fs::remove_file(&marker_path).unwrap();
        let r = run_job(&s, taken, "aurora", TEST_NOW, opts()).unwrap();
        assert_eq!(r.str("status"), "done", "run: {r:?}");
        assert_eq!(r.int("commits"), 1, "run: {r:?}");
        assert!(
            !marker_path.exists(),
            "the repair job ran the failing setup"
        );
        let log_text = std::fs::read_to_string(r.str("log")).unwrap();
        assert!(
            log_text.contains("not run: this job repairs it"),
            "{log_text}"
        );
        let brief = crate::util::must_one(
            &conn,
            "SELECT text FROM briefs WHERE run_id = ?",
            &[&r.str("id")],
        );
        assert!(
            brief
                .str("text")
                .contains("This job repairs the project's setup"),
            "{}",
            brief.str("text")
        );
        // The plain job still waits: the machine stays frozen for it until
        // the mark clears.
        let (next, _, why) = take_job(
            &s,
            "aurora",
            "",
            "",
            false,
            TEST_NOW,
            crate::executor::LOCAL_EXECUTOR,
            &[],
            &BTreeMap::new(),
            &[],
        )
        .unwrap();
        assert!(next.0.is_empty(), "took {next:?}");
        assert!(why.contains("cannot set up"), "why = {why}");
    }

    #[test]
    fn run_job_stopped_by_the_runner_during_setup_is_lost_not_timed_out() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        let (proj, _) = new_repo(&s);
        join(&s, "aurora");

        let started_path = std::path::Path::new(&s.home).join("setup-started");
        let started = crate::util::shell_quote(&started_path.display().to_string());
        let recipe = recipe_with(
            None,
            Some(&[&format!("touch {started} && sleep 60")]),
            None,
            None,
            "",
        );
        crate::env::env_set(
            &s,
            proj.clone(),
            recipe,
            crate::util::HUMAN_AGENT.to_string(),
            "store".into(),
            "slow setup".into(),
        )
        .unwrap();
        let (job, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "slow setup".into(),
            "never reached".into(),
            1,
            0,
            "test".into(),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let job = crate::jobs::lease_job(
            &s,
            "aurora".into(),
            TEST_NOW.into(),
            crate::jobs::LeaseOpts {
                job_id: job.str("id"),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!job.0.is_empty(), "lease returned no job");
        let job_id = job.str("id");
        let cancel = hive_bus::CancellationToken::new();
        let run_cancel = cancel.clone();
        let run_store = s.clone();
        let (result_tx, result_rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let r = run_job(
                &run_store,
                job,
                "aurora",
                TEST_NOW,
                RunOpts {
                    worker_cmd: "sh -c 'echo SUMMARY: never reached'".into(),
                    cancel: run_cancel,
                    ..Default::default()
                },
            );
            let _ = result_tx.send(r);
        });
        // The setup is under way once its first command ran; the runner stops
        // the run there, as the hold cap of a forced restart would.
        let began = std::time::Instant::now();
        while !started_path.exists() {
            assert!(
                began.elapsed() < crate::test_helpers_daemon::test_wait(Duration::from_secs(60)),
                "the setup never started"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        cancel.cancel();
        let r = result_rx
            .recv_timeout(crate::test_helpers_daemon::test_wait(Duration::from_secs(
                60,
            )))
            .expect("the run did not settle after the runner stopped it")
            .unwrap();
        assert_eq!(r.str("status"), "lost", "run: {r:?}");
        assert_eq!(
            r.str("summary"),
            format!("{LOST_RUN_SUMMARY} (during setup; the worker had not started)"),
            "run: {r:?}"
        );
        assert!(
            !r.str("summary").contains("timed out"),
            "a stop is not a timeout: {r:?}"
        );
        let conn = s.conn().unwrap();
        let j = crate::jobs::job_row(&conn, job_id);
        assert_eq!(j.str("status"), "queued", "job: {j:?}");
        // The cut run posts no result: the job is not over.
        assert_eq!(
            crate::util::must_rows(&conn, "SELECT id FROM posts WHERE kind = 'result'", &[]).len(),
            0
        );
    }

    #[test]
    fn base_worktree_command_runs_the_whole_check_in_the_worktree_or_not_at_all() {
        // It spawns git and sh from PATH: other tests swap PATH under these.
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("clone");
        std::fs::create_dir_all(&root).unwrap();
        run(&root, &["git", "init", "-q"]);
        run(
            &root,
            &["git", "commit", "-q", "--allow-empty", "-m", "base"],
        );
        let base = run(&root, &["git", "rev-parse", "HEAD"]).trim().to_string();
        let marks = tmp.path().join("marks");
        std::fs::create_dir_all(&marks).unwrap();
        let mark = |name: &str| marks.join(name).display().to_string();
        // A wrapped check is several lines (the disk and lock preludes come
        // first); each line records where it ran.
        let check = format!(
            "pwd -P > {}\npwd -P > {}\nexit 3",
            crate::util::shell_quote(&mark("first")),
            crate::util::shell_quote(&mark("second"))
        );
        let sh = |cmd: &str| -> i32 {
            let env = crate::util::strip_hook_git_env(
                &std::env::vars()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>(),
            );
            let mut c = std::process::Command::new("/bin/sh");
            c.args(["-c", cmd]).current_dir(&root).env_clear();
            for kv in env {
                if let Some((k, v)) = kv.split_once('=') {
                    c.env(k, v);
                }
            }
            c.env("GIT_CONFIG_GLOBAL", "/dev/null")
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap()
                .code()
                .unwrap_or(-1)
        };
        let cmd = base_worktree_command("../.hive-base-test", &base, &check);
        // The check runs on the base tree, every line of it, and its exit
        // code is the verdict; the worktree goes away again.
        assert_eq!(sh(&cmd), 3);
        for line in ["first", "second"] {
            let ran_in = std::fs::read_to_string(mark(line)).unwrap();
            assert!(
                ran_in.trim().ends_with("/.hive-base-test"),
                "the {line} line ran in {ran_in}"
            );
            std::fs::remove_file(mark(line)).unwrap();
        }
        assert!(!tmp.path().join(".hive-base-test").exists());
        assert_eq!(run(&root, &["git", "worktree", "list"]).lines().count(), 1);
        // The worktree is gone before the check can enter it, as when the
        // clone is moved to the trash mid-run: no line of the check may run
        // in the job's own clone, and the exit says the base was not checked.
        let hook = root.join(".git/hooks/post-checkout");
        std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
        std::fs::write(
            &hook,
            "#!/bin/sh\ncase \"$PWD\" in */.hive-base-test) rm -rf \"$PWD\" ;; esac\n",
        )
        .unwrap();
        run(&root, &["chmod", "+x", ".git/hooks/post-checkout"]);
        assert_eq!(sh(&cmd), 125, "a worktree that vanished is no verdict");
        assert!(!std::path::Path::new(&mark("first")).exists());
        assert!(
            !std::path::Path::new(&mark("second")).exists(),
            "the rest of the check ran outside the base tree: {}",
            std::fs::read_to_string(mark("second")).unwrap_or_default()
        );
    }

    #[test]
    fn run_job_does_not_call_the_base_red_when_the_check_did_not_run() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        let (_, root) = new_repo(&s);
        std::fs::write(std::path::Path::new(&root).join("check.sh"), "exit 0\n").unwrap();
        run(&root, &["git", "add", "check.sh"]);
        run(&root, &["git", "commit", "-q", "-m", "green base"]);
        let base = run(&root, &["git", "rev-parse", "HEAD"]).trim().to_string();
        s.conn()
            .unwrap()
            .execute(
                "UPDATE projects SET verify_cmd = ? WHERE slug = 'proj'",
                ["sh ./check.sh"],
            )
            .unwrap();
        // A floor no disk reaches: the verify prelude exits 71 before the
        // check runs, for the job's tree and for the base tree alike.
        s.set_setting("human", "daemon.min_free_mb", "999999999")
            .unwrap();
        join(&s, "aurora");
        let (job, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "harmless commit".into(),
            "add a note".into(),
            1,
            0,
            "test".into(),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let job = crate::jobs::lease_job(
            &s,
            "aurora".into(),
            TEST_NOW.into(),
            crate::jobs::LeaseOpts {
                job_id: job.str("id"),
                ..Default::default()
            },
        )
        .unwrap();
        let worker = "echo note > NOTE.md && git add NOTE.md && git commit -q -m note && echo 'SUMMARY: added a note'";
        let r = run_job(
            &s,
            job,
            "aurora",
            TEST_NOW,
            RunOpts {
                worker_cmd: format!("sh -c {}", crate::util::shell_quote(worker)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(r.str("status"), "failed", "run: {r:?}");
        let summary = r.str("summary");
        assert!(
            summary.starts_with(
                "1 commit(s) but verify exit 71 (disk full: under daemon.min_free_mb, verification did not run); nothing landed; its commits are kept on branch hive/aurora/"
            ),
            "the unjudged commits stay for the retry: {summary}"
        );
        assert!(
            !summary.contains("already red"),
            "a check that did not run must not call the base red: {summary}"
        );
        assert_eq!(run(&root, &["git", "rev-parse", "HEAD"]).trim(), base);
        // The base tree was not even checked: its prelude would stop the
        // same way.
        let log = std::fs::read_to_string(r.str("log")).unwrap();
        assert!(
            log.contains("the check did not run, so the base tree is not checked either"),
            "{log}"
        );
        assert!(!log.contains("verify the base tree"), "{log}");
        assert_eq!(
            run(&root, &["git", "worktree", "list"]).lines().count(),
            1,
            "no base-tree worktree was made"
        );
        // Recovery treats it as the environment's failure, not the job's.
        assert_eq!(
            crate::failure::classify_failure(String::new(), "failed".into(), summary, false)
                .class
                .0,
            crate::failure::FAILURE_TRANSIENT
        );
    }

    /// Seeds the harness evidence the rescue looks for: a session of this
    /// job, started during the run, whose `git commit` step timed out.
    fn seed_killed_commit(s: &hive_store::Store, job_id: &str, command: &str) {
        s.begin_harness_session(hive_store::HarnessSession {
            id: format!("hs-{job_id}"),
            job_id: job_id.to_string(),
            agent: "aurora".into(),
            started_at: TEST_NOW.into(),
            ..Default::default()
        })
        .unwrap();
        s.append_harness_step(hive_store::HarnessStep {
            session_id: format!("hs-{job_id}"),
            n: 7,
            kind: "observation".into(),
            content: "The command timed out after 900s and was killed.".into(),
            command: command.into(),
            exit_code: Some(-1),
            duration_ms: 900_000,
            ..Default::default()
        })
        .unwrap();
    }

    fn staged_work_job(s: &hive_store::Store) -> Row {
        join(s, "aurora");
        let (job, _) = crate::jobs::add_job(
            s,
            "proj".into(),
            "task".into(),
            "note the rescue".into(),
            "add a note".into(),
            1,
            0,
            "test".into(),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        crate::jobs::lease_job(
            s,
            "aurora".into(),
            TEST_NOW.into(),
            crate::jobs::LeaseOpts {
                job_id: job.str("id"),
                ..Default::default()
            },
        )
        .unwrap()
    }

    // The worker changes a tracked file, stages a new one, and reports
    // that its commit was killed, without committing.
    const STAGED_WORK_WORKER: &str = "printf 'rescued\\n' >> README.md && echo note > NOTE.md && git add NOTE.md && echo 'SUMMARY: failed: the change is complete but git commit was killed at the step deadline, so nothing could be committed'";

    #[test]
    fn run_job_commits_the_work_a_killed_git_commit_left_in_the_tree() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        let (_, root) = new_repo(&s);
        // The check only passes on the rescued tree: both files must be in it.
        s.conn()
            .unwrap()
            .execute(
                "UPDATE projects SET verify_cmd = ? WHERE slug = 'proj'",
                ["grep -q rescued README.md && test -f NOTE.md"],
            )
            .unwrap();
        // Verification must run here whatever this machine's disk looks like.
        s.set_setting(hive_store::HUMAN_AGENT, "daemon.min_free_mb", "0")
            .unwrap();
        let job = staged_work_job(&s);
        seed_killed_commit(
            &s,
            &job.str("id"),
            "git add -A && git commit -q -m \"docs: note the rescue path\"",
        );
        let r = run_job(
            &s,
            job.clone(),
            "aurora",
            TEST_NOW,
            RunOpts {
                worker_cmd: format!("sh -c {}", crate::util::shell_quote(STAGED_WORK_WORKER)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(r.str("status"), "done", "run: {r:?}");
        assert_eq!(r.int("commits"), 1, "run: {r:?}");
        let branch = r.str("branch");
        assert!(!branch.is_empty(), "run: {r:?}");
        let message = run(
            &root,
            &[
                "git",
                "log",
                "-1",
                "--format=%B",
                &format!("refs/heads/{branch}"),
            ],
        );
        assert!(
            message.starts_with("docs: note the rescue path\n"),
            "the rescued commit keeps the worker's message: {message}"
        );
        assert!(
            message.contains("killed at the harness step deadline"),
            "the rescued commit says why the runner made it: {message}"
        );
        assert!(
            message.contains("Hive-Job:"),
            "the clone's hooks still add the provenance trailers: {message}"
        );
        let readme = run(
            &root,
            &["git", "show", &format!("refs/heads/{branch}:README.md")],
        );
        assert!(readme.ends_with("rescued\n"), "landed README: {readme}");
        let note = run(
            &root,
            &["git", "show", &format!("refs/heads/{branch}:NOTE.md")],
        );
        assert_eq!(note, "note\n", "the staged new file lands too");
        let log = std::fs::read_to_string(r.str("log")).unwrap();
        assert!(
            log.contains("the runner committed its work for verification"),
            "{log}"
        );
        assert!(!log.contains("were discarded"), "{log}");
        let stored = crate::jobs::job_row(&conn(&s), job.str("id"));
        assert_eq!(stored.str("status"), "done", "job: {stored:?}");
        assert!(
            stored
                .str("result")
                .contains("worker report: failed: the change is complete"),
            "the worker's own words stay in the result: {stored:?}"
        );
    }

    #[test]
    fn run_job_discards_uncommitted_work_without_a_killed_commit() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        let (_, root) = new_repo(&s);
        let job = staged_work_job(&s);
        // The same worker, but the step that timed out was not a commit.
        seed_killed_commit(&s, &job.str("id"), "cargo test --workspace");
        let r = run_job(
            &s,
            job.clone(),
            "aurora",
            TEST_NOW,
            RunOpts {
                worker_cmd: format!("sh -c {}", crate::util::shell_quote(STAGED_WORK_WORKER)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(r.str("status"), "failed", "run: {r:?}");
        assert_eq!(r.int("commits"), 0, "run: {r:?}");
        assert!(
            r.str("summary")
                .starts_with("failed: the change is complete"),
            "the worker's verdict stands: {r:?}"
        );
        let log = std::fs::read_to_string(r.str("log")).unwrap();
        assert!(
            log.contains("uncommitted changes left in the clone were discarded"),
            "{log}"
        );
        let refs = run(
            &root,
            &[
                "git",
                "for-each-ref",
                "--format=%(refname:short)",
                "refs/heads/hive/",
            ],
        );
        assert_eq!(refs.trim(), "", "nothing lands without a commit");
    }

    /// TASK-169 item 4: `hive job stop` without `--requeue` while the run is
    /// in its end check ends the job cancelled, as in the worker. Before, the
    /// cut check read as the runner's stop: the commits were kept and the
    /// job requeued free, so its next run landed what the owner stopped.
    #[test]
    fn job_stop_in_the_end_check_ends_the_job_rather_than_keeping_its_work() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        let (proj, root) = new_repo(&s);
        join(&s, "aurora");
        let marker = std::path::Path::new(&s.home).join("check-started");
        let mark = crate::util::shell_quote(&marker.display().to_string());
        crate::util::exec1(
            &s.conn().unwrap(),
            "UPDATE projects SET verify_cmd = ? WHERE slug = ?",
            &[&format!("touch {mark}; sleep 60"), &proj.str("slug")],
        );
        let (job, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "a note".into(),
            "add a note".into(),
            1,
            0,
            "test".into(),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let job_id = job.str("id");
        let leased = crate::jobs::lease_job(
            &s,
            "aurora".into(),
            TEST_NOW.into(),
            crate::jobs::LeaseOpts {
                job_id: job_id.clone(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!leased.0.is_empty(), "lease returned no job");
        let run_store = s.clone();
        let (result_tx, result_rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let worker = "echo note > NOTE.md && git add NOTE.md && git commit -q -m note && echo 'SUMMARY: done - added a note'";
            let r = run_job(
                &run_store,
                leased,
                "aurora",
                TEST_NOW,
                RunOpts {
                    worker_cmd: format!("sh -c {}", crate::util::shell_quote(worker)),
                    ..Default::default()
                },
            );
            let _ = result_tx.send(r);
        });
        let began = std::time::Instant::now();
        while !marker.exists() {
            assert!(
                began.elapsed() < crate::test_helpers_daemon::test_wait(Duration::from_secs(60)),
                "the check never started"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        hive_board::stop_job(&s, &job_id, false, crate::util::HUMAN_AGENT).unwrap();
        let r = result_rx
            .recv_timeout(crate::test_helpers_daemon::test_wait(Duration::from_secs(
                60,
            )))
            .expect("the run did not settle after the owner stopped it")
            .unwrap();
        let log = std::fs::read_to_string(r.str("log")).unwrap_or_default();
        assert_eq!(r.str("status"), "failed", "run: {r:?}\n{log}");
        assert!(
            r.str("summary").starts_with("cancelled by the owner"),
            "run: {r:?}"
        );
        let j = crate::jobs::job_row(&s.conn().unwrap(), job_id.clone());
        assert_eq!(j.str("status"), "failed", "job: {j:?}");
        assert!(
            crate::unchecked::pending(&s, &job_id, &root).is_none(),
            "nothing is kept for a next run"
        );
        let kept = crate::unchecked::unchecked_branch("aurora", &job_id);
        assert!(!crate::unchecked::branch_exists(&root, &kept));
    }

    /// TASK-134 follow-up: a forced restart stopped job-20261126T082845Z-4ef6b5
    /// in its end check and removed its clone with the commit. The stop now
    /// keeps the finished work on a branch, requeues the job at no cost of
    /// an attempt, and the next run checks and lands it without a worker.
    #[test]
    fn run_job_stopped_in_its_check_keeps_the_commits_and_the_next_run_lands_them() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        let (proj, root) = new_repo(&s);
        join(&s, "aurora");
        // The first check stalls (the runner stops it there); the next passes.
        let marker = std::path::Path::new(&s.home).join("check-started");
        let mark = crate::util::shell_quote(&marker.display().to_string());
        crate::util::exec1(
            &s.conn().unwrap(),
            "UPDATE projects SET verify_cmd = ? WHERE slug = ?",
            &[
                &format!("test -e {mark} && test -f NOTE.md && exit 0; touch {mark}; sleep 60"),
                &proj.str("slug"),
            ],
        );
        let (job, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "a note".into(),
            "add a note".into(),
            1,
            0,
            "test".into(),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let job_id = job.str("id");
        let lease = || {
            crate::jobs::lease_job(
                &s,
                "aurora".into(),
                TEST_NOW.into(),
                crate::jobs::LeaseOpts {
                    job_id: job_id.clone(),
                    ..Default::default()
                },
            )
            .unwrap()
        };
        let leased = lease();
        assert!(!leased.0.is_empty(), "lease returned no job");
        let cancel = hive_bus::CancellationToken::new();
        let run_cancel = cancel.clone();
        let run_store = s.clone();
        let (result_tx, result_rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let worker = "echo note > NOTE.md && git add NOTE.md && git commit -q -m note && echo 'SUMMARY: done - added a note' && echo 'FINDING: notes help'";
            let r = run_job(
                &run_store,
                leased,
                "aurora",
                TEST_NOW,
                RunOpts {
                    worker_cmd: format!("sh -c {}", crate::util::shell_quote(worker)),
                    cancel: run_cancel,
                    ..Default::default()
                },
            );
            let _ = result_tx.send(r);
        });
        // The worker finished and the end check is under way: the runner
        // stops the run there, as the hold cap of a forced restart does.
        let began = std::time::Instant::now();
        while !marker.exists() {
            assert!(
                began.elapsed() < crate::test_helpers_daemon::test_wait(Duration::from_secs(60)),
                "the check never started"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        cancel.cancel();
        let r = result_rx
            .recv_timeout(crate::test_helpers_daemon::test_wait(Duration::from_secs(
                60,
            )))
            .expect("the run did not settle after the runner stopped it")
            .unwrap();
        let log = std::fs::read_to_string(r.str("log")).unwrap_or_default();
        assert_eq!(r.str("status"), "lost", "run: {r:?}\n{log}");
        let kept = crate::unchecked::unchecked_branch("aurora", &job_id);
        assert!(
            r.str("summary")
                .contains(&format!("1 commit(s) kept on branch {kept}")),
            "run: {r:?}\n{log}"
        );
        // The commits survive the clone: the branch is in the checkout.
        let tip = run(&root, &["git", "rev-parse", &format!("refs/heads/{kept}")]);
        assert_eq!(
            run(&root, &["git", "show", &format!("{}:NOTE.md", tip.trim())]).trim(),
            "note"
        );
        // The job is queued again for its check, at no cost of an attempt.
        let j = crate::jobs::job_row(&s.conn().unwrap(), job_id.clone());
        assert_eq!(j.str("status"), "queued", "job: {j:?}");
        assert_eq!(j.int("attempts"), 0, "job: {j:?}");
        assert!(
            crate::unchecked::pending(&s, &job_id, &root).is_some(),
            "the next run finds the kept work"
        );

        // The next run checks the kept commits and lands them; no worker runs.
        let worker_ran = std::path::Path::new(&s.home).join("worker-ran");
        let again = run_job(
            &s,
            lease(),
            "aurora",
            TEST_NOW,
            RunOpts {
                worker_cmd: format!(
                    "sh -c {}",
                    crate::util::shell_quote(&format!(
                        "touch {}; echo 'SUMMARY: failed: redone'; exit 1",
                        crate::util::shell_quote(&worker_ran.display().to_string())
                    ))
                ),
                ..Default::default()
            },
        )
        .unwrap();
        let log = std::fs::read_to_string(again.str("log")).unwrap_or_default();
        assert_eq!(again.str("status"), "done", "run: {again:?}\n{log}");
        assert!(!worker_ran.exists(), "the task was done again:\n{log}");
        assert!(
            again.str("summary").starts_with("done - added a note"),
            "the stopped worker's report stands: {again:?}"
        );
        let landed = format!("hive/aurora/{job_id}");
        assert_eq!(
            run(
                &root,
                &["git", "rev-parse", &format!("refs/heads/{landed}")]
            )
            .trim(),
            tip.trim(),
            "the kept commits are the job's branch"
        );
        assert!(
            !crate::unchecked::branch_exists(&root, &kept),
            "the kept branch goes once its work landed"
        );
        let j = crate::jobs::job_row(&s.conn().unwrap(), job_id.clone());
        assert_eq!(j.str("status"), "done", "job: {j:?}");
        assert_eq!(j.str("branch"), landed, "job: {j:?}");
    }

    /// The queued jobs a test adds for the TASK-179 lease cases: the one
    /// the run works on, and another one a second runner leases mid-run.
    fn two_jobs_for_lease(s: &hive_store::Store) -> (String, String) {
        // Two lanes in the project: the second job leases beside the first.
        crate::capacity::pin_per_project(s, 4);
        let add = |title: &str| {
            crate::jobs::add_job(
                s,
                "proj".into(),
                "task".into(),
                title.into(),
                "add a note".into(),
                1,
                0,
                "test".into(),
                crate::util::HUMAN_AGENT.into(),
                false,
            )
            .unwrap()
            .0
            .str("id")
        };
        (add("a note"), add("another job"))
    }

    fn lease_one(s: &hive_store::Store, agent: &str, now: &str, job_id: &str, minutes: i32) -> Row {
        crate::jobs::lease_job(
            s,
            agent.into(),
            now.into(),
            crate::jobs::LeaseOpts {
                job_id: job_id.into(),
                minutes,
                ..Default::default()
            },
        )
        .unwrap()
    }

    fn requeue_events(s: &hive_store::Store, job_id: &str) -> i32 {
        count(
            s,
            "SELECT COUNT(*) AS n FROM events WHERE action = 'job.requeue' AND subject = ?",
            &[&job_id],
        )
    }

    /// TASK-179: lease_ok renewed the lease from run_job's start, so after a
    /// run longer than the lease the renewal set lease_until in the past and
    /// a lease_job call for another job in the same second requeued this one
    /// under the running run (job-20261127T121754Z-7426b5,
    /// run-20261127T133610Z-fc5703: commit 58cd3f5a discarded). The renewal
    /// now reads the store's clock at the call.
    #[test]
    fn run_job_longer_than_its_lease_renews_from_the_store_clock_and_ends_done() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let fixed = new_store();
        let (_, root) = new_repo(&fixed);
        // A store on the moving clock: HIVE_NOW is what s.now() reads.
        let _now = hive_testutil::setenv("HIVE_NOW", TEST_NOW);
        let s = hive_store::Store::open(hive_store::Options {
            home: fixed.home.clone(),
            now: String::new(),
            ..Default::default()
        })
        .unwrap();
        join(&s, "aurora");
        join(&s, "rival");
        let mut worker_gate = crate::test_helpers_heartbeat::new_lifecycle_gate("worker");
        let mut verify_gate = crate::test_helpers_heartbeat::new_lifecycle_gate("verification");
        crate::util::exec1(
            &s.conn().unwrap(),
            "UPDATE projects SET verify_cmd = ? WHERE slug = 'proj'",
            &[&format!("{}; test -f NOTE.md", verify_gate.command)],
        );
        let (job_id, other_id) = two_jobs_for_lease(&s);
        // The job's lease stands until the next day, as it did in the log.
        let leased = lease_one(&s, "aurora", TEST_NOW, &job_id, 24 * 60);
        assert_eq!(leased.str("id"), job_id, "lease: {leased:?}");
        let worker = format!(
            "set -e; echo note > NOTE.md; git add NOTE.md; git commit -q -m note; {}; echo 'SUMMARY: done - added a note'",
            worker_gate.command
        );
        let (done_tx, done_rx) = crossbeam_channel::bounded(1);
        let (result_tx, result_rx) = crossbeam_channel::bounded(1);
        let run_store = s.clone();
        std::thread::spawn(move || {
            let r = run_job(
                &run_store,
                leased,
                "aurora",
                TEST_NOW,
                RunOpts {
                    worker_cmd: format!("sh -c {}", crate::util::shell_quote(&worker)),
                    ..Default::default()
                },
            );
            let _ = result_tx.send(r);
            let _ = done_tx.send(());
        });
        worker_gate.wait(&done_rx);
        // The worker ran for three hours: far past minutes_per_job + 5.
        let later = crate::util::plus_minutes(TEST_NOW, 180);
        let _later = hive_testutil::setenv("HIVE_NOW", &later);
        worker_gate.release();
        // Renewed before the check; another runner leases another job while
        // the check runs, which requeues every lease already expired.
        verify_gate.wait(&done_rx);
        let j = crate::jobs::job_row(&s.conn().unwrap(), job_id.clone());
        assert!(
            j.str("lease_until") > later,
            "the renewal must reach past the store's clock {later}: {j:?}"
        );
        let other = lease_one(&s, "rival", &s.now(), &other_id, 0);
        assert_eq!(other.str("id"), other_id, "lease: {other:?}");
        let j = crate::jobs::job_row(&s.conn().unwrap(), job_id.clone());
        assert_eq!(j.str("status"), "leased", "requeued under its run: {j:?}");
        verify_gate.release();
        let r = result_rx
            .recv_timeout(crate::test_helpers_daemon::test_wait(Duration::from_secs(
                120,
            )))
            .expect("the run did not finish")
            .unwrap();
        let log = std::fs::read_to_string(r.str("log")).unwrap_or_default();
        assert_eq!(r.str("status"), "done", "run: {r:?}\n{log}");
        let j = crate::jobs::job_row(&s.conn().unwrap(), job_id.clone());
        assert_eq!(j.str("status"), "done", "job: {j:?}\n{log}");
        assert_eq!(requeue_events(&s, &job_id), 0, "the job was requeued");
        let tip = run(
            &root,
            &[
                "git",
                "rev-parse",
                &format!("refs/heads/{}", j.str("branch")),
            ],
        );
        assert_eq!(
            run(&root, &["git", "show", &format!("{}:NOTE.md", tip.trim())]).trim(),
            "note"
        );
    }

    /// TASK-179: a run whose lease lapsed after its last renewal - the job
    /// requeued for "lease expired" while its commits were being fetched -
    /// finished done with its check passed and the commits in the
    /// checkout. The job takes that result instead of beginning again from
    /// nothing; once another runner has leased the job, the run stays stale.
    #[test]
    fn run_job_stale_after_a_passed_check_and_fetch_gives_the_job_its_result() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        for retaken in [false, true] {
            let s = new_store();
            let (_, root) = new_repo(&s);
            join(&s, "aurora");
            join(&s, "rival");
            crate::util::exec1(
                &s.conn().unwrap(),
                "UPDATE projects SET verify_cmd = 'test -f NOTE.md' WHERE slug = 'proj'",
                &[],
            );
            // The landing fetch into the checkout waits on the gate, after
            // the run's last renewal and before it finishes the job.
            let mut fetch_gate = crate::test_helpers_heartbeat::new_lifecycle_gate("fetch");
            let hook = std::path::Path::new(&root).join(".git/hooks/reference-transaction");
            std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
            std::fs::write(
                &hook,
                format!(
                    "#!/bin/sh\n[ \"$1\" = committed ] || exit 0\ngrep -q refs/hive/landing/ || exit 0\n[ -e {once} ] && exit 0\ntouch {once}\n{}\n",
                    fetch_gate.command,
                    once = crate::util::shell_quote(&format!("{}.once", fetch_gate.ready)),
                ),
            )
            .unwrap();
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            let (job_id, other_id) = two_jobs_for_lease(&s);
            let leased = lease_one(&s, "aurora", TEST_NOW, &job_id, 0);
            let worker = "echo note > NOTE.md && git add NOTE.md && git commit -q -m note && echo 'SUMMARY: done - added a note'";
            let (done_tx, done_rx) = crossbeam_channel::bounded(1);
            let (result_tx, result_rx) = crossbeam_channel::bounded(1);
            let run_store = s.clone();
            std::thread::spawn(move || {
                let r = run_job(
                    &run_store,
                    leased,
                    "aurora",
                    TEST_NOW,
                    RunOpts {
                        worker_cmd: format!("sh -c {}", crate::util::shell_quote(worker)),
                        ..Default::default()
                    },
                );
                let _ = result_tx.send(r);
                let _ = done_tx.send(());
            });
            fetch_gate.wait(&done_rx);
            // An hour on, another runner leases another job: the lapsed
            // lease goes back to the queue under the running run.
            let later = crate::util::plus_minutes(TEST_NOW, 60);
            lease_one(&s, "rival", &later, &other_id, 0);
            assert_eq!(requeue_events(&s, &job_id), 1, "the lease did not lapse");
            if retaken {
                join(&s, "heron");
                let again = lease_one(&s, "heron", &later, &job_id, 0);
                assert_eq!(again.str("id"), job_id, "lease: {again:?}");
            }
            fetch_gate.release();
            let r = result_rx
                .recv_timeout(crate::test_helpers_daemon::test_wait(Duration::from_secs(
                    120,
                )))
                .expect("the run did not finish")
                .unwrap();
            let log = std::fs::read_to_string(r.str("log")).unwrap_or_default();
            let j = crate::jobs::job_row(&s.conn().unwrap(), job_id.clone());
            if retaken {
                assert_eq!(r.str("status"), "stale", "run: {r:?}\n{log}");
                assert_eq!(j.str("status"), "leased", "job: {j:?}");
                assert_eq!(j.str("lease_agent"), "heron", "job: {j:?}");
            } else {
                assert_eq!(r.str("status"), "done", "run: {r:?}\n{log}");
                assert!(r.str("summary").contains("verify exit 0"), "run: {r:?}");
                assert_eq!(j.str("status"), "done", "job: {j:?}\n{log}");
                assert!(!j.str("branch").is_empty(), "job: {j:?}");
                let tip = run(
                    &root,
                    &[
                        "git",
                        "rev-parse",
                        &format!("refs/heads/{}", j.str("branch")),
                    ],
                );
                assert_eq!(
                    run(&root, &["git", "show", &format!("{}:NOTE.md", tip.trim())]).trim(),
                    "note"
                );
                assert!(log.contains("the job takes this result"), "log: {log}");
                assert_eq!(
                    count(
                        &s,
                        "SELECT COUNT(*) AS n FROM events WHERE action = ? AND subject = ?",
                        &[&hive_board::JOB_ADOPT_ACTION, &job_id]
                    ),
                    1
                );
            }
        }
    }

    /// Runs `job_id` (leased to aurora) with a worker that commits NOTE.md,
    /// waits for `marker` - the check under way - then calls `stop` and
    /// returns the stopped run's row with its log.
    fn run_until_stopped_in_check(
        s: &hive_store::Store,
        job_id: &str,
        marker: &std::path::Path,
        stop: impl FnOnce(&hive_bus::CancellationToken),
    ) -> (Row, String) {
        let leased = crate::jobs::lease_job(
            s,
            "aurora".into(),
            TEST_NOW.into(),
            crate::jobs::LeaseOpts {
                job_id: job_id.to_string(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!leased.0.is_empty(), "lease returned no job");
        let cancel = hive_bus::CancellationToken::new();
        let run_cancel = cancel.clone();
        let run_store = s.clone();
        let (result_tx, result_rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let worker = "echo note > NOTE.md && git add NOTE.md && git commit -q -m note && echo 'SUMMARY: done - added a note'";
            let r = run_job(
                &run_store,
                leased,
                "aurora",
                TEST_NOW,
                RunOpts {
                    worker_cmd: format!("sh -c {}", crate::util::shell_quote(worker)),
                    cancel: run_cancel,
                    ..Default::default()
                },
            );
            let _ = result_tx.send(r);
        });
        let began = std::time::Instant::now();
        while !marker.exists() {
            assert!(
                began.elapsed() < crate::test_helpers_daemon::test_wait(Duration::from_secs(60)),
                "the check never started"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        stop(&cancel);
        let r = result_rx
            .recv_timeout(crate::test_helpers_daemon::test_wait(Duration::from_secs(
                60,
            )))
            .expect("the run did not settle after it was stopped")
            .unwrap();
        let log = std::fs::read_to_string(r.str("log")).unwrap_or_default();
        (r, log)
    }

    /// A project whose check stalls the first time (touching `check-started`
    /// under the store home) and passes once NOTE.md is there; a task job.
    fn stalling_check_job(
        s: &hive_store::Store,
        source: &str,
    ) -> (String, String, std::path::PathBuf) {
        let (proj, root) = new_repo(s);
        join(s, "aurora");
        let marker = std::path::Path::new(&s.home).join("check-started");
        let mark = crate::util::shell_quote(&marker.display().to_string());
        crate::util::exec1(
            &s.conn().unwrap(),
            "UPDATE projects SET verify_cmd = ? WHERE slug = ?",
            &[
                &format!("test -e {mark} && test -f NOTE.md && exit 0; touch {mark}; sleep 60"),
                &proj.str("slug"),
            ],
        );
        let (job, _) = crate::jobs::add_job(
            s,
            "proj".into(),
            "task".into(),
            "a note".into(),
            "add a note".into(),
            1,
            0,
            source.into(),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        (job.str("id"), root, marker)
    }

    fn lease_aurora(s: &hive_store::Store, job_id: &str) -> Row {
        crate::jobs::lease_job(
            s,
            "aurora".into(),
            TEST_NOW.into(),
            crate::jobs::LeaseOpts {
                job_id: job_id.to_string(),
                ..Default::default()
            },
        )
        .unwrap()
    }

    /// TASK-152 item 1: the stop kept the commits in the clone when the
    /// fetch onto the kept branch failed, but wrote nothing down, so the
    /// next run's clone removed them and the task was done again. The next
    /// run now fetches them from that clone first and lands them; while the
    /// fetch still fails it ends without touching the clone.
    #[test]
    fn run_job_fetches_commits_a_stopped_check_left_in_its_clone() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        let (job_id, root, marker) = stalling_check_job(&s, "test");
        let kept = crate::unchecked::unchecked_branch("aurora", &job_id);
        // A lock on the kept branch's ref: the fetch into the checkout fails.
        let lock_dir = std::path::Path::new(&root).join(".git/refs/heads/hive/aurora");
        std::fs::create_dir_all(&lock_dir).unwrap();
        let lock = lock_dir.join(format!("{job_id}-unchecked.lock"));
        std::fs::write(&lock, "").unwrap();
        let (r, log) = run_until_stopped_in_check(&s, &job_id, &marker, |c| c.cancel());
        assert_eq!(r.str("status"), "lost", "run: {r:?}\n{log}");
        assert!(
            r.str("summary").contains("is kept for the job's next run"),
            "run: {r:?}\n{log}"
        );
        assert!(!crate::unchecked::branch_exists(&root, &kept));
        let clone = std::path::Path::new(&s.home).join("work").join(&job_id);
        assert!(
            clone.join(".git").exists(),
            "the clone with the commits stays"
        );
        let stranded = crate::unchecked::stranded(&s, &job_id, &root)
            .expect("the stop writes the kept clone down");
        assert_eq!(stranded.clone, clone.display().to_string());
        let j = crate::jobs::job_row(&s.conn().unwrap(), job_id.clone());
        assert_eq!(j.str("status"), "queued", "job: {j:?}");

        // The fetch fails again: the run ends before a clone of its own and
        // the commits stay where they are.
        let worker_ran = std::path::Path::new(&s.home).join("worker-ran");
        let worker = format!(
            "sh -c {}",
            crate::util::shell_quote(&format!(
                "touch {}; echo 'SUMMARY: failed: redone'; exit 1",
                crate::util::shell_quote(&worker_ran.display().to_string())
            ))
        );
        let again = run_job(
            &s,
            lease_aurora(&s, &job_id),
            "aurora",
            TEST_NOW,
            RunOpts {
                worker_cmd: worker.clone(),
                ..Default::default()
            },
        )
        .unwrap();
        let log = std::fs::read_to_string(again.str("log")).unwrap_or_default();
        assert_eq!(again.str("status"), "error", "run: {again:?}\n{log}");
        assert!(
            again.str("summary").contains("could not be fetched onto"),
            "run: {again:?}"
        );
        assert!(!worker_ran.exists(), "no worker starts:\n{log}");
        assert!(clone.join(".git").exists(), "the clone is not removed");
        let j = crate::jobs::job_row(&s.conn().unwrap(), job_id.clone());
        assert_eq!(j.str("status"), "queued", "job: {j:?}");

        // The lock is gone: the next run fetches the commits and lands them.
        std::fs::remove_file(&lock).unwrap();
        let landed_run = run_job(
            &s,
            lease_aurora(&s, &job_id),
            "aurora",
            TEST_NOW,
            RunOpts {
                worker_cmd: worker,
                ..Default::default()
            },
        )
        .unwrap();
        let log = std::fs::read_to_string(landed_run.str("log")).unwrap_or_default();
        assert_eq!(
            landed_run.str("status"),
            "done",
            "run: {landed_run:?}\n{log}"
        );
        assert!(!worker_ran.exists(), "the task was done again:\n{log}");
        let landed = format!("hive/aurora/{job_id}");
        let tip = run(
            &root,
            &["git", "rev-parse", &format!("refs/heads/{landed}")],
        );
        assert_eq!(
            run(&root, &["git", "show", &format!("{}:NOTE.md", tip.trim())]).trim(),
            "note"
        );
        let j = crate::jobs::job_row(&s.conn().unwrap(), job_id.clone());
        assert_eq!(j.str("status"), "done", "job: {j:?}");
    }

    /// TASK-152 item 7: a job whose task the owner cancels during its end
    /// check is stopped there by the cancel; that stop is not a restart's,
    /// so the work is not kept and the job is not queued again.
    #[test]
    fn run_job_cancelled_during_its_check_is_not_requeued() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        let (job_id, root, marker) = stalling_check_job(&s, "test");
        let task = hive_tasks::tasks::new(&s, "proj", "a note", "add a note", 0, "human").unwrap();
        crate::util::exec1(
            &s.conn().unwrap(),
            "UPDATE jobs SET source = ? WHERE id = ?",
            &[&format!("task:{}", task.id), &job_id],
        );
        let s2 = s.clone();
        let (r, log) = run_until_stopped_in_check(&s, &job_id, &marker, |_| {
            hive_tasks::tasks::cancel(&s2, &task.id, "not wanted", "human").unwrap();
        });
        assert_ne!(r.str("status"), "lost", "run: {r:?}\n{log}");
        assert!(
            !r.str("summary").contains("kept on branch"),
            "run: {r:?}\n{log}"
        );
        let kept = crate::unchecked::unchecked_branch("aurora", &job_id);
        assert!(!crate::unchecked::branch_exists(&root, &kept), "{log}");
        assert!(crate::unchecked::pending(&s, &job_id, &root).is_none());
        let j = crate::jobs::job_row(&s.conn().unwrap(), job_id.clone());
        assert_ne!(j.str("status"), "queued", "job: {j:?}\n{log}");
    }

    /// TASK-152 review: a leased job set cancelled (`hive job finish ...
    /// cancelled` from another shell) and then stopped is not queued again
    /// by the stop, nor is its work kept: the run ends with the job
    /// cancelled, not stale.
    #[test]
    fn run_job_cancelled_while_leased_is_not_requeued_by_a_stop() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        let (job_id, root, marker) = stalling_check_job(&s, "test");
        let s2 = s.clone();
        let id2 = job_id.clone();
        let (r, log) = run_until_stopped_in_check(&s, &job_id, &marker, |c| {
            crate::util::exec1(
                &s2.conn().unwrap(),
                "UPDATE jobs SET status = 'cancelled' WHERE id = ?",
                &[&id2],
            );
            c.cancel();
        });
        assert_ne!(r.str("status"), "stale", "run: {r:?}\n{log}");
        assert!(
            log.contains("cancelled by the owner during the check"),
            "{log}"
        );
        let kept = crate::unchecked::unchecked_branch("aurora", &job_id);
        assert!(!crate::unchecked::branch_exists(&root, &kept), "{log}");
        let j = crate::jobs::job_row(&s.conn().unwrap(), job_id.clone());
        assert_ne!(j.str("status"), "queued", "job: {j:?}\n{log}");
        assert_eq!(j.int("attempts"), 1, "job: {j:?}");
    }

    /// TASK-152 item 6: a stop during the card's proof was read as the
    /// card's check failing - the run failed and its work went. It is now a
    /// stopped check: the work is kept and the next run proves and lands it.
    #[test]
    fn run_job_stopped_in_its_card_proof_keeps_the_work() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        s.set_setting("human", hive_store::EVOLVE_REQUIRE_CHECK, "on")
            .unwrap();
        let (job_id, root, marker) = stalling_check_job(&s, "test");
        let mark = crate::util::shell_quote(&marker.display().to_string());
        // The card's check stalls the first time; then it is red on the
        // default branch (no NOTE.md) and green with the change.
        let check = format!(
            "if test -e {mark}; then test -f NOTE.md && echo 'test result: ok. 1 passed; 0 failed'; else touch {mark}; sleep 60; fi"
        );
        let conn = s.conn().unwrap();
        conn.execute(
            "INSERT INTO posts (id, project, area, thread, kind, agent, date, summary, body, created_at) VALUES ('card-note', 'proj', 'a', 'card-note', 'suggestion', 'w', '2026-11-08T00:00:00Z', 's', ?1, '2026-11-08T00:00:00Z')",
            [format!("Class: product\nFile: NOTE.md\nVariable: v\nSignal: check:{check}\n")],
        )
        .unwrap();
        crate::util::exec1(
            &conn,
            "UPDATE jobs SET kind = 'improve', source = 'suggestion:card-note' WHERE id = ?",
            &[&job_id],
        );
        crate::util::exec1(
            &conn,
            "UPDATE projects SET verify_cmd = 'true' WHERE slug = 'proj'",
            &[],
        );
        drop(conn);
        let (r, log) = run_until_stopped_in_check(&s, &job_id, &marker, |c| c.cancel());
        assert_eq!(r.str("status"), "lost", "run: {r:?}\n{log}");
        assert!(
            !r.str("summary").contains("card's check"),
            "a stopped proof is not a failed one: {r:?}\n{log}"
        );
        let kept = crate::unchecked::unchecked_branch("aurora", &job_id);
        assert!(crate::unchecked::branch_exists(&root, &kept), "{log}");
        let j = crate::jobs::job_row(&s.conn().unwrap(), job_id.clone());
        assert_eq!(j.str("status"), "queued", "job: {j:?}");
        assert_eq!(j.int("attempts"), 0, "job: {j:?}");
        let again = run_job(
            &s,
            lease_aurora(&s, &job_id),
            "aurora",
            TEST_NOW,
            RunOpts {
                worker_cmd: "sh -c 'exit 1'".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let log = std::fs::read_to_string(again.str("log")).unwrap_or_default();
        assert_eq!(again.str("status"), "done", "run: {again:?}\n{log}");
    }

    #[test]
    fn run_job_takes_exit_75_from_a_custom_worker_as_an_error_not_a_hold() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        let (_, _root) = new_repo(&s);
        let job = staged_work_job(&s);
        let r = run_job(
            &s,
            job.clone(),
            "aurora",
            TEST_NOW,
            RunOpts {
                worker_cmd: format!("sh -c 'exit {}'", hive_store::HARNESS_HELD_EXIT_CODE),
                ..Default::default()
            },
        )
        .unwrap();
        // Only the native harness holds; a custom worker's 75 is its own exit.
        assert_ne!(r.str("status"), "held", "run: {r:?}");
        assert_eq!(r.int("exit_code"), 75, "run: {r:?}");
        let left = crate::test_helpers::work_leftovers(&s.home);
        assert!(
            left.is_empty(),
            "clone kept for a run that is not held: {left:?}"
        );
        assert!(
            s.held_harness_session_for_job(&job.str("id"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn run_job_requeues_a_worker_ended_from_outside_without_using_an_attempt() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        let (_, root) = new_repo(&s);
        let job = staged_work_job(&s);
        let job_id = job.str("id");
        let attempts = crate::jobs::job_row(&conn(&s), job_id.clone()).int("attempts");
        assert_eq!(attempts, 1, "the lease took an attempt");
        // The worker commits, then is killed from outside as a `pkill` did
        // to every worker on 2026-11-26: SIGTERM, no hold, cancel or timeout.
        let worker =
            "echo note > NOTE.md && git add NOTE.md && git commit -q -m note && kill -TERM $$";
        let r = run_job(
            &s,
            job.clone(),
            "aurora",
            TEST_NOW,
            RunOpts {
                worker_cmd: format!("sh -c {}", crate::util::shell_quote(worker)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(r.int("exit_code"), 143, "run: {r:?}");
        assert_eq!(r.str("status"), "held", "run: {r:?}");
        assert!(
            r.str("summary")
                .starts_with("ended by an outside SIGTERM (exit 143)"),
            "run: {r:?}"
        );
        let after = crate::jobs::job_row(&conn(&s), job_id.clone());
        assert_eq!(after.str("status"), "queued", "job: {after:?}");
        assert_eq!(
            after.int("attempts"),
            attempts - 1,
            "the lease's attempt is given back: {after:?}"
        );
        assert_eq!(after.str("lease_agent"), "", "job: {after:?}");
        // The commit is kept: on the job's -failed branch in the checkout,
        // and in the clone, which stays for the queued job.
        let kept = crate::land::failed_work_branch("aurora", &job_id);
        let log = run(&root, &["git", "log", "--format=%s", "-1", &kept]);
        assert_eq!(log.trim(), "note", "kept branch {kept}");
        let clone = crate::test_helpers::work_leftovers(&s.home);
        assert!(!clone.is_empty(), "the clone is kept");
        // The event names the run and the branch; the next brief reads it.
        let ev = crate::util::must_one(
            &conn(&s),
            "SELECT detail FROM events WHERE action = 'job.signalled' AND subject = ?",
            &[&job_id],
        );
        let d = crate::util::json_map(&ev.str("detail"));
        assert_eq!(
            d.get("run").and_then(|v| v.as_str()),
            Some(r.str("id").as_str())
        );
        assert_eq!(
            d.get("branch").and_then(|v| v.as_str()),
            Some(kept.as_str())
        );
        let earlier = crate::brief::earlier_attempt(&s, &job_id, "fjord").unwrap();
        assert!(earlier.why.contains("outside SIGTERM"), "{earlier:?}");
        assert!(earlier.patch.ends_with(".patch"), "{earlier:?}");
        // Not judged: no result post, and the job is not counted as failed.
        assert_eq!(
            crate::util::must_one(
                &conn(&s),
                "SELECT jobs_failed FROM agents WHERE name = 'aurora'",
                &[]
            )
            .int("jobs_failed"),
            0
        );
        // A worker that exits 143 on its own, the same way, past the limit
        // of such requeues is judged as it stands.
        for _ in 0..crate::signal_stop::SIGNAL_REQUEUE_LIMIT {
            crate::util::event_at(
                &conn(&s),
                TEST_NOW,
                "runner",
                "job.signalled",
                &job_id,
                json!({}),
            );
        }
        let job = crate::jobs::lease_job(
            &s,
            "aurora".into(),
            TEST_NOW.into(),
            crate::jobs::LeaseOpts {
                job_id: job_id.clone(),
                ..Default::default()
            },
        )
        .unwrap();
        let r = run_job(
            &s,
            job,
            "aurora",
            TEST_NOW,
            RunOpts {
                worker_cmd: "sh -c 'exit 143'".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_ne!(r.str("status"), "held", "run: {r:?}");
        assert_eq!(
            crate::jobs::job_row(&conn(&s), job_id.clone()).int("attempts"),
            attempts,
            "past the limit the attempt is used"
        );
    }

    #[test]
    fn run_job_keeps_the_finish_of_a_worker_that_held_after_finishing_the_job() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        let (_, root) = new_repo(&s);
        let job = staged_work_job(&s);
        let job_id = job.str("id");
        let generation = job.int("generation");
        // The worker commits, finishes the job (`hive job finish`, done
        // here by the test once the commit is in), then is held by the
        // runner's restart and exits as the harness does when held.
        let committed = std::path::Path::new(&s.home).join("worker-committed");
        let hold = std::path::Path::new(&s.home).join("worker-hold");
        let worker = format!(
            "echo note > NOTE.md && git add NOTE.md && git commit -q -m note && touch {} && while ! test -e {}; do sleep 0.1; done; exit {}",
            crate::util::shell_quote(&committed.display().to_string()),
            crate::util::shell_quote(&hold.display().to_string()),
            hive_store::HARNESS_HELD_EXIT_CODE
        );
        let run_store = s.clone();
        let (result_tx, result_rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let r = run_job(
                &run_store,
                job,
                "aurora",
                TEST_NOW,
                RunOpts {
                    worker_cmd: format!("sh -c {}", crate::util::shell_quote(&worker)),
                    ..Default::default()
                },
            );
            let _ = result_tx.send(r);
        });
        let began = std::time::Instant::now();
        while !committed.exists() {
            assert!(
                began.elapsed() < crate::test_helpers_daemon::test_wait(Duration::from_secs(60)),
                "the worker never committed"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        let run_id = {
            let c = s.conn().unwrap();
            crate::util::must_one(
                &c,
                "SELECT id FROM runs WHERE job_id = ? AND status = 'running'",
                &[&job_id],
            )
            .str("id")
        };
        crate::jobs::finish_job(
            &s,
            job_id.clone(),
            "aurora".into(),
            "done".into(),
            "added the note".into(),
            String::new(),
            generation,
        )
        .unwrap();
        let traj = std::path::Path::new(&s.home).join("traj-finished.json");
        std::fs::write(&traj, "[]").unwrap();
        hold_session(
            &s,
            "h-finished",
            &job_id,
            &run_id,
            "aurora",
            &traj.display().to_string(),
        );
        std::fs::write(&hold, "").unwrap();
        let r = result_rx
            .recv_timeout(crate::test_helpers_daemon::test_wait(Duration::from_secs(
                120,
            )))
            .expect("the run did not settle after the worker exited")
            .unwrap();
        // Judged by the finish, not taken as a hold: the commit lands.
        assert_eq!(r.str("status"), "done", "run: {r:?}");
        assert_eq!(r.int("commits"), 1, "run: {r:?}");
        assert_eq!(r.int("exit_code"), 75, "the run keeps the real exit: {r:?}");
        let log = std::fs::read_to_string(r.str("log")).unwrap();
        assert!(
            log.contains("the finish stands and the hold is not taken"),
            "{log}"
        );
        let c = s.conn().unwrap();
        let j = crate::jobs::job_row(&c, job_id.clone());
        assert_eq!(j.str("status"), "done", "job: {j:?}");
        let refs = run(
            &root,
            &[
                "git",
                "for-each-ref",
                "--format=%(refname:short)",
                "refs/heads/hive/",
            ],
        );
        assert!(refs.contains("hive/"), "nothing landed: {refs:?}");
        // The held session is settled, not offered to a next run.
        let h = s.harness_session_head("h-finished").unwrap().unwrap();
        assert_eq!(h.status, "resumed", "{h:?}");
        assert!(
            h.error.contains("finished the job before the hold"),
            "{}",
            h.error
        );
        assert!(s.held_harness_session_for_job(&job_id).unwrap().is_none());
        assert!(
            crate::test_helpers::work_leftovers(&s.home).is_empty(),
            "clone kept for a run that is not held"
        );
    }

    #[test]
    fn step_budget_stop_is_the_harness_step_limit_exit_alone() {
        let stopped = "harness stopped: step_limit after 150 model calls and 158 commands; format_errors=0; error: stopped at the step budget: 150 model calls";
        assert!(step_budget_stop(false, 1, stopped));
        assert!(step_budget_stop(false, 1, &format!("  {stopped}")));
        // A timeout, another exit code, or another stop reason is not it.
        assert!(!step_budget_stop(true, 1, stopped));
        assert!(!step_budget_stop(false, 0, stopped));
        assert!(!step_budget_stop(false, 75, stopped));
        assert!(!step_budget_stop(
            false,
            1,
            "harness stopped: time_limit after 40 model calls and 30 commands; format_errors=0"
        ));
        assert!(!step_budget_stop(
            false,
            1,
            "failed: the check is red and I could not fix it"
        ));
        assert!(!step_budget_stop(false, 1, ""));
    }

    #[test]
    fn run_job_judges_a_custom_workers_step_limit_report_without_a_hold() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        let (_, _root) = new_repo(&s);
        let job = staged_work_job(&s);
        // The worker leaves work in the clone and stops as the harness would
        // at the step budget; a custom worker has no session to carry on.
        let worker = "printf 'partial\\n' >> README.md && echo 'SUMMARY: harness stopped: step_limit after 2 model calls and 1 commands; format_errors=0' && exit 1";
        let r = run_job(
            &s,
            job.clone(),
            "aurora",
            TEST_NOW,
            RunOpts {
                worker_cmd: format!("sh -c {}", crate::util::shell_quote(worker)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(r.str("status"), "failed", "run: {r:?}");
        assert!(
            r.str("summary")
                .starts_with("worker exited 1: harness stopped: step_limit"),
            "run: {r:?}"
        );
        assert!(
            crate::test_helpers::work_leftovers(&s.home).is_empty(),
            "clone kept for a run that is not held"
        );
        assert!(
            s.held_harness_session_for_job(&job.str("id"))
                .unwrap()
                .is_none()
        );
        let j = crate::jobs::job_row(&s.conn().unwrap(), job.str("id"));
        assert_eq!(j.str("status"), "failed", "job: {j:?}");
    }

    #[test]
    fn a_step_budget_hold_is_given_once_and_opens_with_its_own_continuation() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        let c = s.conn().unwrap();
        assert!(!step_budget_carried_on(&c, "job-x"));
        // A restart's hold on the job is not the step budget's.
        crate::util::exec1(
            &c,
            "INSERT INTO runs (id, job_id, agent, project, started_at, status, summary) VALUES ('run-r', 'job-x', 'aurora', 'proj', ?, 'held', 'held for the runner''s restart after 12 steps; the next run carries on from harness session h-r')",
            &[&TEST_NOW],
        );
        assert!(!step_budget_carried_on(&c, "job-x"));
        crate::util::exec1(
            &c,
            "INSERT INTO runs (id, job_id, agent, project, started_at, status, summary) VALUES ('run-s', 'job-x', 'aurora', 'proj', ?, 'held', ?)",
            &[
                &TEST_NOW,
                &format!(
                    "{STEP_BUDGET_HELD_SUMMARY} after 150 steps with 3 commit(s) in the clone; the next run carries on from harness session h-s with a fresh budget"
                ),
            ],
        );
        assert!(step_budget_carried_on(&c, "job-x"));
        assert!(!step_budget_carried_on(&c, "job-y"));

        // The carried-on run is told why it stopped and what not to do.
        let held = HeldResume {
            session: hive_store::HarnessSession {
                id: "h-s".into(),
                steps: 150,
                error: STEP_BUDGET_HELD_REASON.into(),
                ..Default::default()
            },
            base: "abc".into(),
        };
        let opening = continuation_message("run-next", &held, "/work/job-x");
        assert!(
            opening.contains("earlier session h-s (150 steps)"),
            "{opening}"
        );
        assert!(opening.contains("stopped at the step budget"), "{opening}");
        assert!(
            opening.contains("the only continuation this job gets"),
            "{opening}"
        );
        assert!(opening.contains("do not widen the scope"), "{opening}");
        assert!(opening.contains("/work/job-x"), "{opening}");
        assert!(!opening.contains("restarted for an update"), "{opening}");
        let restart = HeldResume {
            session: hive_store::HarnessSession {
                id: "h-r".into(),
                steps: 12,
                error: "held: the runner restarts and the next run carries on from here".into(),
                ..Default::default()
            },
            base: "abc".into(),
        };
        let opening = continuation_message("run-next", &restart, "/work/job-x");
        assert!(opening.contains("restarted for an update"), "{opening}");
        assert!(!opening.contains("step budget before"), "{opening}");
    }

    /// A held session of `job` by `agent` from run `run`, with the
    /// trajectory at `trajectory`.
    fn hold_session(
        s: &hive_store::Store,
        id: &str,
        job: &str,
        run: &str,
        agent: &str,
        trajectory: &str,
    ) {
        s.begin_harness_session(hive_store::HarnessSession {
            id: id.into(),
            run_id: run.into(),
            job_id: job.into(),
            agent: agent.into(),
            ..Default::default()
        })
        .unwrap();
        s.finish_harness_session(
            id,
            hive_store::HARNESS_HELD,
            hive_store::HARNESS_HELD,
            "",
            "held for the runner's restart",
            0.0,
            trajectory,
        )
        .unwrap();
    }

    #[test]
    fn a_run_carries_a_held_run_on_only_under_the_same_agent_with_everything_in_place() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        let (project, _root) = new_repo(&s);
        let job = staged_work_job(&s);
        let job_id = job.str("id");
        let mut row = Row::new();
        row.insert("name", serde_json::json!("local"));
        let mut log = RunLog::default();
        let mut ex =
            crate::exec::new_local_exec(row, &s, &s.home, job.clone(), project, "aurora", &mut log);
        crate::keys::make_run_key(&ex.key_path(), "aurora", "run-held").unwrap();
        let base = ex.prepare(&hive_bus::CancellationToken::new()).unwrap();
        let traj = std::path::Path::new(&s.home).join("traj.json");
        std::fs::write(&traj, "[]").unwrap();
        let traj = traj.display().to_string();
        let c = s.conn().unwrap();
        crate::util::exec1(
            &c,
            "INSERT INTO runs (id, job_id, agent, project, started_at, status, base) VALUES ('run-held', ?, 'aurora', 'proj', ?, 'held', ?)",
            &[&job_id, &TEST_NOW, &base],
        );
        let ask = |s: &hive_store::Store, agent: &str, custom: bool, log: &mut RunLog| {
            super::resumable_held_run(s, &job_id, agent, "run-next", &ex, custom, log)
        };

        // Nothing held: a plain start.
        assert!(ask(&s, "aurora", false, &mut log).is_none());
        assert!(!log.text().contains("not carried on"), "{}", log.text());

        // Another agent cannot carry the commits on; the session is settled
        // so the question is not asked again.
        hold_session(&s, "h1", &job_id, "run-held", "aurora", &traj);
        let mut log = RunLog::default();
        assert!(ask(&s, "borealis", false, &mut log).is_none());
        assert!(
            log.text()
                .contains("its worker was aurora, this run's is borealis"),
            "{}",
            log.text()
        );
        let h = s.harness_session_head("h1").unwrap().unwrap();
        assert_eq!(h.status, "resumed");
        assert!(
            h.error
                .starts_with("not carried on: run run-next started over"),
            "{}",
            h.error
        );
        assert!(s.held_harness_session_for_job(&job_id).unwrap().is_none());

        // A custom worker command cannot resume a harness session.
        hold_session(&s, "h2", &job_id, "run-held", "aurora", &traj);
        let mut log = RunLog::default();
        assert!(ask(&s, "aurora", true, &mut log).is_none());
        assert!(log.text().contains("custom worker"), "{}", log.text());

        // The same agent, with the trajectory, clone, key and base: carried on.
        hold_session(&s, "h3", &job_id, "run-held", "aurora", &traj);
        let mut log = RunLog::default();
        let r = ask(&s, "aurora", false, &mut log).expect("resumable");
        assert_eq!(r.session.id, "h3");
        assert_eq!(r.base, base);
        assert_eq!(
            s.harness_session_head("h3").unwrap().unwrap().status,
            "held",
            "deciding does not settle the session; the run does once it starts"
        );
        let opening = super::continuation_message("run-next", &r, &Executor::clone(&ex));
        assert!(
            opening.contains("carrying on your earlier session h3"),
            "{opening}"
        );
        assert!(opening.contains("do not start over"), "{opening}");

        // The trajectory gone: start over.
        std::fs::remove_file(&traj).unwrap();
        let mut log = RunLog::default();
        assert!(ask(&s, "aurora", false, &mut log).is_none());
        assert!(log.text().contains("its trajectory"), "{}", log.text());
        std::fs::write(&traj, "[]").unwrap();

        // The held run's base unknown: start over.
        hold_session(&s, "h4", &job_id, "run-other", "aurora", &traj);
        let mut log = RunLog::default();
        assert!(ask(&s, "aurora", false, &mut log).is_none());
        assert!(log.text().contains("recorded no base"), "{}", log.text());

        // The clone gone: start over.
        hold_session(&s, "h5", &job_id, "run-held", "aurora", &traj);
        ex.cleanup();
        let mut log = RunLog::default();
        assert!(ask(&s, "aurora", false, &mut log).is_none());
        assert!(log.text().contains("its clone"), "{}", log.text());
    }

    #[test]
    fn killed_commit_command_matches_git_commit_only() {
        for (cmd, hit) in [
            ("git add -A && git commit -qm x", true),
            ("git -C /tmp/x commit --amend", true),
            ("cd crates && git commit -q -m 'two words'", true),
            ("git add -A && git status", false),
            ("cargo test --workspace", false),
            ("echo 'git commit' > notes.txt", true),
        ] {
            assert_eq!(GIT_COMMIT_RE.is_match(cmd), hit, "{cmd}");
        }
    }

    #[test]
    fn commit_message_from_command_reads_the_workers_message() {
        assert_eq!(
            commit_message_from_command("git add -A && git commit -q -m \"docs: note\" && echo ok"),
            Some("docs: note".into())
        );
        assert_eq!(
            commit_message_from_command("git commit -qm 'fix: two words'"),
            Some("fix: two words".into())
        );
        assert_eq!(
            commit_message_from_command("git commit --message=\"with \\\"quotes\\\"\""),
            Some("with \"quotes\"".into())
        );
        assert_eq!(
            commit_message_from_command("git commit -am wip"),
            Some("wip".into())
        );
        assert_eq!(commit_message_from_command("git commit -F msg.txt"), None);
        assert_eq!(commit_message_from_command("git commit -a"), None);
    }

    #[test]
    fn run_job_removes_what_the_agent_left_and_writes_it_down() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        new_repo(&s);
        join(&s, "aurora");
        let (job, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "leave a worktree".into(),
            "forget to clean up".into(),
            1,
            0,
            "test".into(),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let job = crate::jobs::lease_job(
            &s,
            "aurora".into(),
            TEST_NOW.into(),
            crate::jobs::LeaseOpts {
                job_id: job.str("id"),
                ..Default::default()
            },
        )
        .unwrap();
        let job_id = job.str("id");
        let left_dir = std::path::Path::new(&s.home).join("left-behind");
        let left = crate::util::shell_quote(&left_dir.display().to_string());
        // The worker adds a worktree outside the clone and never removes it (rule 9).
        let worker = format!(
            "git worktree add -q --detach {left} && echo junk > {left}/junk.txt && echo 'SUMMARY: forgot the worktree'"
        );
        let r = run_job(
            &s,
            job,
            "aurora",
            TEST_NOW,
            RunOpts {
                worker_cmd: format!("sh -c {}", crate::util::shell_quote(&worker)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(r.str("status"), "done", "run: {r:?}");
        assert!(!left_dir.exists(), "the leftover worktree stayed");
        assert!(
            !std::path::Path::new(&r.str("clone")).exists(),
            "the clone stayed"
        );
        let conn = s.conn().unwrap();
        let events = crate::util::must_rows(
            &conn,
            "SELECT actor, subject, detail FROM events WHERE action = 'agent.cleanup'",
            &[],
        );
        assert_eq!(events.len(), 1, "agent.cleanup events: {events:?}");
        assert_eq!(events[0].str("actor"), "aurora");
        assert_eq!(events[0].str("subject"), r.str("id"));
        let detail: serde_json::Value = serde_json::from_str(&events[0].str("detail")).unwrap();
        assert_eq!(detail["job"], job_id);
        assert_eq!(detail["project"], "proj");
        assert_eq!(detail["executor"], "local");
        assert_eq!(detail["status"], "done");
        let left_items = detail["left"].as_array().cloned().unwrap_or_default();
        assert_eq!(left_items.len(), 1, "{detail}");
        assert_eq!(left_items[0]["kind"], "worktree");
        assert_eq!(left_items[0]["removed"], true);
        assert!(
            left_items[0]["path"]
                .as_str()
                .unwrap_or("")
                .ends_with("left-behind"),
            "{detail}"
        );
        assert!(left_items[0]["bytes"].as_i64().unwrap_or(0) > 0, "{detail}");
        let text = detail["text"].as_str().unwrap_or("");
        assert!(
            text.starts_with("clone removed (")
                && text.contains("left 1 worktree behind; removed 1"),
            "{text}"
        );
        let log = std::fs::read_to_string(r.str("log")).unwrap();
        assert!(log.contains("cleanup: clone removed"), "{log}");

        // A run whose agent cleaned up writes down a clean finish.
        let (job, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "tidy".into(),
            "clean up".into(),
            1,
            0,
            "test".into(),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let job = crate::jobs::lease_job(
            &s,
            "aurora".into(),
            TEST_NOW.into(),
            crate::jobs::LeaseOpts {
                job_id: job.str("id"),
                ..Default::default()
            },
        )
        .unwrap();
        let r = run_job(
            &s,
            job,
            "aurora",
            TEST_NOW,
            RunOpts {
                worker_cmd: "sh -c 'echo SUMMARY: tidy'".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let row = crate::util::must_one(
            &conn,
            "SELECT detail FROM events WHERE action = 'agent.cleanup' AND subject = ?",
            &[&r.str("id")],
        );
        let detail: serde_json::Value = serde_json::from_str(&row.str("detail")).unwrap();
        assert_eq!(detail["left"].as_array().map(Vec::len), Some(0), "{detail}");
        assert!(
            detail["text"]
                .as_str()
                .unwrap_or("")
                .ends_with("nothing left behind"),
            "{detail}"
        );
    }

    #[test]
    fn run_job_casts_votes_for_cards_on_its_ballot() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        let s = new_store();
        // The card names a check; with check-skips-quorum on it would be
        // eligible at once and never reach a ballot, which this test needs.
        set_setting(&s, hive_store::EVOLVE_CHECK_SKIPS_QUORUM, "off");
        new_bare_repo(&s);
        join(&s, "aurora");
        join(&s, "alice");
        let card = crate::board::create_post(
            &s,
            TEST_NOW.into(),
            crate::board::PostOpts {
                kind: "suggestion".into(),
                project: "proj".into(),
                area: "evolution".into(),
                thread: "votes".into(),
                agent: crate::board::actor_agent(&s, "alice".into()),
                session: "alice-session".into(),
                summary: "Empty input to the parser returns an error instead of panicking".into(),
                body: "Class: product\nFile: src/parser.rs\nVariable: empty-input guard\nHypothesis: an empty input returns Err\nSignal: check:parser".into(),
                ..Default::default()
            },
        )
        .expect("suggestion post");
        let card_id = card.str("id");
        let on_ballot = crate::tally::ballot_items_for(
            &s,
            "proj".into(),
            "aurora".into(),
            TEST_NOW.into(),
            String::new(),
        );
        assert_eq!(on_ballot.len(), 1, "fixture card is not on aurora's ballot");
        let (job, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "ballot".into(),
            "vote on the open cards".into(),
            "read the ballot and vote".into(),
            1,
            0,
            "test".into(),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let job = crate::jobs::lease_job(
            &s,
            "aurora".into(),
            TEST_NOW.into(),
            crate::jobs::LeaseOpts {
                job_id: job.str("id"),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!job.0.is_empty(), "lease returned no job");
        let worker = format!(
            "echo 'SUMMARY: voted on the ballot' && echo 'VOTE: {card_id} up one file, one variable, a check to move' && echo 'VOTE: 20260101T000000Z-nobody-000000 up this card is not on the ballot'"
        );
        let r = run_job(
            &s,
            job,
            "aurora",
            TEST_NOW,
            RunOpts {
                worker_cmd: format!("sh -c {}", crate::util::shell_quote(&worker)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(r.str("status"), "done", "run: {r:?}");
        let votes = count(
            &s,
            "SELECT COUNT(*) AS n FROM posts WHERE kind = 'vote' AND agent = 'aurora' AND on_post = ?",
            &[&card_id],
        );
        assert_eq!(votes, 1, "the VOTE line for a ballot card was not cast");
        let stray = count(
            &s,
            "SELECT COUNT(*) AS n FROM posts WHERE kind = 'vote' AND on_post = '20260101T000000Z-nobody-000000'",
            &[],
        );
        assert_eq!(stray, 0, "a VOTE line for a card off the ballot was cast");
        let result = crate::util::must_one(
            &s.conn().unwrap(),
            "SELECT body FROM posts WHERE kind = 'result' AND agent = 'aurora' ORDER BY created_at DESC LIMIT 1",
            &[],
        );
        assert!(
            result.str("body").contains("Votes: 1 cast of 2"),
            "result post does not count the cast vote: {}",
            result.str("body")
        );
        let after = crate::tally::ballot_items_for(
            &s,
            "proj".into(),
            "aurora".into(),
            TEST_NOW.into(),
            String::new(),
        );
        assert!(after.is_empty(), "a voted card is still on aurora's ballot");
    }

    /// One leased task job of `proj` run with `worker` as its shell command;
    /// the finished run's row.
    fn run_worker(s: &hive_store::Store, worker: &str, before: impl FnOnce(&str)) -> Row {
        new_bare_repo(s);
        join(s, "aurora");
        let (job, _) = crate::jobs::add_job(
            s,
            "proj".into(),
            "task".into(),
            "look around and report".into(),
            "report what you saw".into(),
            1,
            0,
            "test".into(),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let job = crate::jobs::lease_job(
            s,
            "aurora".into(),
            TEST_NOW.into(),
            crate::jobs::LeaseOpts {
                job_id: job.str("id"),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!job.0.is_empty(), "lease returned no job");
        before(&job.str("id"));
        run_job(
            s,
            job,
            "aurora",
            TEST_NOW,
            RunOpts {
                worker_cmd: format!("sh -c {}", crate::util::shell_quote(worker)),
                ..Default::default()
            },
        )
        .unwrap()
    }

    fn seed_session_cost(
        s: &hive_store::Store,
        id: &str,
        run_id: &str,
        job_id: &str,
        started_at: &str,
        tokens: [(i64, i64); 2],
        cost: f64,
    ) {
        s.begin_harness_session(hive_store::HarnessSession {
            id: id.into(),
            run_id: run_id.into(),
            job_id: job_id.into(),
            agent: "aurora".into(),
            started_at: started_at.into(),
            ..Default::default()
        })
        .unwrap();
        for (n, (prompt, completion)) in tokens.into_iter().enumerate() {
            s.append_harness_step(hive_store::HarnessStep {
                session_id: id.into(),
                n: n as i64,
                kind: "assistant".into(),
                prompt_tokens: prompt,
                completion_tokens: completion,
                ..Default::default()
            })
            .unwrap();
        }
        s.finish_harness_session(id, "done", "submitted", "", "", cost, "")
            .unwrap();
    }

    /// TASK-148 item 3: a lead run started without `--model` (no model
    /// set: the provider's default) records the model its harness session
    /// resolved on its decisions, not an empty one.
    #[test]
    fn a_run_without_a_model_records_the_default_its_session_resolved() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        let c = s.conn().unwrap();
        assert_eq!(resolved_run_model(&*c, "", "run-1", "job-1", TEST_NOW), "");
        s.begin_harness_session(hive_store::HarnessSession {
            id: "h-other".into(),
            run_id: "run-0".into(),
            job_id: "job-0".into(),
            model: "someone-elses".into(),
            started_at: TEST_NOW.into(),
            ..Default::default()
        })
        .unwrap();
        s.begin_harness_session(hive_store::HarnessSession {
            id: "h-lead".into(),
            run_id: "run-1".into(),
            job_id: "job-1".into(),
            provider: "copilot".into(),
            model: "provider-default".into(),
            started_at: TEST_NOW.into(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            resolved_run_model(&*c, "", "run-1", "job-1", TEST_NOW),
            "provider-default"
        );
        assert_eq!(
            resolved_run_model(&*c, "pinned", "run-1", "job-1", TEST_NOW),
            "pinned",
            "a model the run was started with wins"
        );
    }

    /// Runs a lead job of `proj`, leased by its lead and pinned to agent file
    /// `pin` (empty for none), down the native harness's path: the harness
    /// is a script that says the `--model` it was given and decides `post`
    /// yes. `before` gets the job's id ahead of the run.
    fn run_fake_lead(
        s: &hive_store::Store,
        post: &str,
        pin: &str,
        before: impl FnOnce(&str),
    ) -> Row {
        crate::lead::ensure_lead(s, "proj".into()).unwrap();
        let (job, _) = crate::jobs::add_job(
            s,
            "proj".into(),
            crate::lead::LEAD_KIND.into(),
            "decide the asks".into(),
            format!("Decide {post}"),
            0,
            crate::lead::LEAD_JOB_PRIORITY,
            format!("lead:proj:{post}"),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        crate::util::exec1(
            &s.conn().unwrap(),
            "UPDATE jobs SET agent = ? WHERE id = ?",
            &[&pin, &job.str("id")],
        );
        let job = crate::jobs::lease_job(
            s,
            "proj-lead".into(),
            TEST_NOW.into(),
            crate::jobs::LeaseOpts {
                job_id: job.str("id"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(job.str("kind"), crate::lead::LEAD_KIND, "{job:?}");
        before(&job.str("id"));
        let harness = std::path::Path::new(&s.home).join(format!("fake-harness-{post}.sh"));
        std::fs::write(
            &harness,
            format!(
                "#!/bin/sh\nmodel=''; prev=''\n\
                 for a in \"$@\"; do [ \"$prev\" = --model ] && model=\"$a\"; prev=\"$a\"; done\n\
                 echo \"SUMMARY: done - ran on [$model]\"\n\
                 echo 'DECIDE: {post} yes small and reversible'\n"
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&harness, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        crate::exec::TEST_HARNESS_BIN
            .lock()
            .insert(s.home.clone(), harness.display().to_string());
        let r = run_job(s, job, "proj-lead", TEST_NOW, RunOpts::default());
        crate::exec::TEST_HARNESS_BIN.lock().remove(&s.home);
        r.unwrap()
    }

    fn owner_ask(s: &hive_store::Store, thread: &str) -> String {
        crate::board::create_post(
            s,
            TEST_NOW.into(),
            crate::board::PostOpts {
                kind: "suggestion".into(),
                project: "proj".into(),
                area: "build".into(),
                thread: thread.into(),
                summary: format!("Cache the {thread} step between runs"),
                body: "The step runs twice per job.\n\nYes: Queue the cache work\nNo: Close it\n"
                    .into(),
                agent: crate::board::actor_agent(s, "daemon".into()),
                for_owner: true,
                ..Default::default()
            },
        )
        .unwrap()
        .str("id")
    }

    fn decision_of(s: &hive_store::Store, post: &str) -> Row {
        crate::util::must_one(
            &s.conn().unwrap(),
            "SELECT answer, decided_by, decided_model FROM decisions WHERE post_id = ?",
            &[&post],
        )
    }

    /// TASK-154 item 3 (TASK-148 item 3 through `run_job`): a lead run
    /// started without `--model` records on its `DECIDE:` lines the model
    /// its harness session resolved, not an empty one.
    #[test]
    fn run_job_records_the_sessions_default_model_on_the_leads_decisions() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        new_repo(&s);
        assert_eq!(s.run_model(crate::lead::LEAD_KIND), "", "no model is set");
        let post = owner_ask(&s, "run-default");
        // The harness resolved the provider's default: its session of the
        // job, started during the run, names it.
        let r = run_fake_lead(&s, &post, "", |job_id| {
            s.begin_harness_session(hive_store::HarnessSession {
                id: "h-run-default".into(),
                job_id: job_id.into(),
                provider: "copilot".into(),
                model: "provider-default".into(),
                started_at: TEST_NOW.into(),
                ..Default::default()
            })
            .unwrap();
        });
        assert!(r.str("summary").contains("ran on []"), "run: {r:?}");
        let d = decision_of(&s, &post);
        assert_eq!(d.str("answer"), "yes", "run: {r:?}");
        assert_eq!(d.str("decided_by"), "proj-lead");
        assert_eq!(d.str("decided_model"), "provider-default", "{d:?}");
    }

    /// TASK-154 item 3 (TASK-148 item 5 through `run_job`): a lead run
    /// pinned to an agent file is started on that file's model, and its
    /// `DECIDE:` lines record it, not the kind's setting.
    #[test]
    fn run_job_records_the_agent_files_model_on_the_leads_decisions() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        let (_, root) = new_repo(&s);
        set_setting(&s, "light_model", "light-setting");
        let agents = std::path::Path::new(&root).join(".hive/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("decider.md"),
            "---\ndescription: Decides.\nmodel: agent-pinned\nhive:\n  kinds: [lead]\n---\nDecide.\n",
        )
        .unwrap();
        run(&root, &["git", "add", ".hive/agents/decider.md"]);
        run(&root, &["git", "commit", "-q", "-m", "decider agent"]);
        let post = owner_ask(&s, "run-pinned");
        let r = run_fake_lead(&s, &post, "decider", |_| {});
        assert!(
            r.str("summary").contains("ran on [agent-pinned]"),
            "run: {r:?}"
        );
        let d = decision_of(&s, &post);
        assert_eq!(d.str("answer"), "yes", "run: {r:?}");
        assert_eq!(d.str("decided_model"), "agent-pinned", "{d:?}");
    }

    /// TASK-148 item 5: `run_job` starts a run pinned to an agent file on
    /// that file's model (what its decisions then record), and a custom
    /// worker command on none.
    #[test]
    fn a_run_as_an_agent_file_is_started_on_its_model() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        set_setting(&s, "light_model", "light-setting");
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".hive/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("decider.md"),
            "---\ndescription: Decides.\nmodel: agent-pinned\nhive:\n  kinds: [lead]\n---\nDecide.\n",
        )
        .unwrap();
        let mut job = Row::new();
        job.insert("id", json!("J-1"));
        job.insert("kind", json!(crate::lead::LEAD_KIND));
        job.insert("agent", json!("decider"));
        let spec = crate::agent_specs::spec_for_job(&s, &job, "proj", dir.path().to_str().unwrap())
            .spec
            .expect("the pinned agent file");
        assert_eq!(
            started_model(&s, crate::lead::LEAD_KIND, "", Some(&spec), ""),
            "agent-pinned"
        );
        assert_eq!(
            started_model(&s, crate::lead::LEAD_KIND, "", None, ""),
            "light-setting"
        );
        assert_eq!(
            started_model(&s, crate::lead::LEAD_KIND, "", Some(&spec), "sh -c true"),
            "",
            "a custom worker command picks its own model"
        );
    }

    #[test]
    fn run_cost_sums_the_runs_sessions_and_no_one_elses() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        // Two sessions of the run (a held run carried on), one of the job
        // started during the run without a run id, and three that are not
        // this run's: another run's, the job's from before, another job's.
        seed_session_cost(
            &s,
            "h1",
            "run-1",
            "job-1",
            TEST_NOW,
            [(100, 10), (200, 20)],
            0.5,
        );
        seed_session_cost(
            &s,
            "h2",
            "run-1",
            "job-1",
            TEST_NOW,
            [(50, 5), (0, 0)],
            0.25,
        );
        seed_session_cost(&s, "h3", "", "job-1", TEST_NOW, [(7, 3), (0, 0)], 0.0);
        seed_session_cost(
            &s,
            "x1",
            "run-2",
            "job-1",
            TEST_NOW,
            [(9999, 999), (0, 0)],
            9.0,
        );
        seed_session_cost(
            &s,
            "x2",
            "",
            "job-1",
            "2000-01-01T00:00:00Z",
            [(9999, 9), (0, 0)],
            9.0,
        );
        seed_session_cost(&s, "x3", "", "job-2", TEST_NOW, [(9999, 9), (0, 0)], 9.0);
        let c = s.conn().unwrap();
        let got = run_cost(&*c, "run-1", "job-1", TEST_NOW);
        assert_eq!(got.prompt_tokens, 357);
        assert_eq!(got.completion_tokens, 38);
        assert!((got.cost - 0.75).abs() < 1e-9, "{got:?}");
        // A run without a harness session (a custom worker) costs nothing.
        assert_eq!(
            run_cost(&*c, "run-none", "job-none", TEST_NOW),
            RunCost::default()
        );
        assert_eq!(trim_cost(0.75), "0.75");
        assert_eq!(trim_cost(0.0042), "0.0042");
        assert_eq!(trim_cost(0.0), "0.00");
    }

    #[test]
    fn a_finished_run_records_its_token_cost() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        let s = new_store();
        let r = run_worker(&s, "echo 'SUMMARY: done - looked around'", |job_id| {
            // The harness session of this run: it names the job and starts
            // as the run does (the shell worker cannot write one itself).
            seed_session_cost(
                &s,
                "h-run",
                "",
                job_id,
                TEST_NOW,
                [(1200, 80), (300, 20)],
                0.04,
            );
        });
        assert_eq!(r.str("status"), "done", "run: {r:?}");
        assert_eq!(r.int("prompt_tokens"), 1500, "run: {r:?}");
        assert_eq!(r.int("completion_tokens"), 100, "run: {r:?}");
        assert!((r.float("cost") - 0.04).abs() < 1e-9, "run: {r:?}");
        let log = std::fs::read_to_string(r.str("log")).unwrap();
        assert!(
            log.contains("cost: 1500 prompt + 100 completion tokens, 0.04 (provider units)"),
            "{log}"
        );
        let c = s.conn().unwrap();
        let ev = crate::util::must_one(
            &*c,
            "SELECT detail FROM events WHERE action = 'run.finish' AND subject = ?",
            &[&r.str("id")],
        );
        let d: Value = serde_json::from_str(&ev.str("detail")).unwrap();
        assert_eq!(d["prompt_tokens"], 1500, "{d}");
        assert_eq!(d["completion_tokens"], 100, "{d}");
    }

    #[test]
    fn the_runner_reads_the_outcome_code_before_the_regex() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        for (worker, status, outcome, source, logged) in [
            // The code classifies: a `blocked` code fails the run whatever
            // the words after it say.
            (
                "echo 'SUMMARY: blocked - the executor has no login'",
                "failed",
                "blocked",
                "code",
                "outcome: blocked (from the worker's SUMMARY outcome code)",
            ),
            (
                "echo 'SUMMARY: nothing-to-do - bc3f9bcc already covers it'",
                "done",
                "nothing-to-do",
                "code",
                "outcome: nothing-to-do (from the worker's SUMMARY outcome code)",
            ),
            // A `done` code is done, even with \"blocked:\" in its words.
            (
                "echo 'SUMMARY: done - unstuck the blocked: parser'",
                "done",
                "done",
                "code",
                "outcome: done (from the worker's SUMMARY outcome code)",
            ),
            // No code: the regex fallback, logged as such.
            (
                "echo 'SUMMARY: looked around'",
                "done",
                "done",
                "text",
                "outcome: done (from the summary text: no outcome code, regex fallback)",
            ),
            (
                "echo 'SUMMARY: the fixture is absent, blocked on it'",
                "done",
                "done",
                "text",
                "outcome: done (from the summary text: no outcome code, regex fallback)",
            ),
            // No report at all.
            (
                "true",
                "failed",
                "",
                "none",
                "outcome: none (from the absence of a summary)",
            ),
        ] {
            let s = new_store();
            let r = run_worker(&s, worker, |_| {});
            assert_eq!(r.str("status"), status, "{worker}: run {r:?}");
            assert_eq!(r.str("outcome"), outcome, "{worker}: run {r:?}");
            assert_eq!(r.str("outcome_source"), source, "{worker}: run {r:?}");
            let log = std::fs::read_to_string(r.str("log")).unwrap();
            assert!(log.contains(logged), "{worker}: {log}");
            let c = s.conn().unwrap();
            let ev = crate::util::must_one(
                &*c,
                "SELECT detail FROM events WHERE action = 'run.finish' AND subject = ?",
                &[&r.str("id")],
            );
            let d: Value = serde_json::from_str(&ev.str("detail")).unwrap();
            assert_eq!(d["outcome_source"], source, "{worker}: {d}");
        }
    }

    fn harvest_fixture(s: &hive_store::Store, brain: bool) -> String {
        new_bare_repo(s);
        if brain {
            s.conn()
                .unwrap()
                .execute("UPDATE projects SET brain = 1 WHERE slug = 'proj'", [])
                .unwrap();
        }
        join(s, "aurora");
        let (job, _) = crate::jobs::add_job(
            s,
            "proj".into(),
            "task".into(),
            "look around and report".into(),
            "report what you saw".into(),
            1,
            0,
            "test".into(),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let job = crate::jobs::lease_job(
            s,
            "aurora".into(),
            TEST_NOW.into(),
            crate::jobs::LeaseOpts {
                job_id: job.str("id"),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!job.0.is_empty(), "lease returned no job");
        let job_id = job.str("id");
        let worker = [
            "echo 'SUMMARY: looked around'",
            "echo 'FINDING: the parser panics on empty input'",
            "echo 'FINDING: shared: pitfalls/gh-active-account - gh issue view answers for pull request numbers too'",
            "echo 'SUGGESTION: Class: hive; File: crates/hive-runner/src/brief.rs; Variable: the FINDING line; Hypothesis: workers share pitfalls; Tier: 1; Signal: check:brief'",
            "echo 'SUGGESTION: Class: product; File: src/parser.rs; Variable: empty-input guard; Hypothesis: an empty input returns Err; Tier: 1; Signal: check:parser'",
        ]
        .join(" && ");
        let r = run_job(
            s,
            job,
            "aurora",
            TEST_NOW,
            RunOpts {
                worker_cmd: format!("sh -c {}", crate::util::shell_quote(&worker)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(r.str("status"), "done", "run: {r:?}");
        job_id
    }

    #[test]
    fn run_job_sends_hive_cards_and_shared_findings_to_the_commons() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        let s = new_store();
        let job_id = harvest_fixture(&s, false);
        let c = s.conn().unwrap();
        let shared = crate::util::must_one(
            &c,
            "SELECT area, thread, summary, body, origin_project FROM posts WHERE project = 'commons' AND kind = 'finding'",
            &[],
        );
        assert_eq!(shared.str("area"), "pitfalls");
        assert_eq!(shared.str("thread"), "gh-active-account");
        assert_eq!(shared.str("origin_project"), "proj");
        assert!(
            shared.str("summary").starts_with("gh issue view answers"),
            "{}",
            shared.str("summary")
        );
        assert!(
            shared.str("body").contains("Seen on project `proj`"),
            "{}",
            shared.str("body")
        );
        let wish = crate::util::must_one(
            &c,
            "SELECT area, thread, summary, body, origin_project FROM posts WHERE project = 'commons' AND kind = 'suggestion'",
            &[],
        );
        assert_eq!(wish.str("area"), "patterns");
        assert_eq!(wish.str("thread"), job_id.to_lowercase());
        assert_eq!(wish.str("origin_project"), "proj");
        assert!(
            wish.str("body").contains("Class: hive")
                && wish.str("body").contains("wish about Hive itself"),
            "{}",
            wish.str("body")
        );
        let local = crate::util::must_one(
            &c,
            "SELECT body, origin_project FROM posts WHERE project = 'proj' AND kind = 'suggestion'",
            &[],
        );
        assert!(
            local.str("body").contains("Class: product"),
            "{}",
            local.str("body")
        );
        assert_eq!(local.str("origin_project"), "");
        let result = crate::util::must_one(
            &c,
            "SELECT body FROM posts WHERE project = 'proj' AND kind = 'result' AND agent = 'aurora' ORDER BY created_at DESC LIMIT 1",
            &[],
        );
        let body = result.str("body");
        assert!(
            body.contains("- Finding: the parser panics on empty input"),
            "{body}"
        );
        assert!(
            body.contains("- Finding (shared board): gh issue view answers"),
            "{body}"
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM posts WHERE project = 'proj' AND kind = 'finding'",
                &[]
            ),
            0,
            "a shared finding was also posted locally"
        );
    }

    #[test]
    fn run_job_keeps_hive_cards_on_the_brain() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        let s = new_store();
        harvest_fixture(&s, true);
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM posts WHERE project = 'commons' AND kind = 'suggestion'",
                &[]
            ),
            0,
            "the brain's own hive card left for the commons"
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM posts WHERE project = 'proj' AND kind = 'suggestion'",
                &[]
            ),
            2
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM posts WHERE project = 'commons' AND kind = 'finding' AND origin_project = 'proj'",
                &[]
            ),
            1,
            "a shared finding from the brain still reaches the commons"
        );
    }

    #[test]
    fn run_job_casts_informed_votes_on_amendments_from_its_ballot() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        join(&s, "alice");
        s.put_record(
            hive_store::Record {
                id: "EVO-constitution-proj".into(),
                project: "proj".into(),
                kind: "evolution".into(),
                slug: "constitution".into(),
                title: "Constitution".into(),
                status: "active".into(),
                date: "2026-09-01".into(),
                path: "docs/evolution/constitution.md".into(),
                ..Default::default()
            },
            std::collections::BTreeMap::from([
                ("loop".to_string(), json!("off")),
                ("limits".to_string(), json!("none")),
            ]),
            crate::test_helpers_hands::TEST_CONSTITUTION,
            crate::util::HUMAN_AGENT,
            "proj",
        )
        .unwrap();
        let am = crate::board::create_post(
            &s,
            "2026-09-02T08:00:00Z".into(),
            crate::board::PostOpts {
                kind: "amendment".into(),
                project: "proj".into(),
                area: "governance".into(),
                thread: "fleet-votes".into(),
                summary: "Count a fleet as one voter".into(),
                body: crate::test_helpers_hands::TEST_AMENDMENT.into(),
                agent: crate::board::actor_agent(&s, "alice".into()),
                session: "alice-session".into(),
                ..Default::default()
            },
        )
        .expect("amendment post");
        let am_id = am.str("id");
        let on_ballot = crate::tally::ballot_items_for(
            &s,
            "proj".into(),
            "aurora".into(),
            TEST_NOW.into(),
            String::new(),
        );
        assert_eq!(
            on_ballot.len(),
            1,
            "the amendment is not on aurora's ballot"
        );
        assert_eq!(on_ballot[0]["kind"], "amendment");
        assert_eq!(on_ballot[0]["hash"], am.str("text_hash"));
        let (job, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "ballot".into(),
            "vote on the open cards".into(),
            "read the ballot and vote".into(),
            1,
            0,
            "test".into(),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        let job = crate::jobs::lease_job(
            &s,
            "aurora".into(),
            TEST_NOW.into(),
            crate::jobs::LeaseOpts {
                job_id: job.str("id"),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!job.0.is_empty(), "lease returned no job");
        let worker = format!(
            "echo 'SUMMARY: voted on the ballot' && echo 'VOTE: {am_id} up a fleet becomes one voter instead of one per worker'"
        );
        let r = run_job(
            &s,
            job,
            "aurora",
            TEST_NOW,
            RunOpts {
                worker_cmd: format!("sh -c {}", crate::util::shell_quote(&worker)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(r.str("status"), "done", "run: {r:?}");
        let vote = crate::util::must_one(
            &s.conn().unwrap(),
            "SELECT text_hash, has_read_line, body FROM posts WHERE kind = 'vote' AND agent = 'aurora' AND on_post = ?",
            &[&am_id],
        );
        assert!(
            !vote.0.is_empty(),
            "the VOTE line for the amendment was not cast"
        );
        assert_eq!(vote.str("text_hash"), am.str("text_hash"));
        assert!(
            vote.str("body").contains("Read: a fleet becomes one voter"),
            "the vote body lacks the Read line: {}",
            vote.str("body")
        );
        let res = hive_tally::compute_light(
            &s,
            "proj",
            "",
            hive_tally::Thresholds {
                now: TEST_NOW.into(),
                ..Default::default()
            },
        )
        .unwrap();
        let tallied = res
            .amendments
            .iter()
            .find(|a| a.id == am_id)
            .expect("amendment in the tally");
        assert_eq!(
            (tallied.voters, tallied.badhash, tallied.noread),
            (1, 0, 0),
            "the hub's vote did not count as informed: {tallied:?}"
        );
        let after = crate::tally::ballot_items_for(
            &s,
            "proj".into(),
            "aurora".into(),
            TEST_NOW.into(),
            String::new(),
        );
        assert!(
            after.is_empty(),
            "a voted amendment is still on aurora's ballot"
        );
    }

    #[test]
    fn scheduling_concurrent_leases_keep_one_job_per_agent() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        crate::capacity::pin_per_project(&s, 2);
        let jobs = vec![
            scheduling_job(&s, "proj", "first", 100),
            scheduling_job(&s, "proj", "second", 90),
        ];
        let start = Arc::new(std::sync::Barrier::new(jobs.len() + 1));
        let (tx, rx) = crossbeam_channel::bounded(jobs.len());
        for job in jobs.clone() {
            let s = s.clone();
            let start = Arc::clone(&start);
            let tx = tx.clone();
            std::thread::spawn(move || {
                start.wait();
                let got = crate::jobs::lease_job(
                    &s,
                    "aurora".into(),
                    TEST_NOW.into(),
                    crate::jobs::LeaseOpts {
                        job_id: job.str("id"),
                        ..Default::default()
                    },
                );
                tx.send(got).unwrap();
            });
        }
        start.wait();
        let mut leased = Vec::new();
        let mut refusals = Vec::new();
        for _ in 0..jobs.len() {
            match rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap() {
                Ok(job) if !job.0.is_empty() => leased.push(job),
                Ok(_) => {}
                Err(e) => refusals.push(e.to_string()),
            }
        }
        assert_eq!(leased.len(), 1, "one agent acquired {leased:?}");
        assert_eq!(refusals.len(), 1, "refusals: {refusals:?}");
        assert!(refusals[0].contains("aurora"), "{refusals:?}");
        assert!(refusals[0].contains(&leased[0].str("id")), "{refusals:?}");
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM events WHERE action = 'job.lease'",
                &[]
            ),
            1
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM jobs WHERE status = 'queued' AND generation = 0 AND attempts = 0",
                &[]
            ),
            1
        );
        crate::jobs::finish_job(
            &s,
            leased[0].str("id"),
            "aurora".into(),
            "done".into(),
            "finished".into(),
            "".into(),
            leased[0].int("generation"),
        )
        .unwrap();
        let next = crate::jobs::lease_job(
            &s,
            "aurora".into(),
            TEST_NOW.into(),
            crate::jobs::LeaseOpts::default(),
        )
        .unwrap();
        assert!(!next.0.is_empty(), "worker was not reusable");
    }
    #[test]
    fn scheduling_provisional_finish_keeps_capacity() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        for status in ["done", "failed", "cancelled", "blocked"] {
            for scope in ["worker", "project"] {
                let s = new_store();
                new_bare_repo(&s);
                join(&s, "aurora");
                join(&s, "borealis");
                let first = scheduling_job(&s, "proj", "first", 100);
                let next = scheduling_job(&s, "proj", "next", 90);
                let leased = crate::jobs::lease_job(
                    &s,
                    "aurora".into(),
                    TEST_NOW.into(),
                    crate::jobs::LeaseOpts {
                        job_id: first.str("id"),
                        ..Default::default()
                    },
                )
                .unwrap();
                assert!(!leased.0.is_empty(), "first lease {status}/{scope}");
                let run_id = scheduling_run(&s, &first, "aurora");
                crate::jobs::finish_job(
                    &s,
                    first.str("id"),
                    "aurora".into(),
                    status.into(),
                    "provisional report".into(),
                    "".into(),
                    leased.int("generation"),
                )
                .unwrap();
                let mut agent = "borealis";
                if scope == "worker" {
                    crate::capacity::pin_per_project(&s, 2);
                    agent = "aurora";
                }
                let got = crate::jobs::lease_job(
                    &s,
                    agent.into(),
                    TEST_NOW.into(),
                    crate::jobs::LeaseOpts {
                        job_id: next.str("id"),
                        ..Default::default()
                    },
                );
                match got {
                    Ok(job) => assert!(
                        job.0.is_empty(),
                        "provisional {status} released {scope} capacity before run {run_id} ended: {job:?}"
                    ),
                    Err(e) if scope == "worker" => {
                        assert!(e.to_string().contains(&first.str("id")), "{e}");
                    }
                    Err(e) => panic!("{e}"),
                }
                let conn = s.conn().unwrap();
                crate::util::exec1(
                    &conn,
                    "UPDATE runs SET status = ?, finished_at = ? WHERE id = ?",
                    &[&status, &TEST_NOW, &run_id],
                );
                let got = crate::jobs::lease_job(
                    &s,
                    agent.into(),
                    TEST_NOW.into(),
                    crate::jobs::LeaseOpts {
                        job_id: next.str("id"),
                        ..Default::default()
                    },
                )
                .unwrap();
                assert!(
                    !got.0.is_empty() && got.int("generation") == 1 && got.int("attempts") == 1,
                    "capacity did not reopen after run ended: {got:?}"
                );
            }
        }
    }
    #[test]
    fn scheduling_running_lease_counts_once() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        let s = new_store();
        new_bare_repo(&s);
        crate::capacity::pin_per_project(&s, 2);
        let first = scheduling_job(&s, "proj", "first", 100);
        let second = scheduling_job(&s, "proj", "second", 90);
        let third = scheduling_job(&s, "proj", "third", 80);
        assert!(
            !lease_job_id(&s, "aurora", &first).unwrap().0.is_empty(),
            "first lease"
        );
        scheduling_run(&s, &first, "aurora");
        assert!(
            !lease_job_id(&s, "borealis", &second).unwrap().0.is_empty(),
            "a running lease consumed two project slots"
        );
        let got = lease_job_id(&s, "cygnus", &third).unwrap();
        assert!(got.0.is_empty(), "project cap was exceeded: {got:?}");
    }
    #[test]
    fn scheduling_pick_worker_prefers_the_holder_of_queued_held_work() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        join(&s, "borealis");
        set_setting(&s, "quorum", "1");
        crate::capacity::pin_per_project(&s, 2);
        let job = scheduling_job(&s, "proj", "held-work", 100);
        // borealis leased last, so the rotation would pick aurora next...
        let leased = lease_job_id(&s, "borealis", &job).unwrap();
        assert!(!leased.0.is_empty(), "{leased:?}");
        assert_eq!(pick_worker(&s, TEST_NOW, "", &[]).unwrap(), "aurora");
        // ...until its run holds for a restart and the job returns to the
        // queue: only borealis can carry it on, so borealis goes first.
        s.begin_harness_session(hive_store::HarnessSession {
            id: "h-held".into(),
            run_id: "run-h-held".into(),
            job_id: job.str("id"),
            agent: "borealis".into(),
            ..Default::default()
        })
        .unwrap();
        s.finish_harness_session(
            "h-held",
            hive_store::HARNESS_HELD,
            hive_store::HARNESS_HELD,
            "",
            "held for the runner's restart",
            0.0,
            "/nowhere/traj.json",
        )
        .unwrap();
        let conn = s.conn().unwrap();
        crate::util::exec1(
            &conn,
            "UPDATE jobs SET status = 'queued', lease_agent = '', lease_until = '' WHERE id = ?",
            &[&job.str("id")],
        );
        assert_eq!(pick_worker(&s, TEST_NOW, "", &[]).unwrap(), "borealis");
        // Excluded this pass (or busy), the rotation goes on as before.
        assert_eq!(
            pick_worker(&s, TEST_NOW, "", &["borealis".into()]).unwrap(),
            "aurora"
        );
        // The work was held on this machine: a hand's slot passes borealis,
        // who stays free for the local slot that carries the clone on.
        crate::util::exec1(
            &conn,
            "INSERT INTO runs (id, job_id, agent, project, started_at, status, executor) VALUES ('run-h-held', ?, 'borealis', 'proj', ?, 'held', 'local')",
            &[&job.str("id"), &TEST_NOW],
        );
        assert_eq!(pick_worker_on(&s, "", &[], "hands-1").unwrap(), "aurora");
        assert_eq!(
            pick_worker_on(&s, "", &[], crate::executor::LOCAL_EXECUTOR).unwrap(),
            "borealis"
        );
        // Held on a hand, the session carries on nowhere: any slot may take
        // borealis.
        crate::util::exec1(
            &conn,
            "UPDATE runs SET executor = 'hands-2' WHERE id = 'run-h-held'",
            &[],
        );
        assert_eq!(pick_worker_on(&s, "", &[], "hands-1").unwrap(), "borealis");
        crate::util::exec1(
            &conn,
            "UPDATE runs SET executor = 'local' WHERE id = 'run-h-held'",
            &[],
        );
        // Once the held work is taken again, nothing pins borealis.
        crate::util::exec1(
            &conn,
            "UPDATE jobs SET status = 'done' WHERE id = ?",
            &[&job.str("id")],
        );
        assert_eq!(pick_worker(&s, TEST_NOW, "", &[]).unwrap(), "aurora");
    }
    #[test]
    fn scheduling_pick_worker_skips_provisional_completion() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        for status in ["done", "failed", "cancelled", "blocked"] {
            let s = new_store();
            new_bare_repo(&s);
            join(&s, "aurora");
            join(&s, "borealis");
            set_setting(&s, "quorum", "1");
            crate::capacity::pin_per_project(&s, 2);
            let first = scheduling_job(&s, "proj", "controller-still-running", 100);
            let second = scheduling_job(&s, "proj", "already-finished", 90);
            for (job, agent) in [(&first, "aurora"), (&second, "borealis")] {
                let leased = lease_job_id(&s, agent, job).unwrap();
                assert!(!leased.0.is_empty(), "lease {status}: {leased:?}");
                if agent == "aurora" {
                    scheduling_run(&s, job, agent);
                }
                crate::jobs::finish_job(
                    &s,
                    job.str("id"),
                    agent.into(),
                    status.into(),
                    "reported".into(),
                    "".into(),
                    leased.int("generation"),
                )
                .unwrap();
            }
            let picked = pick_worker(&s, TEST_NOW, "", &[]).unwrap();
            assert_eq!(picked, "borealis", "{status}");
            let conn = s.conn().unwrap();
            crate::util::exec1(
                &conn,
                "UPDATE runs SET status = ?, finished_at = ? WHERE job_id = ?",
                &[&status, &TEST_NOW, &first.str("id")],
            );
            let picked = pick_worker(&s, TEST_NOW, "", &[]).unwrap();
            assert_eq!(picked, "aurora", "{status}");
            assert_eq!(
                count(
                    &s,
                    "SELECT COUNT(*) AS n FROM agents WHERE kind = 'worker'",
                    &[]
                ),
                2
            );
        }
    }
    #[test]
    fn scheduling_expired_lease_preserves_generation_takeover() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        let job = scheduling_job(&s, "proj", "expired", 100);
        let first = crate::jobs::lease_job(
            &s,
            "aurora".into(),
            TEST_NOW.into(),
            crate::jobs::LeaseOpts {
                job_id: job.str("id"),
                minutes: 1,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!first.0.is_empty(), "first lease");
        let run_id = scheduling_run(&s, &job, "aurora");
        let got = crate::jobs::lease_job(
            &s,
            "aurora".into(),
            first.str("lease_until"),
            crate::jobs::LeaseOpts {
                job_id: job.str("id"),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(got.0.is_empty(), "lease retaken at deadline: {got:?}");
        let replacement = crate::jobs::lease_job(
            &s,
            "aurora".into(),
            crate::util::plus_minutes(TEST_NOW, 2.0),
            crate::jobs::LeaseOpts {
                job_id: job.str("id"),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            !replacement.0.is_empty()
                && replacement.int("generation") == 2
                && replacement.int("attempts") == 2,
            "expired takeover: {replacement:?}"
        );
        let err = crate::jobs::finish_job(
            &s,
            job.str("id"),
            "aurora".into(),
            "done".into(),
            "stale result".into(),
            "".into(),
            first.int("generation"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("Stale lease"), "{err}");
        let conn = s.conn().unwrap();
        let got = crate::util::must_one(&conn, "SELECT status FROM runs WHERE id = ?", &[&run_id])
            .str("status");
        assert_eq!(got, "running");
    }
    #[test]
    fn scheduling_fairness_includes_leases_before_run_start() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        let s = new_store();
        set_setting(&s, "capacity", "2");
        crate::capacity::pin_per_project(&s, 2);
        let conn = s.conn().unwrap();
        let mut scratch = Vec::new();
        for (i, project) in ["first", "second", "third"].iter().enumerate() {
            let root = crate::test_helpers_heartbeat::heartbeat_temp_dir();
            let dir = root.join(format!("fairness-{project}"));
            scratch.push(root);
            std::fs::create_dir_all(&dir).unwrap();
            crate::util::exec1(
                &conn,
                "INSERT INTO projects (slug, name, path, added_at) VALUES (?, ?, ?, ?)",
                &[project, project, &dir.display().to_string(), &TEST_NOW],
            );
            scheduling_job(&s, project, &format!("{project}-high"), 100 - i as i32 * 10);
            scheduling_job(&s, project, &format!("{project}-low"), 99 - i as i32 * 10);
        }
        for (i, agent) in ["aurora", "borealis", "cygnus"].iter().enumerate() {
            let got = crate::jobs::lease_job(
                &s,
                (*agent).into(),
                TEST_NOW.into(),
                crate::jobs::LeaseOpts::default(),
            )
            .unwrap();
            let project = ["first", "second", "third"][i];
            assert_eq!(got.str("project"), project, "dispatch {}: {got:?}", i + 1);
            assert_eq!(got.str("title"), format!("{project}-high"));
        }
        assert_eq!(count(&s, "SELECT COUNT(*) AS n FROM runs", &[]), 0);
        let got = crate::jobs::lease_job(
            &s,
            "draco".into(),
            TEST_NOW.into(),
            crate::jobs::LeaseOpts {
                projects: vec!["first".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(got.str("project"), "first");
        assert_eq!(got.str("title"), "first-low");
    }
    #[test]
    fn scheduling_mixed_batch_does_not_pause_on_last_error() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        let (s, first) = crate::test_helpers_heartbeat::daemon_retry_fixture();
        let second = scheduling_job(&s, "proj", "slow-error", 90);
        set_setting(&s, "capacity", "2");
        crate::capacity::pin_per_project(&s, 2);
        set_setting(&s, "jobs_per_day", "0");
        let (reflect_job, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "reflect".into(),
            "already reflected".into(),
            String::new(),
            0,
            1,
            format!("reflect:proj:{}", &TEST_NOW[..10]),
            crate::util::HUMAN_AGENT.into(),
            true,
        )
        .unwrap();
        crate::jobs::finish_job(
            &s,
            reflect_job.str("id"),
            crate::util::HUMAN_AGENT.into(),
            "done".into(),
            "already reflected".into(),
            String::new(),
            -1,
        )
        .unwrap();
        let mut failure_gate =
            crate::test_helpers_heartbeat::new_lifecycle_gate("last runner error");
        let script = format!(
            "if [ \"$HIVE_JOB\" = {} ]; then echo 'SUMMARY: ok'; exit 0; fi; \
             if [ ! -f {} ]; then {}; fi; \
             echo 'Error: Authentication token found but could not be validated.' >&2; exit 1",
            crate::util::shell_quote(&first.str("id")),
            crate::util::shell_quote(&failure_gate.ready),
            failure_gate.command
        );
        let cancel = hive_bus::CancellationToken::new()
            .with_timeout(crate::test_helpers_heartbeat::LIFECYCLE_WAIT_TIMEOUT);
        let (done_tx, done_rx) = crossbeam_channel::bounded(1);
        let (result_tx, result_rx) = crossbeam_channel::bounded(1);
        let out = crate::test_helpers::OutBuf::new();
        let waits = Arc::new(Mutex::new(Vec::<Duration>::new()));
        let daemon_store = s.clone();
        let daemon_cancel = cancel.clone();
        let daemon_out = out.writer();
        let waits_for_sleep = Arc::clone(&waits);
        std::thread::spawn(move || {
            let result = crate::daemon::daemon(
                &daemon_cancel,
                &daemon_store,
                crate::daemon::DaemonOpts {
                    interval: 7,
                    max_loops: 3,
                    grace: Duration::from_secs(1),
                    out: Some(daemon_out),
                    worker_cmd: crate::util::shell_join(&["sh".into(), "-c".into(), script]),
                    sleep: Some(Arc::new(move |d| {
                        waits_for_sleep.lock().unwrap().push(d);
                        true
                    })),
                    ..Default::default()
                },
            );
            let _ = result_tx.send(result);
            let _ = done_tx.send(());
        });
        failure_gate.wait(&done_rx);
        // The batch is mixed once the daemon has reported the successful run
        // while the failing one is still held at the gate, so wait for that
        // report (report() clears all_errors before it says the line). Not a
        // heartbeat of the daemon's second loop: a bounded daemon with one
        // run in flight waits for that run's result before the loop's
        // heartbeat (daemon.rs, `max_loops > 0 && inflight.len() == 1`), so
        // when the success was reported before loop 2 began (a loaded
        // machine) its beat came only after the gate opened, and the test
        // held the gate for it until the 60 s cancel: result=interrupted.
        let reported = format!("done {} ", first.str("id"));
        crate::test_helpers_heartbeat::lifecycle_eventually(
            "the daemon's report of the successful run",
            || out.string().lines().any(|line| line.starts_with(&reported)),
        );
        failure_gate.release();
        crate::test_helpers_heartbeat::wait_lifecycle_run(&done_rx);
        let result = result_rx.recv().unwrap();
        let waits = waits.lock().unwrap().clone();
        assert!(
            result == "done" && waits.is_empty(),
            "mixed batch paused after its last run errored: result={result} waits={waits:?}\n{}",
            out.string()
        );
        let n = count(
            &s,
            "SELECT COUNT(*) AS n FROM runs WHERE job_id = ? AND status = 'done'",
            &[&first.str("id")],
        );
        assert_eq!(n, 1, "successful runs = {n}, want 1\n{}", out.string());
        let n = count(
            &s,
            "SELECT COUNT(*) AS n FROM runs WHERE job_id = ? AND status = 'error'",
            &[&second.str("id")],
        );
        assert_eq!(
            n,
            2,
            "prompt retries after mixed batch = {n}, want 2\n{}",
            out.string()
        );
        cancel.cancel();
    }

    #[test]
    fn worker_lifecycle_delayed_suggestion() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        const STARTUP_FAILURE: &str = "injected missed startup heartbeat";
        for (name, store_reported, takeover) in [
            ("output_completion", false, ""),
            ("store_completion_before_exit", true, ""),
            ("lease_retaken_during_worker", false, "worker"),
            ("lease_retaken_during_verification", false, "verification"),
        ] {
            hive_bus::reset_default();
            let s = new_store();
            let (_, root) = new_bare_repo(&s);
            let _env_3 = hive_testutil::setenv("HIVE_SESSION", "");
            set_setting(&s, "minutes_per_job", "1");
            set_setting(&s, "jobs_per_day", "0");
            set_setting(&s, "quorum", "2");
            for agent in [
                "proposer",
                "voter-one",
                "voter-two",
                "quiet",
                "rival",
                "observer",
            ] {
                join(&s, agent);
            }
            let suggestion = crate::board::create_post(
                &s,
                TEST_NOW.into(),
                crate::board::PostOpts {
                    kind: "suggestion".into(),
                    project: "proj".into(),
                    area: "evolution".into(),
                    thread: "slow-lifecycle".into(),
                    agent: crate::board::actor_agent(&s, "proposer".into()),
                    session: "proposal-session".into(),
                    summary: "Keep slow worker lifecycle stable".into(),
                    body: "One variable: lifecycle stability. Signal: README.md".into(),
                    ..Default::default()
                },
            )
            .unwrap();
            for agent in ["voter-one", "voter-two"] {
                crate::board::create_post(
                    &s,
                    TEST_NOW.into(),
                    crate::board::PostOpts {
                        kind: "vote".into(),
                        project: "proj".into(),
                        area: "evolution".into(),
                        thread: "slow-lifecycle".into(),
                        agent: crate::board::actor_agent(&s, agent.into()),
                        session: format!("{agent}-session"),
                        on: suggestion.str("id"),
                        vote: "up".into(),
                        summary: "A bounded lifecycle regression guard".into(),
                        ..Default::default()
                    },
                )
                .unwrap();
            }
            crate::test_helpers_heartbeat::assert_lifecycle_suggestion(
                &s,
                &suggestion.str("id"),
                TEST_NOW,
                "eligible",
                false,
                true,
            );
            let planned =
                crate::plan::plan_jobs(&s, "proj".into(), TEST_NOW.into(), "daemon".into())
                    .unwrap();
            assert_eq!(planned.len(), 1, "plan {name}: {planned:?}");
            let job = planned[0].clone();
            assert_eq!(job.str("kind"), "improve", "{name}");
            assert_eq!(
                job.str("source"),
                format!("suggestion:{}", suggestion.str("id")),
                "{name}"
            );
            let again = crate::plan::plan_jobs(&s, "proj".into(), TEST_NOW.into(), "daemon".into())
                .unwrap();
            assert!(again.is_empty(), "planned twice before dispatch: {again:?}");

            let fake = crate::test_helpers_heartbeat::fake_heartbeat_clock();
            let missed_startup = Arc::new(AtomicBool::new(false));
            let missed_startup_for_now = Arc::clone(&missed_startup);
            let now_clock = Arc::clone(&fake.clock);
            *crate::heartbeat::HEARTBEAT_NOW
                .write()
                .expect("heartbeat now lock") = Box::new(move |_| {
                if !missed_startup_for_now.swap(true, Ordering::SeqCst) {
                    return Err(anyhow::anyhow!(STARTUP_FAILURE));
                }
                Ok(chrono::DateTime::<chrono::Utc>::from_timestamp_nanos(
                    now_clock.load(Ordering::SeqCst),
                ))
            });
            let mut worker_gate = crate::test_helpers_heartbeat::new_lifecycle_gate("worker exit");
            let mut verify_gate = crate::test_helpers_heartbeat::new_lifecycle_gate("verification");
            let check = format!(
                "{}; test \"$(git show HEAD:lifecycle.txt)\" = 'slow lifecycle'",
                verify_gate.command
            );
            crate::util::exec1(
                &s.conn().unwrap(),
                "UPDATE projects SET verify_cmd = ? WHERE slug = 'proj'",
                &[&check],
            );
            let mut script = format!(
                "set -e; printf 'slow lifecycle\\n' > lifecycle.txt; git add lifecycle.txt; \
                 git commit -qm 'lifecycle fixture'; {}",
                worker_gate.command
            );
            if !store_reported {
                script.push_str("; printf 'SUMMARY: slow lifecycle complete\\n'");
            }
            let worker = crate::util::shell_join(&["sh".into(), "-c".into(), script]);
            let base = run(&root, &["git", "rev-parse", "HEAD"]).trim().to_string();
            let cancel = hive_bus::CancellationToken::new();
            let (done_tx, done_rx) = crossbeam_channel::bounded(1);
            let (result_tx, result_rx) = crossbeam_channel::bounded(1);
            let run_store = s.clone();
            let run_cancel = cancel.clone();
            let job_id = job.str("id");
            std::thread::spawn(move || {
                let out = work_once(
                    &run_store,
                    "quiet",
                    "proj",
                    &job_id,
                    TEST_NOW,
                    RunOpts {
                        worker_cmd: worker,
                        cancel: run_cancel,
                        ..Default::default()
                    },
                );
                let _ = result_tx.send(out);
                let _ = done_tx.send(());
            });
            worker_gate.wait(&done_rx);
            assert_eq!(
                crate::test_helpers_heartbeat::heartbeat_receive(&fake.periods),
                hive_board::HEARTBEAT_INTERVAL,
                "heartbeat period {name}"
            );
            crate::test_helpers_heartbeat::heartbeat_eventually(|| {
                missed_startup.load(Ordering::SeqCst)
            });
            let (h, exists) = crate::test_helpers_heartbeat::runner_heartbeat(&s, "quiet").unwrap();
            assert!(
                !exists,
                "failed startup observation produced evidence: {h:?}"
            );
            let leased = crate::jobs::job_row(&s.conn().unwrap(), job.str("id"));

            let beat_at = chrono::DateTime::<chrono::Utc>::from_timestamp_nanos(
                fake.clock.fetch_add(
                    hive_board::HEARTBEAT_INTERVAL.as_nanos() as i64,
                    Ordering::SeqCst,
                ) + hive_board::HEARTBEAT_INTERVAL.as_nanos() as i64,
            );
            fake.ticks.send(beat_at).unwrap();
            crate::test_helpers_heartbeat::heartbeat_eventually(|| {
                let (h, _) = crate::test_helpers_heartbeat::runner_heartbeat(&s, "quiet").unwrap();
                h.at == hive_store::rfc3339_nano(beat_at)
            });
            let (first, _) = crate::test_helpers_heartbeat::runner_heartbeat(&s, "quiet").unwrap();
            assert_eq!(leased.str("status"), "leased", "initial lifecycle {name}");
            assert_eq!(leased.int("generation"), 1, "initial lifecycle {name}");
            assert_eq!(leased.int("attempts"), 1, "initial lifecycle {name}");
            assert_eq!(first.job_id, job.str("id"), "initial lifecycle {name}");
            assert_eq!(first.agent, "quiet", "initial lifecycle {name}");
            assert_eq!(first.generation, 1, "initial lifecycle {name}");
            assert!(first.stopped_at.is_empty(), "initial lifecycle {name}");

            let edge = beat_at + hive_board::dead_after(&s);
            fake.clock.store(
                (edge - chrono::Duration::nanoseconds(1))
                    .timestamp_nanos_opt()
                    .unwrap(),
                Ordering::SeqCst,
            );
            crate::test_helpers_heartbeat::assert_lifecycle_presence(
                &s,
                chrono::DateTime::<chrono::Utc>::from_timestamp_nanos(
                    fake.clock.load(Ordering::SeqCst),
                ),
                "online",
            );
            fake.clock
                .store(edge.timestamp_nanos_opt().unwrap(), Ordering::SeqCst);
            crate::test_helpers_heartbeat::assert_lifecycle_presence(&s, edge, "dead");
            assert_eq!(
                hive_store::rfc3339_nano(edge),
                leased.str("lease_until"),
                "fixture missed the lease boundary"
            );
            let other = crate::jobs::lease_job(
                &s,
                "rival".into(),
                hive_store::rfc3339_nano(edge),
                crate::jobs::LeaseOpts {
                    job_id: job.str("id"),
                    minutes: 1,
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(
                other.0.is_empty(),
                "lease stolen at its exact deadline: {other:?}"
            );
            fake.ticks.send(edge).unwrap();
            crate::test_helpers_heartbeat::heartbeat_eventually(|| {
                let (h, _) = crate::test_helpers_heartbeat::runner_heartbeat(&s, "quiet").unwrap();
                h.at == hive_store::rfc3339_nano(edge)
            });
            crate::test_helpers_heartbeat::assert_lifecycle_presence(&s, edge, "online");
            let current = crate::jobs::job_row(&s.conn().unwrap(), job.str("id"));
            assert_eq!(
                current.0, leased.0,
                "missed/recovered heartbeat changed the lease"
            );
            assert_eq!(
                count(
                    &s,
                    "SELECT COUNT(*) AS n FROM events WHERE action = 'job.renew' AND subject = ?",
                    &[&job.str("id")]
                ),
                0,
                "heartbeat renewed the lease"
            );

            let mut replacement = Row::new();
            let mut replacement_at = String::new();
            let mut takeover_fn = || {
                let current = crate::jobs::job_row(&s.conn().unwrap(), job.str("id"));
                let until =
                    chrono::DateTime::parse_from_rfc3339(&current.str("lease_until")).unwrap();
                let mut at = chrono::DateTime::<chrono::Utc>::from_timestamp_nanos(
                    fake.clock.load(Ordering::SeqCst),
                );
                if at < until {
                    at = until.with_timezone(&chrono::Utc);
                }
                at += chrono::Duration::seconds(1);
                fake.clock
                    .store(at.timestamp_nanos_opt().unwrap(), Ordering::SeqCst);
                replacement_at = hive_store::rfc3339_nano(at);
                replacement = crate::jobs::lease_job(
                    &s,
                    "quiet".into(),
                    replacement_at.clone(),
                    crate::jobs::LeaseOpts {
                        job_id: job.str("id"),
                        minutes: 1,
                        ..Default::default()
                    },
                )
                .unwrap();
                assert!(
                    !replacement.0.is_empty()
                        && replacement.int("generation") == first.generation as i32 + 1
                        && replacement.int("attempts") == 2,
                    "expired lease takeover: {replacement:?}"
                );
                let (before, _) =
                    crate::test_helpers_heartbeat::runner_heartbeat(&s, "quiet").unwrap();
                let late = hive_board::ExecutionHeartbeat {
                    identity: first.identity.clone(),
                    at: replacement_at.clone(),
                    ..Default::default()
                };
                assert!(
                    !hive_board::record_heartbeat(&s, late).unwrap(),
                    "stale heartbeat accepted"
                );
                assert!(
                    !hive_board::stop_heartbeat(&s, first.identity.clone(), at).unwrap(),
                    "stale stop accepted"
                );
                assert!(
                    !crate::jobs::renew_lease(
                        &s.conn().unwrap(),
                        replacement_at.clone(),
                        job.str("id"),
                        "quiet".into(),
                        first.generation as i32,
                        5
                    ),
                    "old generation renewed the replacement lease"
                );
                let err = crate::jobs::finish_job(
                    &s,
                    job.str("id"),
                    "quiet".into(),
                    "done".into(),
                    "late result".into(),
                    String::new(),
                    first.generation as i32,
                )
                .unwrap_err();
                assert!(
                    err.to_string().contains("Stale lease"),
                    "old generation finished the replacement job: {err}"
                );
                let (after, _) =
                    crate::test_helpers_heartbeat::runner_heartbeat(&s, "quiet").unwrap();
                assert_eq!(after, before, "stale observations changed heartbeat");
                let current = crate::jobs::job_row(&s.conn().unwrap(), job.str("id"));
                assert_eq!(current.0, replacement.0, "stale work changed replacement");
            };
            if takeover == "worker" {
                takeover_fn();
            }
            if store_reported {
                crate::jobs::finish_job(
                    &s,
                    job.str("id"),
                    "quiet".into(),
                    "done".into(),
                    "slow lifecycle complete".into(),
                    String::new(),
                    first.generation as i32,
                )
                .unwrap();
                let at = chrono::DateTime::<chrono::Utc>::from_timestamp_nanos(
                    fake.clock.fetch_add(
                        hive_board::HEARTBEAT_INTERVAL.as_nanos() as i64,
                        Ordering::SeqCst,
                    ) + hive_board::HEARTBEAT_INTERVAL.as_nanos() as i64,
                );
                fake.ticks.send(at).unwrap();
                crate::test_helpers_heartbeat::heartbeat_eventually(|| {
                    let (h, _) =
                        crate::test_helpers_heartbeat::runner_heartbeat(&s, "quiet").unwrap();
                    h.at == hive_store::rfc3339_nano(at)
                });
                crate::test_helpers_heartbeat::assert_lifecycle_presence(&s, at, "online");
            }
            worker_gate.release();
            if takeover != "worker" {
                verify_gate.wait(&done_rx);
                let r = crate::util::must_one(
                    &s.conn().unwrap(),
                    "SELECT * FROM runs WHERE id = ?",
                    &[&first.run_id],
                );
                assert_eq!(r.str("status"), "running", "run ended before verification");
                let (stopped, _) =
                    crate::test_helpers_heartbeat::runner_heartbeat(&s, "quiet").unwrap();
                assert!(
                    !stopped.stopped_at.is_empty(),
                    "worker heartbeat did not stop before verification"
                );
                fake.clock.fetch_add(
                    (2 * hive_board::HEARTBEAT_INTERVAL.as_nanos()) as i64,
                    Ordering::SeqCst,
                );
                // Stopped two beats ago but well inside `dead_after`: the worker stays
                // online while its run is verified, so no sweep can take the run away.
                crate::test_helpers_heartbeat::assert_lifecycle_presence(
                    &s,
                    chrono::DateTime::<chrono::Utc>::from_timestamp_nanos(
                        fake.clock.load(Ordering::SeqCst),
                    ),
                    "online",
                );
                if takeover == "verification" {
                    takeover_fn();
                }
                verify_gate.release();
            }
            crate::test_helpers_heartbeat::wait_lifecycle_run(&done_rx);
            let (result, reason) = result_rx
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .unwrap();
            assert!(!result.0.is_empty(), "work result {name}");
            assert!(reason.is_empty(), "work reason {name}: {reason}");
            let log = std::fs::read_to_string(result.str("log")).unwrap();
            assert!(
                log.contains(&format!("presence unavailable: {STARTUP_FAILURE}")),
                "missed startup heartbeat was not reported:\n{log}"
            );

            let mut successful = result.clone();
            let mut want_runs = 1;
            if !takeover.is_empty() {
                assert_eq!(result.str("status"), "stale", "old run was not fenced");
                assert_eq!(result.str("branch"), "", "old run was not fenced");
                assert_eq!(result.int("commits"), 1, "old run was not fenced");
                let refs = run(
                    &root,
                    &[
                        "git",
                        "for-each-ref",
                        "--format=%(refname)",
                        "refs/heads/hive/",
                    ],
                );
                assert!(refs.is_empty(), "stale run landed a branch: {refs}");
                let current = crate::jobs::job_row(&s.conn().unwrap(), job.str("id"));
                assert_eq!(
                    current.0, replacement.0,
                    "old run overwrote replacement lease"
                );
                crate::test_helpers_heartbeat::assert_lifecycle_suggestion(
                    &s,
                    &suggestion.str("id"),
                    &replacement_at,
                    "eligible",
                    false,
                    true,
                );
                worker_gate.release();
                verify_gate.release();
                successful = run_job(
                    &s,
                    replacement.clone(),
                    "quiet",
                    &replacement_at,
                    RunOpts {
                        worker_cmd: crate::util::shell_join(&[
                            "sh".into(),
                            "-c".into(),
                            format!(
                                "set -e; printf 'slow lifecycle\\n' > lifecycle.txt; git add lifecycle.txt; \
                                 git commit -qm 'lifecycle fixture'; {}; printf 'SUMMARY: slow lifecycle complete\\n'",
                                worker_gate.command
                            ),
                        ]),
                        cancel: cancel.clone(),
                        ..Default::default()
                    },
                )
                .unwrap();
                want_runs += 1;
            }
            assert_eq!(successful.str("status"), "done", "successful run");
            assert_eq!(successful.int("commits"), 1, "successful run");
            if takeover == "verification" {
                // The fenced run's check passed on this very tree and left
                // its memo; the replacement commits the identical fixture
                // and skips the identical check.
                assert!(
                    successful
                        .str("verify")
                        .starts_with("verified earlier by run at "),
                    "successful run: {:?}",
                    successful.str("verify")
                );
            } else {
                assert_eq!(successful.str("verify"), "verify exit 0", "successful run");
            }
            assert_eq!(
                successful.str("branch"),
                format!("hive/quiet/{}", job.str("id")),
                "successful run"
            );
            assert!(!successful.str("finished_at").is_empty(), "successful run");
            let finished = crate::jobs::job_row(&s.conn().unwrap(), job.str("id"));
            assert_eq!(finished.str("status"), "done", "job completion");
            assert_eq!(finished.str("lease_until"), "", "job completion");
            assert_eq!(finished.int("generation"), want_runs, "job completion");
            assert_eq!(finished.int("attempts"), want_runs, "job completion");
            assert!(!finished.str("finished_at").is_empty(), "job completion");
            assert_eq!(
                finished.str("branch"),
                successful.str("branch"),
                "job completion"
            );
            assert_eq!(
                finished.str("result"),
                successful.str("summary"),
                "job completion"
            );
            assert!(
                finished.str("result").contains("slow lifecycle complete"),
                "job completion"
            );
            assert_eq!(
                count(
                    &s,
                    "SELECT COUNT(*) AS n FROM runs WHERE job_id = ?",
                    &[&job.str("id")]
                ),
                want_runs,
                "runs"
            );
            assert_eq!(
                count(
                    &s,
                    "SELECT COUNT(*) AS n FROM runs WHERE job_id = ? AND status = 'done'",
                    &[&job.str("id")]
                ),
                1,
                "successful runs"
            );
            assert_eq!(
                count(
                    &s,
                    "SELECT COUNT(*) AS n FROM posts WHERE kind = 'result' AND area = 'jobs' AND thread = ?",
                    &[&hive_board::slugify(&job.str("id"))]
                ),
                1,
                "result posts"
            );
            let agent = crate::board::agent_row(&s.conn().unwrap(), "quiet".into());
            assert_eq!(agent.int("jobs_done"), 1, "worker completion counted");
            assert_eq!(agent.int("jobs_failed"), 0, "worker completion counted");
            let (final_beat, _) =
                crate::test_helpers_heartbeat::runner_heartbeat(&s, "quiet").unwrap();
            assert_eq!(final_beat.run_id, successful.str("id"), "final heartbeat");
            assert_eq!(final_beat.generation, want_runs as i64, "final heartbeat");
            assert!(!final_beat.stopped_at.is_empty(), "final heartbeat");
            let now = chrono::DateTime::<chrono::Utc>::from_timestamp_nanos(
                fake.clock.load(Ordering::SeqCst),
            );
            crate::test_helpers_heartbeat::assert_lifecycle_presence(&s, now, "idle");
            assert!(
                !hive_board::record_heartbeat(
                    &s,
                    hive_board::ExecutionHeartbeat {
                        identity: first.identity.clone(),
                        at: hive_store::rfc3339_nano(
                            now + chrono::Duration::from_std(hive_board::HEARTBEAT_INTERVAL)
                                .unwrap(),
                        ),
                        ..Default::default()
                    }
                )
                .unwrap(),
                "finished execution returned online"
            );
            let (after, _) = crate::test_helpers_heartbeat::runner_heartbeat(&s, "quiet").unwrap();
            assert_eq!(after, final_beat, "late heartbeat replaced final evidence");
            assert!(
                !crate::jobs::renew_lease(
                    &s.conn().unwrap(),
                    hive_store::rfc3339_nano(now),
                    job.str("id"),
                    "quiet".into(),
                    finished.int("generation"),
                    5
                ),
                "completed job reacquired a lease"
            );
            crate::test_helpers_heartbeat::assert_lifecycle_suggestion(
                &s,
                &suggestion.str("id"),
                &hive_store::rfc3339_nano(now),
                "built",
                false,
                false,
            );
            for _ in 0..2 {
                crate::plan::plan_jobs(
                    &s,
                    "proj".into(),
                    hive_store::rfc3339_nano(now),
                    "daemon".into(),
                )
                .unwrap();
            }
            assert_eq!(
                count(
                    &s,
                    "SELECT COUNT(*) AS n FROM jobs WHERE source = ?",
                    &[&job.str("source")]
                ),
                1,
                "suggestion planned repeatedly"
            );
            let (again, why) = work_once(
                &s,
                "quiet",
                "proj",
                &job.str("id"),
                &hive_store::rfc3339_nano(now),
                RunOpts {
                    worker_cmd: crate::util::shell_join(&[
                        "sh".into(),
                        "-c".into(),
                        "printf 'SUMMARY: slow lifecycle complete\\n'".into(),
                    ]),
                    cancel: cancel.clone(),
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(
                again.0.is_empty() && why.contains("no queued job"),
                "completed suggestion dispatched again: {again:?}, {why:?}"
            );
            assert_eq!(
                run(&root, &["git", "rev-parse", "HEAD"]).trim(),
                base,
                "worker moved the project's default branch"
            );
            let refs = run(
                &root,
                &[
                    "git",
                    "for-each-ref",
                    "--format=%(refname:short)",
                    "refs/heads/hive/",
                ],
            )
            .trim()
            .to_string();
            assert_eq!(refs, successful.str("branch"), "result branch");
            let content = run(
                &root,
                &[
                    "git",
                    "show",
                    &format!("{}:lifecycle.txt", successful.str("branch")),
                ],
            );
            assert_eq!(content, "slow lifecycle\n", "landed content");
            let left = work_leftovers(&s.home);
            assert!(left.is_empty(), "worker left temporary state: {left:?}");
            cancel.cancel();
            drop(fake);
        }
    }

    fn set_setting(s: &hive_store::Store, key: &str, value: &str) {
        s.set_setting(crate::util::HUMAN_AGENT, key, value).unwrap();
    }

    fn scheduling_job(s: &hive_store::Store, project: &str, title: &str, priority: i32) -> Row {
        crate::jobs::add_job(
            s,
            project.into(),
            "task".into(),
            title.into(),
            "exercise scheduling".into(),
            0,
            priority,
            format!("scheduling:{title}"),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .unwrap()
        .0
    }

    fn scheduling_run(s: &hive_store::Store, job: &Row, agent: &str) -> String {
        let id = format!("run-{}", job.str("id"));
        let conn = s.conn().unwrap();
        crate::util::exec1(
            &conn,
            "INSERT INTO runs (id, job_id, agent, project, started_at, status) VALUES (?, ?, ?, ?, ?, 'running')",
            &[&id, &job.str("id"), &agent, &job.str("project"), &TEST_NOW],
        );
        id
    }

    fn lease_job_id(s: &hive_store::Store, agent: &str, job: &Row) -> Result<Row> {
        crate::jobs::lease_job(
            s,
            agent.into(),
            TEST_NOW.into(),
            crate::jobs::LeaseOpts {
                job_id: job.str("id"),
                ..Default::default()
            },
        )
    }
}
