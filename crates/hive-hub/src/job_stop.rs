//! `POST /api/v1/jobs/{id}/stop {requeue}`: the console's Stop for a run on
//! any machine, this one, a paired or a rented one. It writes the same
//! `run.stop` event as `hive job stop JOB [--requeue]`
//! ([`hive_board::stop_job`]), as the owner, and the run's watcher on the
//! machine that runs it stops the run within a few seconds.

use crate::httpx::{Request, ResponseWriter};
use crate::server::Handler;
use serde_json::{Value, json};

/// A job id as a path names it: one plain token.
fn plain_job_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

impl Handler {
    pub(crate) fn addition_job_stop(&self, w: &mut ResponseWriter, r: &Request, id: &str) {
        if !crate::addition_method(w, r, vec!["POST".to_string()]) {
            return;
        }
        // Same-origin guard and a JSON object body (`{}` or `{"requeue": true}`).
        let (payload, ok) = crate::server::read_payload(w, r);
        if !ok {
            return;
        }
        if !plain_job_id(id) {
            crate::write_error(
                w,
                400,
                "Invalid job id".into(),
                "A job id is one plain word of letters, digits, - and _.".into(),
                "Open the job from its list and stop it there.".into(),
            );
            return;
        }
        let requeue = match payload.get("requeue") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(b)) => *b,
            Some(_) => {
                crate::write_error(
                    w,
                    400,
                    "Invalid requeue".into(),
                    "requeue is true or false.".into(),
                    "Send {\"requeue\": true} to put the job back in the queue.".into(),
                );
                return;
            }
        };
        if !self.addition_available(
            w,
            r,
            vec!["jobs".to_string(), "runs".to_string(), "events".to_string()],
        ) {
            return;
        }
        let Some(store) = self.store.as_deref() else {
            crate::addition_failure(w, anyhow::anyhow!("store is not available"));
            return;
        };
        let _guard = crate::ADDITION_WRITES
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        match hive_board::stop_job(store, id, requeue, hive_store::HUMAN_AGENT) {
            Ok(stop) => crate::write_json(
                w,
                200,
                json!({
                    "job": stop.job,
                    "run": stop.run,
                    "executor": if stop.executor.is_empty() {
                        hive_board::LOCAL_EXECUTOR.to_string()
                    } else {
                        stop.executor
                    },
                    "requeue": stop.requeue,
                }),
            ),
            Err(err) => {
                if let Some(cli) = err.downcast_ref::<hive_cli::Error>()
                    && cli.code == 2
                {
                    let (code, why, fix) = if cli.what.starts_with("No job ") {
                        (
                            404,
                            "No job has this id.",
                            "Open the job from its list and stop it there.",
                        )
                    } else {
                        (
                            409,
                            "The job has no run in flight to stop.",
                            "Refresh: the run may have ended already.",
                        )
                    };
                    let why = if cli.why.is_empty() {
                        why.to_string()
                    } else {
                        cli.why.clone()
                    };
                    let fix = if cli.fix.is_empty() {
                        fix.to_string()
                    } else {
                        cli.fix.clone()
                    };
                    crate::write_error(w, code, cli.what.clone(), why, fix);
                    return;
                }
                crate::addition_failure(w, err)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    fn stop_store() -> crate::test_helpers_http::TestStore {
        let s = crate::test_helpers_http::additions_store(false);
        crate::test_helpers_http::additions_project(&s, "demo");
        let c = s.conn().unwrap();
        c.execute(
            "INSERT INTO jobs(id,project,kind,tier,title,brief,status,lease_agent,created_at)
             VALUES ('job-away','demo','task',1,'Away','Runs on a rented machine','leased','bob','2026-09-06T10:00:00Z'),
                    ('job-waits','demo','task',1,'Waits','Not started','queued','','2026-09-06T10:00:00Z')",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO runs (id, job_id, agent, project, started_at, status, executor)
             VALUES ('run-away', 'job-away', 'bob', 'demo', '2026-09-06T11:00:00Z', 'running', 'hands-cr-1')",
            [],
        )
        .unwrap();
        drop(c);
        s
    }

    fn stop_events(s: &hive_store::Store) -> Vec<(String, String, Value)> {
        let c = s.conn().unwrap();
        let mut q = c
            .prepare(
                "SELECT actor, subject, detail FROM events WHERE action = 'run.stop' ORDER BY seq",
            )
            .unwrap();
        q.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .unwrap()
        .map(|row| {
            let (actor, subject, detail) = row.unwrap();
            (actor, subject, serde_json::from_str(&detail).unwrap())
        })
        .collect()
    }

    #[test]
    fn job_stop_route_asks_the_run_on_its_machine_to_stop() {
        let _serial = crate::test_helpers::serial();
        let s = stop_store();
        let (status, page, raw) = crate::test_helpers_http::request(
            crate::test_helpers_http::handler(&s),
            "POST",
            "/api/v1/jobs/job-away/stop",
            r#"{"requeue": true}"#,
            &[
                ("Origin", "http://127.0.0.1:8765"),
                ("Host", "127.0.0.1:8765"),
            ],
        );
        assert_eq!(status, 200, "{raw}");
        assert_eq!(page["job"], "job-away");
        assert_eq!(page["run"], "run-away");
        assert_eq!(page["executor"], "hands-cr-1");
        assert_eq!(page["requeue"], true);
        let events = stop_events(&s);
        assert_eq!(events.len(), 1, "{events:?}");
        let (actor, subject, detail) = &events[0];
        assert_eq!(actor, hive_store::HUMAN_AGENT, "the owner stops it");
        assert_eq!(subject, "run-away");
        assert_eq!(detail["job"], "job-away");
        assert_eq!(detail["executor"], "hands-cr-1");
        assert_eq!(detail["requeue"], true);

        // Without requeue (an empty body): the job ends cancelled.
        let (status, page, raw) = crate::test_helpers_http::request(
            crate::test_helpers_http::handler(&s),
            "POST",
            "/api/v1/jobs/job-away/stop",
            "{}",
            &[],
        );
        assert_eq!(status, 200, "{raw}");
        assert_eq!(page["requeue"], false);
        let events = stop_events(&s);
        assert_eq!(events.len(), 2, "{events:?}");
        assert_eq!(events[1].2["requeue"], false);
    }

    #[test]
    fn job_stop_route_refuses_what_it_cannot_stop() {
        let _serial = crate::test_helpers::serial();
        let s = stop_store();
        let post = |path: &str, body: &str, headers: &[(&str, &str)]| {
            crate::test_helpers_http::request(
                crate::test_helpers_http::handler(&s),
                "POST",
                path,
                body,
                headers,
            )
        };
        // No run in flight: stop_job's refusal, in its own words.
        let (status, page, raw) = post("/api/v1/jobs/job-waits/stop", "{}", &[]);
        assert_eq!(status, 409, "{raw}");
        let what = page["error"].as_str().unwrap_or_default();
        assert!(
            what.contains("no run in flight") && what.contains("cancelled"),
            "{raw}"
        );
        let (status, _, raw) = post("/api/v1/jobs/job-none/stop", "{}", &[]);
        assert_eq!(status, 404, "{raw}");
        // Another site's page cannot stop a run.
        let (status, _, raw) = post(
            "/api/v1/jobs/job-away/stop",
            "{}",
            &[
                ("Origin", "https://evil.example"),
                ("Host", "127.0.0.1:8765"),
            ],
        );
        assert_eq!(status, 403, "{raw}");
        let (status, _, raw) = post("/api/v1/jobs/job-away/stop", r#"{"requeue":"yes"}"#, &[]);
        assert_eq!(status, 400, "{raw}");
        let (status, _, raw) = post("/api/v1/jobs/job.away/stop", "{}", &[]);
        assert_eq!(status, 400, "{raw}");
        let (status, _, raw) = crate::test_helpers_http::request(
            crate::test_helpers_http::handler(&s),
            "GET",
            "/api/v1/jobs/job-away/stop",
            "",
            &[],
        );
        assert_eq!(status, 405, "{raw}");
        assert!(stop_events(&s).is_empty(), "a refused stop writes nothing");
    }
}
