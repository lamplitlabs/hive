//! The open pull requests of experiments (`evolve/<id>`, kind
//! `experiment`), looked after on every delivery pass (`poll_open`) on top
//! of what any open pull request gets:
//!
//! - **Superseded**: the experiment's change is on the default branch
//!   already - landed by a squash, a cherry-pick, another pull request, or a
//!   merge the forge did not see as this one's. Merging the head into the
//!   default branch would change nothing (`git merge-tree` gives the default
//!   branch's own tree), so the pull request is closed with a comment naming
//!   the commit that holds its change (`gh pr close --comment`), recorded as
//!   `land.pr-closed` with `reason: superseded` and `superseded_by`, any
//!   pull request built on it moves onto its base, and its branch is
//!   deleted on the remote (leasing on the head judged) and in the checkout.
//! - **Stale**: open [`STALE_DAYS`] days with no push (the newest `land.pr`
//!   of its number). One based on the default branch that lacks the default
//!   branch's tip takes it in (`land.pr-refreshed`, as a conflicting pull
//!   request does), which restarts its clock; one that does not merge
//!   cleanly, or that stands on another pull request rather than on the
//!   default branch, is closed with a comment saying why (`reason: stale`);
//!   its branch stays on the remote, nothing is lost.
//!
//! An experiment is measured on the default branch, so its pull request is
//! never built on another one and never has one built on it (`stacks`,
//! `deliver_experiment`): a pull request merged into a pull request strands
//! its change when the one below is closed or squashed.

use super::*;

/// Days an experiment's pull request may stand with no push before the
/// pass brings it up to date with the default branch or closes it.
pub(super) const STALE_DAYS: i64 = 3;

/// An experiment's pull request: kind `experiment`, or any head on the
/// `evolve/` branches (one proposed by hand).
pub(super) fn is_experiment(pr: &Value) -> bool {
    kind_of(pr) == "experiment" || field(pr, "head").starts_with("evolve/")
}

/// One open experiment pull request: closed when its change is on the
/// default branch, refreshed or closed when it went stale. Returns whether
/// the pull request was settled (closed, or refreshed: the follow-up of an
/// ordinary open pull request then waits for the forge to judge the new
/// head), with the log lines.
pub(super) fn tend_open(
    s: &pulse_store::Store,
    project: &Row,
    up: &Upstream,
    pr: &Value,
    st: &crate::landing_pr::PrState,
    listed: &BTreeMap<i64, crate::landing_pr::PrState>,
    now: &str,
) -> (bool, Vec<String>) {
    let slug = project.str("slug");
    let root = project.str("path");
    let number = number_of(pr);
    let head = if st.head.is_empty() {
        field_of(pr, "head")
    } else {
        st.head.clone()
    };
    let line = format!("experiment pr {slug}: #{number}");
    if head.is_empty() {
        return (false, Vec::new());
    }
    let def_sha = match fetch_head(s, &root, up, &up.remote_branch) {
        Ok(sha) if !sha.is_empty() => sha,
        Ok(_) => return (false, vec![format!("{line}: {} is gone", up.name())]),
        Err(e) => return (false, vec![format!("{line}: {e}")]),
    };
    let head_sha = match fetch_head(s, &root, up, &head) {
        Ok(sha) if !sha.is_empty() => sha,
        // The forge closes a pull request whose branch went; it says so
        // on its own next pass.
        Ok(_) => return (false, Vec::new()),
        Err(e) => return (false, vec![format!("{line}: {e}")]),
    };
    // The forge's listing lagging a push: judge the head on the next pass.
    if !st.head_sha.is_empty() && st.head_sha != head_sha {
        return (false, Vec::new());
    }
    if let Some(by) = superseded_by(&root, &def_sha, &head_sha, &field_of(pr, "from")) {
        return (
            true,
            close_superseded(s, project, up, pr, &head, &head_sha, &by, listed, now),
        );
    }
    tend_stale(s, project, up, pr, &head, &head_sha, &def_sha, listed, now)
}

