//! The decide role (ADR-072, first slice): on each daemon pass, every open
//! routine ask the lead may decide is put to `decide_model` on
//! `decide_provider` in one call with no tools. A yes or a no is decided
//! under the lead's name through `hive_board::decide_with`, so every gate of
//! a lead's decision still applies; a pass, an error, a late answer or one
//! that cannot be read leaves the ask in the lead's brief, as before the
//! role. While the role is on and no outage is open, the lead's brief leaves
//! out the asks the role still holds (`held_by_role`). In
//! `decide.mode = shadow` the answer is recorded and nothing is decided.
//!
//! The asks are the ones the lead decides today: `owner_addressed_asks`
//! (finding, handoff or suggestion marked for the owner, still open, not a
//! major dependency upgrade) whatever `owner.presence` says (TASK-129:
//! nothing waits on the owner but a hard stop), less the hard stops, the
//! owner-only proposals and anything that waits on a compute request.

use std::collections::BTreeMap;
use std::sync::LazyLock;
use std::time::Duration;

use serde_json::json;

/// Events that close an ask for the role: it was answered once and the
/// answer left it to the lead (pass), was only recorded (shadow), or
/// `hive_board::decide` refused it. The role does not ask about it again.
const ANSWERED_ACTIONS: &[&str] = &["decide.pass", "decide.shadow", "decide.refused"];
/// Meta keys of the current outage: its first reason, and how many
/// fallbacks it has counted since.
const OUTAGE_META: &str = "decide.outage";
const OUTAGE_COUNT_META: &str = "decide.outage_fallbacks";
/// One failed try on one ask: no answer, a late one or one that cannot be
/// read. The ask waits `RETRY_MINUTES` before it is sent again, so the asks
/// behind it reach the role first, and after `MAX_TRIES` it is the lead's
/// for good.
const FAILED_ACTION: &str = "decide.failed";
const RETRY_MINUTES: i64 = 60;
const MAX_TRIES: i64 = 3;
/// The role's reason is one line of at most this many bytes, cut at a
/// character boundary, plus "..." when cut: it goes into the decision, the
/// result post and the job brief.
const REASON_CHARS: usize = 300;
/// How much of a post body and of the thread the call carries.
const BODY_CHARS: usize = 4000;
const THREAD_CHARS: usize = 8000;

#[derive(Debug, Clone, Default)]
struct DecideRuns {
    inflight: bool,
    lines: Vec<String>,
    /// Passes in a row that panicked, and the daemon time before which no
    /// pass starts after them (`PANIC_BACKOFF_MINUTES`). A pass that ends
    /// normally resets both.
    panics: usize,
    retry_at: String,
}

/// The wait after the second, third and later panics of the pass in a row:
/// while the store refuses writes every pass calls the provider before its
/// first write panics, so each tick would cost one call for nothing. The
/// first panic waits for nothing: the next tick tries again.
const PANIC_BACKOFF_MINUTES: [i64; 3] = [1, 5, 15];

fn panic_backoff(panics: usize) -> Option<i64> {
    match panics {
        0 | 1 => None,
        n => Some(PANIC_BACKOFF_MINUTES[(n - 2).min(PANIC_BACKOFF_MINUTES.len() - 1)]),
    }
}

/// How long the provider's default model, once resolved for an empty
/// `decide_model`, is reused before the list is fetched again.
const MODEL_CACHE: Duration = Duration::from_secs(3600);

/// The resolved default model by store home and provider, with when it
/// was resolved: an empty `decide_model` fetched the provider's whole model
/// list on every pass that had asks.
type ResolvedModels = BTreeMap<(String, String), (String, std::time::Instant)>;
static DEFAULT_MODELS: LazyLock<parking_lot::Mutex<ResolvedModels>> =
    LazyLock::new(|| parking_lot::Mutex::new(ResolvedModels::new()));

static DECIDE_RUNS: LazyLock<parking_lot::Mutex<BTreeMap<String, DecideRuns>>> =
    LazyLock::new(|| parking_lot::Mutex::new(BTreeMap::new()));

/// Takes a pass's in-flight mark off when its thread ends, however it ends,
/// and hands over the lines the pass wrote. Without it a panic in the
/// handler of a panic left the mark set: `tend_decide` then started no pass
/// until the daemon restarted, no outage was on record, and `held_by_role`
/// kept the role's asks out of the lead's brief all that time.
struct PassEnd {
    /// `run_key` of the pass.
    key: String,
    now: String,
    lines: Vec<String>,
    panicked: bool,
}

impl Drop for PassEnd {
    fn drop(&mut self) {
        let mut runs = DECIDE_RUNS.lock();
        let run = runs.entry(std::mem::take(&mut self.key)).or_default();
        run.inflight = false;
        run.lines.append(&mut self.lines);
        if !self.panicked {
            run.panics = 0;
            run.retry_at.clear();
            return;
        }
        run.panics += 1;
        run.retry_at.clear();
        if let Some(wait) = panic_backoff(run.panics) {
            if let Ok(at) = crate::util::parse_iso(&self.now) {
                run.retry_at = crate::util::format_iso(at + chrono::Duration::minutes(wait));
            }
            run.lines.push(format!(
                "decide: the pass panicked {} times in a row; the next pass waits {wait} min",
                run.panics
            ));
        }
    }
}

/// One ask the role may answer.
#[derive(Debug, Clone, Default)]
pub(crate) struct DecideAsk {
    pub id: String,
    pub kind: String,
    pub area: String,
    pub thread: String,
    pub agent: String,
    pub summary: String,
    pub body: String,
}

/// What the model said, once read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DecideAnswer {
    pub answer: String,
    pub reason: String,
}

/// The daemon's hook: starts one decide pass for `slug` in the background
/// when the role is on (or in shadow) and an ask waits,
/// and returns the lines the last finished pass wrote. A pass may wait up to
/// `decide.timeout_seconds` per ask, so it never holds the daemon's loop.
#[cfg(test)]
pub(crate) fn tend_decide(s: &hive_store::Store, slug: &str, now: &str) -> Vec<String> {
    tend_decide_with(
        s,
        slug,
        now,
        &crate::lead::owner_addressed_asks(s, slug.to_string()),
    )
}

/// `tend_decide` over `questions`, the tick's `lead::owner_addressed_asks`
/// of `slug`: the daemon reads them once per tick and hands the same list
/// to the lead's pass (`lead::tend_lead_with`), and the pass started here
/// works from it too.
pub(crate) fn tend_decide_with(
    s: &hive_store::Store,
    slug: &str,
    now: &str,
    questions: &[crate::lead::OwnerQuestion],
) -> Vec<String> {
    let mut runs = DECIDE_RUNS.lock();
    let run = runs.entry(run_key(s, slug)).or_default();
    let mut lines = std::mem::take(&mut run.lines);
    if run.inflight || !role_due(s) || (!run.retry_at.is_empty() && now < run.retry_at.as_str()) {
        return lines;
    }
    let asks = role_asks(s, questions.to_vec(), now);
    if asks.is_empty() {
        return lines;
    }
    run.inflight = true;
    let (s, pass_slug, pass_now) = (s.clone(), slug.to_string(), now.to_string());
    let home = s.home.clone();
    let spawned = spawn_pass(&home, move || {
        let slug = pass_slug;
        let now = pass_now;
        // Dropped when this thread ends, by a return or by a panic.
        let mut end = PassEnd {
            key: run_key(&s, &slug),
            now: now.clone(),
            lines: Vec::new(),
            panicked: false,
        };
        // A panic must not leave the pass marked in flight, which would
        // keep the role off until the daemon restarts: it is an outage,
        // said once, and the next pass tries again.
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            decide_pass_over(&s, &slug, &now, asks)
        })) {
            Ok(lines) => end.lines = lines,
            Err(panic) => {
                end.panicked = true;
                let reason = format!("the decide pass panicked: {}", panic_text(&panic));
                // The line is kept before the outage is written: a store
                // that refuses writes is what makes a pass panic, and then
                // this write fails too. `end` still says the line and takes
                // the mark off.
                end.lines = vec![format!(
                    "decide {slug}: {reason}; the next pass tries again"
                )];
                let by = role_by(&s);
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    fallback(&s, &now, &slug, "", &by, &reason)
                }));
                // Only an outage on record gives the asks back to the lead
                // (`held_by_role`); without one the next pass tries again.
                if outage_open(&s) {
                    end.lines = vec![format!(
                        "decide {slug}: {reason}; the asks stay with the lead"
                    )];
                }
            }
        }
    });
    if let Err(e) = spawned {
        // No thread, no pass: the mark comes off and the next tick tries.
        run.inflight = false;
        lines.push(format!(
            "decide {slug}: the pass could not start ({e}); the next pass tries again"
        ));
    }
    lines
}

/// Starts the pass thread. Tests make it fail for one store home.
fn spawn_pass(
    home: &str,
    f: impl FnOnce() + Send + 'static,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    #[cfg(test)]
    if tests::SPAWN_FAILS.lock().unwrap().contains(home) {
        return Err(std::io::Error::other("no thread for the test"));
    }
    let _ = home;
    std::thread::Builder::new()
        .name("hive-decide".into())
        .spawn(f)
}

/// The key of a project's pass state: one store's project, so the state of
/// one hub never reaches another's (tests open many stores in one process).
fn run_key(s: &hive_store::Store, slug: &str) -> String {
    format!("{}\n{slug}", s.home)
}

fn outage_open(s: &hive_store::Store) -> bool {
    !s.meta(OUTAGE_META)
        .map(|(v, _)| v)
        .unwrap_or_default()
        .is_empty()
}

/// Whether the role runs at all now: `decide.mode` is not off. The lead
/// decides the owner's routine asks whatever `owner.presence` says
/// (TASK-129), so the role does too.
fn role_due(s: &hive_store::Store) -> bool {
    s.decide_mode() != "off"
}

fn panic_text(panic: &Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".into())
}

/// The role's model and provider as the settings name them.
fn role_by(s: &hive_store::Store) -> hive_board::DecidedBy {
    hive_board::DecidedBy {
        model: s.decide_model(),
        provider: s.decide_provider(),
        role: true,
    }
}

