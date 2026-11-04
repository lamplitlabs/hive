//! Several finished branches landed as one. A landing verifies each branch
//! on its own, and on a project whose check takes long that serializes the
//! queue: on amistio-template one verify took 1,210 s on average (36 timed
//! landings over three days, the longest 1,642 s) and the queue was often two
//! to four branches deep, so a finished branch waited through the verifies of
//! everything ahead of it. When several plain branches wait at once, the pass
//! merges them into one chain the way their individual landings would, lands
//! the chain through `land::land_branch` as one branch (one verify, one
//! fast-forward, one push) and then marks each member landed. Whatever goes
//! wrong with the chain costs the pass only that one attempt: the members are
//! landed one by one afterwards, and their own verifies name the culprit.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Where a batch's synthetic branch lives. The prefix keeps `land_branch`'s
/// branch rule (`LANDED_BRANCH_PREFIXES`) and tells it from an agent's branch.
pub(crate) const BATCH_BRANCH_PREFIX: &str = "pulse/batch/";

/// The setting that turns batch landing off (`landing.batch = off`).
pub(crate) const SETTING: &str = "landing.batch";

/// One branch of a batch. `after` is the chain's head once the branch was
/// merged, so the member's own landing record can name the commits it added
/// (from the member before it, or the batch's `before`, up to `after`), the
/// way a single landing names the default branch's move.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BatchMember {
    pub job: String,
    pub branch: String,
    pub title: String,
    #[serde(default)]
    pub after: String,
}

/// A chain of finished branches that lands together: the synthetic branch
/// `chain` points at `head`, the merge of every member onto `before` (the
/// default branch's tip when the chain was built) in finished_at order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LandBatch {
    pub id: String,
    pub chain: String,
    pub before: String,
    pub head: String,
    pub members: Vec<BatchMember>,
}

impl LandBatch {
    pub(crate) fn is_member(&self, branch: &str) -> bool {
        self.members.iter().any(|m| m.branch == branch)
    }

    pub(crate) fn branches(&self) -> Vec<String> {
        self.members.iter().map(|m| m.branch.clone()).collect()
    }

    /// "3 branches" (a batch never has one; `util::plural` would say "branchs").
    fn count(&self) -> String {
        format!("{} branches", self.members.len())
    }

    /// "batch 1a2b3c4d5e6f with 3 branches", the words every note and post
    /// about the batch share.
    pub(crate) fn describe(&self) -> String {
        format!("batch {} with {}", self.id, self.count())
    }

    /// The merge commit's title should the chain itself need a merge commit
    /// (the default branch moved between the build and the landing).
    pub(crate) fn title(&self) -> String {
        format!("Land {}: {}", self.count(), self.branches().join(", "))
    }

    /// What the events carry about the members: branch and job of each.
    pub(crate) fn members_json(&self) -> serde_json::Value {
        serde_json::json!(
            self.members
                .iter()
                .map(|m| serde_json::json!({"branch": m.branch, "job": m.job, "after": m.after}))
                .collect::<Vec<_>>()
        )
    }

    /// The commits `branch` added to the chain, as `(before, after)`: from the
    /// head the member before it left (the batch's `before` for the first) to
    /// its own. Both are on the default branch once the chain landed.
    pub(crate) fn transition(&self, branch: &str) -> Option<(String, String)> {
        let i = self.members.iter().position(|m| m.branch == branch)?;
        let before = if i == 0 {
            self.before.clone()
        } else {
            self.members[i - 1].after.clone()
        };
        Some((before, self.members[i].after.clone()))
    }
}

/// What the chain's own landing said of its check (`verify` on the
/// `land.batch` event), for the members' records: "verify exit 0", or the memo
/// that stood in for it.
pub(crate) fn chain_verify_note(s: &pulse_store::Store, batch: &LandBatch) -> String {
    let Ok(conn) = s.conn() else {
        return String::new();
    };
    conn.query_row(
        "SELECT COALESCE(json_extract(detail, '$.verify'), '') FROM events WHERE action = 'land.batch' AND subject = ? AND json_extract(detail, '$.batch') = ? ORDER BY seq DESC LIMIT 1",
        rusqlite::params![batch.chain, batch.id],
        |r| r.get::<_, String>(0),
    )
    .unwrap_or_default()
}

/// Batch landing is on unless the owner set `landing.batch` to off, read the
/// way `landing.verify_memo` is.
pub(crate) fn batch_on(s: &pulse_store::Store) -> bool {
    s.setting(SETTING, "on") != "off"
}

