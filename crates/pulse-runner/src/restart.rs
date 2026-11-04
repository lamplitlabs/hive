//! The restart handover. A newer binary on disk (the self-update's build,
//! or one installed by hand) or the owner's ask for a restart arms a restart
//! while runs are in flight: the daemon keeps dispatching the work that fits
//! what is left of the window, drains for `daemon.restart_drain_minutes`,
//! stays for the owner's queued work one more drain at most and exits with
//! code 3 once the runs it waits for have ended, so the service restarts it
//! on the new build. Runs still in flight when the drain ends are asked to
//! hold (`daemon::hold_for_restart`, `daemon.restart_hold`) as a restart
//! asked for now holds them, so the handover waits only for runs past their
//! harness; the console's drain without "now" waits for every run, as the
//! owner chose. [`Restart`] holds the handover's state for the daemon
//! loop; the measures its decisions rest on - how long a run of each kind
//! is expected to take, which kinds fit the window, which runs the window
//! follows and what the notices say - are the functions beside it, tested
//! on their own.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use crate::daemon::{Dispatch, RestartAsk};

type Row = crate::util::Row;

/// What one tick's look at the handover (`Restart::tend`) decided.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Tend {
    /// The daemon goes on, saying the line when there is one: the notice
    /// that arms the restart, or the stay for the owner's work, said once.
    Goes(Option<String>),
    /// The daemon exits with code 3 so the service restarts it, saying why.
    Exits(String),
}

/// The handover's state while a restart is pending; the default is none.
#[derive(Debug, Default)]
pub(crate) struct Restart {
    /// Set when the binary changed on disk or the owner asked for a restart
    /// while runs were in flight: the daemon keeps dispatching and landing
    /// for up to daemon.restart_drain_minutes and exits with code 3 as soon
    /// as nothing is in flight (restart handover).
    pending: Option<Instant>,
    /// The runs in flight when the restart was armed: the window follows them.
    anchors: BTreeSet<String>,
    /// When the restart was armed, ISO; the heartbeat carries it.
    since: String,
    /// Set when the restart's drain is over, no fleet kind fits what is left
    /// of the window and the owner's work is queued: when the stay for it
    /// began (`restart_stay`). Cleared when the owner's run starts and when
    /// kinds fit again, so the bound measures the owner's work waiting with
    /// nothing starting.
    stay_since: Option<Instant>,
    /// Set when the owner asked for the restart now: the runs in flight were
    /// asked to hold and the wait after the loop is capped (daemon.hold_seconds).
    forced: bool,
    /// Set when the owner asked for a restart without "now": the daemon drains
    /// like a self-update - it starts nothing new and exits with code 3 once
    /// the runs in flight have ended or daemon.restart_drain_minutes is up, so
    /// nothing is held or lost.
    console_drain: bool,
}

impl Restart {
    /// Whether the owner asked for the restart now: the wait after the loop
    /// is capped by daemon.hold_seconds, not the grace.
    pub(crate) fn forced(&self) -> bool {
        self.forced
    }

    /// Whether the owner's ask without "now" has the daemon draining.
    pub(crate) fn console_draining(&self) -> bool {
        self.console_drain && self.pending.is_some()
    }

    /// The owner asked for the restart now (`RestartAsk::force`): the runs in
    /// flight were asked to hold and the daemon leaves at once. The wait
    /// after the loop beats with the runs it waits for, so a restart not
    /// armed before now records them here.
    pub(crate) fn force(&mut self, s: &pulse_store::Store, inflight: &BTreeMap<String, Dispatch>) {
        if self.pending.is_none() {
            self.anchors = inflight.keys().cloned().collect();
            self.since = s.now();
        }
        self.forced = true;
    }

    /// The owner asked for a restart without "now": the daemon drains like a
    /// self-update. The first tick to see the ask arms the drain and says so
    /// (the line returned); the ticks after it, which read the same ask,
    /// add nothing.
    pub(crate) fn drain_as_asked(
        &mut self,
        s: &pulse_store::Store,
        ask: &RestartAsk,
        inflight: &BTreeMap<String, Dispatch>,
    ) -> Option<String> {
        let line = if self.pending.is_none() {
            self.pending = Some(Instant::now());
            self.anchors = inflight.keys().cloned().collect();
            self.since = s.now();
            Some(format!(
                "restart asked by {} at {}: {} run(s) in flight; draining - nothing new starts and the daemon exits with code 3 once they end or after {}",
                ask.actor,
                ask.at,
                inflight.len(),
                pulse_cli::format_duration(restart_drain(s))
            ))
        } else {
            None
        };
        self.console_drain = true;
        line
    }

