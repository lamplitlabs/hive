use crate::httpx::{Request, ResponseWriter};
use crate::server::{Handler, JsonMap};
use rusqlite::types::Value as SqlValue;
use serde_json::{Value, json};

pub(crate) const JOB_REPAIR_HEADER: &str = "X-Hive-Job-Repair";

/// A job id as the hub accepts it in a path: one plain token, so it can
/// name a folder under `work/jobs/` without ever reaching outside it.
fn plain_job_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

impl Handler {
    pub(crate) fn addition_jobs(&self, w: &mut ResponseWriter, r: &Request, parts: Vec<String>) {
        if (parts.len() == 3 || parts.len() == 4) && parts[0] == "jobs" && parts[2] == "artifacts" {
            self.addition_job_artifacts(w, r, &parts);
            return;
        }
        if parts.len() != 2 || parts[0] != "jobs" || (parts[1] != "failed" && parts[1] != "repair")
        {
            crate::addition_not_found(w);
            return;
        }
        let method = if parts[1] == "repair" { "POST" } else { "GET" };
        if !crate::addition_method(w, r, vec![method.to_string()])
            || !self.addition_available(
                w,
                r,
                vec![
                    "jobs".to_string(),
                    "projects".to_string(),
                    "runs".to_string(),
                    "events".to_string(),
                    "posts".to_string(),
                    "settings".to_string(),
                    "tasks".to_string(),
                ],
            )
        {
            return;
        }
        let Some(store) = self.store.as_deref() else {
            crate::addition_failure(w, anyhow::anyhow!("store is not available"));
            return;
        };
        let mut project = r.query("project");
        if r.method == "GET" {
            let ok;
            (project, ok) = self.job_repair_project(w, project, String::new());
            if !ok {
                return;
            }
            let mut limit = hive_runner::FAILED_JOBS_DEFAULT_LIMIT;
            let values = r.query_all("limit");
            if !values.is_empty() {
                if values.len() != 1 {
                    crate::write_error(
                        w,
                        400,
                        "Invalid limit".to_string(),
                        "Send one page size.".to_string(),
                        "Use limit=50.".to_string(),
                    );
                    return;
                }
                match values[0].parse::<i32>() {
                    Ok(n) if (1..=hive_runner::FAILED_JOBS_MAX_LIMIT).contains(&n) => limit = n,
                    _ => {
                        crate::write_error(
                            w,
                            400,
                            "Invalid limit".to_string(),
                            format!(
                                "limit must be between 1 and {}.",
                                hive_runner::FAILED_JOBS_MAX_LIMIT
                            ),
                            "Use limit=50.".to_string(),
                        );
                        return;
                    }
                }
            }
            match hive_runner::read_failed_jobs(
                store,
                hive_runner::FailedJobsQuery {
                    project,
                    limit,
                    before: r.query("before"),
                },
            ) {
                Ok(page) => crate::write_json(w, 200, go_value(&page)),
                Err(err) => {
                    if let Some(cli) = err.downcast_ref::<hive_cli::Error>()
                        && cli.code == 2
                    {
                        let code = if cli.what.starts_with("No project ") {
                            404
                        } else {
                            400
                        };
                        crate::write_error(
                            w,
                            code,
                            cli.what.clone(),
                            cli.why.clone(),
                            cli.fix.clone(),
                        );
                        return;
                    }
                    crate::addition_failure(w, err)
                }
            }
            return;
        }
        if !crate::loopback_host(r) {
            crate::write_error(
                w,
                403,
                "Job repair is available only on loopback".to_string(),
                "The request named a host other than this machine.".to_string(),
                "Open the hub on 127.0.0.1.".to_string(),
            );
            return;
        }
        if r.header(JOB_REPAIR_HEADER) != "yes" {
            crate::write_error(
                w,
                403,
                format!("missing {JOB_REPAIR_HEADER}: yes"),
                "An explicit confirmation header is required.".to_string(),
                "Select up to five eligible jobs and confirm in the hub.".to_string(),
            );
            return;
        }
        let (payload, ok) = crate::read_payload(w, r);
        if !ok {
            return;
        }
        for key in payload.keys() {
            if key != "job_ids" && key != "project" {
                crate::write_error(
                    w,
                    400,
                    format!("Unknown repair field: {key}"),
                    "Only job_ids and project are accepted; the actor is server-owned.".to_string(),
                    "Send explicit job IDs without an actor or override.".to_string(),
                );
                return;
            }
        }
        let (body_project, ok) = crate::addition_string(w, &payload, "project", false);
        if !ok {
            return;
        }
        let ok;
        (project, ok) = self.job_repair_project(w, project, body_project);
        if !ok {
            return;
        }
        let Some(ids) = payload.get("job_ids").and_then(Value::as_array) else {
            invalid_job_ids(w);
            return;
        };
        if ids.is_empty() || ids.len() > hive_runner::JOB_REPAIR_MAX_SELECTION as usize {
            invalid_job_ids(w);
            return;
        }
        let mut input = hive_runner::RepairJobsInput {
            project,
            ..hive_runner::RepairJobsInput::default()
        };
        let mut seen = std::collections::BTreeSet::new();
        for value in ids {
            let Some(id) = value.as_str() else {
                crate::write_error(
                    w,
                    400,
                    "Invalid job_ids".to_string(),
                    "Every job ID must be a string.".to_string(),
                    "Select existing jobs from the failed-job list.".to_string(),
                );
                return;
            };
            if id.is_empty() || !seen.insert(id.to_string()) {
                invalid_job_ids(w);
                return;
            }
            input.job_i_ds.push(id.to_string());
        }
        let _guard = crate::ADDITION_WRITES
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        match hive_runner::repair_failed_jobs(store, input) {
            Ok(result) => crate::write_json(w, 200, go_value(&result)),
            Err(err) => crate::addition_failure(w, err),
        }
    }

