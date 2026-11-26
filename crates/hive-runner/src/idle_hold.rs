//! A daemon-wide idle hold tells the owner when it keeps the owner's work
//! waiting.
//!
//! The daily cap (daily_cap.rs, e4bc527b) was the first hold to say so: on
//! 2026-11-03 every slot idled behind it with one deduped log line an hour
//! while the owner nudged tasks that could not start. The other holds that
//! stop the whole daemon had the same silence: the load governor's hold
//! (`daemon_load`: load over the line, memory under the floor), the disk
//! floor, the bulk volume away (bulk.rs), the restart drain (restart.rs) and
//! every slot busy with the fleet's own work. Each logs its reason through
//! the idle log (idle_log.rs) and nothing else.
//!
//! This module is the daily cap's pattern made shared, so no hold carries a
//! copy of it: [`ask`] posts one coalesced owner question on the brain
//! project through `hygiene::post_daemon_owner_question`, [`answer`] answers
//! every open one of a kind, and [`IdleHolds`] decides, once per tick, from
//! the holds the dispatch saw ([`Hold`]) and the owner's queued work
//! ([`owner_waiting`]), which kinds to ask about and which to answer. A
//! hold asks the first tick it has stood longer than one planning pass
//! (`daemon.pass_seconds`) while the owner's work is still queued after the
//! tick's dispatch, and its question is answered the tick the hold lifts or
//! the owner's work no longer waits behind it.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::util::Row;

/// The event a hold's question writes on the brain project.
pub(crate) const EVENT: &str = "capacity.idle-hold";
/// The area every hold's thread sits in.
pub(crate) const AREA: &str = "runner";

/// One kind of daemon-wide hold: its coalesce kind (also its thread), the
/// words the owner reads and how long the owner's oldest queued job must
/// have waited before the hold is worth a question.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Kind {
    /// The question's `coalesce_kind` and its thread under [`AREA`].
    pub(crate) ask_kind: &'static str,
    /// What holds, as the summary starts: "machine load holds ...".
    pub(crate) label: &'static str,
    /// What the owner can do about it.
    pub(crate) cure: &'static str,
    /// What the answer says once the hold itself is gone.
    pub(crate) lifted: &'static str,
    /// Minutes the owner's oldest queued job must have waited, beyond the
    /// one planning pass every kind waits.
    pub(crate) owner_wait_minutes: i64,
}

/// Load over the governor's line (`daemon_load::hold_reason`).
pub(crate) static LOAD: Kind = Kind {
    ask_kind: "idle-load",
    label: "machine load",
    cure: "The load governor starts nothing new while the one-minute load is over its line (twice the CPUs; `daemon.max_load` is automatic now), and at twice that line not even the owner's express slot opens. Close what else loads this machine, or lower `capacity` so the fleet's own builds load it less.",
    lifted: "the load is under the line again",
    owner_wait_minutes: 0,
};

/// Free memory under the governor's floor, or the kernel's critical level.
pub(crate) static MEMORY: Kind = Kind {
    ask_kind: "idle-memory",
    label: "low memory",
    cure: "The load governor starts nothing new while free memory is under its floor (`daemon.min_free_mb` of memory is automatic now, 1024 MB) or the kernel reports critical pressure. Close what holds the memory, or lower `capacity` so fewer runs share it.",
    lifted: "free memory is over the floor again",
    owner_wait_minutes: 0,
};

/// The work disk under `daemon.min_free_mb`. The reclaim ladder answers a
/// crossing on the next maintenance pass, so the owner hears only when it
/// did not clear it in the time the disk-floor finding waits.
pub(crate) static DISK_FLOOR: Kind = Kind {
    ask_kind: "idle-disk-floor",
    label: "the disk floor",
    cure: "Nothing new starts while the work filesystem has less free space than `daemon.min_free_mb`; the reclaim ladder has run and did not clear it. Free space on that disk, or set `daemon.min_free_mb` to the floor this machine should keep (Settings in the console).",
    lifted: "the disk is over the floor again",
    owner_wait_minutes: crate::daemon::DISK_FLOOR_WANTED_MINUTES as i64,
};

/// The bulk volume (bulk.rs) is not mounted.
pub(crate) static BULK_AWAY: Kind = Kind {
    ask_kind: "idle-bulk-away",
    label: "the bulk disk away",
    cure: "The clones, caches and worktrees live on another disk (`hive bulk status`) and nothing starts while it is away. Plug the disk back in; nothing is lost. To work without it: `hive bulk detach --lost` starts with empty dirs.",
    lifted: "the bulk disk is back",
    owner_wait_minutes: 0,
};

/// The restart window (restart.rs): a console drain starts nothing new.
pub(crate) static RESTART_DRAIN: Kind = Kind {
    ask_kind: "idle-restart-drain",
    label: "the restart drain",
    cure: "A restart is pending and the daemon drains the runs in flight before it exits for the service to restart it (`daemon.restart_drain_minutes`); the next daemon starts your work first. To restart sooner, ask for the restart now from the console's daemon controls (the runs in flight hold and carry on later).",
    lifted: "the restart drain is over",
    owner_wait_minutes: 0,
};

/// Every local slot holds the fleet's own work, and the owner's work did
/// not get the express slot either.
pub(crate) static FLEET_FULL: Kind = Kind {
    ask_kind: "idle-fleet-full",
    label: "the fleet's own work",
    cure: "Every slot of this machine runs the fleet's own work and your job did not get the express slot. Raise `capacity` if the machine has room, pair another machine (`hive executor add`), or let the runs in flight finish.",
    lifted: "a slot is free for the owner's work again",
    owner_wait_minutes: 30,
};

/// Every kind, in the order they are tended.
pub(crate) static KINDS: [&Kind; 6] = [
    &LOAD,
    &MEMORY,
    &DISK_FLOOR,
    &BULK_AWAY,
    &RESTART_DRAIN,
    &FLEET_FULL,
];

/// One hold the tick's dispatch saw: its kind and the idle reason it logged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Hold {
    pub(crate) kind: &'static Kind,
    pub(crate) reason: String,
}

impl Hold {
    pub(crate) fn new(kind: &'static Kind, reason: impl Into<String>) -> Self {
        Self {
            kind,
            reason: reason.into(),
        }
    }
}

/// The governor's hold line as a kind: the load, else the memory.
pub(crate) fn governor_kind(hold: &str) -> &'static Kind {
    if hold.contains(&format!("{}load ", crate::daemon_load::HOLD_PREFIX)) {
        &LOAD
    } else {
        &MEMORY
    }
}

