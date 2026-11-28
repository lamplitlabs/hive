//! One event when an owner-driven job has waited too long, naming why
//! (TASK-067 part c).
//!
//! On 2026-11-03 the owner's TASK-055 queued at 04:40 and started at 15:18:
//! the lanes were full of the fleet's work, the load sat at 26-30 over a
//! line of 24 and a restart window closed slots in between - and nothing the
//! owner could read said so; the task page said "waiting for a free agent"
//! all day. So on each tick the daemon notes, for every owner-driven job
//! still queued (jobs.rs `OWNER_DRIVEN_SQL`), the holds that tick's dispatch
//! saw (idle_hold.rs: the load, memory, the disk floor, the bulk disk, a
//! restart drain, every slot busy with the fleet's work; and the daily cap),
//! short: "lanes full", "load 27 > 24". Once the job has waited
//! `hive_tasks::OWNER_WAIT_MINUTES` it writes one
//! `hive_tasks::OWNER_WAIT_EVENT` on the job naming them all, and logs it.
//! `hive status` lists the open waits ([`status_lines`]) and the hub's task
//! wait line names the reasons (hive_tasks::Work::waiting). A job queued
//! again later is a new wait and gets its own event.

use crate::idle_hold::{BULK_AWAY, DISK_FLOOR, FLEET_FULL, Hold, LOAD, MEMORY, RESTART_DRAIN};
use crate::util::Row;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::LazyLock;

static LOAD_LINE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"load (\d+(?:\.\d+)?) on \d+ CPUs is over the line (\d+(?:\.\d+)?)")
        .expect("load line regex")
});
static MEMORY_LINE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(\d+) MB of memory free, under the floor (\d+)").expect("memory regex")
});

/// A hold as the owner reads it in a list: "load 27 > 24", "lanes full".
pub(crate) fn short_reason(hold: &Hold) -> String {
    let kind = hold.kind.ask_kind;
    if kind == LOAD.ask_kind {
        if let Some(c) = LOAD_LINE.captures(&hold.reason) {
            let load = c[1].parse::<f64>().unwrap_or_default();
            return format!("load {load:.0} > {}", &c[2]);
        }
        return "machine load".into();
    }
    if kind == MEMORY.ask_kind {
        if let Some(c) = MEMORY_LINE.captures(&hold.reason) {
            return format!("memory {} MB < {} MB", &c[1], &c[2]);
        }
        return "memory pressure".into();
    }
    if kind == FLEET_FULL.ask_kind {
        return "lanes full".into();
    }
    if kind == RESTART_DRAIN.ask_kind {
        return "restart drain".into();
    }
    if kind == DISK_FLOOR.ask_kind {
        return "disk floor".into();
    }
    if kind == BULK_AWAY.ask_kind {
        return "bulk disk away".into();
    }
    hold.kind.label.to_string()
}

/// The short reasons of one tick: its holds, the daily cap when it held,
/// and - when nothing named holds - the tick's own idle reason (every slot
/// busy is "lanes full"; an empty queue is no reason at all).
pub(crate) fn tick_reasons(holds: &[Hold], cap_holding: bool, idle: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for h in holds {
        add_reason(&mut out, short_reason(h));
    }
    if cap_holding {
        add_reason(&mut out, "daily cap reached".into());
    }
    if out.is_empty() {
        let idle = idle.trim();
        if idle.starts_with("every executor slot is busy") {
            add_reason(&mut out, "lanes full".into());
        } else if !idle.is_empty() && !idle.starts_with("no queued job") {
            add_reason(&mut out, crate::util::trunc(idle, 80));
        }
    }
    out
}

fn add_reason(out: &mut Vec<String>, r: String) {
    if !r.is_empty() && !out.contains(&r) {
        out.push(r);
    }
}