    /// One tick's look at the handover. `changed` is whether the binary on
    /// disk differs from the one this daemon runs: it arms the restart while
    /// runs are in flight (or exits at once with none, or with a zero
    /// drain), and once the drain is over it leaves when no fleet kind fits
    /// what is left of the window - unless the owner's work is queued, which
    /// it stays for, one more drain at most (`restart_stay`). With the
    /// binary unchanged, a console drain exits when nothing is in flight or
    /// its drain is over; a restart armed by a binary that changed back is
    /// off. `idle_log` carries the stay line, said once while it holds.
    pub(crate) fn tend(
        &mut self,
        s: &pulse_store::Store,
        changed: bool,
        inflight: &BTreeMap<String, Dispatch>,
        idle_log: &mut crate::idle_log::IdleLog,
    ) -> Tend {
        if changed {
            let drain = restart_drain(s);
            match self.pending {
                None if inflight.is_empty() || drain.is_zero() => {
                    return Tend::Exits(
                        "pulse changed on disk; exiting with code 3 so the service restarts it"
                            .to_string(),
                    );
                }
                None => {
                    self.anchors = inflight.keys().cloned().collect();
                    let fits = kind_fits(s);
                    let window = restart_window(
                        drain,
                        &s.now(),
                        anchor_ends(s, inflight, &self.anchors, &fits),
                    );
                    let kinds = kinds_within(&fits, window);
                    let starts = restart_starts_phrase(&kinds, fits.len(), &fits, window, drain);
                    let line = format!(
                        "pulse changed on disk; {} run(s) in flight finish first and {starts}, then this daemon exits with code 3 so the service restarts it",
                        inflight.len(),
                    );
                    self.pending = Some(Instant::now());
                    self.since = s.now();
                    return Tend::Goes(Some(line));
                }
                Some(_) if inflight.is_empty() => {
                    return Tend::Exits(
                        "pulse changed on disk and nothing is in flight; exiting with code 3 so the service restarts it"
                            .to_string(),
                    );
                }
                Some(since) if since.elapsed() >= drain => {
                    let fits = kind_fits(s);
                    let left = restart_window(
                        Duration::ZERO,
                        &s.now(),
                        anchor_ends(s, inflight, &self.anchors, &fits),
                    );
                    if kinds_within(&fits, left).is_empty() {
                        match restart_stay(
                            self.console_drain,
                            owner_work_queued(s),
                            &mut self.stay_since,
                            Instant::now(),
                            drain,
                        ) {
                            RestartStay::StayedOut => {
                                return Tend::Exits(format!(
                                    "restart drain of {} over and the owner's work queued for as long again has not started (the slots cannot lease it here); exiting with code 3 after the {} run(s) in flight finish, the next daemon starts it first",
                                    pulse_cli::format_duration(drain),
                                    inflight.len()
                                ));
                            }
                            RestartStay::Leaves => {
                                return Tend::Exits(format!(
                                    "restart drain of {} over and no new run fits before the slowest of {} run(s) in flight is expected to end; exiting with code 3 after they finish",
                                    pulse_cli::format_duration(drain),
                                    inflight.len()
                                ));
                            }
                            RestartStay::Stays => {}
                        }
                        // The owner's work starts in any window: the tick
                        // leases it on the owner lane and its run anchors
                        // the window, so this daemon stays for it rather
                        // than leave it queued for the next.
                        return Tend::Goes(idle_log.line(
                            "restart-drain",
                            &format!(
                                "restart drain of {} over and no fleet run fits before the slowest of {} run(s) in flight is expected to end, but the owner's work is queued: it starts first, then this daemon exits with code 3",
                                pulse_cli::format_duration(drain),
                                inflight.len()
                            ),
                            Instant::now(),
                        ));
                    }
                    idle_log.clear("restart-drain");
                    self.stay_since = None;
                }
                Some(_) => {}
            }
        } else if let Some(since) = self.pending
            && self.console_drain
        {
            // The console asked for a restart (no binary change): drain.
            if inflight.is_empty() {
                return Tend::Exits(
                    "restart asked and nothing is in flight; exiting with code 3 so the service restarts it"
                        .to_string(),
                );
            }
            let drain = restart_drain(s);
            if since.elapsed() >= drain {
                return Tend::Exits(format!(
                    "restart drain of {} over with {} run(s) in flight; exiting with code 3 after they finish",
                    pulse_cli::format_duration(drain),
                    inflight.len()
                ));
            }
        } else if self.pending.is_some() {
            // The binary was changed back: the handover is off.
            self.pending = None;
            self.anchors.clear();
            self.since.clear();
            self.stay_since = None;
            idle_log.clear("restart-drain");
        }
        Tend::Goes(None)
    }

    /// The heartbeat's note and `restart` record while a restart is pending
    /// (`restart_note`, `restart_beat`); empty and absent otherwise.
    pub(crate) fn heartbeat(
        &self,
        now: &str,
        inflight: &BTreeMap<String, Dispatch>,
    ) -> (String, Option<serde_json::Value>) {
        if self.pending.is_none() {
            return (String::new(), None);
        }
        (
            restart_note(now, inflight, &self.anchors),
            Some(self.beat(inflight, true)),
        )
    }

    /// The heartbeat's record of the handover (`restart_beat`): the wait
    /// after the loop beats with `dispatching` false.
    pub(crate) fn beat(
        &self,
        inflight: &BTreeMap<String, Dispatch>,
        dispatching: bool,
    ) -> serde_json::Value {
        restart_beat(&self.since, inflight, &self.anchors, dispatching)
    }

    /// The restart window: all kinds of run still fit it (no hold),
    /// only the short ones and the owner's work do (the line names
    /// them and the slots lease nothing else), or the owner's work
    /// alone (`restart_fits`). A console drain closes the slots. The
    /// hold line, if any, and the project and kind pairs the slots may
    /// lease - every kind when there is no hold, so the pairs are empty.
    pub(crate) fn hold(
        &self,
        s: &pulse_store::Store,
        now: &str,
        inflight: &BTreeMap<String, Dispatch>,
    ) -> (Option<String>, Vec<(String, String)>) {
        let Some(since) = self.pending else {
            return (None, Vec::new());
        };
        // A console drain starts nothing new: the restart is wanted,
        // not just a new build, so the runs in flight are waited out.
        if self.console_drain {
            let (fits, who) = restart_fits(Vec::new(), true);
            let drain_left = restart_drain(s).saturating_sub(since.elapsed());
            return (
                Some(format!(
                    "restarting as asked: {who}, draining {} in flight with {} left",
                    inflight.len(),
                    pulse_cli::format_duration(drain_left)
                )),
                fits,
            );
        }
        let drain_left = restart_drain(s).saturating_sub(since.elapsed());
        let fits = kind_fits(s);
        let left = restart_window(
            drain_left,
            now,
            anchor_ends(s, inflight, &self.anchors, &fits),
        );
        // Whole seconds, so the idle log sees one line as the countdown moves.
        let left = Duration::from_secs(left.as_secs());
        let kinds = kinds_within(&fits, left);
        if kinds.len() == fits.len() {
            return (None, Vec::new());
        }
        let (fits, who) = restart_fits(kinds, false);
        (
            Some(format!(
                "restarting: {who} with {} left of the restart window; {} in flight",
                pulse_cli::format_duration(left),
                inflight.len()
            )),
            fits,
        )
    }

    /// A run dispatched now, `key` among the runs in flight: the owner's
    /// anchors a pending restart (`anchors_restart`) - the window follows it
    /// to its expected end, the kinds that fit keep starting while it runs,
    /// and the stay for the owner's queued work starts over.
    pub(crate) fn anchor(&mut self, key: &str, job: &Row) {
        if anchors_restart(self.pending.is_some(), job) {
            self.anchors.insert(key.to_string());
            self.stay_since = None;
        }
    }
}

/// How long a daemon whose binary changed on disk keeps dispatching while
/// runs are in flight (daemon.restart_drain_minutes); zero exits at once.
fn restart_drain(s: &pulse_store::Store) -> Duration {
    Duration::from_secs((s.setting_int("daemon.restart_drain_minutes").max(0) as u64) * 60)
}

/// How long new runs may still start while a restart is pending: the drain
/// that is left, or the time until the last of the runs this daemon had in
/// flight when the restart was armed is expected to end (`anchors` being
/// their start times and expected lengths, see `anchor_ends`), whichever is
/// longer. The restart waits for those runs in any case, so a new run that
/// ends before the slowest of them is expected to delays nothing, while a
/// daemon that stops dispatching the moment the drain is over idles every
/// project for as long as that one run takes. The expectation, not the
/// budget, sets the window: runs end near their kind's measure, and a run
/// admitted against a two-hour budget while the runs in flight ended within
/// the half hour held nine idle slots for forty minutes at each release.
/// The fleet's runs started inside the window are not anchors: each would
/// open a fresh window and the restart would never come while the queue had
/// work. The owner's are (`anchors_restart`): the restart waits for them
/// anyway, so the window follows them and the short kinds keep starting.
fn restart_window(
    drain_left: Duration,
    now: &str,
    anchors: impl IntoIterator<Item = (String, Duration)>,
) -> Duration {
    let now = crate::util::must_iso(now);
    let slowest = anchors
        .into_iter()
        .filter(|(started, _)| !started.is_empty())
        .map(|(started, expected)| {
            (crate::util::must_iso(&started) + chrono::Duration::seconds(expected.as_secs() as i64)
                - now)
                .num_seconds()
                .max(0) as u64
        })
        .max()
        .unwrap_or(0);
    drain_left.max(Duration::from_secs(slowest))
}

