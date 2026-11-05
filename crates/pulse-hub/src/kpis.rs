//! Daily KPIs for the Home page's "Over time" panel: what the fleet got
//! done, what it did on its own, and what it spent, per UTC day.
//!
//! `GET /api/v1/kpis?days=N[&project=SLUG]` answers with one row per day
//! over the last N days ending today (inclusive), zero-filled so a chart
//! can draw it without gaps: harness sessions with their tokens and cost
//! (velocity and spend), jobs finished done or failed, landings
//! (`land.landed` events and `land.merged` pull requests) and tasks moved
//! to done (`task.status` events with `to = done`).
//!
//! The autonomy counters say what the fleet did without a human and how
//! often it needed one: pull-request reviews an agent finished with a
//! verdict (`review.merged`, `review.changes`, `review.owner`) and the
//! share it merged itself (`review.merged` not `by: owner`); `finding`
//! posts by agents (bugs, regressions, evidence; the daemon's notices and
//! the owner's posts excluded); Dependabot alerts first seen that day
//! (each alert number once, from `security.alerts`); security fix jobs
//! finished done (`source` under `security:`); and the posts flagged
//! `for_owner` - the questions, hold stops and "your call" findings that
//! waited on the owner. Each source is one `GROUP BY` day query; rows
//! older than the window or dated in the future are dropped.

use crate::httpx::{Request, ResponseWriter};
use crate::server::Handler;
use rusqlite::params;
use serde::Serialize;
use std::collections::BTreeMap;

pub const KPIS_DEFAULT_DAYS: i64 = 30;
pub const KPIS_MAX_DAYS: i64 = 90;

/// One UTC day of fleet output, autonomy and spend.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct KpiDay {
    pub day: String,
    pub sessions: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cost: f64,
    pub jobs_done: i64,
    pub jobs_failed: i64,
    pub landings: i64,
    pub tasks_done: i64,
    /// Pull-request reviews an agent finished with a verdict.
    pub reviews: i64,
    /// Of those, the pull requests the agent merged itself.
    pub reviews_merged: i64,
    /// Board findings agents posted: bugs, regressions, evidence.
    pub findings: i64,
    /// Dependabot alerts first seen that day.
    pub vulns_found: i64,
    /// Security fix jobs finished done.
    pub vuln_fixes: i64,
    /// Posts flagged for the owner: the times a human was needed.
    pub owner_asks: i64,
}

/// The sums over a window of [`KpiDay`]s.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct KpiTotals {
    pub sessions: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cost: f64,
    pub jobs_done: i64,
    pub jobs_failed: i64,
    pub landings: i64,
    pub tasks_done: i64,
    pub reviews: i64,
    pub reviews_merged: i64,
    pub findings: i64,
    pub vulns_found: i64,
    pub vuln_fixes: i64,
    pub owner_asks: i64,
}

/// The `/api/v1/kpis` payload: `series` has exactly `days` entries, oldest
/// first; `priced` says whether any session recorded a cost, so the UI can
/// hide the money when the provider prices nothing.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct KpisReport {
    pub now: String,
    pub days: i64,
    pub from: String,
    pub to: String,
    pub project: String,
    pub series: Vec<KpiDay>,
    pub totals: KpiTotals,
    pub priced: bool,
}

const SESSIONS_SQL: &str = "SELECT substr(started_at, 1, 10), COUNT(*), \
    COALESCE(SUM(prompt_tokens), 0), COALESCE(SUM(completion_tokens), 0), COALESCE(SUM(cost), 0) \
    FROM harness_sessions WHERE started_at >= ?1 AND (?2 = '' OR project = ?2) GROUP BY 1";

const JOBS_SQL: &str = "SELECT substr(finished_at, 1, 10), \
    COALESCE(SUM(status = 'done'), 0), COALESCE(SUM(status = 'failed'), 0) \
    FROM jobs WHERE status IN ('done', 'failed') AND finished_at >= ?1 \
    AND (?2 = '' OR project = ?2) GROUP BY 1";

// A landing is a branch merged on this machine (`land.landed`) or a pull
// request merged on the forge (`land.merged`); a branch takes one road or
// the other, never both.
const LANDINGS_SQL: &str = "SELECT substr(at, 1, 10), COUNT(*) FROM events \
    WHERE action IN ('land.landed', 'land.merged') AND at >= ?1 \
    AND (?2 = '' OR (json_valid(detail) AND json_extract(detail, '$.project') = ?2)) GROUP BY 1";

