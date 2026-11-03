use std::collections::{BTreeMap, HashMap};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::util::Row;

/// How long workers the shutdown cancelled get to stop on their own before
/// the commands they wait in are ended; a daemon restart that waited on a
/// landing's git for three hours had no such bound.
pub(crate) const CANCEL_GRACE: Duration = Duration::from_secs(30);
/// How long a worker gets to return once its commands were ended, at most;
/// what still runs after it is left behind.
pub(crate) const DETACH_SETTLE: Duration = Duration::from_secs(5);

#[derive(Debug, Default)]
pub(crate) struct LandingWorkers {
    workers: HashMap<String, Worker>,
    /// Overrides `CANCEL_GRACE` (the daemon gives a short forced drain a
    /// short grace; tests use milliseconds).
    pub(crate) cancel_grace: Option<Duration>,
}

/// One project's landing thread, with the stage it is at and when it began.
#[derive(Debug)]
struct Worker {
    handle: JoinHandle<LandingWorkerResult>,
    stage: crate::daemon_maintenance::Stage,
    started: Instant,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct LandingWorkerResult {
    pub(crate) slug: String,
    pub(crate) lines: Vec<String>,
    pub(crate) hold: Option<String>,
    pub(crate) seconds: f64,
    pub(crate) start_seq: i32,
    /// The worker did not return by the end of the shutdown and was left
    /// behind: its hold and timing are unknown, so they stay as they were.
    pub(crate) detached: bool,
}

impl LandingWorkers {
    pub(crate) fn busy(&self) -> usize {
        self.workers.len()
    }

    /// The grace cancelled workers get before their commands are ended.
    pub(crate) fn cancel_grace(&self) -> Duration {
        self.cancel_grace.unwrap_or(CANCEL_GRACE)
    }