/// The runs a pending restart waits for: those in flight when it was armed
/// (the anchors), or every run in flight once the anchors are done - the
/// handover waits for all of them.
fn restart_waits<'a>(
    inflight: &'a BTreeMap<String, Dispatch>,
    anchors: &BTreeSet<String>,
) -> Vec<&'a Dispatch> {
    let anchored: Vec<&Dispatch> = inflight
        .iter()
        .filter(|(key, _)| anchors.contains(*key))
        .map(|(_, d)| d)
        .collect();
    if anchored.is_empty() {
        inflight.values().collect()
    } else {
        anchored
    }
}

/// The heartbeat's record of a pending restart: since when, how many runs
/// are in flight, whether new ones still start, and the runs the new build
/// waits for. A brain run can take an hour and a bare "restarting" read as
/// stuck for all of it; the console names the run and its age instead.
fn restart_beat(
    since: &str,
    inflight: &BTreeMap<String, Dispatch>,
    anchors: &BTreeSet<String>,
    dispatching: bool,
) -> serde_json::Value {
    let waits: Vec<serde_json::Value> = restart_waits(inflight, anchors)
        .into_iter()
        .map(|d| {
            serde_json::json!({
                "agent": d.worker,
                "kind": d.job.str("kind"),
                "project": d.job.str("project"),
                "job": d.job.str("id"),
                "started_at": d.started_at,
            })
        })
        .collect();
    serde_json::json!({
        "since": since,
        "inflight": inflight.len(),
        "dispatching": dispatching,
        "waits_for": waits,
    })
}

/// Minutes and whole hours as a human writes them ("44m", "1h26m"); never
/// seconds, which only suggest a precision the note does not need.
fn minutes_text(secs: u64) -> String {
    let minutes = secs / 60;
    if minutes < 60 {
        format!("{minutes}m")
    } else {
        format!("{}h{}m", minutes / 60, minutes % 60)
    }
}

/// The heartbeat note of a pending restart: the runs in flight and the ones
/// the new build waits for, each with how long it has run so far.
fn restart_note(
    now: &str,
    inflight: &BTreeMap<String, Dispatch>,
    anchors: &BTreeSet<String>,
) -> String {
    let waits = restart_waits(inflight, anchors);
    if waits.is_empty() {
        return format!("restarting: {} in flight", inflight.len());
    }
    let now_at = crate::util::must_iso(now);
    let mut names: Vec<String> = waits
        .iter()
        .take(3)
        .map(|d| {
            let ran = if d.started_at.is_empty() {
                String::new()
            } else {
                let secs = (now_at - crate::util::must_iso(&d.started_at))
                    .num_seconds()
                    .max(0) as u64;
                format!(" ({} so far)", minutes_text(secs))
            };
            format!(
                "{}'s {} on {}{ran}",
                d.worker,
                d.job.str("kind"),
                d.job.str("project")
            )
        })
        .collect();
    if waits.len() > 3 {
        names.push(format!("{} more", waits.len() - 3));
    }
    let list = match names.len() {
        1 => names.remove(0),
        2 => names.join(" and "),
        _ => {
            let last = names.pop().unwrap_or_default();
            format!("{} and {last}", names.join(", "))
        }
    };
    format!(
        "restarting: {} in flight; the new build waits for {list}",
        inflight.len()
    )
}

/// Finished runs a measured `run_fits` needs before it trusts the measurement.
const RUN_FITS_MIN_SAMPLES: usize = 10;
/// Finished runs the measurement looks back over.
const RUN_FITS_WINDOW: usize = 50;
/// Setup and report around the measured worker time of a run.
const RUN_FITS_MARGIN: Duration = Duration::from_secs(5 * 60);

/// The expected length of a run, by the project and the kind of its job.
type Fits = BTreeMap<(String, String), Duration>;

/// How long a new run of `kind` on `project` is expected to take, used to
/// decide whether one still fits inside a restart window: the 90th percentile
/// of the last fifty real runs of that kind on that project that ran a worker
/// to an end plus five minutes of setup and report. Under ten such runs the
/// project borrows the measure of the kind over every project, under ten of
/// those the measure of all runs together; the budget plus five minutes caps
/// it and stands in while fewer than ten runs exist at all. Kinds differ by an
/// order of magnitude - a ballot takes a minute, an improve run half an hour -
/// and so do projects: an improve run on the brain takes an hour where one on
/// a blog takes five minutes, so one measure per kind held the blog's runs for
/// every window and let the brain's overrun it. Runs the window outlives are
/// waited for anyway, so a low estimate costs one run's overrun before the
/// restart.
fn run_fits(s: &pulse_store::Store, project: &str, kind: &str) -> Duration {
    let cap = run_fits_cap(s);
    let Ok(conn) = s.conn() else { return cap };
    (!project.is_empty())
        .then(|| run_fits_p90(&conn, project, kind))
        .flatten()
        .or_else(|| {
            (!kind.is_empty())
                .then(|| run_fits_p90(&conn, "", kind))
                .flatten()
        })
        .or_else(|| run_fits_p90(&conn, "", ""))
        .map(|p90| (p90 + RUN_FITS_MARGIN).min(cap))
        .unwrap_or(cap)
}

/// The budget plus the margin: no run is expected to take longer.
fn run_fits_cap(s: &pulse_store::Store) -> Duration {
    Duration::from_secs((s.setting_int("minutes_per_job").max(0) as u64) * 60) + RUN_FITS_MARGIN
}

/// The 90th percentile of the worker time of the last fifty real runs of
/// `kind` on `project` (every kind, every project when empty) that ran to an
/// end, once ten exist.
fn run_fits_p90(
    conn: &(impl crate::util::Querier + ?Sized),
    project: &str,
    kind: &str,
) -> Option<Duration> {
    let mut sql = String::from(
        "SELECT r.seconds FROM runs r LEFT JOIN jobs j ON j.id = r.job_id WHERE r.finished_at != ''
         AND r.mode = 'real' AND r.seconds > 0 AND r.status IN ('done', 'failed', 'timeout')",
    );
    let limit = RUN_FITS_WINDOW as i64;
    let mut args: Vec<&dyn rusqlite::ToSql> = Vec::new();
    if !project.is_empty() {
        sql.push_str(" AND j.project = ?");
        args.push(&project);
    }
    if !kind.is_empty() {
        sql.push_str(" AND j.kind = ?");
        args.push(&kind);
    }
    sql.push_str(" ORDER BY r.finished_at DESC LIMIT ?");
    args.push(&limit);
    let mut seconds: Vec<f64> = crate::util::must_rows(conn, &sql, &args)
        .iter()
        .map(|r| r.float("seconds"))
        .collect();
    if seconds.len() < RUN_FITS_MIN_SAMPLES {
        return None;
    }
    seconds.sort_by(f64::total_cmp);
    let p90 = seconds[(seconds.len() * 9 / 10).min(seconds.len() - 1)];
    Some(Duration::from_secs(p90.ceil() as u64))
}