    pub(crate) fn job_repair_project(
        &self,
        w: &mut ResponseWriter,
        query: String,
        body: String,
    ) -> (String, bool) {
        if !self.project.is_empty()
            && ((!query.is_empty() && query != self.project)
                || (!body.is_empty() && body != self.project))
        {
            crate::write_error(
                w,
                404,
                "No such project in this hub".to_string(),
                "This hub is scoped to a different project.".to_string(),
                "Use this hub's project.".to_string(),
            );
            return (String::new(), false);
        }
        if !query.is_empty() && !body.is_empty() && query != body {
            crate::write_error(
                w,
                400,
                "Conflicting projects".to_string(),
                "The query and body name different projects.".to_string(),
                "Use one project for the whole selection.".to_string(),
            );
            return (String::new(), false);
        }
        if !self.project.is_empty() {
            return (self.project.clone(), true);
        }
        if !body.is_empty() {
            return (body, true);
        }
        (query, true)
    }

    /// `GET /api/v1/jobs/{id}/artifacts`: the files a job's worker left for
    /// the owner under `HIVE_HOME/work/jobs/{id}/artifacts`, newest first
    /// (an empty list when the job never made one);
    /// `GET /api/v1/jobs/{id}/artifacts/{name}`: one of them, served the way
    /// a conversation's artifacts are. An id that is not a job answers 404.
    fn addition_job_artifacts(&self, w: &mut ResponseWriter, r: &Request, parts: &[String]) {
        let id = parts[1].as_str();
        if !plain_job_id(id) {
            crate::addition_not_found(w);
            return;
        }
        if !crate::addition_method(w, r, vec!["GET".to_string()])
            || !self.addition_available(w, r, vec!["jobs".to_string()])
        {
            return;
        }
        let Some(store) = self.store.as_deref() else {
            crate::addition_failure(w, anyhow::anyhow!("store is not available"));
            return;
        };
        let rows = match self.addition_rows(
            r,
            "SELECT id FROM jobs WHERE id=?",
            vec![SqlValue::Text(id.to_string())],
        ) {
            Ok(rows) => rows,
            Err(err) => {
                crate::addition_failure(w, err);
                return;
            }
        };
        if rows.is_empty() {
            crate::write_error(
                w,
                404,
                "No such job".to_string(),
                format!("No job {id:?} is known to this hub."),
                "Open a job from a task or the failed-job list.".to_string(),
            );
            return;
        }
        let dir = hive_harness::artifacts::job_artifacts_dir(&store.home, id);
        let page = hive_harness::report::PAGE;
        // Every finished job has its report: Hive builds the page the first
        // time the job's folder is listed without one, and again each time
        // the page itself is opened, so it says where the work stands now.
        let finished = store
            .conn()
            .ok()
            .and_then(|c| {
                c.query_row("SELECT status FROM jobs WHERE id = ?1", [id], |r| {
                    r.get::<_, String>(0)
                })
                .ok()
            })
            .is_some_and(|status| hive_runner::work_report::job_finished(&status));
        let wanted = if parts.len() == 3 {
            finished && !dir.join(page).is_file()
        } else {
            parts[3] == page
        };
        if wanted && let Err(e) = hive_runner::work_report::job(store, id, &dir) {
            eprintln!("job {id}: the report page could not be built: {e:#}");
        }
        if parts.len() == 3 {
            let artifacts = hive_harness::artifacts::list(&dir);
            crate::write_json(w, 200, json!({"job": id, "artifacts": artifacts}));
            return;
        }
        let name = parts[3].as_str();
        let Some(path) = hive_harness::artifacts::file(&dir, name) else {
            crate::write_error(
                w,
                404,
                "No such artifact".to_string(),
                format!("{name} is not a file in this job's artifacts folder."),
                "Open the job and pick a file from its evidence list.".to_string(),
            );
            return;
        };
        crate::chat::serve_artifact_file(w, r, name, &path);
    }
}

