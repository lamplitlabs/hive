//! Finished work the runner stopped in its check (TASK-134 follow-up).
//!
//! A run whose worker had finished and committed was in its end check when a
//! forced restart (the hold cap, `daemon.restart_max_minutes`) stopped it; the
//! run was `lost`, the job went back to the queue and the clone - the only
//! copy of the commits - was removed, so the next run did the task again from
//! nothing (job-20261126T082845Z-4ef6b5, 2026-11-26). Now the stop keeps the
//! clone's commits on [`unchecked_branch`] in the project's checkout and
//! writes [`UNCHECKED_EVENT`] down; the job's next run on this machine takes
//! that branch instead of starting a worker, and checks and lands it as the
//! stopped run would have.

use crate::util::Row;

/// The event a stopped check leaves on its job: what the next run needs to
/// check the kept commits as the run that made them.
pub(crate) const UNCHECKED_EVENT: &str = "run.unchecked";

/// The branch in the project's checkout that holds the commits of a run the
/// runner stopped in its check. Not the job's own `hive/<agent>/<job>`: the
/// run that checks them lands there, and refuses a branch that exists.
pub(crate) fn unchecked_branch(agent: &str, job_id: &str) -> String {
    format!("hive/{agent}/{job_id}{UNCHECKED_SUFFIX}")
}
pub(crate) const UNCHECKED_SUFFIX: &str = "-unchecked";

/// Finished, unchecked work of a job: the commits on `branch` past `base`,
/// signed by `pubkey` for `agent` (the run that made them), and the report
/// lines its worker wrote.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Unchecked {
    /// The run that recorded it (the one the runner stopped).
    pub run: String,
    /// The worker whose commits these are; the identity check holds them to it.
    pub agent: String,
    pub pubkey: String,
    pub branch: String,
    pub base: String,
    /// The worker's report lines (`SUMMARY:`, `FINDING:`, `VOTE:` ...).
    pub report: String,
    pub commits: i64,
}

impl Unchecked {
    fn detail(&self) -> serde_json::Value {
        serde_json::json!({
            "run": self.run,
            "agent": self.agent,
            "pubkey": self.pubkey,
            "branch": self.branch,
            "base": self.base,
            "report": self.report,
            "commits": self.commits,
        })
    }
}

/// The report lines of a worker's output, the rest left out: enough for the
/// run that checks the work to read the worker's summary, findings and votes
/// again. `summary` stands in when no `SUMMARY:` line is among them (the
/// worker reported through `hive job finish`).
pub(crate) fn report_lines(text: &str, summary: &str) -> String {
    let mut lines: Vec<&str> = text
        .lines()
        .filter(|l| {
            crate::brief::REPORT_LINE_RE.is_match(l)
                || crate::brief::VOTE_LINE_RE.is_match(l)
                || crate::brief::DECIDE_LINE_RE.is_match(l)
                || crate::brief::VERDICT_LINE_RE.is_match(l)
        })
        .collect();
    let has_summary = lines.iter().any(|l| {
        crate::brief::REPORT_LINE_RE
            .captures(l)
            .is_some_and(|c| &c[1] == "SUMMARY")
    });
    let fallback = format!("SUMMARY: {}", summary.trim());
    if !has_summary && !summary.trim().is_empty() {
        lines.insert(0, &fallback);
    }
    lines.join("\n")
}

/// Writes the job's unchecked work down (`run.unchecked` on the job).
pub(crate) fn record(
    q: &(impl crate::util::Querier + ?Sized),
    now: &str,
    job_id: &str,
    u: &Unchecked,
) {
    crate::util::event_at(q, now, "runner", UNCHECKED_EVENT, job_id, u.detail());
}

