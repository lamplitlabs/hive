//! A task's work delivered as one pull request. When every job of a task
//! is done, the daemon merges the jobs' branches onto the default branch in
//! a scratch worktree as `pulse/task-<id>`, pushes that branch and opens one
//! pull request for it with `gh` (`land.pr`, detail `task`). Nothing lands
//! on the local default branch: the forge's own checks are the build signal,
//! and a person (or a review lead) merges.
//!
//! The same maintenance pass polls each open pull request with
//! `gh pr view <n> --json state,mergedAt,url`. Merged: `land.merged`, the
//! jobs' branches are cleared (nothing of the task waits any more), the
//! local default branch is fast-forwarded from its upstream and the task is
//! done. Closed unmerged: `land.pr-closed`, the branches are cleared too and
//! the task goes back to in_progress with a note, so its lead plans again.

use std::time::Duration;

use serde_json::{Value, json};

use crate::util::Row;

pub(crate) const ACTOR: &str = "daemon";

/// The branch a task's pull request comes from.
pub(crate) fn task_branch(task: &str) -> String {
    format!("pulse/task-{task}")
}

/// One pass over the projects: open the pull requests now owed, then read
/// the state of every open one. Returns the log lines.
pub(crate) fn tend_task_prs(s: &pulse_store::Store, projects: &[Row], now: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for p in projects {
        if p.str("path").is_empty() {
            continue;
        }
        lines.extend(deliver_ready(s, p, now));
        lines.extend(poll_open(s, p, now));
    }
    lines
}

/// The tasks of a project whose jobs are all finished, with at least one
/// done job whose branch has not been delivered, and no pull request open.
/// (task id, title, its done jobs as (job id, branch)).
type ReadyTask = (String, String, Vec<(String, String)>);

fn ready_tasks(s: &pulse_store::Store, slug: &str) -> Vec<ReadyTask> {
    let Ok(conn) = s.conn() else {
        return Vec::new();
    };
    let tasks = crate::util::query_rows(
        &conn,
        "SELECT id, title FROM tasks WHERE project = ? AND status NOT IN ('done', 'cancelled') ORDER BY id",
        &[&slug],
    )
    .unwrap_or_default();
    let mut out = Vec::new();
    for t in tasks {
        let id = t.str("id");
        if open_pr(&conn, slug, &id).is_some() {
            continue;
        }
        let (exact, like) = pulse_store::task_source_params(&id);
        let jobs = crate::util::query_rows(
            &conn,
            &format!(
                "SELECT id, status, branch FROM jobs WHERE project = ? AND {} ORDER BY created_at, id",
                pulse_store::task_source_clause("source")
            ),
            &[&slug, &exact, &like],
        )
        .unwrap_or_default();
        if jobs
            .iter()
            .any(|j| matches!(j.str("status").as_str(), "queued" | "leased" | "blocked"))
        {
            continue;
        }
        let branches: Vec<(String, String)> = jobs
            .iter()
            .filter(|j| j.str("status") == "done" && !j.str("branch").is_empty())
            .map(|j| (j.str("id"), j.str("branch")))
            .collect();
        if !branches.is_empty() {
            out.push((id, t.str("title"), branches));
        }
    }
    out
}

/// The task's pull request still open as far as PULSE knows: the newest
/// `land.pr` of the task with no `land.merged` or `land.pr-closed` after it,
/// as its event detail.
pub(crate) fn open_pr(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    task: &str,
) -> Option<Value> {
    let rows = crate::util::query_rows(
        q,
        "SELECT action, detail FROM events WHERE subject = ? AND action IN ('land.pr', 'land.merged', 'land.pr-closed')
         AND json_extract(detail, '$.task') = ? ORDER BY seq DESC LIMIT 1",
        &[&slug, &task],
    )
    .unwrap_or_default();
    let last = rows.first()?;
    (last.str("action") == "land.pr")
        .then(|| serde_json::from_str(&last.str("detail")).unwrap_or(Value::Null))
}