/// The commit of the default branch (`def_sha`) that holds the change of
/// `head_sha`, when merging the head into the default branch would change
/// nothing; None when the head still brings something. Two branches that
/// share no history (the default branch squashed to a new root after the
/// head forked) are compared from `from`, the base the pull request was
/// delivered on, while that is still a commit the head descends from.
fn superseded_by(root: &str, def_sha: &str, head_sha: &str, from: &str) -> Option<String> {
    let git = |args: &[&str]| crate::util::git_timeout(root, Duration::from_secs(120), args);
    if is_ancestor(root, head_sha, def_sha) {
        return Some(head_sha.to_string());
    }
    let fork = {
        let base = git(&["merge-base", def_sha, head_sha]);
        let base = base.stdout.trim().to_string();
        if base.is_empty() {
            crate::experiment::run_base_fork(root, head_sha, from)
        } else {
            base
        }
    };
    if fork.is_empty() {
        return None;
    }
    let merge_base = format!("--merge-base={fork}");
    let merged = git(&["merge-tree", "--write-tree", &merge_base, def_sha, head_sha]);
    if !merged.ok() {
        return None;
    }
    let tree = merged
        .stdout
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    let def_tree = git(&["rev-parse", &format!("{def_sha}^{{tree}}")]);
    if tree.len() < 40 || tree != def_tree.stdout.trim() {
        return None;
    }
    // The commit on the default branch with the same patch as one of the
    // head's (a cherry-pick, or a squash of a single commit), newest first.
    let cherry = git(&[
        "rev-list",
        "--cherry-mark",
        "--left-only",
        "--no-merges",
        &format!("{def_sha}...{head_sha}"),
    ]);
    if let Some(sha) = cherry
        .stdout
        .lines()
        .find_map(|l| l.trim().strip_prefix('='))
        .filter(|sha| sha.len() >= 40)
    {
        return Some(sha.to_string());
    }
    // Else the newest commit of the default branch touching what the head
    // changed: a squash of several commits, or the change written anew.
    let files: Vec<String> = git(&["diff", "--name-only", &fork, head_sha])
        .stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    if !files.is_empty() {
        let mut args = vec!["log", "-1", "--format=%H", def_sha, "--"];
        args.extend(files.iter().map(String::as_str));
        let touched = git(&args).stdout.trim().to_string();
        if touched.len() >= 40 {
            return Some(touched);
        }
    }
    Some(def_sha.to_string())
}

/// The commit's first line, for the comment: `abc123def456 (subject)`.
fn describe_commit(root: &str, sha: &str) -> String {
    let subject = crate::util::git_timeout(
        root,
        Duration::from_secs(10),
        &["log", "-1", "--format=%s", sha],
    )
    .stdout
    .trim()
    .to_string();
    let short = crate::recipe::short(sha, 12);
    if subject.is_empty() {
        short
    } else {
        format!("{short} ({})", crate::util::trunc(&subject, 100))
    }
}

/// Closes an experiment's pull request whose change is on the default
/// branch already, with a comment naming the commit `by` that holds it;
/// the ones built on it move onto its base, and its branch goes from the
/// remote and the checkout.
#[allow(clippy::too_many_arguments)]
fn close_superseded(
    s: &pulse_store::Store,
    project: &Row,
    up: &Upstream,
    pr: &Value,
    head: &str,
    head_sha: &str,
    by: &str,
    listed: &BTreeMap<i64, crate::landing_pr::PrState>,
    now: &str,
) -> Vec<String> {
    let slug = project.str("slug");
    let root = project.str("path");
    let number = number_of(pr);
    let def = &up.remote_branch;
    let commit = describe_commit(&root, by);
    let comment = format!(
        "Closed by PULSE: the change of `{head}` is on `{def}` already, in {commit}. \
         Merging this pull request would change nothing, so it is closed and the branch `{head}` deleted."
    );
    let line = format!("experiment pr {slug}: #{number}");
    if let Err(e) = crate::landing_pr::close_pull_request(&root, &up.repo, number, &comment) {
        return vec![format!(
            "{line} is on {def} already in {} but was not closed: {e}",
            crate::recipe::short(by, 12)
        )];
    }
    let mut detail = settle_detail(
        pr,
        &slug,
        &crate::landing_pr::PrState {
            state: "CLOSED".into(),
            ..Default::default()
        },
    );
    detail["reason"] = json!("superseded");
    detail["superseded_by"] = json!(by);
    detail["head_sha"] = json!(head_sha);
    event(s, now, "land.pr-closed", &slug, detail);
    let mut lines = vec![format!(
        "{line} closed: its change is on {def} already in {}",
        crate::recipe::short(by, 12)
    )];
    // Its change is on the default branch: the ones built on it move onto
    // its base as after a merge, before its branch goes from the remote.
    lines.extend(retarget_stacked(s, project, up, pr, listed, false, now));
    match delete_remote_branch(s, &root, up, head, head_sha) {
        Ok(()) => lines[0].push_str(&format!("; {head} deleted on {}", up.remote)),
        Err(e) => lines[0].push_str(&format!("; {head} not deleted on {}: {e}", up.remote)),
    }
    for note in delete_delivered_branches(s, &slug, &root, pr) {
        lines[0].push_str(&format!("; {note}"));
    }
    clear_branches(s, &slug, pr);
    lines
}