    pub(crate) fn drain(&mut self) -> Vec<LandingWorkerResult> {
        let done = self
            .workers
            .iter()
            .filter_map(|(slug, w)| w.handle.is_finished().then_some(slug.clone()))
            .collect::<Vec<_>>();
        let mut out = Vec::new();
        for slug in done {
            let Some(w) = self.workers.remove(&slug) else {
                continue;
            };
            out.push(joined(&slug, w.handle));
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn start_projects(
        &mut self,
        cancel: &pulse_bus::CancellationToken,
        s: &pulse_store::Store,
        projects: &[Row],
        actor: &str,
        now: &str,
        held: &BTreeMap<String, String>,
    ) -> Vec<String> {
        let limit = landing_parallel(s);
        let mut lines = Vec::new();
        if limit <= 1 || s.setting("landing", "") != "agents" {
            return lines;
        }
        // Projects with landings to do go first; one that only owes a push,
        // carries a hold note to re-check (so a cleared hold is seen to
        // clear), or is due a fetch of its upstream (so a merge made on the
        // forge reaches the checkout; `landing_hold::refresh_upstream`)
        // takes a slot that is left and never starves a landing.
        let push_on = crate::landing_hold::push_on(s);
        let mut with_landings = Vec::new();
        let mut to_recheck = Vec::new();
        for project in projects {
            let slug = project.str("slug");
            if self.workers.contains_key(&slug) {
                continue;
            }
            let ready = ready_landings(s, &slug, now);
            if ready > 0 {
                with_landings.push((project, format!("{ready} ready")));
            } else if (push_on && crate::landing_hold::push_owed(s, slug.clone()))
                || pulse_tasks::pull_request_asked(s, &slug)
            {
                to_recheck.push((project, "push owed".to_string()));
            } else if held.contains_key(&slug) {
                to_recheck.push((project, "hold to re-check".to_string()));
            } else if push_on && crate::landing_hold::refresh_due(s, &slug, now) {
                to_recheck.push((project, "upstream refresh".to_string()));
            }
        }
        for (project, why) in with_landings.into_iter().chain(to_recheck) {
            if self.workers.len() >= limit {
                break;
            }
            let slug = project.str("slug");
            let ctx = cancel.child();
            let store = s.clone();
            let project = project.clone();
            let actor = actor.to_string();
            let now = now.to_string();
            let prev = held.get(&slug).cloned();
            lines.push(format!("landing {slug}: started ({why})"));
            self.spawn(&slug, move |stage| {
                worker(ctx, store, project, actor, now, prev, stage)
            });
        }
        lines
    }

    /// Runs `work` as the landing worker of `slug`, on a thread of its own;
    /// `work` notes where it is in the stage it is handed.
    pub(crate) fn spawn(
        &mut self,
        slug: &str,
        work: impl FnOnce(crate::daemon_maintenance::Stage) -> LandingWorkerResult + Send + 'static,
    ) {
        let stage: crate::daemon_maintenance::Stage =
            std::sync::Arc::new(std::sync::Mutex::new(format!("starting {slug}")));
        let worker_stage = stage.clone();
        let handle = std::thread::Builder::new()
            .name(format!("landing-{slug}"))
            .spawn(move || work(worker_stage))
            .expect("spawn landing worker");
        self.workers.insert(
            slug.to_string(),
            Worker {
                handle,
                stage,
                started: Instant::now(),
            },
        );
    }

    /// Waits for the workers, and never for good. Without a deadline and
    /// with `ctx` live they finish on their own; past `deadline` `ctx` is
    /// cancelled (the flag says so). Once `ctx` is cancelled - here or by
    /// the caller - the workers get `cancel_grace` to stop, then the
    /// commands they still wait in are ended (proc::end_groups_of), and a
    /// worker that has not returned soon after is left behind with a line
    /// naming its project and stage: a git that ignores its kill, or a call
    /// no budget reaches, cannot hold a daemon restart (it did for 3h17m).
    #[cfg(test)]
    pub(crate) fn finish(
        &mut self,
        ctx: &pulse_bus::CancellationToken,
        deadline: Option<Instant>,
    ) -> (Vec<LandingWorkerResult>, bool) {
        let mut settle = Settle::new(self.cancel_grace());
        self.finish_settling(ctx, deadline, &mut settle, &|| {})
    }

    /// [`finish`](Self::finish) with the grace shared with the rest of a
    /// shutdown (the maintenance pass, the experiment lane); `beat` is
    /// called about once a second while the workers run uncancelled, so the
    /// daemon's watchdog does not take a wait it allows for a wedge.
    pub(crate) fn finish_settling(
        &mut self,
        ctx: &pulse_bus::CancellationToken,
        deadline: Option<Instant>,
        settle: &mut Settle,
        beat: &dyn Fn(),
    ) -> (Vec<LandingWorkerResult>, bool) {
        let mut cut = false;
        let workers = std::mem::take(&mut self.workers);
        let all_done = |ws: &HashMap<String, Worker>| ws.values().all(|w| w.handle.is_finished());
        let mut beat_at = Instant::now();
        while !all_done(&workers) && !ctx.is_cancelled() {
            if let Some(deadline) = deadline
                && Instant::now() >= deadline
            {
                ctx.cancel();
                cut = true;
                break;
            }
            if beat_at.elapsed() >= Duration::from_secs(1) {
                beat();
                beat_at = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut ended: HashMap<String, Vec<String>> = HashMap::new();
        if !all_done(&workers) {
            wait_until(|| all_done(&workers), settle.until());
            for (slug, w) in workers.iter().filter(|(_, w)| !w.handle.is_finished()) {
                ended.insert(slug.clone(), settle.end_groups_of(w.handle.thread()));
            }
            if ended.values().any(|killed| !killed.is_empty()) {
                wait_until(|| all_done(&workers), settle.after_kill());
            }
        }
        let mut out = Vec::new();
        for (slug, w) in workers {
            if w.handle.is_finished() {
                let mut result = joined(&slug, w.handle);
                if let Some(killed) = ended.remove(&slug).filter(|k| !k.is_empty()) {
                    result.lines.insert(
                        0,
                        format!(
                            "landing {slug}: ended {} after its cancellation so the daemon can go",
                            killed.join(", ")
                        ),
                    );
                }
                out.push(result);
                continue;
            }
            cut = true;
            let stage = w.stage.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let killed = ended.remove(&slug).unwrap_or_default();
            out.push(LandingWorkerResult {
                lines: vec![format!(
                    "{} - the landing runs again on the next daemon",
                    left_behind_line(
                        &format!("landing {slug}: worker"),
                        &stage,
                        w.started.elapsed(),
                        &killed
                    )
                )],
                slug,
                detached: true,
                ..Default::default()
            });
        }
        (out, cut)
    }
}

/// Waits until `until` for `done`.
pub(crate) fn wait_until(done: impl Fn() -> bool, until: Instant) {
    while !done() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The grace a shutdown gives what it cancelled, shared by every part of
/// it: counted once from when the cancellation was first seen, so a pass, a
/// worker and the lane stuck together cost one grace, not three.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Settle {
    grace: Duration,
    since: Option<Instant>,
}

impl Settle {
    pub(crate) fn new(grace: Duration) -> Self {
        Settle { grace, since: None }
    }

    /// When the grace ends; the first call notes the cancellation.
    pub(crate) fn until(&mut self) -> Instant {
        *self.since.get_or_insert_with(Instant::now) + self.grace
    }

    /// How long a thread gets to return once its commands were ended.
    pub(crate) fn after_kill(&self) -> Instant {
        Instant::now() + self.grace.clamp(Duration::from_millis(500), DETACH_SETTLE)
    }

    /// Ends the commands `thread` waits in (see [`crate::proc::end_groups_of`]).
    pub(crate) fn end_groups_of(&self, thread: &std::thread::Thread) -> Vec<String> {
        crate::proc::end_groups_of(
            thread.id(),
            crate::proc::TIMEOUT_KILL_GRACE.min(self.grace.max(Duration::from_millis(100))),
        )
    }
}

/// The line for a thread a shutdown leaves behind: what, where it was, how
/// long it ran and which of its commands were ended.
pub(crate) fn left_behind_line(
    what: &str,
    stage: &str,
    ran: Duration,
    killed: &[String],
) -> String {
    let killed = if killed.is_empty() {
        String::new()
    } else {
        format!(" (ended: {})", killed.join(", "))
    };
    format!(
        "{what}: still at {stage} after {} and its cancellation{killed}; left behind so the daemon can go",
        pulse_cli::format_duration(Duration::from_secs(ran.as_secs()))
    )
}

/// A finished worker's result; a panic becomes an error line.
fn joined(slug: &str, handle: JoinHandle<LandingWorkerResult>) -> LandingWorkerResult {
    match handle.join() {
        Ok(result) => result,
        Err(panic) => LandingWorkerResult {
            slug: slug.to_string(),
            lines: vec![format!(
                "landing {slug}: worker error: {}",
                panic_text(&panic)
            )],
            ..Default::default()
        },
    }
}

pub(crate) fn landing_parallel(s: &pulse_store::Store) -> usize {
    s.setting("landing.parallel", "4")
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|n| *n > 0)
        .unwrap_or(4)
}

pub(crate) fn ready_landings(s: &pulse_store::Store, slug: &str, now: &str) -> usize {
    if crate::land::landing_retry_hold(s, slug, now).is_some() {
        return 0;
    }
    let Ok(conn) = s.conn() else {
        return 0;
    };
    crate::land::pending_landings(&conn, slug)
        .into_iter()
        .filter(|job| job.str("kind") != "experiment")
        .filter(|job| !crate::land::held_branch_waits(s, slug, &job.str("branch"), now))
        .count()
}

fn worker(
    ctx: pulse_bus::CancellationToken,
    s: pulse_store::Store,
    project: Row,
    actor: String,
    now: String,
    prev: Option<String>,
    stage: crate::daemon_maintenance::Stage,
) -> LandingWorkerResult {
    let slug = project.str("slug");
    let started = Instant::now();
    let start_seq = crate::plan::last_event_seq(&s);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::landing_hold::tend_landing_project(
            &ctx,
            &s,
            project,
            actor,
            now,
            false,
            prev,
            Some(&stage),
        )
    }));
    let seconds = started.elapsed().as_secs_f64();
    match result {
        Ok(outcome) => {
            let mut lines = outcome.lines;
            lines.push(format!(
                "landing {}: finished in {:.1}s",
                outcome.slug, seconds
            ));
            LandingWorkerResult {
                slug: outcome.slug,
                lines,
                hold: outcome.hold,
                seconds,
                start_seq,
                detached: false,
            }
        }
        Err(panic) => LandingWorkerResult {
            slug: slug.clone(),
            lines: vec![
                format!("landing {slug}: worker error: {}", panic_text(&panic)),
                format!("landing {slug}: finished in {:.1}s", seconds),
            ],
            hold: None,
            seconds,
            start_seq,
            detached: false,
        },
    }
}

pub(crate) fn add_project_seconds(
    s: &pulse_store::Store,
    _slug: &str,
    seconds: f64,
    _start_seq: i32,
) {
    let Ok(conn) = s.conn() else {
        return;
    };
    for _ in 0..20 {
        if conn
            .execute(
                "UPDATE events
                SET detail = json_insert(detail, '$.project_seconds', ?)
              WHERE action = 'land.landed'
                AND json_extract(detail, '$.project_seconds') IS NULL",
                rusqlite::params![seconds],
            )
            .is_ok()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    for _ in 0..20 {
        if conn
            .execute(
                "UPDATE events
                SET detail = json_insert(detail, '$.project_seconds', ?)
              WHERE action = 'land.pushed'
                AND json_extract(detail, '$.project_seconds') IS NULL",
                rusqlite::params![seconds],
            )
            .is_ok()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn panic_text(panic: &Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::*;
    use crate::test_helpers_daemon::*;
    use std::collections::BTreeMap;
    use std::path::Path;

    fn add_project(s: &pulse_store::Store, slug: &str, verify_cmd: &str) -> (Row, String) {
        let dir = Path::new(&s.home)
            .join("tmp")
            .join(format!("{slug}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.parent().unwrap()).expect("project parent");
        let target = dir.display().to_string();
        run(".", &["git", "clone", "-q", &template_repo(), &target]);
        run(&target, &["git", "remote", "remove", "origin"]);
        let root = std::fs::canonicalize(&dir)
            .unwrap_or(dir)
            .display()
            .to_string();
        let now = s.now();
        let conn = s.conn().expect("conn");
        conn.execute(
            "INSERT INTO projects (slug, name, path, remote, brain, verify_cmd, default_branch, status, added_at, added_by, last_seen)
             VALUES (?, ?, ?, '', 0, ?, 'main', 'active', ?, ?, ?)",
            rusqlite::params![slug, slug, root, verify_cmd, now, crate::util::HUMAN_AGENT, now],
        )
        .expect("insert project");
        pulse_goals::ensure_default(s, slug).expect("default goal");
        (crate::board::project_by_slug(&conn, slug.into()), root)
    }

    fn overlap_script(s: &pulse_store::Store) -> (String, String) {
        let dir = Path::new(&s.home).join("tmp").join("landing-workers");
        std::fs::create_dir_all(&dir).expect("script dir");
        let active = dir.join("active");
        let overlap = dir.join("overlap");
        let script = dir.join("verify.sh");
        std::fs::write(
            &script,
            format!(
                r#"#!/bin/sh
mkdir -p "{active}"
touch "{active}/$$"
if [ "$(find "{active}" -type f | wc -l | tr -d ' ')" -gt 1 ]; then
  echo yes > "{overlap}"
fi
sleep 1
rm -f "{active}/$$"
"#,
                active = active.display(),
                overlap = overlap.display()
            ),
        )
        .expect("write verify");
        run(".", &["chmod", "+x", &script.display().to_string()]);
        (script.display().to_string(), overlap.display().to_string())
    }

    fn set_landing_parallel(s: &pulse_store::Store, value: &str) {
        exec1(
            s,
            "INSERT INTO settings (name, value, updated_at, updated_by) VALUES ('landing.parallel', ?, ?, 'test')
             ON CONFLICT(name) DO UPDATE SET value = excluded.value",
            &[&value, &TEST_NOW],
        );
    }

    #[test]
    fn landing_parallel_defaults_to_four_and_one_disables_workers() {
        let s = new_store();
        assert_eq!(landing_parallel(&s), 4);
        set_landing_parallel(&s, "1");
        assert_eq!(landing_parallel(&s), 1);
        set_landing_parallel(&s, "2");
        assert_eq!(landing_parallel(&s), 2);
    }

    #[test]
    fn landing_workers_land_two_projects_concurrently() {
        let _serial = crate::test_helpers::serial();
        pulse_bus::reset_default();
        let _env = pulse_testutil::env_lock();
        let s = new_store();
        set_setting(&s, "landing", "agents");
        let (script, overlap) = overlap_script(&s);
        let (p1, r1) = add_project(&s, "proj-a", &script);
        let (p2, r2) = add_project(&s, "proj-b", &script);
        push_branch(&s, &p1, &r1, "pulse/a/1");
        push_branch(&s, &p2, &r2, "pulse/b/1");

        let cancel = pulse_bus::CancellationToken::new();
        let mut workers = LandingWorkers::default();
        let started = std::time::Instant::now();
        let lines =
            workers.start_projects(&cancel, &s, &[p1, p2], "daemon", TEST_NOW, &BTreeMap::new());
        assert_eq!(lines.len(), 2, "started lines = {lines:?}");
        let (results, cut) = workers.finish(&cancel, None);
        assert!(!cut, "finish cut workers");
        let elapsed = started.elapsed();
        for result in &results {
            add_project_seconds(&s, &result.slug, result.seconds, result.start_seq);
        }
        // Measured against the landings' own durations, not the wall clock,
        // so a loaded machine that slows every git step does not fail this.
        let serial: f64 = results.iter().map(|r| r.seconds).sum();
        assert!(
            elapsed.as_secs_f64() < serial * 0.8,
            "landings did not overlap enough: {elapsed:?} vs {serial:.3}s run one after the other, results = {results:?}"
        );
        assert!(
            Path::new(&overlap).exists(),
            "verify marker did not prove overlap; results = {results:?}"
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM events WHERE action = 'land.landed'",
                &[]
            ),
            2
        );
        assert!(
            results
                .iter()
                .flat_map(|r| r.lines.iter())
                .any(|line| line.contains("finished in")),
            "finish lines missing duration: {results:?}"
        );
    }

    #[test]
    fn a_hold_or_an_owed_push_gets_a_worker_without_pending_landings() {
        let _serial = crate::test_helpers::serial();
        pulse_bus::reset_default();
        let _env = pulse_testutil::env_lock();
        let s = new_store();
        set_setting(&s, "landing", "agents");
        set_setting(&s, "landing.push", "main");
        let (project, _repo) = add_project(&s, "proj-quiet", "");
        let cancel = pulse_bus::CancellationToken::new();
        let mut workers = LandingWorkers::default();
        // A quiet project is visited once every UPSTREAM_REFRESH_MINUTES so
        // its upstream is fetched; this one tracks nothing, so the visit is
        // silent and the next is in ten minutes.
        let lines = workers.start_projects(
            &cancel,
            &s,
            std::slice::from_ref(&project),
            "daemon",
            TEST_NOW,
            &BTreeMap::new(),
        );
        assert_eq!(
            lines,
            vec!["landing proj-quiet: started (upstream refresh)".to_string()]
        );
        let (results, cut) = workers.finish(&cancel, None);
        assert!(!cut, "finish cut workers: {results:?}");
        assert!(
            results[0].hold.is_none()
                && results[0]
                    .lines
                    .iter()
                    .all(|l| l.starts_with("landing proj-quiet: finished in")),
            "a refresh of a branch that tracks nothing said {results:?}"
        );
        assert!(
            workers
                .start_projects(
                    &cancel,
                    &s,
                    std::slice::from_ref(&project),
                    "daemon",
                    TEST_NOW,
                    &BTreeMap::new(),
                )
                .is_empty(),
            "a project with nothing to do got a worker"
        );
        assert!(crate::landing_hold::refresh_due(
            &s,
            "proj-quiet",
            &crate::util::plus_minutes(
                TEST_NOW,
                crate::landing_hold::UPSTREAM_REFRESH_MINUTES as f64
            )
        ));

        // A hold note left behind (the push went through by hand) is
        // re-checked and seen to clear.
        let mut held = BTreeMap::new();
        held.insert(
            "proj-quiet".to_string(),
            "11 commits landed but not pushed; push failed: rejected".to_string(),
        );
        let lines = workers.start_projects(
            &cancel,
            &s,
            std::slice::from_ref(&project),
            "daemon",
            TEST_NOW,
            &held,
        );
        assert_eq!(
            lines,
            vec!["landing proj-quiet: started (hold to re-check)".to_string()]
        );
        let (results, cut) = workers.finish(&cancel, None);
        assert!(!cut, "finish cut workers: {results:?}");
        assert_eq!(results.len(), 1);
        assert!(results[0].hold.is_none(), "hold stayed: {results:?}");
        assert!(
            results[0]
                .lines
                .iter()
                .any(|l| l == "landing resumed proj-quiet"),
            "no resumed line: {results:?}"
        );

        // A landing nobody pushed yet gets its push tried, pending or not.
        exec1(
            &s,
            "INSERT INTO events (at, actor, action, subject, detail) VALUES (?, 'daemon', 'land.landed', 'proj-quiet', '{\"project\":\"proj-quiet\"}')",
            &[&TEST_NOW],
        );
        let lines = workers.start_projects(
            &cancel,
            &s,
            std::slice::from_ref(&project),
            "daemon",
            TEST_NOW,
            &BTreeMap::new(),
        );
        assert_eq!(
            lines,
            vec!["landing proj-quiet: started (push owed)".to_string()]
        );
        let (results, cut) = workers.finish(&cancel, None);
        assert!(!cut, "finish cut workers: {results:?}");
        assert!(
            results[0]
                .hold
                .as_deref()
                .is_some_and(|h| h.contains("no upstream")),
            "the owed push was not tried: {results:?}"
        );
    }

    #[test]
    fn busy_project_is_skipped_by_next_worker_pass() {
        let _serial = crate::test_helpers::serial();
        pulse_bus::reset_default();
        let _env = pulse_testutil::env_lock();
        let s = new_store();
        set_setting(&s, "landing", "agents");
        let (script, _) = overlap_script(&s);
        let (project, repo) = add_project(&s, "proj-busy", &script);
        push_branch(&s, &project, &repo, "pulse/busy/1");

        let cancel = pulse_bus::CancellationToken::new();
        let mut workers = LandingWorkers::default();
        assert_eq!(
            workers
                .start_projects(
                    &cancel,
                    &s,
                    std::slice::from_ref(&project),
                    "daemon",
                    TEST_NOW,
                    &BTreeMap::new(),
                )
                .len(),
            1
        );
        assert!(
            workers
                .start_projects(
                    &cancel,
                    &s,
                    &[project],
                    "daemon",
                    TEST_NOW,
                    &BTreeMap::new(),
                )
                .is_empty(),
            "busy project was started again"
        );
        let (results, cut) = workers.finish(&cancel, None);
        assert!(!cut, "finish cut workers: {results:?}");
    }

    #[test]
    fn landing_parallel_one_leaves_daemon_on_serial_path() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let s = new_store();
        set_setting(&s, "landing", "agents");
        set_landing_parallel(&s, "1");
        let (project, repo) = add_project(&s, "proj-serial", "");
        push_branch(&s, &project, &repo, "pulse/serial/1");
        let cancel = pulse_bus::CancellationToken::new();
        let mut workers = LandingWorkers::default();
        let lines = workers.start_projects(
            &cancel,
            &s,
            &[project],
            "daemon",
            TEST_NOW,
            &BTreeMap::new(),
        );
        assert!(lines.is_empty(), "parallel=1 spawned workers: {lines:?}");
        assert_eq!(workers.busy(), 0);
    }

    #[test]
    fn project_lock_blocks_second_lander_while_worker_runs() {
        let _serial = crate::test_helpers::serial();
        pulse_bus::reset_default();
        let _env = pulse_testutil::env_lock();
        let s = new_store();
        set_setting(&s, "landing", "agents");
        let (script, _) = overlap_script(&s);
        let (project, repo) = add_project(&s, "proj-lock", &script);
        push_branch(&s, &project, &repo, "pulse/lock/1");

        let cancel = pulse_bus::CancellationToken::new();
        let mut workers = LandingWorkers::default();
        let lines = workers.start_projects(
            &cancel,
            &s,
            std::slice::from_ref(&project),
            "daemon",
            TEST_NOW,
            &BTreeMap::new(),
        );
        assert_eq!(lines.len(), 1, "worker did not start: {lines:?}");
        let lock = Path::new(&repo).join(".git").join("pulse-land.lock");
        for _ in 0..50 {
            if lock.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(lock.exists(), "worker never took the project landing lock");
        let locked = (0..50).any(|_| {
            let out = crate::land::land_pending_pass(
                &cancel,
                &s,
                project.clone(),
                "daemon".into(),
                TEST_NOW.into(),
                true,
                1,
                false,
                false,
            );
            if out.iter().any(|r| r.state == "locked") {
                true
            } else {
                std::thread::sleep(Duration::from_millis(20));
                false
            }
        });
        assert!(locked, "second lander did not observe the project lock");
        let (results, cut) = workers.finish(&cancel, None);
        assert!(!cut, "finish cut workers: {results:?}");
    }

    #[test]
    fn shutdown_finish_joins_worker_in_flight() {
        let _serial = crate::test_helpers::serial();
        pulse_bus::reset_default();
        let _env = pulse_testutil::env_lock();
        let s = new_store();
        set_setting(&s, "landing", "agents");
        let (script, _) = overlap_script(&s);
        let (project, repo) = add_project(&s, "proj-shutdown", &script);
        push_branch(&s, &project, &repo, "pulse/shutdown/1");
        let cancel = pulse_bus::CancellationToken::new();
        let mut workers = LandingWorkers::default();
        workers.start_projects(
            &cancel,
            &s,
            &[project],
            "daemon",
            TEST_NOW,
            &BTreeMap::new(),
        );
        let (results, cut) = workers.finish(&cancel, None);
        assert!(!cut, "finish cut workers: {results:?}");
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM events WHERE action = 'land.landed'",
                &[]
            ),
            1
        );
    }

    // A worker stuck where no cancellation reaches (a channel nobody sends
    // on) is left behind at the deadline, named with its stage, instead of
    // joined: a daemon restart once waited 3h17m on such a join.
    #[test]
    fn finish_leaves_a_stuck_worker_behind_at_its_deadline() {
        let (_never_tx, never_rx) = crossbeam_channel::bounded::<()>(1);
        let (release_tx, release_rx) = crossbeam_channel::bounded::<()>(1);
        let mut workers = LandingWorkers {
            cancel_grace: Some(Duration::from_millis(100)),
            ..Default::default()
        };
        workers.spawn("stuck-proj", move |stage| {
            crate::daemon_maintenance::set_stage(Some(&stage), "pushing stuck-proj");
            let _ = never_rx.recv();
            let _ = release_rx.recv();
            LandingWorkerResult::default()
        });
        workers.spawn("quick-proj", |stage| {
            crate::daemon_maintenance::set_stage(Some(&stage), "landing quick-proj");
            LandingWorkerResult {
                slug: "quick-proj".into(),
                lines: vec!["landing quick-proj: finished in 0.0s".into()],
                ..Default::default()
            }
        });
        let ctx = pulse_bus::CancellationToken::new();
        let started = Instant::now();
        let (results, cut) =
            workers.finish(&ctx, Some(Instant::now() + Duration::from_millis(100)));
        let took = started.elapsed();
        drop(release_tx);
        assert!(
            took < Duration::from_secs(1),
            "finish took {took:?}: {results:?}"
        );
        assert!(cut, "a worker left behind is a cut: {results:?}");
        assert!(
            ctx.is_cancelled(),
            "the deadline did not cancel the workers"
        );
        assert_eq!(workers.busy(), 0);
        let stuck = results
            .iter()
            .find(|r| r.slug == "stuck-proj")
            .expect("no result for the stuck worker");
        assert!(stuck.detached, "{stuck:?}");
        assert!(stuck.hold.is_none() && stuck.lines.len() == 1, "{stuck:?}");
        assert!(
            stuck.lines[0]
                .starts_with("landing stuck-proj: worker: still at pushing stuck-proj after ")
                && stuck.lines[0].contains("left behind so the daemon can go"),
            "{:?}",
            stuck.lines
        );
        let quick = results
            .iter()
            .find(|r| r.slug == "quick-proj")
            .expect("no result for the quick worker");
        assert!(!quick.detached, "{quick:?}");
    }

    // Once cancelled, a worker waiting in a git that hangs gets its git
    // ended through the process registry and returns on its own; nothing is
    // left behind.
    #[test]
    fn finish_ends_the_git_a_cancelled_worker_waits_in() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo").display().to_string();
        pulse_testutil::git(&["init", "-q", "-b", "main", &root]).unwrap();
        let hang = dir.path().join("hang.sh");
        std::fs::write(&hang, "#!/bin/sh\nsleep 120\n").unwrap();
        run(".", &["chmod", "+x", &hang.display().to_string()]);
        let fsmonitor = format!("core.fsmonitor={}", hang.display());
        let mut workers = LandingWorkers {
            cancel_grace: Some(Duration::from_millis(100)),
            ..Default::default()
        };
        workers.spawn("git-proj", move |stage| {
            crate::daemon_maintenance::set_stage(Some(&stage), "landing git-proj");
            // An hour of budget: only the shutdown can end it in time.
            let r = crate::util::git_timeout(
                &root,
                Duration::from_secs(3600),
                &["-c", &fsmonitor, "status", "--porcelain"],
            );
            LandingWorkerResult {
                slug: "git-proj".into(),
                lines: vec![format!("landing git-proj: git ok={}", r.ok())],
                ..Default::default()
            }
        });
        // Let the git start and block on its fsmonitor.
        std::thread::sleep(Duration::from_millis(300));
        let ctx = pulse_bus::CancellationToken::new();
        ctx.cancel();
        let started = Instant::now();
        let (results, cut) = workers.finish(&ctx, None);
        let took = started.elapsed();
        assert!(
            took < test_wait(Duration::from_secs(10)),
            "finish took {took:?}: {results:?}"
        );
        assert_eq!(results.len(), 1, "{results:?}");
        assert!(
            !results[0].detached,
            "the worker was left behind: {results:?}"
        );
        assert!(!cut, "a worker that returned is no cut: {results:?}");
        let lines = &results[0].lines;
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(
            lines[0].starts_with("landing git-proj: ended git -c core.fsmonitor=false ")
                && lines[0].contains(" (pid ")
                && lines[0].ends_with("after its cancellation so the daemon can go"),
            "{lines:?}"
        );
        assert_eq!(lines[1], "landing git-proj: git ok=false");
    }
}
