//! The watch on a run's hand. A run on a hand (a machine reached over SSH,
//! executor.rs) holds a worker slot, its job's lease and, at a restart, the
//! runner itself until it ends, and the hand does not report the run's
//! progress back. On 2026-11-07 two runs on hands-cr-067 sat "running" for
//! hours after the machine stopped answering (a probe over the open
//! connection got no reply) while the restart waited on them.
//!
//! So while a run goes on a hand, the runner probes the machine every
//! [`PROBE_EVERY`] over the connection the run already shares (from behind
//! the hub's NAT a fresh connect is often refused while the open one works,
//! executor.rs `ssh_options`). Any answer counts, a failing one too: only a
//! probe that times out or cannot reach the machine is a miss. After
//! [`PROBES_MISSED`] misses in a row the run is ended - it reads as lost and
//! its job goes back to the queue (run.rs) - the machine is marked
//! unreachable, so nothing new is sent there until the runner's probe
//! passes again, and the run log and the bus (`executor.silent`) say why.
use crate::exec::RunLog;
use crate::executor::LOCAL_EXECUTOR;
use crate::util::{Row, event_at, exec1, plus_minutes, trunc};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How often a busy hand is probed.
pub(crate) const PROBE_EVERY: Duration = Duration::from_secs(5 * 60);
/// Misses in a row that end the run: about a quarter of an hour of silence.
pub(crate) const PROBES_MISSED: u32 = 3;
/// How long one probe may take; the run's own start allows the same.
pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_secs(120);

/// The watch's pace, so a test can run it in milliseconds.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Timing {
    pub(crate) every: Duration,
    pub(crate) missed: u32,
    pub(crate) timeout: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            every: PROBE_EVERY,
            missed: PROBES_MISSED,
            timeout: PROBE_TIMEOUT,
        }
    }
}

/// Probes the hand: Ok when it answered at all, the reason when it did not.
pub(crate) type Probe =
    Arc<dyn Fn(&Row, &pulse_bus::CancellationToken, Duration) -> Result<(), String> + Send + Sync>;

/// The watch while it runs; it stops when the run ends ([`HandWatch::stop`],
/// or dropped on any way out of the run).
pub(crate) struct HandWatch {
    stop: Option<pulse_bus::CancellationToken>,
    why: Arc<Mutex<Option<String>>>,
}

impl HandWatch {
    fn none() -> HandWatch {
        HandWatch {
            stop: None,
            why: Arc::new(Mutex::new(None)),
        }
    }

    /// Why the watch ended the run, when it did: the run's summary.
    pub(crate) fn why(&self) -> Option<String> {
        self.why.lock().ok().and_then(|w| w.clone())
    }

    pub(crate) fn stop(&mut self) {
        if let Some(t) = self.stop.take() {
            t.cancel();
        }
    }
}