// A review ends with one verdict event: merged, changes asked for, or
// handed to the owner. `review.merged` with `by: owner` is the owner's
// Merge click honoured by the daemon, so it counts as a review but not as
// one merged on its own.
const REVIEWS_SQL: &str = "SELECT substr(at, 1, 10), COUNT(*), \
    COALESCE(SUM(action = 'review.merged' AND COALESCE(json_extract(detail, '$.by'), '') != 'owner'), 0) \
    FROM events WHERE action IN ('review.merged', 'review.changes', 'review.owner') AND at >= ?1 \
    AND json_valid(detail) AND (?2 = '' OR json_extract(detail, '$.project') = ?2) GROUP BY 1";

// Findings by agents: the daemon's own notices (restarts, disk, recipe)
// and the owner's posts are not the fleet's discoveries.
const FINDINGS_SQL: &str = "SELECT substr(date, 1, 10), COUNT(*) FROM posts \
    WHERE kind = 'finding' AND date >= ?1 AND agent NOT IN ('daemon', 'human') \
    AND (?2 = '' OR project = ?2) GROUP BY 1";

// Every `security.alerts` event lists the open alert numbers of one
// project; an alert is found on the first day it appears.
const VULNS_FOUND_SQL: &str = "SELECT substr(first_at, 1, 10), COUNT(*) FROM ( \
    SELECT e.subject, n.value AS number, MIN(e.at) AS first_at \
    FROM events e, json_each(json_extract(e.detail, '$.numbers')) n \
    WHERE e.action = 'security.alerts' AND json_valid(e.detail) AND (?2 = '' OR e.subject = ?2) \
    GROUP BY 1, 2) WHERE first_at >= ?1 GROUP BY 1";

const VULN_FIXES_SQL: &str = "SELECT substr(finished_at, 1, 10), COUNT(*) FROM jobs \
    WHERE status = 'done' AND source LIKE 'security:%' AND finished_at >= ?1 \
    AND (?2 = '' OR project = ?2) GROUP BY 1";

// Posts flagged `for_owner`: questions, hold stops and findings that
// waited on a human, whoever posted them.
const OWNER_ASKS_SQL: &str = "SELECT substr(date, 1, 10), COUNT(*) FROM posts \
    WHERE date >= ?1 AND json_valid(extra) AND json_extract(extra, '$.for_owner') = 1 \
    AND (?2 = '' OR project = ?2) GROUP BY 1";

const TASKS_DONE_SQL: &str = "SELECT substr(at, 1, 10), COUNT(*) FROM events \
    WHERE action = 'task.status' AND at >= ?1 \
    AND json_valid(detail) AND json_extract(detail, '$.to') = 'done' \
    AND (?2 = '' OR json_extract(detail, '$.project') = ?2) GROUP BY 1";

/// The one-column autonomy counters, each filled from its own query.
#[derive(Clone, Copy)]
enum Field {
    Findings,
    VulnsFound,
    VulnFixes,
    OwnerAsks,
}

