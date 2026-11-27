//! `hive review`: what the review agent runs inside its job clone to read
//! a pull request, put its head in the clone, and give exactly one verdict
//! (merge, changes, owner). Every GitHub call goes through
//! `review::gh`, which runs as the account that can see the repository; the
//! agent never calls `gh` itself. The owner can run the same commands at a
//! terminal with `--project`.

use anyhow::Result;
use hive_cli::{Context, FlagSet};
use serde::Serialize;
use std::io::Write;

use crate::review::{self, OpenPr, PrView};
use crate::review_check;
use crate::util::Row;

pub(crate) fn review_flags(fs: &mut FlagSet) {
    fs.string_var(
        "note",
        "",
        "changes|owner: the review's findings (the follow-up job's brief, or what the owner reads)",
    );
    fs.string_var(
        "by",
        "",
        "merge|changes: `owner` gives the owner's verdict (a merge past the complexity rule; changes after a hand-over); a command run outside a job is the owner's",
    );
}

#[derive(Serialize)]
struct ListEntry {
    #[serde(flatten)]
    pr: OpenPr,
    project: String,
    /// `waiting`, `queued`, `running`, or `failed`.
    state: String,
    job: String,
    reasons: Vec<String>,
    /// `#N` when the pull request stacks on one that must merge first.
    #[serde(skip_serializing_if = "Option::is_none")]
    stacked_on: Option<String>,
}

#[derive(Serialize)]
struct ShowOut<'a> {
    project: &'a str,
    #[serde(flatten)]
    view: &'a PrView,
    kind: &'a str,
    task: &'a str,
    reasons: &'a [String],
    care: &'a [String],
}

fn store(c: &Context) -> Result<&hive_store::Store> {
    c.store
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("review: store is needed"))
}

fn usage(sub: &str) -> anyhow::Error {
    let what = if sub.is_empty() {
        "hive review needs a subcommand".to_string()
    } else {
        format!("hive review: unknown subcommand {sub:?}")
    };
    hive_cli::errorf(2, &what, "see the usage", "hive help review").into()
}

fn number_arg(args: &[String]) -> Result<i64> {
    match args
        .first()
        .and_then(|a| a.trim_start_matches('#').parse::<i64>().ok())
    {
        Some(n) if n > 0 => Ok(n),
        _ => Err(hive_cli::errorf(
            2,
            "Missing pull request number",
            "the command needs the pull request's number",
            "hive review show 7",
        )
        .into()),
    }
}

fn need_note(fs: &FlagSet, verdict: &str) -> Result<String> {
    let note = crate::commands::str_flag(fs, "note").trim().to_string();
    if note.is_empty() {
        return Err(hive_cli::errorf(
            2,
            "Missing --note",
            &format!("a {verdict} verdict carries the review's findings"),
            &format!("hive review {verdict} N --note \"...\""),
        )
        .into());
    }
    Ok(note)
}

/// The project: `--project`, else `HIVE_PROJECT` (a job clone has it),
/// else the checkout the command runs in.
fn project_of(c: &Context) -> Result<Row> {
    crate::env::current_project(store(c)?, c.project.clone())
}

/// What the store knows of the pull request, or a shell for a pull request
/// Hive did not open (a human's): then the kind is unknown and nothing
/// folds a follow-up into a task.
fn known_pr(s: &hive_store::Store, slug: &str, number: i64, view: &PrView) -> OpenPr {
    s.conn()
        .ok()
        .and_then(|c| review::pr_state(&c, slug, number))
        .map(|(pr, _)| pr)
        .unwrap_or_else(|| OpenPr {
            number,
            url: view.url.clone(),
            title: view.title.clone(),
            kind: String::new(),
            head: view.head.clone(),
            base: view.base.clone(),
            sha: view.head_sha.clone(),
            ..Default::default()
        })
}

fn fetch_view(s: &hive_store::Store, project: &Row, number: i64) -> Result<PrView> {
    read_view(s, project, number).map_err(|e| view_error(number, &e))
}

/// `gh pr view` of the pull request, its error as gh said it.
fn read_view(
    s: &hive_store::Store,
    project: &Row,
    number: i64,
) -> std::result::Result<PrView, String> {
    let slug = project.str("slug");
    let url = s
        .conn()
        .ok()
        .and_then(|c| review::pr_state(&c, &slug, number))
        .map(|(pr, _)| pr.url)
        .unwrap_or_default();
    let repo = review::repo_of(project, &url);
    review::pr_view(s, &slug, &repo, number)
}

fn view_error(number: i64, e: &str) -> anyhow::Error {
    hive_cli::errorf(
        1,
        &format!("Cannot read pull request #{number}"),
        e,
        "gh auth status lists the accounts this machine is logged in as",
    )
    .into()
}

/// A verdict needs an open pull request: one merged or closed on the forge
/// (the owner may have clicked Merge while the review ran) takes none.
fn need_open(view: &PrView) -> Result<()> {
    if view.state.is_empty() || view.state == "OPEN" {
        return Ok(());
    }
    Err(hive_cli::errorf(
        2,
        &format!(
            "Pull request #{} is {}",
            view.number,
            view.state.to_ascii_lowercase()
        ),
        "a verdict is for an open pull request",
        "finish the job with what you found: hive job finish $HIVE_JOB done --result '...'",
    )
    .into())
}

/// A pull request stacked on another is merged after it, never before:
/// merging it now would merge it into the branch below, not land it.
fn need_bottom(s: &hive_store::Store, project: &Row, number: i64) -> Result<()> {
    let slug = project.str("slug");
    let def = crate::baseline::default_branch(project.clone());
    let Some(below) = s
        .conn()
        .ok()
        .and_then(|c| review::pr_state(&c, &slug, number))
        .and_then(|(pr, _)| review::stacked_on(&pr, &def))
    else {
        return Ok(());
    };
    Err(hive_cli::errorf(
        2,
        &format!("Pull request #{number} is stacked on {below}"),
        &format!("{below} must merge first; merging #{number} now would merge it into {below}'s branch, not land it"),
        &format!("hive review merge {}", below.trim_start_matches('#')),
    )
    .into())
}

/// A pull request already decided since its last push takes no second
/// verdict from its reviewer: the agent issues exactly one. That binds the
/// reviewer only - a hand-over (`review.owner`) asks the owner for theirs,
/// and the owner answers it with any of merge, changes or close:
///
/// - the owner's merge (`--by owner`) stands open after `review.owner`,
///   `review.changes`, `review.failed` and a reviewer's merge that waits
///   (`review.merge-wait`);
/// - the owner's changes after `review.owner` and `review.failed` (after a
///   `review.changes` its follow-up job is already at work, and a waiting
///   merge is the reviewer's go-ahead the owner answers with merge or
///   close).
///
/// The owner's close is GitHub's (the delivery's poll records it as
/// `land.pr-closed` whatever the review said). Only a merge or a close is
/// final.
fn need_undecided(s: &hive_store::Store, slug: &str, number: i64, by: Decider) -> Result<()> {
    let Some((_, action)) = s
        .conn()
        .ok()
        .and_then(|c| review::pr_state(&c, slug, number))
    else {
        return Ok(());
    };
    if by == Decider::Reviewer
        && let Some(job) = std::env::var("HIVE_JOB")
            .ok()
            .filter(|j| !j.trim().is_empty())
        && s.conn()
            .is_ok_and(|c| review::requeued_by(&c, slug, number, job.trim()))
    {
        return Err(hive_cli::errorf(
            2,
            &format!("Pull request #{number} already has a verdict ({})", review::AGAIN_ACTION),
            "this review could not check the head out and queued the review again; a fresh review job reads it",
            "finish the job: hive job finish $HIVE_JOB done --result '...'",
        )
        .into());
    }
    let open_to_owner: &[&str] = match by {
        Decider::Reviewer => &[],
        Decider::OwnerMerge => &[
            "review.owner",
            "review.changes",
            "review.failed",
            review::WAIT_ACTION,
        ],
        Decider::OwnerChanges => &["review.owner", "review.failed"],
    };
    if action == "land.pr" || open_to_owner.contains(&action.as_str()) {
        return Ok(());
    }
    Err(hive_cli::errorf(
        2,
        &format!("Pull request #{number} already has a verdict ({action})"),
        "a review gives exactly one verdict per push of the pull request",
        "finish the job: hive job finish $HIVE_JOB done --result '...'",
    )
    .into())
}