/// Whether `kind` is the load governor's hold (load or memory), which the
/// owner's work can pass through the reservation.
fn is_governor(kind: &Kind) -> bool {
    kind.ask_kind == LOAD.ask_kind || kind.ask_kind == MEMORY.ask_kind
}

/// This machine as the load governor saw it on one tick, after the
/// dispatch: the limits and pressure the dispatch read, the local runs in
/// flight, the experiment lane's measurements, the slots kept for a
/// self-update build, and the owner-driven runs among the local ones.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GovernorTick {
    pub(crate) limits: crate::daemon_load::Limits,
    pub(crate) pressure: crate::daemon_load::Pressure,
    pub(crate) running: usize,
    pub(crate) measuring: usize,
    pub(crate) reserved: usize,
    pub(crate) owner_running: usize,
}

/// Whether the load governor's hold actually refuses the owner's next run.
/// A load or memory hold alone no longer blocks the owner: the reservation
/// (`daemon_load::reserve_for_owner`, docs/owner-reservation.md) lets the
/// owner's one run start over the line, and under a soft hold the express
/// slot takes a second while the cap - halved under the kernel's memory
/// warning - has room. The answer is the governor's own
/// (`daemon_load::owner_refusal`, the one the yield asks too) on the tick's
/// real counts, so the owner is asked only about a hold the dispatch would
/// not let the owner's work through; otherwise whatever keeps the owner's
/// job queued is not the machine's load, and asking about it is noise.
pub(crate) fn governor_refuses_owner(tick: &GovernorTick) -> bool {
    crate::daemon_load::owner_refusal(
        &tick.limits,
        &tick.pressure,
        tick.running,
        tick.measuring,
        tick.reserved,
        tick.owner_running,
    )
    .is_some()
}

/// Whether runs are going and every one of them is the fleet's own work
/// (none `jobs::OWNER_DRIVEN_SQL`): the slots a dispatch found all taken
/// are then the fleet's, on this machine and on paired ones alike.
pub(crate) fn fleet_holds_every_slot(s: &hive_store::Store) -> bool {
    let Ok(conn) = s.conn() else {
        return false;
    };
    let row = crate::util::must_one(
        &conn,
        &format!(
            "SELECT COUNT(*) AS n, COALESCE(SUM(CASE WHEN j.id IS NOT NULL AND {} THEN 1 ELSE 0 END), 0) AS owner FROM runs r LEFT JOIN jobs j ON j.id = r.job_id WHERE r.status = 'running'",
            crate::jobs::OWNER_DRIVEN_SQL
        ),
        &[],
    );
    row.int("n") > 0 && row.int("owner") == 0
}

/// The owner's work queued in active projects after the tick's dispatch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct OwnerWait {
    /// Owner-driven jobs queued (`jobs::OWNER_DRIVEN_SQL`).
    pub(crate) queued: i32,
    /// The one of them queued longest.
    pub(crate) oldest: String,
    /// How long it has waited, in seconds (`jobs::job_wait_seconds`).
    pub(crate) seconds: i64,
}

/// The owner's queued work right now.
pub(crate) fn owner_waiting(s: &hive_store::Store, now: &str) -> OwnerWait {
    let Ok(conn) = s.conn() else {
        return OwnerWait::default();
    };
    let rows = crate::util::must_rows(
        &conn,
        &format!(
            "SELECT j.* FROM jobs j JOIN projects p ON p.slug = j.project WHERE j.status = 'queued' AND p.status = 'active' AND {}",
            crate::jobs::OWNER_DRIVEN_SQL
        ),
        &[],
    );
    let mut wait = OwnerWait {
        queued: rows.len() as i32,
        ..Default::default()
    };
    for job in &rows {
        let seconds = crate::jobs::job_wait_seconds(&*conn, job, now);
        if wait.oldest.is_empty() || seconds > wait.seconds {
            wait.oldest = job.str("id");
            wait.seconds = seconds;
        }
    }
    wait
}

/// One owner question of a daemon-wide hold.
pub(crate) struct Ask<'a> {
    pub(crate) brain: &'a Row,
    pub(crate) ask_kind: &'a str,
    pub(crate) thread: &'a str,
    pub(crate) summary: String,
    pub(crate) body: String,
    /// The event written with the post, and its detail (the post id is added).
    pub(crate) event: &'a str,
    pub(crate) detail: serde_json::Value,
    pub(crate) now: &'a str,
    pub(crate) actor: &'a str,
}

/// Posts the owner question, once per open hold: the key coalesces on the
/// project and kind with numbers squashed, so the same hold a tick later is
/// the same question. Returns the post made, or an empty row when one is
/// open already or the brain is unset.
pub(crate) fn ask(s: &hive_store::Store, a: Ask<'_>) -> Row {
    if a.brain.0.is_empty() {
        return Row::default();
    }
    let slug = a.brain.str("slug");
    let Ok(Some(post)) = crate::hygiene::post_daemon_owner_question(
        s,
        crate::hygiene::DaemonQuestionAsk {
            project: slug.clone(),
            ask_kind: a.ask_kind.into(),
            area: AREA.into(),
            thread: a.thread.into(),
            summary: crate::util::trunc(&a.summary, 230),
            body: a.body,
            actor: a.actor.into(),
            now: a.now.into(),
        },
    ) else {
        return Row::default();
    };
    if let Ok(conn) = s.conn() {
        let mut detail = a.detail;
        if let Some(map) = detail.as_object_mut() {
            map.insert("post".into(), serde_json::json!(post.str("id")));
        }
        crate::util::event_at(&conn, a.now, a.actor, a.event, &slug, detail);
    }
    post
}

/// Answers every open question of `ask_kind` on the brain project: the
/// summary is `summary`, the body `body` with the question it answers.
/// Returns the posts answered.
pub(crate) fn answer(
    s: &hive_store::Store,
    brain: &Row,
    ask_kind: &str,
    summary: &str,
    body: &str,
    now: &str,
    actor: &str,
) -> usize {
    if brain.0.is_empty() {
        return 0;
    }
    let slug = brain.str("slug");
    let mut closed = 0;
    for q in crate::hygiene::open_daemon_questions_of_kind(s, &slug, ask_kind) {
        let id = q.str("id");
        if crate::board::create_post(
            s,
            now.to_string(),
            crate::board::PostOpts {
                kind: "answer".into(),
                project: slug.clone(),
                area: q.str("area"),
                thread: q.str("thread"),
                summary: crate::util::trunc(summary, 230),
                body: format!("{body}\nre: {id}\n"),
                resolution: "answered".into(),
                on: id,
                agent: crate::board::actor_agent(s, actor.to_string()),
                date: now.to_string(),
                ..Default::default()
            },
        )
        .is_ok()
        {
            closed += 1;
        }
    }
    closed
}

