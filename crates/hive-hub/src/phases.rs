//! Where work stands in the lifecycle (TASK-141, the AI-native SDLC of
//! `docs/sdlc.md`), read by the console:
//!
//! - `GET /api/v1/tasks/{id}/lifecycle`: one task's phases in order, each
//!   with what it left behind (the task, the lead's sittings, the jobs, the
//!   branches, the pull request, the verdict, the merge) and the gate that
//!   lets it pass, plus the phase it is in and who it waits on. The task
//!   page's Lifecycle tab draws it.
//! - `GET /api/v1/tasks/lifecycle?project=P`: every task of one project with
//!   its phase, and the phases in order with the task ids in each. The Tasks
//!   page's Phases view draws it.
//!
//! The phase is derived (`hive_tasks::lifecycle_for`): no ledger is kept.
//! The hub adds what a link needs and the derivation does not read: each
//! job's harness session, so a job id in the console opens its run.

use crate::httpx::{Request, ResponseWriter};
use crate::server::{Handler, write_error, write_json};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// The tables the lifecycle reads; a store from before any of them answers
/// 503 like the other task routes.
const LIFECYCLE_TABLES: [&str; 5] = ["tasks", "task_revisions", "jobs", "runs", "events"];

/// One phase as the console names it: the word on the wire, the heading
/// `docs/sdlc.md` gives it, and its gate (empty off a task's path).
pub(crate) fn phase_json(p: hive_tasks::Phase) -> Value {
    json!({"phase": p.as_str(), "label": p.label(), "gate": p.gate()})
}

impl Handler {
    /// `tasks/lifecycle` and `tasks/{id}/lifecycle`; false when the path is
    /// neither, so the tasks route goes on.
    pub(crate) fn addition_task_lifecycle_route(
        &self,
        w: &mut ResponseWriter,
        r: &Request,
        parts: &[String],
    ) -> bool {
        match parts {
            [_, one] if one == "lifecycle" => self.addition_project_lifecycle(w, r),
            [_, id, one] if one == "lifecycle" => self.addition_task_lifecycle(w, r, id),
            _ => return false,
        }
        true
    }

    fn lifecycle_ready(&self, w: &mut ResponseWriter, r: &Request) -> bool {
        crate::addition_method(w, r, vec!["GET".to_string()])
            && self.addition_available(
                w,
                r,
                LIFECYCLE_TABLES.iter().map(|t| t.to_string()).collect(),
            )
    }

    /// `GET /api/v1/tasks/{id}/lifecycle`.
    fn addition_task_lifecycle(&self, w: &mut ResponseWriter, r: &Request, id: &str) {
        if !self.lifecycle_ready(w, r) {
            return;
        }
        let Some(store) = self.store.as_deref() else {
            crate::addition_failure(w, "store is not available".to_string());
            return;
        };
        // The same lookup as the task itself: 404 for an unknown id or one
        // outside the selected project.
        if let Err(e) = self.addition_task(r, id) {
            crate::addition_failure(w, e.to_string());
            return;
        }
        let lifecycle = match hive_tasks::lifecycle_for(store, id) {
            Ok(l) => l,
            Err(e) => {
                crate::addition_failure(w, e.to_string());
                return;
            }
        };
        let mut v = match serde_json::to_value(&lifecycle) {
            Ok(v) => v,
            Err(e) => {
                crate::addition_failure(w, e.to_string());
                return;
            }
        };
        if let Err(e) = self.lifecycle_decorate(r, &mut v) {
            crate::addition_failure(w, e.to_string());
            return;
        }
        v["phases"] = Value::Array(
            hive_tasks::Phase::TASK_PATH
                .iter()
                .map(|p| phase_json(*p))
                .collect(),
        );
        write_json(w, 200, v);
    }

