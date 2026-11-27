//! The owner's side of the pull request flow: where every pull request
//! Hive opened for a project stands (`GET /api/v1/projects/<slug>/prs`,
//! `hive_tasks::pr_states_for`) and the one button the owner needs,
//! Merge (`POST /api/v1/projects/<slug>/prs/<number>/merge`, `POST
//! /api/v1/tasks/<id>/pr/merge`). The hub does not merge anything itself:
//! it records the ask as a `review.ask` event and the reviewer's daemon
//! pass merges the pull request on GitHub as the owner, so the forge
//! credentials and the checkout stay with the one process that has them.
use crate::additions::{
    ADDITION_WRITES, addition_failure, addition_method, addition_not_found, addition_row_string,
};
use crate::httpx::{Request, ResponseWriter};
use crate::server::{Handler, write_error, write_json};
use hive_tasks::PrState;
use serde_json::{Value, json};

impl Handler {
    /// `/api/v1/projects/<slug>/prs` and `.../prs/<number>/merge`.
    pub(crate) fn addition_project_prs(
        &self,
        w: &mut ResponseWriter,
        r: &Request,
        parts: &[String],
    ) {
        let merge = parts.len() == 5 && parts[4] == "merge";
        if parts.len() != 3 && !merge {
            addition_not_found(w);
            return;
        }
        let methods = if merge { vec!["POST"] } else { vec!["GET"] };
        if !addition_method(w, r, methods.into_iter().map(String::from).collect())
            || (merge && !crate::server::same_origin_write(w, r))
            || !self.addition_available(w, r, vec!["projects".into()])
        {
            return;
        }
        let Some(store) = self.store.as_deref() else {
            return;
        };
        let slug = parts[1].as_str();
        match store.project(slug) {
            Ok(Some(_)) => {}
            Ok(None) => {
                write_error(
                    w,
                    404,
                    "No such project".to_string(),
                    format!("There is no project {slug}."),
                    "Pick one from the project list.".to_string(),
                );
                return;
            }
            Err(e) => {
                addition_failure(w, e.to_string());
                return;
            }
        }
        if !merge {
            match hive_tasks::pr_states_for(store, slug) {
                Ok(prs) => write_json(w, 200, json!({"prs": prs})),
                Err(e) => addition_failure(w, e.to_string()),
            }
            return;
        }
        let Ok(number) = parts[3].parse::<i64>() else {
            write_error(
                w,
                400,
                "Invalid pull request number".to_string(),
                format!("{} is not a pull request number.", parts[3]),
                "Use the number from the project's pull request list.".to_string(),
            );
            return;
        };
        let _lock = ADDITION_WRITES.lock().unwrap_or_else(|e| e.into_inner());
        let find = |prs: Vec<PrState>| prs.into_iter().find(|p| p.number == number);
        let pr = match hive_tasks::pr_states_for(store, slug).map(find) {
            Ok(Some(pr)) => pr,
            Ok(None) => {
                write_error(
                    w,
                    404,
                    "No such pull request".to_string(),
                    format!(
                        "Hive opened no pull request #{number} for {slug}, or it was merged or closed more than two weeks ago."
                    ),
                    "Pick one from the project's pull request list.".to_string(),
                );
                return;
            }
            Err(e) => {
                addition_failure(w, e.to_string());
                return;
            }
        };
        if !pr.can_merge {
            let (why, fix) = merge_refusal(&pr.state, pr.number, &pr.note, &pr.follow_up);
            write_error(w, 409, "Nothing to merge".to_string(), why, fix);
            return;
        }
        if let Some((why, fix)) = stale_press(r, number, &pr.sha) {
            write_error(w, 409, "The pull request moved".to_string(), why, fix);
            return;
        }
        if let Err(e) = hive_tasks::ask_merge(
            store,
            slug,
            number,
            &pr.task,
            &pr.sha,
            hive_store::HUMAN_AGENT,
        ) {
            addition_failure(w, e.to_string());
            return;
        }
        match hive_tasks::pr_states_for(store, slug).map(find) {
            Ok(Some(pr)) => write_json(w, 202, json!({"pr": pr})),
            Ok(None) => write_json(w, 202, json!({"pr": Value::Null})),
            Err(e) => addition_failure(w, e.to_string()),
        }
    }

