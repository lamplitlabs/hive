//! Drafting a tailored project goal (TASK-117).
//!
//! The generated goal ([`crate::get`] without an owner row) is a template:
//! the README's purpose sentence and the commit history's path. A draft is
//! the comprehensive version: a model side-call reads the project's evidence
//! (the README and the docs, the recent commits, the accepted ADRs, the
//! board's goal readings, the tasks and the owner's own words) and writes a
//! goal with a `Why:` paragraph. The draft never becomes the goal by itself:
//! it is posted as a proposal on `goal/amend` (the shape [`crate::review`]
//! reads), the owner takes it with one yes (`hive decide`) or dismisses it,
//! and a dismissed draft is never proposed again unchanged.
//!
//! The model call and the board post live in the runner (this crate knows
//! neither a provider nor the board); this module holds the evidence, the
//! prompt, the reply's rules, the comparison with the goal in force and with
//! dismissed drafts, and the re-check schedule the daemon keeps in `meta`.

use crate::review::{AMEND_THREAD, AREA, parse_proposal};
use anyhow::{Result, anyhow};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Days between the daemon's re-drafts of a project's goal; 0 turns the
/// daemon's drafting off.
pub const RECHECK_DAYS_SETTING: &str = "goal.recheck_days";
pub const DEFAULT_RECHECK_DAYS: i64 = 14;
/// The author of a drafted proposal on the board.
pub const DRAFTER: &str = "goal-drafter";
/// The `meta` key prefix of a project's last re-check.
pub const META_PREFIX: &str = "goal.recheck.";
/// Two goals whose word sets overlap at least this much say the same thing:
/// a draft this close to the goal in force, to the waiting proposal or to a
/// dismissed draft is not proposed.
pub const SAME_SHARE: f64 = 0.8;
/// An event trigger (an accepted ADR, the owner's words, a review verdict)
/// re-drafts at most this often; the cadence is the floor.
pub const MIN_GAP_HOURS: i64 = 6;
/// A failed draft (the model did not answer, the reply broke the rules) is
/// tried again after this long.
pub const RETRY_MINUTES: i64 = 60;
/// A lead's fresh reading at or above this score says the goal is reached.
pub const REACHED_SCORE: i32 = 90;

/// What drafting did: `proposed` (a post waits for the owner), or why it
/// stopped short - `same-as-goal`, `dismissed-before` (the owner dismissed
/// the same draft), `already-proposed` (the same draft waits).
pub const PROPOSED: &str = "proposed";
pub const SAME_AS_GOAL: &str = "same-as-goal";
pub const DISMISSED_BEFORE: &str = "dismissed-before";
pub const ALREADY_PROPOSED: &str = "already-proposed";

const README_CHARS: usize = 6000;
const DOC_FILES: usize = 24;
const DOC_CHARS: usize = 300;
const COMMITS: usize = 30;
const ADRS: usize = 20;
const READINGS: usize = 3;
const TASKS: usize = 25;
const OWNER_WORDS: usize = 12;
const OWNER_WORD_CHARS: usize = 500;
const DISMISSED_SHOWN: usize = 3;

/// Words that make the owner's chat or task message a statement of
/// direction rather than an ordinary answer.
const DIRECTION_WORDS: &[&str] = &[
    "goal",
    "direction",
    "vision",
    "focus",
    "priorit",
    "mission",
    "strategy",
    "roadmap",
];

/// The evidence one draft rests on.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub project: String,
    pub name: String,
    pub goal: crate::Goal,
    pub readme: String,
    /// `docs/<file>: <first heading or line>`.
    pub docs: Vec<String>,
    /// `YYYY-MM-DD subject`, newest first.
    pub commits: Vec<String>,
    /// `ADR-NNN title`, newest first.
    pub adrs: Vec<String>,
    /// The board's goal readings (`goal/review`) and the store's own verdict.
    pub readings: Vec<String>,
    /// `TASK-NNN [status] title`, newest first.
    pub tasks: Vec<String>,
    /// What the owner said: task briefs, messages, chat - newest first.
    pub owner: Vec<String>,
    /// Drafts the owner dismissed, newest first.
    pub dismissed: Vec<String>,
}

