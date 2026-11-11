//! Time budgets for the daemon's background stages (TASK-081).
//!
//! The landing maintenance pass (daemon_maintenance.rs) and the chores pass
//! (daemon_chores.rs: Dependabot alerts, pipeline verdicts, compute) run off
//! the daemon tick, one stage after the other. Each stage has the budget
//! `daemon.stage_budget_seconds` sets. A stage works in units - one project,
//! or the whole stage when it cannot be split - and the budget is checked
//! between units, never inside one: a unit is never killed mid-write. A stage
//! still going when its budget is spent stops before its next unit, records a
//! `daemon.slow-stage` event with its duration and the units it left, and the
//! next pass starts with the first unit it left, so the projects at the end of
//! the list are not starved by a slow one at the front. A stage that cannot
//! be split and ran past its budget is recorded the same way once it ends.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The event a stage over its budget writes.
pub(crate) const SLOW_STAGE_ACTION: &str = "daemon.slow-stage";
/// The setting naming every stage's budget, in seconds; 0 lets a stage run
/// to its end.
pub(crate) const STAGE_BUDGET: &str = "daemon.stage_budget_seconds";

/// Per stage, the unit a stage over its budget left first; the next pass of
/// that stage starts there. Kept by the owner of the passes across them.
pub(crate) type Resume = Arc<Mutex<BTreeMap<String, String>>>;

/// The budget `daemon.stage_budget_seconds` sets; `None` when it is 0.
pub(crate) fn budget(s: &hive_store::Store) -> Option<Duration> {
    let secs = s.setting_int(STAGE_BUDGET);
    (secs > 0).then(|| Duration::from_secs(secs as u64))
}

/// Runs the stages of one background pass under their budget.
pub(crate) struct Budgeted<'a> {
    pub(crate) s: &'a hive_store::Store,
    pub(crate) actor: &'a str,
    pub(crate) budget: Option<Duration>,
    pub(crate) resume: &'a Resume,
    pub(crate) cancel: &'a hive_bus::CancellationToken,
    /// Where the pass is, for the watchdog's line (`set_stage`).
    pub(crate) stage: Option<&'a crate::daemon_maintenance::Stage>,
}

