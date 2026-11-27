//! The branches a delivery follow-up job works with, carried into its clone.
//!
//! A join job (`land.rebase` with `join`), a rebase job (`land.rebase`) and
//! a merge-fix job (`land.merge-broken`) are told to start from the task's
//! base and merge job branches into it. A job's clone is made of the
//! project's default branch alone - a local `git clone -b main` whose origin
//! is then removed, or a hand's clone of a bundle of `main` - so neither the
//! base (`hive/task-TASK-117`, the head of the pull request the task stacks
//! on) nor the job branches were there: the joins of TASK-149 and TASK-151
//! onto `hive/task-TASK-117` stopped on 27 November 2026 with nothing to
//! merge. [`refs_for_job`] names the refs of the project's checkout such a
//! job needs; the executor carries them into the clone as branches of the
//! same names (`hive_hand_core::local::carry_branches`), and the bundle a
//! hand is sent holds them beside the default branch.

use serde_json::Value;

/// The actions whose `follow_up` job merges branches onto a base.
const FOLLOW_UP_ACTIONS: [&str; 2] = ["land.rebase", "land.merge-broken"];

/// The refs of the checkout at `root` that job `job_id` of project `slug`
/// merges - the base it starts from when that is not `default_branch`, then
/// each branch it is told to merge - in the order its brief names them. A
/// job no delivery pass queued, or one whose refs are all gone, needs none.
/// A copy queued by `hive job retry --as-new` needs what the job it copies
/// needed.
pub(crate) fn refs_for_job(
    s: &hive_store::Store,
    slug: &str,
    job_id: &str,
    root: &str,
    default_branch: &str,
) -> Vec<String> {
    if slug.is_empty() || job_id.is_empty() || root.is_empty() {
        return Vec::new();
    }
    let Ok(conn) = s.conn() else {
        return Vec::new();
    };
    let mut ids = vec![job_id.to_string()];
    let mut job = crate::util::Row::new();
    job.insert("id", serde_json::json!(job_id));
    let (chain_root, _) = crate::recover::recovery_chain_from(&conn, job);
    if chain_root != job_id {
        ids.push(chain_root);
    }
    let mut detail = Value::Null;
    for id in &ids {
        let row = crate::util::query_one(
            &conn,
            "SELECT detail FROM events WHERE subject = ? AND action IN (?, ?) AND json_valid(detail)
             AND json_extract(detail, '$.follow_up') = ? ORDER BY seq DESC LIMIT 1",
            &[&slug, &FOLLOW_UP_ACTIONS[0], &FOLLOW_UP_ACTIONS[1], id],
        )
        .ok()
        .flatten();
        if let Some(row) = row {
            detail = serde_json::from_str(&row.str("detail")).unwrap_or(Value::Null);
            break;
        }
    }
    drop(conn);
    if detail.is_null() {
        return Vec::new();
    }
    let mut names: Vec<String> = Vec::new();
    let mut add = |name: &str| {
        if !name.is_empty() && name != default_branch && !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    };
    if let Some(base) = detail.get("base").and_then(Value::as_str) {
        add(base);
    }
    match detail.get("branches").and_then(Value::as_array) {
        Some(list) => list.iter().filter_map(Value::as_str).for_each(&mut add),
        None => {
            if let Some(b) = detail.get("branch").and_then(Value::as_str) {
                add(b);
            }
        }
    }
    let remotes = remote_refs(root);
    names
        .iter()
        .filter_map(|name| ref_of(root, &remotes, name))
        .collect()
}

/// Every remote-tracking ref of the checkout.
fn remote_refs(root: &str) -> Vec<String> {
    crate::util::git(
        root,
        &["for-each-ref", "--format=%(refname)", "refs/remotes"],
    )
    .stdout
    .lines()
    .map(str::to_string)
    .collect()
}

/// The ref of the checkout that holds branch `name`: its own branch, else
/// the upstream's as last fetched (a stacked base is fetched there and has
/// no local branch). `None` when it has neither.
fn ref_of(root: &str, remotes: &[String], name: &str) -> Option<String> {
    let own = format!("refs/heads/{name}");
    if crate::util::git(root, &["rev-parse", "--verify", "--quiet", &own]).ok() {
        return Some(own);
    }
    remotes
        .iter()
        .find(|r| {
            r.strip_prefix("refs/remotes/")
                .and_then(|rest| rest.split_once('/'))
                .is_some_and(|(_, branch)| branch == name)
        })
        .cloned()
}
