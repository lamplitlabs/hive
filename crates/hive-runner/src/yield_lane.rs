//! The owner's work goes first in its own project.
//!
//! `lease_job` already takes an owner-driven job (a `task:` job, a lead's
//! sitting, an answer) before the fleet's own work, but a project's work
//! lanes (`project_lanes`, per-project share) count every run alike:
//! when the fleet's improve jobs hold them, the owner's task waits for them
//! to finish - once four hours, behind one `redbase` job, while the daemon
//! leased fifty other runs around it. So each tick the daemon looks for an
//! owner-driven work job queued in a project whose lanes are all held and
//! some by fleet runs, and asks the youngest of those runs to hold
//! (`request_harness_hold_for_run`): the run stops at its current step with
//! its clone and commits kept, its job goes back to the queue costing it no
//! attempt, and the same agent carries it on later. `lease_job` keeps a
//! project's fleet work in the queue while an owner's work job waits there
//! (`fleet_waits_sql`), so the freed lane goes to the owner, not back to the
//! run that yielded it. A project whose default branch is red is left alone:
//! the owner's task waits behind the fix job there (`redbase`), and holding
//! the fix would only lengthen that wait.
//!
//! Two more rules, from 2026-11-03, when a role lead's sitting (kind `lead`,
//! not a work kind) was held fifty-eight times in a row for one owner task:
//! each time it held, the daemon leased it straight back into the freed lane
//! (lead jobs lease first on every slot) and asked it to hold again, while
//! the task never started. So a job that yielded to the owner's job waits,
//! whatever its kind, until that job has started or left the queue. And the
//! runner's restart window is no reason to leave a fleet run in the way:
//! owner-driven work fits every window (`lease_job`, `LeaseOpts.fits`), so
//! the lane freed for it is one it can take - a rule from 2026-11-04, when
//! an owner's task on the brain queued 13 hours with no attempt because no
//! window ever had room for a 2.5-hour `task` and no run was asked to hold
//! for a job the window would not start.
//!
//! Since the same day `lease_job` lets the owner's work job past a
//! project's narrowed lanes at once (one beyond the count, its commit cap
//! widened for the checks it shares the lock with, `owner_commit_cap`),
//! so the owner's job leases on the tick it is queued; the fleet run in its
//! way is still asked to hold, so the lanes return to their count and its
//! own commit is not killed waiting on the owner's check. Once the owner's
//! job has leased it is no longer queued, and nothing more is asked for it.

use crate::util::{Row, event_at, must_rows};
use serde_json::json;

/// The event a yield writes on the run's job: `{"project", "run", "to"}`,
/// `to` the owner's job the lane went to.
pub(crate) const YIELD_ACTION: &str = "job.yield";

/// The `lease_job` clause (over `j`) that keeps a project's fleet work jobs
/// queued while an owner-driven work job of the same project waits, and any
/// fleet job that yielded its lane to an owner's job queued while that job
/// still waits (the job's `job.yield` events name it in `to`). The red
/// default branch's fix job passes: the owner's work waits behind it.
pub(crate) fn fleet_waits_sql() -> String {
    let work = crate::jobs::work_kinds_sql();
    format!(
        "NOT (NOT {owner} AND ((j.kind IN ({work}) AND EXISTS (SELECT 1 FROM jobs q JOIN projects qp ON qp.slug = q.project WHERE q.project = j.project AND q.status = 'queued' AND qp.status = 'active' AND q.kind IN ({work}) AND {owner_q})) OR EXISTS (SELECT 1 FROM events y JOIN jobs t ON t.id = json_extract(y.detail, '$.to') WHERE y.action = '{yield}' AND y.subject = j.id AND t.status = 'queued' AND t.project = j.project)))",
        owner = crate::jobs::OWNER_DRIVEN_SQL,
        owner_q = crate::jobs::owner_driven_sql("q"),
        yield = YIELD_ACTION,
    )
}

/// One pass: for every project with an owner-driven work job queued and
/// no lane open to it - its work lanes full, or every slot of the fleet
/// taken - asks the youngest fleet run in the way to hold. Returns one log
/// line per run asked; a run already asked, or past its harness, is not
/// asked twice and makes no line. `_fits`, the daemon's restart window, is
/// no longer consulted: owner-driven work fits every window, so a run in
/// the way is asked to hold whatever the window names. Every lane admits
/// the owner's job here; the daemon asks [`yield_lanes_if_admitted`].
#[cfg(test)]
pub(crate) fn yield_lanes_to_owner(
    s: &hive_store::Store,
    now: &str,
    actor: &str,
    _fits: &[(String, String)],
) -> Vec<String> {
    yield_lanes_if_admitted(s, now, actor, &OwnerAdmission::default()).asked
}

/// Whether the owner's job could start once a fleet run yields its lane
/// (docs/owner-reservation.md). `everywhere` is what closes every slot this
/// tick (a console drain, bulk storage away, the disk floor); `local` what
/// closes this machine's alone (the login gate, the load governor:
/// `daemon_load::owner_refusal`). None admits. `free_slots` lists the
/// slots the dispatch would offer the owner's work this tick with no run
/// asked to hold - the paired machines' free slots and this machine's
/// governor room (`daemon_load::owner_room`); none free is a full fleet.
/// None counts the jobs in flight against the fleet's nominal total
/// (`capacity::fleet`), for callers without a governor. Each entry names
/// the machine of one free slot (`executor::LOCAL_EXECUTOR` for this one),
/// and a slot counts for an owner job only when its machine could run that
/// job ([`Machines::refuses`]): an idle paired machine that does not serve
/// the project or lacks what the job needs is no room for it (review of
/// PR 284, item b). `local_kept` says a
/// slot freed on this machine would be its last free one under the cap,
/// which the dispatch keeps back from the owner's work past its project's
/// share (`jobs::LeaseOpts::keep_slot`): a local run is then not asked to
/// hold for such a job, since the slot it frees would not start it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct OwnerAdmission {
    pub(crate) everywhere: Option<String>,
    pub(crate) local: Option<String>,
    pub(crate) free_slots: Option<Vec<String>>,
    pub(crate) local_kept: bool,
}

/// The machines a yield pass reads, each looked up once: whether a slot of
/// one could take a given owner job.
struct Machines<'a> {
    s: &'a hive_store::Store,
    now: &'a str,
    rows: std::collections::BTreeMap<String, (Row, std::collections::BTreeMap<String, String>)>,
}

impl<'a> Machines<'a> {
    fn new(s: &'a hive_store::Store, now: &'a str) -> Self {
        Self {
            s,
            now,
            rows: std::collections::BTreeMap::new(),
        }
    }