/// The task's last pull request was closed unmerged and no job of the task
/// has been queued since: the lead plans again before the task can close.
pub(crate) fn closed_awaiting_work(s: &pulse_store::Store, slug: &str, task: &str) -> bool {
    let Ok(conn) = s.conn() else {
        return false;
    };
    let last = crate::util::query_rows(
        &conn,
        "SELECT action, at FROM events WHERE subject = ? AND action IN ('land.pr', 'land.merged', 'land.pr-closed')
         AND json_extract(detail, '$.task') = ? ORDER BY seq DESC LIMIT 1",
        &[&slug, &task],
    )
    .unwrap_or_default();
    let Some(last) = last.first() else {
        return false;
    };
    if last.str("action") != "land.pr-closed" {
        return false;
    }
    let (exact, like) = pulse_store::task_source_params(task);
    let at = last.str("at");
    crate::util::query_rows(
        &conn,
        &format!(
            "SELECT id FROM jobs WHERE project = ? AND created_at > ? AND {} LIMIT 1",
            pulse_store::task_source_clause("source")
        ),
        &[&slug, &at, &exact, &like],
    )
    .unwrap_or_default()
    .is_empty()
}

fn event(s: &pulse_store::Store, now: &str, action: &str, slug: &str, detail: Value) {
    if let Ok(conn) = s.conn() {
        crate::util::event_at(&conn, now, ACTOR, action, slug, detail);
    }
}

/// A hold already recorded for exactly these branches: the pass does not
/// retry a conflict or a push failure until the task's work changes.
fn held_already(s: &pulse_store::Store, slug: &str, task: &str, key: &str) -> bool {
    let Ok(conn) = s.conn() else {
        return false;
    };
    crate::util::query_rows(
        &conn,
        "SELECT action, detail FROM events WHERE subject = ? AND action IN ('land.pr', 'land.pr-held', 'land.merged', 'land.pr-closed')
         AND json_extract(detail, '$.task') = ? ORDER BY seq DESC LIMIT 1",
        &[&slug, &task],
    )
    .unwrap_or_default()
    .first()
    .is_some_and(|r| {
        r.str("action") == "land.pr-held"
            && serde_json::from_str::<Value>(&r.str("detail"))
                .ok()
                .and_then(|v| v.get("work").and_then(|w| w.as_str()).map(str::to_string))
                .as_deref()
                == Some(key)
    })
}