/// A draft that passed the goal rules: `text` is exactly what the owner
/// takes with a yes (the proposal body read back through
/// [`parse_proposal`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    pub text: String,
    pub why: String,
    pub body: String,
}

/// The result of one drafting run, for the CLI and the daemon's line.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    pub project: String,
    pub trigger: String,
    pub status: String,
    /// The proposal post: the new one, or the one this draft matched.
    pub post: String,
    pub text: String,
    pub why: String,
    pub model: String,
}

impl Outcome {
    /// One line for the daemon log and the CLI.
    pub fn line(&self) -> String {
        match self.status.as_str() {
            PROPOSED => format!(
                "goal: proposed a drafted goal for {} as {} ({}); the owner takes it with `hive decide {} yes`",
                self.project, self.post, self.trigger, self.post
            ),
            SAME_AS_GOAL => format!(
                "goal: the drafted goal for {} matches the goal in force ({}); nothing proposed",
                self.project, self.trigger
            ),
            DISMISSED_BEFORE => format!(
                "goal: the drafted goal for {} matches a draft the owner dismissed ({}); nothing proposed",
                self.project, self.post
            ),
            ALREADY_PROPOSED => format!(
                "goal: the drafted goal for {} matches the proposal waiting for the owner ({}); nothing proposed",
                self.project, self.post
            ),
            other => format!("goal: {} for {}", other, self.project),
        }
    }
}

/// Where a draft stands against the goal in force, the waiting proposal
/// and the dismissed drafts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
    New,
    SameAsGoal,
    Waiting(String),
    Dismissed(String),
}

// ---------------------------------------------------------------- evidence

fn clip(text: &str, limit: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut out: String = text.chars().take(limit).collect();
    out.push_str(" ...");
    out
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn readme(path: &str) -> String {
    use std::io::Read;
    let dir = std::path::Path::new(path.trim());
    if path.trim().is_empty() {
        return String::new();
    }
    for name in crate::README_NAMES {
        let file = dir.join(name);
        if !file.is_file() {
            continue;
        }
        let mut bytes = Vec::new();
        if std::fs::File::open(&file)
            .and_then(|f| f.take(crate::README_READ_LIMIT).read_to_end(&mut bytes))
            .is_ok()
        {
            return clip(&String::from_utf8_lossy(&bytes), README_CHARS);
        }
    }
    String::new()
}

fn docs(path: &str) -> Vec<String> {
    use std::io::Read;
    if path.trim().is_empty() {
        return Vec::new();
    }
    let dir = std::path::Path::new(path.trim()).join("docs");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".md") && !n.starts_with('.'))
        .collect();
    names.sort();
    names
        .into_iter()
        .take(DOC_FILES)
        .map(|name| {
            let mut bytes = Vec::new();
            let _ = std::fs::File::open(dir.join(&name))
                .and_then(|f| f.take(4096).read_to_end(&mut bytes));
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let first = text
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty() && *l != "---")
                .unwrap_or("")
                .trim_start_matches('#')
                .trim()
                .to_string();
            format!("docs/{name}: {}", clip(&first, DOC_CHARS))
        })
        .collect()
}

fn strings(conn: &Connection, sql: &str, p: &[&dyn rusqlite::ToSql]) -> Vec<String> {
    let Ok(mut st) = conn.prepare(sql) else {
        return Vec::new();
    };
    st.query_map(p, |r| r.get::<_, String>(0))
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
}

