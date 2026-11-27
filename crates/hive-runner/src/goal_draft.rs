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
    let resp = p.chat(
        &cancel,
        hive_provider::Request {
            model: model.to_string(),
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
    if let Ok(conn) = s.conn() {
        crate::util::event_at(
            &conn,
            &s.now(),
            draft::DRAFTER,
            "goal.draft",
            slug,
            serde_json::json!({"trigger": trigger, "status": out.status, "post": out.post}),
        );
    }
    Ok(out)
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
                    let line = recheck(&s, &slug, &trigger, &sig, &now);
                    LINES.lock().push(line);
                    INFLIGHT.lock().remove(&slug);
                }
            });
        if spawned.is_err() {
            INFLIGHT.lock().remove(&slug);
            lines.push(format!("goal: could not start drafting {slug}'s goal"));
        }
    }
    lines
}

/// The active projects due a re-draft, with the trigger and the signals.
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
        // Seen once, the ADR does not fire again; the cadence does.
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
}
