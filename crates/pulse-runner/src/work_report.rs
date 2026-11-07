//! The page PULSE builds for every job and session (pulse-harness report.rs
//! `build_full`): where the work stands now, who did it and how, its commits
//! and the files they changed, the checks, and the agent's own report.json
//! and screenshots when it left them. The hub builds it whenever the page is
//! asked for, and when a finished piece of work is listed without one, so
//! the page says where the work stands rather than where it stood.
use anyhow::{Result, anyhow};
use pulse_harness::report::{self, Facts, FileChange};
use pulse_store::ConversationMessage;
use rusqlite::{Connection, OptionalExtension};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::Path;

/// How many commits a page lists.
const COMMITS_SHOWN: usize = 60;
/// How long a summary the page's lede holds.
const SUMMARY_CHARS: usize = 900;

/// Builds the page of job `job_id` in `dir`, its artifacts folder.
pub fn job(s: &pulse_store::Store, job_id: &str, dir: &Path) -> Result<report::Check> {
    let facts = job_facts(s, job_id)?;
    Ok(report::build_full(dir, &facts))
}

/// Builds the page of conversation `id` in `dir`, its artifacts folder.
pub fn session(s: &pulse_store::Store, id: &str, dir: &Path) -> Result<report::Check> {
    let facts = session_facts(s, id)?;
    Ok(report::build_full(dir, &facts))
}

/// A job PULSE is done with: its page can say how it ended.
pub fn job_finished(status: &str) -> bool {
    matches!(status, "done" | "failed" | "cancelled")
}

/// A conversation that is over: finished by its agent, the owner or PULSE.
pub fn session_finished(status: &str) -> bool {
    matches!(status, "closed" | "archived")
}

struct Run {
    id: String,
    agent: String,
    seconds: f64,
    summary: String,
    verify: String,
    executor: String,
}

fn job_facts(s: &pulse_store::Store, job_id: &str) -> Result<Facts> {
    let conn = s.conn()?;
    let (project, kind, title, source, status, result): (
        String,
        String,
        String,
        String,
        String,
        String,
    ) = conn
        .query_row(
            "SELECT project, kind, title, source, status, result FROM jobs WHERE id = ?1",
            [job_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| anyhow!("no job {job_id}"))?;
    let run = conn
        .query_row(
            "SELECT id, agent, COALESCE(seconds, 0.0), summary, verify, executor FROM runs
             WHERE job_id = ?1 ORDER BY started_at DESC LIMIT 1",
            [job_id],
            |r| {
                Ok(Run {
                    id: r.get(0)?,
                    agent: r.get(1)?,
                    seconds: r.get(2)?,
                    summary: r.get(3)?,
                    verify: r.get(4)?,
                    executor: r.get(5)?,
                })
            },
        )
        .optional()?;
    let harness = match &run {
        Some(run) => conn
            .query_row(
                "SELECT model, cost, steps FROM harness_sessions WHERE run_id = ?1 ORDER BY started_at DESC LIMIT 1",
                [&run.id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?, r.get::<_, i64>(2)?)),
            )
            .optional()?,
        None => None,
    };
    let task = pulse_store::task_source_id(&source).map(str::to_string);
    let task_title = match &task {
        Some(id) => conn
            .query_row("SELECT title FROM tasks WHERE id = ?1", [id], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .unwrap_or_default(),
        None => String::new(),
    };
    let found = landings(&conn, job_id, |d| {
        d.get("job").and_then(Value::as_str) == Some(job_id)
            || d.get("jobs")
                .and_then(Value::as_array)
                .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(job_id)))
    })?;
    let project_row = s.project(&project)?;
    let default_branch = project_row
        .as_ref()
        .map(|p| p.default_branch.clone())
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| "main".into());
    let (commits, files) = match &project_row {
        Some(p) => job_commits(&p.path, job_id),
        None => (Vec::new(), Vec::new()),
    };
    let (state, status_line) = job_state(&status, best(&found), commits.len(), &default_branch);

    let mut about = Vec::new();
    if let Some(run) = &run {
        about.push(("Agent".to_string(), run.agent.clone()));
    }
    if let Some((model, cost, steps)) = &harness {
        about.push(("Model".to_string(), model.clone()));
        if *steps > 0 {
            about.push(("Steps".to_string(), steps.to_string()));
        }
        if *cost > 0.0 {
            about.push(("Cost".to_string(), format!("${cost:.2}")));
        }
    }
    if let Some(run) = &run {
        if run.seconds > 0.0 {
            about.push(("Took".to_string(), took(run.seconds)));
        }
        if !run.executor.is_empty() && run.executor != "local" {
            about.push(("Ran on".to_string(), run.executor.clone()));
        }
    }
    about.push(("Job".to_string(), job_id.to_string()));

    let summary = run
        .as_ref()
        .map(|r| clean_summary(&r.summary))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| clean_summary(&result));
    let mut checks = Vec::new();
    if let Some(run) = &run
        && !run.verify.trim().is_empty()
    {
        checks.push(format!("Checks before landing: {}", run.verify.trim()));
    }
    Ok(Facts {
        kind: match &task {
            Some(id) => format!("Task {id}"),
            None => format!("{} job", capital(&kind)),
        },
        title: if task_title.trim().is_empty() {
            title
        } else {
            task_title
        },
        summary: cap(&summary, SUMMARY_CHARS),
        state,
        status: status_line,
        about,
        links: pr_links(best(&found)),
        commits,
        files,
        checks,
        notes: Vec::new(),
        built: built_at(s),
    })
}