/// Who gives the verdict `need_undecided` checks: the reviewing agent, or
/// the owner answering a hand-over.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Decider {
    Reviewer,
    OwnerMerge,
    OwnerChanges,
}

/// The owner gives this verdict: `--by owner`, or a command run outside
/// any job (a job's run has `HIVE_JOB`; the owner's terminal and the hub
/// have none).
fn by_owner(fs: &FlagSet) -> bool {
    crate::commands::str_flag(fs, "by").trim() == "owner"
        || std::env::var("HIVE_JOB")
            .unwrap_or_default()
            .trim()
            .is_empty()
}

pub(crate) fn cmd_review(c: &mut Context, fs: &FlagSet, args: &[String]) -> Result<()> {
    if args.is_empty() {
        return Err(usage(""));
    }
    let (sub, rest) = (args[0].as_str(), &args[1..]);
    match sub {
        "list" => list(c),
        "show" => show(c, number_arg(rest)?),
        "diff" => diff(c, number_arg(rest)?),
        "checkout" => checkout(c, number_arg(rest)?),
        "check" => check(c, number_arg(rest)?),
        "merge" => merge(c, fs, number_arg(rest)?),
        "changes" => changes(c, fs, number_arg(rest)?),
        "owner" => owner(c, fs, number_arg(rest)?),
        other => Err(usage(other)),
    }
}

