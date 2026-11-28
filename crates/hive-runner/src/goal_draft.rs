//! The runner's half of goal drafting (TASK-117): the model side-call that
//! drafts a project's goal from its evidence, the proposal post the owner
//! takes with one yes, and the daemon's re-check on a cadence and on events.
//!
//! The draft is never the goal: it goes to the board as an opinion on
//! `goal/amend` whose body is a proposal ([`hive_goals::review::parse_proposal`]),
//! and only the owner's yes through `hive decide`
//! ([`hive_goals::review::apply_decision`]) sets it. A draft the same as the
//! goal in force, as the proposal already waiting, or as one the owner
//! dismissed is not posted. Without a model (no login, no provider) the
//! generated template stays the goal and the daemon tries again later.

use anyhow::{Result, anyhow};
use hive_goals::draft::{self, Outcome, RecheckState, Standing};
use std::sync::{Arc, LazyLock, RwLock};

/// How long one drafting call may take.
pub(crate) const DRAFT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(4 * 60);
/// The answer budget of the drafting call: a goal is at most 4000
/// characters, with room for the reason.
pub(crate) const DRAFT_TOKENS: i32 = 2000;

/// The model call: `(store, model, system, user) -> reply`. Tests swap it.
pub(crate) type DraftCallFn =
    dyn Fn(&hive_store::Store, &str, &str, &str) -> Result<String> + Send + Sync;

static DRAFT_CALL: LazyLock<RwLock<Arc<DraftCallFn>>> =
    LazyLock::new(|| RwLock::new(Arc::new(run_draft_call)));

#[cfg(test)]
pub(crate) fn set_draft_call(f: Arc<DraftCallFn>) -> Arc<DraftCallFn> {
    std::mem::replace(&mut *DRAFT_CALL.write().expect("draft call lock"), f)
}

/// Projects with a drafting run in flight, so the daemon never starts a
/// second one for the same project.
static INFLIGHT: LazyLock<parking_lot::Mutex<std::collections::BTreeSet<String>>> =
    LazyLock::new(Default::default);
/// Lines the background drafts leave for the daemon's next pass.
static LINES: LazyLock<parking_lot::Mutex<Vec<String>>> = LazyLock::new(Default::default);

/// The model a draft is written with: the compaction model when one is set
/// (the harness's own side-call model), else the worker model, else the
/// light model.
pub(crate) fn draft_model(s: &hive_store::Store) -> String {
    let compact = s.setting("harness.compact_model", "").trim().to_string();
    if !compact.is_empty() {
        return compact;
    }
    let worker = s.setting("worker_model", "").trim().to_string();
    if !worker.is_empty() {
        return worker;
    }
    s.light_model()
}

/// The model the call names: `model`, or - when no model setting is set,
/// as on a default install - the provider's default chat model, the way
/// Ask, the harness and the decide role resolve it. An empty name would
/// reach the provider as no model at all.
pub(crate) fn resolved_model(
    p: &dyn hive_provider::Provider,
    cancel: &hive_provider::CancelToken,
    model: &str,
) -> Result<String> {
    let model = model.trim();
    if !model.is_empty() {
        return Ok(model.to_string());
    }
    let models = p.models(cancel)?;
    models
        .iter()
        .find(|m| m.default && m.chat)
        .or_else(|| models.iter().find(|m| m.chat))
        .map(|m| m.id.clone())
        .ok_or_else(|| {
            anyhow!(
                "no model is set and {} lists no chat model; set worker_model or light_model",
                p.name()
            )
        })
}

