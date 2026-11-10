//! A card proves itself (evolve.require_check). A job planned from a voted
//! card that names a check - `Signal: check:<command>` - lands only when the
//! check fails on the default branch and passes with the job's change: the
//! test the change adds or fixes is red on main and green on the branch. The
//! fixtures the experiments measure saw almost none of the changes the loop
//! made (76 of 85 experiments tied by 2026-11-08), so a card's own check is
//! the proof. Experiments are measured on both trees by the lane already
//! (pulse-evolve, the card-check fixture); this is the same proof for the
//! improve jobs the planner builds from cards, run in the job's clone after
//! its worker: the check as it stands, then the check in a scratch worktree
//! of the run's base.
use crate::exec::RunLog;

/// Why the job's change is not proven by its card's check, or None when it
/// is - or when there is nothing to prove here: the rule is off, the job is
/// no card's, or the card names no check (the planner holds those).
/// `verify` runs a shell command in the job's clone.
pub(crate) fn card_unproven(
    s: &pulse_store::Store,
    job: &crate::util::Row,
    base: &str,
    log: &RunLog,
    verify: &dyn Fn(&str) -> (Option<i32>, String),
) -> Option<String> {
    if !s.setting_bool(pulse_store::EVOLVE_REQUIRE_CHECK) || job.str("kind") == "experiment" {
        return None;
    }
    let id = job.str("source").strip_prefix("suggestion:")?.to_string();
    let post = pulse_tally::load_post(s, &id).ok()?;
    let check = pulse_tally::card_check_of(&post.body);
    if check.is_empty() {
        return None;
    }
    if pulse_store::secret_shaped(&check) || check.contains(['\n', '\0']) || base.is_empty() {
        return Some("the card's check cannot be run as written".into());
    }
    let (rc, out) = verify(&check);
    let base_cmd = format!(
        "d=$(mktemp -d) && git worktree add --detach --quiet \"$d\" {base} && (cd \"$d\" && {check}); rc=$?; git worktree remove --force \"$d\" >/dev/null 2>&1; exit $rc"
    );
    let (base_rc, base_out) = verify(&base_cmd);
    let verdict = proof(rc, &out, base_rc, &base_out);
    log.add(format!(
        "card check `{check}` (card {id}): on the default branch {}, with the change {}",
        side(base_rc, &base_out),
        side(rc, &out)
    ));
    match verdict {
        Proof::Proven => None,
        Proof::AlreadyGreen => Some(format!(
            "the card's check already passes on the default branch (`{check}`), so it shows nothing the change fixes"
        )),
        Proof::StillRed => Some(format!(
            "the card's check still fails with the change (`{check}`)"
        )),
        Proof::CountNotDown(b, n) => Some(format!(
            "the card's check counted {b} on the default branch and {n} with the change (`{check}`); the count must fall"
        )),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Proof {
    Proven,
    AlreadyGreen,
    StillRed,
    CountNotDown(u64, u64),
}

/// Red on the base and green with the change; a check that prints a count
/// proves the change when the count falls, as the measurement scores it.
fn proof(rc: Option<i32>, out: &str, base_rc: Option<i32>, base_out: &str) -> Proof {
    if let (Some(b), Some(n)) = (count(base_out), count(out)) {
        return if n < b {
            Proof::Proven
        } else {
            Proof::CountNotDown(b, n)
        };
    }
    if passed(base_rc, base_out) {
        Proof::AlreadyGreen
    } else if !passed(rc, out) {
        Proof::StillRed
    } else {
        Proof::Proven
    }
}

/// A check passes when it exits 0 and its test runner ran a test.
fn passed(rc: Option<i32>, out: &str) -> bool {
    rc == Some(0) && !pulse_evolve::ran_no_test(out)
}

/// Output that is one integer: a count.
fn count(out: &str) -> Option<u64> {
    out.trim().parse().ok()
}

fn side(rc: Option<i32>, out: &str) -> String {
    match (count(out), rc) {
        (Some(n), _) => format!("counted {n}"),
        (None, Some(0)) if pulse_evolve::ran_no_test(out) => "ran no test".into(),
        (None, Some(0)) => "passed".into(),
        (None, Some(code)) => format!("failed (exit {code})"),
        (None, None) => "did not finish".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_is_proven_by_a_check_red_on_the_base_and_green_with_it() {
        let ok = "test result: ok. 1 passed; 0 failed";
        let none = "running 0 tests\ntest result: ok. 0 passed; 0 failed";
        // The test the change adds: absent on the base (no test ran), green after.
        assert_eq!(proof(Some(0), ok, Some(0), none), Proof::Proven);
        // A test the change fixes: red on the base, green after.
        assert_eq!(proof(Some(0), ok, Some(101), "1 failed"), Proof::Proven);
        // Already green on the base: nothing to prove.
        assert_eq!(proof(Some(0), ok, Some(0), ok), Proof::AlreadyGreen);
        // Still red with the change.
        assert_eq!(proof(Some(1), "failed", Some(1), "failed"), Proof::StillRed);
        // Ran no test with the change either: still red.
        assert_eq!(proof(Some(0), none, Some(0), none), Proof::StillRed);
        // A check cut short counts as red.
        assert_eq!(proof(None, "", Some(1), "failed"), Proof::StillRed);
        // A count must fall.
        assert_eq!(proof(Some(0), "3\n", Some(0), "7\n"), Proof::Proven);
        assert_eq!(proof(Some(0), "7", Some(0), "7"), Proof::CountNotDown(7, 7));
        assert_eq!(side(Some(0), none), "ran no test");
        assert_eq!(side(Some(2), "x"), "failed (exit 2)");
        assert_eq!(side(Some(0), "4"), "counted 4");
    }

    #[test]
    fn only_a_card_job_with_a_check_is_held_to_its_check() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        // new_store pins the card-proof rule off for the rest of the suite;
        // this test is about the rule itself.
        s.set_setting("human", pulse_store::EVOLVE_REQUIRE_CHECK, "on")
            .unwrap();
        let conn = s.conn().unwrap();
        for (id, body) in [
            (
                "card-check",
                "Class: product\nFile: src/a.rs\nVariable: v\nSignal: check:cargo test -p a parses -- --exact\n",
            ),
            (
                "card-path",
                "Class: product\nFile: src/a.rs\nVariable: v\nSignal: src/a.rs\n",
            ),
        ] {
            conn.execute(
                "INSERT INTO posts (id, project, area, thread, kind, agent, date, summary, body, created_at) VALUES (?1, 'proj', 'a', ?1, 'suggestion', 'w', '2026-11-08T00:00:00Z', 's', ?2, '2026-11-08T00:00:00Z')",
                [id, body],
            )
            .unwrap();
        }
        drop(conn);
        let job = |kind: &str, source: &str| {
            let mut r = crate::util::Row::new();
            r.insert("kind", serde_json::json!(kind));
            r.insert("source", serde_json::json!(source));
            r
        };
        let log = RunLog::default();
        let ran = std::cell::RefCell::new(Vec::<String>::new());
        let verify = |cmd: &str| -> (Option<i32>, String) {
            ran.borrow_mut().push(cmd.to_string());
            if cmd.contains("git worktree add") {
                (
                    Some(0),
                    "running 0 tests\ntest result: ok. 0 passed; 0 failed".into(),
                )
            } else {
                (Some(0), "test result: ok. 1 passed; 0 failed".into())
            }
        };
        // Red on the base (no such test yet), green with the change: proven.
        assert_eq!(
            card_unproven(
                &s,
                &job("improve", "suggestion:card-check"),
                "abc123",
                &log,
                &verify
            ),
            None
        );
        assert_eq!(ran.borrow().len(), 2);
        assert!(ran.borrow()[1].contains("git worktree add --detach --quiet \"$d\" abc123"));
        assert!(
            log.text()
                .contains("on the default branch ran no test, with the change passed"),
            "{}",
            log.text()
        );
        // No check, no card, an experiment, or the rule off: nothing run.
        ran.borrow_mut().clear();
        assert_eq!(
            card_unproven(
                &s,
                &job("improve", "suggestion:card-path"),
                "abc123",
                &log,
                &verify
            ),
            None
        );
        assert_eq!(
            card_unproven(&s, &job("improve", "task:TASK-1"), "abc123", &log, &verify),
            None
        );
        assert_eq!(
            card_unproven(
                &s,
                &job("experiment", "suggestion:card-check"),
                "abc123",
                &log,
                &verify
            ),
            None
        );
        s.set_setting("human", pulse_store::EVOLVE_REQUIRE_CHECK, "off")
            .unwrap();
        assert_eq!(
            card_unproven(
                &s,
                &job("improve", "suggestion:card-check"),
                "abc123",
                &log,
                &verify
            ),
            None
        );
        assert!(ran.borrow().is_empty());
        // Already green on the base: unproven, and the reason says so.
        s.set_setting("human", pulse_store::EVOLVE_REQUIRE_CHECK, "on")
            .unwrap();
        let green = |_: &str| -> (Option<i32>, String) {
            (Some(0), "test result: ok. 1 passed; 0 failed".into())
        };
        let why = card_unproven(
            &s,
            &job("improve", "suggestion:card-check"),
            "abc123",
            &log,
            &green,
        )
        .unwrap();
        assert!(
            why.starts_with("the card's check already passes on the default branch"),
            "{why}"
        );
    }
}
