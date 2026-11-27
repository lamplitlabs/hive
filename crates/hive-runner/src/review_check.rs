//! The review's check, run by the runner (`hive review check N`), and the
//! merge gate that reads it.
//!
//! The brief always said a red check is never merged, but only the brief
//! said it: `hive review merge N` refused forge-reported failures only, and
//! a project whose forge runs no check had nothing between the reviewer's
//! word and the merge. Pull request 296 was merged with a red check that
//! way (TASK-147). Now the check goes through the runner: `hive review
//! check N` runs the project's own check in the review's clone, on the pull
//! request's head, and records `review.check` with the head it ran on, its
//! exit and its log. A reviewer's `hive review merge N` refuses unless the
//! newest check of this review is green and ran on the head the forge would
//! merge ([`merge_gate`]); the owner's `--by owner` keeps its meaning and
//! skips the gate.
//!
//! A red check whose failing tests all pass when re-run alone
//! (`review.rerun_cmd`) is a flaky test, and is still not merged: one flaky
//! test must not teach the reviewer that red is fine. The check records the
//! names (`flaky`) and the CLI queues the fix on the pull request's head
//! itself, as the review's verdict. A red check followed by a green one of
//! the same head is the same thing seen from the other side, and the gate
//! refuses that too.

use serde_json::{Value, json};
use std::time::Duration;

use crate::review::PrView;
use crate::util::Row;

/// The event a check of a pull request's head records.
pub(crate) const ACTION: &str = "review.check";
/// The setting naming the command that re-runs one failing test alone.
pub(crate) const RERUN_SETTING: &str = "review.rerun_cmd";
/// At most this many failing tests are re-run alone; a check with more
/// failures is plainly red.
const RERUN_CAP: usize = 5;

/// One recorded check of a pull request's head.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Recorded {
    pub head_sha: String,
    /// The check's exit code; `None` when it was stopped at its time bound.
    pub exit: Option<i32>,
    pub green: bool,
    /// The full output, kept under `<HIVE_HOME>/logs/review/`.
    pub log: String,
    pub cmd: String,
    /// The failing tests (or failure lines) of a red check.
    pub failures: Vec<String>,
    /// The failing tests that passed when re-run alone; non-empty only when
    /// every failing test did.
    pub flaky: Vec<String>,
    /// Why the check said nothing about the tree (a full disk, a verify lock
    /// that stayed held); empty when it ran.
    pub did_not_run: String,
}