fn dismissed(conn: &Connection, slug: &str) -> Vec<(String, String)> {
    let Ok(mut st) = conn.prepare(
        "SELECT p.id, p.body FROM posts p JOIN decisions d ON d.post_id = p.id WHERE p.project = ? AND p.area = ? AND p.thread = ? AND d.answer = 'no' ORDER BY d.decided_at DESC, p.id DESC LIMIT 50",
    ) else {
        return Vec::new();
    };
    st.query_map(params![slug, AREA, AMEND_THREAD], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })
    .map(|rows| {
        rows.flatten()
            .filter_map(|(id, body)| parse_proposal(&body).map(|(text, _)| (id, text)))
            .collect()
    })
    .unwrap_or_default()
}

fn direction_sql(column: &str) -> String {
    let likes: Vec<String> = DIRECTION_WORDS
        .iter()
        .map(|w| format!("{column} LIKE '%{w}%'"))
        .collect();
    format!("({})", likes.join(" OR "))
}

/// Reads everything a draft rests on. The goal in force must read; the rest
/// is best effort - a missing README or an empty board gives less evidence,
/// not an error.
pub fn evidence(s: &hive_store::Store, slug: &str) -> Result<Evidence> {
    let goal = crate::get(s, slug)?;
    let conn = s.conn()?;
    let p = crate::project(&conn, slug)?;
    let mut readings: Vec<String> = Vec::new();
    if let Ok(r) = crate::review::review(s, slug) {
        let mut line = format!("store reading: {}", r.verdict);
        if r.scored {
            line.push_str(&format!(" ({} of 100)", r.score));
        }
        let kinds: Vec<&str> = r.advice.iter().map(|a| a.kind.as_str()).collect();
        if !kinds.is_empty() {
            line.push_str(&format!("; advice: {}", kinds.join(", ")));
        }
        readings.push(line);
    }
    for (summary, body) in strings(
        &conn,
        "SELECT summary || char(31) || body FROM posts WHERE project = ? AND area = ? AND thread = ? AND kind = 'finding' ORDER BY date DESC, id DESC LIMIT ?",
        &[&slug, &AREA, &crate::review::REVIEW_THREAD, &(READINGS as i64)],
    )
    .into_iter()
    .filter_map(|row| {
        row.split_once('\u{1f}')
            .map(|(a, b)| (a.to_string(), b.to_string()))
    }) {
        readings.push(format!(
            "lead reading: {} - {}",
            one_line(&summary),
            clip(&one_line(&body), 400)
        ));
    }
    let tasks = strings(
        &conn,
        "SELECT id || ' [' || status || '] ' || title FROM tasks WHERE project = ? ORDER BY updated_at DESC, id DESC LIMIT ?",
        &[&slug, &(TASKS as i64)],
    );
    let adrs = strings(
        &conn,
        "SELECT id || ' ' || title FROM records WHERE project = ? AND kind = 'adr' AND status = 'Accepted' ORDER BY number DESC, id DESC LIMIT ?",
        &[&slug, &(ADRS as i64)],
    );
    let mut owner: Vec<(String, String)> = Vec::new();
    let human = hive_store::HUMAN_AGENT;
    for row in strings(
        &conn,
        "SELECT r.at || char(31) || 'task ' || t.id || ' (' || t.title || '): ' || r.brief FROM task_revisions r JOIN tasks t ON t.id = r.task_id WHERE t.project = ? AND r.author = ? ORDER BY r.at DESC LIMIT ?",
        &[&slug, &human, &(OWNER_WORDS as i64)],
    )
    .into_iter()
    .chain(strings(
        &conn,
        "SELECT at || char(31) || 'message: ' || body FROM messages WHERE project = ? AND author = ? ORDER BY at DESC LIMIT ?",
        &[&slug, &human, &(OWNER_WORDS as i64)],
    ))
    .chain(strings(
        &conn,
        &format!(
            "SELECT m.at || char(31) || 'chat: ' || m.content FROM conversation_messages m JOIN conversations c ON c.id = m.conversation_id WHERE c.project = ? AND m.role = 'user' AND {} ORDER BY m.at DESC LIMIT ?",
            direction_sql("m.content")
        ),
        &[&slug, &(OWNER_WORDS as i64)],
    )) {
        if let Some((at, text)) = row.split_once('\u{1f}') {
            owner.push((at.to_string(), clip(&one_line(text), OWNER_WORD_CHARS)));
        }
    }
    owner.sort_by(|a, b| b.0.cmp(&a.0));
    owner.truncate(OWNER_WORDS);
    let dismissed = dismissed(&conn, slug)
        .into_iter()
        .take(DISMISSED_SHOWN)
        .map(|(_, text)| text)
        .collect();
    Ok(Evidence {
        project: slug.to_string(),
        name: p.name.clone(),
        goal,
        readme: readme(&p.path),
        docs: docs(&p.path),
        commits: crate::history::recent_subjects(&p.path, COMMITS),
        adrs,
        readings,
        tasks,
        owner: owner.into_iter().map(|(_, t)| t).collect(),
        dismissed,
    })
}