/// Why a branch with these changed `files` cannot ride in a batch, or None.
/// The reasons are the refusals and holds `land_branch` applies to a plain
/// branch (owner paths, nothing but the result file, a recipe change, which
/// lands alone and waits for a human) plus any lockfile at all: a lockfile
/// branch is dated by the release-age rule and may wait for its day, and a
/// chain carrying one would be asked the same question about every member's
/// work. Left out of the batch, such a branch gets exactly the individual
/// landing it got before, and never sinks the others.
pub(crate) fn member_refusal(files: &[String], branch: &str) -> Option<String> {
    let refused = crate::land::refused_paths(files.to_vec(), branch.to_string());
    if !refused.is_empty() {
        let shown = refused.into_iter().take(4).collect::<Vec<_>>().join(", ");
        return Some(format!("changes {shown}, which no landing accepts"));
    }
    let result_file = crate::land::RESULT_FILE;
    if files.is_empty() || (files.len() == 1 && files[0] == result_file) {
        return Some(format!("changes nothing but {result_file}"));
    }
    if let Some(f) = files
        .iter()
        .find(|f| crate::land::HUMAN_LANDED_PATHS.contains(&f.as_str()))
    {
        return Some(format!("changes {f}, which lands alone"));
    }
    let lockfiles = crate::land::setup_lockfiles();
    if let Some(f) = files.iter().find(|f| lockfiles.contains(f)) {
        return Some(format!(
            "changes {f}, which the release-age rule dates on its own"
        ));
    }
    None
}

/// Whether `job`'s branch can be a member of a batch built on the default
/// branch `def` of `root` now, and why not when it cannot. A branch already
/// contained in `def`, one with no common history (its landing replays from
/// the run base) or one unreadable is left to its own landing, which handles
/// each of those in its own way.
fn member_check(root: &str, def: &str, job: &crate::util::Row) -> Result<BatchMember, String> {
    let branch = job.str("branch");
    if !crate::land::LANDED_BRANCH_PREFIXES
        .iter()
        .any(|p| branch.starts_with(p))
    {
        return Err(format!("{branch} is not a branch pulse lands"));
    }
    let (exists, unreadable) = crate::land::branch_exists(root.to_string(), branch.clone());
    if !unreadable.is_empty() {
        return Err(format!("{branch} cannot be read: {unreadable}"));
    }
    if !exists {
        return Err(format!("{branch} does not exist"));
    }
    if crate::util::git_timeout(
        root,
        Duration::from_secs(30),
        &["merge-base", "--is-ancestor", &branch, def],
    )
    .ok()
    {
        return Err(format!("{branch} is already contained in {def}"));
    }
    let base =
        crate::util::git_timeout(root, Duration::from_secs(30), &["merge-base", def, &branch])
            .stdout
            .trim()
            .to_string();
    if base.is_empty() {
        return Err(format!("{branch} shares no history with {def}"));
    }
    let files = crate::land::branch_files(root.to_string(), base, branch.clone());
    if let Some(why) = member_refusal(&files, &branch) {
        return Err(format!("{branch} {why}"));
    }
    Ok(BatchMember {
        job: job.str("id"),
        branch,
        title: job.str("title"),
        after: String::new(),
    })
}

/// The batch's name: twelve hex digits of a hash over the base commit and the
/// member branches, so the same queue on the same default branch names the
/// same batch and a pass that died mid-way leaves nothing a later one cannot
/// recognise and remove.
fn batch_id(before: &str, members: &[BatchMember]) -> String {
    let mut h = Sha256::new();
    h.update(before.as_bytes());
    for m in members {
        h.update(b"\n");
        h.update(m.branch.as_bytes());
    }
    hex::encode(h.finalize())[..12].to_string()
}

/// Removes every `pulse/batch/*` branch of `root`: a batch's synthetic branch
/// lives only for the pass that built it, so any found at the start of a pass
/// was left by a daemon that died between building and landing it.
pub(crate) fn drop_stale_chains(root: &str) -> Vec<String> {
    let listed = crate::util::git_timeout(
        root,
        Duration::from_secs(30),
        &[
            "for-each-ref",
            "--format=%(refname:short)",
            &format!("refs/heads/{BATCH_BRANCH_PREFIX}"),
        ],
    );
    let mut dropped = Vec::new();
    if !listed.ok() {
        return dropped;
    }
    for name in listed
        .stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
    {
        if drop_chain(root, name) {
            dropped.push(name.to_string());
        }
    }
    dropped
}

/// Deletes the synthetic branch `chain` of `root`; true when it is gone.
pub(crate) fn drop_chain(root: &str, chain: &str) -> bool {
    crate::util::git_timeout(root, Duration::from_secs(30), &["branch", "-D", chain]).ok()
        || !crate::land::branch_exists(root.to_string(), chain.to_string()).0
}

/// The pulse identity every landing commit is made with.
const PULSE_IDENTITY: [&str; 6] = [
    "-c",
    "user.name=pulse",
    "-c",
    "user.email=pulse@pulse.local",
    "-c",
    "commit.gpgsign=false",
];