    /// `GET /api/v1/tasks/lifecycle?project=P`.
    fn addition_project_lifecycle(&self, w: &mut ResponseWriter, r: &Request) {
        if !self.lifecycle_ready(w, r) {
            return;
        }
        let Some(store) = self.store.as_deref() else {
            crate::addition_failure(w, "store is not available".to_string());
            return;
        };
        let project = self.addition_project(r);
        if project.is_empty() {
            write_error(
                w,
                400,
                "Which project?".to_string(),
                "Tasks by phase are read one project at a time.".to_string(),
                "Add ?project=<slug>.".to_string(),
            );
            return;
        }
        match store.project(&project) {
            Ok(Some(_)) => {}
            Ok(None) => {
                write_error(
                    w,
                    404,
                    "no such project".to_string(),
                    format!("No project is registered as {project:?}."),
                    "Pick a project from the Tasks page's project filter.".to_string(),
                );
                return;
            }
            Err(e) => {
                crate::addition_failure(w, e);
                return;
            }
        }
        let lifecycles = match hive_tasks::lifecycles_for(store, &project) {
            Ok(l) => l,
            Err(e) => {
                crate::addition_failure(w, e.to_string());
                return;
            }
        };
        let groups = hive_tasks::by_phase(&lifecycles);
        // Every phase a task can be in, in order, each with its tasks: the
        // task's path, then where tasks end.
        let order = hive_tasks::Phase::TASK_PATH
            .iter()
            .copied()
            .chain([hive_tasks::Phase::Done, hive_tasks::Phase::Cancelled]);
        let phases: Vec<Value> = order
            .map(|p| {
                let mut v = phase_json(p);
                v["tasks"] = json!(groups.get(&p).cloned().unwrap_or_default());
                v
            })
            .collect();
        // One row per task: the phase, who it waits on and the note, without
        // the steps (the task page reads those on its own).
        let tasks: Vec<Value> = lifecycles
            .iter()
            .map(|l| {
                json!({
                    "task": l.task,
                    "phase": l.phase,
                    "waits_on": l.waits_on,
                    "note": l.note,
                    "rework": l.rework,
                })
            })
            .collect();
        write_json(
            w,
            200,
            json!({"project": project, "phases": phases, "tasks": tasks}),
        );
    }

