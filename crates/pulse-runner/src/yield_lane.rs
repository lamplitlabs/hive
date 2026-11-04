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
//! whatever its kind, until that job has started or left the queue. And no
//! run is asked to hold for an owner's job the runner's restart window has
//! no room for (`fits`): the hold would cost the run its step for nothing.

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
        "NOT (NOT {owner} AND ((j.kind IN ({work}) AND j.source NOT LIKE '{red}%' AND EXISTS (SELECT 1 FROM jobs q JOIN projects qp ON qp.slug = q.project WHERE q.project = j.project AND q.status = 'queued' AND qp.status = 'active' AND q.kind IN ({work}) AND {owner_q})) OR EXISTS (SELECT 1 FROM events y JOIN jobs t ON t.id = json_extract(y.detail, '$.to') WHERE y.action = '{yield}' AND y.subject = j.id AND t.status = 'queued' AND t.project = j.project)))",
        owner = crate::jobs::OWNER_DRIVEN_SQL,
        red = crate::redbase::RED_BASE_SOURCE_PREFIX,
        owner_q = crate::jobs::owner_driven_sql("q"),
        yield = YIELD_ACTION,
    )
}

/// One pass: for every project with an owner-driven work job queued and
/// no lane open to it - its work lanes full, or its per-project share
/// slots - asks the youngest fleet run in the way to hold. Returns one log
/// line per run asked; a run already asked, or past its harness, is not
/// asked twice and makes no line. `fits` is the daemon's restart window:
/// the project and kind pairs a run may still start as (empty: any); an
/// owner's job the window has no room for gets no lane held for it.
pub(crate) fn yield_lanes_to_owner(
    s: &pulse_store::Store,
    now: &str,
    actor: &str,
    fits: &[(String, String)],
) -> Vec<String> {
    let Ok(conn) = s.conn() else {
        return Vec::new();
    };
    let work = crate::jobs::work_kinds_sql();
    let waiting = must_rows(
        &conn,
        &format!(
            "SELECT j.id, j.project, j.kind FROM jobs j JOIN projects p ON p.slug = j.project WHERE j.status = 'queued' AND p.status = 'active' AND j.kind IN ({work}) AND {} ORDER BY j.created_at, j.id",
            crate::jobs::OWNER_DRIVEN_SQL
        ),
        &[],
    );
    if waiting.is_empty() {
        return Vec::new();
    }
    let red = crate::redbase::holds(s, now);
    let limit = crate::jobs::parallel_per_project(s).max(1);
    let mut lines = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for owner in waiting {
        let slug = owner.str("project");
        if !seen.insert(slug.clone()) || red.contains_key(&slug) {
            continue;
        }
        if !fits.is_empty()
            && !fits
                .iter()
                .any(|(project, kind)| *project == slug && *kind == owner.str("kind"))
        {
            continue;
        }
        let lanes = crate::jobs::project_lanes(s, &slug).lanes.max(1);
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
        let in_the_way: Vec<Row> = if work_held >= lanes {
            occupied.into_iter().filter(|k| is_work(k)).collect()
        } else if occupied.len() as i32 >= limit {
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
                    "SELECT id, started_at FROM runs r WHERE r.job_id = ? AND r.status = 'running' ORDER BY r.started_at DESC, r.id DESC LIMIT 1",
                    &[&k.str("id")],
                );
                (!run.0.is_empty()).then(|| (run.str("started_at"), run.str("id"), k))
            })
            .collect::<Vec<_>>();
        fleet.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
        let Some((_, run_id, job)) = fleet.into_iter().next() else {
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
        lines.push(format!(
            "{slug}: {} ({}) holds after its current step so the owner's {} can start; it carries on later",
            job.str("id"),
            job.str("kind"),
            owner.str("id")
        ));
    }
    lines
}