impl Budgeted<'_> {
    /// Runs `run` for each unit, in the order the last pass left them, until
    /// the budget is spent. At least one unit always runs, so a stage whose
    /// first unit alone outlasts the budget still moves forward.
    pub(crate) fn each<T>(
        &self,
        name: &str,
        units: &[T],
        key: impl Fn(&T) -> String,
        mut run: impl FnMut(&T) -> Vec<String>,
    ) -> Vec<String> {
        let keys = units.iter().map(&key).collect::<Vec<_>>();
        let first = {
            let resume = self.resume.lock().unwrap_or_else(|e| e.into_inner());
            resume
                .get(name)
                .and_then(|k| keys.iter().position(|x| x == k))
                .unwrap_or(0)
        };
        let order = (first..units.len()).chain(0..first).collect::<Vec<_>>();
        let started = Instant::now();
        let mut lines = Vec::new();
        for (n, &i) in order.iter().enumerate() {
            if self.cancel.is_cancelled() {
                return lines;
            }
            if let Some(budget) = self.budget
                && n > 0
                && started.elapsed() >= budget
            {
                let left = order[n..]
                    .iter()
                    .map(|&j| keys[j].clone())
                    .collect::<Vec<_>>();
                self.resume
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(name.to_string(), left[0].clone());
                lines.push(self.slow(name, started.elapsed(), budget, n, &left));
                return lines;
            }
            crate::daemon_maintenance::set_stage(self.stage, format!("{name} {}", keys[i]));
            lines.extend(run(&units[i]));
        }
        self.resume
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(name);
        if let Some(budget) = self.budget
            && started.elapsed() > budget
        {
            lines.push(self.slow(name, started.elapsed(), budget, order.len(), &[]));
        }
        lines
    }

    /// Runs a stage that cannot be split, unless the pass was cancelled;
    /// one that ran past its budget is recorded once it ends.
    pub(crate) fn once(&self, name: &str, run: impl FnOnce() -> Vec<String>) -> Vec<String> {
        if self.cancel.is_cancelled() {
            return Vec::new();
        }
        crate::daemon_maintenance::set_stage(self.stage, name);
        let started = Instant::now();
        let mut lines = run();
        if let Some(budget) = self.budget
            && started.elapsed() > budget
        {
            lines.push(self.slow(name, started.elapsed(), budget, 1, &[]));
        }
        lines
    }

    /// Writes the `daemon.slow-stage` event and returns the daemon's line.
    fn slow(
        &self,
        name: &str,
        took: Duration,
        budget: Duration,
        ran: usize,
        left: &[String],
    ) -> String {
        if let Ok(conn) = self.s.conn() {
            crate::util::event_at(
                &conn,
                &self.s.now(),
                self.actor,
                SLOW_STAGE_ACTION,
                name,
                serde_json::json!({
                    "stage": name,
                    "ms": took.as_millis() as u64,
                    "budget_ms": budget.as_millis() as u64,
                    "ran": ran,
                    "skipped": left,
                }),
            );
        }
        let took = hive_cli::format_duration(Duration::from_millis(took.as_millis() as u64));
        let budget = hive_cli::format_duration(budget);
        if left.is_empty() {
            format!(
                "slow stage: {name} took {took}, over its {budget} budget; it ran to its end (a stage is never cut mid-write)"
            )
        } else {
            format!(
                "slow stage: {name} took {took}, over its {budget} budget; skipped until the next pass: {}",
                left.join(", ")
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::*;

    fn slow_events(s: &hive_store::Store) -> Vec<crate::util::Row> {
        let conn = s.conn().unwrap();
        crate::util::must_rows(
            &conn,
            "SELECT subject, detail FROM events WHERE action = ? ORDER BY seq",
            &[&SLOW_STAGE_ACTION],
        )
    }

    #[test]
    fn a_stage_over_its_budget_skips_its_other_units_until_the_next_pass() {
        let s = new_store();
        let resume = Resume::default();
        let cancel = hive_bus::CancellationToken::new();
        let b = Budgeted {
            s: &s,
            actor: "daemon",
            budget: Some(Duration::from_millis(50)),
            resume: &resume,
            cancel: &cancel,
            stage: None,
        };
        let units = ["a", "b", "c"].map(String::from);
        let mut ran = Vec::new();
        let lines = b.each(
            "pipeline",
            &units,
            |u| u.clone(),
            |u| {
                ran.push(u.clone());
                if u == "a" {
                    std::thread::sleep(Duration::from_millis(80));
                }
                Vec::new()
            },
        );
        // a ran past the budget and was not cut; b and c wait.
        assert_eq!(ran, ["a"]);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].starts_with("slow stage: pipeline took ")
                && lines[0].ends_with("skipped until the next pass: b, c"),
            "{lines:?}"
        );
        let events = slow_events(&s);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].str("subject"), "pipeline");
        let detail: serde_json::Value = serde_json::from_str(&events[0].str("detail")).unwrap();
        assert!(detail["ms"].as_u64().unwrap() >= 80, "{detail}");
        assert_eq!(detail["budget_ms"], 50);
        assert_eq!(detail["skipped"], serde_json::json!(["b", "c"]));

        // The next pass starts where this one stopped and wraps round.
        let mut ran = Vec::new();
        let lines = b.each(
            "pipeline",
            &units,
            |u| u.clone(),
            |u| {
                ran.push(u.clone());
                Vec::new()
            },
        );
        assert_eq!(ran, ["b", "c", "a"]);
        assert!(lines.is_empty(), "{lines:?}");
        assert!(resume.lock().unwrap().is_empty());
        assert_eq!(slow_events(&s).len(), 1);
    }

    #[test]
    fn a_stage_that_cannot_be_split_is_recorded_once_it_ends() {
        let s = new_store();
        let resume = Resume::default();
        let cancel = hive_bus::CancellationToken::new();
        let b = Budgeted {
            s: &s,
            actor: "daemon",
            budget: Some(Duration::from_millis(20)),
            resume: &resume,
            cancel: &cancel,
            stage: None,
        };
        let lines = b.once("compute", || {
            std::thread::sleep(Duration::from_millis(40));
            vec!["compute: done".into()]
        });
        assert_eq!(lines[0], "compute: done");
        assert!(lines[1].contains("it ran to its end"), "{lines:?}");
        assert_eq!(slow_events(&s).len(), 1);
        // Within its budget, or with none, nothing is recorded.
        let quick = b.once("compute", Vec::new);
        assert!(quick.is_empty());
        let unbudgeted = Budgeted { budget: None, ..b };
        unbudgeted.once("compute", || {
            std::thread::sleep(Duration::from_millis(40));
            Vec::new()
        });
        assert_eq!(slow_events(&s).len(), 1);
        // A cancelled pass starts nothing more.
        cancel.cancel();
        let mut called = false;
        unbudgeted.once("compute", || {
            called = true;
            Vec::new()
        });
        assert!(!called);
    }

    #[test]
    fn the_budget_setting_turns_off_at_zero() {
        let s = new_store();
        assert_eq!(budget(&s), Some(Duration::from_secs(60)));
        crate::test_helpers_daemon::set_setting(&s, STAGE_BUDGET, "0");
        assert_eq!(budget(&s), None);
        crate::test_helpers_daemon::set_setting(&s, STAGE_BUDGET, "5");
        assert_eq!(budget(&s), Some(Duration::from_secs(5)));
    }
}