impl Recorded {
    fn from_detail(v: &Value) -> Self {
        let list = |k: &str| -> Vec<String> {
            v[k].as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        Recorded {
            head_sha: v["head_sha"].as_str().unwrap_or_default().to_string(),
            exit: v["exit"].as_i64().map(|e| e as i32),
            green: v["green"].as_bool().unwrap_or(false),
            log: v["log"].as_str().unwrap_or_default().to_string(),
            cmd: v["cmd"].as_str().unwrap_or_default().to_string(),
            failures: list("failures"),
            flaky: list("flaky"),
            did_not_run: v["did_not_run"].as_str().unwrap_or_default().to_string(),
        }
    }

    /// A check that ran and failed (a stop at the time bound counts); a
    /// check that did not run is not red, it is nothing.
    pub(crate) fn red(&self) -> bool {
        !self.green && self.did_not_run.is_empty()
    }

    /// `exit 1`, `stopped at its time bound`.
    pub(crate) fn exit_words(&self) -> String {
        match self.exit {
            Some(c) => format!("exit {c}"),
            None => "stopped at its time bound".to_string(),
        }
    }

    /// The one line the CLI prints and the refusals quote.
    pub(crate) fn line(&self, number: i64) -> String {
        let at = short(&self.head_sha);
        let log = if self.log.is_empty() {
            String::new()
        } else {
            format!("; log {}", self.log)
        };
        if self.green {
            return format!("check of #{number} green at {at} (`{}`){log}", self.cmd);
        }
        if !self.did_not_run.is_empty() {
            return format!(
                "check of #{number} did not run at {at}: {}{log}",
                self.did_not_run
            );
        }
        let failed = if self.failures.is_empty() {
            String::new()
        } else {
            format!("; failed: {}", list_words(&self.failures))
        };
        let flaky = if self.flaky.is_empty() {
            String::new()
        } else {
            format!(
                "; {} passed when re-run alone (flaky)",
                list_words(&self.flaky)
            )
        };
        format!(
            "check of #{number} red at {at} ({}){failed}{flaky}{log}",
            self.exit_words()
        )
    }
}

fn short(sha: &str) -> &str {
    if sha.len() > 12 { &sha[..12] } else { sha }
}

fn list_words(items: &[String]) -> String {
    let shown: Vec<&str> = items.iter().take(5).map(String::as_str).collect();
    let more = if items.len() > 5 {
        format!(" +{} more", items.len() - 5)
    } else {
        String::new()
    };
    format!("{}{more}", shown.join(", "))
}

/// The failing tests a check printed: libtest's `test NAME ... FAILED`
/// lines and the indented names under a `failing tests:` heading
/// (scripts/verify-check.sh). The flag is true when a failure line named a
/// test this cannot re-run (a name with spaces): then no failure is called
/// flaky.
pub(crate) fn failed_tests(text: &str) -> (Vec<String>, bool) {
    let mut out: Vec<String> = Vec::new();
    let mut odd = false;
    let mut in_list = false;
    let valid = |n: &str| {
        !n.is_empty()
            && n.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
    };
    let push = |n: &str, out: &mut Vec<String>| {
        if !out.iter().any(|o| o == n) {
            out.push(n.to_string());
        }
    };
    for line in text.lines() {
        let t = line.trim();
        if let Some(name) = t
            .strip_prefix("test ")
            .and_then(|r| r.strip_suffix(" ... FAILED"))
        {
            in_list = false;
            if valid(name) {
                push(name, &mut out);
            } else {
                odd = true;
            }
            continue;
        }
        if t.contains("failing tests") && t.ends_with(':') {
            in_list = true;
            continue;
        }
        if in_list {
            if line.starts_with(' ') && valid(t) {
                push(t, &mut out);
            } else {
                in_list = false;
            }
        }
    }
    (out, odd)
}

/// The leading `env NAME=value ...` of the project's check, so a test re-run
/// alone builds where the check built (its CARGO_TARGET_DIR) instead of
/// compiling the workspace again elsewhere. Empty when the check has none,
/// or when it quotes anything there.
fn env_prefix(vc: &str) -> String {
    let mut words = vc.split_whitespace();
    if words.next() != Some("env") {
        return String::new();
    }
    let assigns: Vec<&str> = words
        .take_while(|w| !w.starts_with('-') && w.contains('='))
        .collect();
    if assigns.is_empty() || assigns.iter().any(|w| w.contains(['\'', '"', '`'])) {
        return String::new();
    }
    format!("env {} ", assigns.join(" "))
}

/// The command that re-runs `test` alone, or empty when the setting is
/// empty or names no `{test}`.
fn rerun_cmd(s: &hive_store::Store, vc: &str, test: &str) -> String {
    let tmpl = s.setting(RERUN_SETTING, "");
    if !tmpl.contains("{test}") {
        return String::new();
    }
    format!(
        "{}{}",
        env_prefix(vc),
        tmpl.replace("{test}", &crate::util::shell_quote(test))
    )
}

fn budget(s: &hive_store::Store) -> Duration {
    let minutes = crate::util::setting_int(s, "minutes_per_job").max(1);
    Duration::from_secs((minutes as u64) * 60).max(Duration::from_secs(60))
}

/// Re-runs each failing test alone in `dir`; true when every one passed
/// (its run exits 0 and says `test NAME ... ok`).
fn all_pass_alone(
    s: &hive_store::Store,
    slug: &str,
    dir: &str,
    vc: &str,
    tests: &[String],
) -> bool {
    if tests.is_empty() || tests.len() > RERUN_CAP {
        return false;
    }
    for t in tests {
        let cmd = rerun_cmd(s, vc, t);
        if cmd.is_empty() {
            return false;
        }
        let (rc, so, se) = crate::land::sh_with_timeout(
            dir.to_string(),
            crate::verify_lock::wrap(s, slug, &cmd),
            budget(s),
        );
        let ok_line = format!("test {t} ... ok");
        let passed = matches!(rc.as_deref(), Some(0))
            && format!("{so}\n{se}").lines().any(|l| l.trim() == ok_line);
        if !passed {
            return false;
        }
    }
    true
}

fn git_line(dir: &str, args: &[&str]) -> Result<String, String> {
    let out = crate::util::git_timeout(dir, Duration::from_secs(30), args);
    if out.ok() {
        Ok(out.stdout.trim().to_string())
    } else {
        Err(format!(
            "git {} failed in {dir}: {}",
            args.join(" "),
            out.stderr.trim()
        ))
    }
}

/// Runs the project's check in `dir` (the review's clone, on the pull
/// request's head after `hive review checkout`), keeps its log and records
/// `review.check`. Refused, with nothing recorded, when the project has no
/// check, when `dir` is not at the pull request's head, or when its tracked
/// files differ from that head: a check of anything else says nothing about
/// what would be merged.
pub(crate) fn run(
    s: &hive_store::Store,
    project: &Row,
    view: &PrView,
    dir: &str,
    actor: &str,
    now: &str,
) -> Result<Recorded, String> {
    let slug = project.str("slug");
    let number = view.number;
    let vc = project.str("verify_cmd").trim().to_string();
    if vc.is_empty() {
        return Err(format!(
            "{slug} has no check recorded (hive env show --project {slug}); without one no reviewer merge can be proved green, so hand #{number} to the owner"
        ));
    }
    let head = git_line(dir, &["rev-parse", "HEAD"])?;
    if !view.head_sha.is_empty() && head != view.head_sha {
        return Err(format!(
            "this clone is at {}, the head of #{number} is {}: run `hive review checkout {number}` first",
            short(&head),
            short(&view.head_sha)
        ));
    }
    let dirty = git_line(dir, &["status", "--porcelain", "--untracked-files=no"])?;
    if !dirty.is_empty() {
        return Err(format!(
            "tracked files in this clone differ from {} ({}): the check must run on the pull request's head as it is",
            short(&head),
            crate::util::trunc(&dirty.lines().collect::<Vec<_>>().join(", "), 160)
        ));
    }
    let (rc, so, se) = crate::land::sh_with_timeout(
        dir.to_string(),
        crate::verify_lock::wrap(s, &slug, &vc),
        budget(s),
    );
    let exit = rc.map(|c| *c);
    let folder = std::path::Path::new(&s.home).join("logs").join("review");
    let _ = std::fs::create_dir_all(&folder);
    let log = folder
        .join(format!(
            "{slug}-pr{number}-{}-{}.log",
            short(&head),
            crate::util::stamp_of(now)
        ))
        .display()
        .to_string();
    let text = format!(
        "$ {vc}\n# pull request #{number} at {head} in {dir}\n# exit: {}\n\n--- stdout ---\n{so}\n--- stderr ---\n{se}",
        exit.map(|c| c.to_string())
            .unwrap_or_else(|| "none (stopped at its time bound)".into())
    );
    let log = match std::fs::write(&log, text) {
        Ok(()) => log,
        Err(_) => String::new(),
    };
    let mut rec = Recorded {
        head_sha: head,
        exit,
        green: exit == Some(0),
        log,
        cmd: vc.clone(),
        ..Default::default()
    };
    if let Some(c) = exit
        && crate::verify_lock::did_not_run(c)
    {
        rec.did_not_run = crate::verify_lock::did_not_run_reason(c).to_string();
    } else if !rec.green {
        let full = format!("{so}\n{se}");
        let (tests, odd) = failed_tests(&full);
        rec.failures = if tests.is_empty() {
            crate::land::verify_failures(&crate::land::verify_tail(&so, &se))
        } else {
            tests.clone()
        };
        if exit.is_some() && !odd && all_pass_alone(s, &slug, dir, &vc, &tests) {
            rec.flaky = tests;
        }
    }
    let mut detail = json!({
        "project": slug, "number": number, "url": view.url, "head_sha": rec.head_sha,
        "exit": rec.exit, "green": rec.green, "log": rec.log, "cmd": rec.cmd,
        "failures": rec.failures, "flaky": rec.flaky, "note": rec.line(number),
    });
    if !rec.did_not_run.is_empty() {
        detail["did_not_run"] = json!(rec.did_not_run);
    }
    let job = std::env::var("HIVE_JOB").unwrap_or_default();
    if !job.is_empty() {
        detail["job"] = json!(job);
    }
    let conn = s.conn().map_err(|e| e.to_string())?;
    crate::util::event_at(&conn, now, actor, ACTION, &slug, detail);
    Ok(rec)
}

/// The checks recorded for the current review of a pull request, oldest
/// first: those after its newest `review.queued` (every one when no review
/// was queued, as for the owner at a terminal).
pub(crate) fn recorded(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    number: i64,
) -> Vec<Recorded> {
    let since = crate::util::query_rows(
        q,
        "SELECT seq FROM events WHERE subject = ? AND action = 'review.queued' AND json_extract(detail, '$.number') = ? ORDER BY seq DESC LIMIT 1",
        &[&slug, &number],
    )
    .unwrap_or_default()
    .first()
    .map(|r| r.int("seq") as i64)
    .unwrap_or(0);
    crate::util::query_rows(
        q,
        "SELECT detail FROM events WHERE subject = ? AND action = ? AND seq > ? AND json_extract(detail, '$.number') = ? ORDER BY seq",
        &[&slug, &ACTION, &since, &number],
    )
    .unwrap_or_default()
    .iter()
    .map(|r| Recorded::from_detail(&serde_json::from_str(&r.str("detail")).unwrap_or_default()))
    .collect()
}

/// Whether a reviewer may merge the pull request at `head_sha`: the newest
/// check of this review is green and ran on that head, and no red check of
/// that head came before it. The error says which check stands in the way
/// and where its log is.
pub(crate) fn merge_gate(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    number: i64,
    head_sha: &str,
) -> Result<Recorded, String> {
    let checks = recorded(q, slug, number);
    let Some(newest) = checks.last() else {
        return Err(format!(
            "no check of #{number} is recorded for this review: run `hive review check {number}` in the clone after `hive review checkout {number}`"
        ));
    };
    if !head_sha.is_empty() && newest.head_sha != head_sha {
        return Err(format!(
            "the newest check ran on {}, but the head of #{number} is {} now ({}): run `hive review checkout {number}` and `hive review check {number}` again",
            short(&newest.head_sha),
            short(head_sha),
            newest.line(number)
        ));
    }
    if !newest.green {
        return Err(format!(
            "{}; a red check is never merged",
            newest.line(number)
        ));
    }
    if let Some(red) = checks
        .iter()
        .rev()
        .skip(1)
        .find(|c| c.head_sha == newest.head_sha && c.red())
    {
        return Err(format!(
            "{} before the green one: a check that is red once and green again is flaky, and the reviewer does not merge it",
            red.line(number)
        ));
    }
    Ok(newest.clone())
}

/// The brief of the fix a flaky red check queues on the pull request's head.
pub(crate) fn flaky_note(rec: &Recorded, number: i64) -> String {
    format!(
        "The check of #{number} (`{}`) was red at {} ({}): {} failed in the full run and passed when re-run alone, so {} flaky. Make {} deterministic - find the shared state, timing, order or port {} depend on - and keep every assertion; do not skip or weaken a test. The pull request merges only on a green check of its head.{}",
        rec.cmd,
        short(&rec.head_sha),
        rec.exit_words(),
        rec.flaky.join(", "),
        if rec.flaky.len() == 1 {
            "it is"
        } else {
            "they are"
        },
        if rec.flaky.len() == 1 { "it" } else { "them" },
        if rec.flaky.len() == 1 {
            "it may"
        } else {
            "they may"
        },
        if rec.log.is_empty() {
            String::new()
        } else {
            format!(" Log of the red run: {}", rec.log)
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_tests_reads_libtest_lines_and_the_check_summary() {
        let text = "running 3 tests\ntest a::b ... ok\ntest a::c ... FAILED\n\nfailures:\n    a::c\n\nverify-check: failing tests:\n  a::c\n  d::e\nverify-check: 1 chunk(s) failed\n";
        assert_eq!(
            failed_tests(text),
            (vec!["a::c".to_string(), "d::e".to_string()], false)
        );
        let (names, odd) = failed_tests("test x - should panic ... FAILED\n");
        assert!(names.is_empty());
        assert!(odd);
        assert_eq!(failed_tests("error: could not compile\n"), (vec![], false));
    }

    #[test]
    fn env_prefix_keeps_the_checks_variables_only() {
        assert_eq!(
            env_prefix(
                "env CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$HOME/t} CARGO_INCREMENTAL=0 sh scripts/verify-check.sh"
            ),
            "env CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$HOME/t} CARGO_INCREMENTAL=0 "
        );
        assert_eq!(env_prefix("cargo test"), "");
        assert_eq!(env_prefix("env A='x y' sh c.sh"), "");
    }
}