/// The daily KPIs over the `days` UTC days ending on `now`'s day, for one
/// project or (empty `project`) the whole fleet.
pub fn daily_kpis(
    store: &pulse_store::Store,
    now: &str,
    days: i64,
    project: &str,
) -> anyhow::Result<KpisReport> {
    let days = days.clamp(1, KPIS_MAX_DAYS);
    let today = now.get(..10).unwrap_or_default();
    let to = chrono::NaiveDate::parse_from_str(today, "%Y-%m-%d")
        .map_err(|err| anyhow::anyhow!("the store clock {now:?} is not a date: {err}"))?;
    let from = to - chrono::Duration::days(days - 1);
    let mut series: BTreeMap<String, KpiDay> = (0..days)
        .map(|offset| {
            let day = (from + chrono::Duration::days(offset))
                .format("%Y-%m-%d")
                .to_string();
            let row = KpiDay {
                day: day.clone(),
                ..Default::default()
            };
            (day, row)
        })
        .collect();
    let from_day = from.format("%Y-%m-%d").to_string();
    let conn = store.conn()?;

    let mut stmt = conn.prepare(SESSIONS_SQL)?;
    let rows = stmt.query_map(params![from_day, project], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, f64>(4)?,
        ))
    })?;
    for row in rows {
        let (day, sessions, prompt, completion, cost) = row?;
        if let Some(entry) = series.get_mut(&day) {
            entry.sessions = sessions;
            entry.prompt_tokens = prompt;
            entry.completion_tokens = completion;
            entry.cost = cost;
        }
    }

    let mut stmt = conn.prepare(JOBS_SQL)?;
    let rows = stmt.query_map(params![from_day, project], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (day, done, failed) = row?;
        if let Some(entry) = series.get_mut(&day) {
            entry.jobs_done = done;
            entry.jobs_failed = failed;
        }
    }

    let mut stmt = conn.prepare(LANDINGS_SQL)?;
    let rows = stmt.query_map(params![from_day, project], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (day, landings) = row?;
        if let Some(entry) = series.get_mut(&day) {
            entry.landings = landings;
        }
    }

    let mut stmt = conn.prepare(TASKS_DONE_SQL)?;
    let rows = stmt.query_map(params![from_day, project], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (day, tasks_done) = row?;
        if let Some(entry) = series.get_mut(&day) {
            entry.tasks_done = tasks_done;
        }
    }

    let mut stmt = conn.prepare(REVIEWS_SQL)?;
    let rows = stmt.query_map(params![from_day, project], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (day, reviews, merged) = row?;
        if let Some(entry) = series.get_mut(&day) {
            entry.reviews = reviews;
            entry.reviews_merged = merged;
        }
    }

    for (sql, field) in [
        (FINDINGS_SQL, Field::Findings),
        (VULNS_FOUND_SQL, Field::VulnsFound),
        (VULN_FIXES_SQL, Field::VulnFixes),
        (OWNER_ASKS_SQL, Field::OwnerAsks),
    ] {
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(params![from_day, project], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (day, n) = row?;
            if let Some(entry) = series.get_mut(&day) {
                match field {
                    Field::Findings => entry.findings = n,
                    Field::VulnsFound => entry.vulns_found = n,
                    Field::VulnFixes => entry.vuln_fixes = n,
                    Field::OwnerAsks => entry.owner_asks = n,
                }
            }
        }
    }

    let series: Vec<KpiDay> = series.into_values().collect();
    let mut totals = KpiTotals::default();
    for day in &series {
        totals.sessions += day.sessions;
        totals.prompt_tokens += day.prompt_tokens;
        totals.completion_tokens += day.completion_tokens;
        totals.cost += day.cost;
        totals.jobs_done += day.jobs_done;
        totals.jobs_failed += day.jobs_failed;
        totals.landings += day.landings;
        totals.tasks_done += day.tasks_done;
        totals.reviews += day.reviews;
        totals.reviews_merged += day.reviews_merged;
        totals.findings += day.findings;
        totals.vulns_found += day.vulns_found;
        totals.vuln_fixes += day.vuln_fixes;
        totals.owner_asks += day.owner_asks;
    }
    let priced = totals.cost > 0.0;
    Ok(KpisReport {
        now: now.to_string(),
        days,
        from: from_day,
        to: today.to_string(),
        project: project.to_string(),
        series,
        totals,
        priced,
    })
}

