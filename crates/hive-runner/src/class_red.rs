//! A project's check red on the base tree of one class of machine (TASK-144).
//!
//! On 26 November two runs of the brain sat 2 h 37 on a rented Linux
//! machine where the project's check is red on the base: nothing could
//! land, and every code run of the brain on such a machine since 23
//! November had ended "base already red, nothing landed". The end check of
//! a run on a paired or rented machine now writes down which class of
//! machine (OS and architecture) found the base red ([`RED_EVENT`]), and the
//! lease keeps the project's code-changing jobs off every machine of that
//! class ([`lease_filter_sql`]) until a check there passes ([`GREEN_EVENT`])
//! or [`RED_HOURS`] pass with no run to say so. The job that fixes it still
//! goes: one whose needs name the class's OS was sent there on purpose.

use crate::util::{Querier, Row};
use serde_json::json;

/// The event a run on a remote machine writes when its end check found the
/// project's base tree red there (subject: the project; detail: `class`,
/// `executor`, `job`, `run`, `base`).
pub(crate) const RED_EVENT: &str = "class.base-red";
/// The event that says the project's check passed on a machine of the class
/// again (subject: the project; detail as [`RED_EVENT`]).
pub(crate) const GREEN_EVENT: &str = "class.base-green";
/// How long a red mark keeps code jobs off a class with no later run there
/// to clear it: long enough to stop the waste, short enough that a check
/// fixed by a landing elsewhere is tried there again within the day.
pub(crate) const RED_HOURS: f64 = 24.0;

/// The class of an executor row: its OS and architecture, as `linux/amd64`.
pub(crate) fn class_of(exrow: &Row) -> String {
    class_of_caps(&crate::executor::caps(exrow))
}

/// The class of a machine that offers `caps`.
pub(crate) fn class_of_caps(caps: &std::collections::BTreeSet<String>) -> String {
    let os = ["linux", "macos", "windows"]
        .into_iter()
        .find(|w| caps.contains(*w))
        .unwrap_or("");
    let arch = ["amd64", "arm64"]
        .into_iter()
        .find(|w| caps.contains(*w))
        .unwrap_or("");
    format!("{os}/{arch}")
}

#[allow(clippy::too_many_arguments)]
fn mark(
    q: &(impl Querier + ?Sized),
    now: &str,
    action: &str,
    project: &str,
    exrow: &Row,
    job: &str,
    run: &str,
    base: &str,
) {
    crate::util::event_at(
        q,
        now,
        "runner",
        action,
        project,
        json!({"class": class_of(exrow), "executor": exrow.str("name"), "job": job, "run": run, "base": base}),
    );
}

/// Writes down that `project`'s base tree is red on the class of `exrow`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn mark_red(
    q: &(impl Querier + ?Sized),
    now: &str,
    project: &str,
    exrow: &Row,
    job: &str,
    run: &str,
    base: &str,
) {
    mark(q, now, RED_EVENT, project, exrow, job, run, base);
}

/// Writes down that `project`'s check passed on the class of `exrow`, when
/// the class was marked red: one event per recovery, none on every green run.
pub(crate) fn mark_green(
    q: &(impl Querier + ?Sized),
    now: &str,
    project: &str,
    exrow: &Row,
    job: &str,
    run: &str,
    base: &str,
) {
    if latest(q, project, &class_of(exrow)).str("action") == RED_EVENT {
        mark(q, now, GREEN_EVENT, project, exrow, job, run, base);
    }
}

fn latest(q: &(impl Querier + ?Sized), project: &str, class: &str) -> Row {
    crate::util::must_one(
        q,
        "SELECT action, at FROM events WHERE action IN (?, ?) AND subject = ? AND json_valid(detail) AND json_extract(detail, '$.class') = ? ORDER BY seq DESC LIMIT 1",
        &[&RED_EVENT, &GREEN_EVENT, &project, &class],
    )
}