fn session_facts(s: &pulse_store::Store, id: &str) -> Result<Facts> {
    let row = s.chat_get(id)?.ok_or_else(|| anyhow!("no session {id}"))?;
    let conn = s.conn()?;
    let messages = s.chat_messages(id, 0).unwrap_or_default();
    let found = if row.branch.is_empty() {
        landings(&conn, id, |d| {
            d.get("conversation").and_then(Value::as_str) == Some(id)
        })?
    } else {
        landings(&conn, &row.branch, |d| {
            d.get("conversation").and_then(Value::as_str) == Some(id)
                || d.get("head").and_then(Value::as_str) == Some(row.branch.as_str())
        })?
    };
    let land = best(&found);
    let proposed = found.iter().find(|l| l.action == "land.pr");
    let project_row = if row.project.is_empty() {
        None
    } else {
        s.project(&row.project)?
    };
    let default_branch = project_row
        .as_ref()
        .map(|p| p.default_branch.clone())
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| "main".into());
    let (commits, files) = match &project_row {
        Some(p) => session_commits(
            &p.path,
            &default_branch,
            &row.branch,
            &proposed.map(|l| l.str("from")).unwrap_or_default(),
            land.map_or(0, Landing::number),
        ),
        None => (Vec::new(), Vec::new()),
    };
    let (state, status_line) = session_state(&row, land, &default_branch);

    let mut notes = Vec::new();
    let mut subs = conn.prepare(
        "SELECT title, status, note FROM conversations WHERE parent = ?1 AND id != ?1 ORDER BY created_at",
    )?;
    for sub in subs.query_map([id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })? {
        let (title, status, note) = sub?;
        let word = match status.as_str() {
            "closed" => "finished",
            "archived" => "archived",
            "running" => "still working",
            "waiting" => "waits for your answer",
            "idle" if note == pulse_harness::conversation::RESTART_NOTE => {
                "stopped by a restart before it finished"
            }
            _ => "open",
        };
        notes.push(format!(
            "Sub-session \u{201c}{}\u{201d}: {word}.",
            untitled(&title)
        ));
    }
    let mut tasks =
        conn.prepare("SELECT id, title, status FROM tasks WHERE created_by = ?1 ORDER BY id")?;
    for task in tasks.query_map([id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })? {
        let (task_id, title, status) = task?;
        notes.push(format!(
            "{task_id} \u{201c}{}\u{201d}: {}.",
            untitled(&title),
            status.replace('_', " ")
        ));
    }

    let mut about = Vec::new();
    if !row.model.is_empty() {
        about.push(("Model".to_string(), row.model.clone()));
    }
    if row.turns > 0 {
        about.push(("Turns".to_string(), row.turns.to_string()));
    }
    let end = if row.last_at.is_empty() {
        &row.updated_at
    } else {
        &row.last_at
    };
    if let Some(secs) = seconds_between(&row.created_at, end)
        && secs > 0.0
    {
        about.push(("Took".to_string(), took(secs)));
    }
    if row.cost > 0.0 {
        about.push(("Cost".to_string(), format!("${:.2}", row.cost)));
    }
    about.push(("Session".to_string(), id.to_string()));
    Ok(Facts {
        kind: if row.parent.is_empty() || row.parent == row.id {
            "Session".into()
        } else {
            "Sub-session".into()
        },
        title: untitled(&row.title),
        summary: cap(&session_summary(&messages), SUMMARY_CHARS),
        state,
        status: status_line,
        about,
        links: pr_links(land),
        commits,
        files,
        checks: Vec::new(),
        notes,
        built: built_at(s),
    })
}

