//! A task's pull request merges every finished branch of the task onto its
//! base (`deliver_task`). Each branch passed its own check in its own clone,
//! but two of them can be green alone and broken together: one changes a
//! function's signature, the other adds a caller of the old one. Both merge
//! without a conflict, the task branch does not compile, and every job
//! later started on it - a rebase, a follow-up - fails its pre-commit
//! verify on code it never touched (`pulse/task-TASK-096` at 770cf7ea,
//! ADR-070 step 3 beside TASK-103, 2026-11-10).
//!
//! So when the pass merged two or more sides into the task branch of a
//! Cargo project, it compiles the merged tree (`cargo check --workspace
//! --all-targets`, [`check_merged_tree`]) before anything is pushed. A tree
//! that does not compile is work for an agent first, like a conflict: a
//! `task` job on the same task merges the branches and makes them compile
//! together, the merged jobs keep their results but lose their branches,
//! and `land.merge-broken` says so ([`fix_or_hold`]). A second broken
//! merge of the same task is held for the owner. `landing.merge_check` off
//! skips the check. A check that cannot run - no free build lane and no
//! cargo, a registry that cannot be reached, the time budget spent - never
//! holds the work: only cargo's own "could not compile" does.

use super::*;

/// The setting that turns the check off.
pub(crate) const SETTING: &str = "landing.merge_check";

/// How long the check may take. A build lane warm from the project's jobs
/// checks this workspace in under a minute; a cold one takes a few.
const BUDGET: Duration = Duration::from_secs(20 * 60);

/// The command, as it runs in the merged worktree.
pub(super) const COMMAND: &[&str] = &[
    "cargo",
    "check",
    "--workspace",
    "--all-targets",
    "--message-format=short",
];

/// How many fix jobs a task's broken merge gets before the owner is asked.
const FIXES_PER_TASK: usize = 1;

/// Compiler lines kept for the brief, the event and the note.
const ERROR_LINES: usize = 20;

/// What the check said about a merged tree.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Checked {
    /// Not run, or it could not say: why, for the log (empty when there is
    /// nothing worth saying, as for a project that is not a Cargo one).
    Skipped(String),
    Passed,
    /// Cargo could not compile it: its error lines.
    Broken(Vec<String>),
}

/// Compiles the merged tree in `worktree` when the project wants it and the
/// tree is a Cargo workspace, building into a lane of the project's shared
/// build cache when one is free so the dependencies stay compiled.
pub(super) fn check_merged_tree(
    s: &hive_store::Store,
    slug: &str,
    root: &str,
    def: &str,
    worktree: &str,
) -> Checked {
    if s.setting(SETTING, "on") == "off" {
        // The owner's choice is not news on every delivery line.
        return Checked::Skipped(String::new());
    }
    if !crate::buildcache::is_cargo_project(std::path::Path::new(worktree)) {
        return Checked::Skipped(String::new());
    }
    let mut env = crate::proc::environ();
    // The default branch's recipe pins the toolchain and sets the build
    // variables; a branch does not choose what checks it.
    let recipe = crate::recipe::read_recipe(s, slug, root, def)
        .0
        .unwrap_or_default();
    let mut argv = crate::recipe::mise_exec_argv(&recipe.tools);
    if !argv.is_empty() {
        env.extend(
            crate::recipe::mise_env(worktree)
                .into_iter()
                .map(|[k, v]| format!("{k}={v}")),
        );
    }
    env.extend(recipe.env.iter().map(|(k, v)| format!("{k}={v}")));
    // The console's bundle is not what this check is about, and building
    // it would take npm and minutes.
    env.push("HIVE_HUB_SKIP_BUILD=1".into());
    let lane = if recipe.env.contains_key("CARGO_TARGET_DIR") {
        None
    } else {
        crate::buildcache::acquire(std::path::Path::new(&s.home), slug)
            .ok()
            .flatten()
    };
    if let Some(lane) = &lane {
        env.push(format!("CARGO_TARGET_DIR={}", lane.dir.display()));
    }
    argv.extend(COMMAND.iter().map(|a| a.to_string()));
    let out = crate::proc::run_group(
        &hive_bus::CancellationToken::new(),
        &argv,
        worktree,
        &env,
        &[],
        BUDGET,
    );
    drop(lane);
    let out = match out {
        Ok(out) => out,
        Err(e) => return Checked::Skipped(format!("{} could not start: {e}", COMMAND.join(" "))),
    };
    if out.timeout {
        return Checked::Skipped(format!(
            "{} ran past {} minutes",
            COMMAND.join(" "),
            BUDGET.as_secs() / 60
        ));
    }
    if out.exit_code == 0 {
        return Checked::Passed;
    }
    let text = format!("{}\n{}", out.stdout, out.stderr);
    if !text.contains("could not compile") {
        return Checked::Skipped(format!(
            "{} failed before compiling anything (exit {}): {}",
            COMMAND.join(" "),
            out.exit_code,
            crate::run::squeeze(&text, 200)
        ));
    }
    Checked::Broken(error_lines(&text))
}