// ------------------------------------------------------------------ prompt

/// The system prompt of the drafting call.
pub const SYSTEM: &str = "You draft the goal of a software project for its owner. The goal is shared with every human and agent who works on the project: leads plan from it, workers read it in every brief, and the review scores the work against it. Write it from the evidence given, not from general advice.

A good goal:
- says who the project is for and what outcome they get, in the project's own terms;
- names the few directions that matter now and how progress would be seen (checks, measures, user-visible results);
- keeps the owner's own stated direction and the accepted decisions; it never contradicts them;
- is plain prose, 80 to 300 words, at most 4000 characters, with no headings;
- contains no file system paths of anyone's machine, no secrets, tokens, keys or e-mail addresses.

Answer in exactly this shape and nothing else:
GOAL:
<the goal>
WHY:
<one short paragraph: which evidence the goal rests on and what changed against the goal in force>";

fn section(out: &mut String, title: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    out.push_str(&format!("\n## {title}\n"));
    for item in items {
        out.push_str("- ");
        out.push_str(item);
        out.push('\n');
    }
}

/// The user prompt: the evidence, newest first in each part.
pub fn prompt(ev: &Evidence) -> String {
    let mut out = format!(
        "Project: {} ({})\n\n## The goal in force ({})\n{}\n",
        ev.name, ev.project, ev.goal.source, ev.goal.text
    );
    if !ev.readme.is_empty() {
        out.push_str(&format!("\n## README\n{}\n", ev.readme));
    }
    section(&mut out, "Documents", &ev.docs);
    section(&mut out, "Recent commits", &ev.commits);
    section(&mut out, "Accepted decisions (ADRs)", &ev.adrs);
    section(&mut out, "Goal readings", &ev.readings);
    section(&mut out, "Tasks", &ev.tasks);
    section(&mut out, "What the owner said", &ev.owner);
    section(
        &mut out,
        "Drafts the owner dismissed - do not repeat them",
        &ev.dismissed,
    );
    out.push_str("\nDraft the goal now.\n");
    out
}

// ------------------------------------------------------------------ reply

fn marker(line: &str, word: &str) -> Option<String> {
    let t = line.trim().trim_start_matches('#').trim();
    let t = t.trim_start_matches("**").trim_start_matches('*');
    let lower = t.to_lowercase();
    if !lower.starts_with(word) {
        return None;
    }
    let rest = t[word.len()..]
        .trim_start_matches("**")
        .trim_start_matches('*');
    let rest = rest.strip_prefix(':')?;
    Some(rest.trim_start_matches("**").trim().to_string())
}