/// A landing event that names the work.
struct Landing {
    action: String,
    at: String,
    detail: Map<String, Value>,
}

impl Landing {
    fn str(&self, key: &str) -> String {
        self.detail
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    }

    fn number(&self) -> i64 {
        self.detail
            .get("number")
            .and_then(Value::as_i64)
            .unwrap_or(0)
    }
}

/// The landing events whose detail mentions `needle` and that `names`
/// confirms, newest first.
fn landings(
    conn: &Connection,
    needle: &str,
    names: impl Fn(&Map<String, Value>) -> bool,
) -> Result<Vec<Landing>> {
    let mut stmt = conn.prepare(
        "SELECT action, at, detail FROM events
         WHERE action >= 'land.' AND action < 'land/' AND detail LIKE ?1
         ORDER BY seq DESC LIMIT 40",
    )?;
    let rows = stmt.query_map([format!("%{needle}%")], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (action, at, detail) = row?;
        if let Ok(Value::Object(detail)) = serde_json::from_str::<Value>(&detail)
            && names(&detail)
        {
            out.push(Landing { action, at, detail });
        }
    }
    Ok(out)
}

/// The landing that says most about where the work stands: merged, landed,
/// a pull request, held - the newest of the first kind there is.
fn best(found: &[Landing]) -> Option<&Landing> {
    ["land.merged", "land.landed", "land.pr", "land.held"]
        .iter()
        .find_map(|want| found.iter().find(|l| l.action == *want))
}

fn pr_links(land: Option<&Landing>) -> Vec<(String, String)> {
    match land {
        Some(l) if l.number() > 0 && !l.str("url").is_empty() => {
            vec![(format!("Pull request #{}", l.number()), l.str("url"))]
        }
        _ => Vec::new(),
    }
}

fn job_state(
    status: &str,
    land: Option<&Landing>,
    commits: usize,
    default_branch: &str,
) -> (String, String) {
    if let Some(l) = land
        && let Some(found) = landed_state(l, default_branch)
    {
        return found;
    }
    let (state, line) = match status {
        "done" if commits > 0 => ("Done", "Finished; its commits have not landed yet."),
        "done" => ("Done", "Finished with no commits to land."),
        "failed" => ("Failed", "The job failed before it finished."),
        "cancelled" => ("Cancelled", "The job was cancelled."),
        "leased" => ("Working", "An agent is working on it now."),
        "queued" | "blocked" => ("Queued", "It waits for an agent."),
        other => return (capital(other), String::new()),
    };
    (state.to_string(), line.to_string())
}

fn session_state(
    row: &pulse_store::Conversation,
    land: Option<&Landing>,
    default_branch: &str,
) -> (String, String) {
    if let Some(l) = land
        && matches!(l.action.as_str(), "land.merged" | "land.landed")
        && let Some(found) = landed_state(l, default_branch)
    {
        return found;
    }
    let (state, line) = match row.status.as_str() {
        "running" => ("Working", "The agent is working on it now.".to_string()),
        "waiting" => ("Waiting for you", "The agent waits for your answer.".to_string()),
        "idle" if row.note == pulse_harness::conversation::RESTART_NOTE => (
            "Stopped",
            "PULSE stopped its turn to restart and nothing carried it on; send it \u{201c}continue\u{201d}."
                .to_string(),
        ),
        _ => match land.and_then(|l| landed_state(l, default_branch)) {
            Some(found) => return found,
            None => match row.status.as_str() {
                "closed" => ("Finished", sentence(&row.note, "Finished.")),
                "archived" => ("Archived", "Put away.".to_string()),
                _ => ("Open", "Open: it waits for your next message.".to_string()),
            },
        },
    };
    (state.to_string(), line)
}