    /// `POST /api/v1/tasks/<id>/pr/merge`: the owner's Merge for the task's
    /// own pull request. Answers with the task and its refreshed delivery,
    /// like the task sheet reads them.
    pub(crate) fn addition_task_merge(
        &self,
        w: &mut ResponseWriter,
        r: &Request,
        store: &hive_store::Store,
        id: &str,
    ) {
        let task = match self.addition_task(r, id) {
            Ok(v) => v,
            Err(e) => {
                addition_failure(w, e.to_string());
                return;
            }
        };
        let delivery = match hive_tasks::delivery_for(store, id) {
            Ok(v) => v,
            Err(e) => {
                addition_failure(w, e.to_string());
                return;
            }
        };
        if !delivery.can_merge {
            let (why, fix) = merge_refusal(
                &delivery.state,
                delivery.number,
                &delivery.note,
                &delivery.follow_up,
            );
            write_error(w, 409, "Nothing to merge".to_string(), why, fix);
            return;
        }
        if let Some((why, fix)) = stale_press(r, delivery.number, &delivery.head_sha) {
            write_error(w, 409, "The pull request moved".to_string(), why, fix);
            return;
        }
        let project = addition_row_string(&task, "project");
        if let Err(e) = hive_tasks::ask_merge(
            store,
            &project,
            delivery.number,
            id,
            &delivery.head_sha,
            hive_store::HUMAN_AGENT,
        ) {
            addition_failure(w, e.to_string());
            return;
        }
        let delivery = match hive_tasks::delivery_for(store, id) {
            Ok(v) => serde_json::to_value(v).unwrap_or(Value::Null),
            Err(e) => {
                addition_failure(w, e.to_string());
                return;
            }
        };
        write_json(w, 202, json!({"task": task, "delivery": delivery}));
    }
}

/// The commit the console showed on the card the owner pressed Merge on
/// (`{"sha": "..."}`); empty when the press names none (an older console,
/// a script), and the hub then judges the press on the head it holds.
fn pressed_sha(r: &Request) -> String {
    serde_json::from_slice::<Value>(&r.body)
        .ok()
        .and_then(|v| v["sha"].as_str().map(|s| s.trim().to_string()))
        .unwrap_or_default()
}

/// Why a Merge pressed for a commit that is no longer the pull request's
/// head does nothing, as (why, fix): a newer commit came back to "needs
/// your review" while the card was open, and the press was for the commit
/// the card showed, not for that one. None when the press names no commit
/// or names the head Hive holds.
fn stale_press(r: &Request, number: i64, head: &str) -> Option<(String, String)> {
    let pressed = pressed_sha(r);
    if pressed.is_empty() || head.is_empty() || pressed == head {
        return None;
    }
    let short = |s: &str| s.chars().take(12).collect::<String>();
    Some((
        format!(
            "Merge was pressed for commit {} of PR #{number}; the pull request is at {} now.",
            short(&pressed),
            short(head)
        ),
        "Nothing was merged. Look at the new commit on the card, then press Merge again if it is still yours to merge."
            .to_string(),
    ))
}

