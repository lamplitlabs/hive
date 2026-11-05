//! An owner-driven job that has waited in the queue long enough to be worth
//! a sentence (TASK-067 part c).
//!
//! On 2026-11-03 the owner's TASK-055 was queued at 04:40 and started at
//! 15:18: the lanes were full of the fleet's work, the load sat at 26-30
//! over a line of 24 and a restart window closed slots in between, and the
//! task page said "waiting for a free agent" all day. The daemon (pulse-runner
//! `owner_wait`) now notes on each tick what held every queued owner-driven
//! job back, and once one has waited [`OWNER_WAIT_MINUTES`] it writes one
//! [`OWNER_WAIT_EVENT`] on the job naming the holds it saw. This module reads
//! those events back: `pulse status` lists the open ones and a task's wait
//! line names the reasons. A wait is open while its job is still queued and
//! nothing queued or leased it again after the event; the next wait of the
//! same job gets its own event.

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// The event the daemon writes on a job once it has waited too long.
pub const OWNER_WAIT_EVENT: &str = "job.owner-wait";

/// How long an owner-driven job waits in the queue before the daemon says why.
pub const OWNER_WAIT_MINUTES: i64 = 10;

/// What the daemon writes in the event's detail.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerWaitDetail {
    /// When the job was queued (its last `job.add`, `job.queued` or
    /// `job.requeue`, else its creation).
    pub since: String,
    /// How long it had waited when the event was written, in seconds.
    #[serde(default)]
    pub waited_s: i64,
    /// The holds seen while it waited, short: "lanes full", "load 27 > 24".
    #[serde(default)]
    pub reasons: Vec<String>,
}

/// One open wait: the job still queued, the event that named it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct OwnerWait {
    pub job: String,
    pub project: String,
    pub title: String,
    /// When the job was queued.
    pub since: String,
    /// When the daemon wrote the event.
    pub at: String,
    pub reasons: Vec<String>,
}

impl OwnerWait {
    /// "waiting since 04:40 UTC: lanes full, load 27 > 24"; a wait that
    /// began on another day than `now` names the day too.
    pub fn line(&self, now: &str) -> String {
        let since = since_clock(&self.since, now);
        if self.reasons.is_empty() {
            format!("waiting since {since}")
        } else {
            format!("waiting since {since}: {}", self.reasons.join(", "))
        }
    }
}

/// "04:40 UTC", or "2026-11-03 04:40 UTC" when `since` is not on `now`'s day.
pub fn since_clock(since: &str, now: &str) -> String {
    if since.len() < 16 {
        return since.to_string();
    }
    let clock = &since[11..16];
    if now.len() >= 10 && now[..10] == since[..10] {
        format!("{clock} UTC")
    } else {
        format!("{} {clock} UTC", &since[..10])
    }
}

/// Every open owner wait, the longest first.
pub fn open_owner_waits(s: &pulse_store::Store) -> Result<Vec<OwnerWait>> {
    let conn = s.conn()?;
    let mut stmt = conn.prepare(
        "SELECT e.at, e.subject, e.detail, j.project, j.title FROM events e
         JOIN jobs j ON j.id = e.subject
         WHERE e.action = ?1 AND j.status = 'queued'
           AND NOT EXISTS (SELECT 1 FROM events x WHERE x.subject = e.subject AND x.seq > e.seq
                AND x.action IN (?1, 'job.add', 'job.queued', 'job.requeue', 'job.lease'))
         ORDER BY e.seq",
    )?;
    let rows = stmt.query_map([OWNER_WAIT_EVENT], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (at, job, detail, project, title) = row?;
        let d: OwnerWaitDetail = serde_json::from_str(&detail).unwrap_or_default();
        out.push(OwnerWait {
            job,
            project,
            title,
            since: if d.since.is_empty() {
                at.clone()
            } else {
                d.since
            },
            at,
            reasons: d.reasons,
        });
    }
    out.sort_by(|a, b| a.since.cmp(&b.since).then_with(|| a.job.cmp(&b.job)));
    Ok(out)
}
