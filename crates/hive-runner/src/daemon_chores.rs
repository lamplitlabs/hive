//! The daemon's chores off the tick (TASK-081): the Dependabot alert watch
//! (security.rs), the pipeline verdicts (pipeline.rs) and the compute worker's
//! listing (compute.rs `tick`) ran inline in the 15-second dispatch tick and
//! made it late - 488 slow-tick lines with a median of 21.5 s, the longest
//! stages among them pipeline, security and compute. They run here, on one
//! background thread of the same shape as the landing maintenance thread:
//! the tick asks for a pass and goes on leasing; a pass asked while one runs
//! is merged into the next one. Each stage has the budget
//! `daemon.stage_budget_seconds` sets (stage_budget.rs).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// One pass asked by the tick: the compute listing always, the security and
/// pipeline stages for `projects` on a planning pass (`None` otherwise).
#[derive(Debug, Clone, Default)]
pub(crate) struct ChoresAsk {
    pub(crate) now: String,
    pub(crate) projects: Option<Vec<String>>,
}

impl ChoresAsk {
    /// A later ask over an earlier one not yet taken: the newer clock, and the
    /// projects of whichever asked for them.
    fn merge(self, later: ChoresAsk) -> ChoresAsk {
        ChoresAsk {
            now: later.now,
            projects: later.projects.or(self.projects),
        }
    }
}

/// What one stage of a pass runs; the daemon's are security, pipeline and
/// compute, and tests put their own in.
pub(crate) type ProjectChore =
    Arc<dyn Fn(&hive_store::Store, &str, &str, &str) -> Vec<String> + Send + Sync>;
pub(crate) type PassChore =
    Arc<dyn Fn(&hive_bus::CancellationToken, &str) -> Vec<String> + Send + Sync>;

/// How long a chores pass may run before the daemon says so: past it the
/// pass is in a call no stage budget reaches (budgets are checked between
/// units), and compute waits behind it.
pub(crate) const CHORES_STUCK_AFTER: Duration = Duration::from_secs(30 * 60);

#[derive(Default)]
struct Shared {
    asked: Option<ChoresAsk>,
    running: bool,
    closed: bool,
    /// When the running pass began, and whether its overrun was said.
    began: Option<Instant>,
    stuck_said: bool,
}

pub(crate) struct DaemonChores {
    shared: Arc<(Mutex<Shared>, std::sync::Condvar)>,
    lines_rx: crossbeam_channel::Receiver<String>,
    cancel: hive_bus::CancellationToken,
    thread: Option<std::thread::JoinHandle<()>>,
    stage: crate::daemon_maintenance::Stage,
}

/// The daemon's chores: security and pipeline per project, then compute.
pub(crate) fn start(
    s: &hive_store::Store,
    actor: &str,
    compute: crate::compute::ComputeWorker,
    compute_ctx: hive_bus::CancellationToken,
) -> DaemonChores {
    let security: ProjectChore = Arc::new(crate::security::tend_dependency_alerts);
    let pipeline: ProjectChore = Arc::new(crate::pipeline::tend_pipeline);
    let compute: PassChore = Arc::new(move |_, now| compute.tick(&compute_ctx, now));
    start_with(
        s,
        actor,
        vec![("security", security), ("pipeline", pipeline)],
        compute,
    )
}