/// The asks of `slug` the lead's pass leaves to the role: while
/// `decide.mode` is on, the lead is active and no outage is open, the ones
/// the role still lists (`role_asks`), whatever `owner.presence` says: the
/// role runs with the owner at hand too (TASK-129, `role_due`), so the same
/// asks are held then. An ask the role passed, failed on, or may not answer
/// (a hard stop among them) stays in the lead's brief; in shadow the lead
/// decides them all. Without this every ask cost one small call and a lead
/// run besides.
pub(crate) fn held_by_role(
    s: &hive_store::Store,
    slug: &str,
    questions: &[crate::lead::OwnerQuestion],
    now: &str,
) -> std::collections::BTreeSet<String> {
    if questions.is_empty()
        || s.decide_mode() != "on"
        || !s
            .meta(OUTAGE_META)
            .map(|(v, _)| v)
            .unwrap_or_default()
            .is_empty()
    {
        return Default::default();
    }
    match s.agent(&crate::lead::lead_name(slug.to_string())) {
        Ok(Some(a)) if a.kind == "lead" && a.status == "active" => {}
        _ => return Default::default(),
    }
    role_asks(s, questions.to_vec(), now)
        .into_iter()
        .map(|a| a.id)
        .collect()
}

/// The asks of `slug` the role may answer now, oldest first.
#[cfg(test)]
pub(crate) fn decide_asks(s: &hive_store::Store, slug: &str, now: &str) -> Vec<DecideAsk> {
    role_asks(
        s,
        crate::lead::owner_addressed_asks(s, slug.to_string()),
        now,
    )
}

/// Of `questions` (the lead's `owner_addressed_asks`), the ones the role
/// may answer at `now`: not owner-only, no pair or onboard marker, no
/// compute request, not answered by the role already, and not failed
/// `MAX_TRIES` times or within the last `RETRY_MINUTES`.
fn role_asks(
    s: &hive_store::Store,
    questions: Vec<crate::lead::OwnerQuestion>,
    now: &str,
) -> Vec<DecideAsk> {
    if questions.is_empty() {
        return Vec::new();
    }
    let Ok(conn) = s.conn() else {
        return Vec::new();
    };
    let marks = vec!["?"; ANSWERED_ACTIONS.len()].join(", ");
    let answered_sql =
        format!("SELECT 1 AS x FROM events WHERE action IN ({marks}) AND subject = ? LIMIT 1");
    let retry_since = crate::util::parse_iso(now)
        .map(|t| crate::util::format_iso(t - chrono::Duration::minutes(RETRY_MINUTES)))
        .unwrap_or_default();
    questions
        .into_iter()
        .filter_map(|q| {
            // A hard stop is the owner's alone (TASK-129): never the role's.
            if !q.hard_stop.is_empty() {
                return None;
            }
            let post =
                crate::util::must_one(&conn, "SELECT body FROM posts WHERE id = ?", &[&q.id]);
            let ask = DecideAsk {
                body: post.str("body"),
                id: q.id,
                kind: q.kind,
                area: q.area,
                thread: q.thread,
                agent: q.agent,
                summary: q.summary,
            };
            if !hive_board::owner_only_proposal(&ask.summary, &ask.body).is_empty()
                || crate::lead::LEAD_HUMAN_ONLY_MARKER_RE.is_match(&ask.body)
            {
                return None;
            }
            // Compute is money: never put to the role.
            if !crate::util::must_one(
                &conn,
                "SELECT 1 AS x FROM compute_requests WHERE proposal_post = ?",
                &[&ask.id],
            )
            .0
            .is_empty()
            {
                return None;
            }
            let mut args: Vec<&dyn rusqlite::ToSql> = ANSWERED_ACTIONS
                .iter()
                .map(|a| a as &dyn rusqlite::ToSql)
                .collect();
            args.push(&ask.id);
            if !crate::util::must_one(&conn, &answered_sql, &args)
                .0
                .is_empty()
            {
                return None;
            }
            let failed = crate::util::must_one(
                &conn,
                "SELECT COUNT(*) AS n, COALESCE(MAX(at), '') AS last FROM events WHERE subject = ? AND action = ?",
                &[&ask.id, &FAILED_ACTION],
            );
            if i64::from(failed.int("n")) >= MAX_TRIES
                || (!failed.str("last").is_empty()
                    && failed.str("last").as_str() > retry_since.as_str())
            {
                return None;
            }
            Some(ask)
        })
        .collect()
}

/// One decide pass over the open asks of `slug`, in the caller's thread.
/// Returns one line per ask it touched.
///
/// The mode is read again before each ask and after its answer, so a mode
/// switched off or to shadow mid-pass stops the pass before it records or
/// decides anything more. `owner.presence` plays no part (TASK-129): an
/// owner who returns mid-pass changes nothing, the routine asks are the
/// role's either way. Right before a write the ask is checked again as the
/// lead's own path checks it: still open, and not a major security upgrade.
///
/// A call that fails (no answer, a late or an unreadable one) records one
/// `decide.failed` try on that ask, which then waits `RETRY_MINUTES` and is
/// the lead's for good after `MAX_TRIES`, so the asks behind it reach the
/// role. A provider error or a timeout ends the pass and opens the
/// store-wide outage (`decide.fallback`): the provider is out, each further
/// call would only wait again, and every project's asks go to its lead
/// until an answer comes back. An unreadable answer is that ask's alone: it
/// is a failed try, the pass goes on and no outage opens.
#[cfg(test)]
pub(crate) fn decide_pass(s: &hive_store::Store, slug: &str, now: &str) -> Vec<String> {
    decide_pass_over(s, slug, now, decide_asks(s, slug, now))
}

/// `decide_pass` over `asks`, the role's asks of `slug` as the tick read
/// them (`tend_decide_with`).
fn decide_pass_over(
    s: &hive_store::Store,
    slug: &str,
    now: &str,
    asks: Vec<DecideAsk>,
) -> Vec<String> {
    if !role_due(s) {
        return Vec::new();
    }
    let lead = crate::lead::lead_name(slug.to_string());
    match s.agent(&lead) {
        Ok(Some(a)) if a.kind == "lead" && a.status == "active" => {}
        _ => return Vec::new(),
    }
    if asks.is_empty() {
        return Vec::new();
    }
    let mut by = role_by(s);
    let timeout = Duration::from_secs(
        s.setting("decide.timeout_seconds", "30")
            .trim()
            .parse::<u64>()
            .unwrap_or(30)
            .max(1),
    );
    let mut lines = Vec::new();
    let provider: std::sync::Arc<dyn hive_provider::Provider> = match open_role(s, &mut by, timeout)
    {
        Ok(p) => std::sync::Arc::from(p),
        Err(e) => {
            let reason = crate::run::err_message(&e);
            fallback(s, now, slug, &asks[0].id, &by, &reason);
            lines.push(format!(
                "decide {slug}: the role cannot run, the asks stay with the lead: {reason}"
            ));
            return lines;
        }
    };
    for ask in asks {
        if !role_due(s) {
            lines.push(format!("decide {slug}: stopped: decide.mode is off"));
            break;
        }
        let prompt = decide_prompt(s, &ask);
        let called = call_role(&provider, &by, prompt, timeout);
        let out = called.is_err();
        let answer = called.and_then(|text| {
            read_answer(&text)
                .ok_or_else(|| anyhow::anyhow!("unreadable answer: {}", clip(text.trim(), 200)))
        });
        let answer = match answer {
            Ok(a) => a,
            Err(e) => {
                let reason = crate::run::err_message(&e);
                failed(s, now, slug, &ask.id, &by, &reason);
                lines.push(format!(
                    "decide {slug}: {} left to the lead: {reason}",
                    ask.id
                ));
                // Only a provider that is out opens the outage; an
                // unreadable answer is this ask's failed try alone.
                if out {
                    fallback(s, now, slug, &ask.id, &by, &reason);
                    break;
                }
                continue;
            }
        };
        recovered(s, now, &by);
        // The answer may have taken `timeout`: read the mode and the ask
        // again before anything is recorded.
        if !role_due(s) {
            lines.push(format!(
                "decide {slug}: {} not recorded: decide.mode is off",
                ask.id
            ));
            break;
        }
        let mode = s.decide_mode();
        if let Some(why) = gone(s, &ask.id) {
            lines.push(format!("decide {slug}: {} not recorded: {why}", ask.id));
            continue;
        }
        let detail = json!({
            "project": slug,
            "answer": answer.answer,
            "reason": answer.reason,
            "decided_model": by.model,
            "decided_provider": by.provider,
        });
        if mode == "shadow" {
            event(s, now, &lead, "decide.shadow", &ask.id, detail);
            lines.push(format!(
                "decide {slug}: {} would be {} (shadow): {}",
                ask.id, answer.answer, answer.reason
            ));
            continue;
        }
        if answer.answer == "pass" {
            event(s, now, &lead, "decide.pass", &ask.id, detail);
            lines.push(format!(
                "decide {slug}: {} passed to the lead: {}",
                ask.id, answer.reason
            ));
            continue;
        }
        match hive_board::decide_with(
            s,
            &ask.id,
            &answer.answer,
            &answer.reason,
            &lead,
            by.clone(),
        ) {
            Ok(d) => lines.push(format!(
                "decide {slug}: {} {} by {lead} with {} on {}: {}",
                d.post_id,
                d.answer,
                if d.decided_model.is_empty() {
                    "the default model"
                } else {
                    &d.decided_model
                },
                d.decided_provider,
                answer.reason
            )),
            Err(e) => {
                let why = crate::run::err_message(&e);
                // A refusal because the role was turned off, or the mode
                // moved, between the check and the write is not the ask's:
                // it is not recorded, so the role may answer it on a later
                // pass.
                if !role_due(s) || s.decide_mode() != "on" {
                    lines.push(format!("decide {slug}: {} not decided: {why}", ask.id));
                    break;
                }
                let mut detail = detail;
                detail["refused"] = json!(why);
                event(s, now, &lead, "decide.refused", &ask.id, detail);
                lines.push(format!("decide {slug}: {} refused: {why}", ask.id));
            }
        }
    }
    lines
}