pub(crate) fn run_draft_call(
    s: &hive_store::Store,
    model: &str,
    system: &str,
    user: &str,
) -> Result<String> {
    let p = crate::login_gate::local_provider(
        s.home.clone(),
        s.setting("provider", hive_provider::DEFAULT),
        s.provider_settings(),
    )?;
    let cancel = hive_provider::CancelToken::new().with_timeout(DRAFT_TIMEOUT);
    let model = resolved_model(p.as_ref(), &cancel, model)?;
    let resp = p.chat(
        &cancel,
        hive_provider::Request {
            model,
            messages: vec![
                hive_provider::Message {
                    content: system.to_string(),
                    ..hive_provider::Message::new(hive_provider::Role::System)
                },
                hive_provider::Message {
                    content: user.to_string(),
                    ..hive_provider::Message::new(hive_provider::Role::User)
                },
            ],
            tools: Vec::new(),
            max_tokens: DRAFT_TOKENS,
            initiator: "agent".into(),
            ..Default::default()
        },
    )?;
    Ok(resp
        .message
        .map(|m| m.content)
        .unwrap_or_default()
        .trim()
        .to_string())
}

fn gist(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    crate::util::trunc(&line, 180)
}

/// Drafts `slug`'s goal and posts it as a proposal when it is new. The
/// goal itself never changes here.
pub(crate) fn propose(s: &hive_store::Store, slug: &str, trigger: &str) -> Result<Outcome> {
    let ev = draft::evidence(s, slug)?;
    let model = draft_model(s);
    let call = DRAFT_CALL.read().expect("draft call lock").clone();
    let reply = call(s, &model, draft::SYSTEM, &draft::prompt(&ev))
        .map_err(|e| anyhow!("the model did not draft a goal: {}", failure(e)))?;
    if draft::keeps_goal(&reply) {
        let out = Outcome {
            project: slug.to_string(),
            trigger: trigger.to_string(),
            status: draft::KEPT.into(),
            model,
            ..Default::default()
        };
        record(s, slug, trigger, &out);
        return Ok(out);
    }
    let d = draft::check_reply(s, &reply, trigger)?;
    let mut out = Outcome {
        project: slug.to_string(),
        trigger: trigger.to_string(),
        text: d.text.clone(),
        why: d.why.clone(),
        model: model.clone(),
        ..Default::default()
    };
    match draft::standing(s, slug, &d.text)? {
        Standing::SameAsGoal => out.status = draft::SAME_AS_GOAL.into(),
        Standing::Waiting(post) => {
            out.status = draft::ALREADY_PROPOSED.into();
            out.post = post;
        }
        Standing::Dismissed(post) => {
            out.status = draft::DISMISSED_BEFORE.into();
            out.post = post;
        }
        Standing::New => {
            let now = s.now();
            let mut extra = std::collections::BTreeMap::new();
            extra.insert("goal_draft".to_string(), serde_json::json!(trigger));
            if !model.is_empty() {
                extra.insert("model".to_string(), serde_json::json!(model));
            }
            let post = crate::board::create_post(
                s,
                now.clone(),
                crate::board::PostOpts {
                    kind: "opinion".into(),
                    project: slug.to_string(),
                    area: hive_goals::review::AREA.into(),
                    thread: hive_goals::review::AMEND_THREAD.into(),
                    summary: format!("proposed goal: {}", gist(&d.text)),
                    body: d.body.clone(),
                    agent: crate::board::actor_agent(s, draft::DRAFTER.to_string()),
                    date: now.clone(),
                    extra,
                    ..Default::default()
                },
            )?;
            out.status = draft::PROPOSED.into();
            out.post = post.str("id");
        }
    }
    record(s, slug, trigger, &out);
    Ok(out)
}

/// The `goal.draft` event of one outcome. A store that refuses the write
/// loses the event, never the drafting thread: `crate::util::event_at`
/// unwraps, and a panic there left the project marked in flight until the
/// daemon restarted.
fn record(s: &hive_store::Store, slug: &str, trigger: &str, out: &Outcome) {
    let detail = serde_json::json!({"trigger": trigger, "status": out.status, "post": out.post});
    if let Err(e) = s.event(draft::DRAFTER, "goal.draft", slug, detail) {
        LINES.lock().push(format!(
            "goal: could not record {slug}'s draft: {}",
            failure(e)
        ));
    }
}