    /// Why a slot of machine `name` (empty is this one) could not take the
    /// owner's job `job`, or None when it could - the checks the lease
    /// makes of a machine (`run::take_job_first`, `jobs::lease_job`): it is
    /// active, serves the job's project, offers every word the job needs,
    /// takes a job that needs nothing only when it is not kept for the work
    /// that needs it, and is not marked unfit for the project.
    fn refuses(&mut self, name: &str, job: &Row) -> Option<String> {
        let name = if name.is_empty() {
            crate::executor::LOCAL_EXECUTOR
        } else {
            name
        };
        let (ex, unfit) = self.rows.entry(name.to_string()).or_insert_with(|| {
            let ex = crate::executor::executor_row(self.s, name);
            let unfit = match self.s.conn() {
                Ok(conn) => crate::recipe::unfit_projects(&*conn, name, self.now),
                Err(_) => std::collections::BTreeMap::new(),
            };
            (ex, unfit)
        });
        let slug = job.str("project");
        if ex.str("status") != "active" {
            return Some(format!("machine {name} is not active"));
        }
        if !crate::executor::executor_allows(ex, &slug) {
            return Some(format!("machine {name} does not serve {slug}"));
        }
        let needs = job.str("needs");
        let missing = hive_store::needs::missing(&needs, &crate::executor::caps(ex));
        if !missing.is_empty() {
            return Some(format!(
                "machine {name} does not offer {}, which the job needs",
                missing.join(", ")
            ));
        }
        if crate::executor::reserved(ex) && needs.trim().is_empty() {
            return Some(format!("machine {name} is kept for the work that needs it"));
        }
        unfit.get(&slug).map(|why| {
            format!(
                "machine {name} {} for {slug}",
                crate::recipe::unfit_phrase(why)
            )
        })
    }
}

/// The refusal a slot freed on this machine meets when it would be the
/// last free one, kept back from the owner's work past its share.
const KEPT_SLOT: &str = "the slot freed here would be this machine's last free one, kept back from the owner's work past its project's share";

impl OwnerAdmission {
    /// Why a lane freed on `executor` would not start the owner's job;
    /// `past_share` says the job is past its project's share, so the
    /// machine's kept slot does not take it.
    fn refusal(&self, executor: &str, past_share: bool) -> Option<&str> {
        self.everywhere.as_deref().or_else(|| {
            if !(executor.is_empty() || executor == crate::executor::LOCAL_EXECUTOR) {
                return None;
            }
            self.local
                .as_deref()
                .or_else(|| (past_share && self.local_kept).then_some(KEPT_SLOT))
        })
    }
}

/// What one yield pass did: the runs it asked to hold, and the owner's jobs
/// it asked nothing for because the freed lane would not start them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct YieldPass {
    pub(crate) asked: Vec<String>,
    pub(crate) refused: Vec<String>,
}

/// [`yield_lanes_to_owner`] asking only where the owner's job could then
/// start: a fleet run is held only when `admission` lets the owner's job
/// into the lane it frees, on the executor it runs on. Before this, a load
/// over `daemon.max_load` refused the owner's job after every yield, the
/// held run leased straight back, and the pair repeated every 25-68 s
/// (TASK-055 on 2026-11-03: 58 holds, no start in ten hours).
pub(crate) fn yield_lanes_if_admitted(
    s: &hive_store::Store,
    now: &str,
    actor: &str,
    admission: &OwnerAdmission,
) -> YieldPass {
    let mut pass = YieldPass::default();
    let Ok(conn) = s.conn() else {
        return pass;
    };
    let work = crate::jobs::work_kinds_sql();
    let waiting = must_rows(
        &conn,
        &format!(
            "SELECT j.id, j.project, j.kind, j.needs FROM jobs j JOIN projects p ON p.slug = j.project WHERE j.status = 'queued' AND p.status = 'active' AND j.kind IN ({work}) AND {} ORDER BY j.created_at, j.id",
            crate::jobs::OWNER_DRIVEN_SQL
        ),
        &[],
    );
    if waiting.is_empty() {
        return pass;
    }
    // The owner's work passes its project's share (`lease_job`, TASK-136),
    // so the share puts no run in its way; a full fleet does: no slot the
    // dispatch would offer the owner's job, a fleet run of the owner's
    // project gives its slot up. Full is the governor's room this tick, not
    // the fleet's nominal total: with the cap halved by the kernel's memory
    // warning the jobs in flight never reached the total, and nothing was
    // asked while the owner's task waited (review of PR 276). A free slot
    // counts only for an owner job its machine could run (review of PR
    // 284, item b), so that is read per job below.
    let nominal_full = match admission.free_slots {
        Some(_) => false,
        None => {
            let taken = crate::util::must_one(
                &conn,
                &format!(
                    "SELECT COUNT(*) AS n FROM jobs k JOIN projects kp ON kp.slug = k.project WHERE kp.status = 'active' AND {}",
                    crate::jobs::OCCUPIED_JOB_SQL
                ),
                &[],
            )
            .int("n");
            taken >= crate::capacity::fleet(s).total.max(1)
        }
    };
    let mut machines = Machines::new(s, now);
    let mut seen = std::collections::BTreeSet::new();
    for owner in waiting {
        let slug = owner.str("project");
        if !seen.insert(slug.clone()) {
            continue;
        }
        let fleet_full = match &admission.free_slots {
            Some(slots) => !slots
                .iter()
                .any(|name| machines.refuses(name, &owner).is_none()),
            None => nominal_full,
        };
        // The lanes stand in the owner's way only where they are narrowed
        // (its checks take turns and fewer fit the commit cap than the
        // share) or where its own work is at its bound (owner_work_bound).
        // The plain share does not: the owner's job leases past it, and a
        // fleet run asked to hold at a share of one gave up a lane the
        // owner's task did not need.
        let lanes = crate::jobs::project_lanes(s, &slug);
        let narrowed = (lanes.lanes < lanes.base).then_some(lanes.lanes.max(1));
        let bound = crate::jobs::owner_work_bound(s);
        let occupied = must_rows(
            &conn,
            &format!(
                "SELECT k.id, k.kind, k.source FROM jobs k WHERE k.project = ? AND {}",
                crate::jobs::OCCUPIED_JOB_SQL
            ),
            &[&slug],
        );
        let is_work = |k: &Row| crate::jobs::WORK_KINDS.contains(&k.str("kind").as_str());
        let work_held = occupied.iter().filter(|k| is_work(k)).count() as i32;
        // The owner's work past its share does not take the machine's last
        // free slot (jobs.rs keep_slot, counting the project's owner work
        // jobs): a run freeing that slot is not asked to hold for it.
        let owner_held = occupied
            .iter()
            .filter(|k| is_work(k) && crate::jobs::is_owner_driven(k))
            .count() as i32;
        let past_share = owner_held >= crate::jobs::parallel_per_project(s);
        let lanes_held =
            narrowed.is_some_and(|n| work_held >= n) || bound.is_some_and(|b| work_held >= b);
        let in_the_way: Vec<Row> = if lanes_held {
            occupied.into_iter().filter(|k| is_work(k)).collect()
        } else if fleet_full {
            occupied
        } else {
            continue;
        };
        let mut fleet = in_the_way
            .into_iter()
            .filter(|k| !crate::jobs::is_owner_driven(k))
            .filter_map(|k| {
                let run = crate::util::must_one(
                    &conn,
                    "SELECT id, started_at, executor FROM runs r WHERE r.job_id = ? AND r.status = 'running' ORDER BY r.started_at DESC, r.id DESC LIMIT 1",
                    &[&k.str("id")],
                );
                (!run.0.is_empty()).then(|| (run.str("started_at"), run.str("id"), k, run.str("executor")))
            })
            .collect::<Vec<_>>();
        fleet.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
        // The youngest run whose lane would start the owner's job; when no
        // freed lane would, nothing is asked and the pass says why once. A
        // lane frees on the machine the run is on, so that machine must be
        // one that could run the owner's job: a run on a paired machine
        // that does not serve the project, or lacks what the job needs,
        // would lose its step for nothing (review of PR 284, item c).
        let mut refusal = None;
        let pick = fleet.into_iter().find(|(_, _, _, executor)| {
            let why = admission
                .refusal(executor, past_share)
                .map(str::to_string)
                .or_else(|| machines.refuses(executor, &owner));
            match why {
                Some(why) => {
                    refusal.get_or_insert(why);
                    false
                }
                None => true,
            }
        });
        let Some((_, run_id, job, _)) = pick else {
            if let Some(why) = refusal {
                pass.refused.push(format!(
                    "{slug}: the owner's {} waits for a lane, but no fleet run is asked to hold while the freed lane could not start it: {why}",
                    owner.str("id")
                ));
            }
            continue;
        };
        let asked = s.request_harness_hold_for_run(&run_id).unwrap_or(0);
        if asked == 0 {
            continue;
        }
        event_at(
            &*conn,
            now,
            actor,
            YIELD_ACTION,
            &job.str("id"),
            json!({"project": slug, "run": run_id, "to": owner.str("id")}),
        );
        pass.asked.push(format!(
            "{slug}: {} ({}) holds after its current step so the owner's {} can start; it carries on later",
            job.str("id"),
            job.str("kind"),
            owner.str("id")
        ));
    }
    pass
}