/// Merged, landed, in review or held, from the landing event.
fn landed_state(l: &Landing, default_branch: &str) -> Option<(String, String)> {
    let into = {
        let b = l.str("base");
        if b.is_empty() {
            default_branch.to_string()
        } else {
            b
        }
    };
    let number = l.number();
    let found = match l.action.as_str() {
        "land.merged" => {
            let at = {
                let m = l.str("merged_at");
                if m.is_empty() { l.at.clone() } else { m }
            };
            (
                "Merged",
                if number > 0 {
                    format!("Merged into {into} as pull request #{number}{}.", on(&at))
                } else {
                    format!("Merged into {into}{}.", on(&at))
                },
            )
        }
        "land.landed" => ("Landed", format!("Landed on {into}{}.", on(&l.at))),
        "land.pr" => (
            "In review",
            format!(
                "Pull request #{number} is open for review, since{}.",
                on(&l.at).replacen(" on", "", 1)
            ),
        ),
        "land.held" => {
            let why = [l.str("reason"), l.str("note")]
                .into_iter()
                .find(|t| !t.trim().is_empty())
                .unwrap_or_default();
            (
                "Held",
                if why.is_empty() {
                    format!("Held{}.", on(&l.at))
                } else {
                    format!(
                        "Held{}: {}.",
                        on(&l.at),
                        cap(why.trim_end_matches('.'), 240)
                    )
                },
            )
        }
        _ => return None,
    };
    Some((found.0.to_string(), found.1))
}

/// The commits that carry the job's `Pulse-Job:` trailer, oldest first, and
/// the files they changed. A rebased copy keeps the subject of the commit it
/// copies; the newest copy stands for both.
fn job_commits(repo: &str, job_id: &str) -> (Vec<(String, String)>, Vec<FileChange>) {
    // The whole trailer line, so job-1 does not find job-10's commits.
    let id: String = job_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_string()
            } else {
                format!("\\{c}")
            }
        })
        .collect();
    let grep = format!("--grep=^Pulse-Job: {id}$");
    let out = crate::util::git(
        repo,
        &[
            "log",
            "--all",
            "--reverse",
            "-E",
            &grep,
            "--format=%h%x09%H%x09%s",
        ],
    );
    if !out.ok() {
        return (Vec::new(), Vec::new());
    }
    let mut order: Vec<String> = Vec::new();
    let mut by_subject: BTreeMap<String, (String, String)> = BTreeMap::new();
    for line in out.stdout.lines() {
        let mut parts = line.splitn(3, '\t');
        let (Some(short), Some(full), Some(subject)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if !by_subject.contains_key(subject) {
            order.push(subject.to_string());
        }
        by_subject.insert(subject.to_string(), (short.to_string(), full.to_string()));
    }
    let mut commits = Vec::new();
    let mut files = Vec::new();
    let mut index = BTreeMap::new();
    for subject in order.iter().take(COMMITS_SHOWN) {
        let (short, full) = &by_subject[subject];
        let shown = crate::util::git(repo, &["show", "--numstat", "--format=", full]);
        if shown.ok() {
            numstat(&shown.stdout, &mut files, &mut index);
        }
        commits.push((short.clone(), subject.clone()));
    }
    (commits, files)
}