pub(crate) fn start_with(
    s: &hive_store::Store,
    actor: &str,
    per_project: Vec<(&'static str, ProjectChore)>,
    compute: PassChore,
) -> DaemonChores {
    let shared: Arc<(Mutex<Shared>, std::sync::Condvar)> = Arc::default();
    let (lines_tx, lines_rx) = crossbeam_channel::bounded::<String>(1024);
    let cancel = hive_bus::CancellationToken::new();
    let stage: crate::daemon_maintenance::Stage = Arc::default();
    let thread = {
        let shared = shared.clone();
        let cancel = cancel.clone();
        let stage = stage.clone();
        let s = s.clone();
        let actor = actor.to_string();
        std::thread::Builder::new()
            .name("daemon-chores".into())
            .spawn(move || {
                let resume = crate::stage_budget::Resume::default();
                loop {
                    let ask = {
                        let (mu, cv) = &*shared;
                        let mut st = mu.lock().unwrap_or_else(|e| e.into_inner());
                        loop {
                            if let Some(ask) = st.asked.take() {
                                st.running = true;
                                st.began = Some(Instant::now());
                                st.stuck_said = false;
                                break Some(ask);
                            }
                            if st.closed {
                                break None;
                            }
                            st = cv.wait(st).unwrap_or_else(|e| e.into_inner());
                        }
                    };
                    let Some(ask) = ask else {
                        return;
                    };
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        run_pass(
                            &s,
                            &actor,
                            &resume,
                            &cancel,
                            &stage,
                            &per_project,
                            &compute,
                            &ask,
                        )
                    }))
                    .unwrap_or_else(|panic| {
                        vec![format!("daemon chores error: {}", panic_text(&panic))]
                    });
                    for line in result {
                        let _ = lines_tx.try_send(line);
                    }
                    crate::daemon_maintenance::set_stage(Some(&stage), "idle");
                    let (mu, cv) = &*shared;
                    {
                        let mut st = mu.lock().unwrap_or_else(|e| e.into_inner());
                        st.running = false;
                        st.began = None;
                    }
                    cv.notify_all();
                }
            })
            .expect("spawn daemon chores thread")
    };
    DaemonChores {
        shared,
        lines_rx,
        cancel,
        thread: Some(thread),
        stage,
    }
}