/// The unchecked work the job's next run should check: the latest
/// `run.unchecked` of the job, while no later run judged the job (a run that
/// errored or was stopped before judging leaves it waiting) and its branch is
/// still in the checkout at `root`.
pub(crate) fn pending(s: &hive_store::Store, job_id: &str, root: &str) -> Option<Unchecked> {
    let conn = s.conn().ok()?;
    let ev = crate::util::query_one(
        &conn,
        "SELECT detail FROM events WHERE action = ? AND subject = ? ORDER BY seq DESC LIMIT 1",
        &[&UNCHECKED_EVENT, &job_id],
    )
    .ok()??;
    let d = crate::util::json_map(&ev.str("detail"));
    let text = |k: &str| {
        d.get(k)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let u = Unchecked {
        run: text("run"),
        agent: text("agent"),
        pubkey: text("pubkey"),
        branch: text("branch"),
        base: text("base"),
        report: text("report"),
        commits: d.get("commits").and_then(|v| v.as_i64()).unwrap_or(0),
    };
    if u.run.is_empty() || u.agent.is_empty() || u.branch.is_empty() || u.base.is_empty() {
        return None;
    }
    let later: Row = crate::util::must_one(
        &conn,
        &format!(
            "SELECT COUNT(*) AS n FROM runs WHERE job_id = ? AND id != ? AND started_at >= (SELECT started_at FROM runs WHERE id = ?) AND status NOT IN ({UNJUDGED_RUN_STATUSES})"
        ),
        &[&job_id, &u.run, &u.run],
    );
    if later.int("n") > 0 {
        return None;
    }
    if root.is_empty() || !branch_exists(root, &u.branch) {
        return None;
    }
    Some(u)
}

/// Run statuses that leave a job's work unjudged: the run is on, or it
/// stopped before any verdict.
const UNJUDGED_RUN_STATUSES: &str = "'running', 'error', 'lost', 'held', 'dry-run'";

pub(crate) fn branch_exists(root: &str, branch: &str) -> bool {
    crate::util::git(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .ok()
}

/// Puts the kept commits into the run's fresh `clone`: its HEAD becomes the
/// branch's tip. Returns why it could not.
pub(crate) fn check_out(clone: &str, root: &str, u: &Unchecked) -> Result<(), String> {
    let fetched = crate::util::git(
        clone,
        &[
            "fetch",
            "-q",
            "--no-tags",
            root,
            &format!("refs/heads/{}", u.branch),
        ],
    );
    if !fetched.ok() {
        return Err(format!(
            "could not fetch {} into the clone: {}",
            u.branch,
            fetched.stderr.trim()
        ));
    }
    let reset = crate::util::git(clone, &["reset", "-q", "--hard", "FETCH_HEAD"]);
    if !reset.ok() {
        return Err(format!(
            "could not check out {}: {}",
            u.branch,
            reset.stderr.trim()
        ));
    }
    if !crate::util::git(clone, &["merge-base", "--is-ancestor", &u.base, "HEAD"]).ok() {
        return Err(format!(
            "{} does not stand on its base {}",
            u.branch, u.base
        ));
    }
    Ok(())
}

/// Removes the branch once its commits are judged and landed on the job's own.
pub(crate) fn drop_branch(root: &str, branch: &str) {
    let _ = crate::util::git(root, &["branch", "-q", "-D", branch]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_lines_keep_the_report_and_fall_back_to_the_summary() {
        let out =
            "compiling...\nSUMMARY: done - added a note\nnoise\nFINDING: x\nVOTE: SUG-1 up good\n";
        assert_eq!(
            report_lines(out, "ignored"),
            "SUMMARY: done - added a note\nFINDING: x\nVOTE: SUG-1 up good"
        );
        assert_eq!(
            report_lines("FINDING: y", "done - via hive job finish"),
            "SUMMARY: done - via hive job finish\nFINDING: y"
        );
        assert_eq!(report_lines("", ""), "");
        assert_eq!(
            unchecked_branch("aurora", "job-1"),
            "hive/aurora/job-1-unchecked"
        );
    }
}
