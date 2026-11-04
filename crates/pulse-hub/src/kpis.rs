//! Daily KPIs for the Home page's "Over time" panel: what the fleet got
//! done and what it spent, per UTC day.
//!
//! `GET /api/v1/kpis?days=N[&project=SLUG]` answers with one row per day
//! over the last N days ending today (inclusive), zero-filled so a chart
//! can draw it without gaps: harness sessions with their tokens and cost
//! (velocity and spend), jobs finished done or failed, landings
//! (`land.landed` events) and tasks moved to done (`task.status` events
//! with `to = done`). Each source is one `GROUP BY` day query; rows older
//! than the window or dated in the future are dropped.

use crate::httpx::{Request, ResponseWriter};
use crate::server::Handler;
use rusqlite::params;
use serde::Serialize;
use std::collections::BTreeMap;

pub const KPIS_DEFAULT_DAYS: i64 = 30;
pub const KPIS_MAX_DAYS: i64 = 90;

/// One UTC day of fleet output and spend.
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

const LANDINGS_SQL: &str = "SELECT substr(at, 1, 10), COUNT(*) FROM events \
    WHERE action = 'land.landed' AND at >= ?1 \
    AND (?2 = '' OR (json_valid(detail) AND json_extract(detail, '$.project') = ?2)) GROUP BY 1";

const TASKS_DONE_SQL: &str = "SELECT substr(at, 1, 10), COUNT(*) FROM events \
    WHERE action = 'task.status' AND at >= ?1 \
    AND json_valid(detail) AND json_extract(detail, '$.to') = 'done' \
    AND (?2 = '' OR json_extract(detail, '$.project') = ?2) GROUP BY 1";

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
        s.conn()
            .unwrap()
            .execute(
                "INSERT INTO events (at, actor, action, subject, detail) VALUES (?, 'test', ?, 'x', ?)",
                params![at, action, detail],
            )
            .expect("insert event");
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
        assert_eq!(day_fields(&series[1]), (2, 300, 30, 0.0, 0, 0, 1, 0));
        assert_eq!(day_fields(&series[2]), (1, 0, 0, 0.0, 2, 1, 2, 1));
        assert_eq!(day_fields(&body["totals"]), (3, 300, 30, 0.0, 2, 1, 3, 1));
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