/// Whether run `run_id` of `job_id` was asked to hold for the owner's work
/// (as against the runner's restart): the run's summary says which.
pub(crate) fn yielded(s: &hive_store::Store, job_id: &str, run_id: &str) -> bool {
    let Ok(conn) = s.conn() else {
        return false;
    };
    !crate::util::must_one(
        &conn,
        "SELECT 1 AS x FROM events WHERE action = ? AND subject = ? AND json_extract(detail, '$.run') = ? LIMIT 1",
        &[&YIELD_ACTION, &job_id, &run_id],
    )
    .0
    .is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn join(s: &hive_store::Store, name: &str) {
        crate::test_helpers::join(s, name);
    }

    fn queue(s: &hive_store::Store, kind: &str, source: &str) -> String {
        let (job, _) = crate::jobs::add_job(
            s,
            "proj".into(),
            kind.into(),
            source.into(),
            "brief".into(),
            1,
            10,
            source.into(),
            hive_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        job.str("id")
    }

    fn lease(s: &hive_store::Store, agent: &str, fits: Vec<(String, String)>) -> Row {
        crate::jobs::lease_job(
            s,
            agent.into(),
            crate::test_helpers::TEST_NOW.into(),
            crate::jobs::LeaseOpts {
                any_proj: true,
                fits,
                ..Default::default()
            },
        )
        .unwrap()
    }

    /// The run of a leased job, in flight with its harness session.
    fn running(s: &hive_store::Store, job: &str, agent: &str, started_at: &str) -> String {
        let run_id = format!("run-{job}");
        s.conn()
            .unwrap()
            .execute(
                "INSERT INTO runs (id, job_id, agent, project, started_at, status) VALUES (?, ?, ?, 'proj', ?, 'running')",
                rusqlite::params![run_id, job, agent, started_at],
            )
            .unwrap();
        s.begin_harness_session(hive_store::HarnessSession {
            id: format!("h-{job}"),
            run_id: run_id.clone(),
            job_id: job.into(),
            agent: agent.into(),
            project: "proj".into(),
            ..Default::default()
        })
        .unwrap();
        run_id
    }

    fn hold_requested(s: &hive_store::Store, job: &str) -> bool {
        !s.harness_session_head(&format!("h-{job}"))
            .unwrap()
            .unwrap()
            .hold_requested_at
            .is_empty()
    }

    /// One work lane: the project's checks take turns and one 10-minute
    /// check fits the commit cap.
    fn one_lane(s: &hive_store::Store) {
        crate::capacity::pin_per_project(s, 4);
        s.set_setting(
            hive_store::HUMAN_AGENT,
            crate::verify_lock::SETTING,
            crate::verify_lock::PROJECT,
        )
        .unwrap();
        let conn = s.conn().unwrap();
        event_at(
            &*conn,
            "2026-09-01T00:00:00Z",
            "daemon",
            "land.landed",
            "hive/x/1",
            json!({"project": "proj", "sha": "abc", "verify": "verify exit 0", "verify_seconds": 600}),
        );
        s.set_setting(hive_store::HUMAN_AGENT, "harness.commit_timeout", "900")
            .unwrap();
        s.set_setting(hive_store::HUMAN_AGENT, "harness.step_timeout", "900")
            .unwrap();
        assert_eq!(crate::jobs::project_lanes(s, "proj").lanes, 1);
    }

    #[test]
    fn no_fleet_run_is_asked_to_hold_while_the_freed_lane_could_not_start_the_owners_job() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        one_lane(&s);
        for name in ["aurora", "borealis"] {
            join(&s, name);
        }
        let fleet = queue(&s, "improve", "suggestion:abc");
        assert_eq!(lease(&s, "aurora", Vec::new()).str("id"), fleet);
        let run = running(&s, &fleet, "aurora", "2026-09-01T01:00:00Z");
        let task = queue(&s, "task", "task:TASK-001");
        let yields = || {
            crate::test_helpers::count(
                &s,
                "SELECT COUNT(*) AS n FROM events WHERE action = ?",
                &[&YIELD_ACTION],
            )
        };
        // The machine would refuse the owner's job once the lane is free
        // (load over the line with the owner's one run over it in flight):
        // nothing is asked, no job.yield is written, the refusal is said.
        let refused = OwnerAdmission {
            local: Some("machine saturated: load 30.0 on 12 CPUs is over the line 24".into()),
            ..Default::default()
        };
        for _ in 0..3 {
            let pass =
                yield_lanes_if_admitted(&s, crate::test_helpers::TEST_NOW, "daemon", &refused);
            assert!(pass.asked.is_empty(), "{pass:?}");
            assert_eq!(pass.refused.len(), 1, "{pass:?}");
            assert!(
                pass.refused[0].contains(&task) && pass.refused[0].contains("load 30.0"),
                "{pass:?}"
            );
        }
        assert_eq!(yields(), 0);
        assert!(!hold_requested(&s, &fleet));
        // A hold that closes every slot (a console drain, the disk floor)
        // refuses the same way, whatever executor the run is on.
        let drained = OwnerAdmission {
            everywhere: Some("restarting: waiting for 1 run(s) in flight".into()),
            ..Default::default()
        };
        assert!(
            yield_lanes_if_admitted(&s, crate::test_helpers::TEST_NOW, "daemon", &drained)
                .asked
                .is_empty()
        );
        assert_eq!(yields(), 0);
        // A refusal of this machine alone does not stop a yield on a paired
        // one: that machine carries its own load.
        paired(&s, "hands-1", "*", "", false);
        s.conn()
            .unwrap()
            .execute("UPDATE runs SET executor = 'hands-1' WHERE id = ?", [&run])
            .unwrap();
        let pass = yield_lanes_if_admitted(&s, crate::test_helpers::TEST_NOW, "daemon", &refused);
        assert_eq!(pass.asked.len(), 1, "{pass:?}");
        assert_eq!(yields(), 1);
        assert!(hold_requested(&s, &fleet));
        assert!(yielded(&s, &fleet, &run));
    }

    #[test]
    fn a_fleet_run_is_asked_to_hold_once_the_machine_would_admit_the_owners_job() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        one_lane(&s);
        join(&s, "aurora");
        let fleet = queue(&s, "improve", "suggestion:abc");
        assert_eq!(lease(&s, "aurora", Vec::new()).str("id"), fleet);
        let run = running(&s, &fleet, "aurora", "2026-09-01T01:00:00Z");
        let task = queue(&s, "task", "task:TASK-001");
        let pass = yield_lanes_if_admitted(
            &s,
            crate::test_helpers::TEST_NOW,
            "daemon",
            &OwnerAdmission::default(),
        );
        assert!(pass.refused.is_empty(), "{pass:?}");
        assert_eq!(pass.asked.len(), 1, "{pass:?}");
        assert!(pass.asked[0].contains(&task), "{pass:?}");
        assert!(yielded(&s, &fleet, &run));
    }

    #[test]
    fn no_fleet_run_is_asked_to_hold_at_the_plain_share_while_slots_are_free() {
        // Review of PR 276: the owner's work leases past its project's
        // share, so a share of one is no lane in its way. A fleet run of
        // the owner's project was still asked to hold the tick the owner's
        // task was queued, although slots were free and the task leased
        // anyway.
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        s.set_setting(hive_store::HUMAN_AGENT, "capacity", "4")
            .unwrap();
        assert_eq!(crate::jobs::parallel_per_project(&s), 1);
        for name in ["aurora", "borealis"] {
            join(&s, name);
        }
        let fleet = queue(&s, "improve", "suggestion:abc");
        assert_eq!(lease(&s, "aurora", Vec::new()).str("id"), fleet);
        let _run = running(&s, &fleet, "aurora", "2026-09-01T01:00:00Z");
        let task = queue(&s, "task", "task:TASK-001");
        let pass = yield_lanes_if_admitted(
            &s,
            crate::test_helpers::TEST_NOW,
            "daemon",
            &OwnerAdmission::default(),
        );
        assert!(pass.asked.is_empty(), "{pass:?}");
        assert!(!hold_requested(&s, &fleet));
        assert_eq!(lease(&s, "borealis", Vec::new()).str("id"), task);
    }

    #[test]
    fn a_full_fleet_is_the_slots_the_governor_offers_not_the_fleets_total() {
        // Review of PR 276: the fleet-full yield compared the jobs in flight
        // with the fleet's nominal total. With one of four slots taken and
        // no slot the governor would offer (the cap halved by the memory
        // warning, the owner's run already in it), nothing was asked.
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        s.set_setting(hive_store::HUMAN_AGENT, "capacity", "4")
            .unwrap();
        crate::capacity::pin_per_project(&s, 1);
        for name in ["aurora", "borealis"] {
            join(&s, name);
        }
        let fleet = queue(&s, "improve", "suggestion:abc");
        assert_eq!(lease(&s, "aurora", Vec::new()).str("id"), fleet);
        let _run = running(&s, &fleet, "aurora", "2026-09-01T01:00:00Z");
        let task = queue(&s, "task", "task:TASK-001");
        let free = |n: usize| OwnerAdmission {
            free_slots: Some(vec![crate::executor::LOCAL_EXECUTOR.to_string(); n]),
            ..Default::default()
        };
        // A slot the governor would offer: the owner's task takes it.
        let pass = yield_lanes_if_admitted(&s, crate::test_helpers::TEST_NOW, "daemon", &free(1));
        assert!(pass.asked.is_empty(), "{pass:?}");
        assert!(!hold_requested(&s, &fleet));
        // None, under the fleet's total of four: the fleet run holds.
        assert_eq!(crate::capacity::fleet(&s).total, 4);
        let pass = yield_lanes_if_admitted(&s, crate::test_helpers::TEST_NOW, "daemon", &free(0));
        assert_eq!(pass.asked.len(), 1, "{pass:?}");
        assert!(
            pass.asked[0].contains(&fleet) && pass.asked[0].contains(&task),
            "{pass:?}"
        );
        assert!(hold_requested(&s, &fleet));
    }

    #[test]
    fn no_local_run_holds_for_owner_work_past_its_share_when_it_would_free_the_kept_slot() {
        // Review of PR 276, item B: the fleet full, the project runs fleet
        // job F and the owner's task T1, and T2 waits. F was asked to hold,
        // but the slot it freed was the machine's last, kept back from the
        // owner's work past its share, and T2 was refused it: F lost its
        // step for nothing.
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        assert_eq!(crate::jobs::parallel_per_project(&s), 1);
        for name in ["aurora", "borealis", "cinder"] {
            join(&s, name);
        }
        let fleet = queue(&s, "improve", "suggestion:abc");
        assert_eq!(lease(&s, "aurora", Vec::new()).str("id"), fleet);
        let _run = running(&s, &fleet, "aurora", "2026-09-01T01:00:00Z");
        let first = queue(&s, "task", "task:TASK-001");
        assert_eq!(lease(&s, "borealis", Vec::new()).str("id"), first);
        let _first_run = running(&s, &first, "borealis", "2026-09-01T01:10:00Z");
        let second = queue(&s, "task", "task:TASK-002");
        let full = |local_kept: bool| OwnerAdmission {
            free_slots: Some(Vec::new()),
            local_kept,
            ..Default::default()
        };
        // The freed slot would be the kept one: nothing is asked, and the
        // pass says why.
        let pass =
            yield_lanes_if_admitted(&s, crate::test_helpers::TEST_NOW, "daemon", &full(true));
        assert!(pass.asked.is_empty(), "{pass:?}");
        assert_eq!(pass.refused.len(), 1, "{pass:?}");
        assert!(
            pass.refused[0].contains(&second) && pass.refused[0].contains("kept back"),
            "{pass:?}"
        );
        assert!(!hold_requested(&s, &fleet));
        // A slot that would not be the last one starts T2: F holds.
        let pass =
            yield_lanes_if_admitted(&s, crate::test_helpers::TEST_NOW, "daemon", &full(false));
        assert_eq!(pass.asked.len(), 1, "{pass:?}");
        assert!(pass.asked[0].contains(&fleet), "{pass:?}");
        assert!(hold_requested(&s, &fleet));
    }

    #[test]
    fn the_owners_first_task_in_its_share_is_not_refused_the_kept_slot() {
        // The kept slot refuses only the owner's work past its share: the
        // owner's first task in a project the fleet fills takes it, so the
        // fleet run in the way is asked to hold.
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        assert_eq!(crate::jobs::parallel_per_project(&s), 1);
        join(&s, "aurora");
        let fleet = queue(&s, "improve", "suggestion:abc");
        assert_eq!(lease(&s, "aurora", Vec::new()).str("id"), fleet);
        let _run = running(&s, &fleet, "aurora", "2026-09-01T01:00:00Z");
        let task = queue(&s, "task", "task:TASK-001");
        let pass = yield_lanes_if_admitted(
            &s,
            crate::test_helpers::TEST_NOW,
            "daemon",
            &OwnerAdmission {
                free_slots: Some(Vec::new()),
                local_kept: true,
                ..Default::default()
            },
        );
        assert_eq!(pass.asked.len(), 1, "{pass:?}");
        assert!(
            pass.asked[0].contains(&fleet) && pass.asked[0].contains(&task),
            "{pass:?}"
        );
    }

    #[test]
    fn a_fleet_run_holds_when_the_owners_work_is_at_the_locks_bound_with_slots_free() {
        // Review of PR 276, item G: the yield's bound branch. With the
        // verify lock on and a share of one, the owner's work runs to two
        // work jobs; at that bound a fleet work run of the project is in
        // the owner's way though slots are free, and it is asked to hold.
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        s.set_setting(hive_store::HUMAN_AGENT, "capacity", "4")
            .unwrap();
        assert_eq!(crate::jobs::parallel_per_project(&s), 1);
        s.set_setting(
            hive_store::HUMAN_AGENT,
            crate::verify_lock::SETTING,
            crate::verify_lock::PROJECT,
        )
        .unwrap();
        assert_eq!(crate::jobs::owner_work_bound(&s), Some(2));
        for name in ["aurora", "borealis"] {
            join(&s, name);
        }
        let fleet = queue(&s, "improve", "suggestion:abc");
        assert_eq!(lease(&s, "aurora", Vec::new()).str("id"), fleet);
        let _run = running(&s, &fleet, "aurora", "2026-09-01T01:00:00Z");
        let first = queue(&s, "task", "task:TASK-001");
        assert_eq!(lease(&s, "borealis", Vec::new()).str("id"), first);
        let _first_run = running(&s, &first, "borealis", "2026-09-01T01:10:00Z");
        let second = queue(&s, "task", "task:TASK-002");
        let free = OwnerAdmission {
            free_slots: Some(vec![crate::executor::LOCAL_EXECUTOR.to_string(); 2]),
            ..Default::default()
        };
        let pass = yield_lanes_if_admitted(&s, crate::test_helpers::TEST_NOW, "daemon", &free);
        assert_eq!(pass.asked.len(), 1, "{pass:?}");
        assert!(
            pass.asked[0].contains(&fleet) && pass.asked[0].contains(&second),
            "{pass:?}"
        );
        assert!(hold_requested(&s, &fleet));
        // The owner's own run is never asked.
        assert!(!hold_requested(&s, &first));
    }

    #[test]
    fn under_the_bound_with_slots_free_no_fleet_run_holds_for_a_locked_project() {
        // The bound branch's other side: one owner run in flight beside the
        // fleet's under a bound of three (a share of two), slots free.
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        crate::capacity::pin_per_project(&s, 2);
        s.set_setting(
            hive_store::HUMAN_AGENT,
            crate::verify_lock::SETTING,
            crate::verify_lock::PROJECT,
        )
        .unwrap();
        assert_eq!(crate::jobs::owner_work_bound(&s), Some(3));
        for name in ["aurora", "borealis"] {
            join(&s, name);
        }
        let fleet = queue(&s, "improve", "suggestion:abc");
        assert_eq!(lease(&s, "aurora", Vec::new()).str("id"), fleet);
        let _run = running(&s, &fleet, "aurora", "2026-09-01T01:00:00Z");
        let first = queue(&s, "task", "task:TASK-001");
        assert_eq!(lease(&s, "borealis", Vec::new()).str("id"), first);
        let _first_run = running(&s, &first, "borealis", "2026-09-01T01:10:00Z");
        let _second = queue(&s, "task", "task:TASK-002");
        let pass = yield_lanes_if_admitted(
            &s,
            crate::test_helpers::TEST_NOW,
            "daemon",
            &OwnerAdmission {
                free_slots: Some(vec![crate::executor::LOCAL_EXECUTOR.to_string(); 2]),
                ..Default::default()
            },
        );
        assert!(pass.asked.is_empty(), "{pass:?}");
        assert!(!hold_requested(&s, &fleet));
    }

    /// A paired machine `name` serving `projects`, active, offering
    /// `labels` and marked reserved or not, as the probe leaves it.
    fn paired(s: &hive_store::Store, name: &str, projects: &str, labels: &str, reserved: bool) {
        crate::executor::add_executor(
            s,
            crate::test_helpers::TEST_NOW,
            name,
            "ci@box",
            "/w",
            1,
            projects,
            "human",
        )
        .unwrap();
        s.conn()
            .unwrap()
            .execute(
                "UPDATE executors SET status = 'active', os = 'linux', arch = 'x86_64', labels = ?, reserved = ? WHERE name = ?",
                rusqlite::params![labels, i64::from(reserved), name],
            )
            .unwrap();
    }

    /// A fleet run of `job` in flight on `executor`.
    fn running_on(s: &hive_store::Store, job: &str, agent: &str, started_at: &str, executor: &str) {
        let run_id = running(s, job, agent, started_at);
        s.conn()
            .unwrap()
            .execute(
                "UPDATE runs SET executor = ? WHERE id = ?",
                rusqlite::params![executor, run_id],
            )
            .unwrap();
    }

    #[test]
    fn a_free_paired_slot_the_owners_job_cannot_use_is_no_room_for_it() {
        // Review of PR 284, item b: every free slot of a paired machine
        // counted as room, whatever the waiting job needs, so a fleet run
        // was never asked to hold while an idle machine that could not run
        // the owner's job stood by. A slot now counts only on a machine
        // that serves the project, offers what the job needs, is not kept
        // for other work and is not marked unfit for the project.
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        for name in ["aurora", "borealis"] {
            join(&s, name);
        }
        paired(&s, "other", "elsewhere", "", false);
        paired(&s, "kept", "*", "", true);
        paired(&s, "unfit", "*", "", false);
        crate::recipe::mark_unfit(
            &*s.conn().unwrap(),
            crate::test_helpers::TEST_NOW,
            "unfit",
            "proj",
            "cargo",
        );
        let fleet = queue(&s, "improve", "suggestion:abc");
        assert_eq!(lease(&s, "aurora", Vec::new()).str("id"), fleet);
        let _run = running(&s, &fleet, "aurora", "2026-09-01T01:00:00Z");
        let task = queue(&s, "task", "task:TASK-001");
        let slots = |names: &[&str]| OwnerAdmission {
            free_slots: Some(names.iter().map(|n| n.to_string()).collect()),
            ..Default::default()
        };
        // None of these four could run the owner's task: the fleet is
        // full for it, and the fleet's run here is asked to hold.
        let pass = yield_lanes_if_admitted(
            &s,
            crate::test_helpers::TEST_NOW,
            "daemon",
            &slots(&["other", "kept", "unfit"]),
        );
        assert_eq!(pass.asked.len(), 1, "{pass:?}");
        assert!(pass.asked[0].contains(&task), "{pass:?}");
        assert!(hold_requested(&s, &fleet));
        // A job needing cuda: only the gpu machine is room for it.
        let s2 = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s2);
        for name in ["aurora", "borealis"] {
            join(&s2, name);
        }
        paired(&s2, "gpu", "*", "cuda", false);
        paired(&s2, "box", "*", "", false);
        let fleet2 = queue(&s2, "improve", "suggestion:abc");
        assert_eq!(lease(&s2, "aurora", Vec::new()).str("id"), fleet2);
        // The fleet's run is on the gpu: the slot it frees could take a
        // job that needs cuda.
        running_on(&s2, &fleet2, "aurora", "2026-09-01T01:00:00Z", "gpu");
        let task2 = queue(&s2, "task", "task:TASK-001");
        s2.conn()
            .unwrap()
            .execute("UPDATE jobs SET needs = 'cuda' WHERE id = ?", [&task2])
            .unwrap();
        // The gpu's free slot is room for it: nothing is asked.
        let pass = yield_lanes_if_admitted(
            &s2,
            crate::test_helpers::TEST_NOW,
            "daemon",
            &slots(&["gpu"]),
        );
        assert!(pass.asked.is_empty(), "{pass:?}");
        assert!(!hold_requested(&s2, &fleet2));
        // A free slot on a machine without cuda is not.
        let pass = yield_lanes_if_admitted(
            &s2,
            crate::test_helpers::TEST_NOW,
            "daemon",
            &slots(&["box"]),
        );
        assert_eq!(pass.asked.len(), 1, "{pass:?}");
        assert!(pass.asked[0].contains(&task2), "{pass:?}");
        assert!(hold_requested(&s2, &fleet2));
    }

    #[test]
    fn a_fleet_run_on_a_paired_machine_the_owners_job_cannot_use_is_not_asked_to_hold() {
        // Review of PR 284, item c: the yield asks only when the slot it
        // frees could take the job it is asked for. The fleet's two runs
        // fill the fleet; the younger is on a paired machine that does not
        // serve the project, so freeing it would not start the owner's
        // task: the older, on this machine, is asked instead. When only
        // the paired machine's run is in the way, none is asked and the
        // pass says why.
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        for name in ["aurora", "borealis", "cinder"] {
            join(&s, name);
        }
        crate::capacity::pin_per_project(&s, 4);
        paired(&s, "other", "elsewhere", "", false);
        let older = queue(&s, "improve", "suggestion:abc");
        assert_eq!(lease(&s, "aurora", Vec::new()).str("id"), older);
        running_on(&s, &older, "aurora", "2026-09-01T01:00:00Z", "local");
        let younger = queue(&s, "improve", "suggestion:def");
        assert_eq!(lease(&s, "borealis", Vec::new()).str("id"), younger);
        running_on(&s, &younger, "borealis", "2026-09-01T02:00:00Z", "other");
        let task = queue(&s, "task", "task:TASK-001");
        let full = OwnerAdmission {
            free_slots: Some(Vec::new()),
            ..Default::default()
        };
        let pass = yield_lanes_if_admitted(&s, crate::test_helpers::TEST_NOW, "daemon", &full);
        assert_eq!(pass.asked.len(), 1, "{pass:?}");
        assert!(
            pass.asked[0].contains(&older) && pass.asked[0].contains(&task),
            "{pass:?}"
        );
        assert!(hold_requested(&s, &older));
        assert!(!hold_requested(&s, &younger));
        // The local run gone, the paired one alone stands in the way.
        let s2 = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s2);
        for name in ["aurora", "borealis"] {
            join(&s2, name);
        }
        paired(&s2, "other", "elsewhere", "", false);
        let only = queue(&s2, "improve", "suggestion:abc");
        assert_eq!(lease(&s2, "aurora", Vec::new()).str("id"), only);
        running_on(&s2, &only, "aurora", "2026-09-01T01:00:00Z", "other");
        let task2 = queue(&s2, "task", "task:TASK-001");
        let pass = yield_lanes_if_admitted(&s2, crate::test_helpers::TEST_NOW, "daemon", &full);
        assert!(pass.asked.is_empty(), "{pass:?}");
        assert!(!hold_requested(&s2, &only));
        assert_eq!(pass.refused.len(), 1, "{pass:?}");
        assert!(
            pass.refused[0].contains(&task2)
                && pass.refused[0].contains("machine other does not serve proj"),
            "{pass:?}"
        );
    }

    #[test]
    fn the_owners_queued_work_holds_the_youngest_fleet_run_in_its_lane_once() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        one_lane(&s);
        for name in ["aurora", "borealis", "cinder"] {
            join(&s, name);
        }
        let fleet = queue(&s, "improve", "suggestion:abc");
        assert_eq!(lease(&s, "aurora", Vec::new()).str("id"), fleet);
        let run = running(&s, &fleet, "aurora", "2026-09-01T01:00:00Z");
        // Nothing of the owner's waits: the fleet's run goes on.
        assert!(yield_lanes_to_owner(&s, crate::test_helpers::TEST_NOW, "daemon", &[]).is_empty());
        assert!(!hold_requested(&s, &fleet));

        let task = queue(&s, "task", "task:TASK-001");
        // The lane is held by the fleet's run: the daemon's pass asks the
        // run to hold, once, with the event on its job...
        let lines = yield_lanes_to_owner(&s, crate::test_helpers::TEST_NOW, "daemon", &[]);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].contains(&fleet) && lines[0].contains(&task),
            "{lines:?}"
        );
        assert!(hold_requested(&s, &fleet));
        assert!(yielded(&s, &fleet, &run));
        assert!(!yielded(&s, &fleet, "run-other"));
        // ...and the owner's task does not wait for the hold to be honoured:
        // it takes a lane beyond the narrowed count at once (2026-11-04: the
        // owner's tasks on the brain serialised 10-13 h behind one lane).
        assert_eq!(lease(&s, "borealis", Vec::new()).str("id"), task);
        // Leased, the task is waiting no longer: nothing more is asked for
        // it, and the run already asked is not asked again.
        assert!(
            yield_lanes_to_owner(&s, crate::test_helpers::TEST_NOW, "daemon", &[]).is_empty(),
            "a run already asked is not asked again"
        );

        // The run held: its job is queued again, and the lane it freed is
        // the owner's - the fleet job waits on it, for the agent that held
        // it, until the owner's run is done.
        s.finish_harness_session(
            &format!("h-{fleet}"),
            hive_store::HARNESS_HELD,
            hive_store::HARNESS_HELD,
            "",
            "held",
            0.0,
            "/nowhere/traj.json",
        )
        .unwrap();
        let conn = s.conn().unwrap();
        conn.execute(
            "UPDATE runs SET status = 'held', finished_at = started_at WHERE id = ?",
            [&run],
        )
        .unwrap();
        conn.execute(
            "UPDATE jobs SET status = 'queued', lease_agent = '', lease_until = '' WHERE id = ?",
            [&fleet],
        )
        .unwrap();
        drop(conn);
        assert!(yield_lanes_to_owner(&s, crate::test_helpers::TEST_NOW, "daemon", &[]).is_empty());
        assert!(lease(&s, "aurora", Vec::new()).0.is_empty());
        assert!(lease(&s, "cinder", Vec::new()).0.is_empty());
        s.conn()
            .unwrap()
            .execute(
                "UPDATE jobs SET status = 'done', lease_agent = '', lease_until = '' WHERE id = ?",
                [&task],
            )
            .unwrap();
        assert_eq!(lease(&s, "aurora", Vec::new()).str("id"), fleet);
    }

    #[test]
    fn fleet_work_waits_while_the_owners_work_is_queued_in_its_project() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        crate::capacity::pin_per_project(&s, 4);
        for name in ["aurora", "borealis", "cinder"] {
            join(&s, name);
        }
        let fleet = queue(&s, "improve", "suggestion:abc");
        let research = queue(&s, "reflect", "research:proj:1");
        let task = queue(&s, "task", "task:TASK-001");
        // The owner's task needs a machine with a word this one lacks; a
        // restart window alone would not hold it back (the owner's work
        // fits every window since 2026-11-04).
        s.conn()
            .unwrap()
            .execute("UPDATE jobs SET needs = 'gpu' WHERE id = ?", [&task])
            .unwrap();
        let plain = std::collections::BTreeSet::new();
        let lease_on = |agent: &str,
                        fits: Vec<(String, String)>,
                        caps: &std::collections::BTreeSet<String>| {
            crate::jobs::lease_job(
                &s,
                agent.into(),
                crate::test_helpers::TEST_NOW.into(),
                crate::jobs::LeaseOpts {
                    any_proj: true,
                    fits,
                    caps: Some(caps.clone()),
                    ..Default::default()
                },
            )
            .unwrap()
        };
        let improve_only = vec![("proj".to_string(), "improve".to_string())];
        // A slot that fits improve jobs alone, on a machine that cannot take
        // the task, finds the fleet's job - but the owner's task is queued in
        // the project: the fleet's work waits.
        assert!(
            lease_on("aurora", improve_only.clone(), &plain)
                .0
                .is_empty()
        );
        // Research is not work and passes.
        assert_eq!(
            lease_on(
                "aurora",
                vec![("proj".to_string(), "reflect".to_string())],
                &plain
            )
            .str("id"),
            research
        );
        // Once the owner's task is in flight, on the machine that offers
        // the word, the fleet's job leases.
        let gpu = std::collections::BTreeSet::from(["gpu".to_string()]);
        assert_eq!(lease_on("borealis", Vec::new(), &gpu).str("id"), task);
        assert_eq!(lease_on("cinder", improve_only, &plain).str("id"), fleet);
    }

    #[test]
    fn no_hold_while_a_lane_is_open_or_the_lanes_hold_the_owners_own_work() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        crate::capacity::pin_per_project(&s, 2);
        // Two slots in the fleet: a hold is asked only once both are taken.
        s.set_setting(hive_store::HUMAN_AGENT, "capacity", "2")
            .unwrap();
        for name in ["aurora", "borealis", "cinder"] {
            join(&s, name);
        }
        let fleet = queue(&s, "improve", "suggestion:abc");
        assert_eq!(lease(&s, "aurora", Vec::new()).str("id"), fleet);
        running(&s, &fleet, "aurora", "2026-09-01T01:00:00Z");
        let task = queue(&s, "task", "task:TASK-001");
        // Two lanes, one held: the owner's task has a lane and leases itself.
        assert!(yield_lanes_to_owner(&s, crate::test_helpers::TEST_NOW, "daemon", &[]).is_empty());
        assert!(!hold_requested(&s, &fleet));
        assert_eq!(lease(&s, "borealis", Vec::new()).str("id"), task);
        running(&s, &task, "borealis", "2026-09-01T01:05:00Z");
        // Both slots held, one by the owner's own work: a second task asks
        // the fleet's run to hold - never the owner's, though it is younger.
        let second = queue(&s, "task", "task:TASK-002");
        let lines = yield_lanes_to_owner(&s, crate::test_helpers::TEST_NOW, "daemon", &[]);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].contains(&fleet) && lines[0].contains(&second),
            "{lines:?}"
        );
        assert!(hold_requested(&s, &fleet));
        assert!(
            !hold_requested(&s, &task),
            "the owner's own run is never held"
        );
    }

    #[test]
    fn a_full_project_yields_a_fleet_research_run_when_its_work_lanes_are_open() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        crate::capacity::pin_per_project(&s, 2);
        s.set_setting(hive_store::HUMAN_AGENT, "capacity", "2")
            .unwrap();
        for name in ["aurora", "borealis", "cinder"] {
            join(&s, name);
        }
        // Two slots, both held by the fleet: one improve, one research. The
        // owner's task has a work lane but no slot: the daemon leases nothing
        // while every slot is taken (lease_job itself no longer bounds the
        // owner's work by the share, TASK-136).
        let fleet = queue(&s, "improve", "suggestion:abc");
        let research = queue(&s, "reflect", "research:proj:1");
        assert_eq!(lease(&s, "aurora", Vec::new()).str("id"), fleet);
        running(&s, &fleet, "aurora", "2026-09-01T01:00:00Z");
        assert_eq!(lease(&s, "borealis", Vec::new()).str("id"), research);
        running(&s, &research, "borealis", "2026-09-01T01:10:00Z");
        let task = queue(&s, "task", "task:TASK-001");
        let lines = yield_lanes_to_owner(&s, crate::test_helpers::TEST_NOW, "daemon", &[]);
        assert_eq!(lines.len(), 1, "{lines:?}");
        // The youngest fleet run in the way is the research run.
        assert!(
            lines[0].contains(&research) && lines[0].contains(&task),
            "{lines:?}"
        );
        assert!(hold_requested(&s, &research));
        assert!(!hold_requested(&s, &fleet));
    }

    #[test]
    fn a_run_that_yielded_to_the_owner_waits_until_the_owners_job_has_started() {
        // 2026-11-03: a role lead's sitting (kind lead, not a work kind)
        // held 58 times in a row for one owner task - each hold freed the
        // slot, the lead job leased it straight back, and the task never
        // started. Whatever yielded waits while the job it yielded to is
        // still queued.
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        crate::capacity::pin_per_project(&s, 1);
        for name in ["aurora", "borealis"] {
            join(&s, name);
        }
        let role = hive_tasks::leads::add(
            &s,
            hive_tasks::leads::LeadInput {
                project: "proj".into(),
                role: "rsi".into(),
                author: hive_store::HUMAN_AGENT.into(),
                ..Default::default()
            },
        )
        .unwrap();
        crate::lead_roles::ensure_role_lead(&s, &role).unwrap();
        let sitting = queue(&s, "lead", &role.source("20261103T085326Z"));
        let lease_as = |agent: &str| {
            crate::jobs::lease_job(
                &s,
                agent.into(),
                crate::test_helpers::TEST_NOW.into(),
                crate::jobs::LeaseOpts {
                    any_proj: true,
                    ..Default::default()
                },
            )
            .unwrap()
        };
        assert_eq!(lease_as(&role.agent).str("id"), sitting);
        let run = running(&s, &sitting, &role.agent, "2026-11-03T12:27:00Z");
        let task = queue(&s, "task", "task:TASK-055");
        // The one slot (`capacity` 1) is the sitting's: the task waits for
        // it, the sitting is asked to hold.
        let lines = yield_lanes_to_owner(&s, crate::test_helpers::TEST_NOW, "daemon", &[]);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(hold_requested(&s, &sitting));
        assert!(yielded(&s, &sitting, &run));
        honour_hold(&s, &sitting);
        // The slot is free. Before: the lead leased its sitting back at
        // once and was asked to hold again (58 times). Now it waits while
        // the task it yielded to is queued...
        assert!(
            lease_as(&role.agent).0.is_empty(),
            "the sitting that yielded leases again before the owner's task started"
        );
        // ...and the owner's task takes the lane.
        assert_eq!(lease_as("aurora").str("id"), task);
        // Once the owner's job is out of the queue (here done; in flight
        // would do with a lane open) the sitting carries on.
        s.conn()
            .unwrap()
            .execute(
                "UPDATE jobs SET status = 'done', lease_agent = '', lease_until = '' WHERE id = ?",
                [&task],
            )
            .unwrap();
        assert_eq!(lease_as(&role.agent).str("id"), sitting);
    }

    #[test]
    fn a_fleet_run_in_the_way_is_asked_to_hold_whatever_the_restart_window_names() {
        // Before 2026-11-04 no run was asked to hold for an owner's job the
        // restart window had no room for. The window never had room for a
        // 2.5-hour task on the brain, so the owner's task queued 13 h with
        // no attempt; now the owner's work fits every window, and the lane
        // is asked for whatever the window names.
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        one_lane(&s);
        for name in ["aurora", "borealis"] {
            join(&s, name);
        }
        let fleet = queue(&s, "improve", "suggestion:abc");
        assert_eq!(lease(&s, "aurora", Vec::new()).str("id"), fleet);
        running(&s, &fleet, "aurora", "2026-09-01T01:00:00Z");
        let task = queue(&s, "task", "task:TASK-001");
        // The restart window names the project's short runs only: the
        // fleet's run in the lane is asked to hold all the same...
        let short = vec![("proj".to_string(), "reflect".to_string())];
        let lines = yield_lanes_to_owner(&s, crate::test_helpers::TEST_NOW, "daemon", &short);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].contains(&fleet) && lines[0].contains(&task),
            "{lines:?}"
        );
        assert!(hold_requested(&s, &fleet));
        // ...and the owner's task leases through that window.
        assert_eq!(lease(&s, "borealis", short).str("id"), task);
    }

    /// What the run leaves when its hold is honoured: the session and run
    /// `held`, the job queued again.
    fn honour_hold(s: &hive_store::Store, job: &str) {
        s.finish_harness_session(
            &format!("h-{job}"),
            hive_store::HARNESS_HELD,
            hive_store::HARNESS_HELD,
            "",
            "held",
            0.0,
            "/nowhere/traj.json",
        )
        .unwrap();
        let conn = s.conn().unwrap();
        conn.execute(
            "UPDATE runs SET status = 'held', finished_at = started_at WHERE job_id = ?",
            [job],
        )
        .unwrap();
        conn.execute(
            "UPDATE jobs SET status = 'queued', lease_agent = '', lease_until = '' WHERE id = ?",
            [job],
        )
        .unwrap();
    }

    #[test]
    fn the_holder_stays_free_for_its_own_work_and_another_worker_takes_the_owners() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        crate::capacity::pin_per_project(&s, 2);
        s.set_setting(hive_store::HUMAN_AGENT, "quorum", "2")
            .unwrap();
        for name in ["aurora", "borealis", "cinder"] {
            join(&s, name);
        }
        let f1 = queue(&s, "improve", "suggestion:one");
        let f2 = queue(&s, "improve", "suggestion:two");
        assert_eq!(lease(&s, "aurora", Vec::new()).str("id"), f1);
        running(&s, &f1, "aurora", "2026-09-01T01:00:00Z");
        assert_eq!(lease(&s, "borealis", Vec::new()).str("id"), f2);
        running(&s, &f2, "borealis", "2026-09-01T01:10:00Z");
        let task = queue(&s, "task", "task:TASK-001");
        let lines = yield_lanes_to_owner(&s, crate::test_helpers::TEST_NOW, "daemon", &[]);
        assert!(lines.len() == 1 && lines[0].contains(&f2), "{lines:?}");
        honour_hold(&s, &f2);
        // The daemon picks the next worker: not borealis, whose held work
        // waits behind the owner's task, but cinder, who takes the task.
        let picked = crate::run::pick_worker(&s, crate::test_helpers::TEST_NOW, "", &[]).unwrap();
        assert_eq!(picked, "cinder");
        assert_eq!(lease(&s, "cinder", Vec::new()).str("id"), task);
        // Nobody else takes the held job while borealis is free; when f1
        // ends, borealis is picked and carries its own work on.
        assert!(lease(&s, "aurora", Vec::new()).0.is_empty());
        let conn = s.conn().unwrap();
        conn.execute(
            "UPDATE jobs SET status = 'done', lease_agent = '', lease_until = '' WHERE id = ?",
            [&f1],
        )
        .unwrap();
        conn.execute("UPDATE runs SET status = 'done' WHERE job_id = ?", [&f1])
            .unwrap();
        drop(conn);
        let picked = crate::run::pick_worker(&s, crate::test_helpers::TEST_NOW, "", &[]).unwrap();
        assert_eq!(picked, "borealis");
        assert_eq!(lease(&s, "borealis", Vec::new()).str("id"), f2);
    }
}