/// Why the owner's Merge does nothing in this state, as (why, fix). Every
/// state but `needs_owner`, `review_failed` and `merge_waiting` (the
/// reviewer's approved merge waits for its checks or the forge; the
/// owner's Merge is the same merge, tried at once) has the reviewer or the
/// daemon still acting, and a second ask would race it.
fn merge_refusal(state: &str, number: i64, note: &str, follow_up: &str) -> (String, String) {
    match state {
        "pr_open" => (
            format!("PR #{number} is open and waits for its review."),
            "The reviewer reads it on the daemon's next pass and merges it or hands it to you."
                .to_string(),
        ),
        "in_review" => (
            format!("PR #{number} is being reviewed."),
            "Wait for the reviewer's verdict; it merges or hands the pull request to you."
                .to_string(),
        ),
        "merge_asked" => (
            format!("You already asked for PR #{number} to be merged."),
            "The daemon merges it on its next pass.".to_string(),
        ),
        "changes_requested" => (
            format!(
                "The reviewer asked for changes on PR #{number}{}.",
                if note.is_empty() {
                    String::new()
                } else {
                    format!(": {note}")
                }
            ),
            if follow_up.is_empty() {
                "The pull request comes back for review once they are made.".to_string()
            } else {
                format!("Follow-up {follow_up} makes them; the pull request comes back for review.")
            },
        ),
        "merged" => (
            format!("PR #{number} is merged already."),
            "Nothing is left to merge.".to_string(),
        ),
        "closed" => (
            format!("PR #{number} was closed without merging."),
            "The next finished work opens a new one.".to_string(),
        ),
        "held" => (
            format!(
                "The pull request could not be opened{}.",
                if note.is_empty() {
                    String::new()
                } else {
                    format!(": {note}")
                }
            ),
            "The daemon tries again on its next pass.".to_string(),
        ),
        "held_for_owner" => (
            format!(
                "The pull request could not be opened{}.",
                if note.is_empty() {
                    String::new()
                } else {
                    format!(": {note}")
                }
            ),
            "The daemon tries again only once the work changes: resolve it on the branch, or tell the lead what to do on the task's thread."
                .to_string(),
        ),
        _ => (
            "This task has no open pull request.".to_string(),
            "The daemon opens one once every step is done; it can be merged after its review."
                .to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers_http as http;

    fn emit(s: &hive_store::Store, action: &str, detail: &str) {
        hive_bus::emit(s, "daemon", action, "one", detail).unwrap();
    }

    #[test]
    fn the_owner_merges_a_tasks_pull_request_once_the_reviewer_hands_it_over() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(false);
        http::additions_project(&s, "one");
        let task =
            hive_tasks::new(&s.store, "one", "Impressum page", "add it", 0, "human").unwrap();
        let id = task.id.clone();
        let merge = || {
            http::additions_request(
                http::handler(&s),
                "POST",
                &format!("/api/v1/tasks/{id}/pr/merge"),
                "{}",
            )
        };
        // No pull request yet: nothing to merge.
        let (status, result) = merge();
        assert_eq!(status, 409, "{result:?}");
        assert_eq!(result["error"], "Nothing to merge");

        s.store
            .conn()
            .unwrap()
            .execute(
                "INSERT INTO jobs (id, project, kind, title, source, status, created_at, created_by, branch) VALUES \
                 ('JOB-imp', 'one', 'task', 'Add the page', ?, 'done', '2026-10-30T14:00:00Z', 'one-lead', 'hive/worker/JOB-imp')",
                [format!("task:{id}")],
            )
            .unwrap();
        emit(
            &s.store,
            "land.pr",
            &format!(
                r#"{{"project":"one","kind":"task","task":"{id}","number":7,"url":"https://github.com/o/r/pull/7","head":"hive/task/{id}","base":"main","remote":"origin","to":"abc123","opened":true,"title":"Impressum page"}}"#
            ),
        );
        let (status, result) =
            http::additions_request(http::handler(&s), "GET", &format!("/api/v1/tasks/{id}"), "");
        assert_eq!(status, 200);
        assert_eq!(result["delivery"]["state"], "pr_open");
        assert_eq!(result["delivery"]["can_merge"], false);
        // Open and under review: the reviewer is acting, the button waits.
        let (status, result) = merge();
        assert_eq!(status, 409, "{result:?}");
        assert!(
            result["why"]
                .as_str()
                .unwrap()
                .contains("waits for its review"),
            "{result:?}"
        );
        emit(
            &s.store,
            "review.queued",
            r#"{"project":"one","number":7,"url":"https://github.com/o/r/pull/7","job":"JOB-rev","kind":"task","reasons":[]}"#,
        );
        assert_eq!(merge().0, 409);

        emit(
            &s.store,
            "review.owner",
            r#"{"project":"one","number":7,"url":"https://github.com/o/r/pull/7","reasons":["touches 40 files"],"note":"the schema change needs you"}"#,
        );
        let (status, result) =
            http::additions_request(http::handler(&s), "GET", &format!("/api/v1/tasks/{id}"), "");
        assert_eq!(status, 200);
        assert_eq!(result["delivery"]["state"], "needs_owner");
        assert_eq!(result["delivery"]["can_merge"], true);
        assert_eq!(result["delivery"]["reasons"][0], "touches 40 files");
        assert_eq!(result["delivery"]["note"], "the schema change needs you");

        let (status, result) = merge();
        assert_eq!(status, 202, "{result:?}");
        assert_eq!(result["delivery"]["state"], "merge_asked");
        assert_eq!(result["delivery"]["can_merge"], false);
        assert_eq!(result["task"]["id"], id);
        let asks: Vec<(String, String, String)> = s
            .store
            .conn()
            .unwrap()
            .prepare("SELECT actor, subject, detail FROM events WHERE action = 'review.ask'")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(asks.len(), 1);
        assert_eq!((asks[0].0.as_str(), asks[0].1.as_str()), ("human", "one"));
        let detail: Value = serde_json::from_str(&asks[0].2).unwrap();
        assert_eq!(
            detail,
            json!({"project": "one", "number": 7, "action": "merge", "task": id, "sha": "abc123"}),
            "the ask names the head the owner pressed Merge for"
        );
        // Asked once; the daemon answers before another ask is taken.
        let (status, result) = merge();
        assert_eq!(status, 409, "{result:?}");
        assert!(
            result["why"]
                .as_str()
                .unwrap()
                .starts_with("You already asked"),
            "{result:?}"
        );
        emit(
            &s.store,
            "review.merged",
            r#"{"project":"one","number":7,"url":"https://github.com/o/r/pull/7","by":"owner"}"#,
        );
        let (status, result) =
            http::additions_request(http::handler(&s), "GET", &format!("/api/v1/tasks/{id}"), "");
        assert_eq!(status, 200);
        assert_eq!(result["delivery"]["state"], "merged");
        assert_eq!(merge().0, 409);
        let (status, _) = http::additions_request(
            http::handler(&s),
            "GET",
            &format!("/api/v1/tasks/{id}/pr/merge"),
            "",
        );
        assert_eq!(status, 405);
    }

    #[test]
    fn the_owner_may_merge_a_pull_request_whose_approved_merge_waits() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(false);
        http::additions_project(&s, "one");
        let task =
            hive_tasks::new(&s.store, "one", "Impressum page", "add it", 0, "human").unwrap();
        let id = task.id.clone();
        emit(
            &s.store,
            "land.pr",
            &format!(
                r#"{{"project":"one","kind":"task","task":"{id}","number":7,"url":"https://github.com/o/r/pull/7","head":"hive/task/{id}","base":"main","remote":"origin","to":"abc123","opened":true,"title":"Impressum page"}}"#
            ),
        );
        emit(
            &s.store,
            "review.merge-wait",
            r#"{"project":"one","number":7,"waiting":"forge","attempt":1,"due":"2026-10-31T13:10:00Z","note":"approved; GitHub refused the merge (HTTP 502); retried at 2026-10-31T13:10:00Z (attempt 2)"}"#,
        );
        let (status, result) =
            http::additions_request(http::handler(&s), "GET", &format!("/api/v1/tasks/{id}"), "");
        assert_eq!(status, 200);
        assert_eq!(result["delivery"]["state"], "merge_waiting");
        assert_eq!(result["delivery"]["can_merge"], true);
        let (status, result) = http::additions_request(
            http::handler(&s),
            "POST",
            &format!("/api/v1/tasks/{id}/pr/merge"),
            "{}",
        );
        assert_eq!(status, 202, "{result:?}");
        assert_eq!(result["delivery"]["state"], "merge_asked");
    }

    /// The console sends the commit of the card the owner pressed Merge on;
    /// a press for a commit that is no longer the head is refused and says
    /// so, on the project's card and on the task's sheet alike. Nothing is
    /// asked of the daemon then.
    #[test]
    fn a_merge_pressed_for_a_commit_that_is_no_longer_the_head_is_refused() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(false);
        http::additions_project(&s, "one");
        let task = hive_tasks::new(&s.store, "one", "Footer", "add it", 0, "human").unwrap();
        let id = task.id.clone();
        emit(
            &s.store,
            "land.pr",
            &format!(
                r#"{{"project":"one","kind":"task","task":"{id}","number":8,"url":"https://github.com/o/r/pull/8","head":"hive/task/{id}","base":"main","to":"abc123abc123abc1","opened":true,"title":"Footer"}}"#
            ),
        );
        emit(
            &s.store,
            "review.owner",
            r#"{"project":"one","number":8,"url":"https://github.com/o/r/pull/8","reasons":["touches 40 files"]}"#,
        );
        // A newer commit came back to "needs your review" while the card
        // showed the old one.
        emit(
            &s.store,
            "land.pr",
            &format!(
                r#"{{"project":"one","kind":"task","task":"{id}","number":8,"url":"https://github.com/o/r/pull/8","head":"hive/task/{id}","base":"main","to":"def456def456def4","title":"Footer"}}"#
            ),
        );
        emit(
            &s.store,
            "review.owner",
            r#"{"project":"one","number":8,"url":"https://github.com/o/r/pull/8","reasons":["touches 40 files"]}"#,
        );
        let asks = || -> Vec<Value> {
            s.store
                .conn()
                .unwrap()
                .prepare("SELECT detail FROM events WHERE action = 'review.ask' ORDER BY seq")
                .unwrap()
                .query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .map(|d| serde_json::from_str(&d.unwrap()).unwrap())
                .collect()
        };
        for path in [
            "/api/v1/projects/one/prs/8/merge".to_string(),
            format!("/api/v1/tasks/{id}/pr/merge"),
        ] {
            let (status, result) = http::additions_request(
                http::handler(&s),
                "POST",
                &path,
                r#"{"sha":"abc123abc123abc1"}"#,
            );
            assert_eq!(status, 409, "{path}: {result:?}");
            assert_eq!(result["error"], "The pull request moved", "{result:?}");
            let why = result["why"].as_str().unwrap();
            assert!(
                why.contains("pressed for commit abc123abc123")
                    && why.contains("at def456def456 now"),
                "{why}"
            );
            assert!(asks().is_empty(), "{path}: a stale press asks nothing");
        }
        // The card's commit is the head: the ask is for it.
        let (status, result) = http::additions_request(
            http::handler(&s),
            "POST",
            "/api/v1/projects/one/prs/8/merge",
            r#"{"sha":"def456def456def4"}"#,
        );
        assert_eq!(status, 202, "{result:?}");
        assert_eq!(asks()[0]["sha"], "def456def456def4");
    }

    #[test]
    fn a_projects_pull_requests_are_listed_and_merged_by_number() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(false);
        http::additions_project(&s, "one");
        let (status, result) =
            http::additions_request(http::handler(&s), "GET", "/api/v1/projects/one/prs", "");
        assert_eq!(status, 200, "{result:?}");
        assert_eq!(result["prs"], json!([]));
        let (status, _) =
            http::additions_request(http::handler(&s), "GET", "/api/v1/projects/nine/prs", "");
        assert_eq!(status, 404);

        // A conversation's branch proposed as is, and a task's pull request
        // in review.
        emit(
            &s.store,
            "land.pr",
            r#"{"project":"one","kind":"proposed","conversation":"chat-1","number":8,"url":"https://github.com/o/r/pull/8","head":"hive/chat-chat-1","base":"main","title":"Chat: the footer","opened":true}"#,
        );
        emit(
            &s.store,
            "land.pr",
            r#"{"project":"one","kind":"task","task":"TASK-a","number":9,"url":"https://github.com/o/r/pull/9","head":"hive/task/TASK-a","base":"main","title":"Add the page","opened":true}"#,
        );
        emit(
            &s.store,
            "review.queued",
            r#"{"project":"one","number":9,"url":"https://github.com/o/r/pull/9","job":"JOB-rev","kind":"task","reasons":[]}"#,
        );
        let (status, result) =
            http::additions_request(http::handler(&s), "GET", "/api/v1/projects/one/prs", "");
        assert_eq!(status, 200, "{result:?}");
        let prs = result["prs"].as_array().unwrap();
        let seen: Vec<(i64, &str)> = prs
            .iter()
            .map(|p| (p["number"].as_i64().unwrap(), p["state"].as_str().unwrap()))
            .collect();
        assert_eq!(seen, vec![(9, "in_review"), (8, "pr_open")]);
        assert_eq!(prs[1]["conversation"], "chat-1");
        assert_eq!(prs[1]["kind"], "proposed");

        // Open, in review, unknown or malformed: nothing to merge.
        for (path, code) in [
            ("/api/v1/projects/one/prs/8/merge", 409),
            ("/api/v1/projects/one/prs/9/merge", 409),
            ("/api/v1/projects/one/prs/77/merge", 404),
            ("/api/v1/projects/one/prs/seven/merge", 400),
            ("/api/v1/projects/one/prs/8/close", 404),
        ] {
            let (status, result) = http::additions_request(http::handler(&s), "POST", path, "{}");
            assert_eq!(status, code, "{path}: {result:?}");
        }
        let (status, _) = http::additions_request(
            http::handler(&s),
            "GET",
            "/api/v1/projects/one/prs/8/merge",
            "",
        );
        assert_eq!(status, 405);

        // The reviewer hands the conversation's pull request to the owner.
        emit(
            &s.store,
            "review.queued",
            r#"{"project":"one","number":8,"url":"https://github.com/o/r/pull/8","job":"JOB-rev2","kind":"proposed","reasons":["no tests"]}"#,
        );
        let (status, result) = http::additions_request(
            http::handler(&s),
            "POST",
            "/api/v1/projects/one/prs/8/merge",
            "{}",
        );
        assert_eq!(status, 202, "{result:?}");
        assert_eq!(result["pr"]["state"], "merge_asked");
        assert_eq!(result["pr"]["number"], 8);
        assert_eq!(result["pr"]["can_merge"], false);
        let detail: String = s
            .store
            .conn()
            .unwrap()
            .query_row(
                "SELECT detail FROM events WHERE action = 'review.ask'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&detail).unwrap(),
            json!({"project": "one", "number": 8, "action": "merge"}),
            "a pull request without a task names none"
        );
        let (status, _) = http::additions_request(
            http::handler(&s),
            "POST",
            "/api/v1/projects/one/prs/8/merge",
            "{}",
        );
        assert_eq!(status, 409, "asked once");

        // A failed merge offers the button again.
        emit(
            &s.store,
            "review.failed",
            r#"{"project":"one","number":8,"url":"https://github.com/o/r/pull/8","error":"gh pr merge: not mergeable"}"#,
        );
        let (status, result) =
            http::additions_request(http::handler(&s), "GET", "/api/v1/projects/one/prs", "");
        assert_eq!(status, 200);
        let pr = result["prs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["number"] == 8)
            .unwrap()
            .clone();
        assert_eq!(pr["state"], "review_failed");
        assert_eq!(pr["note"], "gh pr merge: not mergeable");
        assert_eq!(pr["can_merge"], true);
    }

    #[test]
    fn asking_for_a_pull_request_by_hand_is_refused() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(false);
        http::additions_project(&s, "one");
        let task =
            hive_tasks::new(&s.store, "one", "Impressum page", "add it", 0, "human").unwrap();
        let (status, result) = http::additions_request(
            http::handler(&s),
            "POST",
            &format!("/api/v1/tasks/{}/pull-request", task.id),
            "{}",
        );
        assert_eq!(status, 409, "{result:?}");
        assert!(
            result["why"].as_str().unwrap().contains("automatically"),
            "{result:?}"
        );
        assert!(!hive_tasks::pull_request_asked(&s.store, "one"));
    }
}