#[allow(clippy::too_many_arguments)]
fn run_pass(
    s: &hive_store::Store,
    actor: &str,
    resume: &crate::stage_budget::Resume,
    cancel: &hive_bus::CancellationToken,
    stage: &crate::daemon_maintenance::Stage,
    per_project: &[(&'static str, ProjectChore)],
    compute: &PassChore,
    ask: &ChoresAsk,
) -> Vec<String> {
    let b = crate::stage_budget::Budgeted {
        s,
        actor,
        budget: crate::stage_budget::budget(s),
        resume,
        cancel,
        stage: Some(stage),
    };
    let mut lines = Vec::new();
    if let Some(projects) = &ask.projects {
        for (name, chore) in per_project {
            lines.extend(b.each(
                name,
                projects,
                |slug| slug.clone(),
                |slug| chore(s, slug, &ask.now, actor),
            ));
        }
    }
    lines.extend(b.once("compute", || compute(cancel, &ask.now)));
    lines
}

fn panic_text(panic: &Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".into())
}

impl DaemonChores {
    /// Asks for a pass; returns at once. An ask the thread has not taken yet
    /// is merged with this one.
    pub(crate) fn ask(&self, ask: ChoresAsk) {
        let (mu, cv) = &*self.shared;
        let mut st = mu.lock().unwrap_or_else(|e| e.into_inner());
        if st.closed {
            return;
        }
        st.asked = Some(match st.asked.take() {
            Some(earlier) => earlier.merge(ask),
            None => ask,
        });
        cv.notify_all();
    }

    /// The lines the passes said since the last call, without waiting, and
    /// once per pass a note when it has run past `CHORES_STUCK_AFTER`.
    pub(crate) fn drain(&self) -> Vec<String> {
        let mut lines = self.lines_rx.try_iter().collect::<Vec<_>>();
        lines.extend(self.stuck_note(CHORES_STUCK_AFTER));
        lines
    }

    fn stuck_note(&self, after: Duration) -> Option<String> {
        let mut st = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
        let ran = st.began?.elapsed();
        if st.stuck_said || ran < after {
            return None;
        }
        st.stuck_said = true;
        drop(st);
        Some(format!(
            "daemon chores pass running for {} (at: {}); security, pipeline and compute wait behind it",
            hive_cli::format_duration(Duration::from_secs(ran.as_secs())),
            self.stage()
        ))
    }

    /// Whether a pass runs or waits to.
    pub(crate) fn busy(&self) -> bool {
        let st = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
        st.running || st.asked.is_some()
    }

    /// Where the running pass is (`security proj`, `compute`, `idle`).
    pub(crate) fn stage(&self) -> String {
        self.stage.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Closes the chores: the pass asked last still runs - its compute stage
    /// starts the provisioning the tick saw approved - and is waited for up
    /// to `wait` (calling `beat` about once a second); past it the pass is
    /// cancelled between two units, and after `grace` more it is left
    /// behind. Returns the lines the passes said, and a note when it was cut.
    pub(crate) fn close(
        &mut self,
        wait: Duration,
        grace: Duration,
        beat: &dyn Fn(),
    ) -> Vec<String> {
        {
            let (mu, cv) = &*self.shared;
            mu.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
            cv.notify_all();
        }
        let mut lines = Vec::new();
        let started = Instant::now();
        let mut cut = false;
        let mut beaten = Instant::now();
        while self.thread.as_ref().is_some_and(|t| !t.is_finished()) {
            if !cut && started.elapsed() >= wait {
                cut = true;
                lines.push(format!(
                    "daemon chores still running after {} (at: {}); cancelled between two units",
                    hive_cli::format_duration(wait),
                    self.stage()
                ));
                self.cancel.cancel();
            }
            if cut && started.elapsed() >= wait + grace {
                lines.push(format!(
                    "daemon chores did not stop {} after their cancellation (at: {}); left behind",
                    hive_cli::format_duration(grace),
                    self.stage()
                ));
                self.thread = None;
                break;
            }
            if !cut && beaten.elapsed() >= Duration::from_secs(1) {
                beat();
                beaten = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(20).min(wait.max(Duration::from_millis(1))));
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        lines.extend(self.lines_rx.try_iter());
        lines
    }
}

impl Drop for DaemonChores {
    fn drop(&mut self) {
        let (mu, cv) = &*self.shared;
        mu.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
        cv.notify_all();
        self.cancel.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::*;

    fn wait_idle(c: &DaemonChores) {
        let until = Instant::now() + crate::test_helpers_daemon::test_wait(Duration::from_secs(10));
        while c.busy() {
            assert!(Instant::now() < until, "the chores pass never ended");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The tick asks and goes on: a pipeline stage that hangs on one project
    /// does not hold the ask, the stage is cut at its budget with a
    /// daemon.slow-stage event, and the next pass starts with what it left.
    #[test]
    fn a_slow_stage_runs_off_the_tick_and_is_skipped_past_its_budget() {
        let s = new_store();
        crate::test_helpers_daemon::set_setting(&s, crate::stage_budget::STAGE_BUDGET, "1");
        let seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let pipeline: ProjectChore = {
            let seen = seen.clone();
            let gate = gate.clone();
            Arc::new(move |_, slug, _, _| {
                seen.lock().unwrap().push(format!("pipeline {slug}"));
                if slug == "slow" {
                    // Held until the test has seen the ask return.
                    let (mu, cv) = &*gate;
                    let _open = cv.wait_while(mu.lock().unwrap(), |open| !*open).unwrap();
                    std::thread::sleep(Duration::from_millis(1100));
                }
                vec![format!("pipeline: {slug} judged")]
            })
        };
        let computes = Arc::new(Mutex::new(0));
        let compute: PassChore = {
            let computes = computes.clone();
            Arc::new(move |_, _| {
                *computes.lock().unwrap() += 1;
                Vec::new()
            })
        };
        let mut c = start_with(&s, "daemon", vec![("pipeline", pipeline)], compute);
        let tick = Instant::now();
        c.ask(ChoresAsk {
            now: TEST_NOW.into(),
            projects: Some(vec!["slow".into(), "a".into(), "b".into()]),
        });
        // The ask returns while the slow project is still being judged.
        assert!(tick.elapsed() < Duration::from_millis(500));
        assert!(c.busy());
        {
            let (mu, cv) = &*gate;
            *mu.lock().unwrap() = true;
            cv.notify_all();
        }
        wait_idle(&c);
        let lines = c.drain();
        assert_eq!(*seen.lock().unwrap(), ["pipeline slow"], "{lines:?}");
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("slow stage: pipeline took ")
                    && l.ends_with("skipped until the next pass: a, b")),
            "{lines:?}"
        );
        assert_eq!(*computes.lock().unwrap(), 1, "compute still ran this pass");
        let conn = s.conn().unwrap();
        let slow = crate::util::must_one(
            &conn,
            "SELECT COUNT(*) AS n FROM events WHERE action = 'daemon.slow-stage' AND subject = 'pipeline'",
            &[],
        );
        assert_eq!(slow.int("n"), 1);
        drop(conn);
        // The next planning pass starts where this one stopped.
        seen.lock().unwrap().clear();
        c.ask(ChoresAsk {
            now: TEST_NOW.into(),
            projects: Some(vec!["slow".into(), "a".into(), "b".into()]),
        });
        wait_idle(&c);
        assert_eq!(seen.lock().unwrap()[..2], ["pipeline a", "pipeline b"]);
        let lines = c.close(Duration::from_secs(5), Duration::from_secs(1), &|| {});
        assert!(
            lines.iter().all(|l| !l.contains("left behind")),
            "{lines:?}"
        );
    }

    /// A pass held in a call no budget reaches is said once, not every tick.
    #[test]
    fn a_pass_that_overruns_is_said_once() {
        let s = new_store();
        let release = Arc::new(Mutex::new(()));
        let held = release.lock().unwrap();
        let compute: PassChore = {
            let release = release.clone();
            Arc::new(move |_, _| {
                drop(release.lock().unwrap());
                Vec::new()
            })
        };
        let mut c = start_with(&s, "daemon", Vec::new(), compute);
        c.ask(ChoresAsk {
            now: TEST_NOW.into(),
            projects: None,
        });
        let until = Instant::now() + crate::test_helpers_daemon::test_wait(Duration::from_secs(10));
        while c.stage() != "compute" {
            assert!(Instant::now() < until, "the pass never reached compute");
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(20));
        let note = c
            .stuck_note(Duration::from_millis(10))
            .expect("an overrun note");
        assert!(note.contains("(at: compute)"), "{note}");
        assert_eq!(c.stuck_note(Duration::from_millis(10)), None);
        drop(held);
        wait_idle(&c);
        assert_eq!(c.stuck_note(Duration::ZERO), None);
        c.close(Duration::from_secs(5), Duration::from_secs(1), &|| {});
    }

    #[test]
    fn a_tick_that_only_reaps_asks_for_compute_alone_and_asks_merge() {
        let s = new_store();
        let seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let security: ProjectChore = {
            let seen = seen.clone();
            Arc::new(move |_, slug, now, _| {
                seen.lock().unwrap().push(format!("security {slug} {now}"));
                Vec::new()
            })
        };
        let compute: PassChore = {
            let seen = seen.clone();
            Arc::new(move |_, now| {
                seen.lock().unwrap().push(format!("compute {now}"));
                Vec::new()
            })
        };
        // Asks made before the thread takes them are one pass.
        let gate = Arc::new(Mutex::new(()));
        let held = gate.lock().unwrap();
        let blocker: ProjectChore = {
            let gate = gate.clone();
            Arc::new(move |_, _, _, _| {
                drop(gate.lock().unwrap());
                Vec::new()
            })
        };
        let mut c = start_with(
            &s,
            "daemon",
            vec![("blocker", blocker), ("security", security)],
            compute,
        );
        c.ask(ChoresAsk {
            now: "t1".into(),
            projects: Some(vec!["p".into()]),
        });
        // The first pass is held in the blocker; these two merge.
        let until = Instant::now() + crate::test_helpers_daemon::test_wait(Duration::from_secs(10));
        while c.stage() != "blocker p" {
            assert!(
                Instant::now() < until,
                "the first pass never reached the blocker"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        c.ask(ChoresAsk {
            now: "t2".into(),
            projects: Some(vec!["q".into()]),
        });
        c.ask(ChoresAsk {
            now: "t3".into(),
            projects: None,
        });
        drop(held);
        wait_idle(&c);
        assert_eq!(
            *seen.lock().unwrap(),
            ["security p t1", "compute t1", "security q t3", "compute t3"]
        );
        // Closing runs the ask made last.
        seen.lock().unwrap().clear();
        c.ask(ChoresAsk {
            now: "t4".into(),
            projects: None,
        });
        c.close(Duration::from_secs(5), Duration::from_secs(1), &|| {});
        assert_eq!(*seen.lock().unwrap(), ["compute t4"]);
    }
}
