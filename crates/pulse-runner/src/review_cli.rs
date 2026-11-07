//! `pulse review`: what the review agent runs inside its job clone to read
//! a pull request, put its head in the clone, and give exactly one verdict
//! (merge, changes, owner). Every GitHub call goes through
//! `review::gh`, which runs as the account that can see the repository; the
//! agent never calls `gh` itself. The owner can run the same commands at a
//! terminal with `--project`.

use anyhow::Result;
use pulse_cli::{Context, FlagSet};
use serde::Serialize;
use std::io::Write;

use crate::review::{self, OpenPr, PrView};
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
        "merge: `owner` merges for the owner, past the complexity rule",
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

fn store(c: &Context) -> Result<&pulse_store::Store> {
    c.store
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("review: store is needed"))
}

fn usage(sub: &str) -> anyhow::Error {
    let what = if sub.is_empty() {
        "pulse review needs a subcommand".to_string()
    } else {
        format!("pulse review: unknown subcommand {sub:?}")
    };
    pulse_cli::errorf(2, &what, "see the usage", "pulse help review").into()
}

fn number_arg(args: &[String]) -> Result<i64> {
    match args
        .first()
        .and_then(|a| a.trim_start_matches('#').parse::<i64>().ok())
    {
        Some(n) if n > 0 => Ok(n),
        _ => Err(pulse_cli::errorf(
            2,
            "Missing pull request number",
            "the command needs the pull request's number",
            "pulse review show 7",
        )
        .into()),
    }
}

fn need_note(fs: &FlagSet, verdict: &str) -> Result<String> {
    let note = crate::commands::str_flag(fs, "note").trim().to_string();
    if note.is_empty() {
        return Err(pulse_cli::errorf(
            2,
            "Missing --note",
            &format!("a {verdict} verdict carries the review's findings"),
            &format!("pulse review {verdict} N --note \"...\""),
        )
        .into());
    }
    Ok(note)
}

/// The project: `--project`, else `PULSE_PROJECT` (a job clone has it),
/// else the checkout the command runs in.
fn project_of(c: &Context) -> Result<Row> {
    crate::env::current_project(store(c)?, c.project.clone())
}

/// What the store knows of the pull request, or a shell for a pull request
/// PULSE did not open (a human's): then the kind is unknown and nothing
/// folds a follow-up into a task.
fn known_pr(s: &pulse_store::Store, slug: &str, number: i64, view: &PrView) -> OpenPr {
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

fn fetch_view(s: &pulse_store::Store, project: &Row, number: i64) -> Result<PrView> {
    let slug = project.str("slug");
    let url = s
        .conn()
        .ok()
        .and_then(|c| review::pr_state(&c, &slug, number))
        .map(|(pr, _)| pr.url)
        .unwrap_or_default();
    let repo = review::repo_of(project, &url);
    review::pr_view(s, &slug, &repo, number).map_err(|e| {
        pulse_cli::errorf(
            1,
            &format!("Cannot read pull request #{number}"),
            &e,
            "gh auth status lists the accounts this machine is logged in as",
        )
        .into()
    })
}

/// A verdict needs an open pull request: one merged or closed on the forge
/// (the owner may have clicked Merge while the review ran) takes none.
fn need_open(view: &PrView) -> Result<()> {
    if view.state.is_empty() || view.state == "OPEN" {
        return Ok(());
    }
    Err(pulse_cli::errorf(
        2,
        &format!(
            "Pull request #{} is {}",
            view.number,
            view.state.to_ascii_lowercase()
        ),
        "a verdict is for an open pull request",
        "finish the job with what you found: pulse job finish $PULSE_JOB done --result '...'",
    )
    .into())
}

/// A pull request stacked on another is merged after it, never before:
/// merging it now would merge it into the branch below, not land it.
fn need_bottom(s: &pulse_store::Store, project: &Row, number: i64) -> Result<()> {
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
    Err(pulse_cli::errorf(
        2,
        &format!("Pull request #{number} is stacked on {below}"),
        &format!("{below} must merge first; merging #{number} now would merge it into {below}'s branch, not land it"),
        &format!("pulse review merge {}", below.trim_start_matches('#')),
    )
    .into())
}

/// A pull request already decided since its last push takes no second
/// verdict: the agent issues exactly one. The owner's merge (`--by
/// owner`) is the answer to a hand-off, so `review.owner` and
/// `review.changes` stand open to it; only a merge is final.
fn need_undecided(
    s: &pulse_store::Store,
    slug: &str,
    number: i64,
    owner_merge: bool,
) -> Result<()> {
    let Some((_, action)) = s
        .conn()
        .ok()
        .and_then(|c| review::pr_state(&c, slug, number))
    else {
        return Ok(());
    };
    if action == "land.pr"
        || (owner_merge && matches!(action.as_str(), "review.owner" | "review.changes"))
    {
        return Ok(());
    }
    Err(pulse_cli::errorf(
        2,
        &format!("Pull request #{number} already has a verdict ({action})"),
        "a review gives exactly one verdict per push of the pull request",
        "finish the job: pulse job finish $PULSE_JOB done --result '...'",
    )
    .into())
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
        "merge" => merge(c, fs, number_arg(rest)?),
        "changes" => changes(c, fs, number_arg(rest)?),
        "owner" => owner(c, fs, number_arg(rest)?),
        other => Err(usage(other)),
    }
}