/// Deletes `head` on the upstream, leasing on `sha` (the head the pass
/// judged: a push made since keeps the branch), and its tracking ref.
fn delete_remote_branch(
    s: &pulse_store::Store,
    root: &str,
    up: &Upstream,
    head: &str,
    sha: &str,
) -> Result<(), String> {
    let (env, cleanup, why) = crate::forge::push_credential_env(s, root.to_string());
    if !why.is_empty() {
        return Err(why);
    }
    let pushed = crate::util::git_timeout_env(
        root,
        Duration::from_secs(120),
        &env,
        &[
            "push",
            &format!("--force-with-lease=refs/heads/{head}:{sha}"),
            &up.url,
            &format!(":refs/heads/{head}"),
        ],
    );
    cleanup("", &[]);
    if !pushed.ok() {
        return Err(pulse_cred::redact(
            &crate::run::squeeze(&format!("{} {}", pushed.stderr, pushed.stdout), 200),
            &[],
        ));
    }
    let _ = crate::util::git_timeout(
        root,
        Duration::from_secs(10),
        &[
            "update-ref",
            "-d",
            &format!("refs/remotes/{}/{head}", up.remote),
        ],
    );
    Ok(())
}

/// When the pull request of `number` last moved: its newest `land.pr`
/// (opened, pushed, refreshed or retargeted).
fn last_moved(s: &pulse_store::Store, slug: &str, number: i64) -> String {
    let Ok(conn) = s.conn() else {
        return String::new();
    };
    crate::util::query_one(
        &conn,
        "SELECT at FROM events WHERE subject = ? AND action = 'land.pr' AND json_valid(detail)
         AND json_extract(detail, '$.number') = ? ORDER BY seq DESC LIMIT 1",
        &[&slug, &number],
    )
    .ok()
    .flatten()
    .map(|r| r.str("at"))
    .unwrap_or_default()
}