/// Why the ask may no longer be written, checked right before the write as
/// the lead's own path checks it (`run::apply_result_decisions`): it was
/// answered, decided or its thread closed, or it is a major security
/// upgrade. `None` while it still waits.
fn gone(s: &hive_store::Store, post: &str) -> Option<String> {
    let Ok(conn) = s.conn() else {
        return Some("the store cannot be read".into());
    };
    if crate::lead::is_security_major_finding(&conn, post) {
        return Some(
            "it is a major dependency upgrade: the owner's call, or dependencies.major_updates's"
                .into(),
        );
    }
    if !crate::lead::owner_ask_is_open(&conn, post) {
        return Some("it was answered or its thread closed meanwhile".into());
    }
    None
}

/// One failed try on `post`: counted per ask so it is not sent again on
/// every pass (`role_asks`).
fn failed(
    s: &hive_store::Store,
    now: &str,
    slug: &str,
    post: &str,
    by: &hive_board::DecidedBy,
    reason: &str,
) {
    event(
        s,
        now,
        "daemon",
        FAILED_ACTION,
        post,
        json!({
            "project": slug,
            "reason": reason,
            "decided_model": by.model,
            "decided_provider": by.provider,
        }),
    );
}

fn event(
    s: &hive_store::Store,
    now: &str,
    actor: &str,
    action: &str,
    subject: &str,
    detail: serde_json::Value,
) {
    if let Ok(conn) = s.conn() {
        crate::util::event_at(&conn, now, actor, action, subject, detail);
    }
}

/// Opens the role's provider. An empty model (no `decide_model`,
/// `measure_model`, `light_model` nor `worker_model`: the shipped default)
/// is resolved here to the provider's default chat model, and `by.model`
/// names it, so the call never sends an empty model and the decision
/// records the model that answered. The resolved name is kept for
/// `MODEL_CACHE` (an hour): the list is not fetched on every pass.
fn open_role(
    s: &hive_store::Store,
    by: &mut hive_board::DecidedBy,
    timeout: Duration,
) -> anyhow::Result<Box<dyn hive_provider::Provider>> {
    let p = hive_provider::open(
        &by.provider,
        hive_provider::Options {
            home: s.home.clone(),
            settings: s.provider_settings(),
            ..Default::default()
        },
    )?;
    if by.model.trim().is_empty() {
        let key = (s.home.clone(), by.provider.clone());
        if let Some((model, at)) = DEFAULT_MODELS.lock().get(&key)
            && at.elapsed() < MODEL_CACHE
        {
            by.model = model.clone();
            return Ok(p);
        }
        let cancel = hive_provider::CancelToken::new().with_timeout(timeout);
        let models = p.models(&cancel)?;
        let chosen = models
            .iter()
            .find(|m| m.default && m.chat)
            .or_else(|| models.iter().find(|m| m.chat))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no decide model: decide_model is empty and {} lists no chat model",
                    by.provider
                )
            })?;
        by.model = chosen.id.clone();
        DEFAULT_MODELS
            .lock()
            .insert(key, (by.model.clone(), std::time::Instant::now()));
    }
    Ok(p)
}

/// The one call: the post, its thread and the rubric, no tools. The answer
/// must come before `timeout`; the provider is also handed the deadline, so
/// one that reads it stops on its own.
fn call_role(
    p: &std::sync::Arc<dyn hive_provider::Provider>,
    by: &hive_board::DecidedBy,
    prompt: String,
    timeout: Duration,
) -> anyhow::Result<String> {
    let cancel = hive_provider::CancelToken::new().with_timeout(timeout);
    let req = hive_provider::Request {
        model: by.model.clone(),
        messages: vec![hive_provider::Message {
            role: hive_provider::Role::USER,
            content: prompt,
            ..Default::default()
        }],
        tools: vec![],
        initiator: "user".into(),
        ..Default::default()
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let token = cancel.clone();
    let p = p.clone();
    std::thread::spawn(move || {
        let _ = tx.send(p.chat(&token, req));
    });
    match rx.recv_timeout(timeout) {
        Ok(r) => Ok(r?
            .message
            .map(|m| m.content)
            .unwrap_or_default()
            .trim()
            .to_string()),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            cancel.cancel();
            anyhow::bail!("no answer within {}s", timeout.as_secs())
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            anyhow::bail!("the provider's call ended without an answer")
        }
    }
}

/// The prompt of one ask: what to answer and how, the rubric of the lead's
/// brief, the post and its thread so far.
pub(crate) fn decide_prompt(s: &hive_store::Store, ask: &DecideAsk) -> String {
    let (yes, no) = crate::lead::ask_meanings(&ask.body);
    let mut b = String::new();
    b.push_str("You decide a routine ask addressed to the owner of a software project, as the project's lead would: nothing waits on the owner but a hard stop. Answer yes, no or pass.\n\n");
    b.push_str("Rubric: ");
    b.push_str(crate::lead::ROUTINE_ASK_RUBRIC);
    b.push_str("\nAnswer pass when the post does not give you enough to decide; the lead then reads it in full.\n\n");
    b.push_str("Reply with one JSON object and nothing else: {\"answer\":\"yes\"|\"no\"|\"pass\",\"reason\":\"one sentence\"}\n\n");
    b.push_str(&format!(
        "## The ask\n\n{} {} on {}/{} by {}\nSummary: {}\n",
        ask.kind, ask.id, ask.area, ask.thread, ask.agent, ask.summary
    ));
    b.push_str(&format!(
        "Yes means: {}\nNo means: {}\n\n",
        if yes.is_empty() { "act on it" } else { &yes },
        if no.is_empty() { "close it" } else { &no }
    ));
    b.push_str(&clip(&ask.body, BODY_CHARS));
    b.push_str("\n\n## The thread so far\n\n");
    let thread = s
        .conn()
        .map(|c| {
            crate::util::must_rows(
                &c,
                "SELECT id, kind, agent, date, summary, body FROM posts \
                 WHERE project = (SELECT project FROM posts WHERE id = ?) AND area = ? AND thread = ? AND id <> ? \
                 ORDER BY date, rowid",
                &[&ask.id, &ask.area, &ask.thread, &ask.id],
            )
        })
        .unwrap_or_default();
    let mut said = String::new();
    for p in thread.iter().rev() {
        let entry = format!(
            "- {} {} by {} at {}: {}\n{}\n",
            p.str("kind"),
            p.str("id"),
            p.str("agent"),
            p.str("date"),
            p.str("summary"),
            clip(&p.str("body"), 1000)
        );
        if said.len() + entry.len() > THREAD_CHARS {
            break;
        }
        said.insert_str(0, &entry);
    }
    if said.is_empty() {
        b.push_str("(no other posts)\n");
    } else {
        b.push_str(&said);
    }
    b
}

fn clip(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &text[..end])
}

/// Reads `{"answer":"yes"|"no"|"pass","reason":"..."}` out of the reply; a
/// code fence around it is allowed. The reason becomes one line (its
/// whitespace runs folded to one space) of at most `REASON_CHARS` bytes,
/// cut at a character boundary, plus "..." when cut: it goes
/// into the decision, the result post and the job brief. Anything else, an
/// answer outside the three, or a yes or no without a reason, cannot be
/// read.
pub(crate) fn read_answer(text: &str) -> Option<DecideAnswer> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end < start {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(&text[start..=end]).ok()?;
    let answer = v.get("answer")?.as_str()?.trim().to_ascii_lowercase();
    let reason = v
        .get("reason")
        .and_then(|r| r.as_str())
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let reason = clip(&reason, REASON_CHARS);
    match answer.as_str() {
        "yes" | "no" if !reason.is_empty() => Some(DecideAnswer { answer, reason }),
        "pass" => Some(DecideAnswer { answer, reason }),
        _ => None,
    }
}

/// A fallback: the first of an outage writes one `decide.fallback` event
/// with its reason; later ones are only counted. An outage opened while
/// the events table refused the insert has its meta and no event: the next
/// fallback writes the event then.
fn fallback(
    s: &hive_store::Store,
    now: &str,
    slug: &str,
    post: &str,
    by: &hive_board::DecidedBy,
    reason: &str,
) {
    let open = s.meta(OUTAGE_META).map(|(v, _)| v).unwrap_or_default();
    let count = s
        .meta(OUTAGE_COUNT_META)
        .map(|(v, _)| v.parse::<i64>().unwrap_or(0))
        .unwrap_or(0);
    let _ = s.set_meta(OUTAGE_COUNT_META, &(count + 1).to_string());
    if !open.is_empty() && fallback_said(s) {
        return;
    }
    let reason = if open.is_empty() {
        let _ = s.set_meta(OUTAGE_META, reason);
        reason.to_string()
    } else {
        open
    };
    if let Ok(conn) = s.conn() {
        let _ = conn.execute(
            "INSERT INTO events (at, actor, action, subject, detail) VALUES (?, ?, ?, ?, ?)",
            rusqlite::params![
                now,
                "daemon",
                "decide.fallback",
                post,
                crate::util::py_json(json!({
                    "project": slug,
                    "reason": reason,
                    "decided_model": by.model,
                    "decided_provider": by.provider,
                })),
            ],
        );
    }
}

/// Whether the open outage has its `decide.fallback` event: one written
/// after the last `decide.recovered`.
fn fallback_said(s: &hive_store::Store) -> bool {
    let Ok(conn) = s.conn() else {
        return true;
    };
    conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) > COALESCE((SELECT MAX(seq) FROM events WHERE action = 'decide.recovered'), 0) \
         FROM events WHERE action = 'decide.fallback'",
        [],
        |r| r.get::<_, bool>(0),
    )
    .unwrap_or(true)
}

