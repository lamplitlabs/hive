//! Provenance (ADR-022): the trailers every commit of a run carries, added
//! by a `prepare-commit-msg` hook local to the clone.

/// The trailer keys, in the order the hook writes them.
pub static PROVENANCE_TRAILERS: &[&str] = &["Hive-Agent", "Hive-Job", "Hive-Executor"];

/// The words the hook's refusal opens with when a commit would go out
/// without one of its trailers (the trailer follows, quoted).
pub const MISSING_TRAILER_SENTENCE: &str = "this commit would not carry its trailer";

/// The `prepare-commit-msg` hook that adds this run's trailers.
///
/// `--if-exists addIfDifferent`, not `doNothing`: git matches a trailer's key
/// by prefix, so a message whose last paragraph is `H: x` already "had" a
/// Hive-Agent, a Hive-Job and a Hive-Executor for `doNothing`, and the commit
/// went out with none of them, to be refused at the landing. A trailer the
/// message carries with the same value is still not added twice (an amend).
/// After adding, the hook reads the trailers back and fails the commit with a
/// plain sentence when one is still missing (`H: <agent>` is the same
/// Hive-Agent with the same value for git), so the worker learns it at
/// `git commit`, not from a refused landing.
///
/// `--no-divider` on both calls: without it `interpret-trailers` takes a
/// `---` line as the start of a patch, puts the trailers above it and parses
/// none below it, while the landing reads a commit's trailers (`git log
/// %(trailers)`) from the end of the whole message - so a message holding a
/// `---` line went out with trailers the landing could not see, or the hook
/// refused it as missing them.
pub fn provenance_hook(agent: &str, job_id: &str, executor: &str) -> String {
    let values = [agent, job_id, executor];
    let trailers = PROVENANCE_TRAILERS
        .iter()
        .zip(values)
        .map(|(k, v)| crate::util::shell_quote(&format!("{k}: {v}")))
        .collect::<Vec<_>>();
    let parts = trailers
        .iter()
        .map(|t| format!("--trailer {t}"))
        .collect::<Vec<_>>()
        .join(" ");
    let list = trailers.join(" ");
    format!(
        "#!/bin/sh\n\
# hive: provenance trailers for every commit of this run (ADR-022). Local to this clone, never committed.\n\
git interpret-trailers --no-divider --in-place --if-exists addIfDifferent {parts} \"$1\" || exit 1\n\
have=$(git interpret-trailers --no-divider --parse \"$1\") || exit 1\n\
for t in {list}; do\n\
\tprintf '%s\\n' \"$have\" | grep -qxF -- \"$t\" && continue\n\
\techo \"hive: {MISSING_TRAILER_SENTENCE} '$t' (ADR-022), so the landing would refuse it. git reads the last paragraph of a message as trailers and took a 'Word: text' line there for it; end the message with a sentence instead and commit again.\" >&2\n\
\texit 1\n\
done\n"
    )
}

#[cfg(test)]
mod tests {
    use super::provenance_hook;
    use std::path::Path;

    /// A repository whose `prepare-commit-msg` is the run's hook.
    fn repo_with_hook(dir: &Path, agent: &str) {
        crate::test_helpers::run(dir, &["git", "init", "-q"]);
        let hook = dir.join(".git/hooks/prepare-commit-msg");
        std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
        std::fs::write(&hook, provenance_hook(agent, "job-1", "local")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn commit(dir: &Path, msg: &str) -> std::process::Output {
        std::fs::write(dir.join("f.txt"), msg).unwrap();
        crate::test_helpers::run(dir, &["git", "add", "f.txt"]);
        crate::test_helpers::run_output(dir, &["git", "commit", "-q", "-m", msg])
    }

    fn trailers(dir: &Path) -> String {
        crate::test_helpers::run(dir, &["git", "log", "-1", "--format=%(trailers)"])
            .trim()
            .to_string()
    }

    #[test]
    fn a_last_paragraph_of_word_lines_does_not_stop_the_trailers() {
        let _serial = crate::test_helpers::serial();
        let dir = tempfile::tempdir().unwrap();
        repo_with_hook(dir.path(), "aurora");
        // git matches trailer keys by prefix: `H: x` was a Hive-Agent, a
        // Hive-Job and a Hive-Executor for `doNothing`.
        let out = commit(dir.path(), "subject\n\nbody\n\nH: x");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let want = "H: x\nHive-Agent: aurora\nHive-Job: job-1\nHive-Executor: local";
        assert_eq!(trailers(dir.path()), want);
        // An amend keeps one of each.
        let out = crate::test_helpers::run_output(
            dir.path(),
            &["git", "commit", "-q", "--amend", "--no-edit"],
        );
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(trailers(dir.path()), want);
        // A plain message gets the three as before.
        let out = commit(dir.path(), "second\n\nA sentence.");
        assert!(out.status.success());
        assert_eq!(
            trailers(dir.path()),
            "Hive-Agent: aurora\nHive-Job: job-1\nHive-Executor: local"
        );
    }

    #[test]
    fn a_divider_line_in_the_message_does_not_hide_the_trailers() {
        let _serial = crate::test_helpers::serial();
        let dir = tempfile::tempdir().unwrap();
        repo_with_hook(dir.path(), "aurora");
        // Without --no-divider the hook put the trailers above the `---`
        // line, where `git log %(trailers)` - what the landing reads - does
        // not look.
        let out = commit(
            dir.path(),
            "subject\n\nbody\n---\nA quoted diff stat or a rule line follows.",
        );
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            trailers(dir.path()),
            "Hive-Agent: aurora\nHive-Job: job-1\nHive-Executor: local"
        );
        let body = crate::test_helpers::run(dir.path(), &["git", "log", "-1", "--format=%B"]);
        assert!(
            body.trim_end().ends_with("Hive-Executor: local"),
            "the trailers are not at the end of the message: {body}"
        );
        // An amend keeps one of each.
        let out = crate::test_helpers::run_output(
            dir.path(),
            &["git", "commit", "-q", "--amend", "--no-edit"],
        );
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            trailers(dir.path()),
            "Hive-Agent: aurora\nHive-Job: job-1\nHive-Executor: local"
        );
    }

    #[test]
    fn a_trailer_git_takes_as_present_fails_the_commit_with_a_sentence() {
        let _serial = crate::test_helpers::serial();
        let dir = tempfile::tempdir().unwrap();
        repo_with_hook(dir.path(), "aurora");
        crate::test_helpers::run(
            dir.path(),
            &["git", "commit", "-q", "--allow-empty", "-m", "first"],
        );
        let before = crate::test_helpers::run(dir.path(), &["git", "rev-parse", "HEAD"]);
        // `H: aurora` is the same Hive-Agent with the same value for git, so
        // even addIfDifferent adds no Hive-Agent: the commit must not go out.
        let out = commit(dir.path(), "subject\n\nH: aurora");
        assert!(
            !out.status.success(),
            "the commit went out without its trailer"
        );
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains("this commit would not carry its trailer 'Hive-Agent: aurora'")
                && err.contains("end the message with a sentence instead and commit again"),
            "{err}"
        );
        assert_eq!(
            crate::test_helpers::run(dir.path(), &["git", "rev-parse", "HEAD"]),
            before
        );
    }
}
