#![allow(unused_variables, dead_code, unused_imports)]
use crate::util::Row;
use anyhow::{Result, anyhow};
pub(crate) use hive_hand_core::provenance::{PROVENANCE_TRAILERS, provenance_hook};
// The clone's signing identity is set by the executor path (hive-hand-core,
// ADR-070 step 1d) so the hand configures it the very same way.
#[allow(unused_imports)]
pub(crate) use hive_hand_core::local::{
    agent_email, configure_clone_identity, git_config_quote, git_config_section, remove_run_key,
};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;
use std::time::Duration;
pub(crate) static OWNER_PATHS: &[&str] = hive_tally::OWNER_PATHS;

/// Sets a file's permission bits.
#[cfg(unix)]
fn set_mode(path: impl AsRef<Path>, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

/// Off unix there are no mode bits: the file keeps what its directory gives
/// it. An owner-only ACL is an open step of the port (docs/windows-port.md).
#[cfg(not(unix))]
fn set_mode(_path: impl AsRef<Path>, _mode: u32) -> std::io::Result<()> {
    Ok(())
}

pub(crate) fn make_run_key(path: &str, agent: &str, run_id: &str) -> Result<String> {
    let pub_path = format!("{path}.pub");
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(&pub_path);
    std::fs::create_dir_all(Path::new(path).parent().unwrap_or_else(|| Path::new("."))).map_err(
        |e| {
            crate::util::fail(
                1,
                format!("cannot make the directory for the run's signing key: {e}"),
            )
        },
    )?;
    let out = Command::new("ssh-keygen")
        .args([
            "-q",
            "-t",
            "ed25519",
            "-N",
            "",
            "-C",
            &format!("{agent} {run_id}"),
            "-f",
            path,
        ])
        .output();
    match out {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            if which::which("ssh-keygen").is_err() {
                return Err(crate::util::fail(1, "ssh-keygen is not installed; the runner needs it to give each run its signing key (ADR-022)").into());
            }
            return Err(crate::util::fail(
                1,
                format!(
                    "cannot make the run's signing key: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                ),
            )
            .into());
        }
        Err(e) => {
            if which::which("ssh-keygen").is_err() {
                return Err(crate::util::fail(1, "ssh-keygen is not installed; the runner needs it to give each run its signing key (ADR-022)").into());
            }
            return Err(
                crate::util::fail(1, format!("cannot make the run's signing key: {e}")).into(),
            );
        }
    }
    let _ = set_mode(path, 0o600);
    if !crate::util::file_exists(path) {
        return Err(crate::util::fail(1, format!("the run's signing key was not written at {path}; commits would fail at commit time (ADR-022)")).into());
    }
    let fields = crate::util::read_file(&pub_path)
        .split_whitespace()
        .take(2)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    Ok(fields.join(" "))
}
/// The public key of the run key already at `path`: a run that carries a
/// held run on signs with the same key, so the commits of both runs pass
/// the landing check together. Errors when either half is missing.
pub(crate) fn reuse_run_key(path: &str) -> Result<String> {
    let pub_path = format!("{path}.pub");
    if !crate::util::file_exists(path) || !crate::util::file_exists(&pub_path) {
        return Err(crate::util::fail(
            1,
            format!(
                "the held run's signing key is not at {path}; the run starts over with a new key"
            ),
        )
        .into());
    }
    let fields = crate::util::read_file(&pub_path)
        .split_whitespace()
        .take(2)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if fields.len() < 2 {
        return Err(crate::util::fail(
            1,
            format!("the held run's public key at {pub_path} is unreadable"),
        )
        .into());
    }
    Ok(fields.join(" "))
}
pub(crate) const PRE_COMMIT_HOOK_UNSET: &str = "unset GIT_INDEX_FILE GIT_PREFIX GIT_AUTHOR_NAME GIT_AUTHOR_EMAIL GIT_AUTHOR_DATE GIT_COMMITTER_NAME GIT_COMMITTER_EMAIL GIT_COMMITTER_DATE";
/// The hook runs `check_cmd` (the project's check inside the verify lock,
/// the shared-target-dir touch inside the lock with it: `run::wrapped_check`)
/// and keys the memo it reads and writes by `memo_cmd`, the bare check
/// inside the lock that every other verifier keys by (`run::verify_memo_key`,
/// landing's `vc_run`). The two differ only by the touch, which says nothing
/// about what was verified.
///
/// `base` is the commit the job started from. The hook exports it as
/// `HIVE_VERIFY_BASE` with a fresh `HIVE_VERIFY_SCOPE_FILE` beside it, so a
/// check that can narrow itself to what changed since the base does
/// (`scripts/verify-check.sh` checks the affected crates only: a cold full
/// check outlasts `harness.commit_timeout` on a 2-CPU hand). A check that
/// narrowed itself writes to the scope file, and the hook then writes no
/// verified-tree memo: the memo stands for the full check, which the run's
/// own check and the landing still run. An empty `base` exports nothing.
#[allow(clippy::too_many_arguments)]
pub(crate) fn pre_commit_hook_script(
    check_cmd: &str,
    memo_cmd: &str,
    recipe: Option<&crate::recipe::Recipe>,
    pins: &[String],
    clone: &str,
    hive_home: &str,
    project: &str,
    agent: &str,
    base: &str,
) -> String {
    let mut b = String::new();
    b.push_str("#!/bin/sh\n");
    b.push_str("# hive: verify-before-commit gate (auto-installed by runner)\n");
    b.push_str(PRE_COMMIT_HOOK_UNSET);
    b.push('\n');
    if let Some(r) = recipe {
        let exports = crate::recipe::recipe_env_exports(r);
        if !exports.is_empty() {
            b.push_str(exports.trim_end_matches("; "));
            b.push('\n');
        }
    }
    prepend_verify_memo_skip(&mut b, memo_cmd, clone, hive_home, project);
    prepend_verify_scope(&mut b, base);
    // The verify runs in a subshell: the lock wrapper ends in `exit`, which
    // would otherwise leave the hook before the memo below is written.
    b.push_str("(\n");
    if pins.is_empty() {
        b.push_str(check_cmd);
    } else {
        b.push_str(&crate::recipe::mise_wrap_shell(pins, check_cmd, clone));
    }
    b.push_str("\n)\nrc=$?\n");
    b.push_str(VERIFY_SCOPE_READ);
    b.push_str("if [ \"$rc\" -ne 0 ]; then exit \"$rc\"; fi\n");
    b.push_str("if [ -n \"$hive_scoped\" ]; then\n  echo \"hive: the commit's check was scoped to what changed since the job's base; no verified-tree memo is written, so the run's check and the landing run the full check\" >&2\n  exit 0\nfi\n");
    append_verify_memo_hook(&mut b, memo_cmd, clone, hive_home, project, agent);
    b
}

/// Exports `HIVE_VERIFY_BASE` and `HIVE_VERIFY_SCOPE_FILE` for the check
/// (see [`pre_commit_hook_script`]). Without a scope file to read back the
/// base is not exported either: a check that narrowed itself unseen would
/// leave a memo for a full check that never ran.
fn prepend_verify_scope(b: &mut String, base: &str) {
    let base = base.trim();
    if base.is_empty() {
        b.push_str("hive_scope_file=\n");
        return;
    }
    let base_q = crate::util::shell_quote(base);
    let _ = writeln!(
        b,
        "hive_scope_file=$(mktemp \"${{TMPDIR:-/tmp}}/hive-verify-scope.XXXXXX\" 2>/dev/null || true)
if [ -n \"$hive_scope_file\" ]; then
  export HIVE_VERIFY_BASE={base_q} HIVE_VERIFY_SCOPE_FILE=\"$hive_scope_file\"
fi"
    );
}

/// Reads back whether the check narrowed itself, then removes the scope file.
const VERIFY_SCOPE_READ: &str = "hive_scoped=
if [ -n \"$hive_scope_file\" ]; then
  if [ -s \"$hive_scope_file\" ]; then hive_scoped=1; fi
  rm -f \"$hive_scope_file\"
fi
";

/// The gate skips the rerun when this store already holds a fresh memo for the
/// identical index tree verified with the identical verify command: the memo
/// the hook below writes names the tree and the command hash, and lives under
/// the store-shared `$HIVE_HOME/verified/<project>/`, so a second commit of an
/// unchanged tree (a reword, a split, a retry after a lock timeout, or a commit
/// in a sibling clone of the same store whose own verify just passed) does not
/// queue behind the project verify lock for another full run. The verify lock
/// serializes every clone's verify on the store; replaying a fresh same-store
/// memo is what lets the clones that waited on that lock skip the rerun.
fn prepend_verify_memo_skip(
    b: &mut String,
    verify_cmd: &str,
    clone: &str,
    hive_home: &str,
    project: &str,
) {
    let _ = clone;
    let hash = crate::verify_memo::verify_sha256(verify_cmd);
    let project_q = crate::util::shell_quote(project);
    let home_q = crate::util::shell_quote(hive_home);
    let needle_hash = crate::util::shell_quote(&format!("\"verify_sha256\":\"{hash}\""));
    let _ = writeln!(
        b,
        "tree=$(git diff --quiet 2>/dev/null && git write-tree 2>/dev/null || true)
if [ -n \"$tree\" ]; then
  memo=\"{home_q}/verified/\"{project_q}\"/$tree\"
  if [ -f \"$memo\" ] && grep -qF -- {needle_hash} \"$memo\" 2>/dev/null; then
    echo \"hive: verify skipped: tree $tree already passed this verify on this store\" >&2
    exit 0
  fi
fi"
    );
}

fn append_verify_memo_hook(
    b: &mut String,
    verify_cmd: &str,
    clone: &str,
    hive_home: &str,
    project: &str,
    agent: &str,
) {
    let hash = crate::verify_memo::verify_sha256(verify_cmd);
    let project_q = crate::util::shell_quote(project);
    let home_q = crate::util::shell_quote(hive_home);
    let agent_json = serde_json::to_string(agent).unwrap_or_else(|_| "\"\"".into());
    let clone_json = serde_json::to_string(clone).unwrap_or_else(|_| "\"\"".into());
    // The memo names the index tree, so it is written only when the working
    // tree the verify ran on is that tree: no unstaged changes. The home is
    // the installer's, baked in: the daemon that installed the hook is the one
    // that reads the memo, whatever HIVE_HOME the committing shell carries.
    let _ = writeln!(
        b,
        "tree=$(git diff --quiet 2>/dev/null && git write-tree 2>/dev/null || true)
if [ -n \"$tree\" ]; then
  memo_home={home_q}
  memo_dir=\"$memo_home/verified/\"{project_q}
  at=$(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || true)
  if [ -n \"$at\" ] && mkdir -p \"$memo_dir\" 2>/dev/null; then
    {{ printf '%s' '{{\"verify_sha256\":\"{hash}\",\"at\":\"'; printf '%s' \"$at\"; printf '%s\\n' '\",\"by\":\"hook\",\"agent\":{agent_json},\"clone\":{clone_json}}}'; }} > \"$memo_dir/$tree\" 2>/dev/null || true
  fi
fi"
    );
}
pub(crate) fn write_pre_commit_hook(clone: &str, script: &str) -> Result<()> {
    let hook = Path::new(clone)
        .join(".git")
        .join("hooks")
        .join("pre-commit");
    std::fs::write(&hook, script)?;
    set_mode(&hook, 0o755)?;
    Ok(())
}
pub fn signers_line(agent: &str, pubkey: &str) -> String {
    format!("{} namespaces=\"git\" {}\n", agent_email(agent), pubkey)
}
pub fn signers_text(s: &hive_store::Store) -> String {
    let conn = match s.conn() {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    crate::util::must_rows(
        &conn,
        "SELECT agent, pubkey FROM runs WHERE pubkey != '' ORDER BY started_at, id",
        &[],
    )
    .into_iter()
    .map(|r| signers_line(&r.str("agent"), &r.str("pubkey")))
    .collect()
}
/// The refs in `root` whose commits a landing does not answer for: the
/// project's default branch as it stands now and Hive's own `hive/*`
/// branches, less a conversation's (`hive/chat-*`, which nothing is built
/// on) and `ref_` itself. A worker that builds on a sibling's unmerged
/// branch carries commits signed by the sibling's run key: on 2026-11-05
/// cinder-15 put TASK-078 on TASK-077's commit, whose pull request merged
/// while the run was on, and the landing refused the whole branch as not
/// this run's. Those commits passed this check when their own branch landed,
/// and the delivery stacks the pull request on the sibling's; only the
/// commits past these refs are this run's to answer for.
pub(crate) fn accepted_refs(root: &str, default_branch: &str, ref_: &str) -> Vec<String> {
    let own = if ref_.starts_with("refs/") {
        ref_.to_string()
    } else {
        format!("refs/heads/{ref_}")
    };
    let default_branch = if default_branch.is_empty() {
        "main"
    } else {
        default_branch
    };
    let out = crate::util::git_timeout(
        root,
        Duration::from_secs(60),
        &[
            "for-each-ref",
            "--format=%(refname)",
            &format!("refs/heads/{default_branch}"),
            "refs/heads/hive/",
        ],
    );
    if !out.ok() {
        return Vec::new();
    }
    out.stdout
        .lines()
        .map(str::trim)
        .filter(|r| !r.is_empty() && *r != own && !r.starts_with("refs/heads/hive/chat-"))
        .map(str::to_string)
        .collect()
}
/// How many commits `base..ref_` adds past `accepted`: the ones this run
/// answers for. Zero when git cannot say.
pub(crate) fn own_commit_count(root: &str, base: &str, ref_: &str, accepted: &[String]) -> i32 {
    let mut args = vec!["rev-list", "--count", ref_, "--not", base];
    args.extend(accepted.iter().map(String::as_str));
    args.push("--");
    crate::util::git_timeout(root, Duration::from_secs(60), &args)
        .stdout
        .trim()
        .parse()
        .unwrap_or(0)
}
/// The commits `base..ref_` adds past `accepted` (see `accepted_refs`) that
/// do not carry this run's identity: unsigned or signed by another key,
/// another author, another agent or job in the trailers. Empty when the
/// landing may go on.
#[allow(clippy::too_many_arguments)]
pub(crate) fn landing_problems(
    root: &str,
    base: &str,
    ref_: &str,
    agent: &str,
    job_id: &str,
    pubkey: &str,
    signers_path: &str,
    accepted: &[String],
) -> Vec<String> {
    if let Err(e) = std::fs::write(signers_path, signers_line(agent, pubkey)) {
        return vec![format!("could not write the allowed-signers file: {e}")];
    }
    let _ = set_mode(signers_path, 0o600);
    let format = "%H%x1f%G?%x1f%an%x1f%ae%x1f%(trailers:key=Hive-Agent,valueonly,separator=%x1d)%x1f%(trailers:key=Hive-Job,valueonly,separator=%x1d)%x1e";
    let allowed = format!("gpg.ssh.allowedSignersFile={signers_path}");
    let format_arg = format!("--format={format}");
    let mut args = vec![
        "-c",
        "gpg.ssh.program=ssh-keygen",
        "-c",
        &allowed,
        "log",
        &format_arg,
        ref_,
        "--not",
        base,
    ];
    args.extend(accepted.iter().map(String::as_str));
    args.push("--");
    let out = crate::util::git_timeout(root, Duration::from_secs(120), &args);
    let _ = std::fs::remove_file(signers_path);
    if !out.ok() {
        return vec![format!(
            "could not read the fetched commits: {}",
            out.stderr.trim()
        )];
    }
    let mut problems = Vec::new();
    for record in out
        .stdout
        .split('\x1e')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let mut f = record
            .split('\x1f')
            .map(|s| s.trim().to_string())
            .collect::<Vec<_>>();
        f.resize(6, String::new());
        let short_sha = crate::recipe::short(&f[0], 12);
        if f[1] != "G" {
            let w = match f[1].as_str() {
                "N" => "is not signed".to_string(),
                "U" => "is signed by a key that is not this run's".to_string(),
                "B" => "has a bad signature".to_string(),
                "E" => "has a signature that cannot be checked".to_string(),
                x => format!("has signature status {x}"),
            };
            problems.push(format!("commit {short_sha} {w}"));
        }
        if f[2] != agent || f[3] != agent_email(agent) {
            problems.push(format!(
                "commit {short_sha} is authored by {} <{}>, not {agent}",
                f[2], f[3]
            ));
        }
        let agents = split_trailer(&f[4]);
        let jobs = split_trailer(&f[5]);
        if agents.is_empty() {
            problems.push(format!("commit {short_sha} lacks its Hive-Agent trailer"));
        } else if agents.len() != 1 || agents[0] != agent {
            problems.push(format!(
                "commit {short_sha} names {} in its Hive-Agent trailer, not {agent}",
                agents.join(", ")
            ));
        }
        if jobs.is_empty() {
            problems.push(format!("commit {short_sha} lacks its Hive-Job trailer"));
        } else if jobs.len() != 1 || jobs[0] != job_id {
            problems.push(format!(
                "commit {short_sha} names job {} in its Hive-Job trailer, not {job_id}",
                jobs.join(", ")
            ));
        }
    }
    problems
}
pub(crate) fn split_trailer(s: &str) -> Vec<String> {
    s.split('\x1d')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}
/// Probe, as the daemon, the ssh hands whose `next_probe_at` is empty or due: the drained hands
/// that were never active (`NEVER_ACTIVE_SQL`), which a pass activates and a failure holds for
/// the daemon interval or the first `LOGIN_BACKOFF_MINS` step, whichever is longer; and the hands
/// that went `unreachable`, which a pass puts back in rotation and a failure holds for twice the
/// last wait, from the minute `note_executor_run` set up to `UNREACHABLE_BACKOFF_MAX_MINS`.
/// Returns `[name, note]` per hand that became active.
pub(crate) fn reprobe_due(s: &hive_store::Store, interval: i32, now: &str) -> Vec<[String; 2]> {
    let Ok(conn) = s.conn() else {
        return Vec::new();
    };
    let due: Vec<(String, String, i32)> = conn
        .prepare(&format!(
            "SELECT name, status, backoff_minutes FROM executors WHERE kind = 'ssh' AND ({} OR status = 'unreachable') AND (next_probe_at = '' OR next_probe_at <= ?) ORDER BY name",
            crate::executor::NEVER_ACTIVE_SQL
        ))
        .and_then(|mut q| {
            q.query_map([now], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i32>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()
        })
        .unwrap_or_default();
    let wait = (interval / 60).max(crate::executor::LOGIN_BACKOFF_MINS[0]);
    let mut back = Vec::new();
    for (name, was, backoff) in due {
        let (ok, note) = crate::executor::probe_executor(s, &name, "daemon", now)
            .unwrap_or_else(|e| (false, e.to_string()));
        let status = crate::executor::executor_row(s, &name).str("status");
        if ok && status == "active" {
            let what = if was == "unreachable" {
                "probe passed, back in rotation"
            } else {
                "first probe passed, now active"
            };
            back.push([name, format!("{what}: {note}")]);
        } else if status == "unreachable" {
            let wait = backoff
                .saturating_mul(2)
                .clamp(1, crate::executor::UNREACHABLE_BACKOFF_MAX_MINS)
                .max(interval / 60);
            let _ = conn.execute(
                "UPDATE executors SET backoff_minutes = ?, next_probe_at = ? WHERE name = ?",
                rusqlite::params![wait, crate::util::plus_minutes(now, wait), name],
            );
        } else if status == "drained" {
            let _ = conn.execute(
                "UPDATE executors SET next_probe_at = ? WHERE name = ?",
                [crate::util::plus_minutes(now, wait), name],
            );
        }
    }
    back
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passing_probe(
        _ctx: &hive_bus::CancellationToken,
        _argv: &[String],
        _stdin: &[u8],
        _timeout: std::time::Duration,
    ) -> anyhow::Result<(i32, String, String)> {
        Ok((
            0,
            "hands=ok hive=ok heartbeat=1 git=ok harness=yes keys_ok=yes keys=- remotes=0 disk_free_mb=20480".into(),
            String::new(),
        ))
    }

    fn refused_probe(
        _ctx: &hive_bus::CancellationToken,
        _argv: &[String],
        _stdin: &[u8],
        _timeout: std::time::Duration,
    ) -> anyhow::Result<(i32, String, String)> {
        Ok((
            crate::executor::SSH_EXIT_UNREACH,
            String::new(),
            "ssh: connect to host 20.52.42.230 port 22: Connection refused".into(),
        ))
    }

    fn registered_hand(s: &hive_store::Store) {
        crate::executor::add_executor(
            s,
            crate::test_helpers::TEST_NOW,
            "vm-new",
            "hive@new",
            "/srv/hive",
            1,
            "*",
            hive_store::HUMAN_AGENT,
        )
        .unwrap();
    }

    #[test]
    fn daemon_first_probe_activates_a_hand_that_was_never_active() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        let s = crate::test_helpers::new_store();
        registered_hand(&s);
        assert_eq!(
            crate::executor::executor_row(&s, "vm-new").str("status"),
            "drained"
        );
        let old = crate::executor::set_run_ssh(passing_probe);
        let back = reprobe_due(&s, 30, crate::test_helpers::TEST_NOW);
        crate::executor::restore_run_ssh(old);
        assert_eq!(back.len(), 1, "{back:?}");
        assert_eq!(back[0][0], "vm-new");
        assert_eq!(
            crate::executor::executor_row(&s, "vm-new").str("status"),
            "active"
        );
    }

    #[test]
    fn daemon_probe_keeps_a_hand_drained_after_errors() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        let s = crate::test_helpers::new_store();
        registered_hand(&s);
        s.conn()
            .unwrap()
            .execute(
                "UPDATE executors SET status = 'active' WHERE name = 'vm-new'",
                [],
            )
            .unwrap();
        crate::executor::note_executor_run(
            &s,
            crate::test_helpers::TEST_NOW,
            "vm-new",
            "error",
            "disk full",
            false,
            true,
        );
        assert_eq!(
            crate::executor::executor_row(&s, "vm-new").str("status"),
            "drained"
        );
        let old = crate::executor::set_run_ssh(passing_probe);
        let back = reprobe_due(&s, 30, crate::test_helpers::TEST_NOW);
        let (ok, _) =
            crate::executor::probe_executor(&s, "vm-new", "daemon", crate::test_helpers::TEST_NOW)
                .unwrap();
        crate::executor::restore_run_ssh(old);
        assert!(back.is_empty(), "{back:?}");
        assert!(ok);
        assert_eq!(
            crate::executor::executor_row(&s, "vm-new").str("status"),
            "drained"
        );
    }

    #[test]
    fn daemon_probes_an_unreachable_hand_again_with_a_growing_wait() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        let now = crate::test_helpers::TEST_NOW;
        let s = crate::test_helpers::new_store();
        registered_hand(&s);
        s.conn()
            .unwrap()
            .execute(
                "UPDATE executors SET status = 'active' WHERE name = 'vm-new'",
                [],
            )
            .unwrap();
        for _ in 0..crate::executor::UNREACHABLE_AFTER {
            crate::executor::note_executor_run(
                &s,
                now,
                "vm-new",
                "error",
                "ssh: connect to host 20.52.42.230 port 22: Connection refused",
                true,
                false,
            );
        }
        let row = crate::executor::executor_row(&s, "vm-new");
        assert_eq!(row.str("status"), "unreachable");
        assert_eq!(row.str("next_probe_at"), crate::util::plus_minutes(now, 1));
        // Not due yet: left alone even though the machine would answer.
        let old = crate::executor::set_run_ssh(passing_probe);
        let back = reprobe_due(&s, 30, now);
        crate::executor::restore_run_ssh(old);
        assert!(back.is_empty(), "{back:?}");
        assert_eq!(
            crate::executor::executor_row(&s, "vm-new").str("status"),
            "unreachable"
        );
        // Due and still refused: the wait doubles and the hand stays unreachable.
        let t1 = crate::util::plus_minutes(now, 1);
        let old = crate::executor::set_run_ssh(refused_probe);
        let back = reprobe_due(&s, 30, &t1);
        crate::executor::restore_run_ssh(old);
        assert!(back.is_empty(), "{back:?}");
        let row = crate::executor::executor_row(&s, "vm-new");
        assert_eq!(row.str("status"), "unreachable");
        assert_eq!(row.int("backoff_minutes"), 2);
        assert_eq!(row.str("next_probe_at"), crate::util::plus_minutes(&t1, 2));
        // The wait never grows past the cap.
        s.conn()
            .unwrap()
            .execute(
                "UPDATE executors SET backoff_minutes = ?, next_probe_at = ? WHERE name = 'vm-new'",
                rusqlite::params![crate::executor::UNREACHABLE_BACKOFF_MAX_MINS, t1],
            )
            .unwrap();
        let old = crate::executor::set_run_ssh(refused_probe);
        reprobe_due(&s, 30, &t1);
        crate::executor::restore_run_ssh(old);
        let row = crate::executor::executor_row(&s, "vm-new");
        assert_eq!(
            row.int("backoff_minutes"),
            crate::executor::UNREACHABLE_BACKOFF_MAX_MINS
        );
        // Due and answering: back in rotation with a clean slate.
        let t2 = row.str("next_probe_at");
        let old = crate::executor::set_run_ssh(passing_probe);
        let back = reprobe_due(&s, 30, &t2);
        crate::executor::restore_run_ssh(old);
        assert_eq!(back.len(), 1, "{back:?}");
        assert_eq!(back[0][0], "vm-new");
        assert!(
            back[0][1].starts_with("probe passed, back in rotation"),
            "{back:?}"
        );
        let row = crate::executor::executor_row(&s, "vm-new");
        assert_eq!(row.str("status"), "active");
        assert_eq!(row.int("errors"), 0);
        assert_eq!(row.int("backoff_minutes"), 0);
        assert_eq!(row.str("next_probe_at"), "");
    }

    #[test]
    fn signing_key_must_exist_before_identity() {
        let _serial = crate::test_helpers::serial();
        let clone = tempfile::tempdir().unwrap();
        crate::test_helpers::run(clone.path(), &["git", "init", "-q"]);
        let missing = tempfile::tempdir()
            .unwrap()
            .path()
            .join("missing.key")
            .display()
            .to_string();
        let err = configure_clone_identity(
            &clone.path().display().to_string(),
            "aurora",
            &missing,
            &provenance_hook("aurora", "JOB-1", crate::executor::LOCAL_EXECUTOR),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("signing key"), "{err}");
        let out = std::process::Command::new(hive_gitenv::git_program())
            .args(["config", "--local", "--get", "user.signingkey"])
            .current_dir(clone.path())
            .env_clear()
            .envs(crate::util::git_env(&[]).into_iter().filter_map(|kv| {
                kv.split_once('=')
                    .map(|(k, v)| (k.to_string(), v.to_string()))
            }))
            .output()
            .unwrap();
        assert!(!out.status.success(), "user.signingkey set unexpectedly");
    }

    #[test]
    fn signing_key_creates_work_dir() {
        let _serial = crate::test_helpers::serial();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let key = tempfile::tempdir()
            .unwrap()
            .path()
            .join("work/job.key")
            .display()
            .to_string();
        make_run_key(&key, "aurora", "run-1").unwrap();
        assert!(crate::util::file_exists(&key));
    }

    #[test]
    fn configure_clone_identity_include_file() {
        let _serial = crate::test_helpers::serial();
        let dir = tempfile::tempdir().unwrap().path().join("with space \"q\"");
        std::fs::create_dir_all(&dir).unwrap();
        crate::test_helpers::run(&dir, &["git", "init", "-q"]);
        let key = dir.join("key \"x\" y").display().to_string();
        std::fs::write(&key, b"k").unwrap();
        configure_clone_identity(&dir.display().to_string(), "aurora", &key, "#!/bin/sh\n")
            .unwrap();
        for (k, want) in [
            ("user.name", "aurora".to_string()),
            ("user.email", "aurora@hive.local".to_string()),
            ("gpg.format", "ssh".to_string()),
            ("gpg.ssh.program", "ssh-keygen".to_string()),
            ("user.signingkey", key.clone()),
            ("commit.gpgsign", "true".to_string()),
            ("tag.gpgsign", "false".to_string()),
            (
                "core.hooksPath",
                dir.join(".git/hooks").display().to_string(),
            ),
        ] {
            let got = crate::util::git(&dir.display().to_string(), &["config", "--get", k]);
            assert_eq!(got.stdout, format!("{want}\n"), "{k}: {}", got.stderr);
        }
    }

    /// The hook runs the check it is given, touch and all, and keys the memo
    /// it reads and writes by the bare check, like every other verifier.
    #[test]
    fn pre_commit_hook_runs_the_check_but_keys_the_memo_by_the_bare_command() {
        let bare = "cargo test -p hive-runner";
        let check = crate::exec::freshen_before_check(bare, "the clone's");
        let script = pre_commit_hook_script(
            &check,
            bare,
            None,
            &[],
            "/clone",
            "/home/me/.hive",
            "proj",
            "aurora",
            "",
        );
        assert!(script.contains(&check), "{script}");
        assert!(
            script.contains(&crate::verify_memo::verify_sha256(bare)),
            "{script}"
        );
        assert!(
            !script.contains(&crate::verify_memo::verify_sha256(&check)),
            "{script}"
        );
        // The touch comes after the memo skip: a tree already passed is not
        // rebuilt either.
        let skip = script.find("verify skipped").unwrap();
        let touch = script.find("xargs -0 -r touch -c").unwrap();
        assert!(skip < touch, "{script}");
    }

    #[test]
    fn pre_commit_hook_writes_verified_tree_memo() {
        let script = pre_commit_hook_script(
            "cargo test -p hive-runner",
            "cargo test -p hive-runner",
            None,
            &[],
            "/clone",
            "/home/me/.hive",
            "proj",
            "aurora",
            "",
        );
        let hash = crate::verify_memo::verify_sha256("cargo test -p hive-runner");
        assert!(script.contains("git write-tree"), "{script}");
        assert!(script.contains("/verified/\"proj"), "{script}");
        assert!(script.contains(&hash), "{script}");
        assert!(script.contains("\"by\":\"hook\""), "{script}");
        assert!(script.contains("\"agent\":\"aurora\""), "{script}");
    }

    fn git_in(dir: &std::path::Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// The hook hands the check the job's base (`HIVE_VERIFY_BASE`), so a
    /// check like scripts/verify-check.sh can test only the crates a commit
    /// touches and fit `harness.commit_timeout` on a 2-CPU hand. A check that
    /// narrowed itself says so in `HIVE_VERIFY_SCOPE_FILE`; the hook then
    /// writes no verified-tree memo, so the run's check and the landing, which
    /// trust that memo, still run the full check. A check that ran in full
    /// writes the memo as before, and a hook installed without a base exports
    /// neither variable.
    #[test]
    fn pre_commit_hook_hands_the_base_to_the_check_and_writes_no_memo_for_a_scoped_check() {
        let _serial = crate::test_helpers::serial();
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let clone = dir.path().join("clone");
        std::fs::create_dir_all(&clone).unwrap();
        git_in(&clone, &["init", "-q", "-b", "main"]);
        git_in(&clone, &["config", "user.email", "t@example.com"]);
        git_in(&clone, &["config", "user.name", "t"]);
        git_in(&clone, &["config", "commit.gpgsign", "false"]);
        std::fs::write(clone.join("a.txt"), "base\n").unwrap();
        git_in(&clone, &["add", "a.txt"]);
        git_in(&clone, &["commit", "-q", "-m", "base"]);
        let base = git_in(&clone, &["rev-parse", "HEAD"]);
        let seen = dir.path().join("seen");
        let scope_files = dir.path().join("scope-files");
        // The check records the base it was given and narrows itself while
        // a `scoped` file sits in the clone, as verify-check.sh does when
        // only some crates changed.
        let verify = format!(
            "printf '%s\\n' \"${{HIVE_VERIFY_BASE:-none}}\" >> {seen}\nprintf '%s\\n' \"${{HIVE_VERIFY_SCOPE_FILE:-}}\" >> {files}\nif [ -f scoped ] && [ -n \"${{HIVE_VERIFY_SCOPE_FILE:-}}\" ]; then echo scoped > \"$HIVE_VERIFY_SCOPE_FILE\"; fi",
            seen = crate::util::shell_quote(&seen.display().to_string()),
            files = crate::util::shell_quote(&scope_files.display().to_string()),
        );
        let clone_s = clone.display().to_string();
        let home_s = home.display().to_string();
        let script = pre_commit_hook_script(
            &verify,
            &verify,
            None,
            &[],
            &clone_s,
            &home_s,
            "proj",
            "aurora",
            &base,
        );
        write_pre_commit_hook(&clone_s, &script).unwrap();
        let memo = |tree: &str| {
            crate::verify_memo::lookup(&home_s, "proj", tree, &verify, 24, chrono::Utc::now())
        };
        let seen_lines = || {
            std::fs::read_to_string(&seen)
                .unwrap_or_default()
                .lines()
                .map(str::to_string)
                .collect::<Vec<_>>()
        };

        // A scoped check: the commit lands, no memo stands for its tree.
        std::fs::write(clone.join("scoped"), "").unwrap();
        git_in(&clone, &["add", "scoped"]);
        git_in(&clone, &["commit", "-q", "-m", "scoped"]);
        assert_eq!(seen_lines(), vec![base.clone()], "the check gets the base");
        let scoped_tree = git_in(&clone, &["rev-parse", "HEAD^{tree}"]);
        assert!(
            memo(&scoped_tree).is_none(),
            "a scoped check leaves no memo for the full check to trust"
        );
        // So the same tree is checked again on the next commit.
        git_in(&clone, &["commit", "-q", "--amend", "-m", "scoped again"]);
        assert_eq!(seen_lines().len(), 2, "no memo, so the check runs again");

        // A full check: the memo is written as before.
        git_in(&clone, &["rm", "-q", "scoped"]);
        git_in(&clone, &["commit", "-q", "-m", "full"]);
        let full_tree = git_in(&clone, &["rev-parse", "HEAD^{tree}"]);
        assert!(memo(&full_tree).is_some(), "a full check writes its memo");
        let files = std::fs::read_to_string(&scope_files).unwrap_or_default();
        assert_eq!(files.lines().count(), 3, "{files}");
        for file in files.lines() {
            assert!(
                !file.is_empty() && !std::path::Path::new(file).exists(),
                "the hook removes the scope file {file:?} once it read it"
            );
        }

        // Without a base the hook exports nothing.
        let bare = pre_commit_hook_script(
            &verify,
            &verify,
            None,
            &[],
            &clone_s,
            &home_s,
            "proj",
            "aurora",
            "",
        );
        assert!(!bare.contains("export HIVE_VERIFY_BASE"), "{bare}");
        assert!(script.contains("export HIVE_VERIFY_BASE="), "{script}");
    }

    /// A commit of a tree already verified on this store with the same command
    /// (a reword, a retry after a lock timeout, or a sibling clone's verify of
    /// the same tree) skips the rerun; a new tree still runs the verify.
    #[test]
    fn pre_commit_hook_skips_verify_for_a_tree_already_verified_on_this_store() {
        let _serial = crate::test_helpers::serial();
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let clone = dir.path().join("clone");
        std::fs::create_dir_all(&clone).unwrap();
        git_in(&clone, &["init", "-q", "-b", "main"]);
        git_in(&clone, &["config", "user.email", "t@example.com"]);
        git_in(&clone, &["config", "user.name", "t"]);
        git_in(&clone, &["config", "commit.gpgsign", "false"]);
        let counter = dir.path().join("runs");
        let verify = format!(
            "( printf x >> {} )\nhive_rc=$?\nexit $hive_rc",
            crate::util::shell_quote(&counter.display().to_string())
        );
        let runs = || std::fs::read_to_string(&counter).unwrap_or_default().len();
        let clone_s = clone.display().to_string();
        let home_s = home.display().to_string();
        let script = pre_commit_hook_script(
            &verify,
            &verify,
            None,
            &[],
            &clone_s,
            &home_s,
            "proj",
            "aurora",
            "",
        );
        write_pre_commit_hook(&clone_s, &script).unwrap();

        std::fs::write(clone.join("a.txt"), "one\n").unwrap();
        git_in(&clone, &["add", "a.txt"]);
        git_in(&clone, &["commit", "-q", "-m", "one"]);
        assert_eq!(runs(), 1, "first commit runs the verify");
        let tree = git_in(&clone, &["rev-parse", "HEAD^{tree}"]);

        // Same tree, new commit message: the verify is not rerun.
        git_in(&clone, &["commit", "-q", "--amend", "-m", "one reworded"]);
        assert_eq!(tree, git_in(&clone, &["rev-parse", "HEAD^{tree}"]));
        assert_eq!(runs(), 1, "an unchanged tree skips the verify rerun");

        // A different tree runs it again.
        std::fs::write(clone.join("b.txt"), "two\n").unwrap();
        git_in(&clone, &["add", "b.txt"]);
        git_in(&clone, &["commit", "-q", "-m", "two"]);
        assert_eq!(runs(), 2, "a new tree runs the verify");

        // A memo written by a sibling clone on the same store counts for this
        // one: the verify lock serializes both clones, so replaying it is what
        // spares the queued clone a serial rerun.
        let tree2 = git_in(&clone, &["rev-parse", "HEAD^{tree}"]);
        let memo_path = home.join("verified").join("proj").join(&tree2);
        let memo = std::fs::read_to_string(&memo_path).unwrap();
        std::fs::write(
            &memo_path,
            memo.replace(
                &serde_json::to_string(&clone_s).unwrap(),
                "\"/elsewhere/clone\"",
            ),
        )
        .unwrap();
        git_in(&clone, &["commit", "-q", "--amend", "-m", "two reworded"]);
        assert_eq!(
            runs(),
            2,
            "a sibling clone's memo on the store skips the verify"
        );
    }

    /// The lock wrapper around a verify command ends in `exit`; the hook must
    /// still write the memo for the committed tree, and only when the working
    /// tree it verified is that tree.
    #[test]
    fn pre_commit_hook_memo_survives_an_exiting_verify_and_skips_dirty_trees() {
        let _serial = crate::test_helpers::serial();
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let clone = dir.path().join("clone");
        std::fs::create_dir_all(&clone).unwrap();
        git_in(&clone, &["init", "-q", "-b", "main"]);
        git_in(&clone, &["config", "user.email", "t@example.com"]);
        git_in(&clone, &["config", "user.name", "t"]);
        git_in(&clone, &["config", "commit.gpgsign", "false"]);
        let verify = "( true )\nhive_rc=$?\nexit $hive_rc";
        let script = pre_commit_hook_script(
            verify,
            verify,
            None,
            &[],
            &clone.display().to_string(),
            &home.display().to_string(),
            "proj",
            "aurora",
            "",
        );
        write_pre_commit_hook(&clone.display().to_string(), &script).unwrap();

        std::fs::write(clone.join("a.txt"), "one\n").unwrap();
        git_in(&clone, &["add", "a.txt"]);
        git_in(&clone, &["commit", "-q", "-m", "one"]);
        let tree = git_in(&clone, &["rev-parse", "HEAD^{tree}"]);
        let memo = crate::verify_memo::lookup(
            &home.display().to_string(),
            "proj",
            &tree,
            verify,
            24,
            chrono::Utc::now(),
        )
        .expect("the hook wrote a memo for the committed tree");
        assert_eq!(memo.by, "hook");
        assert_eq!(memo.agent, "aurora");

        // A second commit with another file left modified but unstaged: the
        // verify saw a tree that is not the one committed, so no memo.
        std::fs::write(clone.join("b.txt"), "two\n").unwrap();
        git_in(&clone, &["add", "b.txt"]);
        std::fs::write(clone.join("a.txt"), "changed\n").unwrap();
        git_in(&clone, &["commit", "-q", "-m", "two"]);
        let dirty_tree = git_in(&clone, &["rev-parse", "HEAD^{tree}"]);
        assert!(
            crate::verify_memo::lookup(
                &home.display().to_string(),
                "proj",
                &dirty_tree,
                verify,
                24,
                chrono::Utc::now(),
            )
            .is_none(),
            "no memo for a tree verified with unstaged changes"
        );
    }
}