/// The expected length of a run of each kind of job on each active project,
/// measured once per tick.
///
/// A store error is said on stderr instead of swallowed: an empty table
/// would otherwise close every slot during a restart hold while the daemon
/// looks merely idle.
fn kind_fits(s: &pulse_store::Store) -> Fits {
    kind_fits_result(s).unwrap_or_else(|e| {
        eprintln!("daemon: run fits unavailable: {e}; holding every kind of run this tick");
        Fits::new()
    })
}

fn kind_fits_result(s: &pulse_store::Store) -> Result<Fits, String> {
    let projects = s.projects(true).map_err(|e| e.to_string())?;
    Ok(projects
        .iter()
        .flat_map(|p| {
            pulse_board::JOB_KINDS
                .iter()
                .map(move |kind| (p.slug.clone(), kind.to_string()))
        })
        .map(|(project, kind)| {
            let fit = run_fits(s, &project, &kind);
            ((project, kind), fit)
        })
        .collect())
}

/// The project and kind pairs whose run fits a restart window with `left`
/// to go: their worker is expected to be done before then, their setup and
/// report running into the margin the window already holds for the runs in
/// flight; a pair without a measure needs more than a whole budget of
/// window. All of them means no hold.
fn kinds_within(fits: &Fits, left: Duration) -> Vec<(String, String)> {
    fits.iter()
        .filter(|(_, fit)| **fit < left + RUN_FITS_MARGIN)
        .map(|(pair, _)| pair.clone())
        .collect()
}

/// The queued work (project, kind pairs of `WORK_KINDS` and experiments on
/// active projects) a restart armed now would refuse to start while the
/// runs in flight finish. A self-update build ends in that restart, so it
/// waits for them to start; with nothing in flight the restart is at once
/// and refuses nothing. The owner's work (`OWNER_DRIVEN_SQL`) is never
/// among them: it starts in any window (`restart_fits`), so the build does
/// not wait for it.
pub(crate) fn refused_queued_work(
    s: &pulse_store::Store,
    inflight: &BTreeMap<String, Dispatch>,
    now: &str,
) -> Vec<(String, String)> {
    if inflight.is_empty() {
        return Vec::new();
    }
    let Ok(conn) = s.conn() else {
        return Vec::new();
    };
    let queued: Vec<(String, String)> = crate::util::must_rows(
        &conn,
        &format!(
            "SELECT DISTINCT j.project, j.kind FROM jobs j JOIN projects p ON p.slug = j.project
             WHERE j.status = 'queued' AND p.status = 'active' AND j.kind IN ({}, 'experiment')
             AND NOT {}
             ORDER BY j.project, j.kind",
            crate::jobs::work_kinds_sql(),
            crate::jobs::OWNER_DRIVEN_SQL
        ),
        &[],
    )
    .iter()
    .map(|r| (r.str("project"), r.str("kind")))
    .collect();
    if queued.is_empty() {
        return Vec::new();
    }
    let fits = kind_fits(s);
    let anchors = inflight.keys().cloned().collect();
    let window = restart_window(
        restart_drain(s),
        now,
        anchor_ends(s, inflight, &anchors, &fits),
    );
    let within = kinds_within(&fits, window);
    queued
        .into_iter()
        .filter(|pair| !within.contains(pair))
        .collect()
}

/// Whether the owner's work (`OWNER_DRIVEN_SQL`) is queued on an active
/// project. A restart whose drain is over and whose window nothing fits
/// stays for it, one more drain at most: the next tick leases it on the
/// owner lane and its run anchors the window, where leaving would have
/// stranded the job until the next daemon found a window it fit, which for
/// a task on the brain was never.
fn owner_work_queued(s: &pulse_store::Store) -> bool {
    let Ok(conn) = s.conn() else {
        return false;
    };
    !crate::util::must_rows(
        &conn,
        &format!(
            "SELECT 1 AS one FROM jobs j JOIN projects p ON p.slug = j.project
             WHERE j.status = 'queued' AND p.status = 'active' AND {} LIMIT 1",
            crate::jobs::OWNER_DRIVEN_SQL
        ),
        &[],
    )
    .is_empty()
}

/// What a restart whose drain is over, with no fleet kind fitting what is
/// left of its window, does about the owner's queued work.
#[derive(Debug, PartialEq, Eq)]
enum RestartStay {
    /// Nothing to stay for: a console drain, or no owner's work queued.
    Leaves,
    /// The owner's work is queued and the stay is within its bound: the
    /// next tick leases it on the owner lane.
    Stays,
    /// The owner's work has been queued for a whole drain since the stay
    /// began (or its last run started) and did not start: the slots cannot
    /// lease it here, and the next daemon starts it first thing.
    StayedOut,
}

/// Whether the restart stays for the owner's queued work. `since` is when
/// the stay began, set here on the first tick that stays; the caller clears
/// it when the owner's run starts or kinds fit again, so the bound measures
/// the owner's work waiting with nothing starting. One more `drain` at most:
/// work the slots cannot lease - under a disk floor, behind a project's
/// marks, needing a hand this machine is not - would hold the restart for
/// good, and the binary it waits on is the one that starts it anyway.
fn restart_stay(
    console_drain: bool,
    owner_queued: bool,
    since: &mut Option<Instant>,
    now: Instant,
    drain: Duration,
) -> RestartStay {
    if console_drain || !owner_queued {
        return RestartStay::Leaves;
    }
    let began = *since.get_or_insert(now);
    if now.saturating_duration_since(began) >= drain {
        RestartStay::StayedOut
    } else {
        RestartStay::Stays
    }
}

/// What a restart window with room for `kinds` lets the slots lease, and
/// who the hold line says starts. The owner's work starts in every window:
/// `lease_job` admits owner-driven jobs beside the pairs, and with no pair
/// fitting the slots get the owner lane marker, owner-driven work alone.
/// Measured on the brain, a task runs 2.5 h against a 20-minute drain, so
/// the owner's tasks never fit a window and one waited 13 hours through the
/// day's seven restarts with no attempt. A console drain starts nothing:
/// the owner asked for the restart itself, so the runs in flight are waited
/// out.
fn restart_fits(
    kinds: Vec<(String, String)>,
    console_drain: bool,
) -> (Vec<(String, String)>, String) {
    if console_drain {
        return (Vec::new(), "no new runs".to_string());
    }
    if kinds.is_empty() {
        return (
            crate::jobs::owner_lane_fits(),
            "only the owner's work starts".to_string(),
        );
    }
    let who = format!("the owner's work and {} start", fits_phrase(&kinds));
    (kinds, who)
}