/// A readable answer after an outage writes one `decide.recovered` event
/// with the count of fallbacks the outage had.
fn recovered(s: &hive_store::Store, now: &str, by: &hive_board::DecidedBy) {
    let open = s.meta(OUTAGE_META).map(|(v, _)| v).unwrap_or_default();
    if open.is_empty() {
        return;
    }
    let count = s
        .meta(OUTAGE_COUNT_META)
        .map(|(v, _)| v.parse::<i64>().unwrap_or(0))
        .unwrap_or(0);
    let _ = s.set_meta(OUTAGE_META, "");
    let _ = s.set_meta(OUTAGE_COUNT_META, "0");
    if let Ok(conn) = s.conn() {
        crate::util::event_at(
            &conn,
            now,
            "daemon",
            "decide.recovered",
            "",
            json!({
                "reason": open,
                "fallbacks": count,
                "decided_model": by.model,
                "decided_provider": by.provider,
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::*;
    use std::sync::Mutex;

    /// What the fake provider of a store (by home) does on each call.
    #[derive(Debug, Clone, Default)]
    struct Fake {
        reply: String,
        err: String,
        stall_ms: u64,
        calls: usize,
        models: Vec<String>,
        prompts: Vec<String>,
        tools: usize,
        /// Replies by a word of the prompt, before `reply`.
        replies: Vec<(String, String)>,
        /// The provider's model list; `models` panics when `panic` is set.
        list: Vec<String>,
        panic: bool,
        /// How often the model list was asked for.
        list_calls: usize,
    }

    /// Store homes whose pass thread cannot be started (`spawn_pass`).
    pub(super) static SPAWN_FAILS: LazyLock<Mutex<std::collections::BTreeSet<String>>> =
        LazyLock::new(|| Mutex::new(std::collections::BTreeSet::new()));

    /// What happens in the store while a call of the store's fake waits:
    /// the world moving on between the list and the write.
    type During = std::sync::Arc<dyn Fn() + Send + Sync>;
    static DURING: LazyLock<Mutex<BTreeMap<String, During>>> =
        LazyLock::new(|| Mutex::new(BTreeMap::new()));

    fn during(s: &hive_store::Store, f: impl Fn() + Send + Sync + 'static) {
        DURING
            .lock()
            .unwrap()
            .insert(s.home.clone(), std::sync::Arc::new(f));
    }

    static FAKES: LazyLock<Mutex<BTreeMap<String, Fake>>> =
        LazyLock::new(|| Mutex::new(BTreeMap::new()));
    static FAKE_ONCE: std::sync::Once = std::sync::Once::new();

    struct DecideFake {
        home: String,
    }

    impl hive_provider::Provider for DecideFake {
        fn name(&self) -> &str {
            "decidefake"
        }
        fn label(&self) -> &str {
            "Decide Fake"
        }
        fn chat(
            &self,
            _cancel: &hive_provider::CancelToken,
            req: hive_provider::Request,
        ) -> anyhow::Result<hive_provider::Response> {
            let f = {
                let mut fakes = FAKES.lock().unwrap();
                let f = fakes.entry(self.home.clone()).or_default();
                f.calls += 1;
                f.models.push(req.model.clone());
                f.tools += req.tools.len();
                f.prompts.push(
                    req.messages
                        .first()
                        .map(|m| m.content.clone())
                        .unwrap_or_default(),
                );
                f.clone()
            };
            if f.stall_ms > 0 {
                std::thread::sleep(Duration::from_millis(f.stall_ms));
            }
            let hook = DURING.lock().unwrap().remove(&self.home);
            if let Some(hook) = hook {
                hook();
            }
            if !f.err.is_empty() {
                anyhow::bail!("{}", f.err);
            }
            let prompt = f.prompts.last().cloned().unwrap_or_default();
            let reply = f
                .replies
                .iter()
                .find(|(word, _)| prompt.contains(word.as_str()))
                .map(|(_, r)| r.clone())
                .unwrap_or_else(|| f.reply.clone());
            Ok(hive_provider::Response {
                message: Some(hive_provider::Message {
                    role: hive_provider::Role::ASSISTANT,
                    content: reply,
                    ..Default::default()
                }),
                finish_reason: "stop".into(),
                ..Default::default()
            })
        }
        fn models(
            &self,
            _cancel: &hive_provider::CancelToken,
        ) -> anyhow::Result<Vec<hive_provider::Model>> {
            let f = {
                let mut fakes = FAKES.lock().unwrap();
                let f = fakes.entry(self.home.clone()).or_default();
                f.list_calls += 1;
                f.clone()
            };
            if f.panic {
                panic!("the fake's model list broke");
            }
            Ok(f.list
                .iter()
                .enumerate()
                .map(|(i, id)| hive_provider::Model {
                    id: id.clone(),
                    name: id.clone(),
                    vendor: "decidefake".into(),
                    chat: true,
                    tool_calls: false,
                    vision: false,
                    premium: 0.0,
                    preview: false,
                    context_window: 0,
                    max_output: 0,
                    default: i == 1,
                })
                .collect())
        }
        fn status(
            &self,
            _cancel: &hive_provider::CancelToken,
        ) -> anyhow::Result<hive_provider::Login> {
            Ok(hive_provider::Login {
                logged_in: true,
                ..Default::default()
            })
        }
    }

    fn set(s: &hive_store::Store, key: &str, value: &str) {
        s.set_setting(crate::util::HUMAN_AGENT, key, value)
            .unwrap_or_else(|e| panic!("{key}: {e}"));
    }

    /// A store with project `proj`, its lead, an away owner, the role on
    /// `decide-small` over the fake provider, and the fake answering `reply`.
    fn decide_store(reply: &str) -> TestStore {
        FAKE_ONCE.call_once(|| {
            hive_provider::register("decidefake", "Decide Fake", |o| {
                Ok(Box::new(DecideFake { home: o.home }))
            });
        });
        let s = new_store();
        new_bare_repo(&s);
        join(&s, "aurora");
        set(&s, "owner.presence", hive_store::OWNER_AWAY);
        crate::lead::ensure_lead(&s, "proj".into()).unwrap();
        // The catalogue refuses a provider name it does not know; the fake
        // goes in directly.
        s.conn()
            .unwrap()
            .execute(
                "INSERT INTO settings(name, value, updated_at, updated_by) VALUES ('decide_provider', 'decidefake', ?, 'test') \
                 ON CONFLICT(name) DO UPDATE SET value = excluded.value",
                rusqlite::params![TEST_NOW],
            )
            .unwrap();
        set(&s, "decide_model", "decide-small");
        set(&s, "decide.mode", "on");
        set(&s, "decide.timeout_seconds", "1");
        fake(&s, |f| f.reply = reply.into());
        s
    }

    fn fake(s: &hive_store::Store, edit: impl FnOnce(&mut Fake)) {
        let mut fakes = FAKES.lock().unwrap();
        let f = fakes.entry(s.home.clone()).or_default();
        edit(f);
    }

    fn calls(s: &hive_store::Store) -> Fake {
        FAKES
            .lock()
            .unwrap()
            .get(&s.home)
            .cloned()
            .unwrap_or_default()
    }

    fn ask(s: &hive_store::Store, thread: &str) -> String {
        crate::board::create_post(
            s,
            TEST_NOW.into(),
            crate::board::PostOpts {
                kind: "suggestion".into(),
                project: "proj".into(),
                area: "build".into(),
                thread: thread.into(),
                summary: format!("Cache the {thread} step between runs"),
                body: "The step runs twice per job.\n\nYes: Queue the cache work\nNo: Close it\n"
                    .into(),
                agent: crate::board::actor_agent(s, "daemon".into()),
                for_owner: true,
                ..Default::default()
            },
        )
        .unwrap()
        .str("id")
    }

    fn decision(s: &hive_store::Store, post: &str) -> crate::util::Row {
        crate::util::must_one(
            &s.conn().unwrap(),
            "SELECT answer, words, decided_by, decided_model, decided_provider FROM decisions WHERE post_id = ?",
            &[&post],
        )
    }

    fn events(s: &hive_store::Store, action: &str) -> Vec<crate::util::Row> {
        crate::util::must_rows(
            &s.conn().unwrap(),
            "SELECT actor, subject, detail FROM events WHERE action = ? ORDER BY seq",
            &[&action],
        )
    }

    fn detail(r: &crate::util::Row) -> serde_json::Value {
        serde_json::from_str(&r.str("detail")).unwrap()
    }

    fn assert_decided(s: &hive_store::Store, post: &str, answer: &str) {
        let d = decision(s, post);
        assert_eq!(d.str("answer"), answer, "{d:?}");
        assert_eq!(d.str("decided_by"), "proj-lead");
        assert_eq!(d.str("decided_model"), "decide-small");
        assert_eq!(d.str("decided_provider"), "decidefake");
        assert!(!d.str("words").is_empty(), "the reason is the words");
        let ev = events(s, "decide");
        let ev = ev
            .iter()
            .find(|e| e.str("subject") == post)
            .expect("decide event");
        assert_eq!(ev.str("actor"), "proj-lead");
        let v = detail(ev);
        assert_eq!(v["answer"], answer);
        assert_eq!(v["decided_model"], "decide-small");
        assert_eq!(v["decided_provider"], "decidefake");
        assert_eq!(v["via"], "decide-role");
        assert_eq!(v["on_behalf"], "owner-routine");
    }

    fn assert_left_to_lead(s: &hive_store::Store, post: &str) {
        assert!(is_nil_row(&decision(s, post)), "no decision on {post}");
        assert!(
            crate::lead::owner_addressed_asks(s, "proj".into())
                .iter()
                .any(|q| q.id == post),
            "{post} stays in the lead's brief"
        );
    }

    fn is_nil_row(r: &crate::util::Row) -> bool {
        crate::util::is_nil(r)
    }

    #[test]
    fn a_yes_is_decided_under_the_lead_with_the_role_model_and_provider() {
        let _serial = serial();
        let s = decide_store(r#"{"answer":"yes","reason":"Tier 1 work inside the checks"}"#);
        let post = ask(&s, "yes-ask");
        let lines = decide_pass(&s, "proj", TEST_NOW);
        assert_decided(&s, &post, "yes");
        assert!(
            lines
                .iter()
                .any(|l| l.contains("decide-small on decidefake")),
            "{lines:?}"
        );
        let f = calls(&s);
        assert_eq!(f.calls, 1);
        assert_eq!(f.tools, 0, "the call has no tools");
        assert_eq!(f.models, vec!["decide-small".to_string()]);
        assert!(
            f.prompts[0].contains("Cache the yes-ask step"),
            "{}",
            f.prompts[0]
        );
        assert!(f.prompts[0].contains("Queue the cache work"));
        assert!(f.prompts[0].contains(crate::lead::ROUTINE_ASK_RUBRIC));
        assert!(f.prompts[0].contains(r#"{"answer":"yes"|"no"|"pass""#));
        // Decided: the next pass does not ask again.
        decide_pass(&s, "proj", TEST_NOW);
        assert_eq!(calls(&s).calls, 1);
        assert!(events(&s, "decide.fallback").is_empty());
    }

    #[test]
    fn a_no_is_decided_under_the_lead_with_the_role_model_and_provider() {
        let _serial = serial();
        let s = decide_store("```json\n{\"answer\": \"No\", \"reason\": \"already done\"}\n```");
        let post = ask(&s, "no-ask");
        decide_pass(&s, "proj", TEST_NOW);
        assert_decided(&s, &post, "no");
        assert_eq!(decision(&s, &post).str("words"), "already done");
    }

    #[test]
    fn a_pass_leaves_the_ask_to_the_lead_and_is_not_asked_again() {
        let _serial = serial();
        let s = decide_store(r#"{"answer":"pass","reason":"needs the code"}"#);
        let post = ask(&s, "pass-ask");
        decide_pass(&s, "proj", TEST_NOW);
        assert_left_to_lead(&s, &post);
        let ev = events(&s, "decide.pass");
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].str("subject"), post);
        let v = detail(&ev[0]);
        assert_eq!(v["decided_model"], "decide-small");
        assert_eq!(v["decided_provider"], "decidefake");
        assert!(
            events(&s, "decide.fallback").is_empty(),
            "a pass is an answer"
        );
        decide_pass(&s, "proj", TEST_NOW);
        assert_eq!(calls(&s).calls, 1, "a passed ask is the lead's");
    }

    /// TASK-148 item 1: an unreadable answer is a failed try of that ask
    /// alone. Before, it opened the store-wide outage, which handed every
    /// project's asks to its lead until the next readable answer.
    #[test]
    fn an_unreadable_answer_is_that_asks_try_and_opens_no_outage() {
        let _serial = serial();
        let s = decide_store(r#"{"answer":"yes","reason":"small and reversible"}"#);
        fake(&s, |f| {
            f.replies = vec![(
                "prose-ask".into(),
                "Yes, I think this is a good idea.".into(),
            )]
        });
        let first = ask(&s, "prose-ask");
        decide_pass(&s, "proj", TEST_NOW);
        assert_left_to_lead(&s, &first);
        let tries = events(&s, FAILED_ACTION);
        assert_eq!(tries.len(), 1);
        assert!(
            detail(&tries[0])["reason"]
                .as_str()
                .unwrap()
                .contains("unreadable answer"),
            "{:?}",
            tries[0]
        );
        assert!(events(&s, "decide.fallback").is_empty(), "no outage");
        assert!(!outage_open(&s));
        // The role still holds the other asks: the lead does not get them.
        let other = ask(&s, "other-ask");
        let asks = crate::lead::owner_addressed_asks(&s, "proj".into());
        assert!(held_by_role(&s, "proj", &asks, TEST_NOW).contains(&other));
        decide_pass(&s, "proj", TEST_NOW);
        assert_decided(&s, &other, "yes");
        assert!(events(&s, "decide.recovered").is_empty());
    }

    #[test]
    fn an_outage_is_said_once_and_its_recovery_once() {
        let _serial = serial();
        let s = decide_store("");
        fake(&s, |f| f.err = "502 bad gateway".into());
        let first = ask(&s, "out-ask");
        decide_pass(&s, "proj", TEST_NOW);
        assert_left_to_lead(&s, &first);
        let fb = events(&s, "decide.fallback");
        assert_eq!(fb.len(), 1);
        let v = detail(&fb[0]);
        assert!(v["reason"].as_str().unwrap().contains("502"), "{v}");
        assert_eq!(v["decided_model"], "decide-small");
        assert_eq!(v["decided_provider"], "decidefake");
        // Still out, once the ask may be tried again: counted, not said
        // again.
        let later = crate::util::plus_minutes(TEST_NOW, RETRY_MINUTES as f64 + 1.0);
        decide_pass(&s, "proj", &later);
        assert_eq!(events(&s, "decide.fallback").len(), 1);
        assert_eq!(s.meta(OUTAGE_COUNT_META).unwrap().0, "2");
        // An answer it can read: one recovered event, and the ask decided.
        fake(&s, |f| {
            f.err.clear();
            f.reply = r#"{"answer":"yes","reason":"small and reversible"}"#.into()
        });
        let later = crate::util::plus_minutes(&later, RETRY_MINUTES as f64 + 1.0);
        decide_pass(&s, "proj", &later);
        assert_decided(&s, &first, "yes");
        let rec = events(&s, "decide.recovered");
        assert_eq!(rec.len(), 1);
        assert_eq!(detail(&rec[0])["fallbacks"], 2);
        let second = ask(&s, "out-ask-2");
        decide_pass(&s, "proj", &later);
        assert_decided(&s, &second, "yes");
        assert_eq!(events(&s, "decide.recovered").len(), 1);
    }

    #[test]
    fn an_answer_outside_the_three_or_without_a_reason_cannot_be_read() {
        assert_eq!(read_answer(r#"{"answer":"maybe","reason":"x"}"#), None);
        assert_eq!(read_answer(r#"{"answer":"yes"}"#), None);
        assert_eq!(read_answer("yes"), None);
        assert_eq!(
            read_answer(r#"{"answer":"pass"}"#),
            Some(DecideAnswer {
                answer: "pass".into(),
                reason: String::new()
            })
        );
    }

    #[test]
    fn a_provider_error_is_a_fallback_and_the_lead_keeps_the_ask() {
        let _serial = serial();
        let s = decide_store("");
        fake(&s, |f| f.err = "502 bad gateway".into());
        let first = ask(&s, "err-ask");
        let second = ask(&s, "err-ask-2");
        let lines = decide_pass(&s, "proj", TEST_NOW);
        assert_left_to_lead(&s, &first);
        assert_left_to_lead(&s, &second);
        assert_eq!(
            calls(&s).calls,
            1,
            "the rest of the pass is left to the lead"
        );
        let fb = events(&s, "decide.fallback");
        assert_eq!(fb.len(), 1);
        assert!(
            detail(&fb[0])["reason"].as_str().unwrap().contains("502"),
            "{lines:?}"
        );
        assert!(events(&s, "decide").is_empty());
    }

    #[test]
    fn a_stalled_provider_is_a_fallback_at_the_timeout() {
        let _serial = serial();
        let s = decide_store(r#"{"answer":"yes","reason":"late"}"#);
        fake(&s, |f| f.stall_ms = 4000);
        let post = ask(&s, "stall-ask");
        let started = std::time::Instant::now();
        decide_pass(&s, "proj", TEST_NOW);
        let took = started.elapsed();
        assert!(took < Duration::from_millis(3500), "waited {took:?}");
        assert_left_to_lead(&s, &post);
        let fb = events(&s, "decide.fallback");
        assert_eq!(fb.len(), 1);
        assert!(
            detail(&fb[0])["reason"]
                .as_str()
                .unwrap()
                .contains("no answer within 1s"),
            "{:?}",
            fb[0]
        );
    }

    #[test]
    fn shadow_records_the_answer_and_decides_nothing() {
        let _serial = serial();
        let s = decide_store(r#"{"answer":"yes","reason":"fine"}"#);
        set(&s, "decide.mode", "shadow");
        let post = ask(&s, "shadow-ask");
        decide_pass(&s, "proj", TEST_NOW);
        assert_left_to_lead(&s, &post);
        let ev = events(&s, "decide.shadow");
        assert_eq!(ev.len(), 1);
        let v = detail(&ev[0]);
        assert_eq!(v["answer"], "yes");
        assert_eq!(v["decided_model"], "decide-small");
        decide_pass(&s, "proj", TEST_NOW);
        assert_eq!(calls(&s).calls, 1, "recorded once");
    }

    #[test]
    fn off_makes_no_call() {
        let _serial = serial();
        let s = decide_store(r#"{"answer":"yes","reason":"fine"}"#);
        let post = ask(&s, "off-ask");
        set(&s, "decide.mode", "off");
        decide_pass(&s, "proj", TEST_NOW);
        assert!(tend_decide(&s, "proj", TEST_NOW).is_empty());
        assert_eq!(calls(&s).calls, 0);
        assert!(is_nil_row(&decision(&s, &post)));
    }

    /// TASK-129: the lead decides routine asks whatever owner.presence
    /// says, so the role answers them with the owner present too.
    #[test]
    fn an_owner_at_hand_still_leaves_routine_asks_to_the_role() {
        let _serial = serial();
        let s = decide_store(r#"{"answer":"yes","reason":"fine"}"#);
        set(&s, "owner.presence", "present");
        let post = ask(&s, "present-ask");
        decide_pass(&s, "proj", TEST_NOW);
        assert_eq!(calls(&s).calls, 1);
        assert_decided(&s, &post, "yes");
    }

    #[test]
    fn owner_only_and_major_upgrade_asks_are_never_put_to_the_role() {
        let _serial = serial();
        let s = decide_store(r#"{"answer":"yes","reason":"fine"}"#);
        let spend = crate::board::create_post(
            &s,
            TEST_NOW.into(),
            crate::board::PostOpts {
                kind: "suggestion".into(),
                project: "proj".into(),
                area: "build".into(),
                thread: "spend".into(),
                summary: "Raise compute.max_vms to 4".into(),
                body: "Yes: Raise it\nNo: Keep it\n".into(),
                agent: crate::board::actor_agent(&s, "daemon".into()),
                for_owner: true,
                ..Default::default()
            },
        )
        .unwrap()
        .str("id");
        let major = ask(&s, "major");
        s.event(
            "security",
            "security.major",
            &major,
            json!({"post": major}).to_string(),
        )
        .unwrap();
        let hard = crate::board::create_post(
            &s,
            TEST_NOW.into(),
            crate::board::PostOpts {
                kind: "handoff".into(),
                project: "proj".into(),
                area: "build".into(),
                thread: "hard-stop".into(),
                summary: "Delete the old release tags".into(),
                body: "This crosses a hard stop.\n\nYes: Delete them\nNo: Keep them\n".into(),
                agent: crate::board::actor_agent(&s, "daemon".into()),
                for_owner: true,
                ..Default::default()
            },
        )
        .unwrap()
        .str("id");
        let asks = decide_asks(&s, "proj", TEST_NOW);
        assert!(
            asks.iter()
                .all(|a| a.id != spend && a.id != major && a.id != hard),
            "{asks:?}"
        );
        decide_pass(&s, "proj", TEST_NOW);
        assert_eq!(calls(&s).calls, 0);
    }

    #[test]
    fn the_daemon_hook_runs_the_pass_in_the_background() {
        let _serial = serial();
        let s = decide_store(r#"{"answer":"no","reason":"stale"}"#);
        let post = ask(&s, "hook-ask");
        tend_decide(&s, "proj", TEST_NOW);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut lines = Vec::new();
        while std::time::Instant::now() < deadline {
            lines.extend(tend_decide(&s, "proj", TEST_NOW));
            if !lines.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(lines.iter().any(|l| l.contains(&post)), "{lines:?}");
        assert_decided(&s, &post, "no");
    }

    /// The lead's jobs whose brief names `post`.
    /// Waits (up to 10s) until no pass of `proj` is in flight, without
    /// starting one.
    fn wait_idle(s: &hive_store::Store) {
        let busy = || {
            DECIDE_RUNS
                .lock()
                .get(&run_key(s, "proj"))
                .is_some_and(|r| r.inflight)
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while busy() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!busy(), "the pass is still in flight");
    }

    fn lead_jobs_naming(s: &hive_store::Store, post: &str) -> usize {
        crate::util::must_rows(
            &s.conn().unwrap(),
            "SELECT brief FROM jobs WHERE kind = 'lead'",
            &[],
        )
        .iter()
        .filter(|j| j.str("brief").contains(post))
        .count()
    }

    fn post_answer_on(s: &hive_store::Store, post: &str) {
        crate::board::create_post(
            s,
            TEST_NOW.into(),
            crate::board::PostOpts {
                kind: "answer".into(),
                project: "proj".into(),
                area: "build".into(),
                thread: "answered".into(),
                summary: "Done by hand".into(),
                body: format!("re: {post}\n"),
                on: post.into(),
                agent: crate::board::actor_agent(s, "daemon".into()),
                ..Default::default()
            },
        )
        .unwrap();
    }

    #[test]
    fn an_ask_closed_or_marked_major_while_the_role_thinks_is_not_written() {
        let _serial = serial();
        let s = decide_store(r#"{"answer":"yes","reason":"fine"}"#);
        let answered = ask(&s, "closed-meanwhile");
        let store = (*s).clone();
        let id = answered.clone();
        during(&s, move || post_answer_on(&store, &id));
        let lines = decide_pass(&s, "proj", TEST_NOW);
        assert!(is_nil_row(&decision(&s, &answered)), "{lines:?}");
        assert!(
            lines
                .iter()
                .any(|l| l.contains("answered or its thread closed")),
            "{lines:?}"
        );
        assert!(events(&s, "decide.refused").is_empty());
        assert!(events(&s, "decide").is_empty());

        let major = ask(&s, "major-meanwhile");
        let store = (*s).clone();
        let id = major.clone();
        during(&s, move || {
            crate::util::event_at(
                &store.conn().unwrap(),
                TEST_NOW,
                "security",
                "security.major",
                &id,
                json!({"post": id}),
            )
        });
        let lines = decide_pass(&s, "proj", TEST_NOW);
        assert!(is_nil_row(&decision(&s, &major)), "{lines:?}");
        assert!(
            lines.iter().any(|l| l.contains("major dependency upgrade")),
            "{lines:?}"
        );
        assert!(events(&s, "decide").is_empty());
    }

    #[test]
    fn while_the_role_is_on_the_lead_does_not_sit_for_its_asks() {
        let _serial = serial();
        let s = decide_store(r#"{"answer":"pass","reason":"needs the code"}"#);
        let post = ask(&s, "same-tick");
        // The same tick as the daemon's: the role's pass has not answered
        // yet, and the lead leaves the ask to it.
        crate::lead::tend_lead(&s, "proj".into(), TEST_NOW.into(), "daemon".into());
        assert_eq!(lead_jobs_naming(&s, &post), 0, "the role holds the ask");
        // In shadow the lead decides it.
        set(&s, "decide.mode", "shadow");
        let held = held_by_role(
            &s,
            "proj",
            &crate::lead::owner_addressed_asks(&s, "proj".into()),
            TEST_NOW,
        );
        assert!(held.is_empty(), "{held:?}");
        set(&s, "decide.mode", "on");
        // A pass is the lead's.
        decide_pass(&s, "proj", TEST_NOW);
        crate::lead::tend_lead(&s, "proj".into(), TEST_NOW.into(), "daemon".into());
        assert_eq!(lead_jobs_naming(&s, &post), 1, "a passed ask is the lead's");
    }

    /// TASK-129 with TASK-137: the role runs with the owner at hand too, so
    /// the lead leaves the same asks to it then. A hard stop is never the
    /// role's, so it is never held: one without a recommended answer wakes
    /// the lead, whose brief still leaves the held ask out.
    #[test]
    fn the_role_holds_its_asks_with_the_owner_at_hand_too() {
        let _serial = serial();
        let s = decide_store(r#"{"answer":"yes","reason":"fine"}"#);
        set(&s, "owner.presence", "present");
        // The hard stop first: the second post's id ends in -2, so no id
        // here is the start of another one a brief names.
        let hard = crate::board::create_post(
            &s,
            TEST_NOW.into(),
            crate::board::PostOpts {
                kind: "handoff".into(),
                project: "proj".into(),
                area: "build".into(),
                thread: "hard-stop".into(),
                summary: "Delete the old release tags".into(),
                body: "This crosses a hard stop.\n\nYes: Delete them\nNo: Keep them\n".into(),
                agent: crate::board::actor_agent(&s, "daemon".into()),
                for_owner: true,
                ..Default::default()
            },
        )
        .unwrap()
        .str("id");
        let post = ask(&s, "at-hand");
        assert!(post.starts_with(&hard) && post != hard, "{hard} {post}");
        let asks = crate::lead::owner_addressed_asks(&s, "proj".into());
        let held = held_by_role(&s, "proj", &asks, TEST_NOW);
        assert!(held.contains(&post), "{held:?}");
        assert!(!held.contains(&hard), "{held:?}");
        crate::lead::tend_lead(&s, "proj".into(), TEST_NOW.into(), "daemon".into());
        assert_eq!(lead_jobs_naming(&s, &post), 0, "the role holds the ask");
        assert_eq!(
            lead_jobs_naming(&s, &hard),
            1,
            "the hard stop is the lead's to ask again"
        );
        decide_pass(&s, "proj", TEST_NOW);
        assert_decided(&s, &post, "yes");
        assert!(is_nil_row(&decision(&s, &hard)));
    }

    #[test]
    fn an_outage_gives_the_asks_back_to_the_lead() {
        let _serial = serial();
        let s = decide_store("");
        fake(&s, |f| f.err = "502 bad gateway".into());
        let post = ask(&s, "outage-ask");
        let asks = crate::lead::owner_addressed_asks(&s, "proj".into());
        assert_eq!(held_by_role(&s, "proj", &asks, TEST_NOW).len(), 1);
        decide_pass(&s, "proj", TEST_NOW);
        let later = crate::util::plus_minutes(TEST_NOW, RETRY_MINUTES as f64 + 1.0);
        assert!(
            held_by_role(&s, "proj", &asks, &later).is_empty(),
            "an open outage leaves every ask to the lead"
        );
        crate::lead::tend_lead(&s, "proj".into(), later, "daemon".into());
        assert_eq!(lead_jobs_naming(&s, &post), 1);
    }

    #[test]
    fn a_panic_in_the_pass_is_an_outage_and_the_next_pass_runs() {
        let _serial = serial();
        let s = decide_store(r#"{"answer":"yes","reason":"fine"}"#);
        set(&s, "decide_model", "");
        fake(&s, |f| f.panic = true);
        let post = ask(&s, "panic-ask");
        assert!(tend_decide(&s, "proj", TEST_NOW).is_empty());
        // The call that hands over the panicked pass's lines also starts the
        // next pass: the fake is flipped before it, never after (the review
        // of pull request 278 saw the second pass read panic = true).
        wait_idle(&s);
        let fb = events(&s, "decide.fallback");
        assert_eq!(fb.len(), 1);
        assert!(
            detail(&fb[0])["reason"]
                .as_str()
                .unwrap()
                .contains("the fake's model list broke"),
            "{:?}",
            fb[0]
        );
        assert_left_to_lead(&s, &post);
        // The next pass runs: the role was not left switched off.
        fake(&s, |f| {
            f.panic = false;
            f.list = vec!["small".into(), "fake-default".into()];
        });
        let lines = tend_decide(&s, "proj", TEST_NOW);
        assert!(
            lines
                .iter()
                .any(|l| l.contains("panicked") && l.contains("the asks stay with the lead")),
            "{lines:?}"
        );
        wait_idle(&s);
        let lines = tend_decide(&s, "proj", TEST_NOW);
        assert!(lines.iter().any(|l| l.contains(&post)), "{lines:?}");
        let d = decision(&s, &post);
        assert_eq!(d.str("answer"), "yes", "{d:?}");
        assert_eq!(events(&s, "decide.recovered").len(), 1);
    }

    /// The review of pull request 277: a store that refuses writes is what
    /// makes a pass panic, and then the handler's own outage write panics
    /// too. The in-flight mark comes off all the same, so the next pass
    /// runs; before, nothing but a restart of the daemon started one.
    #[test]
    fn a_panic_while_the_outage_is_written_still_ends_the_pass() {
        let _serial = serial();
        let s = decide_store(r#"{"answer":"yes","reason":"fine"}"#);
        set(&s, "decide_model", "");
        fake(&s, |f| f.panic = true);
        let post = ask(&s, "double-panic-ask");
        // The store takes no event from here on.
        s.conn()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER no_events BEFORE INSERT ON events \
                 BEGIN SELECT RAISE(ABORT, 'the store refuses the write'); END;",
            )
            .unwrap();
        let inflight = || {
            DECIDE_RUNS
                .lock()
                .get(&run_key(&s, "proj"))
                .is_some_and(|r| r.inflight)
        };
        assert!(tend_decide(&s, "proj", TEST_NOW).is_empty());
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while inflight() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!inflight(), "the pass is still marked in flight");
        assert!(events(&s, "decide.fallback").is_empty());
        // The store takes writes again and the provider answers: the next
        // pass runs and decides the ask.
        s.conn()
            .unwrap()
            .execute_batch("DROP TRIGGER no_events")
            .unwrap();
        fake(&s, |f| {
            f.panic = false;
            f.list = vec!["small".into(), "fake-default".into()];
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut lines = Vec::new();
        while std::time::Instant::now() < deadline {
            lines.extend(tend_decide(&s, "proj", TEST_NOW));
            if lines.iter().any(|l| l.contains(&post)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(lines.iter().any(|l| l.contains("panicked")), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains(&post)), "{lines:?}");
        let d = decision(&s, &post);
        assert_eq!(d.str("answer"), "yes", "{d:?}");
    }

    #[test]
    fn a_failing_ask_waits_and_the_asks_behind_it_still_reach_the_role() {
        let _serial = serial();
        let s = decide_store(r#"{"answer":"yes","reason":"small and reversible"}"#);
        fake(&s, |f| {
            f.replies = vec![("bad-ask".into(), "I would say yes, probably.".into())]
        });
        let bad = ask(&s, "bad-ask");
        let good = ask(&s, "good-ask");
        decide_pass(&s, "proj", TEST_NOW);
        assert_left_to_lead(&s, &bad);
        assert_decided(&s, &good, "yes");
        assert_eq!(calls(&s).calls, 2);
        // Inside the wait it is not sent again.
        decide_pass(&s, "proj", &crate::util::plus_minutes(TEST_NOW, 10.0));
        assert_eq!(calls(&s).calls, 2, "the failed ask waits");
        // After the wait it is, up to MAX_TRIES; then it is the lead's.
        let mut at = TEST_NOW.to_string();
        for _ in 0..MAX_TRIES + 1 {
            at = crate::util::plus_minutes(&at, RETRY_MINUTES as f64 + 1.0);
            decide_pass(&s, "proj", &at);
        }
        assert_eq!(calls(&s).calls, 1 + MAX_TRIES as usize);
        assert_eq!(events(&s, FAILED_ACTION).len(), MAX_TRIES as usize);
        assert_left_to_lead(&s, &bad);
        assert!(decide_asks(&s, "proj", &at).is_empty());
        crate::lead::tend_lead(&s, "proj".into(), at, "daemon".into());
        assert_eq!(lead_jobs_naming(&s, &bad), 1);
    }

    #[test]
    fn an_empty_model_is_the_providers_default_and_is_recorded() {
        let _serial = serial();
        let s = decide_store(r#"{"answer":"no","reason":"stale"}"#);
        set(&s, "decide_model", "");
        assert_eq!(s.decide_model(), "", "the shipped default");
        fake(&s, |f| f.list = vec!["first".into(), "fake-default".into()]);
        let post = ask(&s, "default-model");
        decide_pass(&s, "proj", TEST_NOW);
        assert_eq!(calls(&s).models, vec!["fake-default".to_string()]);
        let d = decision(&s, &post);
        assert_eq!(d.str("answer"), "no", "{d:?}");
        assert_eq!(d.str("decided_model"), "fake-default");
    }

    #[test]
    fn the_mode_switched_off_mid_pass_stops_it_and_refuses_nothing() {
        let _serial = serial();
        let s = decide_store(r#"{"answer":"yes","reason":"fine"}"#);
        let first = ask(&s, "off-1");
        let second = ask(&s, "off-2");
        let store = (*s).clone();
        during(&s, move || {
            store
                .set_setting(crate::util::HUMAN_AGENT, "decide.mode", "off")
                .unwrap();
        });
        let lines = decide_pass(&s, "proj", TEST_NOW);
        assert_eq!(calls(&s).calls, 1, "the pass stopped: {lines:?}");
        assert!(events(&s, "decide.refused").is_empty());
        assert!(is_nil_row(&decision(&s, &first)));
        assert!(is_nil_row(&decision(&s, &second)));
        // On again: both reach the role.
        during(&s, || {});
        set(&s, "decide.mode", "on");
        decide_pass(&s, "proj", TEST_NOW);
        assert_decided(&s, &first, "yes");
        assert_decided(&s, &second, "yes");
    }

    /// TASK-148 item 5: a switch to shadow mid-pass does not stop the pass:
    /// it keeps calling and only records from then on.
    #[test]
    fn shadow_mid_pass_keeps_calling_and_only_records() {
        let _serial = serial();
        let s = decide_store(r#"{"answer":"yes","reason":"fine"}"#);
        let first = ask(&s, "shadow-1");
        let second = ask(&s, "shadow-2");
        let store = (*s).clone();
        during(&s, move || {
            store
                .set_setting(crate::util::HUMAN_AGENT, "decide.mode", "shadow")
                .unwrap();
        });
        let lines = decide_pass(&s, "proj", TEST_NOW);
        assert_eq!(calls(&s).calls, 2, "the pass went on: {lines:?}");
        assert!(is_nil_row(&decision(&s, &first)));
        assert!(is_nil_row(&decision(&s, &second)));
        let shadow = events(&s, "decide.shadow");
        assert_eq!(shadow.len(), 2, "{lines:?}");
        assert!(events(&s, "decide").is_empty());
    }

    /// TASK-148 item 5: a call that errors is one failed try of the ask it
    /// was about, besides the outage; the asks behind it get none.
    #[test]
    fn a_call_error_is_one_try_of_that_ask() {
        let _serial = serial();
        let s = decide_store("");
        fake(&s, |f| f.err = "502 bad gateway".into());
        let first = ask(&s, "try-1");
        let second = ask(&s, "try-2");
        decide_pass(&s, "proj", TEST_NOW);
        let tries = events(&s, FAILED_ACTION);
        assert_eq!(tries.len(), 1);
        assert_eq!(tries[0].str("subject"), first);
        assert!(
            detail(&tries[0])["reason"]
                .as_str()
                .unwrap()
                .contains("502")
        );
        assert_left_to_lead(&s, &second);
    }

    /// TASK-148 item 5: an ask `hive_board::decide` refuses is recorded as
    /// refused, never put to the role again, and is the lead's.
    #[test]
    fn a_refused_ask_goes_to_the_lead() {
        let _serial = serial();
        let s = decide_store(r#"{"answer":"yes","reason":"fine"}"#);
        let post = ask(&s, "refused-then-lead");
        // A compute request filed on the ask while the role thinks makes it
        // money: `hive_board::decide` refuses the role's answer.
        let store = (*s).clone();
        let id = post.clone();
        during(&s, move || {
            store
                .conn()
                .unwrap()
                .execute(
                    "INSERT OR IGNORE INTO compute_requests \
                     (id, project, requested_by, size, hours, proposal_post, at, updated_at) \
                     VALUES ('CR-001', 'proj', 'daemon', 'small', 1, ?, '', '')",
                    [&id],
                )
                .unwrap();
        });
        let lines = decide_pass(&s, "proj", TEST_NOW);
        assert!(lines.iter().any(|l| l.contains("refused")), "{lines:?}");
        assert_eq!(events(&s, "decide.refused").len(), 1);
        assert!(is_nil_row(&decision(&s, &post)));
        let later = crate::util::plus_minutes(TEST_NOW, RETRY_MINUTES as f64 + 1.0);
        assert!(decide_asks(&s, "proj", &later).is_empty());
        decide_pass(&s, "proj", &later);
        assert_eq!(calls(&s).calls, 1, "not asked again");
        crate::lead::tend_lead(&s, "proj".into(), later, "daemon".into());
        assert_eq!(lead_jobs_naming(&s, &post), 1);
    }

    /// TASK-148 item 5: with the owner at hand, an ask the role passed, one
    /// it failed on and one an outage left reach the lead all the same.
    #[test]
    fn passed_failed_and_outage_asks_reach_the_lead_with_the_owner_present() {
        let _serial = serial();
        let s = decide_store(r#"{"answer":"pass","reason":""}"#);
        set(&s, "owner.presence", "present");
        fake(&s, |f| {
            f.replies = vec![("bad-one".into(), "maybe".into())];
        });
        let passed = ask(&s, "pass-one");
        let bad = ask(&s, "bad-one");
        decide_pass(&s, "proj", TEST_NOW);
        assert_eq!(events(&s, "decide.pass").len(), 1);
        assert_eq!(events(&s, FAILED_ACTION).len(), 1);
        crate::lead::tend_lead(&s, "proj".into(), TEST_NOW.into(), "daemon".into());
        assert_eq!(lead_jobs_naming(&s, &passed), 1, "the passed ask");
        assert_eq!(lead_jobs_naming(&s, &bad), 1, "the failed ask");

        let s = decide_store("");
        set(&s, "owner.presence", "present");
        fake(&s, |f| f.err = "502 bad gateway".into());
        let out = ask(&s, "outage-one");
        decide_pass(&s, "proj", TEST_NOW);
        assert!(outage_open(&s));
        crate::lead::tend_lead(&s, "proj".into(), TEST_NOW.into(), "daemon".into());
        assert_eq!(lead_jobs_naming(&s, &out), 1, "the outage's ask");
    }

    /// TASK-148 item 2: an empty `decide_model` resolves the provider's
    /// default once, not on every pass that has asks.
    #[test]
    fn the_resolved_default_model_is_kept_between_passes() {
        let _serial = serial();
        let s = decide_store(r#"{"answer":"no","reason":"stale"}"#);
        set(&s, "decide_model", "");
        fake(&s, |f| f.list = vec!["first".into(), "fake-default".into()]);
        ask(&s, "cache-1");
        decide_pass(&s, "proj", TEST_NOW);
        ask(&s, "cache-2");
        decide_pass(&s, "proj", TEST_NOW);
        let f = calls(&s);
        assert_eq!(f.calls, 2);
        assert_eq!(f.models, vec!["fake-default".to_string(); 2]);
        assert_eq!(f.list_calls, 1, "the model list was fetched again");
    }

    /// TASK-148 item 7: while every pass panics (a store that refuses
    /// writes) the passes back off 1, 5 and 15 minutes from the second
    /// panic in a row, and a pass that ends normally resets the wait.
    #[test]
    fn repeated_panics_of_the_pass_back_off() {
        let _serial = serial();
        let s = decide_store(r#"{"answer":"pass","reason":""}"#);
        ask(&s, "backoff-ask");
        refuse_events(&s);
        let tick = |at: &str| {
            let lines = tend_decide(&s, "proj", at);
            wait_idle(&s);
            lines
        };
        tick(TEST_NOW);
        assert_eq!(calls(&s).calls, 1);
        // The first panic waits for nothing.
        tick(TEST_NOW);
        assert_eq!(calls(&s).calls, 2);
        // The second waits a minute.
        let lines = tick(TEST_NOW);
        assert_eq!(calls(&s).calls, 2, "no pass inside the wait");
        assert!(lines.iter().any(|l| l.contains("waits 1 min")), "{lines:?}");
        tick(&crate::util::plus_minutes(TEST_NOW, 0.5));
        assert_eq!(calls(&s).calls, 2);
        tick(&crate::util::plus_minutes(TEST_NOW, 1.5));
        assert_eq!(calls(&s).calls, 3);
        // The third waits five.
        tick(&crate::util::plus_minutes(TEST_NOW, 5.0));
        assert_eq!(calls(&s).calls, 3);
        tick(&crate::util::plus_minutes(TEST_NOW, 7.0));
        assert_eq!(calls(&s).calls, 4);
        // The fourth and later fifteen.
        tick(&crate::util::plus_minutes(TEST_NOW, 20.0));
        assert_eq!(calls(&s).calls, 4);
        // The store takes writes again: the pass ends normally and the next
        // tick runs at once.
        s.conn()
            .unwrap()
            .execute_batch("DROP TRIGGER no_events")
            .unwrap();
        tick(&crate::util::plus_minutes(TEST_NOW, 23.0));
        assert_eq!(calls(&s).calls, 5);
        let second = ask(&s, "backoff-ask-2");
        tick(&crate::util::plus_minutes(TEST_NOW, 23.0));
        tick(&crate::util::plus_minutes(TEST_NOW, 23.0));
        assert_eq!(calls(&s).calls, 6, "no wait after a pass that ended");
        assert_eq!(events(&s, "decide.pass").len(), 2, "{second}");
    }

    /// TASK-148 item 8: an outage opened while the events table refused the
    /// insert gets its `decide.fallback` event on its next fallback.
    #[test]
    fn an_outage_opened_without_its_event_gets_it_later() {
        let _serial = serial();
        let s = decide_store("");
        let by = role_by(&s);
        refuse_events(&s);
        fallback(&s, TEST_NOW, "proj", "P-1", &by, "502 bad gateway");
        assert!(outage_open(&s));
        s.conn()
            .unwrap()
            .execute_batch("DROP TRIGGER no_events")
            .unwrap();
        assert!(events(&s, "decide.fallback").is_empty());
        fallback(&s, TEST_NOW, "proj", "P-2", &by, "a later reason");
        let fb = events(&s, "decide.fallback");
        assert_eq!(fb.len(), 1);
        assert_eq!(detail(&fb[0])["reason"], "502 bad gateway");
        fallback(&s, TEST_NOW, "proj", "P-3", &by, "502 bad gateway");
        assert_eq!(events(&s, "decide.fallback").len(), 1, "said once");
        assert_eq!(s.meta(OUTAGE_COUNT_META).unwrap().0, "3");
        // After a recovery the next outage is said again.
        recovered(&s, TEST_NOW, &by);
        fallback(&s, TEST_NOW, "proj", "P-4", &by, "timeout");
        assert_eq!(events(&s, "decide.fallback").len(), 2);
    }

    /// TASK-148 item 9: a store that refuses every write (events and meta):
    /// the pass ends, no outage could be recorded, so the line says the next
    /// pass tries again, not that the lead has the asks.
    #[test]
    fn every_write_refused_ends_the_pass_and_says_the_next_pass_tries() {
        let _serial = serial();
        let s = decide_store(r#"{"answer":"pass","reason":""}"#);
        let post = ask(&s, "refused-ask");
        refuse_events(&s);
        s.conn()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER no_meta BEFORE INSERT ON meta \
                 BEGIN SELECT RAISE(ABORT, 'the store refuses the write'); END; \
                 CREATE TRIGGER no_meta_update BEFORE UPDATE ON meta \
                 BEGIN SELECT RAISE(ABORT, 'the store refuses the write'); END;",
            )
            .unwrap();
        tend_decide(&s, "proj", TEST_NOW);
        wait_idle(&s);
        assert!(!outage_open(&s));
        let lines = tend_decide(&s, "proj", TEST_NOW);
        wait_idle(&s);
        assert!(
            lines
                .iter()
                .any(|l| l.contains("panicked") && l.contains("the next pass tries again")),
            "{lines:?}"
        );
        assert!(
            !lines.iter().any(|l| l.contains("stay with the lead")),
            "{lines:?}"
        );
        assert!(is_nil_row(&decision(&s, &post)));
    }

    /// TASK-148 item 9: a pass thread that cannot start leaves no mark in
    /// flight; the next tick starts the pass.
    #[test]
    fn a_pass_thread_that_cannot_start_is_tried_on_the_next_tick() {
        let _serial = serial();
        let s = decide_store(r#"{"answer":"yes","reason":"fine"}"#);
        let post = ask(&s, "spawn-ask");
        SPAWN_FAILS.lock().unwrap().insert(s.home.clone());
        let lines = tend_decide(&s, "proj", TEST_NOW);
        SPAWN_FAILS.lock().unwrap().remove(&s.home);
        assert!(
            lines
                .iter()
                .any(|l| l.contains("could not start") && l.contains("the next pass tries again")),
            "{lines:?}"
        );
        assert_eq!(calls(&s).calls, 0);
        tend_decide(&s, "proj", TEST_NOW);
        wait_idle(&s);
        assert_decided(&s, &post, "yes");
    }

    /// TASK-148 item 10: the daemon reads a project's asks once per tick
    /// and hands the same list to the role and to the lead.
    #[test]
    fn the_role_and_the_lead_work_from_the_asks_they_are_handed() {
        let _serial = serial();
        let s = decide_store(r#"{"answer":"yes","reason":"fine"}"#);
        set(&s, "owner.presence", "present");
        let post = ask(&s, "handed");
        // Handed no asks, neither starts work on the waiting one.
        assert!(tend_decide_with(&s, "proj", TEST_NOW, &[]).is_empty());
        wait_idle(&s);
        assert_eq!(calls(&s).calls, 0);
        set(&s, "decide.mode", "off");
        crate::lead::tend_lead_with(
            &s,
            "proj".into(),
            TEST_NOW.into(),
            "daemon".into(),
            Vec::new(),
        );
        assert_eq!(lead_jobs_naming(&s, &post), 0);
        // Handed the tick's reading, the lead sits for it.
        let asks = crate::lead::owner_addressed_asks(&s, "proj".into());
        crate::lead::tend_lead_with(&s, "proj".into(), TEST_NOW.into(), "daemon".into(), asks);
        assert_eq!(lead_jobs_naming(&s, &post), 1);
    }

    fn refuse_events(s: &hive_store::Store) {
        s.conn()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER no_events BEFORE INSERT ON events \
                 BEGIN SELECT RAISE(ABORT, 'the store refuses the write'); END;",
            )
            .unwrap();
    }

    /// TASK-129: presence plays no part, so an owner who returns while the
    /// role waits for an answer does not stop the pass.
    #[test]
    fn the_owner_back_mid_pass_changes_nothing() {
        let _serial = serial();
        let s = decide_store(r#"{"answer":"yes","reason":"fine"}"#);
        let first = ask(&s, "back-1");
        let second = ask(&s, "back-2");
        let store = (*s).clone();
        during(&s, move || {
            store
                .set_setting(crate::util::HUMAN_AGENT, "owner.presence", "present")
                .unwrap();
        });
        decide_pass(&s, "proj", TEST_NOW);
        assert_eq!(calls(&s).calls, 2, "the pass went on");
        assert!(events(&s, "decide.refused").is_empty());
        assert_decided(&s, &first, "yes");
        assert_decided(&s, &second, "yes");
    }

    #[test]
    fn the_reason_is_one_clipped_line() {
        let long = "word ".repeat(200);
        let reply =
            json!({"answer": "no", "reason": format!("stale:\n\n  done in\tPR 12.\n{long}")})
                .to_string();
        let a = read_answer(&reply).unwrap();
        assert!(
            !a.reason.contains('\n') && !a.reason.contains('\t'),
            "{a:?}"
        );
        assert!(a.reason.starts_with("stale: done in PR 12. word"), "{a:?}");
        assert!(a.reason.len() <= REASON_CHARS + 3, "{}", a.reason.len());
    }

    #[test]
    fn the_leads_own_decision_records_the_model_its_run_ran() {
        let _serial = serial();
        let s = decide_store("");
        set(&s, "decide.mode", "off");
        set(&s, "light_model", "light-setting");
        let post = ask(&s, "lead-model");
        let mut job = crate::util::Row::new();
        job.insert("kind", json!(crate::lead::LEAD_KIND));
        job.insert("project", json!("proj"));
        job.insert("source", json!("lead:proj:x"));
        let rep = crate::brief::Report {
            decisions: vec![crate::brief::Decision {
                post_id: post.clone(),
                answer: "yes".into(),
                reason: "fine".into(),
            }],
            ..Default::default()
        };
        let mut log = crate::run::RunLog::default();
        let got = crate::run::apply_result_decisions_ran(&s, job, rep, &mut log, "agent-pinned");
        assert_eq!(got, 1, "{}", log.text());
        let d = decision(&s, &post);
        assert_eq!(d.str("decided_by"), "proj-lead");
        assert_eq!(d.str("decided_model"), "agent-pinned", "{d:?}");
    }
}