/// Takes a project's in-flight mark off when its drafting thread ends,
/// however it ends, as `decide`'s `PassEnd` does: a panic in the draft must
/// not keep the project from being drafted again until the daemon restarts.
struct Inflight(String);

impl Drop for Inflight {
    fn drop(&mut self) {
        INFLIGHT.lock().remove(&self.0);
    }
}

fn failure(err: anyhow::Error) -> String {
    let mut text = crate::run::err_message(&err);
    if let Some(ce) = err.downcast_ref::<hive_cli::Error>() {
        text = ce.what.clone();
        if !ce.why.is_empty() {
            text.push_str(": ");
            text.push_str(&ce.why);
        }
    }
    crate::executor::cut(
        &crate::compute::redact_secrets(&crate::executor::fault_line(&text, 200)),
        200,
    )
}

/// Runs one draft for the daemon and keeps what it found in `meta`: the
/// signals it answered (so the same ADR, owner line or review flag does not
/// fire again) and the outcome, or the failure to wait out.
pub(crate) fn recheck(
    s: &hive_store::Store,
    slug: &str,
    trigger: &str,
    sig: &draft::Signals,
    now: &str,
) -> String {
    let before = draft::load_state(s, slug).unwrap_or_default();
    match propose(s, slug, trigger) {
        Ok(out) => {
            let state = RecheckState {
                checked_at: now.to_string(),
                trigger: trigger.to_string(),
                outcome: out.status.clone(),
                post: out.post.clone(),
                adr_seen: sig.adr_latest.clone(),
                owner_seen: sig.owner_latest.clone(),
                flags: sig.flags.clone(),
                ..Default::default()
            };
            let _ = draft::save_state(s, slug, &state);
            out.line()
        }
        Err(e) => {
            let error = failure(e);
            let state = RecheckState {
                failed_at: now.to_string(),
                error: error.clone(),
                ..before
            };
            let _ = draft::save_state(s, slug, &state);
            format!(
                "goal: drafting {slug}'s goal failed ({trigger}): {error}; the goal in force stays"
            )
        }
    }
}

/// The daemon's full pass: for each active project whose goal is due a
/// re-draft (`goal.recheck_days`, an accepted ADR, the owner's direction, a
/// review flag, or the first draft once onboarding is over), starts one
/// draft in the background. Returns the lines the drafts finished since the
/// last pass.
pub(crate) fn tend_goal_drafts(s: &hive_store::Store, now: String) -> Vec<String> {
    let mut lines = std::mem::take(&mut *LINES.lock());
    let days = s.setting_int(draft::RECHECK_DAYS_SETTING);
    if days <= 0 {
        return lines;
    }
    for slug in due_projects(s, &now, days) {
        let (trigger, sig) = slug.1;
        let slug = slug.0;
        if !INFLIGHT.lock().insert(slug.clone()) {
            continue;
        }
        let s = s.clone();
        let now = now.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("goal-draft-{slug}"))
            .spawn({
                let slug = slug.clone();
                move || {
                    let _inflight = Inflight(slug.clone());
                    let line = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        recheck(&s, &slug, &trigger, &sig, &now)
                    }))
                    .unwrap_or_else(|_| {
                        format!("goal: drafting {slug}'s goal stopped on a fault; the next pass tries again")
                    });
                    LINES.lock().push(line);
                }
            });
        if spawned.is_err() {
            INFLIGHT.lock().remove(&slug);
            lines.push(format!("goal: could not start drafting {slug}'s goal"));
        }
    }
    lines
}