/// The goal and the reason from the model's reply (`GOAL:` ... `WHY:` ...).
/// A reply without the `GOAL:` marker is taken whole as the goal up to a
/// `WHY:` line. Code fences are dropped.
pub fn parse_reply(reply: &str) -> (String, String) {
    let mut goal = String::new();
    let mut why = String::new();
    let mut in_why = false;
    for line in reply.lines() {
        if line.trim_start().starts_with("```") {
            continue;
        }
        if !in_why && let Some(rest) = marker(line, "why") {
            in_why = true;
            why.push_str(&rest);
            continue;
        }
        if !in_why && let Some(rest) = marker(line, "goal") {
            goal.clear();
            goal.push_str(&rest);
            continue;
        }
        let target = if in_why { &mut why } else { &mut goal };
        if !target.is_empty() {
            target.push('\n');
        }
        target.push_str(line.trim_end());
    }
    (goal.trim().to_string(), one_line(&why))
}

/// The proposal body for a goal and its reason: the shape
/// [`crate::review::parse_proposal`] reads.
pub fn proposal_body(text: &str, why: &str) -> String {
    format!(
        "{}\n{}\n\nWhy: {}\n",
        crate::review::AMEND_MARKER,
        text.trim(),
        why.trim()
    )
}

/// Turns the model's reply into a draft the owner may take: the goal must
/// pass [`crate::normalize`] (1-4000 characters, no secret-shaped words, no
/// private paths) and the store's own privacy rules, and must read back the
/// same from its proposal body.
pub fn check_reply(s: &hive_store::Store, reply: &str, trigger: &str) -> Result<Draft> {
    let (text, why) = parse_reply(reply);
    if text.is_empty() {
        return Err(anyhow!("the model's reply held no goal"));
    }
    let text = crate::normalize(&text)
        .map_err(|e| anyhow!("the drafted goal breaks the goal rules: {}", rule_words(&e)))?;
    crate::public_references(s, &text)
        .map_err(|_| anyhow!("the drafted goal names an owner-private path"))?;
    let why = if why.is_empty() {
        format!("drafted by a model from the project's evidence ({trigger})")
    } else {
        why
    };
    let why = crate::normalize(&why)
        .ok()
        .filter(|w| crate::public_references(s, w).is_ok())
        .unwrap_or_else(|| format!("drafted by a model from the project's evidence ({trigger})"));
    let body = proposal_body(&text, &why);
    let (read, read_why) = parse_proposal(&body)
        .ok_or_else(|| anyhow!("the drafted goal does not read back from its proposal"))?;
    Ok(Draft {
        text: read,
        why: read_why,
        body,
    })
}

fn rule_words(e: &anyhow::Error) -> String {
    match e.downcast_ref::<hive_cli::Error>() {
        Some(ce) if !ce.why.is_empty() => ce.why.clone(),
        _ => e.to_string(),
    }
}

// --------------------------------------------------------------- compare

fn words(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3)
        .map(|w| w.to_lowercase())
        .collect()
}

/// How much two goals share, 0.0..=1.0: the overlap of their word sets.
pub fn similarity(a: &str, b: &str) -> f64 {
    let (a, b) = (words(a), words(b));
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let shared = a.intersection(&b).count() as f64;
    let all = a.union(&b).count() as f64;
    shared / all
}

/// Whether two goals say the same thing: equal once whitespace and case
/// are set aside, or overlapping at least [`SAME_SHARE`].
pub fn same(a: &str, b: &str) -> bool {
    one_line(a).to_lowercase() == one_line(b).to_lowercase() || similarity(a, b) >= SAME_SHARE
}

/// Where `text` stands: the same as the goal in force, the same as the
/// proposal waiting for the owner, the same as a dismissed draft, or new.
pub fn standing(s: &hive_store::Store, slug: &str, text: &str) -> Result<Standing> {
    let goal = crate::get(s, slug)?;
    if same(&goal.text, text) {
        return Ok(Standing::SameAsGoal);
    }
    let conn = s.conn()?;
    if let Some(p) = crate::review::open_proposal(&conn, slug)
        && same(&p.text, text)
    {
        return Ok(Standing::Waiting(p.post));
    }
    if let Some((post, _)) = dismissed(&conn, slug)
        .into_iter()
        .find(|(_, old)| same(old, text))
    {
        return Ok(Standing::Dismissed(post));
    }
    Ok(Standing::New)
}