fn list(c: &mut Context) -> Result<()> {
    let s = store(c)?;
    let conn = s.conn()?;
    let projects: Vec<Row> = if c.project.is_empty()
        && std::env::var("PULSE_PROJECT")
            .unwrap_or_default()
            .is_empty()
    {
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
        return pulse_cli::write_json(&mut c.out, &entries);
    }
    if entries.is_empty() {
        writeln!(c.out, "No open PULSE pull requests.")?;
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
        return pulse_cli::write_json(
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
        "" => "not a PULSE pull request".to_string(),
        "task" => format!("PULSE task {}", pr.task),
        "job" => format!("PULSE job {}", pr.job),
        other => format!("PULSE {other}"),
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
            pulse_cli::errorf(
                1,
                &format!("Cannot read the diff of #{number}"),
                &e,
                "pulse review show N",
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
    let sha = review::checkout_into(s, &project, &view, &dir).map_err(|e| {
        pulse_cli::errorf(
            1,
            &format!("Cannot check out pull request #{number}"),
            &e,
            "run it inside the job's clone",
        )
    })?;
    writeln!(
        c.out,
        "{} at {} ({})",
        review::review_branch(number),
        &sha[..sha.len().min(12)],
        view.head
    )?;
    Ok(())
}

fn merge(c: &mut Context, fs: &FlagSet, number: i64) -> Result<()> {
    let project = project_of(c)?;
    let s = store(c)?;
    let slug = project.str("slug");
    let by = crate::commands::str_flag(fs, "by").trim().to_string();
    if !by.is_empty() && by != "owner" {
        return Err(pulse_cli::errorf(
            2,
            &format!("Unknown --by {by:?}"),
            "only the owner merges past the complexity rule",
            "pulse review merge N --by owner",
        )
        .into());
    }
    need_undecided(s, &slug, number, by == "owner")?;
    need_bottom(s, &project, number)?;
    let view = fetch_view(s, &project, number)?;
    need_open(&view)?;
    let actor = crate::commands::actor_of(c);
    let now = crate::commands::now_of(c);
    if by != "owner" {
        let reasons = review::reasons_for(s, &view);
        if !reasons.is_empty() {
            return Err(pulse_cli::errorf(
                2,
                &format!("Pull request #{number} is for the owner to merge"),
                &reasons.join("; "),
                &format!(
                    "pulse review owner {number} --note \"<your findings>\" hands it over; the owner merges with --by owner"
                ),
            )
            .into());
        }
    }
    let by = if by == "owner" {
        "owner".to_string()
    } else {
        actor.clone()
    };
    let url =
        review::merge_viewed(s, &project, number, Some(&view), &by, &actor, &now).map_err(|e| {
            pulse_cli::errorf(
                1,
                &format!("Merge of #{number} failed"),
                &e,
                "the failure is recorded (review.failed); the owner can merge on GitHub",
            )
        })?;
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

fn changes(c: &mut Context, fs: &FlagSet, number: i64) -> Result<()> {
    let note = need_note(fs, "changes")?;
    let project = project_of(c)?;
    let s = store(c)?;
    let slug = project.str("slug");
    need_undecided(s, &slug, number, false)?;
    let view = fetch_view(s, &project, number)?;
    need_open(&view)?;
    let pr = known_pr(s, &slug, number, &view);
    let actor = crate::commands::actor_of(c);
    let now = crate::commands::now_of(c);
    let job =
        review::request_changes(s, &project, &pr, &view, &note, &actor, &now).map_err(|e| {
            pulse_cli::errorf(
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
    need_undecided(s, &slug, number, false)?;
    let view = fetch_view(s, &project, number)?;
    need_open(&view)?;
    let pr = known_pr(s, &slug, number, &view);
    let reasons = review::reasons_for(s, &view);
    let actor = crate::commands::actor_of(c);
    let now = crate::commands::now_of(c);
    let post = review::escalate(s, &project, &pr, &reasons, &note, &actor, &now).map_err(|e| {
        pulse_cli::errorf(
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
        pulse_testutil::SetenvGuard,
        String,
        std::path::PathBuf,
    ) {
        let bin = tempfile::tempdir().unwrap();
        let log = bin.path().join("gh.log").display().to_string();
        let view = bin.path().join("view");
        std::fs::write(
            &view,
            view_json(7, "pulse/task/TASK-001", &[("src/a.rs", 5, 2)], "OPEN"),
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
        (bin, guard, log, view)
    }

    fn view_json(number: i64, head: &str, files: &[(&str, i64, i64)], state: &str) -> String {
        json!({
            "number": number, "title": format!("PR {number} title"), "body": "what it does",
            "url": format!("https://github.com/o/r/pull/{number}"), "state": state,
            "headRefName": head, "headRefOid": "abc123", "baseRefName": "main",
            "author": {"login": "pulse-bot"},
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

    fn land_pr(s: &pulse_store::Store, number: i64, kind: &str, task: &str, job: &str) {
        let head = if kind == "task" {
            format!("pulse/task/{task}")
        } else {
            format!("pulse/worker/{job}")
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

    fn events(s: &pulse_store::Store, action: &str) -> Vec<Value> {
        crate::util::must_rows(
            &*s.conn().unwrap(),
            "SELECT detail FROM events WHERE action = ? ORDER BY seq",
            &[&action],
        )
        .iter()
        .map(|r| serde_json::from_str(&r.str("detail")).unwrap())
        .collect()
    }

    fn job_by_source(s: &pulse_store::Store, source: &str) -> Vec<Row> {
        crate::util::must_rows(
            &*s.conn().unwrap(),
            "SELECT * FROM jobs WHERE source = ? ORDER BY created_at, id",
            &[&source],
        )
    }

    #[test]
    fn merge_refuses_a_pull_request_with_reasons_unless_the_owner_asks() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
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
        assert!(err.contains("pulse review owner 7"), "{err}");
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
        let _env = pulse_testutil::env_lock();
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

    #[test]
    fn a_large_pull_request_is_the_reviewers_to_merge() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let (_bin, _path, log, view) = fake_gh();
        let s = new_store();
        let (_proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "job", "", "JOB-a");
        std::fs::write(
            &view,
            view_json(
                7,
                "pulse/worker/JOB-a",
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
        let _env = pulse_testutil::env_lock();
        let (_bin, _path, log, _view) = fake_gh();
        let s = new_store();
        let (_proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "task", "TASK-001", "JOB-a");
        let (out, err, code) = invoke(&s, "review", "proj", &["merge", "7"]);
        assert_eq!(code, 0, "{out} {err}");
        assert_eq!(calls(&log), ["pr view 7", "pr merge 7"]);
        let merged = events(&s, "review.merged");
        assert_eq!(merged.len(), 1);
        assert_eq!(
            merged[0]["by"],
            pulse_store::HUMAN_AGENT,
            "the CLI's actor, an agent in a job"
        );
        assert_eq!(merged[0]["number"], 7);
    }

    #[test]
    fn a_merged_or_closed_pull_request_takes_no_verdict() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let (_bin, _path, log, view) = fake_gh();
        let s = new_store();
        let (_proj, _repo, _bare) = push_fixture(&s);
        land_pr(&s, 7, "job", "", "JOB-a");
        std::fs::write(
            &view,
            view_json(7, "pulse/worker/JOB-a", &[("src/a.rs", 5, 2)], "MERGED"),
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
        let _env = pulse_testutil::env_lock();
        let (bin, _path, log, view) = fake_gh();
        let s = new_store();
        let (_proj, _repo, _bare) = push_fixture(&s);
        let task = pulse_tasks::new(
            &s,
            "proj",
            "Add the thing",
            "b",
            0,
            pulse_store::HUMAN_AGENT,
        )
        .unwrap();
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
                "Run `pulse review checkout 7` first so your work sits on the pull request's head"
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
        let msgs = pulse_tasks::messages(&s, "proj", &task.id, "", "").unwrap();
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
            view_json(8, "pulse/worker/JOB-b", &[("src/b.rs", 5, 2)], "OPEN"),
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
                .starts_with("Run `pulse review checkout 8` first"),
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
        let _env = pulse_testutil::env_lock();
        let (_bin, _path, log, view) = fake_gh();
        let s = new_store();
        let (_proj, _repo, _bare) = push_fixture(&s);
        let task = pulse_tasks::new(
            &s,
            "proj",
            "Add the thing",
            "b",
            0,
            pulse_store::HUMAN_AGENT,
        )
        .unwrap();
        land_pr(&s, 7, "task", &task.id, "JOB-a");
        std::fs::write(
            &view,
            view_json(
                7,
                "pulse/task/x",
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
        let msgs = pulse_tasks::messages(&s, "proj", &task.id, "", "").unwrap();
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
        let _env = pulse_testutil::env_lock();
        let (_bin, _path, _log, _view) = fake_gh();
        let s = new_store();
        let (proj, _repo, _bare) = push_fixture(&s);
        let (out, _, code) = invoke(&s, "review", "", &["list"]);
        assert_eq!(code, 0);
        assert!(out.contains("No open PULSE pull requests"), "{out}");
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
        assert!(out.contains("Kind: PULSE task TASK-001"), "{out}");
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
        let _env = pulse_testutil::env_lock();
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
                "url": "https://github.com/o/r/pull/8", "head": "pulse/worker/JOB-b",
                "base": "pulse/task/TASK-001", "stack_on": 7, "to": "abc123", "opened": true,
                "title": "PR 8 title"}),
        );

        let (_, err, code) = invoke(&s, "review", "proj", &["merge", "8", "--by", "owner"]);
        assert_eq!(code, 2, "{err}");
        assert!(err.contains("Pull request #8 is stacked on #7"), "{err}");
        assert!(err.contains("#7 must merge first"), "{err}");
        assert!(err.contains("pulse review merge 7"), "{err}");
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