/// Cargo's error lines (`path:line:col: error[E…]: …` with
/// `--message-format=short`, then `error: could not compile …`), in order,
/// each once, at most [`ERROR_LINES`].
pub(super) fn error_lines(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines().map(str::trim_end) {
        let error = line.starts_with("error") || line.contains(": error");
        if error && !line.trim().is_empty() && !out.iter().any(|l| l == line) {
            out.push(line.to_string());
        }
        if out.len() == ERROR_LINES {
            break;
        }
    }
    out
}

/// A task branch whose merged tree does not compile.
#[derive(Debug)]
pub(super) struct Broken {
    /// The job branches merged this pass, as (job id, branch, sha).
    pub(super) merged: Vec<(String, String, String)>,
    /// The merged commit that failed.
    pub(super) tip: String,
    pub(super) errors: Vec<String>,
    pub(super) base: Base,
}

/// Hands a broken merge to a fix job on the same task, or holds the task
/// when a fix job already ran (or none can be queued). The merged jobs lose
/// their branches to the fix job, which takes all of them in.
pub(super) fn fix_or_hold(
    s: &hive_store::Store,
    project: &Row,
    rt: &ReadyTask,
    key: &str,
    b: Broken,
    now: &str,
) -> String {
    let slug = project.str("slug");
    let task = rt.task.as_str();
    let onto = b.base.describe();
    let def = b.base.branch.as_str();
    let names = b
        .merged
        .iter()
        .map(|(_, br, _)| br.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let first = b
        .errors
        .first()
        .cloned()
        .unwrap_or_else(|| "no error line".into());
    let note = format!(
        "{names} merge onto {onto} without a conflict but do not compile together: {}",
        crate::run::squeeze(&first, 200)
    );
    let hold_it = |note: &str| {
        hold(
            s,
            now,
            &slug,
            json!({"task": task, "work": key, "tip": b.tip}),
            note,
        );
        format!("task pr {slug} {task}: held: {note}")
    };
    let Ok(conn) = s.conn() else {
        return hold_it(&note);
    };
    let fixes = crate::util::query_rows(
        &conn,
        "SELECT detail FROM events WHERE subject = ? AND action = 'land.merge-broken' AND json_valid(detail)
         AND json_extract(detail, '$.task') = ? ORDER BY seq DESC",
        &[&slug, &task],
    )
    .unwrap_or_default();
    if fixes.len() >= FIXES_PER_TASK {
        let last: Value = serde_json::from_str(&fixes[0].str("detail")).unwrap_or(Value::Null);
        drop(conn);
        return hold_it(&format!(
            "{note}; job {} already made the task's branches compile together once, so this one is for you",
            field(&last, "follow_up")
        ));
    }
    let mut results = Vec::new();
    let mut tier = 1;
    let mut priority = 0;
    for (job, branch, _) in &b.merged {
        let row = crate::util::query_one(&conn, "SELECT * FROM jobs WHERE id = ?", &[job])
            .ok()
            .flatten()
            .unwrap_or_default();
        tier = tier.max(row.int("tier"));
        priority = priority.max(row.int("priority"));
        results.push(format!(
            "`{branch}` (job {job}):\n\n{}",
            crate::run::squeeze(&row.str("result"), 600)
        ));
    }
    drop(conn);
    let which = if b.base.stacked() {
        format!(
            " - the head of open pull request #{}, which the task's pull request stacks on; fetch it from the upstream first",
            b.base.stack_on
        )
    } else {
        String::new()
    };
    let open = rt
        .open
        .as_ref()
        .map(|pr| {
            format!(
                " The task's pull request #{} is open: fetch `{}` from the upstream and merge it too.",
                number_of(pr),
                task_branch(task)
            )
        })
        .unwrap_or_default();
    let list = b
        .merged
        .iter()
        .map(|(job, branch, sha)| {
            format!(
                "- `{branch}` (job {job}, commit {})",
                crate::recipe::short(sha, 12)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let brief = format!(
        "The finished branches of task {task} ({title}) each passed their own check, but merged together onto `{def}` they do not compile: `{cmd}` on the merged tree (commit {tip}) says\n\n```\n{errors}\n```\n\n\
         The branches, in the order the task's pull request merges them:\n\n{list}\n\n\
         Start from the current `{def}`{which}, merge them in that order (`git merge`, or cherry-pick their commits), fix what one side's change broke in the other - a signature one changed that the other still calls the old way, a name one moved - keeping the intent of both, run the project's check, and commit on your own branch.{open} \
         Leave those branches as they are; the task's pull request opens from your branch once its merged tree compiles.\n\n\
         What the jobs said of their work:\n\n{results}",
        title = rt.title,
        cmd = COMMAND[..4].join(" "),
        tip = crate::recipe::short(&b.tip, 12),
        errors = b.errors.join("\n"),
        results = results.join("\n\n"),
    );
    let queued = crate::jobs::add_job(
        s,
        slug.clone(),
        "task".into(),
        format!("Make {names} compile together: {}", rt.title),
        brief,
        tier,
        priority,
        format!("task:{task}"),
        ACTOR.into(),
        false,
    );
    let follow_up = match queued {
        Ok((j, _)) => j.str("id"),
        Err(e) => return hold_it(&format!("{note}; no job could be queued to fix it: {e}")),
    };
    if let Ok(conn) = s.conn() {
        for (job, _, _) in &b.merged {
            let _ = conn.execute(
                "UPDATE jobs SET branch = '', result = result || ? WHERE id = ? AND project = ?",
                rusqlite::params![
                    format!(
                        "\n\nMerged with the task's other branches it does not compile; job {follow_up} makes them compile together."
                    ),
                    job,
                    slug
                ],
            );
        }
    }
    event(
        s,
        now,
        "land.merge-broken",
        &slug,
        json!({
            "project": slug, "task": task, "tip": b.tip, "base": def,
            "jobs": b.merged.iter().map(|(j, _, _)| j).collect::<Vec<_>>(),
            "branches": b.merged.iter().map(|(_, br, _)| br).collect::<Vec<_>>(),
            "errors": b.errors, "follow_up": follow_up,
        }),
    );
    let _ = hive_tasks::send_message(
        s,
        &slug,
        task,
        ACTOR,
        &format!(
            "Branches {names} merge onto {onto} but do not compile together ({}); job {follow_up} makes them compile before the task's pull request opens.",
            crate::run::squeeze(&first, 200)
        ),
    );
    format!("task pr {slug} {task}: {note}; job {follow_up} makes them compile together")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_lines_keep_cargos_errors_in_order_once() {
        let text = "    Checking hive-runner v0.1.0\n\
            crates/hive-runner/src/landing_pr.rs:342:5: error[E0061]: this function takes 5 arguments but 4 arguments were supplied\n\
            warning: unused import\n\
            crates/hive-runner/src/delivery/experiments.rs:425:29: error[E0061]: this function takes 7 arguments but 6 arguments were supplied\n\
            error: could not compile `hive-runner` (lib) due to 1 previous error\n\
            crates/hive-runner/src/landing_pr.rs:342:5: error[E0061]: this function takes 5 arguments but 4 arguments were supplied\n";
        assert_eq!(
            error_lines(text),
            vec![
                "crates/hive-runner/src/landing_pr.rs:342:5: error[E0061]: this function takes 5 arguments but 4 arguments were supplied",
                "crates/hive-runner/src/delivery/experiments.rs:425:29: error[E0061]: this function takes 7 arguments but 6 arguments were supplied",
                "error: could not compile `hive-runner` (lib) due to 1 previous error",
            ]
        );
    }
}
