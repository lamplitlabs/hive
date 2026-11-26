//! The decide role (ADR-072, first slice): on each daemon pass, every open
//! routine ask the lead may decide is put to `decide_model` on
//! `decide_provider` in one call with no tools. A yes or a no is decided
//! under the lead's name through `hive_board::decide_with`, so every gate of
//! a lead's decision still applies; a pass, an error, a late answer or one
//! that cannot be read leaves the ask in the lead's brief, as before the
//! role. In `decide.mode = shadow` the answer is recorded and nothing is
//! decided.
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
/// How much of a post body and of the thread the call carries.
const BODY_CHARS: usize = 4000;
const THREAD_CHARS: usize = 8000;

#[derive(Debug, Clone, Default)]
struct DecideRuns {
    inflight: bool,
    lines: Vec<String>,
}

static DECIDE_RUNS: LazyLock<parking_lot::Mutex<BTreeMap<String, DecideRuns>>> =
    LazyLock::new(|| parking_lot::Mutex::new(BTreeMap::new()));

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
pub(crate) fn tend_decide(s: &hive_store::Store, slug: &str, now: &str) -> Vec<String> {
    let mut runs = DECIDE_RUNS.lock();
    let run = runs.entry(slug.to_string()).or_default();
    let lines = std::mem::take(&mut run.lines);
    if run.inflight || !role_due(s) || decide_asks(s, slug).is_empty() {
        return lines;
    }
    run.inflight = true;
    let (s, slug, now) = (s.clone(), slug.to_string(), now.to_string());
    std::thread::spawn(move || {
        let out = decide_pass(&s, &slug, &now);
        let mut runs = DECIDE_RUNS.lock();
        let run = runs.entry(slug).or_default();
        run.inflight = false;
        run.lines.extend(out);
    });
    lines
}

/// Whether the role runs at all now: `decide.mode` is not off. The lead
/// decides the owner's routine asks whatever `owner.presence` says
/// (TASK-129), so the role does too.
fn role_due(s: &hive_store::Store) -> bool {
    s.decide_mode() != "off"
}

/// The asks of `slug` the role may answer, oldest first.
pub(crate) fn decide_asks(s: &hive_store::Store, slug: &str) -> Vec<DecideAsk> {
    let questions = crate::lead::owner_addressed_asks(s, slug.to_string());
    if questions.is_empty() {
        return Vec::new();
    }
    let Ok(conn) = s.conn() else {
        return Vec::new();
    };
    let marks = vec!["?"; ANSWERED_ACTIONS.len()].join(", ");
    let answered_sql =
        format!("SELECT 1 AS x FROM events WHERE action IN ({marks}) AND subject = ? LIMIT 1");
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
            Some(ask)
        })
        .collect()
}

/// One decide pass over the open asks of `slug`, in the caller's thread.
/// Returns one line per ask it touched. After the first fallback of a pass
/// (no answer, a late or unreadable one) the rest of the pass is left to the
/// lead: the provider is out, and each further call would only wait again.
pub(crate) fn decide_pass(s: &hive_store::Store, slug: &str, now: &str) -> Vec<String> {
    if !role_due(s) {
        return Vec::new();
    }
    let lead = crate::lead::lead_name(slug.to_string());
    match s.agent(&lead) {
        Ok(Some(a)) if a.kind == "lead" && a.status == "active" => {}
        _ => return Vec::new(),
    }
    let mode = s.decide_mode();
    let by = hive_board::DecidedBy {
        model: s.decide_model(),
        provider: s.decide_provider(),
        role: true,
    };
    let timeout = Duration::from_secs(
        s.setting("decide.timeout_seconds", "30")
            .trim()
            .parse::<u64>()
            .unwrap_or(30)
            .max(1),
    );
    let mut lines = Vec::new();
    for ask in decide_asks(s, slug) {
        let prompt = decide_prompt(s, &ask);
        let answer = call_role(s, &by, prompt, timeout).and_then(|text| {
            read_answer(&text)
                .ok_or_else(|| anyhow::anyhow!("unreadable answer: {}", clip(text.trim(), 200)))
        });
        let answer = match answer {
            Ok(a) => a,
            Err(e) => {
                let reason = crate::run::err_message(&e);
                fallback(s, now, slug, &ask.id, &by, &reason);
                lines.push(format!(
                    "decide {slug}: {} left to the lead: {reason}",
                    ask.id
                ));
                break;
            }
        };
        recovered(s, now, &by);
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
                let mut detail = detail;
                detail["refused"] = json!(why);
                event(s, now, &lead, "decide.refused", &ask.id, detail);
                lines.push(format!("decide {slug}: {} refused: {why}", ask.id));
            }
        }
    }
    lines
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