/// A session's commits: its branch from the commit its pull request was cut
/// from (or where it forks from the default branch), or, once the branch is
/// gone, the commits its pull request's merge brought in.
fn session_commits(
    repo: &str,
    default_branch: &str,
    branch: &str,
    from: &str,
    number: i64,
) -> (Vec<(String, String)>, Vec<FileChange>) {
    let git = |args: &[&str]| crate::util::git(repo, args);
    let has_branch = !branch.is_empty()
        && git(&[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ])
        .ok();
    let (base, head) = if has_branch {
        let base = if !from.is_empty() && git(&["merge-base", "--is-ancestor", from, branch]).ok() {
            from.to_string()
        } else {
            git(&["merge-base", default_branch, branch])
                .stdout
                .trim()
                .to_string()
        };
        (base, branch.to_string())
    } else if number > 0 {
        let merge = git(&[
            "log",
            default_branch,
            "--merges",
            "-F",
            &format!("--grep=Merge pull request #{number} "),
            "--format=%H",
            "-n",
            "1",
        ])
        .stdout
        .trim()
        .to_string();
        if merge.is_empty() {
            return (Vec::new(), Vec::new());
        }
        (format!("{merge}^1"), format!("{merge}^2"))
    } else {
        return (Vec::new(), Vec::new());
    };
    if base.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let log = git(&[
        "log",
        "--reverse",
        "--format=%h%x09%s",
        &format!("{base}..{head}"),
    ]);
    let commits = log
        .stdout
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .take(COMMITS_SHOWN)
        .map(|(sha, subject)| (sha.to_string(), subject.to_string()))
        .collect();
    let mut files = Vec::new();
    let diff = git(&["diff", "--numstat", &format!("{base}...{head}")]);
    if diff.ok() {
        numstat(&diff.stdout, &mut files, &mut BTreeMap::new());
    }
    (commits, files)
}