/// What one tick put ahead of a waiting review (TASK-186): the jobs it
/// leased on this machine's slots while the review stayed queued, and
/// whether the disk headroom closed a local slot. A review runs on this
/// machine alone (jobs.rs `STORE_KINDS`), so a lease on a hand is not ahead
/// of it. On 27 November the wait event of a review queued 40 minutes had
/// an empty reasons list while six work jobs were leased before it.
#[derive(Debug, Default, Clone)]
pub(crate) struct Ahead {
    /// The local leases of the tick, in lease order.
    pub(crate) leased: Vec<Leased>,
    /// A local slot waited for disk headroom this tick.
    pub(crate) disk: bool,
}

/// One local lease of a tick, as a waiting review's reason names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Leased {
    pub(crate) job: String,
    pub(crate) kind: String,
    /// It took the machine's last free slot, the one kept back from the
    /// owner's work past its share for a lead's sitting, an answer or a
    /// review (jobs.rs `LeaseOpts::keep_slot`).
    pub(crate) kept_slot: bool,
}

/// How many leased jobs a review's reason names before "and N more".
const AHEAD_NAMED: usize = 3;

/// The short reason for the jobs leased ahead of a review: "leased first:
/// job-a (task), job-b (lead sitting, kept slot) and 2 more".
fn leased_first(ahead: &[Leased]) -> String {
    if ahead.is_empty() {
        return String::new();
    }
    let named = ahead
        .iter()
        .take(AHEAD_NAMED)
        .map(|l| {
            let kind = if l.kind == crate::lead::LEAD_KIND {
                "lead sitting"
            } else {
                l.kind.as_str()
            };
            let kept = if l.kept_slot { ", kept slot" } else { "" };
            format!("{} ({kind}{kept})", l.job)
        })
        .collect::<Vec<_>>()
        .join(", ");
    match ahead.len().saturating_sub(AHEAD_NAMED) {
        0 => format!("leased first: {named}"),
        more => format!("leased first: {named} and {more} more"),
    }
}

/// The reason a review waited for disk headroom.
const DISK_HEADROOM: &str = "disk headroom";

#[derive(Debug, Default)]
struct Seen {
    /// When the job was queued; a new queuing is a new wait.
    since: String,
    /// The reasons seen while it waited, first seen first.
    reasons: Vec<String>,
    /// The jobs leased ahead of it while it waited (reviews only), first
    /// leased first.
    ahead: Vec<Leased>,
    /// The event is written (or found already written).
    told: bool,
}

impl Seen {
    /// The reasons the event names: the holds, then what was leased first.
    fn all_reasons(&self) -> Vec<String> {
        let mut out = self.reasons.clone();
        add_reason(&mut out, leased_first(&self.ahead));
        out
    }
}

/// The daemon's memory of the owner's queued jobs, tick to tick.
#[derive(Debug, Default)]
pub(crate) struct OwnerWaits {
    jobs: BTreeMap<String, Seen>,
}

impl OwnerWaits {
    /// One tick after dispatch: `reasons` are the tick's ([`tick_reasons`]).
    /// Writes the event for every owner-driven job queued past the line not
    /// yet told, and returns the lines to log.
    #[cfg(test)]
    pub(crate) fn tend(
        &mut self,
        s: &hive_store::Store,
        reasons: &[String],
        now: &str,
        actor: &str,
    ) -> Vec<String> {
        self.tend_with(s, reasons, &Ahead::default(), now, actor)
    }