/// Whether a run dispatched now anchors a pending restart. The fleet's runs
/// do not: each would open a fresh window and the restart would never come
/// while the queue had work. The owner's do: the restart waits for the run
/// anyway, so the window follows it to its expected end and the kinds that
/// fit keep starting, instead of nine slots idling for the 2.5 h of a task
/// on the brain.
fn anchors_restart(restart_pending: bool, job: &Row) -> bool {
    restart_pending && crate::jobs::is_owner_driven(job)
}

/// The shortest expected run among the pairs: the window nothing fits.
fn shortest_fit(fits: &Fits) -> Duration {
    fits.values().copied().min().unwrap_or_default()
}

/// The runs that still fit, said by project: "any run on blog, lamplitlabs;
/// ballot, lead, reflect runs on amistio-template". Projects with the same
/// fitting kinds share a clause, kinds in the order of
/// `pulse_board::JOB_KINDS`.
pub(crate) fn fits_phrase(pairs: &[(String, String)]) -> String {
    let mut by_project: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (project, kind) in pairs {
        by_project.entry(project).or_default().push(kind);
    }
    let mut groups: BTreeMap<Vec<&str>, Vec<&str>> = BTreeMap::new();
    for (project, kinds) in by_project {
        let ordered = pulse_board::JOB_KINDS
            .iter()
            .copied()
            .filter(|kind| kinds.contains(kind))
            .collect::<Vec<_>>();
        groups.entry(ordered).or_default().push(project);
    }
    let mut clauses = groups
        .into_iter()
        .map(|(kinds, projects)| {
            if kinds.len() == pulse_board::JOB_KINDS.len() {
                format!("any run on {}", projects.join(", "))
            } else {
                format!("{} runs on {}", kinds.join(", "), projects.join(", "))
            }
        })
        .collect::<Vec<_>>();
    // The broadest clause first.
    clauses.sort_by_key(|c| !c.starts_with("any run on "));
    clauses.join("; ")
}

/// Start time and expected length of the anchor runs still in flight: the
/// measure of their project and kind, never past the budget that ends them.
fn anchor_ends(
    s: &pulse_store::Store,
    inflight: &BTreeMap<String, Dispatch>,
    anchors: &BTreeSet<String>,
    fits: &Fits,
) -> Vec<(String, Duration)> {
    let budget = Duration::from_secs((s.setting_int("minutes_per_job").max(0) as u64) * 60);
    let cap = run_fits_cap(s);
    inflight
        .iter()
        .filter(|(key, _)| anchors.contains(*key))
        .map(|(_, d)| {
            let fit = fits
                .get(&(d.job.str("project"), d.job.str("kind")))
                .copied()
                .unwrap_or(cap);
            (d.started_at.clone(), fit.min(budget))
        })
        .collect()
}