// ---------------------------------------------------------------- recheck

/// A project's last re-check, kept in `meta` under [`META_PREFIX`]`<slug>`
/// the way the update check keeps its own.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecheckState {
    /// When a draft last ran to an outcome.
    #[serde(default)]
    pub checked_at: String,
    #[serde(default)]
    pub trigger: String,
    #[serde(default)]
    pub outcome: String,
    #[serde(default)]
    pub post: String,
    /// The newest accepted ADR's update time the last check saw.
    #[serde(default)]
    pub adr_seen: String,
    /// The owner's newest direction the last check saw.
    #[serde(default)]
    pub owner_seen: String,
    /// The review flags (drift, stale, reached) the last check saw.
    #[serde(default)]
    pub flags: Vec<String>,
    /// The last failed attempt, cleared by the next outcome.
    #[serde(default)]
    pub failed_at: String,
    #[serde(default)]
    pub error: String,
}

pub fn meta_key(slug: &str) -> String {
    format!("{META_PREFIX}{slug}")
}

pub fn load_state(s: &hive_store::Store, slug: &str) -> Option<RecheckState> {
    match s.meta(&meta_key(slug)) {
        Ok((value, true)) => serde_json::from_str(&value).ok(),
        _ => None,
    }
}

pub fn save_state(s: &hive_store::Store, slug: &str, state: &RecheckState) -> Result<()> {
    s.set_meta(&meta_key(slug), &serde_json::to_string(state)?)
}

/// What the store says now about the things that call for a re-draft.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signals {
    /// The project's onboarding job has not finished yet.
    pub onboarding: bool,
    /// The newest accepted ADR's update time.
    pub adr_latest: String,
    /// The newest owner direction: a task the owner filed or amended, or a
    /// message or chat line of theirs that speaks of goal or direction.
    pub owner_latest: String,
    /// `drift`, `stale` (the review's advice) and `reached` (the review's
    /// `achieved` advice, or a fresh lead reading at [`REACHED_SCORE`] or
    /// more).
    pub flags: Vec<String>,
}

fn latest(conn: &Connection, sql: &str, p: &[&dyn rusqlite::ToSql]) -> String {
    conn.query_row(sql, p, |r| r.get::<_, Option<String>>(0))
        .ok()
        .flatten()
        .unwrap_or_default()
}