    /// [`Self::tend`] with what the tick put ahead of a waiting review
    /// ([`Ahead`]): the jobs it leased first and the disk headroom join the
    /// review's reasons, so its wait says what went before it.
    pub(crate) fn tend_with(
        &mut self,
        s: &hive_store::Store,
        reasons: &[String],
        ahead: &Ahead,
        now: &str,
        actor: &str,
    ) -> Vec<String> {
        let Ok(conn) = s.conn() else {
            return Vec::new();
        };
        let rows = crate::util::must_rows(
            &conn,
            &format!(
                "SELECT j.* FROM jobs j JOIN projects p ON p.slug = j.project WHERE j.status = 'queued' AND p.status = 'active' AND {} ORDER BY j.id",
                crate::jobs::OWNER_DRIVEN_SQL
            ),
            &[],
        );
        let mut lines = Vec::new();
        let mut keep = BTreeMap::new();
        for job in &rows {
            let id = job.str("id");
            let since = crate::jobs::job_wait_since(&*conn, job);
            let mut seen = self.jobs.remove(&id).unwrap_or_default();
            if seen.since != since {
                seen = Seen {
                    since: since.clone(),
                    ..Seen::default()
                };
            }
            for r in reasons {
                if !seen.reasons.contains(r) {
                    seen.reasons.push(r.clone());
                }
            }
            if job.str("kind") == crate::review::KIND {
                if ahead.disk {
                    add_reason(&mut seen.reasons, DISK_HEADROOM.into());
                }
                for leased in &ahead.leased {
                    if leased.job != id && !seen.ahead.contains(leased) {
                        seen.ahead.push(leased.clone());
                    }
                }
            }
            let waited = crate::jobs::job_wait_seconds(&*conn, job, now);
            if !seen.told && waited >= hive_tasks::OWNER_WAIT_MINUTES * 60 {
                seen.told = true;
                if !told_already(&conn, &id, &since) {
                    lines.push(tell(
                        &conn,
                        job,
                        &since,
                        waited,
                        &seen.all_reasons(),
                        now,
                        actor,
                    ));
                }
            }
            keep.insert(id, seen);
        }
        self.jobs = keep;
        lines
    }
}

/// Whether an event already names this wait (the daemon restarted since).
fn told_already(q: &(impl crate::util::Querier + ?Sized), job: &str, since: &str) -> bool {
    crate::util::query_one(
        q,
        "SELECT COUNT(*) AS n FROM events WHERE action = ? AND subject = ? AND at >= ?",
        &[&hive_tasks::OWNER_WAIT_EVENT, &job, &since],
    )
    .ok()
    .flatten()
    .is_some_and(|r| r.int("n") > 0)
}

fn tell(
    q: &(impl crate::util::Querier + ?Sized),
    job: &Row,
    since: &str,
    waited: i64,
    reasons: &[String],
    now: &str,
    actor: &str,
) -> String {
    let id = job.str("id");
    crate::util::event_at(
        q,
        now,
        actor,
        hive_tasks::OWNER_WAIT_EVENT,
        &id,
        json!({"since": since, "waited_s": waited, "reasons": reasons, "project": job.str("project")}),
    );
    let wait = hive_tasks::OwnerWait {
        job: id.clone(),
        project: job.str("project"),
        title: job.str("title"),
        since: since.to_string(),
        at: now.to_string(),
        reasons: reasons.to_vec(),
    };
    format!("owner job {id} ({}) {}", wait.project, wait.line(now))
}

