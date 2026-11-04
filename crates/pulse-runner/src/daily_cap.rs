//! The daily cap (`jobs_per_day`) tells the owner when it stops the fleet.
//!
//! On 2026-11-03 the owner set `jobs_per_day` to 10 from the console's
//! Settings page, beside two other capacity settings, when the fleet had
//! already run 174 times that day. Every slot idled with "daily cap
//! reached" in the daemon log - deduped to one line an hour - and nothing
//! else said so: the board was quiet, the console showed the queue, and the
//! owner spent the afternoon nudging tasks that could not start. Now the
//! daemon posts one owner question on the brain project's board the first
//! tick the cap holds queued work (`post_cap_reached`), and answers it the
//! tick the cap lifts - raised, set to 0, or midnight UTC
//! (`close_cap_notices`) - so the console's owner inbox carries the stop and
//! its cure. The other daemon-wide holds follow the same pattern through
//! the shared helpers in idle_hold.rs, which this module posts through.

use crate::util::Row;

/// The coalesce kind of the owner question and the area/thread it sits in.
pub(crate) const ASK_KIND: &str = "daily-cap";
const THREAD: &str = "daily-cap";
/// The event a post writes on the brain project, with the counts.
pub(crate) const EVENT: &str = "capacity.daily-cap";

/// Posts the owner question, once per open stop: the key coalesces on the
/// project and kind with numbers squashed, so a cap of 10 and 174 jobs
/// today and the same cap at 175 are one question. Returns the post made,
/// or an empty row when one is open already, the brain is unset or nothing
/// of the fleet's is queued.
pub(crate) fn post_cap_reached(
    s: &pulse_store::Store,
    brain: Row,
    done: i32,
    cap: i32,
    queued: i32,
    now: String,
    actor: String,
) -> Row {
    if brain.0.is_empty() || queued <= 0 || cap <= 0 {
        return Row::default();
    }
    let summary = format!(
        "daily cap reached: {done} of {cap} jobs today, {queued} job(s) wait until midnight UTC; raise jobs_per_day or set it to 0 for no cap"
    );
    let body = format!(
        "The fleet has started {done} distinct job(s) today, at or over the daily cap of {cap} (setting `jobs_per_day`), so {queued} queued job(s) wait until midnight UTC; only your own tasks, answers and lead sittings still start. \
The cap is the budget you set for the fleet's own work, and it stops every machine together. To let the work go on now: `pulse settings set jobs_per_day 0` (no cap) or a higher number, or Settings -> How much work runs in the console. \
This post closes itself when the cap lifts ({now}).\n"
    );
    crate::idle_hold::ask(
        s,
        crate::idle_hold::Ask {
            brain: &brain,
            ask_kind: ASK_KIND,
            thread: THREAD,
            summary,
            body,
            event: EVENT,
            detail: serde_json::json!({"done": done, "cap": cap, "queued": queued}),
            now: &now,
            actor: &actor,
        },
    )
}

/// Answers every open cap question of the brain project: the cap lifted.
/// Returns the posts answered.
pub(crate) fn close_cap_notices(
    s: &pulse_store::Store,
    brain: &Row,
    why: &str,
    now: &str,
    actor: &str,
) -> usize {
    crate::idle_hold::answer(
        s,
        brain,
        ASK_KIND,
        &format!("daily cap lifted: {why}"),
        &format!("The daily cap no longer holds the queue: {why}. The fleet's work starts again."),
        now,
        actor,
    )
}