/// The one call: the post, its thread and the rubric, no tools. The answer
/// must come before `timeout`; the provider is also handed the deadline, so
/// one that reads it stops on its own.
fn call_role(
    s: &hive_store::Store,
    by: &hive_board::DecidedBy,
    prompt: String,
    timeout: Duration,
) -> anyhow::Result<String> {
    let p = hive_provider::open(
        &by.provider,
        hive_provider::Options {
            home: s.home.clone(),
            settings: s.provider_settings(),
            ..Default::default()
        },
    )?;
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
        Err(_) => {
            cancel.cancel();
            anyhow::bail!("no answer within {}s", timeout.as_secs())
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
/// code fence around it is allowed. Anything else, an answer outside the
/// three, or a yes or no without a reason, cannot be read.
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
        .trim()
        .to_string();
    match answer.as_str() {
        "yes" | "no" if !reason.is_empty() => Some(DecideAnswer { answer, reason }),
        "pass" => Some(DecideAnswer { answer, reason }),
        _ => None,
    }
}

/// A fallback: the first of an outage writes one `decide.fallback` event
/// with its reason; later ones are only counted.
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
    if !open.is_empty() {
        return;
    }
    let _ = s.set_meta(OUTAGE_META, reason);
    if let Ok(conn) = s.conn() {
        crate::util::event_at(
            &conn,
            now,
            "daemon",
            "decide.fallback",
            post,
            json!({
                "project": slug,
                "reason": reason,
                "decided_model": by.model,
                "decided_provider": by.provider,
            }),
        );
    }
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
            if !f.err.is_empty() {
                anyhow::bail!("{}", f.err);
            }
            Ok(hive_provider::Response {
                message: Some(hive_provider::Message {
                    role: hive_provider::Role::ASSISTANT,
                    content: f.reply.clone(),
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
            Ok(Vec::new())
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

    #[test]
    fn prose_is_a_fallback_said_once_and_recovery_is_said_once() {
        let _serial = serial();
        let s = decide_store("Yes, I think this is a good idea.");
        let first = ask(&s, "prose-ask");
        decide_pass(&s, "proj", TEST_NOW);
        assert_left_to_lead(&s, &first);
        let fb = events(&s, "decide.fallback");
        assert_eq!(fb.len(), 1);
        let v = detail(&fb[0]);
        assert!(
            v["reason"].as_str().unwrap().contains("unreadable answer"),
            "{v}"
        );
        assert_eq!(v["decided_model"], "decide-small");
        assert_eq!(v["decided_provider"], "decidefake");
        // Still out: counted, not said again.
        decide_pass(&s, "proj", TEST_NOW);
        assert_eq!(events(&s, "decide.fallback").len(), 1);
        assert_eq!(s.meta(OUTAGE_COUNT_META).unwrap().0, "2");
        // An answer it can read: one recovered event, and the ask decided.
        fake(&s, |f| {
            f.reply = r#"{"answer":"yes","reason":"small and reversible"}"#.into()
        });
        decide_pass(&s, "proj", TEST_NOW);
        assert_decided(&s, &first, "yes");
        let rec = events(&s, "decide.recovered");
        assert_eq!(rec.len(), 1);
        assert_eq!(detail(&rec[0])["fallbacks"], 2);
        let second = ask(&s, "prose-ask-2");
        decide_pass(&s, "proj", TEST_NOW);
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
        let asks = decide_asks(&s, "proj");
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
}