/// The projects whose base is red on `class` at `now`: the last mark is red
/// and younger than [`RED_HOURS`].
pub(crate) fn red_projects(q: &(impl Querier + ?Sized), class: &str, now: &str) -> Vec<String> {
    let since = crate::util::plus_hours(now, -RED_HOURS);
    crate::util::must_rows(
        q,
        "SELECT DISTINCT subject FROM events WHERE action = ? AND at >= ? AND json_valid(detail) AND json_extract(detail, '$.class') = ? ORDER BY subject",
        &[&RED_EVENT, &since, &class],
    )
    .into_iter()
    .map(|r| r.str("subject"))
    .filter(|p| latest(q, p, class).str("action") == RED_EVENT)
    .collect()
}

/// The lease condition that keeps the code-changing jobs of the projects red
/// on `exrow`'s class off it, empty when none is red there. A job whose
/// needs name the class's OS stays leasable: it was sent there on purpose,
/// such as the job that fixes the check on that system.
pub(crate) fn lease_filter_sql(q: &(impl Querier + ?Sized), exrow: &Row, now: &str) -> String {
    class_filter_sql(q, &class_of(exrow), now)
}

/// [`lease_filter_sql`] for a machine of `class`, such as the one a compute
/// request would rent.
pub(crate) fn class_filter_sql(q: &(impl Querier + ?Sized), class: &str, now: &str) -> String {
    let red = red_projects(q, class, now);
    if red.is_empty() {
        return String::new();
    }
    let quote = |v: &str| format!("'{}'", v.replace('\'', "''"));
    let projects = red.iter().map(|p| quote(p)).collect::<Vec<_>>().join(",");
    let kinds = hive_store::IMPLEMENTATION_JOB_KINDS
        .iter()
        .map(|k| quote(k))
        .collect::<Vec<_>>()
        .join(",");
    let os = class.split('/').next().unwrap_or_default();
    let pinned = if os.is_empty() {
        "0".to_string()
    } else {
        format!(
            "(',' || REPLACE(REPLACE(j.needs, ' ', ''), ';', ',') || ',') LIKE {}",
            quote(&format!("%,{os},%"))
        )
    };
    format!(" AND NOT (j.project IN ({projects}) AND j.kind IN ({kinds}) AND NOT {pinned})")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::TEST_NOW;

    fn hand(name: &str, arch: &str) -> Row {
        let mut r = Row::new();
        for (k, v) in [
            ("name", name),
            ("kind", "ssh"),
            ("os", "linux"),
            ("arch", arch),
            ("labels", ""),
        ] {
            r.0.insert(k.into(), json!(v));
        }
        r
    }

    #[test]
    fn a_red_base_on_a_class_is_cleared_by_a_green_check_or_by_time() {
        let s = crate::test_helpers::new_store();
        let conn = s.conn().unwrap();
        let a = hand("hands-a", "amd64");
        assert_eq!(class_of(&a), "linux/amd64");
        assert!(red_projects(&*conn, "linux/amd64", TEST_NOW).is_empty());
        mark_red(&*conn, TEST_NOW, "proj", &a, "job-1", "run-1", "abc");
        assert_eq!(red_projects(&*conn, "linux/amd64", TEST_NOW), vec!["proj"]);
        assert!(red_projects(&*conn, "linux/arm64", TEST_NOW).is_empty());
        let later = crate::util::plus_hours(TEST_NOW, RED_HOURS + 1.0);
        assert!(red_projects(&*conn, "linux/amd64", &later).is_empty());
        // A green check on another machine of the class clears it, once.
        mark_green(
            &*conn,
            TEST_NOW,
            "proj",
            &hand("hands-b", "amd64"),
            "job-2",
            "run-2",
            "def",
        );
        assert!(red_projects(&*conn, "linux/amd64", TEST_NOW).is_empty());
        mark_green(&*conn, TEST_NOW, "proj", &a, "job-3", "run-3", "def");
        assert_eq!(
            crate::test_helpers::count(
                &s,
                "SELECT COUNT(*) AS n FROM events WHERE action = ?",
                &[&GREEN_EVENT],
            ),
            1
        );
    }
}