/// An experiment's pull request with no push for [`STALE_DAYS`] days:
/// brought up to date with the default branch when it is based there and
/// lacks its tip, closed when it does not merge cleanly or stands on
/// another pull request. One up to date waits for its review.
#[allow(clippy::too_many_arguments)]
fn tend_stale(
    s: &pulse_store::Store,
    project: &Row,
    up: &Upstream,
    pr: &Value,
    head: &str,
    head_sha: &str,
    def_sha: &str,
    listed: &BTreeMap<i64, crate::landing_pr::PrState>,
    now: &str,
) -> (bool, Vec<String>) {
    let slug = project.str("slug");
    let root = project.str("path");
    let number = number_of(pr);
    let moved = last_moved(s, &slug, number);
    if moved.is_empty() {
        return (false, Vec::new());
    }
    let idle = crate::util::must_iso(now) - crate::util::must_iso(&moved);
    if idle < chrono::Duration::days(STALE_DAYS) {
        return (false, Vec::new());
    }
    let days = idle.num_days();
    let base = base_branch_of(pr, up).to_string();
    let def = up.remote_branch.clone();
    let line = format!("experiment pr {slug}: #{number} open {days} days with no push");
    let why = if base != def {
        format!(
            "it stands on `{base}`, another pull request, rather than on `{def}`; an experiment is measured on `{def}` alone and is never merged into another pull request"
        )
    } else if is_ancestor(&root, def_sha, head_sha) {
        // Up to date: it waits for its review.
        return (false, Vec::new());
    } else {
        match merge_base_into_head(s, &root, up, head, head_sha, &def) {
            Ok(Refreshed::Pushed(tip)) => {
                record_refreshed(s, project, pr, head, head_sha, &tip, &def, now);
                return (
                    true,
                    vec![format!(
                        "{line}; {def} merged in and pushed, head now {}",
                        crate::recipe::short(&tip, 12)
                    )],
                );
            }
            Ok(Refreshed::Conflict(files)) => format!(
                "`{def}` moved on and no longer merges into it cleanly ({})",
                files.join(", ")
            ),
            Err(e) => return (false, vec![format!("{line}; not refreshed: {e}")]),
        }
    };
    let comment = format!(
        "Closed by PULSE: this experiment's pull request has had no push for {days} days and {why}. \
         The branch `{head}` stays on the remote; the experiment can be planned again on `{def}`."
    );
    if let Err(e) = crate::landing_pr::close_pull_request(&root, &up.repo, number, &comment) {
        return (false, vec![format!("{line}; not closed: {e}")]);
    }
    let mut detail = settle_detail(
        pr,
        &slug,
        &crate::landing_pr::PrState {
            state: "CLOSED".into(),
            ..Default::default()
        },
    );
    detail["reason"] = json!("stale");
    detail["head_sha"] = json!(head_sha);
    event(s, now, "land.pr-closed", &slug, detail);
    clear_branches(s, &slug, pr);
    let mut lines = vec![format!("{line}; closed: {}", why.replace('`', ""))];
    lines.extend(retarget_stacked(s, project, up, pr, listed, true, now));
    (true, lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::*;
    use crate::test_helpers_daemon::*;

    fn tend_proj(s: &pulse_store::Store, proj: &Row, now: &str) -> Vec<String> {
        crate::forge::forget_sync_memo();
        tend(s, std::slice::from_ref(proj), now)
    }

    /// An `evolve/<id>` branch off `main` with one commit per file, pushed
    /// to the upstream and open there as an experiment's pull request (the
    /// `land.pr` the experiment lane records), opened at `at`. Returns the
    /// pull request's number and the branch's tip.
    fn open_experiment(
        s: &pulse_store::Store,
        repo: &str,
        bare: &str,
        name: &str,
        files: &[(&str, &str)],
        at: &str,
    ) -> (i64, String) {
        let branch = format!("evolve/{name}");
        let from = ref_of(repo, "refs/heads/main");
        run(repo, &["git", "checkout", "-q", "-b", &branch, "main"]);
        for (file, text) in files {
            std::fs::write(std::path::Path::new(repo).join(file), text).expect("write");
            run(repo, &["git", "add", "-A"]);
            run(
                repo,
                &["git", "commit", "-q", "-m", &format!("{name}: {file}")],
            );
        }
        run(repo, &["git", "checkout", "-q", "main"]);
        run(repo, &["git", "push", "-q", bare, &branch]);
        let tip = ref_of(repo, &format!("refs/heads/{branch}"));
        let (number, url) = crate::landing_pr::create_pull_request(
            repo,
            "",
            &branch,
            "main",
            &format!("Experiment {name}"),
            "b",
        )
        .expect("pr create");
        let conn = s.conn().expect("conn");
        crate::util::event_at(
            &conn,
            at,
            ACTOR,
            "land.pr",
            "proj",
            json!({"project": "proj", "kind": "experiment", "number": number, "url": url,
                "head": branch, "base": "main", "remote": "origin", "from": from, "to": tip,
                "jobs": [format!("JOB-{name}")], "job": format!("JOB-{name}"),
                "experiment": format!("JOB-{name}"), "archive": "B-1", "run_id": "run-1",
                "commits": files.len(), "opened": true, "title": format!("Experiment {name}")}),
        );
        (number, tip)
    }

    /// Commits on `main` in the checkout and pushes it to the upstream.
    fn land_on_main(repo: &str, bare: &str, how: &[&str], msg: &str) -> String {
        run(repo, &["git", "checkout", "-q", "main"]);
        let mut argv = vec!["git"];
        argv.extend(how);
        run(repo, &argv);
        if !msg.is_empty() {
            run(repo, &["git", "commit", "-q", "-m", msg]);
        }
        run(repo, &["git", "push", "-q", bare, "main"]);
        ref_of(repo, "refs/heads/main")
    }

    fn closed_detail(s: &pulse_store::Store, number: i64) -> Value {
        let conn = s.conn().expect("conn");
        crate::util::query_one(
            &conn,
            "SELECT detail FROM events WHERE action = 'land.pr-closed'
             AND json_extract(detail, '$.number') = ? ORDER BY seq DESC LIMIT 1",
            &[&number],
        )
        .unwrap()
        .map(|r| serde_json::from_str(&r.str("detail")).unwrap())
        .unwrap_or(Value::Null)
    }

    fn remote_has(bare: &str, branch: &str) -> bool {
        crate::util::git_timeout(
            bare,
            Duration::from_secs(10),
            &[
                "rev-parse",
                "--verify",
                "-q",
                &format!("refs/heads/{branch}"),
            ],
        )
        .ok()
    }

    #[test]
    fn an_open_experiment_pull_request_whose_change_is_on_main_is_closed_with_a_comment_and_its_branch_deleted()
     {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        gh.track_remote(&bare);
        // #7 lands by a cherry-pick of its one commit, #8 by a squash of
        // its two, #9 not at all.
        let (picked, picked_tip) =
            open_experiment(&s, &repo, &bare, "S-1", &[("one.txt", "one\n")], TEST_NOW);
        let (squashed, _) = open_experiment(
            &s,
            &repo,
            &bare,
            "S-2",
            &[("two.txt", "two\n"), ("three.txt", "three\n")],
            TEST_NOW,
        );
        let (kept, kept_tip) =
            open_experiment(&s, &repo, &bare, "S-3", &[("four.txt", "four\n")], TEST_NOW);
        let pick = land_on_main(&repo, &bare, &["cherry-pick", &picked_tip], "");
        let squash = land_on_main(
            &repo,
            &bare,
            &["merge", "--squash", "-q", "evolve/S-2"],
            "squash S-2",
        );
        std::fs::write(std::path::Path::new(&repo).join("later.txt"), "later\n").unwrap();
        land_on_main(&repo, &bare, &["add", "later.txt"], "later work");

        let lines = tend_proj(&s, &proj, TEST_NOW);

        for (n, by, branch) in [
            (picked, &pick, "evolve/S-1"),
            (squashed, &squash, "evolve/S-2"),
        ] {
            assert!(
                lines.iter().any(|l| l.contains(&format!(
                    "#{n} closed: its change is on main already in {}",
                    crate::recipe::short(by, 12)
                )) && l.contains(&format!("{branch} deleted on origin"))),
                "#{n}: {lines:?}"
            );
            let comment = gh.close_comment_of(n);
            assert!(
                comment.contains(&crate::recipe::short(by, 12)) && comment.contains(branch),
                "#{n} comment: {comment}"
            );
            let d = closed_detail(&s, n);
            assert_eq!(d["reason"], "superseded", "#{n}");
            assert_eq!(d["superseded_by"], json!(by), "#{n}");
            assert_eq!(d["kind"], "experiment");
            assert!(!remote_has(&bare, branch), "{branch} still on the remote");
            assert_eq!(
                rev(&repo, &format!("refs/heads/{branch}")),
                "",
                "{branch} still in the checkout"
            );
        }
        assert!(
            gh.calls().contains(&format!("pr close {picked}")),
            "{:?}",
            gh.calls()
        );
        // The one whose change is not on main stays open, its branch too.
        assert_eq!(gh.close_comment_of(kept), "");
        assert_eq!(closed_detail(&s, kept), Value::Null);
        assert!(remote_has(&bare, "evolve/S-3"));
        assert_eq!(bare_ref(&bare, "refs/heads/evolve/S-3"), kept_tip);
        // Settled once: the next pass closes nothing more.
        let again = tend_proj(&s, &proj, TEST_NOW);
        assert!(!again.iter().any(|l| l.contains("closed")), "{again:?}");
        assert_eq!(
            gh.calls()
                .iter()
                .filter(|c| c.starts_with("pr close"))
                .count(),
            2
        );
    }

    #[test]
    fn an_experiment_pull_request_after_main_was_squashed_to_a_new_root_is_closed_when_main_holds_it()
     {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        gh.track_remote(&bare);
        let (number, _) =
            open_experiment(&s, &repo, &bare, "S-9", &[("nine.txt", "nine\n")], TEST_NOW);
        // main squashed to one root commit holding everything, the
        // experiment's change included.
        run(
            &repo,
            &["git", "checkout", "-q", "--orphan", "squashed", "main"],
        );
        run(
            &repo,
            &["git", "checkout", "-q", "evolve/S-9", "--", "nine.txt"],
        );
        run(&repo, &["git", "commit", "-q", "-m", "squash main"]);
        let root = ref_of(&repo, "refs/heads/squashed");
        run(&repo, &["git", "push", "-q", "-f", &bare, "squashed:main"]);
        run(&repo, &["git", "checkout", "-q", "main"]);

        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(
            lines.iter().any(|l| l.contains(&format!(
                "#{number} closed: its change is on main already in {}",
                crate::recipe::short(&root, 12)
            ))),
            "{lines:?}"
        );
        assert!(!remote_has(&bare, "evolve/S-9"));
    }

    #[test]
    fn an_experiment_pull_request_neither_stacks_nor_is_stacked_on() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        gh.track_remote(&bare);
        let (exp, _) = open_experiment(&s, &repo, &bare, "S-1", &[("one.txt", "one\n")], TEST_NOW);
        // A job's pull request opened after it goes on main, not on it.
        let job = push_branch(&s, &proj, &repo, "pulse/worker/a");
        let lines = tend_proj(&s, &proj, TEST_NOW);
        assert!(lines.iter().any(|l| l.contains("#8 opened")), "{lines:?}");
        assert_eq!(gh.base_of(8), "main");
        let conn = s.conn().unwrap();
        assert!(stack_top(&conn, "proj").is_some_and(|t| number_of(&t) == 8));
        drop(conn);
        // A later experiment goes on main too, with the job's open.
        let mut later = push_branch(&s, &proj, &repo, "evolve/S-2");
        later.insert("kind", json!("experiment"));
        let (n, _, opened) =
            deliver_experiment(&s, &proj, &later, "B-2", "run-2", "kept", TEST_NOW)
                .expect("deliver experiment");
        assert!(opened);
        assert_eq!(gh.base_of(n), "main");
        assert_eq!(gh.base_of(exp), "main");
        assert!(
            is_ancestor(
                &repo,
                &ref_of(&repo, "refs/heads/evolve/S-2"),
                &bare_ref(&bare, "refs/heads/evolve/S-2")
            ) && !is_ancestor(
                &repo,
                &ref_of(&repo, &format!("refs/heads/{}", job.str("branch"))),
                &bare_ref(&bare, "refs/heads/evolve/S-2")
            ),
            "the experiment took in the job's branch"
        );
    }

    #[test]
    fn a_stale_experiment_pull_request_is_brought_up_to_date_or_closed() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let (proj, repo, bare) = push_fixture(&s);
        gh.track_remote(&bare);
        let (fresh, fresh_tip) =
            open_experiment(&s, &repo, &bare, "S-1", &[("one.txt", "one\n")], TEST_NOW);
        let (clash, _) =
            open_experiment(&s, &repo, &bare, "S-2", &[("two.txt", "two\n")], TEST_NOW);
        std::fs::write(std::path::Path::new(&repo).join("two.txt"), "other\n").unwrap();
        let main = land_on_main(&repo, &bare, &["add", "two.txt"], "two differently");

        // Two days on, nothing is stale yet.
        let soon = crate::util::plus_hours(TEST_NOW, 48.0);
        let lines = tend_proj(&s, &proj, &soon);
        assert!(!lines.iter().any(|l| l.contains("no push")), "{lines:?}");
        assert_eq!(bare_ref(&bare, "refs/heads/evolve/S-1"), fresh_tip);

        let later = crate::util::plus_hours(TEST_NOW, 4.0 * 24.0);
        let lines = tend_proj(&s, &proj, &later);
        assert!(
            lines.iter().any(|l| l.contains(&format!(
                "#{fresh} open 4 days with no push; main merged in and pushed"
            ))),
            "{lines:?}"
        );
        let tip = bare_ref(&bare, "refs/heads/evolve/S-1");
        assert!(is_ancestor(&repo, &main, &tip), "S-1 lacks main");
        assert_eq!(gh.close_comment_of(fresh), "");
        assert!(
            lines.iter().any(|l| l.contains(&format!(
                "#{clash} open 4 days with no push; closed: main moved on and no longer merges into it cleanly (two.txt)"
            ))),
            "{lines:?}"
        );
        assert!(gh.close_comment_of(clash).contains("no push for 4 days"));
        assert_eq!(closed_detail(&s, clash)["reason"], "stale");
        assert!(
            remote_has(&bare, "evolve/S-2"),
            "a stale branch stays on the remote"
        );
        // The refresh restarted S-1's clock: the next pass leaves it.
        let again = tend_proj(&s, &proj, &crate::util::plus_hours(&later, 1.0));
        assert!(!again.iter().any(|l| l.contains("no push")), "{again:?}");
    }
}