/// What one kind does this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// The hold keeps the owner's work waiting past its bounds: ask.
    Holding,
    /// The hold stands with the owner's work queued, not yet long enough:
    /// neither ask nor answer (a daemon that restarts mid-hold keeps the
    /// question open rather than answer and ask it again).
    Pending,
    /// No hold of this kind, or no owner's work behind it: answer.
    Clear,
}

/// The holds' memory across ticks: when each kind's hold was first seen
/// (it asks once it has stood one pass, the owner's work behind it on an
/// earlier tick too) and whether
/// the owner was last asked (`Some(true)`) or answered (`Some(false)`);
/// unknown on the first tick, when a question from before a restart may be
/// open.
#[derive(Debug, Default)]
pub(crate) struct IdleHolds {
    since: BTreeMap<&'static str, Instant>,
    /// When the owner's work was first seen queued behind each kind's hold.
    waited: BTreeMap<&'static str, Instant>,
    asked: BTreeMap<&'static str, bool>,
}

impl IdleHolds {
    /// One tick: `holds` are what the dispatch saw, `governor` the machine
    /// as the governor saw it, `owner` the owner's work still queued after
    /// it, `pass` one planning pass. Asks and answers on the brain project
    /// and returns the lines to log.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn tend(
        &mut self,
        s: &hive_store::Store,
        brain: &Row,
        holds: &[Hold],
        governor: &GovernorTick,
        owner: &OwnerWait,
        pass: Duration,
        now: &str,
        actor: &str,
        at: Instant,
    ) -> Vec<String> {
        let mut lines = Vec::new();
        // Asked once per tick, and only while a governor hold stands with
        // the owner's work queued behind it.
        let mut refused: Option<bool> = None;
        for kind in KINDS.iter().copied() {
            let hold = holds.iter().find(|h| h.kind.ask_kind == kind.ask_kind);
            let passable = hold.is_some()
                && owner.queued > 0
                && is_governor(kind)
                && !*refused.get_or_insert_with(|| governor_refuses_owner(governor));
            let state = match hold {
                // The governor would admit the owner's run: not this
                // hold's question (answered if one was asked).
                Some(_) if passable => {
                    self.since.remove(kind.ask_kind);
                    self.waited.remove(kind.ask_kind);
                    State::Clear
                }
                None => {
                    self.since.remove(kind.ask_kind);
                    self.waited.remove(kind.ask_kind);
                    State::Clear
                }
                Some(_) => {
                    let since = *self.since.entry(kind.ask_kind).or_insert(at);
                    // The owner's work seen behind the hold on an earlier
                    // tick: a job queued during this one may still start on
                    // the next, whatever its age says.
                    let waited = self.waited.get(kind.ask_kind).copied();
                    if owner.queued <= 0 {
                        self.waited.remove(kind.ask_kind);
                        State::Clear
                    } else if waited.is_none() {
                        self.waited.insert(kind.ask_kind, at);
                        State::Pending
                    } else if at.duration_since(since) >= pass
                        && owner.seconds >= pass.as_secs() as i64
                        && owner.seconds >= kind.owner_wait_minutes * 60
                    {
                        State::Holding
                    } else {
                        State::Pending
                    }
                }
            };
            let was = self.asked.get(kind.ask_kind).copied();
            match state {
                State::Holding if was != Some(true) => {
                    let hold = hold.expect("a holding kind has its hold");
                    let post = ask_hold(s, brain, kind, &hold.reason, owner, now, actor);
                    lines.push(format!(
                        "{} holds the owner's work: {} owner job(s) queued, {} the longest for {}; {}{}",
                        kind.label,
                        owner.queued,
                        owner.oldest,
                        hive_cli::format_duration(Duration::from_secs(owner.seconds.max(0) as u64)),
                        hold.reason,
                        if post.0.is_empty() {
                            String::new()
                        } else {
                            format!(" (asked the owner: {})", post.str("id"))
                        }
                    ));
                    self.asked.insert(kind.ask_kind, true);
                }
                State::Clear if was != Some(false) => {
                    let why = if hold.is_none() {
                        kind.lifted.to_string()
                    } else if passable {
                        "the owner's one run may start over the line again".to_string()
                    } else {
                        "the owner's work no longer waits behind it".to_string()
                    };
                    let closed = answer(
                        s,
                        brain,
                        kind.ask_kind,
                        &format!("{} lifted: {why}", kind.label),
                        &format!(
                            "The {} no longer holds your work: {why}. Your queued work starts again.",
                            kind.label
                        ),
                        now,
                        actor,
                    );
                    if closed > 0 {
                        lines.push(format!(
                            "{} lifted: {why}; {closed} owner question(s) answered",
                            kind.label
                        ));
                    }
                    self.asked.insert(kind.ask_kind, false);
                }
                _ => {}
            }
        }
        lines
    }
}