/// What lifted the cap, for the answer: the setting now, against what the
/// question saw.
pub(crate) fn lift_reason(s: &pulse_store::Store, now: &str) -> String {
    let cap = crate::run::daily_cap(s);
    if cap <= 0 {
        return "jobs_per_day is 0 (no cap)".to_string();
    }
    let done = crate::run::runs_today(s, now);
    format!("{done} of {cap} jobs today (jobs_per_day)")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brain(s: &pulse_store::Store) -> Row {
        let conn = s.conn().unwrap();
        crate::util::exec1(
            &conn,
            "UPDATE projects SET brain = 1 WHERE slug = 'proj'",
            &[],
        );
        crate::board::project_by_slug(&conn, "proj".into())
    }

    #[test]
    fn the_cap_posts_one_question_and_answers_it_when_lifted() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        let b = brain(&s);
        let now = crate::test_helpers::TEST_NOW.to_string();
        let post = post_cap_reached(&s, b.clone(), 174, 10, 3, now.clone(), "daemon".into());
        assert_eq!(post.str("kind"), "question");
        assert!(
            post.str("summary")
                .starts_with("daily cap reached: 174 of 10 jobs today, 3 job(s) wait"),
            "{post:?}"
        );
        assert!(
            post.str("body")
                .contains("pulse settings set jobs_per_day 0")
        );
        // The same stop a tick later, one more job counted: no second post.
        let again = post_cap_reached(&s, b.clone(), 175, 10, 4, now.clone(), "daemon".into());
        assert!(again.0.is_empty(), "{again:?}");
        assert_eq!(
            crate::hygiene::open_daemon_questions_of_kind(&s, "proj", ASK_KIND).len(),
            1
        );
        assert_eq!(
            crate::test_helpers::count(
                &s,
                &format!("SELECT COUNT(*) AS n FROM events WHERE action = '{EVENT}'"),
                &[]
            ),
            1
        );
        // The owner lifts the cap: the question is answered and the thread
        // is quiet; a later stop asks again.
        s.set_setting(pulse_store::HUMAN_AGENT, "jobs_per_day", "0")
            .unwrap();
        let why = lift_reason(&s, &now);
        assert_eq!(why, "jobs_per_day is 0 (no cap)");
        assert_eq!(close_cap_notices(&s, &b, &why, &now, "daemon"), 1);
        assert!(crate::hygiene::open_daemon_questions_of_kind(&s, "proj", ASK_KIND).is_empty());
        assert_eq!(close_cap_notices(&s, &b, &why, &now, "daemon"), 0);
        let later = post_cap_reached(&s, b, 20, 20, 1, now, "daemon".into());
        assert_eq!(later.str("kind"), "question");
    }

    #[test]
    fn no_post_without_a_brain_or_queued_work() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        let now = crate::test_helpers::TEST_NOW.to_string();
        assert!(
            post_cap_reached(&s, Row::default(), 5, 5, 2, now.clone(), "daemon".into())
                .0
                .is_empty()
        );
        let b = brain(&s);
        assert!(
            post_cap_reached(&s, b.clone(), 5, 5, 0, now.clone(), "daemon".into())
                .0
                .is_empty()
        );
        assert!(
            post_cap_reached(&s, b, 5, 0, 2, now, "daemon".into())
                .0
                .is_empty()
        );
        assert!(crate::run::daily_cap_reason(5, 5).contains(crate::run::DAILY_CAP_REASON));
    }

    /// 2026-11-03 in one daemon: the cap is full of the fleet's runs, a
    /// fleet job and an owner task are queued. The owner's task runs past
    /// the cap, the fleet's job waits, the owner hears once on the board
    /// and in the log; lifting the cap answers the question and the fleet's
    /// job runs.
    #[test]
    fn a_full_cap_runs_the_owners_work_tells_the_owner_once_and_lifts() {
        let _serial = crate::test_helpers::serial();
        pulse_bus::reset_default();
        let s = crate::test_helpers::new_store();
        crate::test_helpers::new_bare_repo(&s);
        s.set_setting(pulse_store::HUMAN_AGENT, "daemon.min_free_mb", "0")
            .unwrap();
        s.set_setting(pulse_store::HUMAN_AGENT, "daemon.sweep", "off")
            .unwrap();
        s.set_setting(pulse_store::HUMAN_AGENT, "jobs_per_day", "2")
            .unwrap();
        crate::test_helpers::join(&s, "aurora");
        crate::test_helpers::join(&s, "borealis");
        let b = brain(&s);
        let now = s.now();
        let day = &now[..10];
        // Two distinct jobs ran today, one of them held and run again: the
        // cap of two is reached (three runs, two jobs).
        let conn = s.conn().unwrap();
        for (id, job, status, at) in [
            ("r1", "job-one", "done", format!("{day}T01:00:00Z")),
            ("r2", "job-two", "held", format!("{day}T02:00:00Z")),
            ("r3", "job-two", "done", format!("{day}T03:00:00Z")),
        ] {
            conn.execute(
                "INSERT INTO runs (id, job_id, agent, project, started_at, finished_at, status, mode) VALUES (?, ?, 'aurora', 'proj', ?, ?, ?, 'real')",
                rusqlite::params![id, job, at, at, status],
            )
            .unwrap();
        }
        drop(conn);
        assert_eq!(crate::run::runs_today(&s, &now), 2);
        let add = |kind: &str, source: &str| {
            crate::jobs::add_job(
                &s,
                "proj".into(),
                kind.into(),
                source.into(),
                "echo ok and report".into(),
                1,
                10,
                source.into(),
                pulse_store::HUMAN_AGENT.into(),
                false,
            )
            .unwrap()
            .0
            .str("id")
        };
        let fleet = add("improve", "suggestion:abc");
        let task = add("task", "task:TASK-055");
        let out = crate::test_helpers::OutBuf::new();
        let tick = |out: &crate::test_helpers::OutBuf| {
            let result = crate::daemon::daemon(
                &pulse_bus::CancellationToken::new(),
                &s,
                crate::daemon::DaemonOpts {
                    interval: 1,
                    max_loops: 1,
                    worker_cmd: "sh -c 'echo SUMMARY: all good'".into(),
                    out: Some(out.writer()),
                    ..Default::default()
                },
            );
            assert_eq!(result, "done", "daemon = {result}\n{}", out.string());
        };
        tick(&out);
        let status = |id: &str| crate::jobs::job_row(&*s.conn().unwrap(), id.into()).str("status");
        assert_eq!(
            status(&task),
            "done",
            "the owner's task past the cap\n{}",
            out.string()
        );
        assert_eq!(
            status(&fleet),
            "queued",
            "the fleet's job waits\n{}",
            out.string()
        );
        assert!(
            out.string()
                .contains("daily cap reached: 2 of 2 jobs today")
                && out.string().contains("set jobs_per_day higher or to 0"),
            "the log names the cap and its cure:\n{}",
            out.string()
        );
        let open = crate::hygiene::open_daemon_questions_of_kind(&s, "proj", ASK_KIND);
        assert_eq!(open.len(), 1, "{open:?}\n{}", out.string());
        assert!(
            open[0]
                .str("summary")
                .contains("1 job(s) wait until midnight UTC"),
            "{:?}",
            open[0]
        );
        // A second tick: still held, nothing new said to the owner.
        tick(&out);
        assert_eq!(
            crate::hygiene::open_daemon_questions_of_kind(&s, "proj", ASK_KIND).len(),
            1
        );
        assert_eq!(status(&fleet), "queued");
        // The owner lifts the cap: the question is answered, the fleet runs.
        s.set_setting(pulse_store::HUMAN_AGENT, "jobs_per_day", "0")
            .unwrap();
        out.reset();
        tick(&out);
        assert!(
            crate::hygiene::open_daemon_questions_of_kind(&s, "proj", ASK_KIND).is_empty(),
            "the cap question stays open after the cap lifted\n{}",
            out.string()
        );
        assert!(
            out.string()
                .contains("daily cap lifted: jobs_per_day is 0 (no cap)"),
            "{}",
            out.string()
        );
        assert_eq!(status(&fleet), "done", "{}", out.string());
        let _ = b;
    }
}