/// What the first restart notice says about new runs: only the owner's work
/// starts (no fleet kind fits the window), the owner's work and some kinds
/// (named by project and kind), or all of them.
fn restart_starts_phrase(
    pairs: &[(String, String)],
    all: usize,
    fits: &Fits,
    window: Duration,
    drain: Duration,
) -> String {
    if pairs.is_empty() {
        return format!(
            "only the owner's work starts (one fleet run may take up to {} and would not fit the {} drain)",
            pulse_cli::format_duration(shortest_fit(fits)),
            pulse_cli::format_duration(drain)
        );
    }
    let who = if pairs.len() == all {
        "new runs".to_string()
    } else {
        format!("the owner's work and {}", fits_phrase(pairs))
    };
    if window > drain {
        format!(
            "{who} may start for up to {} while the slowest run in flight is expected to go on",
            pulse_cli::format_duration(window)
        )
    } else {
        format!(
            "{who} may start for up to {}",
            pulse_cli::format_duration(drain)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::*;
    use crate::test_helpers_daemon::{
        dispatch_of, finished_job_on, finished_run, finished_run_of_kind, pairs, queued_job,
        store_with_projects,
    };

    #[test]
    fn kind_fits_reports_store_failure_instead_of_empty_table() {
        let s = new_store();
        s.conn()
            .unwrap()
            .execute("DROP TABLE projects", [])
            .unwrap();
        let err =
            kind_fits_result(&s).expect_err("a missing projects table must surface as an error");
        assert!(err.contains("projects"), "{err}");
        // The tick keeps going on an empty table; the error is already said.
        assert!(kind_fits(&s).is_empty());
    }

    #[test]
    fn run_fits_is_the_budget_until_ten_runs_have_finished() {
        let s = new_store();
        for n in 0..9 {
            finished_run(&s, n, n as f64, "done", "real", 60.0);
        }
        // minutes_per_job is 10 in the test store: the budget plus five.
        assert_eq!(run_fits(&s, "proj", "task"), Duration::from_secs(15 * 60));
        finished_run(&s, 9, 9.0, "done", "real", 60.0);
        // The history names no jobs: every project and kind borrows the one measure.
        assert_eq!(
            run_fits(&s, "proj", "task"),
            Duration::from_secs(60 + 5 * 60)
        );
        assert_eq!(
            run_fits(&s, "other", "ballot"),
            Duration::from_secs(60 + 5 * 60)
        );
    }

    #[test]
    fn run_fits_measures_each_kind_of_job_on_its_own() {
        let s = store_with_projects(&["proj"]);
        crate::test_helpers_daemon::set_setting(&s, "minutes_per_job", "120");
        // Ten ballots of a minute, ten improve runs of half an hour, and one
        // lead run of a minute: too few leads to measure, so they borrow the
        // measure of all kinds together.
        for n in 0..10 {
            finished_run_of_kind(&s, n, "ballot", 60.0);
        }
        for n in 10..20 {
            finished_run_of_kind(&s, n, "improve", 1800.0);
        }
        finished_run_of_kind(&s, 20, "lead", 60.0);
        assert_eq!(
            run_fits(&s, "proj", "ballot"),
            Duration::from_secs(60 + 5 * 60)
        );
        assert_eq!(
            run_fits(&s, "proj", "improve"),
            Duration::from_secs(1800 + 5 * 60)
        );
        assert_eq!(
            run_fits(&s, "proj", "lead"),
            Duration::from_secs(1800 + 5 * 60)
        );
        let fits = kind_fits(&s);
        assert_eq!(fits.len(), pulse_board::JOB_KINDS.len());
        assert_eq!(shortest_fit(&fits), Duration::from_secs(60 + 5 * 60));
        // Twelve minutes left: a ballot's worker is expected to be done, an
        // improve run's is not; nothing fits half a minute.
        assert_eq!(
            kinds_within(&fits, Duration::from_secs(12 * 60)),
            pairs("proj", &["ballot"])
        );
        assert!(kinds_within(&fits, Duration::from_secs(30)).is_empty());
        // Forty minutes: every kind fits.
        assert_eq!(
            kinds_within(&fits, Duration::from_secs(40 * 60)).len(),
            pulse_board::JOB_KINDS.len()
        );
    }

    #[test]
    fn run_fits_measures_the_ninetieth_percentile_of_recent_real_runs() {
        let s = new_store();
        crate::test_helpers_daemon::set_setting(&s, "minutes_per_job", "120");
        // Fifty runs in the window: forty-five of a minute, five of ten.
        for n in 0..45 {
            let status = if n % 2 == 0 { "done" } else { "failed" };
            finished_run(&s, n, 10.0 + n as f64, status, "real", 60.0);
        }
        for n in 45..50 {
            let status = if n == 45 { "timeout" } else { "done" };
            finished_run(&s, n, 10.0 + n as f64, status, "real", 600.0);
        }
        // Neither a dead worker, a setup error, a dry run (all more recent)
        // nor a real run older than the window moves the estimate.
        finished_run(&s, 50, 1.0, "lost", "real", 50_000.0);
        finished_run(&s, 51, 2.0, "error", "real", 50_000.0);
        finished_run(&s, 52, 3.0, "done", "dry-run", 50_000.0);
        finished_run(&s, 53, 100.0, "done", "real", 50_000.0);
        assert_eq!(
            run_fits(&s, "proj", "task"),
            Duration::from_secs(600 + 5 * 60)
        );
    }

    #[test]
    fn run_fits_never_exceeds_the_budget() {
        let s = new_store();
        for n in 0..10 {
            finished_run(&s, n, n as f64, "done", "real", 10_000.0);
        }
        assert_eq!(run_fits(&s, "proj", "task"), Duration::from_secs(15 * 60));
    }

    #[test]
    fn run_fits_measures_a_project_s_own_runs_before_its_kind_s() {
        let s = store_with_projects(&["brain", "blog", "fresh"]);
        crate::test_helpers_daemon::set_setting(&s, "minutes_per_job", "120");
        // Ten improve runs of an hour on the brain, ten of a minute on the
        // blog, and a project with no history of its own.
        for n in 0..10 {
            finished_job_on(&s, n, "brain", "improve");
            finished_run(&s, n, 10.0 + n as f64, "done", "real", 3600.0);
        }
        for n in 10..20 {
            finished_job_on(&s, n, "blog", "improve");
            finished_run(&s, n, 10.0 + n as f64, "done", "real", 60.0);
        }
        let hour = Duration::from_secs(3600 + 5 * 60);
        let minute = Duration::from_secs(60 + 5 * 60);
        assert_eq!(run_fits(&s, "brain", "improve"), hour);
        assert_eq!(run_fits(&s, "blog", "improve"), minute);
        // No history of its own: the kind over every project, whose 90th
        // percentile is the brain's hour; a kind nobody ran borrows all runs.
        assert_eq!(run_fits(&s, "fresh", "improve"), hour);
        assert_eq!(run_fits(&s, "blog", "ballot"), hour);
        let fits = kind_fits(&s);
        assert_eq!(fits.len(), 3 * pulse_board::JOB_KINDS.len());
        // Ten minutes left: only the blog's improve runs fit.
        let within = kinds_within(&fits, Duration::from_secs(10 * 60));
        assert_eq!(within, pairs("blog", &["improve"]));
        assert_eq!(fits_phrase(&within), "improve runs on blog");
        // Two hours: everything.
        assert_eq!(
            kinds_within(&fits, Duration::from_secs(120 * 60)).len(),
            fits.len()
        );
        // The phrase groups projects by what fits and says the broadest first.
        let mut mixed = pairs("blog", pulse_board::JOB_KINDS);
        mixed.extend(pairs("fresh", pulse_board::JOB_KINDS));
        mixed.extend(pairs("brain", &["ballot", "lead", "reflect"]));
        assert_eq!(
            fits_phrase(&mixed),
            "any run on blog, fresh; ballot, reflect, lead runs on brain"
        );
    }

    #[test]
    fn a_self_update_waits_for_queued_work_the_restart_would_refuse() {
        let s = store_with_projects(&["proj"]);
        crate::test_helpers_daemon::set_setting(&s, "minutes_per_job", "120");
        crate::test_helpers_daemon::set_setting(&s, "daemon.restart_drain_minutes", "0");
        // Ten tasks of a minute and ten improve runs of an hour.
        for n in 0..10 {
            finished_run_of_kind(&s, n, "task", 60.0);
        }
        for n in 10..20 {
            finished_run_of_kind(&s, n, "improve", 3600.0);
        }
        let queued = |n: usize, kind: &str| {
            crate::util::exec1(
                &*s.conn().unwrap(),
                "INSERT INTO jobs (id, project, kind, title, brief, status, created_at) VALUES (?, 'proj', ?, 'queued', '', 'queued', ?)",
                &[&format!("queued-{n}"), &kind, &TEST_NOW],
            );
        };
        queued(1, "improve");
        queued(2, "task");
        // A reflect is not work the fleet waits on; it never holds the build.
        queued(3, "reflect");
        let dispatch = |slot: &str, kind: &str| {
            let mut job = Row::new();
            job.0
                .insert("kind".into(), serde_json::Value::String(kind.into()));
            job.0
                .insert("project".into(), serde_json::Value::String("proj".into()));
            Dispatch {
                slot: slot.into(),
                worker: format!("w-{slot}"),
                job,
                started_at: TEST_NOW.into(),
                ..Default::default()
            }
        };
        // Nothing in flight: the restart is at once and refuses nothing.
        assert!(refused_queued_work(&s, &BTreeMap::new(), TEST_NOW).is_empty());
        // A task in flight ends within minutes: its window admits the queued
        // task but not the hour-long improve run.
        let mut inflight = BTreeMap::new();
        inflight.insert("a".to_string(), dispatch("a", "task"));
        let refused = refused_queued_work(&s, &inflight, TEST_NOW);
        assert_eq!(refused, pairs("proj", &["improve"]));
        assert_eq!(fits_phrase(&refused), "improve runs on proj");
        // An improve run in flight stretches the window to its hour: all fit.
        inflight.insert("b".to_string(), dispatch("b", "improve"));
        assert!(refused_queued_work(&s, &inflight, TEST_NOW).is_empty());
    }

    #[test]
    fn refused_queued_work_leaves_the_owner_s_work_out() {
        let s = store_with_projects(&["brain"]);
        crate::test_helpers_daemon::set_setting(&s, "minutes_per_job", "180");
        crate::test_helpers_daemon::set_setting(&s, "daemon.restart_drain_minutes", "20");
        // Ten ballots of a minute; ten tasks and ten improve runs of two and
        // a half hours, as on the brain.
        for n in 0..10 {
            finished_job_on(&s, n, "brain", "ballot");
            finished_run(&s, n, 10.0 + n as f64, "done", "real", 60.0);
        }
        for n in 10..20 {
            finished_job_on(&s, n, "brain", "task");
            finished_run(&s, n, 10.0 + n as f64, "done", "real", 9000.0);
        }
        for n in 20..30 {
            finished_job_on(&s, n, "brain", "improve");
            finished_run(&s, n, 10.0 + n as f64, "done", "real", 9000.0);
        }
        queued_job(&s, "fleet-improve", "brain", "improve", "evolve");
        queued_job(&s, "owner-task", "brain", "task", "task:TASK-001");
        // A ballot in flight: its window has no room for a task or an improve
        // run on the brain. The fleet's improve job is refused; the owner's
        // task is not among the refused, since it starts in any window.
        let mut inflight = BTreeMap::new();
        inflight.insert(
            "a".to_string(),
            dispatch_of("a", "brain", "ballot", "schedule"),
        );
        assert_eq!(
            refused_queued_work(&s, &inflight, TEST_NOW),
            pairs("brain", &["improve"])
        );
        // The fleet's own task of the same kind is refused as before.
        queued_job(&s, "fleet-task", "brain", "task", "evolve");
        assert_eq!(
            refused_queued_work(&s, &inflight, TEST_NOW),
            pairs("brain", &["improve", "task"])
        );
    }

    #[test]
    fn owner_work_queued_sees_the_owner_s_jobs_alone() {
        let s = store_with_projects(&["brain"]);
        assert!(!owner_work_queued(&s));
        queued_job(&s, "fleet-improve", "brain", "improve", "evolve");
        queued_job(&s, "role-sitting", "brain", "lead", "lead-role:reviewer");
        assert!(
            !owner_work_queued(&s),
            "the fleet's work and a role lead's sitting are not the owner's"
        );
        queued_job(&s, "owner-task", "brain", "task", "task:TASK-001");
        assert!(owner_work_queued(&s));
        crate::util::exec1(
            &*s.conn().unwrap(),
            "UPDATE jobs SET status = 'leased' WHERE id = 'owner-task'",
            &[],
        );
        assert!(!owner_work_queued(&s), "a leased job is no longer queued");
        queued_job(&s, "sitting", "brain", "lead", "");
        assert!(
            owner_work_queued(&s),
            "the project lead's sitting is the owner's"
        );
        crate::util::exec1(
            &*s.conn().unwrap(),
            "UPDATE projects SET status = 'paused' WHERE slug = 'brain'",
            &[],
        );
        assert!(
            !owner_work_queued(&s),
            "a paused project's queue does not count"
        );
    }

    #[test]
    fn the_restart_stays_for_the_owner_s_queued_work_one_drain_at_most() {
        let drain = Duration::from_secs(20 * 60);
        let t0 = Instant::now();
        let at = |minutes: u64| t0 + Duration::from_secs(minutes * 60);
        let mut since = None;
        // Nothing to stay for: a console drain, or no owner's work queued;
        // neither starts the stay's clock.
        assert_eq!(
            restart_stay(true, true, &mut since, t0, drain),
            RestartStay::Leaves
        );
        assert_eq!(
            restart_stay(false, false, &mut since, t0, drain),
            RestartStay::Leaves
        );
        assert_eq!(since, None);
        // The owner's work queued: the stay begins and holds for a drain.
        assert_eq!(
            restart_stay(false, true, &mut since, t0, drain),
            RestartStay::Stays
        );
        assert_eq!(since, Some(t0));
        assert_eq!(
            restart_stay(false, true, &mut since, at(19), drain),
            RestartStay::Stays
        );
        assert_eq!(
            restart_stay(false, true, &mut since, at(20), drain),
            RestartStay::StayedOut,
            "the owner's work the slots cannot lease would hold the restart for good"
        );
        // The caller clears the clock when the owner's run starts: a job
        // queued after it gets a drain of its own.
        since = None;
        assert_eq!(
            restart_stay(false, true, &mut since, at(25), drain),
            RestartStay::Stays
        );
        assert_eq!(since, Some(at(25)));
        // A zero drain restarts at once; it stays for nothing.
        let mut at_once = None;
        assert_eq!(
            restart_stay(false, true, &mut at_once, t0, Duration::ZERO),
            RestartStay::StayedOut
        );
    }

    #[test]
    fn restart_fits_opens_every_window_to_the_owner_s_work() {
        // No fleet kind fits: the owner lane marker, owner-driven work alone.
        let (fits, who) = restart_fits(Vec::new(), false);
        assert_eq!(fits, crate::jobs::owner_lane_fits());
        assert_eq!(who, "only the owner's work starts");
        // Some fit: those pairs, which lease_job admits beside the owner's work.
        let kinds = pairs("blog", &["ballot", "reflect"]);
        let (fits, who) = restart_fits(kinds.clone(), false);
        assert_eq!(fits, kinds);
        assert_eq!(
            who,
            "the owner's work and ballot, reflect runs on blog start"
        );
        // A console drain starts nothing, whatever fits: the owner asked for
        // the restart itself.
        let (fits, who) = restart_fits(kinds, true);
        assert!(fits.is_empty());
        assert_eq!(who, "no new runs");
        assert!(restart_fits(Vec::new(), true).0.is_empty());
        // The first notice says the same.
        let fits: Fits = [(
            ("brain".to_string(), "task".to_string()),
            Duration::from_secs(150 * 60),
        )]
        .into_iter()
        .collect();
        assert_eq!(
            restart_starts_phrase(
                &[],
                1,
                &fits,
                Duration::from_secs(20 * 60),
                Duration::from_secs(20 * 60)
            ),
            "only the owner's work starts (one fleet run may take up to 2h30m0s and would not fit the 20m0s drain)"
        );
        assert_eq!(
            restart_starts_phrase(
                &pairs("blog", &["ballot"]),
                2,
                &fits,
                Duration::from_secs(20 * 60),
                Duration::from_secs(20 * 60)
            ),
            "the owner's work and ballot runs on blog may start for up to 20m0s"
        );
    }

    #[test]
    fn an_owner_s_run_anchors_the_restart_window_and_the_fleet_s_does_not() {
        let owner = dispatch_of("a", "brain", "task", "task:TASK-001");
        let fleet = dispatch_of("b", "brain", "improve", "evolve");
        let sitting = dispatch_of("c", "brain", "lead", "");
        assert!(anchors_restart(true, &owner.job));
        assert!(anchors_restart(true, &sitting.job));
        assert!(
            !anchors_restart(true, &fleet.job),
            "a fleet run would open a fresh window each time and the restart would never come"
        );
        assert!(
            !anchors_restart(false, &owner.job),
            "with no restart pending there is no window to anchor"
        );
    }

    #[test]
    fn restart_window_follows_the_slowest_run_in_flight() {
        let now = TEST_NOW;
        let ago = |minutes: f64| crate::util::plus_minutes(now, -minutes);
        let ten = Duration::from_secs(10 * 60);
        // Nothing anchored: the drain alone.
        assert_eq!(
            restart_window(Duration::from_secs(90), now, []),
            Duration::from_secs(90)
        );
        // A run three minutes in, expected to take ten: seven minutes left
        // beats a ninety-second drain; a longer drain still wins.
        let three = ago(3.0);
        assert_eq!(
            restart_window(Duration::from_secs(90), now, [(three.clone(), ten)]),
            Duration::from_secs(7 * 60)
        );
        assert_eq!(
            restart_window(Duration::from_secs(30 * 60), now, [(three.clone(), ten)]),
            Duration::from_secs(30 * 60)
        );
        // Several anchors: the one expected to end last sets the window.
        let eight = ago(8.0);
        assert_eq!(
            restart_window(Duration::ZERO, now, [(eight, ten), (three.clone(), ten)]),
            Duration::from_secs(7 * 60)
        );
        // A quick kind started later still ends sooner.
        let one = ago(1.0);
        assert_eq!(
            restart_window(
                Duration::ZERO,
                now,
                [(one, Duration::from_secs(60)), (three, ten)]
            ),
            Duration::from_secs(7 * 60)
        );
        // Past its expectation: the run is waited for, but opens no window.
        let eleven = ago(11.0);
        assert_eq!(
            restart_window(Duration::ZERO, now, [(eleven, ten)]),
            Duration::ZERO
        );
    }

    #[test]
    fn anchor_ends_measure_each_run_by_its_kind_never_past_the_budget() {
        let s = new_store();
        // minutes_per_job is 10: an unmeasured kind is expected to run to it.
        let job = |kind: &str| {
            let mut r = Row::new();
            r.0.insert("kind".into(), serde_json::Value::String(kind.into()));
            r.0.insert("project".into(), serde_json::Value::String("proj".into()));
            r
        };
        let dispatch = |slot: &str, kind: &str, started: &str| Dispatch {
            slot: slot.into(),
            worker: format!("w-{slot}"),
            job: job(kind),
            started_at: started.into(),
            ..Default::default()
        };
        let mut inflight = BTreeMap::new();
        inflight.insert(
            "a".to_string(),
            dispatch("a", "ballot", "2026-01-01T00:00:00Z"),
        );
        inflight.insert(
            "b".to_string(),
            dispatch("b", "improve", "2026-01-01T00:01:00Z"),
        );
        inflight.insert(
            "c".to_string(),
            dispatch("c", "task", "2026-01-01T00:02:00Z"),
        );
        let anchors: BTreeSet<String> = ["a", "b"].into_iter().map(String::from).collect();
        let fits: Fits = [
            (
                ("proj".to_string(), "ballot".to_string()),
                Duration::from_secs(6 * 60),
            ),
            (
                ("proj".to_string(), "improve".to_string()),
                Duration::from_secs(40 * 60),
            ),
        ]
        .into_iter()
        .collect();
        let ends = anchor_ends(&s, &inflight, &anchors, &fits);
        assert_eq!(
            ends,
            vec![
                (
                    "2026-01-01T00:00:00Z".to_string(),
                    Duration::from_secs(6 * 60)
                ),
                (
                    "2026-01-01T00:01:00Z".to_string(),
                    Duration::from_secs(10 * 60)
                ),
            ],
            "the run started inside the window is no anchor, and no run outlives its budget"
        );
    }

    #[test]
    fn restart_heartbeat_names_the_runs_the_new_build_waits_for() {
        let job = |id: &str, kind: &str, project: &str| {
            let mut r = Row::new();
            r.0.insert("id".into(), serde_json::Value::String(id.into()));
            r.0.insert("kind".into(), serde_json::Value::String(kind.into()));
            r.0.insert("project".into(), serde_json::Value::String(project.into()));
            r
        };
        let dispatch = |worker: &str, job: Row, started: &str| Dispatch {
            slot: "local".into(),
            worker: worker.into(),
            job,
            started_at: started.into(),
            ..Default::default()
        };
        let mut inflight = BTreeMap::new();
        inflight.insert(
            "a".to_string(),
            dispatch(
                "fjord",
                job("job-1", "improve", "amistio-template"),
                "2026-01-01T06:22:41Z",
            ),
        );
        inflight.insert(
            "b".to_string(),
            dispatch(
                "ember",
                job("job-2", "reflect", "project-light"),
                "2026-01-01T07:04:51Z",
            ),
        );
        let anchors: BTreeSet<String> = ["a"].into_iter().map(String::from).collect();
        let now = "2026-01-01T07:07:30Z";
        // The anchor alone sets the wait while it runs; its age is rounded to
        // the minute, and the note stays one line a human can read.
        assert_eq!(
            restart_note(now, &inflight, &anchors),
            "restarting: 2 in flight; the new build waits for fjord's improve on amistio-template (44m so far)"
        );
        let beat = restart_beat("2026-01-01T06:42:03Z", &inflight, &anchors, true);
        assert_eq!(beat["since"], "2026-01-01T06:42:03Z");
        assert_eq!(beat["inflight"], 2);
        assert_eq!(beat["dispatching"], true);
        assert_eq!(
            beat["waits_for"],
            serde_json::json!([{
                "agent": "fjord",
                "kind": "improve",
                "project": "amistio-template",
                "job": "job-1",
                "started_at": "2026-01-01T06:22:41Z",
            }])
        );
        // Once the anchor is done the handover waits for whatever is left.
        inflight.remove("a");
        assert_eq!(
            restart_note(now, &inflight, &anchors),
            "restarting: 1 in flight; the new build waits for ember's reflect on project-light (2m so far)"
        );
        let beat = restart_beat("", &inflight, &anchors, false);
        assert_eq!(beat["waits_for"][0]["agent"], "ember");
        assert_eq!(beat["dispatching"], false);
        // Many anchors: three are named, the rest counted.
        for n in 0..4 {
            inflight.insert(
                format!("k{n}"),
                dispatch(
                    &format!("w{n}"),
                    job(&format!("job-{n}"), "task", "proj"),
                    "",
                ),
            );
        }
        let note = restart_note(now, &inflight, &BTreeSet::new());
        assert!(
            note.starts_with("restarting: 5 in flight; the new build waits for ember's reflect on project-light (2m so far), w0's task on proj, w1's task on proj and 2 more"),
            "{note}"
        );
        assert_eq!(
            restart_note(now, &BTreeMap::new(), &anchors),
            "restarting: 0 in flight"
        );
    }
}