pub fn signals(s: &hive_store::Store, slug: &str) -> Signals {
    let Ok(conn) = s.conn() else {
        return Signals::default();
    };
    let human = hive_store::HUMAN_AGENT;
    let onboarding = conn
        .query_row(
            "SELECT COUNT(*) FROM jobs WHERE project = ? AND kind = 'onboard' AND status IN ('queued', 'leased')",
            [slug],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;
    let adr_latest = latest(
        &conn,
        "SELECT MAX(updated_at) FROM records WHERE project = ? AND kind = 'adr' AND status = 'Accepted'",
        &[&slug],
    );
    let owner_latest = [
        latest(
            &conn,
            "SELECT MAX(r.at) FROM task_revisions r JOIN tasks t ON t.id = r.task_id WHERE t.project = ? AND r.author = ?",
            &[&slug, &human],
        ),
        latest(
            &conn,
            &format!(
                "SELECT MAX(at) FROM messages WHERE project = ? AND author = ? AND {}",
                direction_sql("body")
            ),
            &[&slug, &human],
        ),
        latest(
            &conn,
            &format!(
                "SELECT MAX(m.at) FROM conversation_messages m JOIN conversations c ON c.id = m.conversation_id WHERE c.project = ? AND m.role = 'user' AND {}",
                direction_sql("m.content")
            ),
            &[&slug],
        ),
    ]
    .into_iter()
    .max()
    .unwrap_or_default();
    drop(conn);
    let mut flags = Vec::new();
    if let Ok(r) = crate::review::review(s, slug) {
        for kind in ["drift", "stale"] {
            if r.advice.iter().any(|a| a.kind == kind) {
                flags.push(kind.to_string());
            }
        }
        if r.advice.iter().any(|a| a.kind == "achieved")
            || r.lead
                .as_ref()
                .is_some_and(|l| l.fresh && l.score.is_some_and(|v| v >= REACHED_SCORE))
        {
            flags.push("reached".to_string());
        }
    }
    Signals {
        onboarding,
        adr_latest,
        owner_latest,
        flags,
    }
}

fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

/// Whether a project's last check is so recent (inside [`MIN_GAP_HOURS`], or
/// a failure inside [`RETRY_MINUTES`]) that no signal could make it due: the
/// daemon skips reading the signals then.
pub fn resting(state: Option<&RecheckState>, now: &str, days: i64) -> bool {
    let (Some(state), Some(now)) = (state, parse_time(now)) else {
        return false;
    };
    if let Some(failed) = parse_time(&state.failed_at)
        && now - failed < Duration::minutes(RETRY_MINUTES)
    {
        return true;
    }
    parse_time(&state.checked_at).is_some_and(|checked| {
        now - checked < Duration::hours(MIN_GAP_HOURS) && now - checked < Duration::days(days)
    })
}

/// Why the goal should be drafted again now, or None. `days` is
/// `goal.recheck_days`: 0 turns it off. A project never checked is drafted
/// once its onboarding is over (`first`); after that the cadence
/// (`cadence`), a newly accepted ADR (`adr`), the owner's direction
/// (`owner`) or a review flag the last check did not see (`drift`, `stale`,
/// `reached`) - the event triggers at most every [`MIN_GAP_HOURS`]. A failed
/// attempt waits [`RETRY_MINUTES`].
pub fn trigger(
    state: Option<&RecheckState>,
    sig: &Signals,
    now: &str,
    days: i64,
) -> Option<String> {
    if days <= 0 || sig.onboarding {
        return None;
    }
    let now = parse_time(now)?;
    let Some(state) = state else {
        return Some("first".into());
    };
    if let Some(failed) = parse_time(&state.failed_at)
        && now - failed < Duration::minutes(RETRY_MINUTES)
    {
        return None;
    }
    let Some(checked) = parse_time(&state.checked_at) else {
        return Some("first".into());
    };
    if now - checked >= Duration::days(days) {
        return Some("cadence".into());
    }
    if now - checked < Duration::hours(MIN_GAP_HOURS) {
        return None;
    }
    if sig.adr_latest > state.adr_seen {
        return Some("adr".into());
    }
    if sig.owner_latest > state.owner_seen {
        return Some("owner".into());
    }
    sig.flags.iter().find(|f| !state.flags.contains(f)).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-10-01T12:00:00Z";

    fn state(checked: &str) -> RecheckState {
        RecheckState {
            checked_at: checked.into(),
            adr_seen: "2026-09-01T00:00:00Z".into(),
            owner_seen: "2026-09-01T00:00:00Z".into(),
            ..Default::default()
        }
    }

    fn sig() -> Signals {
        Signals {
            adr_latest: "2026-09-01T00:00:00Z".into(),
            owner_latest: "2026-09-01T00:00:00Z".into(),
            ..Default::default()
        }
    }

    #[test]
    fn reply_parsing_takes_goal_and_why() {
        let (g, w) = parse_reply(
            "```\nGOAL:\nShip the web console to every team.\n\nMeasure it by sign-ins.\nWHY:\nThe README and ADR-070 say so.\n```",
        );
        assert_eq!(
            g,
            "Ship the web console to every team.\n\nMeasure it by sign-ins."
        );
        assert_eq!(w, "The README and ADR-070 say so.");
        let (g, w) = parse_reply("**Goal:** Make checkout fast.\n**Why:** shoppers asked.");
        assert_eq!(g, "Make checkout fast.");
        assert_eq!(w, "shoppers asked.");
        let (g, w) = parse_reply("Make checkout fast.");
        assert_eq!((g.as_str(), w.as_str()), ("Make checkout fast.", ""));
    }

    #[test]
    fn proposal_body_reads_back_with_its_paragraphs() {
        let body = proposal_body(
            "Ship the console to every team.\n\n- one\n- two",
            "the README says so",
        );
        let (text, why) = parse_proposal(&body).unwrap();
        assert_eq!(text, "Ship the console to every team.\n\n- one\n- two");
        assert_eq!(why, "the README says so");
    }

    #[test]
    fn similarity_tells_a_reworded_goal_from_a_new_one() {
        let a = "Make checkout finish in two taps for returning shoppers, measured by the checkout funnel.";
        assert!(same(a, &a.to_uppercase()));
        assert!(same(
            a,
            "Make checkout finish in two taps for returning shoppers; measured by the checkout funnel"
        ));
        assert!(!same(
            a,
            "Run many teams on one hosted brain with isolation, quotas and roles."
        ));
    }

    #[test]
    fn trigger_follows_onboarding_cadence_and_events() {
        let days = DEFAULT_RECHECK_DAYS;
        // Off, and not while onboarding runs.
        assert_eq!(trigger(None, &sig(), NOW, 0), None);
        let onboarding = Signals {
            onboarding: true,
            ..sig()
        };
        assert_eq!(trigger(None, &onboarding, NOW, days), None);
        // Never checked: the first draft.
        assert_eq!(trigger(None, &sig(), NOW, days).as_deref(), Some("first"));
        // Inside the cadence, nothing new: quiet.
        let st = state("2026-09-25T12:00:00Z");
        assert_eq!(trigger(Some(&st), &sig(), NOW, days), None);
        // The cadence is up.
        let old = state("2026-09-17T11:00:00Z");
        assert_eq!(
            trigger(Some(&old), &sig(), NOW, days).as_deref(),
            Some("cadence")
        );
        // A newly accepted ADR, the owner's direction, a new review flag.
        let adr = Signals {
            adr_latest: "2026-09-30T00:00:00Z".into(),
            ..sig()
        };
        assert_eq!(trigger(Some(&st), &adr, NOW, days).as_deref(), Some("adr"));
        let owner = Signals {
            owner_latest: "2026-09-30T00:00:00Z".into(),
            ..sig()
        };
        assert_eq!(
            trigger(Some(&st), &owner, NOW, days).as_deref(),
            Some("owner")
        );
        let drift = Signals {
            flags: vec!["drift".into()],
            ..sig()
        };
        assert_eq!(
            trigger(Some(&st), &drift, NOW, days).as_deref(),
            Some("drift")
        );
        // A flag the last check already saw does not fire again.
        let seen = RecheckState {
            flags: vec!["drift".into()],
            ..st.clone()
        };
        assert_eq!(trigger(Some(&seen), &drift, NOW, days), None);
        // Events wait out the gap after a check; a failure waits its retry.
        let recent = state("2026-10-01T09:00:00Z");
        assert_eq!(trigger(Some(&recent), &adr, NOW, days), None);
        let failed = RecheckState {
            failed_at: "2026-10-01T11:30:00Z".into(),
            ..old.clone()
        };
        assert_eq!(trigger(Some(&failed), &sig(), NOW, days), None);
        let failed_long_ago = RecheckState {
            failed_at: "2026-10-01T10:30:00Z".into(),
            ..old
        };
        assert_eq!(
            trigger(Some(&failed_long_ago), &sig(), NOW, days).as_deref(),
            Some("cadence")
        );
    }
}