fn list(c: &mut Context) -> Result<()> {
    let s = store(c)?;
    let conn = s.conn()?;
    let projects: Vec<Row> =
        if c.project.is_empty() && std::env::var("HIVE_PROJECT").unwrap_or_default().is_empty() {
            crate::board::project_rows(&conn, false)
        } else {
            vec![project_of(c)?]
        };
    let mut entries = Vec::new();
    for p in &projects {
        let slug = p.str("slug");
        let def = crate::baseline::default_branch(p.clone());
        for pr in review::open_prs(&conn, &slug) {
            let job = review::newest_review_job(&conn, &slug, pr.number).map(|(j, _)| j);
            let queued = crate::util::query_rows(
                &conn,
                "SELECT detail FROM events WHERE subject = ? AND action = 'review.queued' AND json_extract(detail, '$.number') = ? ORDER BY seq DESC LIMIT 1",
                &[&slug, &pr.number],
            )
            .unwrap_or_default()
            .into_iter()
            .next()
            .map(|r| serde_json::from_str::<serde_json::Value>(&r.str("detail")).unwrap_or_default());
            let reasons: Vec<String> = queued
                .as_ref()
                .and_then(|q| q["reasons"].as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let (state, job_id) = match &job {
                Some(j) if j.str("status") == "queued" => ("queued", j.str("id")),
                Some(j) if j.str("status") == "leased" => ("running", j.str("id")),
                Some(j) => ("failed", j.str("id")),
                None => ("waiting", String::new()),
            };
            entries.push(ListEntry {
                stacked_on: review::stacked_on(&pr, &def),
                pr,
                project: slug.clone(),
                state: state.to_string(),
                job: job_id,
                reasons,
            });
        }
    }
    if c.json {
        return hive_cli::write_json(&mut c.out, &entries);
    }
    if entries.is_empty() {
        writeln!(c.out, "No open Hive pull requests.")?;
        return Ok(());
    }
    for e in &entries {
        let who = match e.pr.kind.as_str() {
            "task" => format!("task {}", e.pr.task),
            "job" => format!("job {}", e.pr.job),
            other => other.to_string(),
        };
        let mut line = format!(
            "{} #{}  {}  [{}]  {}",
            e.project, e.pr.number, e.pr.title, who, e.state
        );
        if !e.job.is_empty() {
            line.push_str(&format!(" ({})", e.job));
        }
        if !e.reasons.is_empty() {
            line.push_str(&format!("  for the owner: {}", e.reasons.join("; ")));
        }
        if let Some(below) = &e.stacked_on {
            line.push_str(&format!("  stacked on {below}"));
        }
        writeln!(c.out, "{line}")?;
        writeln!(c.out, "    {}", e.pr.url)?;
    }
    Ok(())
}

fn show(c: &mut Context, number: i64) -> Result<()> {
    let project = project_of(c)?;
    let s = store(c)?;
    let slug = project.str("slug");
    let view = fetch_view(s, &project, number)?;
    let pr = known_pr(s, &slug, number, &view);
    let reasons = review::reasons_for(s, &view);
    let care = review::care_for(s, &view);
    if c.json {
        return hive_cli::write_json(
            &mut c.out,
            &ShowOut {
                project: &slug,
                view: &view,
                kind: &pr.kind,
                task: &pr.task,
                reasons: &reasons,
                care: &care,
            },
        );
    }
    writeln!(c.out, "#{} {}", view.number, view.title)?;
    writeln!(c.out, "{}", view.url)?;
    writeln!(
        c.out,
        "{} -> {}  by {}  {}  +{} -{}",
        view.head,
        view.base,
        if view.author.is_empty() {
            "?"
        } else {
            &view.author
        },
        view.state.to_ascii_lowercase(),
        view.additions,
        view.deletions
    )?;
    let kind = match pr.kind.as_str() {
        "" => "not a Hive pull request".to_string(),
        "task" => format!("Hive task {}", pr.task),
        "job" => format!("Hive job {}", pr.job),
        other => format!("Hive {other}"),
    };
    writeln!(c.out, "Kind: {kind}")?;
    if reasons.is_empty() {
        writeln!(
            c.out,
            "May be merged by the reviewer when correct, safe, in scope and green."
        )?;
        for care in &care {
            writeln!(c.out, "Read with extra care: {care}")?;
        }
    } else {
        writeln!(c.out, "For the owner (do not merge):")?;
        for r in &reasons {
            writeln!(c.out, "  - {r}")?;
        }
    }
    writeln!(c.out)?;
    writeln!(c.out, "Files ({}):", view.files.len())?;
    for (p, a, d) in &view.files {
        writeln!(c.out, "  {p}  +{a} -{d}")?;
    }
    if !view.body.trim().is_empty() {
        writeln!(c.out)?;
        writeln!(c.out, "{}", view.body.trim())?;
    }
    Ok(())
}

fn diff(c: &mut Context, number: i64) -> Result<()> {
    let project = project_of(c)?;
    let s = store(c)?;
    let slug = project.str("slug");
    let url = s
        .conn()
        .ok()
        .and_then(|conn| review::pr_state(&conn, &slug, number))
        .map(|(pr, _)| pr.url)
        .unwrap_or_default();
    let text =
        review::pr_diff(s, &slug, &review::repo_of(&project, &url), number).map_err(|e| {
            hive_cli::errorf(
                1,
                &format!("Cannot read the diff of #{number}"),
                &e,
                "hive review show N",
            )
        })?;
    write!(c.out, "{text}")?;
    if !text.ends_with('\n') {
        writeln!(c.out)?;
    }
    Ok(())
}

fn checkout(c: &mut Context, number: i64) -> Result<()> {
    let project = project_of(c)?;
    let s = store(c)?;
    let view = fetch_view(s, &project, number)?;
    let dir = std::env::current_dir()?.display().to_string();
    let sha = match review::checkout_into(s, &project, &view, &dir) {
        Ok(sha) => sha,
        Err(e) => return Err(checkout_failed(c, &project, &view, number, &e)),
    };
    writeln!(
        c.out,
        "{} at {} ({})",
        review::review_branch(number),
        &sha[..sha.len().min(12)],
        view.head
    )?;
    Ok(())
}

/// A checkout that failed. Inside the pull request's own review job, whose
/// pull request is still open for review, the failure is that job's
/// verdict: `review.again` queues a fresh review of the same head (the
/// attempt does not count towards the owner's hand-over), and the agent
/// finishes its job. Anywhere else it is a plain error.
fn checkout_failed(
    c: &Context,
    project: &Row,
    view: &PrView,
    number: i64,
    e: &str,
) -> anyhow::Error {
    let slug = project.str("slug");
    let what = format!("Cannot check out pull request #{number}");
    let requeued = store(c).is_ok_and(|s| {
        review::requeue_after_checkout(
            s,
            &slug,
            view,
            e,
            &crate::commands::actor_of(c),
            &crate::commands::now_of(c),
        )
    });
    if requeued {
        return hive_cli::errorf(
            1,
            &what,
            e,
            &format!(
                "this review's verdict is recorded ({}): the pull request is queued for a fresh review, not handed to the owner - finish the job: hive job finish $HIVE_JOB done --result 'SUMMARY: reviewed #{number}: could not check out, queued again'",
                review::AGAIN_ACTION
            ),
        )
        .into();
    }
    hive_cli::errorf(1, &what, e, "run it inside the job's clone").into()
}

fn merge(c: &mut Context, fs: &FlagSet, number: i64) -> Result<()> {
    let project = project_of(c)?;
    let s = store(c)?;
    let slug = project.str("slug");
    let by = crate::commands::str_flag(fs, "by").trim().to_string();
    if !by.is_empty() && by != "owner" {
        return Err(hive_cli::errorf(
            2,
            &format!("Unknown --by {by:?}"),
            "only the owner merges past the complexity rule",
            "hive review merge N --by owner",
        )
        .into());
    }
    need_undecided(
        s,
        &slug,
        number,
        if by == "owner" {
            Decider::OwnerMerge
        } else {
            Decider::Reviewer
        },
    )?;
    need_bottom(s, &project, number)?;
    let actor = crate::commands::actor_of(c);
    let now = crate::commands::now_of(c);
    // A reviewer's verdict whose read of the pull request failed for a
    // passing reason (a 5xx, a rate limit, a dropped connection) stands
    // like one whose merge did: it waits, and the daemon reads the pull
    // request again - the owner's rule and the forge's gate included -
    // before it merges. A failure that does not pass (401, 403, 404) is
    // an error as before.
    let view = match read_view(s, &project, number) {
        Ok(v) => v,
        Err(e) if by != "owner" && review::passing_forge_error(&e) => {
            let words = review::wait_unread(s, &project, number, &e, &actor, &actor, &now);
            writeln!(
                c.out,
                "Merge of #{number} waits: {words}. The verdict stands; the daemon reads the pull request again and merges it without another review - finish the job."
            )?;
            return Ok(());
        }
        Err(e) => return Err(view_error(number, &e)),
    };
    need_open(&view)?;
    if by != "owner" {
        let reasons = review::reasons_for(s, &view);
        if !reasons.is_empty() {
            return Err(hive_cli::errorf(
                2,
                &format!("Pull request #{number} is for the owner to merge"),
                &reasons.join("; "),
                &format!(
                    "hive review owner {number} --note \"<your findings>\" hands it over; the owner merges with --by owner"
                ),
            )
            .into());
        }
    }
    // The reviewer's merge needs a green check of the head the forge would
    // merge, run through the runner (`hive review check N`) for this
    // review; the owner's `--by owner` is the owner's call and skips it.
    if by != "owner"
        && let Some(said) = refuse_without_green(s, &project, &view, &actor, &now)?
    {
        writeln!(c.out, "{said}")?;
        return Ok(());
    }
    let by = if by == "owner" {
        "owner".to_string()
    } else {
        actor.clone()
    };
    let failed = |e: String| {
        hive_cli::errorf(
            1,
            &format!("Merge of #{number} failed"),
            &e,
            "the failure is recorded (review.failed); the owner can merge on GitHub",
        )
    };
    // The owner's merge is refused or failed outright, as the hub's button
    // shows it; a reviewer's verdict waits for running checks or a forge
    // that answered 5xx, and the daemon merges it later.
    let url = if by == "owner" {
        review::merge_viewed(s, &project, number, Some(&view), &by, &actor, &now).map_err(failed)?
    } else {
        match review::merge_or_wait(s, &project, number, Some(&view), &by, &actor, &now)
            .map_err(failed)?
        {
            review::Verdict::Merged(url) => url,
            review::Verdict::Waits(words) => {
                writeln!(
                    c.out,
                    "Merge of #{number} waits: {words}. The verdict stands; the daemon merges it without another review - finish the job."
                )?;
                return Ok(());
            }
        }
    };
    writeln!(
        c.out,
        "Merged #{number}{} as {by}",
        if url.is_empty() {
            String::new()
        } else {
            format!(" ({url})")
        }
    )?;
    Ok(())
}

/// Refuses a reviewer's merge without a green check of `view`'s head
/// recorded for this review; `None` when the merge may go on. A check that
/// was red on that head and green again is flaky: the pull request is
/// handed to the owner with the tests' names instead, and the words to
/// print come back (the verdict is given), so a flaky test never teaches
/// the reviewer that red is fine.
fn refuse_without_green(
    s: &hive_store::Store,
    project: &Row,
    view: &PrView,
    actor: &str,
    now: &str,
) -> Result<Option<String>> {
    let number = view.number;
    let slug = project.str("slug");
    let conn = s.conn()?;
    let why = match review_check::merge_gate(&conn, &slug, number, &view.head_sha) {
        Ok(_) => return Ok(None),
        Err(why) => why,
    };
    drop(conn);
    let checks = s
        .conn()
        .map(|conn| review_check::recorded(&conn, &slug, number))
        .unwrap_or_default();
    let newest_green_here = checks
        .last()
        .is_some_and(|n| n.green && n.head_sha == view.head_sha);
    let red_here = checks
        .iter()
        .rev()
        .find(|r| r.head_sha == view.head_sha && r.red());
    if let (true, Some(red)) = (newest_green_here, red_here) {
        let pr = known_pr(s, &slug, number, view);
        let tests = if red.failures.is_empty() {
            "the check".to_string()
        } else {
            red.failures.join(", ")
        };
        let reasons = vec![format!(
            "the check of {} was red and then green on the same head: {tests} failed once and passed again (flaky); the reviewer does not merge a red check, even a flaky one",
            &view.head_sha[..view.head_sha.len().min(12)]
        )];
        let note = format!("{}\n\nRed run: {}", why, red.line(number));
        let post = review::escalate(s, project, &pr, &reasons, &note, actor, now).map_err(|e| {
            hive_cli::errorf(
                1,
                &format!("Could not hand #{number} to the owner"),
                &e,
                "the failure is recorded (review.failed)",
            )
        })?;
        return Ok(Some(format!(
            "Not merged: {why}. #{number} handed to the owner with the flaky tests ({tests}){} - finish the job.",
            if post.is_empty() {
                String::new()
            } else {
                format!(", question {post}")
            }
        )));
    }
    Err(hive_cli::errorf(
        2,
        &format!("Pull request #{number} has no green check of its head"),
        &why,
        &format!(
            "hive review check {number} runs the project's check here and records it; a red one is a `hive review changes {number}` or `hive review owner {number}` verdict, never a merge"
        ),
    )
    .into())
}

#[cfg(test)]
thread_local! {
    /// The clone `hive review check` runs in, for a test: the process's
    /// working directory is shared by every test thread.
    static TEST_CLONE: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

/// The job's clone: the working directory the command was run in.
fn clone_dir() -> Result<String> {
    #[cfg(test)]
    {
        let dir = TEST_CLONE.with(|d| d.borrow().clone());
        if !dir.is_empty() {
            return Ok(dir);
        }
    }
    Ok(std::env::current_dir()?.display().to_string())
}

/// `hive review check N`: runs the project's check in this clone, on the
/// pull request's head, and records it (`review.check`). A red check whose
/// failing tests all pass when re-run alone is flaky: the fix is queued on
/// the pull request's head as this review's verdict.
fn check(c: &mut Context, number: i64) -> Result<()> {
    let project = project_of(c)?;
    let s = &store(c)?.clone();
    let slug = project.str("slug");
    let view = fetch_view(s, &project, number)?;
    need_open(&view)?;
    let dir = clone_dir()?;
    let actor = crate::commands::actor_of(c);
    let now = crate::commands::now_of(c);
    let rec = review_check::run(s, &project, &view, &dir, &actor, &now).map_err(|e| {
        hive_cli::errorf(
            2,
            &format!("Cannot check #{number}"),
            &e,
            &format!(
                "hive review checkout {number} in the job's clone, then hive review check {number}"
            ),
        )
    })?;
    writeln!(c.out, "{}", rec.line(number).replacen("check", "Check", 1))?;
    if rec.green {
        writeln!(
            c.out,
            "Recorded for this review: `hive review merge {number}` may merge this head."
        )?;
        return Ok(());
    }
    if !rec.flaky.is_empty() && need_undecided(s, &slug, number, Decider::Reviewer).is_ok() {
        let pr = known_pr(s, &slug, number, &view);
        let note = review_check::flaky_note(&rec, number);
        let job =
            review::request_changes(s, &project, &pr, &view, &note, &actor, &now).map_err(|e| {
                hive_cli::errorf(
                    1,
                    &format!("Could not queue the fix of the flaky tests of #{number}"),
                    &e,
                    "the failure is recorded (review.failed)",
                )
            })?;
        writeln!(
            c.out,
            "Not merged: the failing tests passed when re-run alone, so they are flaky ({}). The fix is queued as this review's verdict: {} ({}) makes them deterministic on the pull request's head - finish the job.",
            rec.flaky.join(", "),
            job.str("id"),
            job.str("kind")
        )?;
        return Ok(());
    }
    writeln!(
        c.out,
        "Red: never merged. Give `hive review changes {number} --note \"<the failures and the fix>\"` or `hive review owner {number} --note \"...\"`."
    )?;
    Ok(())
}

fn changes(c: &mut Context, fs: &FlagSet, number: i64) -> Result<()> {
    let note = need_note(fs, "changes")?;
    let project = project_of(c)?;
    let s = store(c)?;
    let slug = project.str("slug");
    let by = crate::commands::str_flag(fs, "by").trim().to_string();
    if !by.is_empty() && by != "owner" {
        return Err(hive_cli::errorf(
            2,
            &format!("Unknown --by {by:?}"),
            "only the owner answers a hand-over with changes",
            "hive review changes N --note \"...\" --by owner",
        )
        .into());
    }
    let owner = by_owner(fs);
    need_undecided(
        s,
        &slug,
        number,
        if owner {
            Decider::OwnerChanges
        } else {
            Decider::Reviewer
        },
    )?;
    let view = fetch_view(s, &project, number)?;
    need_open(&view)?;
    let pr = known_pr(s, &slug, number, &view);
    let actor = crate::commands::actor_of(c);
    let now = crate::commands::now_of(c);
    let job =
        review::request_changes(s, &project, &pr, &view, &note, &actor, &now).map_err(|e| {
            hive_cli::errorf(
                1,
                &format!("Could not ask for changes on #{number}"),
                &e,
                "the failure is recorded (review.failed)",
            )
        })?;
    writeln!(
        c.out,
        "Changes requested on #{number}; {} ({}, {}) makes them",
        job.str("id"),
        job.str("kind"),
        job.str("source")
    )?;
    Ok(())
}

fn owner(c: &mut Context, fs: &FlagSet, number: i64) -> Result<()> {
    let note = need_note(fs, "owner")?;
    let project = project_of(c)?;
    let s = store(c)?;
    let slug = project.str("slug");
    need_undecided(s, &slug, number, Decider::Reviewer)?;
    let view = fetch_view(s, &project, number)?;
    need_open(&view)?;
    let pr = known_pr(s, &slug, number, &view);
    let reasons = review::reasons_for(s, &view);
    let actor = crate::commands::actor_of(c);
    let now = crate::commands::now_of(c);
    let post = review::escalate(s, &project, &pr, &reasons, &note, &actor, &now).map_err(|e| {
        hive_cli::errorf(
            1,
            &format!("Could not hand #{number} to the owner"),
            &e,
            "the failure is recorded (review.failed)",
        )
    })?;
    writeln!(
        c.out,
        "#{number} handed to the owner{}",
        if post.is_empty() {
            String::new()
        } else {
            format!(" (question {post})")
        }
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::*;
    use crate::test_helpers_daemon::*;
    use crate::test_helpers_provider::invoke;
    use serde_json::{Value, json};

    fn fake_gh() -> (
        tempfile::TempDir,
        hive_testutil::SetenvGuard,
        String,
        std::path::PathBuf,
    ) {
        let bin = tempfile::tempdir().unwrap();
        let log = bin.path().join("gh.log").display().to_string();
        let view = bin.path().join("view");
        std::fs::write(
            &view,
            view_json(7, "hive/task/TASK-001", &[("src/a.rs", 5, 2)], "OPEN"),
        )
        .unwrap();
        let fail = bin.path().join("fail");
        std::fs::write(&fail, "").unwrap();
        std::fs::write(
            bin.path().join("failmsg"),
            "HTTP 422: Can not request changes on your own pull request",
        )
        .unwrap();
        let body = format!(
            "#!/bin/sh\necho \"$@\" >> '{log}'\nif grep -q \"^$2$\" '{fail}'; then echo \"$(cat '{failmsg}')\" >&2; exit 1; fi\ncase \"$1 $2\" in\n\"pr view\") cat '{view}' ;;\n\"pr diff\") echo 'diff --git a/src/a.rs b/src/a.rs' ;;\n\"pr merge\") echo merged ;;\n\"pr review\") echo reviewed ;;\n\"pr comment\") echo commented ;;\n\"auth status\") exit 1 ;;\n*) echo \"unexpected gh $*\" >&2; exit 2 ;;\nesac\n",
            view = view.display(),
            fail = fail.display(),
            failmsg = bin.path().join("failmsg").display(),
        );
        let path = bin.path().join("gh");
        std::fs::write(&path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let guard = hive_testutil::setenv(
            "PATH",
            format!(
                "{}:{}",
                bin.path().display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        );
        (bin, guard, log, view)
    }

    fn view_json(number: i64, head: &str, files: &[(&str, i64, i64)], state: &str) -> String {
        json!({
            "number": number, "title": format!("PR {number} title"), "body": "what it does",
            "url": format!("https://github.com/o/r/pull/{number}"), "state": state,
            "headRefName": head, "headRefOid": "abc123", "baseRefName": "main",
            "author": {"login": "hive-bot"},
            "additions": files.iter().map(|f| f.1).sum::<i64>(), "deletions": files.iter().map(|f| f.2).sum::<i64>(),
            "files": files.iter().map(|(p, a, d)| json!({"path": p, "additions": a, "deletions": d})).collect::<Vec<_>>(),
        })
        .to_string()
    }

    fn calls(log: &str) -> Vec<String> {
        std::fs::read_to_string(log)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with("pr "))
            .map(|l| l.split_whitespace().take(3).collect::<Vec<_>>().join(" "))
            .collect()
    }

    fn land_pr(s: &hive_store::Store, number: i64, kind: &str, task: &str, job: &str) {
        let head = if kind == "task" {
            format!("hive/task/{task}")
        } else {
            format!("hive/worker/{job}")
        };
        let mut d = json!({
            "project": "proj", "kind": kind, "number": number, "url": format!("https://github.com/o/r/pull/{number}"),
            "head": head, "base": "main", "to": "abc123", "opened": true, "title": format!("PR {number} title"),
        });
        if !task.is_empty() {
            d["task"] = json!(task);
        }
        if !job.is_empty() {
            d["job"] = json!(job);
        }
        crate::util::event_at(&s.conn().unwrap(), TEST_NOW, "daemon", "land.pr", "proj", d);
    }

    /// Records a `review.check` of pull request `number` at `head`, as
    /// `hive review check` would after its run.
    fn record_check(
        s: &hive_store::Store,
        number: i64,
        head: &str,
        green: bool,
        failures: &[&str],
    ) {
        crate::util::event_at(
            &s.conn().unwrap(),
            TEST_NOW,
            "worker",
            review_check::ACTION,
            "proj",
            json!({"project": "proj", "number": number, "head_sha": head, "exit": if green { 0 } else { 1 },
                "green": green, "log": format!("/logs/review/proj-pr{number}-{head}.log"), "cmd": "make check",
                "failures": failures, "flaky": []}),
        );
    }

    /// A clone of the project's checkout for `hive review check` to run in,
    /// and the commit it is at.
    fn review_clone(repo: &str) -> (tempfile::TempDir, String, ClonePin) {
        let dir = tempfile::tempdir().unwrap();
        let clone = dir.path().join("clone").display().to_string();
        run(".", &["git", "clone", "-q", repo, &clone]);
        let sha = run(&clone, &["git", "rev-parse", "HEAD"])
            .trim()
            .to_string();
        TEST_CLONE.with(|d| *d.borrow_mut() = clone);
        (dir, sha, ClonePin)
    }

    struct ClonePin;
    impl Drop for ClonePin {
        fn drop(&mut self) {
            TEST_CLONE.with(|d| d.borrow_mut().clear());
        }
    }

    fn set_check(s: &hive_store::Store, cmd: &str) {
        exec1(
            s,
            "UPDATE projects SET verify_cmd = ? WHERE slug = 'proj'",
            &[&cmd],
        );
        set_setting(s, "daemon.min_free_mb", "0");
    }

    fn events(s: &hive_store::Store, action: &str) -> Vec<Value> {
        crate::util::must_rows(
            &*s.conn().unwrap(),
            "SELECT detail FROM events WHERE action = ? ORDER BY seq",
            &[&action],
        )
        .iter()
        .map(|r| serde_json::from_str(&r.str("detail")).unwrap())
        .collect()
    }

    fn job_by_source(s: &hive_store::Store, source: &str) -> Vec<Row> {
        crate::util::must_rows(
            &*s.conn().unwrap(),
            "SELECT * FROM jobs WHERE source = ? ORDER BY created_at, id",
            &[&source],
        )
    }

    #[test]
    fn merge_refuses_a_pull_request_with_reasons_unless_the_owner_asks() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, log, view) = fake_gh();
        let s = new_store();
        let (_proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "job", "", "JOB-a");
        std::fs::write(
            &view,
            view_json(7, "feature/by-hand", &[("src/a.rs", 5, 2)], "OPEN"),
        )
        .unwrap();

        let (out, err, code) = invoke(&s, "review", "proj", &["merge", "7"]);
        assert_eq!(code, 2, "{out} {err}");
        assert!(err.contains("opened by a human/other tool"), "{err}");
        assert!(err.contains("hive review owner 7"), "{err}");
        assert_eq!(calls(&log), ["pr view 7"], "no merge was attempted");
        assert!(events(&s, "review.merged").is_empty());

        let (out, err, code) = invoke(&s, "review", "proj", &["merge", "7", "--by", "owner"]);
        assert_eq!(code, 0, "{out} {err}");
        assert!(out.contains("Merged #7"), "{out}");
        assert_eq!(calls(&log), ["pr view 7", "pr view 7", "pr merge 7"]);
        let merged = events(&s, "review.merged");
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0]["by"], "owner");
        assert_eq!(merged[0]["url"], "https://github.com/o/r/pull/7");

        // Decided: a second verdict is refused before GitHub is asked to merge.
        let (_, err, code) = invoke(&s, "review", "proj", &["merge", "7", "--by", "owner"]);
        assert_eq!(code, 2, "{err}");
        assert!(
            err.contains("already has a verdict (review.merged)"),
            "{err}"
        );
        assert_eq!(
            calls(&log)
                .iter()
                .filter(|c| c.starts_with("pr merge"))
                .count(),
            1
        );
    }

    #[test]
    fn the_owner_merges_a_pull_request_handed_to_them() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, log, view) = fake_gh();
        let s = new_store();
        let (_proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "job", "", "JOB-a");
        std::fs::write(
            &view,
            view_json(7, "feature/by-hand", &[("src/a.rs", 5, 2)], "OPEN"),
        )
        .unwrap();
        let (out, err, code) = invoke(
            &s,
            "review",
            "proj",
            &["owner", "7", "--note", "fine from my side"],
        );
        assert_eq!(code, 0, "{out} {err}");
        assert_eq!(events(&s, "review.owner").len(), 1);

        // The hand-off is the verdict the agent owed; the owner's merge is
        // the answer to it, not a second verdict.
        let (out, err, code) = invoke(&s, "review", "proj", &["merge", "7", "--by", "owner"]);
        assert_eq!(code, 0, "{out} {err}");
        assert!(out.contains("Merged #7"), "{out}");
        assert!(
            calls(&log).iter().any(|c| c == "pr merge 7"),
            "{:?}",
            calls(&log)
        );
        let merged = events(&s, "review.merged");
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0]["by"], "owner");

        // The agent itself still issues exactly one verdict.
        let (_, err, code) = invoke(&s, "review", "proj", &["merge", "7"]);
        assert_eq!(code, 2, "{err}");
        assert!(err.contains("already has a verdict"), "{err}");
    }

    /// TASK-147: one verdict per push binds the reviewer only. After the
    /// hand-over (`review.owner`) the owner's own verdict stays open:
    /// `hive review changes` from the owner's terminal queues the
    /// follow-up job (`pr-fix:<project>:<n>`), while the reviewing job is
    /// still refused a second verdict.
    #[test]
    fn the_owner_asks_for_changes_after_the_hand_over() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, log, view) = fake_gh();
        let s = new_store();
        let (_proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "job", "", "JOB-a");
        std::fs::write(
            &view,
            view_json(7, "hive/worker/JOB-a", &[("src/a.rs", 5, 2)], "OPEN"),
        )
        .unwrap();
        let (out, err, code) = invoke(
            &s,
            "review",
            "proj",
            &["owner", "7", "--note", "touches the auth flow"],
        );
        assert_eq!(code, 0, "{out} {err}");
        assert_eq!(events(&s, "review.owner").len(), 1);

        // The reviewing job gave its one verdict.
        {
            let _job = hive_testutil::setenv("HIVE_JOB", "job-review-7");
            let (_, err, code) = invoke(&s, "review", "proj", &["changes", "7", "--note", "x"]);
            assert_eq!(code, 2, "{err}");
            assert!(
                err.contains("already has a verdict (review.owner)"),
                "{err}"
            );
        }
        assert!(job_by_source(&s, "pr-fix:proj:7").is_empty());

        // The owner's changes are the answer to the hand-over.
        let note = "keep the old token check beside the new one";
        let (out, err, code) = invoke(&s, "review", "proj", &["changes", "7", "--note", note]);
        assert_eq!(code, 0, "{out} {err}");
        let jobs = job_by_source(&s, "pr-fix:proj:7");
        assert_eq!(jobs.len(), 1, "{out}");
        assert_eq!(jobs[0].str("kind"), "improve");
        assert!(
            jobs[0].str("brief").contains(note),
            "{}",
            jobs[0].str("brief")
        );
        assert!(out.contains(&jobs[0].str("id")), "{out}");
        let ev = events(&s, "review.changes");
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0]["follow_up"], jobs[0].str("id"));
        assert!(calls(&log).iter().any(|c| c == "pr review 7"));

        // Its follow-up is at work: a second changes is refused, the
        // owner's merge still stands open.
        let (_, err, code) = invoke(
            &s,
            "review",
            "proj",
            &["changes", "7", "--note", "again", "--by", "owner"],
        );
        assert_eq!(code, 2, "{err}");
        assert!(
            err.contains("already has a verdict (review.changes)"),
            "{err}"
        );
        let (_, err, code) = invoke(
            &s,
            "review",
            "proj",
            &["changes", "7", "--note", "x", "--by", "me"],
        );
        assert_eq!(code, 2, "{err}");
        assert!(err.contains("Unknown --by"), "{err}");
        let (out, err, code) = invoke(&s, "review", "proj", &["merge", "7", "--by", "owner"]);
        assert_eq!(code, 0, "{out} {err}");
        assert_eq!(events(&s, "review.merged").len(), 1);
    }

    #[test]
    fn a_reviewers_merge_needs_a_green_check_of_the_head_recorded_for_this_review() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, log, _view) = fake_gh();
        let s = new_store();
        let (_proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        let merges = |log: &str| {
            calls(log)
                .iter()
                .filter(|c| c.starts_with("pr merge"))
                .count()
        };

        // No check recorded: refused, and the command says how to get one.
        let (out, err, code) = invoke(&s, "review", "proj", &["merge", "7"]);
        assert_eq!(code, 2, "{out} {err}");
        assert!(err.contains("no green check of its head"), "{err}");
        assert!(err.contains("no check of #7 is recorded"), "{err}");
        assert!(err.contains("hive review check 7"), "{err}");
        assert_eq!(merges(&log), 0);

        // A red check of the head: refused, naming the check and its log.
        record_check(&s, 7, "abc123", false, &["a::b"]);
        let (_, err, code) = invoke(&s, "review", "proj", &["merge", "7"]);
        assert_eq!(code, 2, "{err}");
        assert!(err.contains("check of #7 red at abc123 (exit 1)"), "{err}");
        assert!(err.contains("failed: a::b"), "{err}");
        assert!(
            err.contains("log /logs/review/proj-pr7-abc123.log"),
            "{err}"
        );
        assert!(err.contains("a red check is never merged"), "{err}");
        assert_eq!(merges(&log), 0);

        // A green check of an older head: refused, the head moved since.
        record_check(&s, 7, "0ld0ld", true, &[]);
        let (_, err, code) = invoke(&s, "review", "proj", &["merge", "7"]);
        assert_eq!(code, 2, "{err}");
        assert!(
            err.contains("the newest check ran on 0ld0ld, but the head of #7 is abc123"),
            "{err}"
        );
        assert!(
            err.contains("log /logs/review/proj-pr7-0ld0ld.log"),
            "{err}"
        );
        assert_eq!(merges(&log), 0);

        // A check recorded before this review was queued is not this review's.
        record_check(&s, 7, "abc123", true, &[]);
        crate::util::event_at(
            &s.conn().unwrap(),
            TEST_NOW,
            "daemon",
            "review.queued",
            "proj",
            json!({"project": "proj", "number": 7, "sha": "abc123", "job": "JOB-r"}),
        );
        let (_, err, code) = invoke(&s, "review", "proj", &["merge", "7"]);
        assert_eq!(code, 2, "{err}");
        assert!(err.contains("no check of #7 is recorded"), "{err}");
        assert_eq!(merges(&log), 0);
        assert!(events(&s, "review.merged").is_empty());

        // A green check of the head, for this review: merged.
        record_check(&s, 7, "abc123", true, &[]);
        let (out, err, code) = invoke(&s, "review", "proj", &["merge", "7"]);
        assert_eq!(code, 0, "{out} {err}");
        assert!(out.contains("Merged #7"), "{out}");
        assert_eq!(merges(&log), 1);
    }

    #[test]
    fn the_owners_merge_needs_no_recorded_check() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, log, _view) = fake_gh();
        let s = new_store();
        let (_proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        record_check(&s, 7, "abc123", false, &["a::b"]);
        let (out, err, code) = invoke(&s, "review", "proj", &["merge", "7", "--by", "owner"]);
        assert_eq!(code, 0, "{out} {err}");
        assert!(out.contains("Merged #7"), "{out}");
        assert_eq!(calls(&log), ["pr view 7", "pr merge 7"]);
        assert_eq!(events(&s, "review.merged")[0]["by"], "owner");
    }

    #[test]
    fn a_check_red_then_green_on_one_head_is_handed_to_the_owner_not_merged() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, log, _view) = fake_gh();
        let s = new_store();
        let (_proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        record_check(&s, 7, "abc123", false, &["store::tests::races"]);
        record_check(&s, 7, "abc123", true, &[]);
        let (out, err, code) = invoke(&s, "review", "proj", &["merge", "7"]);
        assert_eq!(code, 0, "{out} {err}");
        assert!(out.contains("Not merged"), "{out}");
        assert!(
            out.contains("handed to the owner with the flaky tests (store::tests::races)"),
            "{out}"
        );
        assert!(
            !calls(&log).iter().any(|c| c.starts_with("pr merge")),
            "{:?}",
            calls(&log)
        );
        assert!(events(&s, "review.merged").is_empty());
        let owner = events(&s, "review.owner");
        assert_eq!(owner.len(), 1);
        assert!(
            owner[0].to_string().contains("store::tests::races"),
            "{}",
            owner[0]
        );
        // The verdict is given: the reviewer cannot merge it afterwards.
        let (_, err, code) = invoke(&s, "review", "proj", &["merge", "7"]);
        assert_eq!(code, 2, "{err}");
        assert!(
            err.contains("already has a verdict (review.owner)"),
            "{err}"
        );
    }

    #[test]
    fn review_check_runs_the_check_on_the_head_and_records_it() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, log, view) = fake_gh();
        let s = new_store();
        let (_proj, repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "job", "", "JOB-a");
        let (_dir, sha, _pin) = review_clone(&repo);

        // The clone is not at the pull request's head: nothing is run or recorded.
        set_check(&s, "echo all good");
        let (_, err, code) = invoke(&s, "review", "proj", &["check", "7"]);
        assert_eq!(code, 2, "{err}");
        assert!(err.contains("run `hive review checkout 7` first"), "{err}");
        assert!(events(&s, review_check::ACTION).is_empty());

        // The pull request as Hive pushed it: its head is the clone's commit.
        crate::util::event_at(
            &s.conn().unwrap(),
            TEST_NOW,
            "daemon",
            "land.pr",
            "proj",
            json!({"project": "proj", "kind": "job", "job": "JOB-a", "number": 7, "url": "https://github.com/o/r/pull/7",
                "head": "hive/worker/JOB-a", "base": "main", "to": sha, "opened": false, "title": "PR 7 title"}),
        );
        let at_head = view_json(7, "hive/worker/JOB-a", &[("src/a.rs", 5, 2)], "OPEN")
            .replace("abc123", &sha);
        std::fs::write(&view, &at_head).unwrap();
        let (out, err, code) = invoke(&s, "review", "proj", &["check", "7"]);
        assert_eq!(code, 0, "{out} {err}");
        assert!(
            out.contains(&format!("Check of #7 green at {}", &sha[..12])),
            "{out}"
        );
        let checks = events(&s, review_check::ACTION);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0]["head_sha"], sha.as_str());
        assert_eq!(checks[0]["exit"], 0);
        assert_eq!(checks[0]["green"], true);
        let log_path = checks[0]["log"].as_str().unwrap().to_string();
        assert!(log_path.contains("/logs/review/proj-pr7-"), "{log_path}");
        let kept = std::fs::read_to_string(&log_path).unwrap();
        assert!(
            kept.contains("$ echo all good") && kept.contains("all good\n"),
            "{kept}"
        );

        let (out, err, code) = invoke(&s, "review", "proj", &["merge", "7"]);
        assert_eq!(code, 0, "{out} {err}");
        assert!(out.contains("Merged #7"), "{out}");
        assert!(calls(&log).iter().any(|c| c == "pr merge 7"));
    }

    #[test]
    fn a_red_check_whose_failures_pass_alone_queues_their_fix_and_never_merges() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, log, view) = fake_gh();
        let s = new_store();
        let (_proj, repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "job", "", "JOB-a");
        let (_dir, sha, _pin) = review_clone(&repo);
        std::fs::write(
            &view,
            view_json(7, "hive/worker/JOB-a", &[("src/a.rs", 5, 2)], "OPEN")
                .replace("abc123", &sha),
        )
        .unwrap();
        set_check(&s, "printf 'test a::flaky ... FAILED\\n'; exit 1");

        // Failing alone too: plainly red, nothing is queued.
        set_setting(
            &s,
            review_check::RERUN_SETTING,
            "printf 'test %s ... FAILED\\n' {test}; exit 1",
        );
        let (out, err, code) = invoke(&s, "review", "proj", &["check", "7"]);
        assert_eq!(code, 0, "{out} {err}");
        assert!(
            out.contains("red at") && out.contains("failed: a::flaky"),
            "{out}"
        );
        assert!(out.contains("Red: never merged"), "{out}");
        assert!(events(&s, "review.changes").is_empty());
        assert_eq!(events(&s, review_check::ACTION)[0]["flaky"], json!([]));

        // Passing alone: flaky, and the fix is queued as the verdict.
        set_setting(
            &s,
            review_check::RERUN_SETTING,
            "printf 'test %s ... ok\\n' {test}",
        );
        let (out, err, code) = invoke(&s, "review", "proj", &["check", "7"]);
        assert_eq!(code, 0, "{out} {err}");
        assert!(
            out.contains("a::flaky passed when re-run alone (flaky)"),
            "{out}"
        );
        assert!(out.contains("The fix is queued"), "{out}");
        let checks = events(&s, review_check::ACTION);
        assert_eq!(checks.len(), 2);
        assert_eq!(checks[1]["flaky"], json!(["a::flaky"]));
        assert_eq!(checks[1]["green"], false);
        let changes = events(&s, "review.changes");
        assert_eq!(changes.len(), 1);
        assert!(
            changes[0]["note"]
                .as_str()
                .unwrap_or_default()
                .contains("a::flaky"),
            "{}",
            changes[0]
        );
        // Never merged: the verdict is given, and no merge was asked of GitHub.
        let (_, err, code) = invoke(&s, "review", "proj", &["merge", "7"]);
        assert_eq!(code, 2, "{err}");
        assert!(
            !calls(&log).iter().any(|c| c.starts_with("pr merge")),
            "{:?}",
            calls(&log)
        );
        assert!(events(&s, "review.merged").is_empty());
    }

    #[test]
    fn a_large_pull_request_is_the_reviewers_to_merge() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, log, view) = fake_gh();
        let s = new_store();
        let (_proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "job", "", "JOB-a");
        std::fs::write(
            &view,
            view_json(
                7,
                "hive/worker/JOB-a",
                &[("data/words.json", 900, 100), ("src/a.rs", 5, 2)],
                "OPEN",
            ),
        )
        .unwrap();
        let (out, _, code) = invoke(&s, "review", "proj", &["show", "7"]);
        assert_eq!(code, 0, "{out}");
        assert!(out.contains("May be merged by the reviewer"), "{out}");
        assert!(
            out.contains("Read with extra care: 2 files / 1007 lines changed"),
            "{out}"
        );
        record_check(&s, 7, "abc123", true, &[]);
        let (out, err, code) = invoke(&s, "review", "proj", &["merge", "7"]);
        assert_eq!(code, 0, "{out} {err}");
        assert!(
            calls(&log).iter().any(|c| c == "pr merge 7"),
            "{:?}",
            calls(&log)
        );
        assert_eq!(events(&s, "review.merged").len(), 1);
    }

    #[test]
    fn merge_of_a_simple_pull_request_records_the_agent() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, log, _view) = fake_gh();
        let s = new_store();
        let (_proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        record_check(&s, 7, "abc123", true, &[]);
        let (out, err, code) = invoke(&s, "review", "proj", &["merge", "7"]);
        assert_eq!(code, 0, "{out} {err}");
        assert!(out.contains("Merged #7"), "{out}");
        assert_eq!(calls(&log), ["pr view 7", "pr merge 7"]);
        let merged = events(&s, "review.merged");
        assert_eq!(merged.len(), 1);
        assert_eq!(
            merged[0]["by"],
            hive_store::HUMAN_AGENT,
            "the CLI's actor, an agent in a job"
        );
        assert_eq!(merged[0]["number"], 7);
    }

    #[test]
    fn a_reviewers_merge_the_forge_refuses_with_5xx_keeps_the_verdict() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (bin, _path, log, _view) = fake_gh();
        let s = new_store();
        let (_proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        std::fs::write(bin.path().join("fail"), "merge\n").unwrap();
        std::fs::write(
            bin.path().join("failmsg"),
            "GraphQL: Something went wrong while executing your query (mergePullRequest)",
        )
        .unwrap();
        record_check(&s, 7, "abc123", true, &[]);
        let (out, err, code) = invoke(&s, "review", "proj", &["merge", "7"]);
        assert_eq!(code, 0, "{out} {err}");
        assert!(
            out.contains("Merge of #7 waits: approved; GitHub refused the merge"),
            "{out}"
        );
        assert!(out.contains("finish the job"), "{out}");
        assert!(events(&s, "review.failed").is_empty());
        assert_eq!(events(&s, "review.merge-wait").len(), 1);
        // The verdict stands: a second reviewer verdict is refused...
        let (_, err, code) = invoke(&s, "review", "proj", &["changes", "7", "--note", "x"]);
        assert_eq!(code, 2, "{err}");
        assert!(
            err.contains("already has a verdict (review.merge-wait)"),
            "{err}"
        );
        // ...and the owner may still merge it by hand.
        std::fs::write(bin.path().join("fail"), "").unwrap();
        let (out, err, code) = invoke(&s, "review", "proj", &["merge", "7", "--by", "owner"]);
        assert_eq!(code, 0, "{out} {err}");
        assert_eq!(
            calls(&log)
                .iter()
                .filter(|c| c.starts_with("pr merge"))
                .count(),
            2
        );
    }

    #[test]
    fn a_reviewers_merge_whose_read_fails_for_a_passing_reason_waits() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (bin, _path, log, _view) = fake_gh();
        let s = new_store();
        let (_proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        std::fs::write(bin.path().join("fail"), "view\n").unwrap();

        // A credential refused is no passing failure: an error as before,
        // nothing recorded.
        std::fs::write(
            bin.path().join("failmsg"),
            "HTTP 403: Resource not accessible by integration",
        )
        .unwrap();
        let (_, err, code) = invoke(&s, "review", "proj", &["merge", "7"]);
        assert_eq!(code, 1, "{err}");
        assert!(err.contains("Cannot read pull request #7"), "{err}");
        assert!(events(&s, "review.merge-wait").is_empty());

        // A 502 on the read: the verdict stands as a forge wait, nothing
        // merged, nothing failed.
        std::fs::write(
            bin.path().join("failmsg"),
            "HTTP 502: Bad Gateway (https://api.github.com/graphql)",
        )
        .unwrap();
        let (out, err, code) = invoke(&s, "review", "proj", &["merge", "7"]);
        assert_eq!(code, 0, "{out} {err}");
        assert!(
            out.contains("Merge of #7 waits: approved; GitHub refused the merge (HTTP 502"),
            "{out}"
        );
        assert!(out.contains("finish the job"), "{out}");
        assert!(events(&s, "review.failed").is_empty());
        let wait = events(&s, "review.merge-wait");
        assert_eq!(wait.len(), 1);
        assert_eq!(wait[0]["waiting"], "forge");
        assert_eq!(wait[0]["attempt"], 1);
        assert!(
            !calls(&log).iter().any(|c| c.starts_with("pr merge")),
            "{:?}",
            calls(&log)
        );

        // The owner's own Merge never waits: its failed read is an error.
        let (_, err, code) = invoke(&s, "review", "proj", &["merge", "7", "--by", "owner"]);
        assert_eq!(code, 1, "{err}");
        assert!(err.contains("Cannot read pull request #7"), "{err}");
    }

    #[test]
    fn a_merged_or_closed_pull_request_takes_no_verdict() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, log, view) = fake_gh();
        let s = new_store();
        let (_proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "job", "", "JOB-a");
        std::fs::write(
            &view,
            view_json(7, "hive/worker/JOB-a", &[("src/a.rs", 5, 2)], "MERGED"),
        )
        .unwrap();
        let (_, err, code) = invoke(&s, "review", "proj", &["merge", "7"]);
        assert_eq!(code, 2, "{err}");
        assert!(err.contains("#7 is merged"), "{err}");
        let (_, err, code) = invoke(&s, "review", "proj", &["changes", "7", "--note", "x"]);
        assert_eq!(code, 2, "{err}");
        assert_eq!(calls(&log), ["pr view 7", "pr view 7"]);
        assert!(events(&s, "review.merged").is_empty());
        assert!(events(&s, "review.changes").is_empty());
    }

    #[test]
    fn changes_queues_a_task_job_for_a_task_pull_request_and_an_improve_job_otherwise() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (bin, _path, log, view) = fake_gh();
        let s = new_store();
        let (_proj, _repo, _bare) = push_fixture(&s);
        let task =
            hive_tasks::new(&s, "proj", "Add the thing", "b", 0, hive_store::HUMAN_AGENT).unwrap();
        land_pr(&s, 7, "task", &task.id, "JOB-a");
        land_pr(&s, 8, "job", "", "JOB-b");

        let (_, err, code) = invoke(&s, "review", "proj", &["changes", "7"]);
        assert_eq!(code, 2, "{err}");
        assert!(err.contains("Missing --note"), "{err}");

        let note = "src/a.rs: the new loop never breaks; add the bound and a test";
        let (out, err, code) = invoke(&s, "review", "proj", &["changes", "7", "--note", note]);
        assert_eq!(code, 0, "{out} {err}");
        let jobs = job_by_source(&s, &format!("task:{}", task.id));
        assert_eq!(jobs.len(), 1, "{out}");
        let job = &jobs[0];
        assert_eq!(job.str("kind"), "task");
        assert_eq!(job.str("status"), "queued");
        assert!(
            job.str("brief").starts_with(
                "Run `hive review checkout 7` first so your work sits on the pull request's head"
            ),
            "{}",
            job.str("brief")
        );
        assert!(job.str("brief").contains(note), "{}", job.str("brief"));
        assert!(out.contains(&job.str("id")), "{out}");
        let ev = events(&s, "review.changes");
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0]["number"], 7);
        assert_eq!(ev[0]["note"], note);
        assert_eq!(ev[0]["follow_up"], job.str("id"));
        assert_eq!(calls(&log), ["pr view 7", "pr review 7"]);
        let msgs = hive_tasks::messages(&s, "proj", &task.id, "", "").unwrap();
        assert!(msgs.iter().any(|m| m.body.contains(note)), "{msgs:?}");

        // A second verdict on the same push is refused.
        let (_, err, code) = invoke(&s, "review", "proj", &["changes", "7", "--note", "again"]);
        assert_eq!(code, 2, "{err}");
        assert!(
            err.contains("already has a verdict (review.changes)"),
            "{err}"
        );
        assert_eq!(job_by_source(&s, &format!("task:{}", task.id)).len(), 1);

        // A job pull request: GitHub refuses the review verdict from the
        // author account, so the note becomes a comment; the follow-up is
        // an improve job on pr-fix:<slug>:<number>.
        std::fs::write(bin.path().join("fail"), "review\n").unwrap();
        std::fs::write(
            &view,
            view_json(8, "hive/worker/JOB-b", &[("src/b.rs", 5, 2)], "OPEN"),
        )
        .unwrap();
        let (out, err, code) = invoke(
            &s,
            "review",
            "proj",
            &["changes", "8", "--note", "rename it"],
        );
        assert_eq!(code, 0, "{out} {err}");
        let jobs = job_by_source(&s, "pr-fix:proj:8");
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].str("kind"), "improve");
        assert!(
            jobs[0]
                .str("brief")
                .starts_with("Run `hive review checkout 8` first"),
            "{}",
            jobs[0].str("brief")
        );
        assert!(jobs[0].str("brief").contains("rename it"));
        let tail = &calls(&log)[2..];
        assert_eq!(tail, ["pr view 8", "pr review 8", "pr comment 8"]);
        assert_eq!(events(&s, "review.changes").len(), 2);
        assert!(events(&s, "review.failed").is_empty());
    }

    #[test]
    fn owner_comments_records_and_asks_on_the_board() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, log, view) = fake_gh();
        let s = new_store();
        let (_proj, _repo, _bare) = push_fixture(&s);
        let task =
            hive_tasks::new(&s, "proj", "Add the thing", "b", 0, hive_store::HUMAN_AGENT).unwrap();
        land_pr(&s, 7, "task", &task.id, "JOB-a");
        std::fs::write(
            &view,
            view_json(
                7,
                "hive/task/x",
                &[("src/a.rs", 5, 2), (".env", 1, 0)],
                "OPEN",
            ),
        )
        .unwrap();
        let (out, err, code) = invoke(
            &s,
            "review",
            "proj",
            &[
                "owner",
                "7",
                "--note",
                "Looks right; the .env change needs your eyes",
            ],
        );
        assert_eq!(code, 0, "{out} {err}");
        assert_eq!(calls(&log), ["pr view 7", "pr comment 7"]);
        let ev = events(&s, "review.owner");
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0]["reasons"], json!(["touches .env"]));
        assert_eq!(
            ev[0]["note"],
            "Looks right; the .env change needs your eyes"
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM posts WHERE kind = 'question' AND json_extract(extra, '$.for_owner') = 1 AND summary LIKE 'Merge pull request #7%'",
                &[]
            ),
            1
        );
        let msgs = hive_tasks::messages(&s, "proj", &task.id, "", "").unwrap();
        assert!(
            msgs.iter().any(|m| m.body.contains("touches .env")),
            "{msgs:?}"
        );
        // Handed over: the maintenance pass queues no review for it.
        let proj = crate::util::project_row(&s, "proj");
        assert!(review::tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW).is_empty());
    }

    #[test]
    fn list_and_show_read_the_open_pull_requests() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, _log, _view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        let (out, _, code) = invoke(&s, "review", "", &["list"]);
        assert_eq!(code, 0);
        assert!(out.contains("No open Hive pull requests"), "{out}");
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        land_pr(&s, 8, "job", "", "JOB-b");
        review::tend_reviews(&s, std::slice::from_ref(&proj), TEST_NOW);
        let (out, _, code) = invoke(&s, "review", "", &["list"]);
        assert_eq!(code, 0);
        assert!(
            out.contains("proj #7  PR 7 title  [task TASK-001]  queued (job-"),
            "{out}"
        );
        assert!(
            out.contains("proj #8  PR 8 title  [job JOB-b]  waiting"),
            "{out}"
        );
        let (out, _, code) = invoke(&s, "review", "proj", &["list"]);
        assert_eq!(code, 0);
        assert_eq!(out.matches("proj #").count(), 2, "{out}");

        let (out, err, code) = invoke(&s, "review", "proj", &["show", "7"]);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("#7 PR 7 title"), "{out}");
        assert!(out.contains("Kind: Hive task TASK-001"), "{out}");
        assert!(out.contains("src/a.rs  +5 -2"), "{out}");
        assert!(out.contains("May be merged by the reviewer"), "{out}");
        let (out, _, code) = invoke(&s, "review", "proj", &["diff", "7"]);
        assert_eq!(code, 0);
        assert!(out.starts_with("diff --git"), "{out}");
        let (_, err, code) = invoke(&s, "review", "proj", &["show"]);
        assert_eq!(code, 2);
        assert!(err.contains("Missing pull request number"), "{err}");
    }

    #[test]
    fn merge_refuses_a_stacked_pull_request_and_list_says_what_it_stacks_on() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let (_bin, _path, log, _view) = fake_gh();
        let s = new_store();
        let (_proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        crate::util::event_at(
            &s.conn().unwrap(),
            TEST_NOW,
            "daemon",
            "land.pr",
            "proj",
            json!({"project": "proj", "kind": "job", "job": "JOB-b", "number": 8,
                "url": "https://github.com/o/r/pull/8", "head": "hive/worker/JOB-b",
                "base": "hive/task/TASK-001", "stack_on": 7, "to": "abc123", "opened": true,
                "title": "PR 8 title"}),
        );

        let (_, err, code) = invoke(&s, "review", "proj", &["merge", "8", "--by", "owner"]);
        assert_eq!(code, 2, "{err}");
        assert!(err.contains("Pull request #8 is stacked on #7"), "{err}");
        assert!(err.contains("#7 must merge first"), "{err}");
        assert!(err.contains("hive review merge 7"), "{err}");
        assert!(calls(&log).is_empty(), "{:?}", calls(&log));
        assert!(events(&s, "review.merged").is_empty());
        assert!(events(&s, "review.failed").is_empty());

        let (out, _, code) = invoke(&s, "review", "proj", &["list"]);
        assert_eq!(code, 0);
        assert!(
            out.contains("proj #8  PR 8 title  [job JOB-b]  waiting  stacked on #7"),
            "{out}"
        );
        assert!(
            out.contains("proj #7  PR 7 title  [task TASK-001]  waiting\n"),
            "{out}"
        );
    }
}