fn deliver_ready(s: &pulse_store::Store, project: &Row, now: &str) -> Vec<String> {
    let slug = project.str("slug");
    let mut lines = Vec::new();
    for (task, title, branches) in ready_tasks(s, &slug) {
        let key = branches
            .iter()
            .map(|(_, b)| b.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        if held_already(s, &slug, &task, &key) {
            continue;
        }
        match deliver_task(s, project, &task, &title, &branches, now) {
            Ok(line) => lines.push(line),
            Err(note) => {
                event(
                    s,
                    now,
                    "land.pr-held",
                    &slug,
                    json!({"project": slug, "task": task, "work": key, "note": note}),
                );
                lines.push(format!("task pr {slug} {task}: held: {note}"));
            }
        }
    }
    lines
}

/// Builds `pulse/task-<id>` from the default branch plus every job branch,
/// pushes it and opens (or finds) its pull request.
fn deliver_task(
    s: &pulse_store::Store,
    project: &Row,
    task: &str,
    title: &str,
    branches: &[(String, String)],
    now: &str,
) -> Result<String, String> {
    let slug = project.str("slug");
    let root = project.str("path");
    let def = crate::baseline::default_branch(project.clone());
    let Some((remote, _, remote_branch)) = crate::forge::tracking_of(&root, &def) else {
        return Err(format!(
            "{def} has no upstream to open a pull request against (git branch --set-upstream-to=origin/{def} {def})"
        ));
    };
    let head = task_branch(task);
    let git = |args: &[&str]| crate::util::git_timeout(&root, Duration::from_secs(120), args);
    let git_dir = git(&["rev-parse", "--absolute-git-dir"])
        .stdout
        .trim()
        .to_string();
    if git_dir.is_empty() {
        return Err(format!("{root} is not a git checkout"));
    }
    let wt_path = format!("{git_dir}/pulse-task-pr/{task}");
    let _ = git(&["worktree", "remove", "--force", &wt_path]);
    let _ = std::fs::remove_dir_all(&wt_path);
    let add = git(&["worktree", "add", "--force", "-B", &head, &wt_path, &def]);
    if !add.ok() {
        return Err(format!(
            "worktree add failed: {}",
            crate::run::squeeze(&add.stderr, 200)
        ));
    }
    let in_wt = |args: &[&str]| crate::util::git_timeout(&wt_path, Duration::from_secs(300), args);
    let mut result: Result<(), String> = Ok(());
    for (_, b) in branches {
        let m = in_wt(&[
            "-c",
            "user.name=PULSE",
            "-c",
            "user.email=pulse@localhost",
            "merge",
            "--no-ff",
            "--no-edit",
            "-m",
            &format!("Merge {b} into {head}"),
            b,
        ]);
        if !m.ok() {
            let _ = in_wt(&["merge", "--abort"]);
            result = Err(format!(
                "{b} does not merge cleanly onto {def} with the task's other branches: {}",
                crate::run::squeeze(&format!("{} {}", m.stdout, m.stderr), 200)
            ));
            break;
        }
    }
    let base_sha = git(&["rev-parse", &def]).stdout.trim().to_string();
    let tip = in_wt(&["rev-parse", "HEAD"]).stdout.trim().to_string();
    let _ = git(&["worktree", "remove", "--force", &wt_path]);
    result?;

    let url = crate::forge::push_url(&root, &remote);
    let (env, cleanup, why) = crate::forge::push_credential_env(s, root.clone());
    if !why.is_empty() {
        return Err(why);
    }
    let push = crate::util::git_timeout_env(
        &root,
        Duration::from_secs(300),
        &env,
        &[
            "push",
            "--force",
            &url,
            &format!("refs/heads/{head}:refs/heads/{head}"),
        ],
    );
    cleanup("", &[]);
    if !push.ok() {
        return Err(pulse_cred::redact(
            &format!(
                "push of {head} failed: {}",
                crate::run::squeeze(&format!("{} {}", push.stderr, push.stdout), 200)
            ),
            &[],
        ));
    }
    let repo = crate::landing_pr::repo_slug(&url).unwrap_or_default();
    let (number, pr_url, opened) =
        match crate::landing_pr::open_pull_request(&root, &repo, &head, &remote_branch)? {
            Some((n, u)) => (n, u, false),
            None => {
                let body = pr_body(task, title, branches);
                let out = crate::landing_pr::gh(
                    &root,
                    &repo,
                    &[
                        "pr",
                        "create",
                        "--head",
                        &head,
                        "--base",
                        &remote_branch,
                        "--title",
                        &format!("{task}: {title}"),
                        "--body",
                        &body,
                    ],
                    Duration::from_secs(120),
                )?;
                let u = out
                    .lines()
                    .rev()
                    .map(str::trim)
                    .find(|l| l.starts_with("http"))
                    .unwrap_or_default()
                    .to_string();
                let n = u
                    .rsplit('/')
                    .next()
                    .and_then(|n| n.parse::<i64>().ok())
                    .unwrap_or(0);
                (n, u, true)
            }
        };
    if number == 0 {
        return Err(format!("gh did not name the pull request for {head}"));
    }
    let jobs: Vec<&str> = branches.iter().map(|(j, _)| j.as_str()).collect();
    event(
        s,
        now,
        "land.pr",
        &slug,
        json!({"project": slug, "task": task, "number": number, "url": pr_url, "head": head,
               "base": remote_branch, "remote": remote, "from": base_sha, "to": tip,
               "jobs": jobs, "commits": branches.len(), "opened": opened}),
    );
    Ok(format!(
        "task pr {slug} {task}: #{number} {} {pr_url}",
        if opened { "opened" } else { "updated" }
    ))
}

fn pr_body(task: &str, title: &str, branches: &[(String, String)]) -> String {
    let mut b = format!("The work of PULSE task {task}: {title}.\n\nJobs:\n");
    for (j, br) in branches {
        b.push_str(&format!("- {j} (`{br}`)\n"));
    }
    b.push_str("\nMerging this pull request closes the task; closing it unmerged sends the task back to its lead.\n");
    b
}

/// Reads every open task pull request of the project and settles the
/// merged and closed ones.
fn poll_open(s: &pulse_store::Store, project: &Row, now: &str) -> Vec<String> {
    let slug = project.str("slug");
    let root = project.str("path");
    let Ok(conn) = s.conn() else {
        return Vec::new();
    };
    let tasks: Vec<String> = crate::util::query_rows(
        &conn,
        "SELECT DISTINCT json_extract(detail, '$.task') AS task FROM events WHERE subject = ? AND action = 'land.pr'
         AND json_extract(detail, '$.task') IS NOT NULL",
        &[&slug],
    )
    .unwrap_or_default()
    .into_iter()
    .map(|r| r.str("task"))
    .collect();
    drop(conn);
    let mut lines = Vec::new();
    for task in tasks {
        let Ok(conn) = s.conn() else { break };
        let Some(pr) = open_pr(&conn, &slug, &task) else {
            continue;
        };
        drop(conn);
        let number = pr.get("number").and_then(Value::as_i64).unwrap_or(0);
        let url = pr
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let push = crate::forge::push_url(
            &root,
            pr.get("remote").and_then(Value::as_str).unwrap_or("origin"),
        );
        let repo = crate::landing_pr::repo_slug(&push).unwrap_or_default();
        let view = crate::landing_pr::gh(
            &root,
            &repo,
            &[
                "pr",
                "view",
                &number.to_string(),
                "--json",
                "state,mergedAt,url",
            ],
            Duration::from_secs(60),
        );
        let v: Value = match view.map(|o| serde_json::from_str(o.trim())) {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                lines.push(format!(
                    "task pr {slug} {task}: gh pr view answered oddly: {e}"
                ));
                continue;
            }
            Err(e) => {
                lines.push(format!("task pr {slug} {task}: {e}"));
                continue;
            }
        };
        let state = v.get("state").and_then(Value::as_str).unwrap_or_default();
        match state {
            "MERGED" => lines.push(settle_merged(s, project, &task, number, &url, &v, now)),
            "CLOSED" => lines.push(settle_closed(s, &slug, &task, number, &url, now)),
            _ => {}
        }
    }
    lines
}