/// Merges `member` onto the chain in `worktree` the way its own landing would
/// (a fast-forward when possible, else a merge commit under the pulse identity
/// with the landing's message, a hub export conflict rebuilt), then drops the
/// result file. False, with the merge aborted, when the member conflicts with
/// what is already on the chain.
fn merge_member(worktree: &str, member: &BatchMember, actor: &str) -> bool {
    let branch = &member.branch;
    let mut merged = crate::util::git_timeout(
        worktree,
        Duration::from_secs(120),
        &["merge", "--ff-only", branch],
    );
    if !merged.ok() {
        let title = if member.title.is_empty() {
            branch.clone()
        } else {
            member.title.clone()
        };
        let msg = format!(
            "Land {branch}: {}\n\nPulse-Landed-By: {actor}\nPulse-Job: {}\n",
            crate::util::trunc(&title, 200),
            member.job
        );
        let mut args: Vec<&str> = PULSE_IDENTITY.to_vec();
        args.extend(["merge", "--no-ff", "--no-edit", "-m", &msg, branch]);
        merged = crate::util::git_timeout(worktree, Duration::from_secs(300), &args);
    }
    if !merged.ok() && crate::forge::resolve_hub_out_conflict(worktree, branch) {
        merged =
            crate::util::git_timeout(worktree, Duration::from_secs(30), &["rev-parse", "HEAD"]);
    }
    if !merged.ok() {
        crate::util::git_timeout(worktree, Duration::from_secs(60), &["merge", "--abort"]);
        return false;
    }
    let result_file = crate::land::RESULT_FILE;
    if crate::util::git_timeout(
        worktree,
        Duration::from_secs(10),
        &["ls-files", "--error-unmatch", result_file],
    )
    .ok()
    {
        crate::util::git_timeout(
            worktree,
            Duration::from_secs(30),
            &["rm", "-q", result_file],
        );
        let msg = format!("Land {branch}: drop {result_file}");
        let mut args: Vec<&str> = PULSE_IDENTITY.to_vec();
        args.extend(["commit", "-q", "-m", &msg]);
        crate::util::git_timeout(worktree, Duration::from_secs(60), &args);
    }
    true
}

/// Builds a batch from the pending `jobs` (finished_at order, plain branches
/// only): the members that pass `member_check` are merged one after the other
/// onto the default branch in a throwaway worktree, a member whose merge
/// conflicts is skipped, and the chain's head gets the synthetic branch
/// `pulse/batch/<id>`. Err names why there is no batch to land: fewer than two
/// members merged, or git would not give a worktree. The individual landings
/// that follow owe nothing to a failed build; it cost only the merges.
pub(crate) fn build_batch(
    s: &pulse_store::Store,
    project: &crate::util::Row,
    jobs: &[crate::util::Row],
    actor: &str,
) -> Result<LandBatch, String> {
    let root = project.str("path");
    let def = {
        let d = project.str("default_branch");
        if d.is_empty() { "main".to_string() } else { d }
    };
    let slug = project.str("slug");
    let mut members = Vec::new();
    let mut left_out = Vec::new();
    for job in jobs {
        match member_check(&root, &def, job) {
            Ok(m) => members.push(m),
            Err(why) => left_out.push(why),
        }
    }
    if members.len() < 2 {
        return Err(format!(
            "{} of {} pending branches could ride in a batch{}",
            members.len(),
            jobs.len(),
            if left_out.is_empty() {
                String::new()
            } else {
                format!(" ({})", left_out.join("; "))
            }
        ));
    }
    let before = crate::util::git_timeout(
        &root,
        Duration::from_secs(10),
        &["rev-parse", "--verify", &format!("refs/heads/{def}")],
    )
    .stdout
    .trim()
    .to_string();
    if before.is_empty() {
        return Err(format!("{def} has no commit in {root}"));
    }
    let worktree = Path::new(&s.home)
        .join("work")
        .join(format!("batch-{}-{}", slug, std::process::id()))
        .display()
        .to_string();
    crate::util::git_timeout(
        &root,
        Duration::from_secs(60),
        &["worktree", "remove", "--force", &worktree],
    );
    crate::util::git_timeout(&root, Duration::from_secs(30), &["worktree", "prune"]);
    if Path::new(&worktree).is_dir() {
        let _ = std::fs::remove_dir_all(&worktree);
    }
    let added = crate::util::git_timeout(
        &root,
        Duration::from_secs(120),
        &["worktree", "add", "--detach", &worktree, &def],
    );
    if !added.ok() {
        return Err(format!("git worktree add failed: {}", added.stderr.trim()));
    }
    let _cleanup = ChainCleanup {
        root: root.clone(),
        worktree: worktree.clone(),
    };
    let mut merged: Vec<BatchMember> = Vec::new();
    for mut m in members {
        if merge_member(&worktree, &m, actor) {
            m.after = crate::util::git_timeout(
                &worktree,
                Duration::from_secs(10),
                &["rev-parse", "HEAD"],
            )
            .stdout
            .trim()
            .to_string();
            merged.push(m);
        } else {
            left_out.push(format!(
                "{} conflicts with the branches ahead of it",
                m.branch
            ));
        }
    }
    if merged.len() < 2 {
        return Err(format!(
            "{} of {} pending branches merged onto one chain{}",
            merged.len(),
            jobs.len(),
            if left_out.is_empty() {
                String::new()
            } else {
                format!(" ({})", left_out.join("; "))
            }
        ));
    }
    let head = merged.last().map(|m| m.after.clone()).unwrap_or_default();
    let id = batch_id(&before, &merged);
    let chain = format!("{BATCH_BRANCH_PREFIX}{id}");
    let named = crate::util::git_timeout(
        &root,
        Duration::from_secs(30),
        &["branch", "-f", &chain, &head],
    );
    if !named.ok() || head.is_empty() {
        return Err(format!(
            "git branch -f {chain} failed: {}",
            named.stderr.trim()
        ));
    }
    Ok(LandBatch {
        id,
        chain,
        before,
        head,
        members: merged,
    })
}