impl Handler {
    pub(crate) fn addition_kpis(&self, w: &mut ResponseWriter, r: &Request, parts: Vec<String>) {
        if parts.len() != 1 {
            crate::addition_not_found(w);
            return;
        }
        if !crate::addition_method(w, r, vec!["GET".to_string()])
            || !self.addition_available(
                w,
                r,
                vec![
                    "harness_sessions".to_string(),
                    "jobs".to_string(),
                    "events".to_string(),
                    "posts".to_string(),
                ],
            )
        {
            return;
        }
        let Some(store) = self.store.as_deref() else {
            crate::addition_failure(w, anyhow::anyhow!("store is not available"));
            return;
        };
        let project = self.addition_project(r);
        if !self.valid_code_delivery_project(w, project.clone()) {
            return;
        }
        let (days, ok) = crate::code_deliveries::code_delivery_positive_int(
            w,
            r.query("days"),
            "days".to_string(),
            KPIS_DEFAULT_DAYS,
            KPIS_MAX_DAYS,
        );
        if !ok {
            return;
        }
        match daily_kpis(store, &store.now(), days, &project) {
            Ok(report) => match serde_json::to_value(&report) {
                Ok(payload) => crate::write_json(w, 200, payload),
                Err(err) => crate::addition_failure(w, anyhow::anyhow!(err)),
            },
            Err(err) => crate::addition_failure(w, err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn session(s: &pulse_store::Store, id: &str, project: &str, started_at: &str) {
        s.begin_harness_session(pulse_store::HarnessSession {
            id: id.to_string(),
            project: project.to_string(),
            started_at: started_at.to_string(),
            ..Default::default()
        })
        .expect("begin session");
    }

    fn usage(s: &pulse_store::Store, id: &str, prompt: i64, completion: i64, cost: f64) {
        s.conn()
            .unwrap()
            .execute(
                "UPDATE harness_sessions SET prompt_tokens = ?, completion_tokens = ?, cost = ? WHERE id = ?",
                params![prompt, completion, cost, id],
            )
            .expect("set usage");
    }

    fn job(s: &pulse_store::Store, id: &str, project: &str, status: &str, finished_at: &str) {
        s.conn()
            .unwrap()
            .execute(
                "INSERT INTO jobs (id, project, kind, title, created_at, status, finished_at) \
                 VALUES (?, ?, 'task', 'T', '2026-09-01T00:00:00Z', ?, ?)",
                params![id, project, status, finished_at],
            )
            .expect("insert job");
    }

    fn event(s: &pulse_store::Store, at: &str, action: &str, detail: &str) {
        event_on(s, at, action, "x", detail);
    }

    fn event_on(s: &pulse_store::Store, at: &str, action: &str, subject: &str, detail: &str) {
        s.conn()
            .unwrap()
            .execute(
                "INSERT INTO events (at, actor, action, subject, detail) VALUES (?, 'test', ?, ?, ?)",
                params![at, action, subject, detail],
            )
            .expect("insert event");
    }

    fn fix_job(
        s: &pulse_store::Store,
        id: &str,
        project: &str,
        source: &str,
        status: &str,
        finished_at: &str,
    ) {
        s.conn()
            .unwrap()
            .execute(
                "INSERT INTO jobs (id, project, kind, title, source, created_at, status, finished_at) \
                 VALUES (?, ?, 'task', 'T', ?, '2026-09-01T00:00:00Z', ?, ?)",
                params![id, project, source, status, finished_at],
            )
            .expect("insert job");
    }

    fn post(
        s: &pulse_store::Store,
        id: &str,
        project: &str,
        kind: &str,
        agent: &str,
        date: &str,
        extra: &str,
    ) {
        s.conn()
            .unwrap()
            .execute(
                "INSERT INTO posts (id, project, area, thread, kind, agent, date, summary, body, extra, created_at) \
                 VALUES (?, ?, 'a', 't', ?, ?, ?, 'S', 'B', ?, ?)",
                params![id, project, kind, agent, date, extra, date],
            )
            .expect("insert post");
    }

    fn autonomy_fields(day: &Value) -> (i64, i64, i64, i64, i64, i64) {
        (
            day["reviews"].as_i64().unwrap(),
            day["reviews_merged"].as_i64().unwrap(),
            day["findings"].as_i64().unwrap(),
            day["vulns_found"].as_i64().unwrap(),
            day["vuln_fixes"].as_i64().unwrap(),
            day["owner_asks"].as_i64().unwrap(),
        )
    }

    fn get(s: &pulse_store::Store, path: &str) -> (u16, serde_json::Map<String, Value>, String) {
        let (status, result, raw) = crate::test_helpers_http::request(
            crate::test_helpers_http::handler(s),
            "GET",
            path,
            "",
            &[],
        );
        (status, result.into_iter().collect(), raw)
    }

    fn day_fields(day: &Value) -> (i64, i64, i64, f64, i64, i64, i64, i64) {
        (
            day["sessions"].as_i64().unwrap(),
            day["prompt_tokens"].as_i64().unwrap(),
            day["completion_tokens"].as_i64().unwrap(),
            day["cost"].as_f64().unwrap(),
            day["jobs_done"].as_i64().unwrap(),
            day["jobs_failed"].as_i64().unwrap(),
            day["landings"].as_i64().unwrap(),
            day["tasks_done"].as_i64().unwrap(),
        )
    }

    #[test]
    fn kpis_bucket_per_day_and_zero_fill_the_window() {
        let ts = crate::test_helpers_http::additions_store(false);
        let s = &ts.store;
        session(s, "s-a", "demo", "2026-09-05T10:00:00Z");
        usage(s, "s-a", 100, 10, 0.0);
        session(s, "s-b", "demo", "2026-09-05T11:00:00Z");
        usage(s, "s-b", 200, 20, 0.0);
        session(s, "s-c", "demo", "2026-09-06T01:00:00Z");
        session(s, "s-old", "demo", "2026-09-01T01:00:00Z");
        usage(s, "s-old", 999, 99, 0.0);
        job(s, "j-1", "demo", "done", "2026-09-06T02:00:00Z");
        job(s, "j-2", "demo", "done", "2026-09-06T03:00:00Z");
        job(s, "j-3", "demo", "failed", "2026-09-06T04:00:00Z");
        job(s, "j-4", "demo", "queued", "");
        job(s, "j-old", "demo", "done", "2026-09-01T04:00:00Z");
        event(
            s,
            "2026-09-05T12:00:00Z",
            "land.landed",
            r#"{"project":"demo","sha":"abc"}"#,
        );
        event(
            s,
            "2026-09-06T12:00:00Z",
            "land.landed",
            r#"{"project":"demo","sha":"def"}"#,
        );
        event(
            s,
            "2026-09-06T13:00:00Z",
            "land.landed",
            r#"{"project":"demo","sha":"ghi"}"#,
        );
        event(
            s,
            "2026-09-06T05:00:00Z",
            "task.status",
            r#"{"project":"demo","from":"review","to":"done"}"#,
        );
        event(
            s,
            "2026-09-06T05:30:00Z",
            "task.status",
            r#"{"project":"demo","from":"planned","to":"in_progress"}"#,
        );
        // A future-dated row never lands in the window.
        event(
            s,
            "2026-09-07T05:00:00Z",
            "land.landed",
            r#"{"project":"demo","sha":"future"}"#,
        );
        // A pull request merged on the forge is a landing too.
        event(
            s,
            "2026-09-05T14:00:00Z",
            "land.merged",
            r#"{"project":"demo","number":7,"head":"pulse/task-TASK-001"}"#,
        );

        let (status, body, raw) = get(s, "/api/v1/kpis?days=3");
        assert_eq!(status, 200, "{raw}");
        assert_eq!(body["now"], "2026-09-06T12:00:00Z");
        assert_eq!(body["days"], 3);
        assert_eq!(body["from"], "2026-09-04");
        assert_eq!(body["to"], "2026-09-06");
        assert_eq!(body["project"], "");
        assert_eq!(body["priced"], false);
        let series = body["series"].as_array().unwrap();
        assert_eq!(series.len(), 3, "{raw}");
        let days: Vec<&str> = series.iter().map(|d| d["day"].as_str().unwrap()).collect();
        assert_eq!(days, ["2026-09-04", "2026-09-05", "2026-09-06"]);
        assert_eq!(day_fields(&series[0]), (0, 0, 0, 0.0, 0, 0, 0, 0));
        assert_eq!(day_fields(&series[1]), (2, 300, 30, 0.0, 0, 0, 2, 0));
        assert_eq!(day_fields(&series[2]), (1, 0, 0, 0.0, 2, 1, 2, 1));
        assert_eq!(day_fields(&body["totals"]), (3, 300, 30, 0.0, 2, 1, 4, 1));
        for day in series {
            assert_eq!(autonomy_fields(day), (0, 0, 0, 0, 0, 0), "{raw}");
        }
        assert_eq!(autonomy_fields(&body["totals"]), (0, 0, 0, 0, 0, 0));
    }

    #[test]
    fn kpis_count_the_work_done_without_the_owner_and_the_asks() {
        let ts = crate::test_helpers_http::additions_store(false);
        let s = &ts.store;
        crate::test_helpers_http::additions_project(s, "demo");

        // Reviews: three verdicts on 09-05 (two merged by agents, one by the
        // owner's click), one handed to the owner on 09-06, one changes
        // request on 09-06; a queued review is not a verdict.
        event_on(
            s,
            "2026-09-05T10:00:00Z",
            "review.queued",
            "demo",
            r#"{"project":"demo","number":1}"#,
        );
        event_on(
            s,
            "2026-09-05T11:00:00Z",
            "review.merged",
            "demo",
            r#"{"project":"demo","number":1,"by":"fjord-1"}"#,
        );
        event_on(
            s,
            "2026-09-05T11:30:00Z",
            "review.merged",
            "demo",
            r#"{"project":"demo","number":2,"by":"sable-2"}"#,
        );
        event_on(
            s,
            "2026-09-05T12:00:00Z",
            "review.merged",
            "demo",
            r#"{"project":"demo","number":3,"by":"owner"}"#,
        );
        event_on(
            s,
            "2026-09-06T09:00:00Z",
            "review.owner",
            "demo",
            r#"{"project":"demo","number":4,"reasons":[]}"#,
        );
        event_on(
            s,
            "2026-09-06T09:30:00Z",
            "review.changes",
            "demo",
            r#"{"project":"demo","number":5,"note":"n"}"#,
        );
        event_on(
            s,
            "2026-09-06T09:40:00Z",
            "review.merged",
            "other",
            r#"{"project":"other","number":9,"by":"lumen-3"}"#,
        );

        // Findings: agents' count, the daemon's notices and the owner's do not.
        post(
            s,
            "p-f1",
            "demo",
            "finding",
            "nimbus-1",
            "2026-09-05T08:00:00Z",
            "{}",
        );
        post(
            s,
            "p-f2",
            "demo",
            "finding",
            "quartz-2",
            "2026-09-06T08:00:00Z",
            "{}",
        );
        post(
            s,
            "p-f3",
            "demo",
            "finding",
            "daemon",
            "2026-09-06T08:10:00Z",
            "{}",
        );
        post(
            s,
            "p-f4",
            "demo",
            "finding",
            "human",
            "2026-09-06T08:20:00Z",
            "{}",
        );
        post(
            s,
            "p-f5",
            "other",
            "finding",
            "ember-4",
            "2026-09-06T08:30:00Z",
            "{}",
        );
        post(
            s,
            "p-r1",
            "demo",
            "result",
            "nimbus-1",
            "2026-09-06T08:40:00Z",
            "{}",
        );

        // Owner asks: anything flagged for the owner, whoever posted it.
        post(
            s,
            "p-q1",
            "demo",
            "question",
            "daemon",
            "2026-09-05T09:00:00Z",
            r#"{"for_owner":true}"#,
        );
        post(
            s,
            "p-q2",
            "demo",
            "question",
            "nimbus-1",
            "2026-09-06T09:00:00Z",
            r#"{"for_owner":true}"#,
        );
        post(
            s,
            "p-q3",
            "demo",
            "question",
            "nimbus-1",
            "2026-09-06T09:10:00Z",
            r#"{"for_owner":false}"#,
        );
        post(
            s,
            "p-q4",
            "demo",
            "question",
            "nimbus-1",
            "2026-09-06T09:20:00Z",
            "{}",
        );
        post(
            s,
            "p-q5",
            "other",
            "question",
            "daemon",
            "2026-09-06T09:30:00Z",
            r#"{"for_owner":true}"#,
        );
        // The security pass's "your call" finding is both a finding of the
        // daemon (not counted) and an ask (counted).
        post(
            s,
            "p-s1",
            "demo",
            "finding",
            "daemon",
            "2026-09-06T09:40:00Z",
            r#"{"for_owner":true}"#,
        );

        // Vulnerabilities: alert numbers are found on the first day they
        // are listed; later lists repeat them and may add new ones.
        event_on(
            s,
            "2026-09-04T06:00:00Z",
            "security.alerts",
            "demo",
            r#"{"repo":"o/demo","count":2,"numbers":[10,11]}"#,
        );
        event_on(
            s,
            "2026-09-05T06:00:00Z",
            "security.alerts",
            "demo",
            r#"{"repo":"o/demo","count":2,"numbers":[11,12]}"#,
        );
        event_on(
            s,
            "2026-09-06T06:00:00Z",
            "security.alerts",
            "demo",
            r#"{"repo":"o/demo","count":1,"numbers":[12]}"#,
        );
        event_on(
            s,
            "2026-09-06T06:00:00Z",
            "security.alerts",
            "other",
            r#"{"repo":"o/other","count":1,"numbers":[10]}"#,
        );
        // A fix job is a done job whose source is the security pass's.
        fix_job(
            s,
            "j-fix1",
            "demo",
            "security:demo:package-lock.json:10",
            "done",
            "2026-09-05T20:00:00Z",
        );
        fix_job(
            s,
            "j-fix2",
            "demo",
            "security:demo:package-lock.json:11",
            "done",
            "2026-09-06T20:00:00Z",
        );
        fix_job(
            s,
            "j-fix3",
            "demo",
            "security:demo:uv.lock:12",
            "failed",
            "2026-09-06T20:00:00Z",
        );
        fix_job(
            s,
            "j-plain",
            "demo",
            "task:TASK-001",
            "done",
            "2026-09-06T20:00:00Z",
        );
        fix_job(
            s,
            "j-fix-other",
            "other",
            "security:other:Gemfile.lock:10",
            "done",
            "2026-09-06T20:00:00Z",
        );

        let (status, body, raw) = get(s, "/api/v1/kpis?days=3&project=demo");
        assert_eq!(status, 200, "{raw}");
        let series = body["series"].as_array().unwrap();
        assert_eq!(series.len(), 3, "{raw}");
        assert_eq!(
            autonomy_fields(&series[0]),
            (0, 0, 0, 2, 0, 0),
            "09-04: {raw}"
        );
        assert_eq!(
            autonomy_fields(&series[1]),
            (3, 2, 1, 1, 1, 1),
            "09-05: {raw}"
        );
        assert_eq!(
            autonomy_fields(&series[2]),
            (2, 0, 1, 0, 1, 2),
            "09-06: {raw}"
        );
        assert_eq!(autonomy_fields(&body["totals"]), (5, 2, 2, 3, 2, 3));
        // The security-pass job is a done job like any other.
        assert_eq!(body["totals"]["jobs_done"], 3);

        // The whole fleet adds the other project's review, finding, alert,
        // fix and ask.
        let (status, body, raw) = get(s, "/api/v1/kpis?days=3");
        assert_eq!(status, 200, "{raw}");
        assert_eq!(autonomy_fields(&body["totals"]), (6, 3, 3, 4, 3, 4));

        // A window that starts after the first sighting does not count an
        // alert again.
        let (status, body, raw) = get(s, "/api/v1/kpis?days=1&project=demo");
        assert_eq!(status, 200, "{raw}");
        assert_eq!(autonomy_fields(&body["totals"]), (2, 0, 1, 0, 1, 2));
    }

    #[test]
    fn kpis_report_priced_when_a_session_recorded_cost() {
        let ts = crate::test_helpers_http::additions_store(false);
        let s = &ts.store;
        session(s, "s-free", "demo", "2026-09-05T10:00:00Z");
        session(s, "s-paid", "demo", "2026-09-06T10:00:00Z");
        s.finish_harness_session("s-paid", "done", "ok", "", "", 1.5, "")
            .unwrap();

        let (status, body, raw) = get(s, "/api/v1/kpis?days=2");
        assert_eq!(status, 200, "{raw}");
        assert_eq!(body["priced"], true);
        assert_eq!(body["totals"]["cost"].as_f64().unwrap(), 1.5);
        assert_eq!(body["totals"]["sessions"], 2);
        let series = body["series"].as_array().unwrap();
        assert_eq!(series[0]["cost"].as_f64().unwrap(), 0.0);
        assert_eq!(series[1]["cost"].as_f64().unwrap(), 1.5);
    }

    #[test]
    fn kpis_filter_by_project_and_reject_an_unknown_one() {
        let ts = crate::test_helpers_http::additions_store(false);
        let s = &ts.store;
        crate::test_helpers_http::additions_project(s, "demo");
        session(s, "s-demo", "demo", "2026-09-06T10:00:00Z");
        usage(s, "s-demo", 10, 1, 0.0);
        session(s, "s-other", "other", "2026-09-06T10:00:00Z");
        usage(s, "s-other", 1000, 100, 2.0);
        job(s, "j-demo", "demo", "done", "2026-09-06T02:00:00Z");
        job(s, "j-other", "other", "done", "2026-09-06T02:00:00Z");
        job(s, "j-other-fail", "other", "failed", "2026-09-06T02:00:00Z");
        event(
            s,
            "2026-09-06T12:00:00Z",
            "land.landed",
            r#"{"project":"demo","sha":"abc"}"#,
        );
        event(
            s,
            "2026-09-06T12:00:00Z",
            "land.landed",
            r#"{"project":"other","sha":"def"}"#,
        );
        event(
            s,
            "2026-09-06T05:00:00Z",
            "task.status",
            r#"{"project":"demo","from":"review","to":"done"}"#,
        );
        event(
            s,
            "2026-09-06T05:00:00Z",
            "task.status",
            r#"{"project":"other","from":"review","to":"done"}"#,
        );

        let (status, body, raw) = get(s, "/api/v1/kpis?days=1&project=demo");
        assert_eq!(status, 200, "{raw}");
        assert_eq!(body["project"], "demo");
        assert_eq!(body["priced"], false);
        let series = body["series"].as_array().unwrap();
        assert_eq!(series.len(), 1);
        assert_eq!(series[0]["day"], "2026-09-06");
        assert_eq!(day_fields(&series[0]), (1, 10, 1, 0.0, 1, 0, 1, 1));
        assert_eq!(day_fields(&body["totals"]), (1, 10, 1, 0.0, 1, 0, 1, 1));

        // The whole fleet sees both projects.
        let (status, body, raw) = get(s, "/api/v1/kpis?days=1");
        assert_eq!(status, 200, "{raw}");
        assert_eq!(body["priced"], true);
        assert_eq!(day_fields(&body["totals"]), (2, 1010, 101, 2.0, 2, 1, 2, 2));

        let (status, _, raw) = get(s, "/api/v1/kpis?project=missing");
        assert_eq!(status, 404, "{raw}");
    }

    #[test]
    fn kpis_reject_bad_days_and_non_get_methods() {
        let ts = crate::test_helpers_http::additions_store(false);
        let s = &ts.store;
        for path in [
            "/api/v1/kpis?days=0",
            "/api/v1/kpis?days=91",
            "/api/v1/kpis?days=-3",
            "/api/v1/kpis?days=abc",
        ] {
            let (status, body, raw) = get(s, path);
            assert_eq!(status, 400, "{path}: {raw}");
            assert_eq!(body["error"], "Invalid days", "{path}: {raw}");
        }
        let (status, _, raw) = crate::test_helpers_http::request(
            crate::test_helpers_http::handler(s),
            "POST",
            "/api/v1/kpis",
            "{}",
            &[],
        );
        assert_eq!(status, 405, "{raw}");
        let (status, _, raw) = get(s, "/api/v1/kpis/extra");
        assert_eq!(status, 404, "{raw}");
    }

    #[test]
    fn kpis_default_to_thirty_days_and_cap_at_ninety() {
        let ts = crate::test_helpers_http::additions_store(false);
        let s = &ts.store;
        let (status, body, raw) = get(s, "/api/v1/kpis");
        assert_eq!(status, 200, "{raw}");
        assert_eq!(body["days"], 30);
        assert_eq!(body["from"], "2026-08-08");
        assert_eq!(body["to"], "2026-09-06");
        let series = body["series"].as_array().unwrap();
        assert_eq!(series.len(), 30);
        assert_eq!(series[0]["day"], "2026-08-08");
        assert_eq!(series[29]["day"], "2026-09-06");
        assert_eq!(day_fields(&body["totals"]), (0, 0, 0, 0.0, 0, 0, 0, 0));
        assert_eq!(autonomy_fields(&body["totals"]), (0, 0, 0, 0, 0, 0));
        assert_eq!(body["priced"], false);

        let (status, body, raw) = get(s, "/api/v1/kpis?days=90");
        assert_eq!(status, 200, "{raw}");
        assert_eq!(body["series"].as_array().unwrap().len(), 90);
        assert_eq!(body["from"], "2026-06-09");

        // The library entry point clamps on its own, too.
        let report = daily_kpis(s, "2026-09-06T12:00:00Z", 500, "").unwrap();
        assert_eq!(report.series.len(), 90);
        assert_eq!(report.days, 90);
    }
}
