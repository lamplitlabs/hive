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
//! merge. [`plan_for_job`] names the branches such a job needs and the
//! commit of the checkout each is made from - the one the queuing event
//! recorded, so the clone holds what the brief names even when a branch
//! moved on - and the base the clone starts on; the executor carries them
//! into the clone as branches of the same names
//! (`hive_hand_core::local::fetch_branches`), and the bundle a hand is sent
//! holds them beside the default branch.

use serde_json::Value;

/// The actions whose `follow_up` job merges branches onto a base.
const FOLLOW_UP_ACTIONS: [&str; 2] = ["land.rebase", "land.merge-broken"];

/// What a delivery follow-up job's clone carries beside the branch it is
/// cloned on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Carry {
    /// Each branch to carry as (its name, the rev of the checkout it is made
    /// from): the commit the queuing event recorded when the checkout still
    /// has it (the base and the branches the brief names by commit), else
    /// the branch's ref as it stands.
    pub(crate) branches: Vec<(String, String)>,
    /// The base the job starts from when that is not the default branch -
    /// the head of the pull request the task stacks on. The clone starts on
    /// it, so the run's base is the commit the job builds on and the landing
    /// answers for the job's merges alone, not for the base's commits
    /// (TASK-183 item a).
    pub(crate) start: String,
    /// One line for each branch the checkout could not give - neither its
    /// recorded commit nor a ref of its name is there - and for each carried
    /// at a commit other than its ref's.
    pub(crate) notes: Vec<String>,
}

impl Carry {
    pub(crate) fn is_empty(&self) -> bool {
        self.branches.is_empty()
    }

    /// The names carried, in order.
    pub(crate) fn names(&self) -> Vec<String> {
        self.branches.iter().map(|(n, _)| n.clone()).collect()
    }
}

/// The branches of the checkout at `root` that job `job_id` of project
/// `slug` merges - the base it starts from when that is not
/// `default_branch`, then each branch it is told to merge - in the order
/// its brief names them. A job no delivery pass queued needs none. A copy
/// queued by `hive job retry --as-new` needs what the job it copies needed.
pub(crate) fn plan_for_job(
    s: &hive_store::Store,
    slug: &str,
    job_id: &str,
    root: &str,
    default_branch: &str,
) -> Carry {
    let mut plan = Carry::default();
    if slug.is_empty() || job_id.is_empty() || root.is_empty() {
        return plan;
    }
    let detail = queuing_event(s, slug, job_id);
    if detail.is_null() {
        return plan;
    }
    let text = |v: Option<&Value>| v.and_then(Value::as_str).unwrap_or_default().to_string();
    // (name, the commit the event recorded for it, if any).
    let mut wanted: Vec<(String, String)> = Vec::new();
    let mut add = |name: String, sha: String| {
        if !name.is_empty() && name != default_branch && !wanted.iter().any(|(n, _)| *n == name) {
            wanted.push((name, sha));
        }
    };
    let base = text(detail.get("base"));
    add(base.clone(), text(detail.get("base_sha")));
    match detail.get("branches").and_then(Value::as_array) {
        Some(list) => {
            let shas = detail.get("shas").and_then(Value::as_array);
            for (i, b) in list.iter().enumerate() {
                let sha = text(shas.and_then(|v| v.get(i)));
                add(text(Some(b)), sha);
            }
        }
        None => add(text(detail.get("branch")), text(detail.get("sha"))),
    }
    let remotes = remote_refs(root);
    for (name, sha) in wanted {
        let at_ref = ref_of(root, &remotes, &name);
        let rev = if !sha.is_empty() && has_commit(root, &sha) {
            if let Some(r) = &at_ref
                && rev_of(root, r) != sha
            {
                plan.notes.push(format!(
                    "{name}: carried at {}, the commit the job was given; {r} is at {} now",
                    crate::recipe::short(&sha, 12),
                    crate::recipe::short(&rev_of(root, r), 12)
                ));
            }
            Some(sha.clone())
        } else {
            at_ref
        };
        match rev {
            Some(rev) => {
                if name == base {
                    plan.start = name.clone();
                }
                plan.branches.push((name, rev));
            }
            None => plan.notes.push(if sha.is_empty() {
                format!("{name}: the checkout has no branch of that name, here or as an upstream's")
            } else {
                format!(
                    "{name}: the checkout has neither the commit {} the job was given nor a branch of that name",
                    crate::recipe::short(&sha, 12)
                )
            }),
        }
    }
    plan
}

/// The `land.rebase` / `land.merge-broken` detail that queued `job_id`, or
/// the job a `hive job retry --as-new` copy was made from; null for any
/// other job.
fn queuing_event(s: &hive_store::Store, slug: &str, job_id: &str) -> Value {
    let Ok(conn) = s.conn() else {
        return Value::Null;
    };
    let mut ids = vec![job_id.to_string()];
    let mut job = crate::util::Row::new();
    job.insert("id", serde_json::json!(job_id));
    let (chain_root, _) = crate::recover::recovery_chain_from(&conn, job);
    if chain_root != job_id {
        ids.push(chain_root);
    }
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
            return serde_json::from_str(&row.str("detail")).unwrap_or(Value::Null);
        }
    }
    Value::Null
}

/// Whether the checkout holds commit `sha`.
fn has_commit(root: &str, sha: &str) -> bool {
    sha.chars().all(|c| c.is_ascii_hexdigit())
        && crate::util::git(root, &["cat-file", "-e", &format!("{sha}^{{commit}}")]).ok()
}

/// The commit `r` names in the checkout; empty when none.
fn rev_of(root: &str, r: &str) -> String {
    crate::util::git(root, &["rev-parse", "--verify", "--quiet", r])
        .stdout
        .trim()
        .to_string()
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