/// Records that a batch did not land (`land.batch-failed`: the members, why,
/// and the check's tail when it ran) and returns the landing state the pass
/// reads as "land these one by one now". Nothing else happens to the members
/// here: no requeue, no retry hold, no post, since nothing is known yet about
/// which of them is at fault.
pub(crate) fn fail_batch(
    s: &pulse_store::Store,
    now: &str,
    actor: &str,
    slug: &str,
    batch: &LandBatch,
    why: &str,
    tail: &str,
) -> (String, String) {
    let failures = crate::land::verify_failures(tail);
    if let Ok(conn) = s.conn() {
        crate::util::event_at(
            &conn,
            now,
            actor,
            "land.batch-failed",
            &batch.id,
            serde_json::json!({
                "project": slug,
                "batch": batch.id,
                "chain": batch.chain,
                "before": batch.before,
                "head": batch.head,
                "members": batch.members_json(),
                "why": why,
                "failures": failures,
                "tail": crate::executor::tail(tail, 1500),
            }),
        );
    }
    (
        "batch-failed".into(),
        format!(
            "{} ({}) did not land as one: {why}; its branches land one by one now",
            batch.describe(),
            batch.branches().join(", ")
        ),
    )
}

/// Removes the chain's worktree when the build is over, landed or not; the
/// synthetic branch outlives it until the pass has landed or dropped it.
struct ChainCleanup {
    root: String,
    worktree: String,
}

impl Drop for ChainCleanup {
    fn drop(&mut self) {
        crate::util::git_timeout(
            &self.root,
            Duration::from_secs(60),
            &["worktree", "remove", "--force", &self.worktree],
        );
        crate::util::git_timeout(&self.root, Duration::from_secs(30), &["worktree", "prune"]);
        if Path::new(&self.worktree).is_dir() {
            let _ = std::fs::remove_dir_all(&self.worktree);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn member_refusal_names_what_keeps_a_branch_out() {
        let ok = vec![
            "src/lib.rs".to_string(),
            crate::land::RESULT_FILE.to_string(),
        ];
        assert_eq!(member_refusal(&ok, "pulse/aurora/x"), None);
        assert!(
            member_refusal(&[crate::land::RESULT_FILE.to_string()], "pulse/aurora/x")
                .is_some_and(|w| w.contains("nothing but"))
        );
        assert!(
            member_refusal(&["docs/evolution/STOP".to_string()], "pulse/aurora/x")
                .is_some_and(|w| w.contains("no landing accepts"))
        );
        assert!(
            member_refusal(&[crate::recipe::RECIPE_PATH.to_string()], "pulse/aurora/x")
                .is_some_and(|w| w.contains("lands alone"))
        );
        assert!(
            member_refusal(
                &["Cargo.lock".to_string(), "Cargo.toml".to_string()],
                "pulse/aurora/x"
            )
            .is_some_and(|w| w.contains("release-age"))
        );
    }

    #[test]
    fn batch_id_is_deterministic_and_short() {
        let members = vec![
            BatchMember {
                job: "j1".into(),
                branch: "pulse/a/one".into(),
                ..Default::default()
            },
            BatchMember {
                job: "j2".into(),
                branch: "pulse/a/two".into(),
                ..Default::default()
            },
        ];
        let a = batch_id("abc", &members);
        assert_eq!(a.len(), 12);
        assert_eq!(a, batch_id("abc", &members));
        assert_ne!(a, batch_id("abd", &members));
        let mut swapped = members.clone();
        swapped.reverse();
        assert_ne!(a, batch_id("abc", &swapped));
    }
}