/// The owner's open waits as `hive status` lists them, longest first:
/// "proj job-x (Owner step) waiting since 04:40 UTC: lanes full, load 27 > 24".
pub fn status_lines(s: &hive_store::Store) -> Vec<String> {
    let now = s.now();
    hive_tasks::open_owner_waits(s)
        .unwrap_or_default()
        .iter()
        .map(|w| {
            let title = if w.title.is_empty() {
                String::new()
            } else {
                format!(" ({})", crate::util::trunc(&w.title, 60))
            };
            format!("{} {}{title} {}", w.project, w.job, w.line(&now))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{count, new_bare_repo, new_store};

    fn at(minutes: f64) -> String {
        let t0 = crate::util::must_iso(crate::test_helpers::TEST_NOW);
        (t0 + chrono::Duration::seconds((minutes * 60.0) as i64))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string()
    }

    fn load(l: f64) -> Hold {
        Hold::new(
            &LOAD,
            format!("machine saturated: load {l:.1} on 12 CPUs is over the line 24"),
        )
    }

    fn owner_job(s: &hive_store::Store, title: &str, source: &str) -> String {
        let conn = s.conn().unwrap();
        let id = format!("job-{}", title.to_lowercase().replace(' ', "-"));
        crate::util::exec1(
            &conn,
            "INSERT INTO jobs (id, project, kind, title, source, status, created_at) VALUES (?, 'proj', 'task', ?, ?, 'queued', ?)",
            &[&id, &title, &source, &crate::test_helpers::TEST_NOW],
        );
        id
    }

    fn waits_of(s: &hive_store::Store) -> i32 {
        count(
            s,
            "SELECT COUNT(*) AS n FROM events WHERE action = 'job.owner-wait'",
            &[],
        )
    }

    #[test]
    fn holds_read_short() {
        assert_eq!(short_reason(&load(27.3)), "load 27 > 24");
        assert_eq!(
            short_reason(&Hold::new(
                &MEMORY,
                "machine saturated: 300 MB of memory free, under the floor 1024"
            )),
            "memory 300 MB < 1024 MB"
        );
        assert_eq!(
            short_reason(&Hold::new(
                &FLEET_FULL,
                "every executor slot is busy with the fleet's own work"
            )),
            "lanes full"
        );
        assert_eq!(
            tick_reasons(
                &[Hold::new(&FLEET_FULL, "x"), load(28.9), load(27.0)],
                true,
                "ignored"
            ),
            vec![
                "lanes full",
                "load 29 > 24",
                "load 27 > 24",
                "daily cap reached"
            ]
        );
        assert_eq!(
            tick_reasons(&[], false, "every executor slot is busy"),
            vec!["lanes full"]
        );
        assert!(tick_reasons(&[], false, "no queued job").is_empty());
    }

    #[test]
    fn an_owner_job_queued_over_ten_minutes_gets_one_event_naming_its_holds() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        new_bare_repo(&s);
        let job = owner_job(&s, "Owner step", "task:TASK-055");
        // The fleet's own work is not the owner's: never named.
        owner_job(&s, "Fleet step", "lead-role:proj:reflect");
        crate::util::exec1(
            &s.conn().unwrap(),
            "UPDATE jobs SET kind = 'improve' WHERE id = 'job-fleet-step'",
            &[],
        );
        let mut waits = OwnerWaits::default();
        let lanes = tick_reasons(&[Hold::new(&FLEET_FULL, "busy")], false, "");
        assert!(waits.tend(&s, &lanes, &at(2.0), "daemon").is_empty());
        let both = tick_reasons(&[load(27.3)], false, "");
        assert!(waits.tend(&s, &both, &at(6.0), "daemon").is_empty());
        assert_eq!(waits_of(&s), 0);
        // Past ten minutes: one event, every hold seen while it waited.
        let lines = waits.tend(
            &s,
            &tick_reasons(&[load(27.3)], false, ""),
            &at(11.0),
            "daemon",
        );
        assert_eq!(
            lines,
            vec![format!(
                "owner job {job} (proj) waiting since 10:00 UTC: lanes full, load 27 > 24"
            )]
        );
        assert_eq!(waits_of(&s), 1);
        let detail = crate::util::query_one(
            &s.conn().unwrap(),
            "SELECT subject, detail FROM events WHERE action = 'job.owner-wait'",
            &[],
        )
        .unwrap()
        .unwrap();
        assert_eq!(detail.str("subject"), job);
        let d: serde_json::Value = serde_json::from_str(&detail.str("detail")).unwrap();
        assert_eq!(d["reasons"], json!(["lanes full", "load 27 > 24"]));
        assert_eq!(d["waited_s"], json!(660));
        // Still waiting: not said again, nor by a restarted daemon.
        assert!(waits.tend(&s, &both, &at(20.0), "daemon").is_empty());
        let mut restarted = OwnerWaits::default();
        assert!(restarted.tend(&s, &both, &at(25.0), "daemon").is_empty());
        assert_eq!(waits_of(&s), 1);
        // `hive status` lists it while the job waits.
        let lines = status_lines(&s);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].starts_with(&format!(
                "proj {job} (Owner step) waiting since 10:00 UTC: lanes full, load 27 > 24"
            )),
            "{lines:?}"
        );
        // Queued again later: a new wait, its own event.
        crate::util::event_at(
            &s.conn().unwrap(),
            &at(30.0),
            "daemon",
            "job.lease",
            &job,
            json!({}),
        );
        assert!(status_lines(&s).is_empty());
        crate::util::event_at(
            &s.conn().unwrap(),
            &at(40.0),
            "daemon",
            "job.requeue",
            &job,
            json!({}),
        );
        assert!(waits.tend(&s, &[], &at(45.0), "daemon").is_empty());
        let lines = waits.tend(&s, &tick_reasons(&[], true, ""), &at(51.0), "daemon");
        assert_eq!(
            lines,
            vec![format!(
                "owner job {job} (proj) waiting since 10:40 UTC: daily cap reached"
            )]
        );
        assert_eq!(waits_of(&s), 2);
    }

    /// TASK-186: a review that waits names what went before it - the jobs
    /// the ticks leased on this machine (the kept slot marked), the disk
    /// headroom - where the event of 27 November had an empty list. The
    /// owner's other jobs keep their holds alone.
    #[test]
    fn a_waiting_review_names_what_is_ahead_of_it() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        new_bare_repo(&s);
        let task = owner_job(&s, "Owner step", "task:TASK-1");
        let review = "job-review".to_string();
        crate::util::exec1(
            &s.conn().unwrap(),
            "INSERT INTO jobs (id, project, kind, title, source, status, created_at) VALUES (?, 'proj', 'review', 'Review PR #307', 'review:proj:307', 'queued', ?)",
            &[&review, &crate::test_helpers::TEST_NOW],
        );
        let leased = |job: &str, kind: &str, kept_slot: bool| Leased {
            job: job.into(),
            kind: kind.into(),
            kept_slot,
        };
        let mut waits = OwnerWaits::default();
        let first = Ahead {
            leased: vec![
                leased("job-w1", "task", false),
                leased("job-lead", crate::lead::LEAD_KIND, true),
            ],
            disk: false,
        };
        assert!(
            waits
                .tend_with(&s, &[], &first, &at(2.0), "daemon")
                .is_empty()
        );
        let second = Ahead {
            leased: vec![
                leased("job-w2", "task", false),
                leased("job-w1", "task", false),
                leased("job-w3", "improve", false),
                // Its own lease is not ahead of it.
                leased(&review, "review", false),
            ],
            disk: true,
        };
        let lines = waits.tend_with(&s, &[], &second, &at(11.0), "daemon");
        assert_eq!(lines.len(), 2, "{lines:?}");
        let reasons_of = |job: &str| {
            let row = crate::util::query_one(
                &s.conn().unwrap(),
                "SELECT detail FROM events WHERE action = 'job.owner-wait' AND subject = ?",
                &[&job],
            )
            .unwrap()
            .expect("one job.owner-wait event");
            let d: serde_json::Value = serde_json::from_str(&row.str("detail")).unwrap();
            d["reasons"].clone()
        };
        assert_eq!(
            reasons_of(&review),
            json!([
                "disk headroom",
                "leased first: job-w1 (task), job-lead (lead sitting, kept slot), job-w2 (task) and 1 more"
            ])
        );
        assert_eq!(reasons_of(&task), json!([]));
        assert!(
            lines
                .iter()
                .any(|l| l.contains(&review)
                    && l.contains("disk headroom, leased first: job-w1 (task)")),
            "{lines:?}"
        );
    }
}