fn clear_branches(s: &pulse_store::Store, slug: &str, task: &str) {
    let (exact, like) = pulse_store::task_source_params(task);
    if let Ok(conn) = s.conn() {
        let _ = conn.execute(
            &format!(
                "UPDATE jobs SET branch = '' WHERE project = ? AND status = 'done' AND {}",
                pulse_store::task_source_clause("source")
            ),
            rusqlite::params![slug, exact, like],
        );
    }
}

fn settle_merged(
    s: &pulse_store::Store,
    project: &Row,
    task: &str,
    number: i64,
    url: &str,
    v: &Value,
    now: &str,
) -> String {
    let slug = project.str("slug");
    let merged_at = v
        .get("mergedAt")
        .and_then(Value::as_str)
        .unwrap_or_default();
    event(
        s,
        now,
        "land.merged",
        &slug,
        json!({"project": slug, "task": task, "number": number, "url": url, "merged_at": merged_at}),
    );
    clear_branches(s, &slug, task);
    let mut line = format!("task pr {slug} {task}: #{number} merged; task done");
    if let Err(e) = pulse_tasks::set_status(s, task, "done", ACTOR) {
        line.push_str(&format!(" (status not changed: {e})"));
    }
    if let Some(ff) = fast_forward_from_upstream(s, project, now) {
        line.push_str(&format!("; {ff}"));
    }
    line
}

fn settle_closed(
    s: &pulse_store::Store,
    slug: &str,
    task: &str,
    number: i64,
    url: &str,
    now: &str,
) -> String {
    event(
        s,
        now,
        "land.pr-closed",
        slug,
        json!({"project": slug, "task": task, "number": number, "url": url}),
    );
    clear_branches(s, slug, task);
    let _ = pulse_tasks::set_status(s, task, "in_progress", ACTOR);
    let _ = pulse_tasks::send_message(
        s,
        slug,
        task,
        ACTOR,
        &format!(
            "Pull request #{number} ({url}) was closed without merging. The task is back in progress: its lead reads why on the pull request and plans what comes next; the branch {} stays on the remote.",
            task_branch(task)
        ),
    );
    format!("task pr {slug} {task}: #{number} closed unmerged; task back in progress")
}