/// The active projects due a re-draft, with the trigger and the signals. A
/// project whose proposed goal still waits for the owner is not: a second
/// draft would pile up beside it, and the owner's answer to the first is
/// what the next draft should start from.
pub(crate) fn due_projects(
    s: &hive_store::Store,
    now: &str,
    days: i64,
) -> Vec<(String, (String, draft::Signals))> {
    let Ok(conn) = s.conn() else {
        return Vec::new();
    };
    let slugs: Vec<String> = crate::util::must_rows(
        &conn,
        "SELECT slug FROM projects WHERE status = 'active' AND slug != ? ORDER BY slug",
        &[&hive_store::COMMONS_SLUG],
    )
    .into_iter()
    .map(|r| r.str("slug"))
    .collect();
    drop(conn);
    slugs
        .into_iter()
        .filter_map(|slug| {
            let state = draft::load_state(s, &slug);
            if draft::resting(state.as_ref(), now, days) {
                return None;
            }
            if hive_goals::review::pending(s, &slug).is_some() {
                return None;
            }
            let sig = draft::signals(s, &slug);
            draft::trigger(state.as_ref(), &sig, now, days).map(|t| (slug, (t, sig)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::*;
    use crate::test_helpers_daemon::*;
    use std::sync::Mutex;

    const GOAL_A: &str = "Make the hosted brain the one place where teams, agents and machines plan and land work together, with isolation, quotas and roles per team, measured by teams onboarded and landings per week.";
    const GOAL_B: &str = "Let every member reach the team's brain from the web console and confirm new capability there, so no machine needs its own commands; measured by console confirmations and fewer shell steps.";

    fn reply(goal: &str) -> String {
        format!("GOAL:\n{goal}\nWHY:\nThe README and the accepted decisions point there.")
    }

    struct Fake {
        prompts: Arc<Mutex<Vec<String>>>,
        prev: Arc<DraftCallFn>,
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            set_draft_call(self.prev.clone());
        }
    }

    fn fake(answer: Arc<Mutex<String>>) -> Fake {
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let seen = prompts.clone();
        let prev = set_draft_call(Arc::new(move |_s, _m, _sys, user| {
            seen.lock().unwrap().push(user.to_string());
            let a = answer.lock().unwrap().clone();
            if a.is_empty() {
                anyhow::bail!("not signed in");
            }
            Ok(a)
        }));
        Fake { prompts, prev }
    }

    fn goal_store() -> TestStore {
        let s = store_with_projects(&["shop"]);
        s.conn()
            .unwrap()
            .execute(
                "UPDATE projects SET path = ? WHERE slug = 'shop'",
                [s.home.clone()],
            )
            .unwrap();
        std::fs::write(
            std::path::Path::new(&s.home).join("README.md"),
            "# Shop\n\nShop sells handmade goods to returning customers.\n",
        )
        .unwrap();
        s
    }

    fn proposals(s: &hive_store::Store) -> Vec<(String, String)> {
        let conn = s.conn().unwrap();
        let mut st = conn
            .prepare("SELECT id, body FROM posts WHERE project = 'shop' AND area = 'goal' AND thread = 'amend' ORDER BY id")
            .unwrap();
        st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .unwrap()
            .flatten()
            .filter(|(_, body)| hive_goals::review::is_proposal(body))
            .collect()
    }

    #[test]
    fn a_draft_is_a_proposal_post_and_the_owners_yes_sets_it() {
        let _g = serial();
        let s = goal_store();
        let answer = Arc::new(Mutex::new(reply(GOAL_A)));
        let f = fake(answer);
        let before = hive_goals::get(&s, "shop").unwrap();
        let out = propose(&s, "shop", "manual").unwrap();
        assert_eq!(out.status, draft::PROPOSED, "{out:?}");
        // The evidence reached the model.
        assert!(f.prompts.lock().unwrap()[0].contains("handmade goods"));
        // A proposal post, not a goal change.
        let after = hive_goals::get(&s, "shop").unwrap();
        assert_eq!(after, before);
        let posts = proposals(&s);
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].0, out.post);
        let (text, why) = hive_goals::review::parse_proposal(&posts[0].1).unwrap();
        assert_eq!(text, GOAL_A);
        assert!(why.contains("README"), "{why}");
        let review = hive_goals::review::review(&s, "shop").unwrap();
        assert_eq!(review.proposal.unwrap().post, out.post);
        // The owner's yes sets it.
        hive_board::decide(&s, &out.post, "yes", "", hive_store::HUMAN_AGENT).unwrap();
        let goal = hive_goals::get(&s, "shop").unwrap();
        assert_eq!(goal.text, GOAL_A);
        assert_eq!(goal.source, hive_goals::OWNER);
        // Drafting the same goal again proposes nothing: it is in force.
        let again = propose(&s, "shop", "manual").unwrap();
        assert_eq!(again.status, draft::SAME_AS_GOAL);
        assert_eq!(proposals(&s).len(), 1);
    }

    #[test]
    fn a_dismissed_draft_is_not_proposed_again_but_a_different_one_is() {
        let _g = serial();
        let s = goal_store();
        let answer = Arc::new(Mutex::new(reply(GOAL_A)));
        let _f = fake(answer.clone());
        let first = propose(&s, "shop", "manual").unwrap();
        assert_eq!(first.status, draft::PROPOSED);
        // While it waits, the same draft is not posted twice.
        let waiting = propose(&s, "shop", "cadence").unwrap();
        assert_eq!(waiting.status, draft::ALREADY_PROPOSED);
        assert_eq!(waiting.post, first.post);
        hive_board::decide(&s, &first.post, "no", "not this", hive_store::HUMAN_AGENT).unwrap();
        let goal = hive_goals::get(&s, "shop").unwrap();
        assert_eq!(goal.source, hive_goals::GENERATED);
        // The identical draft - and a reworded copy - stay dismissed.
        let same = propose(&s, "shop", "cadence").unwrap();
        assert_eq!(same.status, draft::DISMISSED_BEFORE);
        assert_eq!(same.post, first.post);
        *answer.lock().unwrap() = reply(&GOAL_A.replace("together,", "together;"));
        assert_eq!(
            propose(&s, "shop", "cadence").unwrap().status,
            draft::DISMISSED_BEFORE
        );
        assert_eq!(proposals(&s).len(), 1, "the dismissed proposal only");
        // A materially different draft is proposed again.
        *answer.lock().unwrap() = reply(GOAL_B);
        let other = propose(&s, "shop", "adr").unwrap();
        assert_eq!(other.status, draft::PROPOSED);
        assert_ne!(other.post, first.post);
        // The dismissed draft reached the model as one not to repeat.
        assert!(
            _f.prompts
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .contains("dismissed"),
            "the prompt names the dismissed drafts"
        );
    }

    #[test]
    fn a_draft_with_a_private_path_or_a_secret_is_rejected() {
        let _g = serial();
        let s = goal_store();
        let answer = Arc::new(Mutex::new(reply(
            "Ship the console; the plan lives in /Users/owner/notes/plan.md for now.",
        )));
        let _f = fake(answer.clone());
        let err = propose(&s, "shop", "manual").unwrap_err().to_string();
        assert!(err.contains("goal rules"), "{err}");
        *answer.lock().unwrap() =
            reply("Ship the console with the key ghp_abcdefghijklmnopqrstuvwxyz0123456789 set.");
        assert!(propose(&s, "shop", "manual").is_err());
        *answer.lock().unwrap() = reply(&"x".repeat(4001));
        assert!(propose(&s, "shop", "manual").is_err());
        assert!(proposals(&s).is_empty());
        assert_eq!(
            hive_goals::get(&s, "shop").unwrap().source,
            hive_goals::GENERATED
        );
    }

    #[test]
    fn the_daemon_rechecks_on_cadence_and_on_an_accepted_adr() {
        let _g = serial();
        let s = goal_store();
        let answer = Arc::new(Mutex::new(reply(GOAL_A)));
        let _f = fake(answer.clone());
        let now = s.now();
        // Onboarding still running: no draft yet.
        s.conn()
            .unwrap()
            .execute(
                "INSERT INTO jobs (id, project, kind, title, status, created_at) VALUES ('job-onb', 'shop', 'onboard', 'Onboard shop', 'queued', ?)",
                [&now],
            )
            .unwrap();
        assert!(due_projects(&s, &now, 14).is_empty());
        // Onboarding over: the first draft.
        s.conn()
            .unwrap()
            .execute("UPDATE jobs SET status = 'done' WHERE id = 'job-onb'", [])
            .unwrap();
        let due = due_projects(&s, &now, 14);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].1.0, "first");
        let line = recheck(&s, "shop", "first", &due[0].1.1, &now);
        assert!(line.contains("proposed"), "{line}");
        let state = draft::load_state(&s, "shop").unwrap();
        assert_eq!(state.checked_at, now);
        assert_eq!(state.outcome, draft::PROPOSED);
        // Inside the cadence nothing is due.
        let later = crate::util::plus_minutes(&now, 24.0 * 60.0);
        assert!(due_projects(&s, &later, 14).is_empty());
        // A newly accepted ADR is.
        s.conn()
            .unwrap()
            .execute(
                "INSERT INTO records (id, project, kind, number, slug, title, status, created_at, created_by, updated_at, origin) VALUES ('ADR-071', 'shop', 'adr', 71, 'teams', 'Teams on one brain', 'Accepted', ?, 'human', ?, 'store')",
                [&later, &later],
            )
            .unwrap();
        // TASK-192: while the first draft waits for the owner nothing is
        // drafted beside it; the owner's answer frees the project.
        assert!(due_projects(&s, &later, 14).is_empty());
        hive_board::decide(&s, &state.post, "no", "not yet", hive_store::HUMAN_AGENT).unwrap();
        let due = due_projects(&s, &later, 14);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].1.0, "adr");
        *answer.lock().unwrap() = reply(GOAL_B);
        let line = recheck(&s, "shop", "adr", &due[0].1.1, &later);
        assert!(line.contains("proposed"), "{line}");
        assert!(
            _f.prompts
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .contains("ADR-071 Teams on one brain")
        );
        // Seen once, the ADR does not fire again; the cadence does, once
        // the owner has answered the draft it brought.
        let waiting = hive_goals::review::pending(&s, "shop").unwrap();
        hive_board::decide(&s, &waiting.post, "no", "", hive_store::HUMAN_AGENT).unwrap();
        let next_day = crate::util::plus_minutes(&later, 24.0 * 60.0);
        assert!(due_projects(&s, &next_day, 14).is_empty());
        let fortnight = crate::util::plus_minutes(&later, 14.0 * 24.0 * 60.0);
        let due = due_projects(&s, &fortnight, 14);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].1.0, "cadence");
        // goal.recheck_days = 0 turns the daemon's drafting off.
        assert!(due_projects(&s, &fortnight, 0).is_empty());
    }

    #[test]
    fn without_a_model_the_template_stays_and_the_failure_waits() {
        let _g = serial();
        let s = goal_store();
        let answer = Arc::new(Mutex::new(String::new()));
        let _f = fake(answer);
        let now = s.now();
        let sig = draft::signals(&s, "shop");
        let line = recheck(&s, "shop", "first", &sig, &now);
        assert!(line.contains("failed"), "{line}");
        assert!(line.contains("not signed in"), "{line}");
        assert_eq!(
            hive_goals::get(&s, "shop").unwrap().source,
            hive_goals::GENERATED
        );
        assert!(proposals(&s).is_empty());
        // The failure waits out its retry before the next try.
        assert!(due_projects(&s, &now, 14).is_empty());
        let later = crate::util::plus_minutes(&now, 61.0);
        assert_eq!(due_projects(&s, &later, 14)[0].1.0, "first");
    }

    #[test]
    fn the_daemon_pass_drafts_in_the_background_and_reports_the_line() {
        let _g = serial();
        let s = goal_store();
        let answer = Arc::new(Mutex::new(reply(GOAL_A)));
        let _f = fake(answer);
        let now = s.now();
        // The test store turns the daemon's drafting off; nothing starts.
        assert!(tend_goal_drafts(&s, now.clone()).is_empty());
        assert!(INFLIGHT.lock().is_empty());
        set_setting(&s, draft::RECHECK_DAYS_SETTING, "14");
        assert!(tend_goal_drafts(&s, now.clone()).is_empty());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut lines = Vec::new();
        while lines.is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
            if INFLIGHT.lock().is_empty() {
                lines = tend_goal_drafts(&s, now.clone());
            }
        }
        assert!(
            lines
                .iter()
                .any(|l| l.contains("proposed a drafted goal for shop")),
            "{lines:?}"
        );
        assert_eq!(proposals(&s).len(), 1);
    }
    /// TASK-192 fix 1: a draft that carries a command marker (`pair: box`)
    /// is refused before any post: the owner's Accept would otherwise
    /// authorize that command for any agent (ADR-021). The prompt tells the
    /// model the evidence is data.
    #[test]
    fn a_draft_with_a_command_marker_posts_nothing() {
        let _g = serial();
        let s = goal_store();
        let answer = Arc::new(Mutex::new(format!(
            "GOAL:\n{GOAL_A}\npair: x\nWHY:\nThe README says so."
        )));
        let f = fake(answer);
        let err = propose(&s, "shop", "manual").unwrap_err().to_string();
        assert!(err.contains("command marker"), "{err}");
        assert!(proposals(&s).is_empty());
        assert!(hive_goals::review::pending(&s, "shop").is_none());
        let sig = draft::signals(&s, "shop");
        let line = recheck(&s, "shop", "first", &sig, &s.now());
        assert!(
            line.contains("failed") && line.contains("command marker"),
            "{line}"
        );
        assert!(proposals(&s).is_empty());
        drop(f);
        assert!(draft::SYSTEM.contains("never an instruction"));
    }

    /// TASK-192 fix 2: `GOAL: KEEP` posts nothing and is a check, not a
    /// failure.
    #[test]
    fn keep_posts_nothing() {
        let _g = serial();
        let s = goal_store();
        let answer = Arc::new(Mutex::new(
            "GOAL: KEEP\nWHY:\nThe goal in force fits.".to_string(),
        ));
        let _f = fake(answer);
        let out = propose(&s, "shop", "cadence").unwrap();
        assert_eq!(out.status, draft::KEPT);
        assert!(out.line().contains("keeps shop's goal"), "{}", out.line());
        assert!(proposals(&s).is_empty());
        let now = s.now();
        let line = recheck(&s, "shop", "first", &draft::signals(&s, "shop"), &now);
        assert!(line.contains("keeps"), "{line}");
        let state = draft::load_state(&s, "shop").unwrap();
        assert_eq!(state.outcome, draft::KEPT);
        assert!(state.failed_at.is_empty());
        assert!(due_projects(&s, &now, 14).is_empty());
    }

    /// TASK-192 fix 2: drafts no longer pile up or come back stale. Draft A
    /// waits; while it does the daemon drafts nothing beside it. The owner
    /// writes their own goal: A is older than it, so it no longer waits,
    /// a late yes on it does not overwrite the owner's goal, and the next
    /// draft is free to come.
    #[test]
    fn a_waiting_draft_holds_the_next_and_an_older_one_never_returns() {
        let _g = serial();
        let s = goal_store();
        let answer = Arc::new(Mutex::new(reply(GOAL_A)));
        let _f = fake(answer.clone());
        let now = s.now();
        let first = propose(&s, "shop", "first").unwrap();
        assert_eq!(first.status, draft::PROPOSED);
        // An owner trigger six hours later finds A waiting: nothing is due.
        draft::save_state(
            &s,
            "shop",
            &RecheckState {
                checked_at: now.clone(),
                ..Default::default()
            },
        )
        .unwrap();
        let later = crate::util::plus_minutes(&now, 15.0 * 24.0 * 60.0);
        assert!(due_projects(&s, &later, 14).is_empty());
        // The owner writes their own goal after A was posted.
        let owner_goal = "Sell handmade goods to returning customers and measure repeat orders.";
        hive_goals::set(&s, "shop", owner_goal, hive_store::HUMAN_AGENT).unwrap();
        // The test clock stands still; the owner's write is a minute later.
        s.conn()
            .unwrap()
            .execute(
                "UPDATE settings SET updated_at = ? WHERE name = 'project.goal.shop'",
                [crate::util::plus_minutes(&now, 1.0)],
            )
            .unwrap();
        assert!(hive_goals::review::pending(&s, "shop").is_none());
        // A late yes on A keeps the owner's goal.
        hive_board::decide(&s, &first.post, "yes", "", hive_store::HUMAN_AGENT).unwrap();
        assert_eq!(hive_goals::get(&s, "shop").unwrap().text, owner_goal);
        // The project is due again on its cadence.
        assert_eq!(due_projects(&s, &later, 14).len(), 1);
    }

    struct Models(Vec<hive_provider::Model>);

    impl hive_provider::Provider for Models {
        fn name(&self) -> &str {
            "models"
        }
        fn label(&self) -> &str {
            "Models"
        }
        fn chat(
            &self,
            _: &hive_provider::CancelToken,
            _: hive_provider::Request,
        ) -> Result<hive_provider::Response> {
            anyhow::bail!("not used")
        }
        fn models(&self, _: &hive_provider::CancelToken) -> Result<Vec<hive_provider::Model>> {
            Ok(self.0.clone())
        }
        fn status(&self, _: &hive_provider::CancelToken) -> Result<hive_provider::Login> {
            Ok(Default::default())
        }
    }

    /// TASK-192: a default install sets no model; the draft names the
    /// provider's default chat model instead of an empty one.
    #[test]
    fn an_empty_model_name_resolves_to_the_providers_default() {
        let m = |id: &str, chat: bool, default: bool| hive_provider::Model {
            id: id.into(),
            name: id.into(),
            vendor: String::new(),
            chat,
            tool_calls: false,
            vision: false,
            premium: 0.0,
            preview: false,
            context_window: 0,
            max_output: 0,
            default,
        };
        let cancel = hive_provider::CancelToken::new();
        let p = Models(vec![
            m("embed", false, true),
            m("small", true, false),
            m("big", true, true),
        ]);
        assert_eq!(resolved_model(&p, &cancel, "  ").unwrap(), "big");
        assert_eq!(resolved_model(&p, &cancel, "set-one").unwrap(), "set-one");
        let p = Models(vec![m("embed", false, true), m("small", true, false)]);
        assert_eq!(resolved_model(&p, &cancel, "").unwrap(), "small");
        let err = resolved_model(&Models(vec![]), &cancel, "")
            .unwrap_err()
            .to_string();
        assert!(err.contains("no chat model"), "{err}");
    }

    /// TASK-192: a fault inside a drafting thread is said once and never
    /// leaves the project marked in flight until the daemon restarts.
    #[test]
    fn a_panicking_draft_frees_the_project() {
        let _g = serial();
        let s = goal_store();
        let prev = set_draft_call(Arc::new(|_, _, _, _| panic!("draft fault")));
        set_setting(&s, draft::RECHECK_DAYS_SETTING, "14");
        let now = s.now();
        let _ = tend_goal_drafts(&s, now.clone());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut lines = Vec::new();
        while std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
            if INFLIGHT.lock().is_empty() {
                lines.extend(std::mem::take(&mut *LINES.lock()));
                if !lines.is_empty() {
                    break;
                }
            }
        }
        set_draft_call(prev);
        assert!(INFLIGHT.lock().is_empty());
        assert!(
            lines.iter().any(|l| l.contains("stopped on a fault")),
            "{lines:?}"
        );
    }
}