fn invalid_job_ids(w: &mut ResponseWriter) {
    crate::write_error(
        w,
        400,
        "Invalid job_ids".to_string(),
        format!(
            "Select between 1 and {} explicit job IDs.",
            hive_runner::JOB_REPAIR_MAX_SELECTION
        ),
        "Select a small set of eligible entries, not all history.".to_string(),
    );
}

fn go_value<T: serde::Serialize>(v: &T) -> Value {
    let text = hive_store::gojson::to_string(v).expect("serialize hub payload");
    serde_json::from_str(&text).expect("reparse hub payload")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job_repair_store() -> crate::test_helpers_http::TestStore {
        let s = crate::test_helpers_http::additions_store(false);
        crate::test_helpers_http::additions_project(&s, "one");
        crate::test_helpers_http::additions_project(&s, "two");
        for id in ["job-one", "job-two"] {
            let project = id.trim_start_matches("job-");
            s.conn().unwrap().execute(
                "INSERT INTO jobs(id,project,kind,tier,title,brief,status,result,created_at,finished_at)
                 VALUES (?,?,'improve',1,'Improve the project','Original task','failed','verify exit 1','2026-09-06T10:00:00Z','2026-09-06T11:00:00Z')",
                rusqlite::params![id, project],
            ).unwrap();
        }
        s
    }

    fn job_repair_request(
        h: crate::server::Handler,
        method: &str,
        path: &str,
        body: &str,
    ) -> (u16, JsonMap, String) {
        crate::test_helpers_http::request(
            h,
            method,
            path,
            body,
            &[(JOB_REPAIR_HEADER, "yes"), ("Host", "127.0.0.1:8765")],
        )
    }

    fn repair_row_count(s: &hive_store::Store, table: &str) -> i64 {
        s.conn()
            .unwrap()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn job_repair_handler_read_and_contract() {
        let _serial = crate::test_helpers::serial();
        let s = job_repair_store();
        let before = repair_row_count(&s, "events");
        let (status, page, raw) = job_repair_request(
            crate::test_helpers_http::handler(&s),
            "GET",
            "/api/v1/jobs/failed?project=one&limit=1",
            "",
        );
        assert_eq!(status, 200, "{raw}");
        assert_eq!(page["historical_total"], 1);
        assert_eq!(page["jobs"].as_array().unwrap().len(), 1);
        assert_eq!(page["jobs"][0]["id"], "job-one");
        assert_eq!(page["jobs"][0]["eligible"], true);
        assert_eq!(page["page_counts"]["eligible"], 1);
        assert_eq!(page["max_selection"], 5);
        assert_ne!(page["jobs"][0]["reason"], "");
        assert_ne!(page["jobs"][0]["fix"], "");
        assert_eq!(page["jobs"][0]["result"], "verify exit 1");
        assert!(!raw.contains("\"brief\""));
        assert!(!raw.contains("\"log\""));
        let (status, page, raw) = job_repair_request(
            crate::test_helpers_http::handler(&s),
            "GET",
            "/api/v1/jobs/failed?limit=1",
            "",
        );
        assert_eq!(status, 200, "{raw}");
        assert_eq!(page["historical_total"], 2);
        assert_eq!(page["jobs"].as_array().unwrap().len(), 1);
        let next_before = page["next_before"].as_str().unwrap();
        assert_ne!(next_before, "");
        let (status, page, raw) = job_repair_request(
            crate::test_helpers_http::handler(&s),
            "GET",
            &format!("/api/v1/jobs/failed?limit=1&before={next_before}"),
            "",
        );
        assert_eq!(status, 200, "{raw}");
        assert_eq!(page["historical_total"], 2);
        assert_eq!(page["jobs"].as_array().unwrap().len(), 1);
        assert_eq!(page["next_before"].as_str().unwrap_or(""), "");
        assert_eq!(repair_row_count(&s, "events"), before);
        for path in [
            "/api/v1/jobs/failed?limit=0",
            "/api/v1/jobs/failed?limit=101",
            "/api/v1/jobs/failed?limit=1.5",
            "/api/v1/jobs/failed?limit=",
            "/api/v1/jobs/failed?limit=1&limit=2",
            "/api/v1/jobs/failed?before=bad",
        ] {
            let (status, _, raw) =
                job_repair_request(crate::test_helpers_http::handler(&s), "GET", path, "");
            assert_eq!(status, 400, "{path}: {raw}");
        }
        let (status, _, raw) = job_repair_request(
            crate::test_helpers_http::handler(&s),
            "GET",
            "/api/v1/jobs/failed?project=missing",
            "",
        );
        assert_eq!(status, 404, "unknown project: {raw}");
    }

    #[test]
    fn job_repair_handler_guards() {
        let _serial = crate::test_helpers::serial();
        let s = job_repair_store();
        let before = repair_row_count(&s, "events");
        for (name, header, value, want) in [
            ("intent", JOB_REPAIR_HEADER, "", 403),
            ("intent value", JOB_REPAIR_HEADER, "true", 403),
            ("origin", "Origin", "https://evil.example", 403),
            ("fetch site", "Sec-Fetch-Site", "cross-site", 403),
            ("content type", "Content-Type", "text/plain", 415),
            ("host", "Host", "rebound.example:8787", 403),
        ] {
            let mut headers = vec![("Content-Type", "application/json")];
            if name != "intent" && name != "intent value" {
                headers.push((JOB_REPAIR_HEADER, "yes"));
            }
            headers.push((header, value));
            let (status, raw) = if name == "content type" {
                let srv = crate::test_helpers_http::server(crate::test_helpers_http::handler(&s));
                let resp = crate::test_helpers::client()
                    .post(format!("{}/api/v1/jobs/repair", srv.url))
                    .header(JOB_REPAIR_HEADER, "yes")
                    .header("Content-Type", "text/plain")
                    .body(r#"{"job_ids":["job-one"]}"#.to_string())
                    .send()
                    .unwrap();
                let status = resp.status().as_u16();
                let raw = resp.text().unwrap();
                (status, raw)
            } else {
                let (status, _, raw) = crate::test_helpers_http::request(
                    crate::test_helpers_http::handler(&s),
                    "POST",
                    "/api/v1/jobs/repair",
                    r#"{"job_ids":["job-one"]}"#,
                    &headers,
                );
                (status, raw)
            };
            assert_eq!(status, want, "{name}: {raw}");
        }
        assert_eq!(repair_row_count(&s, "events"), before);
        assert_eq!(repair_row_count(&s, "jobs"), 2);
    }

    #[test]
    fn job_repair_handler_selection_and_actor() {
        let _serial = crate::test_helpers::serial();
        let s = job_repair_store();
        let _env = hive_testutil::setenv("HIVE_AGENT", "not-the-owner");
        let before = repair_row_count(&s, "events");
        for body in [
            "{}",
            r#"{"job_ids":[]}"#,
            r#"{"job_ids":"all"}"#,
            r#"{"job_ids":[""]}"#,
            r#"{"job_ids":[null]}"#,
            r#"{"job_ids":["job-one","job-one"]}"#,
            r#"{"job_ids":["1","2","3","4","5","6"]}"#,
            r#"{"job_ids":["job-one"],"actor":"daemon"}"#,
            r#"{"job_ids":["job-one"],"override":true}"#,
            r#"{"job_ids":["job-one"],"project":7}"#,
            r#"{"job_ids":["job-one"]} {}"#,
        ] {
            let (status, _, raw) = job_repair_request(
                crate::test_helpers_http::handler(&s),
                "POST",
                "/api/v1/jobs/repair",
                body,
            );
            assert_eq!(status, 400, "{body}: {raw}");
        }
        assert_eq!(repair_row_count(&s, "events"), before);
        let (status, result, raw) = crate::test_helpers_http::request(
            crate::test_helpers_http::handler(&s),
            "POST",
            "/api/v1/jobs/repair",
            r#"{"job_ids":["job-one"],"project":"one"}"#,
            &[
                (JOB_REPAIR_HEADER, "yes"),
                ("Content-Type", "application/json"),
                ("X-Hive-Agent", "spoofed"),
                ("Host", "127.0.0.1:8765"),
            ],
        );
        assert_eq!(status, 200, "{raw}");
        assert_eq!(result["requested"], 1);
        assert_eq!(result["queued"], 1);
        assert_eq!(result["skipped"], 0);
        assert_eq!(result["errors"], 0);
        assert_eq!(result["results"][0]["outcome"], "replanned");
        let actor: String = s
            .conn()
            .unwrap()
            .query_row(
                "SELECT actor FROM events WHERE action = 'job.recover' AND subject = 'job-one'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(actor, hive_store::HUMAN_AGENT);
        let (status, result, raw) = job_repair_request(
            crate::test_helpers_http::handler(&s),
            "POST",
            "/api/v1/jobs/repair",
            r#"{"job_ids":["job-one"],"project":"one"}"#,
        );
        assert_eq!(status, 200, "{raw}");
        assert_eq!(result["queued"], 0);
        assert_eq!(result["skipped"], 1);
        assert_ne!(result["results"][0]["carrier_id"], "");
        assert_eq!(repair_row_count(&s, "jobs"), 3);
    }

    #[test]
    fn job_repair_handler_project_scope_and_holds() {
        let _serial = crate::test_helpers::serial();
        let s = job_repair_store();
        for (method, path, body) in [
            ("GET", "/api/v1/jobs/failed?project=two", ""),
            (
                "POST",
                "/api/v1/jobs/repair",
                r#"{"job_ids":["job-one"],"project":"two"}"#,
            ),
            (
                "POST",
                "/api/v1/jobs/repair?project=two",
                r#"{"job_ids":["job-one"]}"#,
            ),
        ] {
            let mut h = crate::test_helpers_http::handler(&s);
            h.project = "one".to_string();
            let (status, _, raw) = job_repair_request(h, method, path, body);
            assert_eq!(status, 404, "{raw}");
        }
        s.conn()
            .unwrap()
            .execute(
                "UPDATE projects SET status = 'inactive' WHERE slug = 'one'",
                [],
            )
            .unwrap();
        let mut h = crate::test_helpers_http::handler(&s);
        h.project = "one".to_string();
        let (status, result, raw) = job_repair_request(
            h,
            "POST",
            "/api/v1/jobs/repair",
            r#"{"job_ids":["job-one","job-two","unknown"]}"#,
        );
        assert_eq!(status, 200, "{raw}");
        assert_eq!(result["queued"], 0);
        assert_eq!(result["skipped"], 3);
        assert_eq!(result["results"][0]["category"], "inactive_project");
        assert_eq!(result["results"][1]["category"], "not_found");
        assert_eq!(result["results"][2]["category"], "not_found");
        let h = crate::test_helpers_http::handler(&s);
        let (status, _, raw) = job_repair_request(
            h,
            "POST",
            "/api/v1/jobs/repair?project=one",
            r#"{"job_ids":["job-one"],"project":"two"}"#,
        );
        assert_eq!(status, 400, "{raw}");
        let (status, _, raw) = job_repair_request(
            crate::test_helpers_http::handler(&s),
            "POST",
            "/api/v1/jobs/repair",
            r#"{"job_ids":["job-one"],"project":"missing"}"#,
        );
        assert_eq!(status, 404, "{raw}");
        s.set_setting(hive_store::HUMAN_AGENT, "stop", "1").unwrap();
        let (status, result, raw) = job_repair_request(
            crate::test_helpers_http::handler(&s),
            "POST",
            "/api/v1/jobs/repair",
            r#"{"job_ids":["job-two"]}"#,
        );
        assert_eq!(status, 200, "{raw}");
        assert_eq!(result["skipped"], 1);
        assert_eq!(result["results"][0]["category"], "stopped");
        assert_eq!(repair_row_count(&s, "jobs"), 2);
    }

    #[test]
    fn job_repair_handler_methods_and_availability() {
        let _serial = crate::test_helpers::serial();
        let s = job_repair_store();
        for (method, path, want) in [
            ("POST", "/api/v1/jobs/failed", 405),
            ("GET", "/api/v1/jobs/repair", 405),
            ("POST", "/api/v1/jobs/retry-all", 404),
            ("GET", "/api/v1/jobs/failed/extra", 404),
        ] {
            let (status, _, raw) =
                job_repair_request(crate::test_helpers_http::handler(&s), method, path, "{}");
            assert_eq!(status, want, "{method} {path}: {raw}");
        }
        let (status, _, _) = job_repair_request(
            crate::server::new_handler(None, None),
            "POST",
            "/api/v1/jobs/repair",
            r#"{"job_ids":["job-one"]}"#,
        );
        assert_eq!(status, 409);
    }

    #[test]
    fn job_artifacts_are_listed_per_job() {
        let _serial = crate::test_helpers::serial();
        let s = job_repair_store();
        let h = crate::test_helpers_http::handler(&s);
        let (status, _, raw) = crate::test_helpers_http::request(
            h.clone(),
            "GET",
            "/api/v1/jobs/job-unknown/artifacts",
            "",
            &[],
        );
        assert_eq!(status, 404, "unknown job: {raw}");
        for bad in ["..", "a%2Fb", "job%20one", "x%00y"] {
            let (status, _, raw) = crate::test_helpers_http::request(
                h.clone(),
                "GET",
                &format!("/api/v1/jobs/{bad}/artifacts"),
                "",
                &[],
            );
            assert_eq!(status, 404, "{bad}: {raw}");
        }
        let (status, body, raw) = crate::test_helpers_http::request(
            h.clone(),
            "GET",
            "/api/v1/jobs/job-one/artifacts",
            "",
            &[],
        );
        assert_eq!(status, 200, "{raw}");
        assert_eq!(body["job"], "job-one");
        // A finished job left nothing: Hive's report is there all the same.
        let names: Vec<&str> = body["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["report.html"], "{raw}");
        let dir = hive_harness::artifacts::job_artifacts_dir(&s.home, "job-one");
        let built = std::fs::read_to_string(dir.join("report.html")).unwrap();
        assert!(
            built.contains("<title>Improve the project</title>"),
            "{built}"
        );
        assert!(built.contains("Built by Hive"), "{built}");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("shot.png"), b"png").unwrap();
        std::fs::write(dir.join("report.html"), "<h1>hi</h1>").unwrap();
        std::fs::write(dir.join(".hidden"), "x").unwrap();
        let (status, body, raw) = crate::test_helpers_http::request(
            h.clone(),
            "GET",
            "/api/v1/jobs/job-one/artifacts",
            "",
            &[],
        );
        assert_eq!(status, 200, "{raw}");
        let mut names: Vec<&str> = body["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["name"].as_str().unwrap())
            .collect();
        names.sort_unstable();
        assert_eq!(names, vec!["report.html", "shot.png"]);
        let report = body["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == "report.html")
            .unwrap();
        assert_eq!(report["size"], 11);
        assert!(report["modified"].as_str().unwrap().ends_with('Z'));
        // The other job's folder is its own: its own report, nothing of job-one's.
        let (status, body, raw) =
            crate::test_helpers_http::request(h, "GET", "/api/v1/jobs/job-two/artifacts", "", &[]);
        assert_eq!(status, 200, "{raw}");
        let names: Vec<&str> = body["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["report.html"], "{raw}");
    }

    #[test]
    fn job_artifact_files_are_served_safely() {
        let _serial = crate::test_helpers::serial();
        let s = job_repair_store();
        let dir = hive_harness::artifacts::job_artifacts_dir(&s.home, "job-one");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("report.html"), "<h1>hi</h1>").unwrap();
        std::fs::write(dir.join("data.bin"), [0u8, 1, 2, 3]).unwrap();
        std::fs::write(dir.join(".hidden"), "secret").unwrap();
        let srv = crate::test_helpers_http::server(crate::test_helpers_http::handler(&s));
        let client = crate::test_helpers::client();
        let resp = client
            .get(format!(
                "{}/api/v1/jobs/job-one/artifacts/report.html",
                srv.url
            ))
            .send()
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
        assert!(
            resp.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );
        assert!(
            resp.headers()["content-security-policy"]
                .to_str()
                .unwrap()
                .contains("sandbox")
        );
        assert_eq!(resp.headers()["x-content-type-options"], "nosniff");
        assert_eq!(resp.headers()["cache-control"], "no-store");
        assert_eq!(resp.headers()["x-frame-options"], "SAMEORIGIN");
        // The report is Hive's, built again as it is opened.
        let page = resp.text().unwrap();
        assert!(
            page.contains("<title>Improve the project</title>"),
            "{page}"
        );
        assert!(
            page.contains("<span class=\"state bad\">Failed</span>Improve job"),
            "{page}"
        );
        let resp = client
            .get(format!(
                "{}/api/v1/jobs/job-one/artifacts/data.bin",
                srv.url
            ))
            .send()
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
        assert_eq!(resp.headers()["content-type"], "application/octet-stream");
        assert_eq!(
            resp.headers()["content-security-policy"],
            "default-src 'none'; style-src 'unsafe-inline'"
        );
        for name in [
            "..%2F..%2Fetc%2Fpasswd",
            "..%5Cetc%5Cpasswd",
            ".hidden",
            "missing.txt",
        ] {
            let resp = client
                .get(format!("{}/api/v1/jobs/job-one/artifacts/{name}", srv.url))
                .send()
                .unwrap();
            assert_eq!(resp.status().as_u16(), 404, "{name}");
        }
        // A file of an unknown job is never looked up.
        let resp = client
            .get(format!(
                "{}/api/v1/jobs/job-missing/artifacts/report.html",
                srv.url
            ))
            .send()
            .unwrap();
        assert_eq!(resp.status().as_u16(), 404);
        for path in [
            "/api/v1/jobs/job-one/artifacts",
            "/api/v1/jobs/job-one/artifacts/report.html",
        ] {
            let resp = client
                .post(format!("{}{path}", srv.url))
                .header("Content-Type", "application/json")
                .body("{}")
                .send()
                .unwrap();
            assert_eq!(resp.status().as_u16(), 405, "POST {path}");
            assert_eq!(resp.headers()["allow"], "GET");
        }
        let resp = client
            .get(format!(
                "{}/api/v1/jobs/job-one/artifacts/report.html/extra",
                srv.url
            ))
            .send()
            .unwrap();
        assert_eq!(resp.status().as_u16(), 404, "too many path parts");
    }
}