/// Adds `git --numstat` lines to `files`, one row per path.
fn numstat(out: &str, files: &mut Vec<FileChange>, index: &mut BTreeMap<String, usize>) {
    for line in out.lines() {
        let mut parts = line.splitn(3, '\t');
        let (Some(added), Some(removed), Some(path)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let (added, removed) = (added.parse::<u64>().ok(), removed.parse::<u64>().ok());
        match index.get(path) {
            Some(&i) => {
                let f = &mut files[i];
                f.added = f.added.zip(added).map(|(a, b)| a + b);
                f.removed = f.removed.zip(removed).map(|(a, b)| a + b);
            }
            None => {
                index.insert(path.to_string(), files.len());
                files.push(FileChange {
                    path: path.to_string(),
                    added,
                    removed,
                });
            }
        }
    }
}

/// The session's last word: what its agent said when it finished it, or the
/// first paragraph of its last reply.
fn session_summary(messages: &[ConversationMessage]) -> String {
    if let Some(m) = messages.iter().rev().find(|m| {
        m.role == "note" && m.extra.get("closed").and_then(Value::as_str) == Some("agent")
    }) && let Some(rest) = m
        .content
        .strip_prefix(pulse_harness::conversation::FINISHED_BY_AGENT_LINE)
        && !rest.trim().is_empty()
    {
        return rest.trim().to_string();
    }
    messages
        .iter()
        .rev()
        .find(|m| m.role == "assistant" && !m.content.trim().is_empty())
        .map(|m| first_paragraph(&m.content))
        .unwrap_or_default()
}

/// The first paragraph of a reply that is prose: headings and code blocks
/// are passed over.
fn first_paragraph(text: &str) -> String {
    text.split("\n\n")
        .map(str::trim)
        .find(|p| !p.is_empty() && !p.starts_with('#') && !p.starts_with("```"))
        .unwrap_or_default()
        .replace('\n', " ")
}

/// A worker's SUMMARY without its outcome code ("done - ", "failed: ").
fn clean_summary(text: &str) -> String {
    let t = text.trim();
    let t = t.strip_prefix("SUMMARY:").unwrap_or(t).trim_start();
    for code in ["nothing-to-do", "done", "blocked", "failed"] {
        if let Some(rest) = t.strip_prefix(code) {
            let rest = rest.trim_start();
            if let Some(r) = rest.strip_prefix('-').or_else(|| rest.strip_prefix(':')) {
                return r.trim().to_string();
            }
        }
    }
    t.to_string()
}

fn cap(text: &str, max: usize) -> String {
    let t = text.trim();
    if t.chars().count() <= max {
        return t.to_string();
    }
    let mut out: String = t.chars().take(max).collect();
    if let Some(cut) = out.rfind(' ') {
        out.truncate(cut);
    }
    out.push('\u{2026}');
    out
}

fn capital(word: &str) -> String {
    let mut c = word.chars();
    match c.next() {
        Some(f) => f
            .to_uppercase()
            .chain(c)
            .collect::<String>()
            .replace('_', " "),
        None => String::new(),
    }
}

fn untitled(title: &str) -> String {
    if title.trim().is_empty() {
        "Untitled".into()
    } else {
        title.trim().to_string()
    }
}

/// `note` as a sentence of its own, or `otherwise`.
fn sentence(note: &str, otherwise: &str) -> String {
    let t = note.trim().trim_end_matches('.');
    if t.is_empty() {
        return otherwise.to_string();
    }
    format!("{}.", capital(t))
}

/// " on 7 Nov 06:37 UTC", or nothing for a time that does not read.
fn on(at: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(at)
        .map(|t| {
            format!(
                " on {}",
                t.with_timezone(&chrono::Utc).format("%-d %b %H:%M UTC")
            )
        })
        .unwrap_or_default()
}

fn seconds_between(a: &str, b: &str) -> Option<f64> {
    let a = chrono::DateTime::parse_from_rfc3339(a).ok()?;
    let b = chrono::DateTime::parse_from_rfc3339(b).ok()?;
    Some((b - a).num_seconds() as f64)
}

/// "45s", "12m", "1h 21m", "2d 3h".
fn took(seconds: f64) -> String {
    let s = seconds.round() as i64;
    if s < 60 {
        return format!("{s}s");
    }
    let m = s / 60;
    if m < 60 {
        return format!("{m}m");
    }
    let h = m / 60;
    if h < 48 {
        return if m % 60 == 0 {
            format!("{h}h")
        } else {
            format!("{h}h {}m", m % 60)
        };
    }
    format!("{}d {}h", h / 24, h % 24)
}

fn built_at(s: &pulse_store::Store) -> String {
    chrono::DateTime::parse_from_rfc3339(&s.now())
        .map(|t| {
            t.with_timezone(&chrono::Utc)
                .format("%Y-%m-%d %H:%M UTC")
                .to_string()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{new_repo, new_store, run, serial};
    use serde_json::json;

    fn commit(repo: &str, file: &str, body: &str, message: &str) {
        std::fs::write(Path::new(repo).join(file), body).unwrap();
        run(repo, &["git", "add", file]);
        run(repo, &["git", "commit", "-q", "-m", message]);
    }

    fn task(conn: &Connection, id: &str, title: &str, by: &str) {
        conn.execute(
            "INSERT INTO tasks (id, project, title, status, priority, created_by, created_at, updated_at, current_rev)
             VALUES (?1, 'proj', ?2, 'done', 0, ?3, '2026-11-07T09:00:00Z', '2026-11-07T09:00:00Z', 1)",
            [id, title, by],
        )
        .unwrap();
    }

    fn page(dir: &Path) -> String {
        std::fs::read_to_string(dir.join(report::PAGE)).unwrap()
    }

    #[test]
    fn a_finished_job_s_page_says_where_it_landed_who_did_it_and_what_changed() {
        let _serial = serial();
        let s = new_store();
        let (_p, repo) = new_repo(&s);
        commit(
            &repo,
            "notes.md",
            "one\ntwo\n",
            "docs: notes\n\nPulse-Agent: fjord\nPulse-Job: job-1\n",
        );
        commit(&repo, "other.md", "x\n", "unrelated\n\nPulse-Job: job-10\n");
        let conn = s.conn().unwrap();
        task(&conn, "TASK-9", "Write the notes", "owner");
        conn.execute_batch(
            "INSERT INTO jobs (id, project, title, kind, source, status, created_at, result)
               VALUES ('job-1', 'proj', 'notes', 'task', 'task:TASK-9', 'done', '2026-11-07T09:00:00Z', 'SUMMARY: done - wrote them');
             INSERT INTO runs (id, job_id, agent, project, started_at, finished_at, status, seconds, summary, verify)
               VALUES ('run-1', 'job-1', 'fjord', 'proj', '2026-11-07T09:00:00Z', '2026-11-07T10:21:00Z', 'done', 4860,
                       'done - wrote the notes, `two` lines', 'verified by hook');
             INSERT INTO harness_sessions (id, run_id, model, cost, steps, started_at, updated_at)
               VALUES ('hs-1', 'run-1', 'claude-opus-5.5', 1.5, 42, '2026-11-07T09:00:00Z', '2026-11-07T10:21:00Z');",
        )
        .unwrap();
        crate::util::event_at(
            &conn,
            "2026-11-07T10:30:00Z",
            "pulse",
            "land.merged",
            "proj",
            json!({"project": "proj", "jobs": ["job-1"], "number": 12, "base": "main",
                   "merged_at": "2026-11-07T10:30:00Z", "url": "https://github.com/o/r/pull/12"}),
        );
        drop(conn);
        let tmp = tempfile::tempdir().unwrap();
        let c = job(&s, "job-1", tmp.path()).unwrap();
        assert!(c.page, "{c:?}");
        let html = page(tmp.path());
        // The task's title, not the job's; where it stands, in words.
        assert!(html.contains("<title>Write the notes</title>"), "{html}");
        assert!(html.contains("<span class=\"state ok\">Merged</span>Task TASK-9"));
        assert!(html.contains("Merged into main as pull request #12 on 7 Nov 10:30 UTC."));
        // The worker's summary without its outcome code.
        assert!(html.contains("wrote the notes, <code>two</code> lines"));
        assert!(!html.contains("done - wrote"));
        for fact in [
            "<li>Agent <b>fjord</b></li>",
            "<li>Model <b>claude-opus-5.5</b></li>",
            "<li>Steps <b>42</b></li>",
            "<li>Cost <b>$1.50</b></li>",
            "<li>Took <b>1h 21m</b></li>",
            "<li>Job <b>job-1</b></li>",
        ] {
            assert!(html.contains(fact), "{fact}\n{html}");
        }
        // Its own commit by the trailer; job-10's is not job-1's.
        assert!(html.contains("docs: notes"));
        assert!(!html.contains("unrelated"));
        assert!(html.contains("<code>notes.md</code></td><td class=\"add\">+2</td>"));
        assert!(html.contains("Checks before landing: verified by hook"));
        assert!(html.contains("href=\"https://github.com/o/r/pull/12\""));
    }

    #[test]
    fn a_job_that_has_not_landed_says_so() {
        let _serial = serial();
        let s = new_store();
        let (_p, _repo) = new_repo(&s);
        s.conn()
            .unwrap()
            .execute_batch(
                "INSERT INTO jobs (id, project, title, kind, status, created_at, result)
                   VALUES ('job-2', 'proj', 'Audit the docs', 'improve', 'failed', '2026-11-07T09:00:00Z', 'failed: the tests did not build');",
            )
            .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        job(&s, "job-2", tmp.path()).unwrap();
        let html = page(tmp.path());
        assert!(html.contains("<title>Audit the docs</title>"));
        assert!(html.contains("<span class=\"state bad\">Failed</span>Improve job"));
        assert!(html.contains("The job failed before it finished."));
        assert!(html.contains("the tests did not build"));
        assert!(!html.contains("What changed"));
        assert!(job(&s, "job-404", tmp.path()).is_err());
    }

    #[test]
    fn a_session_s_page_names_its_pull_request_sub_sessions_and_tasks() {
        let _serial = serial();
        let s = new_store();
        let (_p, repo) = new_repo(&s);
        let base = run(&repo, &["git", "rev-parse", "HEAD"]).trim().to_string();
        run(&repo, &["git", "checkout", "-q", "-b", "pulse/chat-chat-1"]);
        commit(&repo, "a.txt", "a\nb\n", "session: a");
        run(&repo, &["git", "checkout", "-q", "-"]);
        let conn = s.conn().unwrap();
        conn.execute(
            "INSERT INTO conversations (id, title, project, branch, status, created_at, updated_at, last_at, turns, note, model)
             VALUES ('chat-1', 'Land the fixes', 'proj', 'pulse/chat-chat-1', 'closed', '2026-11-07T02:00:00Z',
                     '2026-11-07T06:38:00Z', '2026-11-07T06:38:00Z', 7,
                     'pull request #9 merged into main; this session is finished', 'claude-fable-5.1')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO conversations (id, title, project, status, created_at, updated_at, note, parent)
             VALUES ('chat-2', 'RSI B', 'proj', 'idle', '2026-11-07T03:01:00Z', '2026-11-07T06:43:00Z', ?1, 'chat-1'),
                    ('chat-3', 'RSI A', 'proj', 'closed', '2026-11-07T03:00:00Z', '2026-11-07T06:04:00Z', '', 'chat-1')",
            [pulse_harness::conversation::RESTART_NOTE],
        )
        .unwrap();
        task(&conn, "TASK-80", "Judge fixtures", "chat-1");
        conn.execute(
            "INSERT INTO conversation_messages (conversation_id, n, at, role, content, extra)
             VALUES ('chat-1', 1, '2026-11-07T06:30:00Z', 'assistant', '## Done\n\nFiled the tasks and landed the fixes.\n\nMore.', '{}')",
            [],
        )
        .unwrap();
        crate::util::event_at(
            &conn,
            "2026-11-07T06:28:00Z",
            "pulse",
            "land.pr",
            "proj",
            json!({"project": "proj", "conversation": "chat-1", "head": "pulse/chat-chat-1", "from": base,
                   "number": 9, "base": "main"}),
        );
        crate::util::event_at(
            &conn,
            "2026-11-07T06:37:20Z",
            "pulse",
            "land.merged",
            "proj",
            json!({"project": "proj", "conversation": "chat-1", "head": "pulse/chat-chat-1", "number": 9,
                   "base": "main", "merged_at": "2026-11-07T06:37:16Z", "url": "https://github.com/o/r/pull/9"}),
        );
        drop(conn);
        let tmp = tempfile::tempdir().unwrap();
        session(&s, "chat-1", tmp.path()).unwrap();
        let html = page(tmp.path());
        assert!(html.contains("<title>Land the fixes</title>"), "{html}");
        assert!(html.contains("<span class=\"state ok\">Merged</span>Session"));
        assert!(html.contains("Merged into main as pull request #9 on 7 Nov 06:37 UTC."));
        // The reply's first paragraph, past its heading.
        assert!(html.contains("<p class=\"lede\">Filed the tasks and landed the fixes.</p>"));
        assert!(html.contains("session: a"));
        assert!(html.contains("<code>a.txt</code></td><td class=\"add\">+2</td>"));
        assert!(html.contains("Sub-session \u{201c}RSI A\u{201d}: finished."));
        assert!(html.contains(
            "Sub-session \u{201c}RSI B\u{201d}: stopped by a restart before it finished."
        ));
        assert!(html.contains("TASK-80 \u{201c}Judge fixtures\u{201d}: done."));
        assert!(html.contains("<li>Turns <b>7</b></li>"));
        assert!(html.contains("<li>Took <b>4h 38m</b></li>"));
        assert!(html.contains("href=\"https://github.com/o/r/pull/9\""));

        // A sub-session a restart stopped says so where it stands.
        let tmp2 = tempfile::tempdir().unwrap();
        session(&s, "chat-2", tmp2.path()).unwrap();
        let html = page(tmp2.path());
        assert!(
            html.contains("<span class=\"state wait\">Stopped</span>Sub-session"),
            "{html}"
        );
        assert!(html.contains("send it \u{201c}continue\u{201d}"));
    }

    #[test]
    fn summaries_lose_their_outcome_code_and_times_read_short() {
        assert_eq!(clean_summary("SUMMARY: done - fixed it"), "fixed it");
        assert_eq!(
            clean_summary("nothing-to-do - already there"),
            "already there"
        );
        assert_eq!(clean_summary("blocked: needs a key"), "needs a key");
        assert_eq!(clean_summary("donetastic work"), "donetastic work");
        assert_eq!(took(45.0), "45s");
        assert_eq!(took(720.0), "12m");
        assert_eq!(took(3600.0), "1h");
        assert_eq!(took(4860.0), "1h 21m");
        assert_eq!(took(60.0 * 60.0 * 51.0), "2d 3h");
        assert_eq!(on("2026-11-07T06:37:16Z"), " on 7 Nov 06:37 UTC");
        assert_eq!(on("soon"), "");
        assert_eq!(cap("one two three", 7), "one\u{2026}");
    }
}