    /// Gives every `job` mark its harness session (`session`), so the
    /// console links a job to its run; a job without one keeps none.
    fn lifecycle_decorate(&self, r: &Request, v: &mut Value) -> anyhow::Result<()> {
        let mut jobs: Vec<String> = Vec::new();
        for step in v["steps"].as_array().into_iter().flatten() {
            for mark in step["left"].as_array().into_iter().flatten() {
                if matches!(mark["kind"].as_str(), Some("job" | "lead"))
                    && let Some(id) = mark["id"].as_str()
                    && !id.is_empty()
                    && !jobs.iter().any(|j| j == id)
                {
                    jobs.push(id.to_string());
                }
            }
        }
        if jobs.is_empty() {
            return Ok(());
        }
        let marks = vec!["?"; jobs.len()].join(",");
        let rows = self.addition_rows(
            r,
            &format!(
                "SELECT job_id, id FROM harness_sessions WHERE job_id IN ({marks}) \
                 ORDER BY started_at"
            ),
            jobs.into_iter().map(rusqlite::types::Value::Text).collect(),
        );
        // A store without harness sessions (an old one) links no runs.
        let Ok(rows) = rows else {
            return Ok(());
        };
        let mut session: BTreeMap<String, String> = BTreeMap::new();
        for row in rows {
            let job = row.get("job_id").and_then(Value::as_str).unwrap_or("");
            let id = row.get("id").and_then(Value::as_str).unwrap_or("");
            if !job.is_empty() && !id.is_empty() {
                // Ordered by start: the newest run wins.
                session.insert(job.to_string(), id.to_string());
            }
        }
        for step in v["steps"].as_array_mut().into_iter().flatten() {
            for mark in step["left"].as_array_mut().into_iter().flatten() {
                let id = mark["id"].as_str().unwrap_or("").to_string();
                if matches!(mark["kind"].as_str(), Some("job" | "lead"))
                    && let Some(s) = session.get(&id)
                {
                    mark["session"] = json!(s);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::test_helpers_http as http;

    fn insert_job(s: &hive_store::Store, id: &str, kind: &str, source: &str, status: &str) {
        let conn = s.conn().unwrap();
        conn.execute(
            "INSERT INTO jobs (id, project, kind, title, source, status, created_at, finished_at, branch) \
             VALUES (?, 'proj', ?, 'Step', ?, ?, '2026-11-26T10:00:00Z', \
             CASE WHEN ? = 'done' THEN '2026-11-26T10:30:00Z' ELSE '' END, \
             CASE WHEN ? = 'done' THEN 'hive/w/' || ? ELSE '' END)",
            rusqlite::params![id, kind, source, status, status, status, id],
        )
        .unwrap();
    }

    #[test]
    fn task_lifecycle_route() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(false);
        http::additions_project(&s, "proj");
        let task = hive_tasks::new(&s, "proj", "Ship it", "the brief", 0, "human").unwrap();
        let call =
            |method: &str, path: &str| http::additions_request(http::handler(&s), method, path, "");
        let path = format!("/api/v1/tasks/{}/lifecycle", task.id);

        // A new task sits in Intake and waits on the lead.
        let (status, v) = call("GET", &path);
        assert_eq!(status, 200, "{v:?}");
        assert_eq!(v["task"], task.id);
        assert_eq!(v["phase"], "intake");
        assert_eq!(v["waits_on"], "lead");
        let steps = v["steps"].as_array().unwrap();
        let names: Vec<&str> = steps.iter().map(|p| p["phase"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            ["intake", "shape", "build", "verify", "review", "deliver"]
        );
        assert_eq!(steps[0]["state"], "current");
        assert_eq!(steps[0]["left"][0]["kind"], "task");
        assert_eq!(steps[0]["left"][0]["id"], task.id);
        assert_ne!(steps[0]["gate"], "");
        // The phases come with the words the console heads them with.
        assert_eq!(v["phases"][1]["label"], "Shape and plan");
        assert_eq!(v["phases"].as_array().unwrap().len(), 6);

        // A step at work: Build, and its job links to its run.
        let source = format!("task:{}", task.id);
        insert_job(&s, "job-a", "task", &source, "leased");
        s.conn()
            .unwrap()
            .execute(
                "INSERT INTO harness_sessions (id, job_id, project, started_at, updated_at) \
                 VALUES ('run-a', 'job-a', 'proj', '2026-11-26T10:01:00Z', '2026-11-26T10:01:00Z')",
                [],
            )
            .unwrap();
        hive_tasks::set_status(&s, &task.id, "in_progress", "daemon").unwrap();
        let (status, v) = call("GET", &path);
        assert_eq!(status, 200, "{v:?}");
        assert_eq!(v["phase"], "build");
        let build = &v["steps"][2];
        assert_eq!(build["state"], "current");
        assert_eq!(build["left"][0]["kind"], "job");
        assert_eq!(build["left"][0]["id"], "job-a");
        assert_eq!(build["left"][0]["session"], "run-a");

        // Unknown task, wrong project, wrong method.
        let (status, _) = call("GET", "/api/v1/tasks/TASK-999/lifecycle");
        assert_eq!(status, 404);
        let (status, _) = call("GET", &format!("{path}?project=other"));
        assert_eq!(status, 404);
        let (status, _) = call("POST", &path);
        assert_eq!(status, 405);
    }

    #[test]
    fn project_lifecycle_route() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(false);
        http::additions_project(&s, "proj");
        http::additions_project(&s, "other");
        let fresh = hive_tasks::new(&s, "proj", "Fresh", "brief", 0, "human").unwrap();
        let busy = hive_tasks::new(&s, "proj", "Busy", "brief", 0, "human").unwrap();
        let dropped = hive_tasks::new(&s, "proj", "Dropped", "brief", 0, "human").unwrap();
        hive_tasks::new(&s, "other", "Elsewhere", "brief", 0, "human").unwrap();
        insert_job(&s, "job-b", "task", &format!("task:{}", busy.id), "leased");
        hive_tasks::set_status(&s, &busy.id, "in_progress", "daemon").unwrap();
        hive_tasks::cancel(&s, &dropped.id, "not needed", "human").unwrap();
        let call =
            |method: &str, path: &str| http::additions_request(http::handler(&s), method, path, "");

        let (status, v) = call("GET", "/api/v1/tasks/lifecycle?project=proj");
        assert_eq!(status, 200, "{v:?}");
        assert_eq!(v["project"], "proj");
        let phases = v["phases"].as_array().unwrap();
        let names: Vec<&str> = phases
            .iter()
            .map(|p| p["phase"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "intake",
                "shape",
                "build",
                "verify",
                "review",
                "deliver",
                "done",
                "cancelled"
            ]
        );
        let ids = |phase: &str| -> Vec<String> {
            phases.iter().find(|p| p["phase"] == phase).unwrap()["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t.as_str().unwrap().to_string())
                .collect()
        };
        assert_eq!(ids("intake"), vec![fresh.id.clone()]);
        assert_eq!(ids("build"), vec![busy.id.clone()]);
        assert_eq!(ids("cancelled"), vec![dropped.id.clone()]);
        assert!(ids("deliver").is_empty());
        // One row per task of the project, none of the other project's.
        let tasks = v["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 3);
        let busy_row = tasks
            .iter()
            .find(|t| t["task"] == busy.id.as_str())
            .unwrap();
        assert_eq!(busy_row["phase"], "build");
        assert_eq!(busy_row["waits_on"], "agent");
        assert!(busy_row.get("steps").is_none());

        // The project is required and must exist; GET only.
        let (status, _) = call("GET", "/api/v1/tasks/lifecycle");
        assert_eq!(status, 400);
        let (status, _) = call("GET", "/api/v1/tasks/lifecycle?project=nope");
        assert_eq!(status, 404);
        let (status, _) = call("POST", "/api/v1/tasks/lifecycle?project=proj");
        assert_eq!(status, 405);
        // `lifecycle` is not taken for a task id by the task detail route.
        let (status, v) = call("GET", "/api/v1/tasks/lifecycle?project=other");
        assert_eq!(status, 200, "{v:?}");
        assert_eq!(v["tasks"].as_array().unwrap().len(), 1);
    }
}