impl Drop for HandWatch {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Watches the hand of a run on `hand` (its executor row); a run here or on
/// anything but an ssh hand needs no watch.
pub(crate) fn watch_hand(
    run_cancel: &pulse_bus::CancellationToken,
    s: &pulse_store::Store,
    hand: &Row,
    run_id: &str,
    job_id: &str,
    log: RunLog,
) -> HandWatch {
    if hand.str("name").is_empty()
        || hand.str("name") == LOCAL_EXECUTOR
        || hand.str("kind") != "ssh"
    {
        return HandWatch::none();
    }
    watch_hand_with(
        run_cancel,
        s,
        hand,
        run_id,
        job_id,
        log,
        Timing::default(),
        Arc::new(probe),
    )
}

/// A probe over the connection the run shares: any answer, a failing one
/// too, means the machine is there.
fn probe(
    hand: &Row,
    cancel: &pulse_bus::CancellationToken,
    timeout: Duration,
) -> Result<(), String> {
    crate::executor::ssh_run_context(cancel, hand, &["probe".to_string()], &[], timeout)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn watch_hand_with(
    run_cancel: &pulse_bus::CancellationToken,
    s: &pulse_store::Store,
    hand: &Row,
    run_id: &str,
    job_id: &str,
    log: RunLog,
    timing: Timing,
    probe: Probe,
) -> HandWatch {
    let watch = run_cancel.child();
    let why = Arc::new(Mutex::new(None));
    let (watch_t, why_t, run_t) = (watch.clone(), Arc::clone(&why), run_cancel.clone());
    let (s, hand, run_id, job_id) = (
        s.clone(),
        hand.clone(),
        run_id.to_string(),
        job_id.to_string(),
    );
    std::thread::spawn(move || {
        let name = hand.str("name");
        let stopped = watch_t.done();
        let mut missed = 0u32;
        let mut first_miss: Option<Instant> = None;
        while let Err(crossbeam_channel::RecvTimeoutError::Timeout) =
            stopped.recv_timeout(timing.every)
        {
            let answer = probe(&hand, &watch_t, timing.timeout);
            if watch_t.is_cancelled() {
                return;
            }
            let reason = match answer {
                Ok(()) => {
                    if missed > 0 {
                        log.add(format!(
                            "{name} answered again after {missed} missed probe(s)"
                        ));
                    }
                    missed = 0;
                    first_miss = None;
                    continue;
                }
                Err(reason) => reason,
            };
            missed += 1;
            let since = *first_miss.get_or_insert_with(Instant::now);
            log.add(format!(
                "{name} did not answer a probe ({missed} of {}): {}",
                timing.missed,
                trunc(&reason, 200)
            ));
            if missed < timing.missed {
                continue;
            }
            let minutes = since.elapsed().as_secs() / 60 + timing.every.as_secs() / 60;
            let summary = format!(
                "the machine {name} stopped answering: {missed} probes in a row over {minutes} min went unanswered; the job returns to the queue"
            );
            if let Ok(mut w) = why_t.lock() {
                *w = Some(summary.clone());
            }
            log.add(summary.clone());
            give_back(&s, &name, &run_id, &job_id, missed, minutes, &reason);
            run_t.cancel();
            return;
        }
    });
    HandWatch {
        stop: Some(watch),
        why,
    }
}

/// Marks the silent hand unreachable (nothing new goes there until the
/// runner's probe passes again) and says so on the bus.
fn give_back(
    s: &pulse_store::Store,
    name: &str,
    run_id: &str,
    job_id: &str,
    missed: u32,
    minutes: u64,
    reason: &str,
) {
    let Ok(conn) = s.conn() else {
        return;
    };
    let now = s.now();
    let note = trunc(
        &format!(
            "stopped answering during run {run_id}: {missed} probes in a row over {minutes} min went unanswered ({reason})"
        ),
        300,
    );
    let next = plus_minutes(&now, 1.0);
    exec1(
        &conn,
        "UPDATE executors SET status = 'unreachable', errors = errors + 1, backoff_minutes = 1, next_probe_at = ?, probe_note = ? WHERE name = ? AND status = 'active'",
        &[&next, &note, &name],
    );
    event_at(
        &conn,
        &now,
        "runner",
        "executor.silent",
        name,
        json!({"run": run_id, "job": job_id, "missed": missed, "minutes": minutes, "why": trunc(reason, 200)}),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn hand_store() -> (crate::test_helpers::TestStore, Row) {
        let s = crate::test_helpers::new_store();
        s.conn()
            .unwrap()
            .execute(
                "INSERT INTO executors (name, kind, address, workdir, status, created_at) VALUES ('hands-x', 'ssh', 'pulse@hands-x', '/srv/pulse', 'active', '2026-11-07T10:00:00Z')",
                [],
            )
            .unwrap();
        let hand = crate::executor::executor_row(&s, "hands-x");
        (s, hand)
    }

    fn fast() -> Timing {
        Timing {
            every: Duration::from_millis(10),
            missed: 3,
            timeout: Duration::from_millis(50),
        }
    }

    fn wait_for(f: impl Fn() -> bool) -> bool {
        for _ in 0..300 {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn a_hand_that_stops_answering_gives_its_run_back() {
        let _serial = crate::test_helpers::serial();
        let (s, hand) = hand_store();
        let run = pulse_bus::CancellationToken::new();
        let log = RunLog::default();
        let tries = Arc::new(AtomicU32::new(0));
        let counted = Arc::clone(&tries);
        let watch = watch_hand_with(
            &run,
            &s,
            &hand,
            "run-1",
            "job-1",
            log.clone(),
            fast(),
            Arc::new(move |_, _, _| {
                counted.fetch_add(1, Ordering::SeqCst);
                Err("ssh probe timed out after 120s".into())
            }),
        );
        assert!(wait_for(|| run.is_cancelled()), "the run was ended");
        assert_eq!(
            tries.load(Ordering::SeqCst),
            3,
            "three misses, then nothing more"
        );
        let why = watch.why().unwrap();
        assert!(
            why.starts_with("the machine hands-x stopped answering: 3 probes in a row"),
            "{why}"
        );
        assert!(why.ends_with("the job returns to the queue"));
        let row = crate::executor::executor_row(&s, "hands-x");
        assert_eq!(row.str("status"), "unreachable");
        assert!(
            row.str("probe_note")
                .contains("stopped answering during run run-1")
        );
        assert_eq!(
            crate::test_helpers::count(
                &s,
                "SELECT COUNT(*) AS n FROM events WHERE action = 'executor.silent' AND subject = 'hands-x'",
                &[]
            ),
            1
        );
        let lines = log.text();
        assert!(
            lines.contains("hands-x did not answer a probe (1 of 3): ssh probe timed out"),
            "{lines}"
        );
    }

    #[test]
    fn an_answer_between_misses_starts_the_count_over() {
        let _serial = crate::test_helpers::serial();
        let (s, hand) = hand_store();
        let run = pulse_bus::CancellationToken::new();
        let log = RunLog::default();
        let tries = Arc::new(AtomicU32::new(0));
        let counted = Arc::clone(&tries);
        // Miss, miss, answer, over and over: never three in a row.
        let mut watch = watch_hand_with(
            &run,
            &s,
            &hand,
            "run-2",
            "job-2",
            log.clone(),
            fast(),
            Arc::new(move |_, _, _| {
                let n = counted.fetch_add(1, Ordering::SeqCst);
                if n % 3 == 2 {
                    Ok(())
                } else {
                    Err("connection refused".into())
                }
            }),
        );
        assert!(wait_for(|| tries.load(Ordering::SeqCst) >= 9));
        watch.stop();
        assert!(!run.is_cancelled(), "the run goes on");
        assert!(watch.why().is_none());
        assert_eq!(
            crate::executor::executor_row(&s, "hands-x").str("status"),
            "active"
        );
        assert!(
            log.text()
                .contains("answered again after 2 missed probe(s)")
        );
    }

    #[test]
    fn the_watch_ends_with_the_run_and_a_run_here_has_none() {
        let _serial = crate::test_helpers::serial();
        let (s, hand) = hand_store();
        let run = pulse_bus::CancellationToken::new();
        let tries = Arc::new(AtomicU32::new(0));
        let counted = Arc::clone(&tries);
        let watch = watch_hand_with(
            &run,
            &s,
            &hand,
            "run-3",
            "job-3",
            RunLog::default(),
            Timing {
                every: Duration::from_millis(30),
                ..fast()
            },
            Arc::new(move |_, _, _| {
                counted.fetch_add(1, Ordering::SeqCst);
                Err("down".into())
            }),
        );
        drop(watch);
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(
            tries.load(Ordering::SeqCst),
            0,
            "stopped before its first probe"
        );
        assert!(!run.is_cancelled());
        // A run on this machine, or a hand that is not ssh: no watch at all.
        let local = crate::executor::executor_row(&s, LOCAL_EXECUTOR);
        let w = watch_hand(&run, &s, &local, "run-4", "job-4", RunLog::default());
        assert!(w.stop.is_none() && w.why().is_none());
    }
}