/// Whether run `run_id` of `job_id` was asked to hold for the owner's work
/// (as against the runner's restart): the run's summary says which.
pub(crate) fn yielded(s: &pulse_store::Store, job_id: &str, run_id: &str) -> bool {
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

    fn join(s: &pulse_store::Store, name: &str) {
        crate::test_helpers::join(s, name);
    }

    fn queue(s: &pulse_store::Store, kind: &str, source: &str) -> String {
        let (job, _) = crate::jobs::add_job(
            s,
            "proj".into(),
            kind.into(),
            source.into(),
            "brief".into(),
            1,
            10,
            source.into(),
            pulse_store::HUMAN_AGENT.into(),
            false,
        )
        .unwrap();
        job.str("id")
    }

    fn lease(s: &pulse_store::Store, agent: &str, fits: Vec<(String, String)>) -> Row {
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
    fn running(s: &pulse_store::Store, job: &str, agent: &str, started_at: &str) -> String {
        let run_id = format!("run-{job}");
        s.conn()
            .unwrap()
            .execute(
                "INSERT INTO runs (id, job_id, agent, project, started_at, status) VALUES (?, ?, ?, 'proj', ?, 'running')",
                rusqlite::params![run_id, job, agent, started_at],
            )
            .unwrap();
        s.begin_harness_session(pulse_store::HarnessSession {
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

    fn hold_requested(s: &pulse_store::Store, job: &str) -> bool {
        !s.harness_session_head(&format!("h-{job}"))
            .unwrap()
            .unwrap()
            .hold_requested_at
            .is_empty()
    }

    /// One work lane: the project's checks take turns and one 10-minute
    /// check fits the commit cap.
    fn one_lane(s: &pulse_store::Store) {
        crate::capacity::pin_per_project(s, 4);
        s.set_setting(
            pulse_store::HUMAN_AGENT,
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
            "pulse/x/1",
            json!({"project": "proj", "sha": "abc", "verify": "verify exit 0", "verify_seconds": 600}),
        );
        s.set_setting(pulse_store::HUMAN_AGENT, "harness.commit_timeout", "900")
            .unwrap();
        s.set_setting(pulse_store::HUMAN_AGENT, "harness.step_timeout", "900")
            .unwrap();
        assert_eq!(crate::jobs::project_lanes(s, "proj").lanes, 1);
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
        // The lane is held by the fleet's run: the owner's task cannot lease...
        assert!(lease(&s, "borealis", Vec::new()).0.is_empty());
        // ...so the run is asked to hold, once, with the event on its job.
        let lines = yield_lanes_to_owner(&s, crate::test_helpers::TEST_NOW, "daemon", &[]);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].contains(&fleet) && lines[0].contains(&task),
            "{lines:?}"
        );
        assert!(hold_requested(&s, &fleet));
        assert!(yielded(&s, &fleet, &run));
        assert!(!yielded(&s, &fleet, "run-other"));
        assert!(
            yield_lanes_to_owner(&s, crate::test_helpers::TEST_NOW, "daemon", &[]).is_empty(),
            "a run already asked is not asked again"
        );

        // The run held: its job is queued again, the lane is free, and the
        // agent that held it takes the owner's task first - the fleet job
        // waits while the owner's work is queued in its project.
        s.finish_harness_session(
            &format!("h-{fleet}"),
            pulse_store::HARNESS_HELD,
            pulse_store::HARNESS_HELD,
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
        assert_eq!(lease(&s, "aurora", Vec::new()).str("id"), task);
        // Now the owner's work holds the lane; nothing yields to it, and the
        // fleet's job waits on the lane as before.
        assert!(yield_lanes_to_owner(&s, crate::test_helpers::TEST_NOW, "daemon", &[]).is_empty());
        assert!(lease(&s, "cinder", Vec::new()).0.is_empty());
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
        let improve_only = vec![("proj".to_string(), "improve".to_string())];
        // A slot that fits improve jobs alone finds the fleet's job, but the
        // owner's task is queued in the project: the fleet's work waits.
        assert!(lease(&s, "aurora", improve_only.clone()).0.is_empty());
        // Research is not work and passes.
        assert_eq!(
            lease(
                &s,
                "aurora",
                vec![("proj".to_string(), "reflect".to_string())]
            )
            .str("id"),
            research
        );
        // Once the owner's task is in flight, the fleet's job leases.
        assert_eq!(lease(&s, "borealis", Vec::new()).str("id"), task);
        assert_eq!(lease(&s, "cinder", improve_only).str("id"), fleet);
    }

    #[test]
    fn no_hold_while_a_lane_is_open_or_the_lanes_hold_the_owners_own_work() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        crate::capacity::pin_per_project(&s, 2);
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
        // Both lanes held, one by the owner's own work: a second task asks
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
        for name in ["aurora", "borealis", "cinder"] {
            join(&s, name);
        }
        // Two slots, both held by the fleet: one improve, one research. The
        // owner's task has a work lane but no slot.
        let fleet = queue(&s, "improve", "suggestion:abc");
        let research = queue(&s, "reflect", "research:proj:1");
        assert_eq!(lease(&s, "aurora", Vec::new()).str("id"), fleet);
        running(&s, &fleet, "aurora", "2026-09-01T01:00:00Z");
        assert_eq!(lease(&s, "borealis", Vec::new()).str("id"), research);
        running(&s, &research, "borealis", "2026-09-01T01:10:00Z");
        let task = queue(&s, "task", "task:TASK-001");
        assert!(lease(&s, "cinder", Vec::new()).0.is_empty());
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
    fn a_red_default_branch_keeps_its_fix_running_and_leasing() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        one_lane(&s);
        for name in ["aurora", "borealis"] {
            join(&s, name);
        }
        let conn = s.conn().unwrap();
        conn.execute(
            "UPDATE projects SET verify_cmd = 'make test', default_branch = 'main' WHERE slug = 'proj'",
            [],
        )
        .unwrap();
        let proj = crate::board::project_by_slug(&conn, "proj".into());
        let head = crate::util::git(
            &proj.str("path"),
            &["rev-parse", "--verify", "--quiet", "refs/heads/main"],
        )
        .stdout
        .trim()
        .to_string();
        drop(conn);
        let fix = crate::redbase::mark_red_base(
            &s,
            &proj,
            &head,
            2,
            "FAIL tests/x.rs",
            crate::test_helpers::TEST_NOW,
        )
        .expect("fix job")
        .str("id");
        assert_eq!(lease(&s, "aurora", Vec::new()).str("id"), fix);
        running(&s, &fix, "aurora", "2026-09-01T01:00:00Z");
        let _task = queue(&s, "task", "task:TASK-001");
        // The owner's task waits behind the fix: nothing yields to it...
        assert!(yield_lanes_to_owner(&s, crate::test_helpers::TEST_NOW, "daemon", &[]).is_empty());
        assert!(!hold_requested(&s, &fix));
        // ...and the fix job itself leases past the owner's queued work.
        let conn = s.conn().unwrap();
        conn.execute(
            "UPDATE jobs SET status = 'queued', lease_agent = '', lease_until = '' WHERE id = ?",
            [&fix],
        )
        .unwrap();
        conn.execute("UPDATE runs SET status = 'failed' WHERE job_id = ?", [&fix])
            .unwrap();
        drop(conn);
        let hold_except = vec![("proj".to_string(), crate::redbase::fix_source("proj"))];
        let got = crate::jobs::lease_job(
            &s,
            "borealis".into(),
            crate::test_helpers::TEST_NOW.into(),
            crate::jobs::LeaseOpts {
                any_proj: true,
                hold_except,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(got.str("id"), fix, "{got:?}");
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
        let role = pulse_tasks::leads::add(
            &s,
            pulse_tasks::leads::LeadInput {
                project: "proj".into(),
                role: "rsi".into(),
                author: pulse_store::HUMAN_AGENT.into(),
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
        // The one slot is the sitting's: the task waits, the sitting is
        // asked to hold.
        assert!(lease_as("aurora").0.is_empty());
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
    fn no_hold_for_an_owners_job_the_restart_window_cannot_start() {
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
        let _task = queue(&s, "task", "task:TASK-001");
        // The restart window has room for the project's short runs only:
        // the task could not start in the freed lane, so nothing is held.
        let short = vec![("proj".to_string(), "reflect".to_string())];
        assert!(
            yield_lanes_to_owner(&s, crate::test_helpers::TEST_NOW, "daemon", &short).is_empty()
        );
        assert!(!hold_requested(&s, &fleet));
        // With room for the task, the lane is asked for.
        let fits = vec![("proj".to_string(), "task".to_string())];
        let lines = yield_lanes_to_owner(&s, crate::test_helpers::TEST_NOW, "daemon", &fits);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(hold_requested(&s, &fleet));
    }

    /// What the run leaves when its hold is honoured: the session and run
    /// `held`, the job queued again.
    fn honour_hold(s: &pulse_store::Store, job: &str) {
        s.finish_harness_session(
            &format!("h-{job}"),
            pulse_store::HARNESS_HELD,
            pulse_store::HARNESS_HELD,
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
        s.set_setting(pulse_store::HUMAN_AGENT, "quorum", "2")
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