/// Fetches the default branch's upstream and fast-forwards the local
/// default branch to it, so the merged work is in the checkout.
pub(crate) fn fast_forward_from_upstream(
    s: &pulse_store::Store,
    project: &Row,
    now: &str,
) -> Option<String> {
    let root = project.str("path");
    let slug = project.str("slug");
    let def = crate::baseline::default_branch(project.clone());
    let (remote, _, remote_branch) = crate::forge::tracking_of(&root, &def)?;
    let (env, cleanup, _) = crate::forge::push_credential_env(s, root.clone());
    let t = crate::forge::PushTarget {
        root: root.clone(),
        url: crate::forge::push_url(&root, &remote),
        upstream: format!("refs/remotes/{remote}/{remote_branch}"),
        remote,
        remote_branch,
        env,
    };
    let fetched = crate::forge::fetch_upstream(&t);
    cleanup("", &[]);
    if !fetched.ok() {
        return Some(format!(
            "fetch of {} failed: {}",
            t.upstream,
            crate::run::squeeze(&fetched.stderr, 160)
        ));
    }
    let behind = crate::util::git_timeout(
        &root,
        Duration::from_secs(30),
        &[
            "rev-list",
            "--count",
            &format!("refs/heads/{def}..{}", t.upstream),
        ],
    )
    .stdout
    .trim()
    .parse::<i32>()
    .unwrap_or(0);
    if behind == 0 {
        return None;
    }
    let name = format!("{}/{}", t.remote, t.remote_branch);
    crate::forge::fast_forward_default(
        s,
        &root,
        &slug,
        &def,
        &name,
        &t.upstream,
        ACTOR,
        now,
        behind,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::*;
    use crate::test_helpers_daemon::*;

    /// A `gh` on PATH that answers like GitHub: `pr list` finds nothing,
    /// `pr create` prints the url of #7, `pr view` prints the contents of
    /// `state` (OPEN until the test writes MERGED or CLOSED). Every call is
    /// logged.
    fn fake_gh() -> (
        tempfile::TempDir,
        pulse_testutil::SetenvGuard,
        String,
        std::path::PathBuf,
    ) {
        let bin = tempfile::tempdir().unwrap();
        let log = bin.path().join("gh.log").display().to_string();
        let state = bin.path().join("state");
        std::fs::write(&state, "OPEN").unwrap();
        let body = format!(
            "#!/bin/sh\necho \"$@\" >> '{log}'\ncase \"$1 $2\" in\n\"pr list\") echo '[]' ;;\n\"pr create\") echo 'https://github.com/o/r/pull/7' ;;\n\"pr view\") st=$(cat '{st}'); m=null; [ \"$st\" = MERGED ] && m='\"2026-11-04T07:00:00Z\"'; echo \"{{\\\"state\\\":\\\"$st\\\",\\\"mergedAt\\\":$m,\\\"url\\\":\\\"https://github.com/o/r/pull/7\\\"}}\" ;;\n*) echo \"unexpected gh $*\" >&2; exit 2 ;;\nesac\n",
            st = state.display()
        );
        let path = bin.path().join("gh");
        std::fs::write(&path, body).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let guard = pulse_testutil::setenv(
            "PATH",
            format!(
                "{}:{}",
                bin.path().display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        );
        (bin, guard, log, state)
    }

    /// A task with two done jobs on their own branches.
    fn task_with_two_branches(s: &pulse_store::Store, proj: &Row, repo: &str) -> String {
        let task =
            pulse_tasks::new(s, "proj", "Add the thing", "b", 0, pulse_store::HUMAN_AGENT).unwrap();
        for name in ["pulse/worker/a", "pulse/worker/b"] {
            let j = push_branch(s, proj, repo, name);
            exec1(
                s,
                "UPDATE jobs SET source = ? WHERE id = ?",
                &[&format!("task:{}", task.id), &j.str("id")],
            );
        }
        exec1(
            s,
            "UPDATE tasks SET status = 'in_progress' WHERE id = ?",
            &[&task.id],
        );
        task.id
    }

    fn status(s: &pulse_store::Store, id: &str) -> String {
        pulse_tasks::get(s, id).unwrap().0.status
    }

    fn calls(log: &str) -> Vec<String> {
        std::fs::read_to_string(log)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with("pr "))
            .map(|l| l.split_whitespace().take(2).collect::<Vec<_>>().join(" "))
            .collect()
    }

    #[test]
    fn a_finished_task_opens_one_pull_request_and_is_done_once_merged() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let (_bin, _path, log, state) = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        let task = task_with_two_branches(&s, &proj, &repo);
        let main_before = ref_of(&repo, "refs/heads/main");

        let lines = tend_task_prs(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert!(
            lines
                .iter()
                .any(|l| l.contains(&format!("{task}: #7 opened"))),
            "{lines:?}"
        );
        assert_eq!(
            ref_of(&repo, "refs/heads/main"),
            main_before,
            "nothing lands locally"
        );
        let head = bare_ref(&bare, &format!("refs/heads/{}", task_branch(&task)));
        for b in ["pulse/worker/a", "pulse/worker/b"] {
            let tip = ref_of(&repo, &format!("refs/heads/{b}"));
            run(&repo, &["git", "merge-base", "--is-ancestor", &tip, &head]);
        }
        let d = pulse_tasks::delivery_for(&s, &task).unwrap();
        assert_eq!(d.state, "pr_open", "{d:?}");
        assert_eq!(d.url, "https://github.com/o/r/pull/7");
        assert_eq!(d.number, 7);
        assert_eq!(d.waiting, 2);

        // Still open: nothing changes, no second pull request.
        let lines = tend_task_prs(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(status(&s, &task), "in_progress");
        assert_eq!(calls(&log), ["pr list", "pr create", "pr view", "pr view"]);

        // Merged on the forge: the upstream main moves, the task is done.
        run(
            &repo,
            &[
                "git",
                "push",
                "-q",
                "origin",
                &format!("{}:main", task_branch(&task)),
            ],
        );
        std::fs::write(&state, "MERGED").unwrap();
        let lines = tend_task_prs(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains("#7 merged; task done")),
            "{lines:?}"
        );
        assert_eq!(status(&s, &task), "done");
        let d = pulse_tasks::delivery_for(&s, &task).unwrap();
        assert_eq!(d.state, "merged", "{d:?}");
        assert_eq!(d.waiting, 0, "the jobs' branches are cleared");
        assert_eq!(
            ref_of(&repo, "refs/heads/main"),
            head,
            "main fast-forwarded"
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM events WHERE action = 'land.merged'",
                &[]
            ),
            1
        );
        // Settled: no more polls.
        let lines = tend_task_prs(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert!(lines.is_empty(), "{lines:?}");
    }

    #[test]
    fn a_pull_request_closed_unmerged_sends_the_task_back_to_work() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let (_bin, _path, _log, state) = fake_gh();
        let s = new_store();
        let (proj, repo, _bare) = push_fixture(&s);
        let task = task_with_two_branches(&s, &proj, &repo);
        tend_task_prs(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert_eq!(
            pulse_tasks::delivery_for(&s, &task).unwrap().state,
            "pr_open"
        );
        // Review sets it aside first; a closed pull request still goes back.
        exec1(
            &s,
            "UPDATE tasks SET status = 'review' WHERE id = ?",
            &[&task],
        );

        std::fs::write(&state, "CLOSED").unwrap();
        let lines = tend_task_prs(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains("closed unmerged")),
            "{lines:?}"
        );
        assert_eq!(status(&s, &task), "in_progress");
        assert!(closed_awaiting_work(&s, "proj", &task));
        let d = pulse_tasks::delivery_for(&s, &task).unwrap();
        assert_eq!(d.waiting, 0, "{d:?}");
        assert_ne!(d.state, "pr_open", "{d:?}");
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM messages WHERE task_id = ? AND body LIKE '%closed without merging%'",
                &[&task]
            ),
            1
        );
        // No new work: nothing is opened again.
        let lines = tend_task_prs(&s, std::slice::from_ref(&proj), TEST_NOW);
        assert!(lines.is_empty(), "{lines:?}");
    }
}