/// The question one hold kind asks.
fn ask_hold(
    s: &hive_store::Store,
    brain: &Row,
    kind: &Kind,
    reason: &str,
    owner: &OwnerWait,
    now: &str,
    actor: &str,
) -> Row {
    let minutes = owner.seconds.max(0) / 60;
    ask(
        s,
        Ask {
            brain,
            ask_kind: kind.ask_kind,
            thread: kind.ask_kind,
            summary: format!(
                "{} holds the owner's work: {} job(s) of yours queued, the longest {minutes} min; {reason}",
                kind.label, owner.queued
            ),
            body: format!(
                "The daemon starts none of your queued work while this hold stands: {reason}. {} job(s) of yours wait, {} the longest ({minutes} min); the fleet's own work waits too.\n\n{}\n\nThis post closes itself when the hold lifts ({now}).\n",
                owner.queued, owner.oldest, kind.cure
            ),
            event: EVENT,
            detail: serde_json::json!({
                "kind": kind.ask_kind,
                "reason": reason,
                "queued": owner.queued,
                "oldest": owner.oldest,
                "seconds": owner.seconds,
            }),
            now,
            actor,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brain(s: &hive_store::Store) -> Row {
        let conn = s.conn().unwrap();
        crate::util::exec1(
            &conn,
            "UPDATE projects SET brain = 1 WHERE slug = 'proj'",
            &[],
        );
        crate::board::project_by_slug(&conn, "proj".into())
    }

    fn waiting(queued: i32, seconds: i64) -> OwnerWait {
        OwnerWait {
            queued,
            oldest: "job-owner".into(),
            seconds,
        }
    }

    fn open(s: &hive_store::Store, kind: &Kind) -> Vec<Row> {
        crate::hygiene::open_daemon_questions_of_kind(s, "proj", kind.ask_kind)
    }

    /// A machine of four slots (load line 24 on 12 CPUs, memory floor
    /// 1024 MB) under `pressure`, with `running` local runs in flight,
    /// `owner_running` of them the owner's.
    fn governor(
        pressure: crate::daemon_load::Pressure,
        running: usize,
        owner_running: usize,
    ) -> GovernorTick {
        GovernorTick {
            limits: crate::daemon_load::Limits {
                max_jobs: 4,
                max_load: 24.0,
                cpus: 12,
                min_free_mem_mb: 1024,
                own_nice: 0,
                build_jobs: 6,
                source: "test",
            },
            pressure,
            running,
            measuring: 0,
            reserved: 0,
            owner_running,
        }
    }

    fn critical() -> crate::daemon_load::Pressure {
        crate::daemon_load::Pressure {
            mem_critical: true,
            ..crate::daemon_load::CALM
        }
    }

    #[test]
    fn a_hold_asks_once_after_one_pass_and_answers_when_it_lifts() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        let b = brain(&s);
        let now = crate::test_helpers::TEST_NOW;
        let pass = Duration::from_secs(60);
        let t0 = Instant::now();
        // The machine swapping: the owner's run is refused, so the load
        // hold is the owner's business.
        let gov = governor(critical(), 1, 0);
        let load = [Hold::new(
            &LOAD,
            "machine saturated: load 60.0 on 12 CPUs is over the line 24",
        )];
        let mut holds = IdleHolds::default();
        // The first tick of the hold: inside one pass, nothing said.
        let lines = holds.tend(
            &s,
            &b,
            &load,
            &gov,
            &waiting(1, 120),
            pass,
            now,
            "daemon",
            t0,
        );
        assert!(lines.is_empty(), "{lines:?}");
        assert!(open(&s, &LOAD).is_empty());
        // Past a pass but the owner's work seen for the first time: it may
        // start on the next tick, so nothing is said yet either.
        let mut fresh = IdleHolds::default();
        let lines = fresh.tend(
            &s,
            &b,
            &load,
            &gov,
            &waiting(1, 600),
            Duration::ZERO,
            now,
            "daemon",
            t0,
        );
        assert!(lines.is_empty(), "{lines:?}");
        // One pass later, the owner's job still queued: one question.
        let t1 = t0 + pass;
        let lines = holds.tend(
            &s,
            &b,
            &load,
            &gov,
            &waiting(1, 180),
            pass,
            now,
            "daemon",
            t1,
        );
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].starts_with("machine load holds the owner's work: 1 owner job(s) queued")
                && lines[0].contains("(asked the owner: "),
            "{lines:?}"
        );
        let q = open(&s, &LOAD);
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].str("thread"), "idle-load");
        assert!(q[0].str("body").contains("lower `capacity`"), "{q:?}");
        // The next pass: still held, nothing new said or posted.
        let lines = holds.tend(
            &s,
            &b,
            &load,
            &gov,
            &waiting(2, 240),
            pass,
            now,
            "daemon",
            t1 + pass,
        );
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(open(&s, &LOAD).len(), 1);
        // The hold lifts: answered once.
        let lines = holds.tend(
            &s,
            &b,
            &[],
            &gov,
            &waiting(2, 300),
            pass,
            now,
            "daemon",
            t1 + pass,
        );
        assert_eq!(
            lines,
            vec![
                "machine load lifted: the load is under the line again; 1 owner question(s) answered"
            ]
        );
        assert!(open(&s, &LOAD).is_empty());
        let lines = holds.tend(
            &s,
            &b,
            &[],
            &gov,
            &waiting(2, 300),
            pass,
            now,
            "daemon",
            t1 + pass,
        );
        assert!(lines.is_empty(), "{lines:?}");
    }

    #[test]
    fn no_question_without_owner_work_and_kinds_keep_their_own_bounds() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        let b = brain(&s);
        let now = crate::test_helpers::TEST_NOW;
        let t0 = Instant::now();
        let gov = governor(crate::daemon_load::CALM, 0, 0);
        let mut holds = IdleHolds::default();
        let disk = [Hold::new(
            &DISK_FLOOR,
            "disk under floor: 512 MB free on /w (daemon.min_free_mb 8192)",
        )];
        // Only the fleet's work queued: the hold is the fleet's business.
        assert!(
            holds
                .tend(
                    &s,
                    &b,
                    &disk,
                    &gov,
                    &waiting(0, 0),
                    Duration::ZERO,
                    now,
                    "daemon",
                    t0
                )
                .is_empty()
        );
        // The disk floor waits for the reclaim ladder's turn first.
        assert!(
            holds
                .tend(
                    &s,
                    &b,
                    &disk,
                    &gov,
                    &waiting(1, 60),
                    Duration::ZERO,
                    now,
                    "daemon",
                    t0
                )
                .is_empty()
        );
        assert!(open(&s, &DISK_FLOOR).is_empty());
        let lines = holds.tend(
            &s,
            &b,
            &disk,
            &gov,
            &waiting(1, DISK_FLOOR.owner_wait_minutes * 60),
            Duration::ZERO,
            now,
            "daemon",
            t0,
        );
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(open(&s, &DISK_FLOOR).len(), 1);
        // The owner's work left the queue under the same hold: answered.
        let lines = holds.tend(
            &s,
            &b,
            &disk,
            &gov,
            &waiting(0, 0),
            Duration::ZERO,
            now,
            "daemon",
            t0,
        );
        assert_eq!(
            lines,
            vec![
                "the disk floor lifted: the owner's work no longer waits behind it; 1 owner question(s) answered"
            ]
        );
        // Without a brain nothing is posted, but the log still says it.
        let mut holds = IdleHolds::default();
        let bulk = [Hold::new(&BULK_AWAY, "bulk storage away")];
        let mut tick = || {
            holds.tend(
                &s,
                &Row::default(),
                &bulk,
                &gov,
                &waiting(1, 0),
                Duration::ZERO,
                now,
                "daemon",
                t0,
            )
        };
        assert!(tick().is_empty(), "the owner's work seen once");
        let lines = tick();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(!lines[0].contains("asked the owner"), "{lines:?}");
        assert!(open(&s, &BULK_AWAY).is_empty());
    }

    #[test]
    fn a_load_hold_the_owner_passes_asks_nothing_until_the_owner_is_refused() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        let b = brain(&s);
        let now = crate::test_helpers::TEST_NOW;
        let pass = Duration::from_secs(60);
        let t0 = Instant::now();
        let load = [Hold::new(
            &LOAD,
            "machine saturated: load 30.0 on 12 CPUs is over the line 24; one express slot stays open for the owner's work",
        )];
        let mem = [Hold::new(
            &MEMORY,
            "machine saturated: 300 MB of memory free, under the floor 1024",
        )];
        // No owner run going here: the owner's one run may start over the
        // line (docs/owner-reservation.md), so neither a load nor a memory
        // hold is worth a question, however long.
        let loaded = crate::daemon_load::Pressure {
            load1: Some(30.0),
            ..crate::daemon_load::CALM
        };
        let short = crate::daemon_load::Pressure {
            free_mem_mb: Some(300),
            ..crate::daemon_load::CALM
        };
        let mut holds = IdleHolds::default();
        for (i, (held, pressure)) in [(&load, loaded), (&mem, short), (&load, loaded)]
            .into_iter()
            .enumerate()
        {
            let gov = governor(pressure, 4, 0);
            assert!(!governor_refuses_owner(&gov));
            let lines = holds.tend(
                &s,
                &b,
                held,
                &gov,
                &waiting(1, 600),
                pass,
                now,
                "daemon",
                t0 + pass * (i as u32 * 2),
            );
            assert!(lines.is_empty(), "{lines:?}");
        }
        assert!(open(&s, &LOAD).is_empty() && open(&s, &MEMORY).is_empty());
        // The load past twice the line, and the owner's one run over it
        // already going on this machine: the governor refuses the next
        // owner job, and the hold asks after a pass.
        let swamped = crate::daemon_load::Pressure {
            load1: Some(60.0),
            ..crate::daemon_load::CALM
        };
        let gov = governor(swamped, 4, 1);
        assert!(governor_refuses_owner(&gov));
        let t1 = t0 + pass * 10;
        assert!(
            holds
                .tend(
                    &s,
                    &b,
                    &load,
                    &gov,
                    &waiting(1, 600),
                    pass,
                    now,
                    "daemon",
                    t1
                )
                .is_empty()
        );
        let lines = holds.tend(
            &s,
            &b,
            &load,
            &gov,
            &waiting(1, 660),
            pass,
            now,
            "daemon",
            t1 + pass,
        );
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].starts_with("machine load holds the owner's work"),
            "{lines:?}"
        );
        assert_eq!(open(&s, &LOAD).len(), 1);
        // The owner's run ends: the next one may start over the line, so
        // the open question is answered, though the load still stands.
        let lines = holds.tend(
            &s,
            &b,
            &load,
            &governor(swamped, 3, 0),
            &waiting(1, 720),
            pass,
            now,
            "daemon",
            t1 + pass * 2,
        );
        assert_eq!(
            lines,
            vec![
                "machine load lifted: the owner's one run may start over the line again; 1 owner question(s) answered"
            ]
        );
        assert!(open(&s, &LOAD).is_empty());
    }

    #[test]
    fn a_load_hold_under_the_memory_warning_with_the_halved_cap_half_taken_asks_nothing() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        let b = brain(&s);
        let now = crate::test_helpers::TEST_NOW;
        let pass = Duration::from_secs(60);
        let t0 = Instant::now();
        let load = [Hold::new(
            &LOAD,
            "machine saturated: load 30.0 on 12 CPUs is over the line 24; one express slot stays open for the owner's work",
        )];
        // The kernel's warning halves the four slots to two; the load is
        // over the line but under twice it, a soft hold. One local run is
        // going - the owner's own - so the halved cap is half taken and the
        // dispatch's express slot takes the owner's next run beside it.
        let warned = crate::daemon_load::Pressure {
            load1: Some(30.0),
            mem_warning: true,
            ..crate::daemon_load::CALM
        };
        let gov = governor(warned, 1, 1);
        let dispatch = crate::daemon_load::decide(&gov.limits, &warned, 1, 0, 0, 1);
        assert!(dispatch.hold.is_some() && dispatch.express, "{dispatch:?}");
        assert!(!governor_refuses_owner(&gov));
        let mut holds = IdleHolds::default();
        for i in 0..4u32 {
            let lines = holds.tend(
                &s,
                &b,
                &load,
                &gov,
                &waiting(1, 600 + i64::from(i) * 60),
                pass,
                now,
                "daemon",
                t0 + pass * i,
            );
            assert!(lines.is_empty(), "{lines:?}");
        }
        assert!(open(&s, &LOAD).is_empty());
        // The halved cap full beside the owner's run in flight: the
        // dispatch keeps the owner out, and the hold asks after a pass.
        let full = governor(warned, 3, 1);
        assert!(!crate::daemon_load::decide(&full.limits, &warned, 3, 0, 0, 1).express);
        assert!(governor_refuses_owner(&full));
        let t1 = t0 + pass * 10;
        let mut asked = Vec::new();
        for i in 0..2u32 {
            asked.extend(holds.tend(
                &s,
                &b,
                &load,
                &full,
                &waiting(1, 1200 + i64::from(i) * 60),
                pass,
                now,
                "daemon",
                t1 + pass * i,
            ));
        }
        assert_eq!(asked.len(), 1, "{asked:?}");
        assert_eq!(open(&s, &LOAD).len(), 1);
    }

    #[test]
    fn a_restarted_daemon_keeps_a_pending_hold_open_and_answers_a_lifted_one() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        let b = brain(&s);
        let now = crate::test_helpers::TEST_NOW;
        let t0 = Instant::now();
        let pass = Duration::from_secs(60);
        let gov = governor(
            crate::daemon_load::Pressure {
                free_mem_mb: Some(300),
                ..critical()
            },
            1,
            0,
        );
        let mem = [Hold::new(
            &MEMORY,
            "machine saturated: 300 MB of memory free, under the floor 1024",
        )];
        let mut first = IdleHolds::default();
        for _ in 0..2 {
            first.tend(
                &s,
                &b,
                &mem,
                &gov,
                &waiting(1, 600),
                Duration::ZERO,
                now,
                "daemon",
                t0,
            );
        }
        assert_eq!(open(&s, &MEMORY).len(), 1);
        // A new daemon, the hold still standing but inside its first pass:
        // the question stays open, not answered and asked again.
        let mut second = IdleHolds::default();
        assert!(
            second
                .tend(
                    &s,
                    &b,
                    &mem,
                    &gov,
                    &waiting(1, 600),
                    pass,
                    now,
                    "daemon",
                    t0
                )
                .is_empty()
        );
        assert_eq!(open(&s, &MEMORY).len(), 1);
        // A new daemon that finds the hold gone answers it.
        let mut third = IdleHolds::default();
        let lines = third.tend(&s, &b, &[], &gov, &waiting(1, 600), pass, now, "daemon", t0);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(open(&s, &MEMORY).is_empty());
        assert_eq!(
            crate::test_helpers::count(
                &s,
                &format!("SELECT COUNT(*) AS n FROM events WHERE action = '{EVENT}'"),
                &[]
            ),
            1
        );
    }

    /// A daemon fixture for one hold: the brain is `proj`, one owner task
    /// and one fleet improve job queued (`back` minutes ago), a planning
    /// pass every tick so one tick is one pass.
    struct Daemon {
        s: crate::test_helpers::TestStore,
        out: crate::test_helpers::OutBuf,
        task: String,
    }

    impl Daemon {
        fn new(back: i64) -> Self {
            hive_bus::reset_default();
            let s = crate::test_helpers::new_store();
            crate::test_helpers::new_bare_repo(&s);
            for (k, v) in [
                ("daemon.min_free_mb", "0"),
                ("daemon.sweep", "off"),
                ("daemon.pass_seconds", "0"),
                ("jobs_per_day", "0"),
            ] {
                s.set_setting(hive_store::HUMAN_AGENT, k, v).unwrap();
            }
            crate::test_helpers::join(&s, "aurora");
            brain(&s);
            // The jobs are queued `back` minutes before the daemon's now:
            // through a handle of the store whose clock reads then.
            let then = crate::util::plus_minutes(crate::test_helpers::TEST_NOW, -(back as f64));
            let early = hive_store::Store::open(hive_store::Options {
                home: s.home.clone(),
                now: then,
                ..Default::default()
            })
            .unwrap();
            let add = |kind: &str, source: &str| {
                crate::jobs::add_job(
                    &early,
                    "proj".into(),
                    kind.into(),
                    source.into(),
                    "echo ok and report".into(),
                    1,
                    10,
                    source.into(),
                    hive_store::HUMAN_AGENT.into(),
                    false,
                )
                .unwrap()
                .0
                .str("id")
            };
            add("improve", "suggestion:abc");
            let task = add("task", "task:TASK-077");
            Self {
                s,
                out: crate::test_helpers::OutBuf::new(),
                task,
            }
        }

        /// Two daemon passes, as a fresh daemon (the daily cap's test runs
        /// one): the first sees the owner's work behind the hold, the
        /// second asks. What it said.
        fn tick(&self) -> String {
            self.out.reset();
            let result = crate::daemon::daemon(
                &hive_bus::CancellationToken::new(),
                &self.s,
                crate::daemon::DaemonOpts {
                    interval: 1,
                    max_loops: 2,
                    sleep: Some(std::sync::Arc::new(|_| true)),
                    worker_cmd: "sh -c 'echo SUMMARY: all good'".into(),
                    out: Some(self.out.writer()),
                    ..Default::default()
                },
            );
            assert_eq!(result, "done", "daemon = {result}\n{}", self.out.string());
            self.out.string()
        }

        fn status(&self, id: &str) -> String {
            crate::jobs::job_row(&*self.s.conn().unwrap(), id.into()).str("status")
        }

        /// Every question of the kind ever posted, open or answered.
        fn asked(&self, kind: &Kind) -> i32 {
            crate::test_helpers::count(
                &self.s,
                "SELECT COUNT(*) AS n FROM posts WHERE kind = 'question' AND thread = ?",
                &[&kind.ask_kind],
            )
        }

        /// The held ticks: one question on the first, none on the second,
        /// the owner's task still queued; `cure` is in the question.
        fn held_twice(&self, kind: &Kind, cure: &str) {
            let out = self.tick();
            assert!(
                out.contains(&format!(
                    "{} holds the owner's work: 1 owner job(s) queued",
                    kind.label
                )) && out.contains("(asked the owner: "),
                "the log names the hold and the question:\n{out}"
            );
            assert_eq!(self.status(&self.task), "queued", "{out}");
            let q = open(&self.s, kind);
            assert_eq!(q.len(), 1, "{q:?}\n{out}");
            assert_eq!(q[0].str("area"), AREA);
            assert!(
                q[0].str("body").contains(cure),
                "the question carries its cure: {:?}",
                q[0]
            );
            let out = self.tick();
            assert_eq!(self.status(&self.task), "queued", "{out}");
            assert_eq!(
                (open(&self.s, kind).len(), self.asked(kind)),
                (1, 1),
                "the second pass asks nothing new\n{out}"
            );
            assert_eq!(
                crate::test_helpers::count(
                    &self.s,
                    &format!("SELECT COUNT(*) AS n FROM events WHERE action = '{EVENT}'"),
                    &[]
                ),
                1,
                "{out}"
            );
        }

        /// The tick after the hold lifted: answered, the owner's task ran.
        fn lifted(&self, kind: &Kind) {
            let out = self.tick();
            assert!(
                out.contains(&format!(
                    "{} lifted: {}; 1 owner question(s) answered",
                    kind.label, kind.lifted
                )),
                "{out}"
            );
            assert!(open(&self.s, kind).is_empty(), "{out}");
            assert_eq!(self.asked(kind), 1, "{out}");
            assert_eq!(self.status(&self.task), "done", "{out}");
            // Other kinds were never asked about.
            for other in KINDS.iter().filter(|k| k.ask_kind != kind.ask_kind) {
                assert_eq!(self.asked(other), 0, "{}: {out}", other.ask_kind);
            }
        }
    }

    #[test]
    fn a_load_hold_asks_the_owner_once_and_answers_when_it_lifts() {
        let _serial = crate::test_helpers::serial();
        let d = Daemon::new(0);
        // A load hold alone lets the owner's one run start over the line
        // (docs/owner-reservation.md); with the machine swapping as well
        // not even that slot opens, and the hold names the load.
        let pin = crate::daemon_load::pin_pressure(crate::daemon_load::Pressure {
            load1: Some(1000.0),
            mem_critical: true,
            ..crate::daemon_load::CALM
        });
        d.held_twice(&LOAD, "lower `capacity`");
        drop(pin);
        d.lifted(&LOAD);
    }

    #[test]
    fn a_memory_hold_asks_the_owner_once_and_answers_when_it_lifts() {
        let _serial = crate::test_helpers::serial();
        let d = Daemon::new(0);
        // Memory under the floor alone lets the owner's one run start
        // (docs/owner-reservation.md); the kernel's critical pressure, the
        // machine swapping, keeps even that slot shut.
        let pin = crate::daemon_load::pin_pressure(crate::daemon_load::Pressure {
            free_mem_mb: Some(100),
            mem_critical: true,
            ..crate::daemon_load::CALM
        });
        d.held_twice(&MEMORY, "Close what holds the memory");
        drop(pin);
        d.lifted(&MEMORY);
    }

    #[test]
    fn a_disk_floor_hold_asks_the_owner_once_and_answers_when_it_lifts() {
        let _serial = crate::test_helpers::serial();
        // The reclaim ladder has had its turn: the task waited 20 minutes.
        let d = Daemon::new(20);
        let _free = crate::executor::pin_free_mb(512);
        d.s.set_setting(
            hive_store::HUMAN_AGENT,
            "daemon.min_free_mb",
            &i32::MAX.to_string(),
        )
        .unwrap();
        d.held_twice(&DISK_FLOOR, "daemon.min_free_mb");
        d.s.set_setting(hive_store::HUMAN_AGENT, "daemon.min_free_mb", "0")
            .unwrap();
        d.lifted(&DISK_FLOOR);
    }

    #[test]
    fn a_disk_floor_crossing_the_ladder_may_still_clear_asks_nothing() {
        let _serial = crate::test_helpers::serial();
        let d = Daemon::new(1);
        let _free = crate::executor::pin_free_mb(512);
        d.s.set_setting(
            hive_store::HUMAN_AGENT,
            "daemon.min_free_mb",
            &i32::MAX.to_string(),
        )
        .unwrap();
        let out = d.tick();
        assert!(out.contains("idle: disk under floor"), "{out}");
        assert_eq!(d.asked(&DISK_FLOOR), 0, "{out}");
    }

    #[cfg(unix)]
    #[test]
    fn a_bulk_disk_away_asks_the_owner_once_and_answers_when_it_is_back() {
        let _serial = crate::test_helpers::serial();
        let d = Daemon::new(0);
        let home = std::path::Path::new(&d.s.home).to_path_buf();
        let work = home.join("work");
        std::fs::write(home.join(crate::bulk::ID_FILE), "abc123\n").unwrap();
        std::os::unix::fs::symlink(
            std::path::Path::new("/Volumes/NotPluggedIn/hive"),
            home.join(crate::bulk::LINK),
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&work);
        std::os::unix::fs::symlink(std::path::Path::new(crate::bulk::LINK).join("work"), &work)
            .unwrap();
        d.held_twice(&BULK_AWAY, "hive bulk detach --lost");
        // The owner gives up on the disk: the bulk is local again.
        std::fs::remove_file(&work).unwrap();
        std::fs::remove_file(home.join(crate::bulk::LINK)).unwrap();
        std::fs::remove_file(home.join(crate::bulk::ID_FILE)).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        d.lifted(&BULK_AWAY);
    }

    /// A daemon left running with one fleet run in flight that goes on
    /// until [`Live::release`]: the drain and fleet-full holds stand only
    /// while a run is going.
    struct Live {
        s: crate::test_helpers::TestStore,
        out: crate::test_helpers::OutBuf,
        go: std::path::PathBuf,
        cancel: hive_bus::CancellationToken,
        done: crossbeam_channel::Receiver<String>,
    }

    impl Live {
        fn start(restart_on_change: bool, per_project: i32) -> Self {
            hive_bus::reset_default();
            let s = crate::test_helpers::new_store();
            crate::test_helpers::new_bare_repo(&s);
            for (k, v) in [
                ("daemon.min_free_mb", "0"),
                ("daemon.sweep", "off"),
                ("daemon.pass_seconds", "0"),
                ("jobs_per_day", "0"),
                ("capacity", "1"),
            ] {
                s.set_setting(hive_store::HUMAN_AGENT, k, v).unwrap();
            }
            if per_project > 0 {
                crate::capacity::pin_per_project(&s, per_project);
            }
            crate::test_helpers::join(&s, "aurora");
            brain(&s);
            add(&s, 0, "improve", "suggestion:abc");
            let go = std::path::Path::new(&s.home).join("go");
            let out = crate::test_helpers::OutBuf::new();
            let opts = crate::daemon::DaemonOpts {
                interval: 1,
                out: Some(out.writer()),
                restart_on_change,
                worker_cmd: format!(
                    "sh -c 'while [ ! -f {} ]; do sleep 0.1; done; echo SUMMARY: ok'",
                    go.display()
                ),
                ..Default::default()
            };
            let cancel = hive_bus::CancellationToken::new();
            let (done_tx, done) = crossbeam_channel::bounded(1);
            {
                let (s, cancel) = (s.clone(), cancel.clone());
                std::thread::spawn(move || {
                    let _ = done_tx.send(crate::daemon::daemon(&cancel, &s, opts));
                });
            }
            let live = Self {
                s,
                out,
                go,
                cancel,
                done,
            };
            live.wait_for("the fleet's run never started", &|| {
                live.out.string().contains("leased job-")
            });
            live
        }

        fn wait_for(&self, what: &str, f: &dyn Fn() -> bool) {
            let began = Instant::now();
            while !f() {
                if began.elapsed() > crate::test_helpers_daemon::test_wait(Duration::from_secs(60))
                {
                    self.stop();
                    panic!("{what}\n{}", self.out.string());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }

        fn loops(&self) -> i64 {
            self.s
                .meta("daemon")
                .ok()
                .and_then(|(v, _)| serde_json::from_str::<serde_json::Value>(&v).ok())
                .and_then(|v| v["loops"].as_i64())
                .unwrap_or(0)
        }

        /// Wakes the daemon with a setting and waits for the pass.
        fn another_pass(&self) {
            let before = self.loops();
            s_wake(&self.s);
            self.wait_for("no second pass", &|| self.loops() > before);
        }

        fn release(&self) {
            std::fs::write(&self.go, "").unwrap();
        }

        fn stop(&self) {
            self.release();
            self.cancel.cancel();
        }

        fn result(&self) -> String {
            self.done
                .recv_timeout(crate::test_helpers_daemon::test_wait(Duration::from_secs(
                    90,
                )))
                .unwrap_or_else(|_| {
                    self.stop();
                    panic!("the daemon did not exit\n{}", self.out.string())
                })
        }

        fn status(&self, id: &str) -> String {
            crate::jobs::job_row(&*self.s.conn().unwrap(), id.into()).str("status")
        }

        fn asked(&self, kind: &Kind) -> i32 {
            crate::test_helpers::count(
                &self.s,
                "SELECT COUNT(*) AS n FROM posts WHERE kind = 'question' AND thread = ?",
                &[&kind.ask_kind],
            )
        }
    }

    impl Drop for Live {
        fn drop(&mut self) {
            self.stop();
        }
    }

    fn s_wake(s: &hive_store::Store) {
        let n = s
            .setting("board.stale_days", "7")
            .parse::<i32>()
            .unwrap_or(7);
        s.set_setting(
            hive_store::HUMAN_AGENT,
            "board.stale_days",
            &(n + 1).to_string(),
        )
        .unwrap();
    }

    /// Queues a job `back` minutes before the test's now.
    fn add(s: &hive_store::Store, back: i64, kind: &str, source: &str) -> String {
        let then = crate::util::plus_minutes(crate::test_helpers::TEST_NOW, -(back as f64));
        let early = hive_store::Store::open(hive_store::Options {
            home: s.home.clone(),
            now: then,
            ..Default::default()
        })
        .unwrap();
        crate::jobs::add_job(
            &early,
            "proj".into(),
            kind.into(),
            source.into(),
            "echo ok and report".into(),
            1,
            10,
            source.into(),
            hive_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap()
        .0
        .str("id")
    }

    #[test]
    fn the_fleet_filling_every_slot_leaves_the_express_slot_to_the_owners_task() {
        let _serial = crate::test_helpers::serial();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        // The fleet's run holds the machine's one slot and the project's
        // one-slot share. Until TASK-136 the owner's task, queued 31
        // minutes ago, could not pass the share into the express slot, so
        // the slot stood empty and the owner was asked to raise `capacity`.
        // The owner's work now passes its project's share: the express slot
        // takes it on the tick, and there is nothing to ask the owner.
        let d = Live::start(false, 1);
        let task = add(&d.s, 31, "task", "task:TASK-079");
        d.wait_for("the owner's task never took the express slot", &|| {
            d.status(&task) != "queued"
        });
        assert!(
            d.out
                .string()
                .contains("one express slot stays open for the owner's work"),
            "{}",
            d.out.string()
        );
        d.another_pass();
        assert_eq!(d.asked(&FLEET_FULL), 0, "{}", d.out.string());
        assert!(open(&d.s, &FLEET_FULL).is_empty(), "{}", d.out.string());
        d.release();
        d.wait_for("the owner's task never ran", &|| d.status(&task) == "done");
        assert_eq!(d.asked(&FLEET_FULL), 0, "{}", d.out.string());
        d.cancel.cancel();
        let _ = d.result();
    }

    #[test]
    fn a_restart_drain_asks_the_owner_once_and_the_next_daemon_answers_it() {
        let _serial = crate::test_helpers::serial();
        let _login = crate::login_gate::swap_login_gate(Box::new(|_| String::new()));
        let d = Live::start(true, 0);
        crate::daemon::ask_restart(&d.s, "owner", serde_json::json!({"from": "settings"})).unwrap();
        d.wait_for("the drain never began", &|| {
            d.out.string().contains("draining")
        });
        let task = add(&d.s, 0, "task", "task:TASK-078");
        d.wait_for("the drain never asked the owner", &|| {
            !open(&d.s, &RESTART_DRAIN).is_empty()
                && d.out
                    .string()
                    .contains("the restart drain holds the owner's work: 1 owner job(s) queued")
        });
        let q = open(&d.s, &RESTART_DRAIN);
        assert!(
            q[0].str("body").contains("ask for the restart now"),
            "{q:?}"
        );
        d.another_pass();
        assert_eq!(d.status(&task), "queued", "{}", d.out.string());
        assert_eq!(d.asked(&RESTART_DRAIN), 1, "{}", d.out.string());
        // The fleet's run ends; the daemon goes for the restart.
        d.release();
        assert_eq!(d.result(), "restart", "{}", d.out.string());
        assert_eq!(d.asked(&RESTART_DRAIN), 1, "{}", d.out.string());
        // The next daemon: no drain, the owner's task runs, the question
        // is answered.
        d.out.reset();
        let result = crate::daemon::daemon(
            &hive_bus::CancellationToken::new(),
            &d.s,
            crate::daemon::DaemonOpts {
                interval: 1,
                max_loops: 1,
                worker_cmd: "sh -c 'echo SUMMARY: all good'".into(),
                out: Some(d.out.writer()),
                ..Default::default()
            },
        );
        assert_eq!(result, "done", "{}", d.out.string());
        assert!(
            d.out.string().contains(&format!(
                "the restart drain lifted: {}; 1 owner question(s) answered",
                RESTART_DRAIN.lifted
            )),
            "{}",
            d.out.string()
        );
        assert!(open(&d.s, &RESTART_DRAIN).is_empty());
        assert_eq!(d.status(&task), "done", "{}", d.out.string());
    }

    #[test]
    fn the_governor_line_names_its_kind() {
        assert_eq!(
            governor_kind("machine saturated: load 40.2 on 12 CPUs is over the line 24"),
            &LOAD
        );
        assert_eq!(
            governor_kind(
                "machine saturated: load 30.0 on 12 CPUs is over the line 24; one express slot stays open for the owner's work"
            ),
            &LOAD
        );
        assert_eq!(
            governor_kind("machine saturated: 300 MB of memory free, under the floor 1024"),
            &MEMORY
        );
        assert_eq!(
            governor_kind("machine saturated: the kernel reports critical memory pressure"),
            &MEMORY
        );
        let mut seen = std::collections::BTreeSet::new();
        for kind in KINDS.iter().copied() {
            assert!(seen.insert(kind.ask_kind), "{} twice", kind.ask_kind);
            assert_ne!(kind.ask_kind, crate::daily_cap::ASK_KIND);
        }
    }
}
