//! A job's local run on the machine that holds its clone (ADR-070 step 1d):
//! the clone's preparation (a fresh `git clone`, or the held run's clone
//! kept), the run's signing identity in it, the worker's run in its own
//! process group with its output captured, and the cleanup or hold after.
//!
//! The brain's `LocalExec` drives these from its job and project rows; the
//! hand drives them from what the lease tells it. Nothing here reads the
//! store: every piece takes a small parameter struct of plain values, and
//! the heartbeat that watches the worker comes in as a [`ProcessObserver`]
//! the caller built.

use crate::proc::{Outcome, ProcessObserver};
use crate::tidy::Leftover;
use anyhow::Result;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long a fresh `git clone` of the project may take.
pub const CLONE_TIMEOUT: Duration = Duration::from_secs(300);

/// A run's log lines, shared between the thread that runs the job and the
/// ones that report on it; clones share the same lines.
#[derive(Debug, Clone, Default)]
pub struct RunLog(Arc<Mutex<Vec<String>>>);
impl RunLog {
    pub fn add(&self, line: impl Into<String>) {
        self.lines().push(line.into())
    }
    pub fn put(&self, line: &str) {
        self.lines().push(line.into())
    }
    /// Every non-empty line, joined by newlines.
    pub fn text(&self) -> String {
        self.lines()
            .iter()
            .filter(|s| !s.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }
    fn lines(&self) -> std::sync::MutexGuard<'_, Vec<String>> {
        self.0.lock().expect("run log")
    }
}
/// What is written to the log becomes one line per newline-separated part.
impl Write for RunLog {
    fn write(&mut self, p: &[u8]) -> std::io::Result<usize> {
        for line in String::from_utf8_lossy(p)
            .trim_end_matches('\n')
            .split('\n')
        {
            self.put(line);
        }
        Ok(p.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The address an agent's commits carry.
pub fn agent_email(agent: &str) -> String {
    format!("{agent}@pulse.local")
}

/// The `[section]` or `[section "sub"]` header a git config key lives under.
pub fn git_config_section(key: &str) -> String {
    let (section, rest) = key.split_once('.').unwrap_or((key, ""));
    if let Some((sub, _)) = rest.rsplit_once('.') {
        format!("[{} {}]\n", section, git_config_quote(sub))
    } else {
        format!("[{section}]\n")
    }
}

/// A git config value in double quotes, escaped.
pub fn git_config_quote(v: &str) -> String {
    format!(
        "\"{}\"",
        v.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    )
}

/// Points the clone at the run's identity: the agent's name and address,
/// SSH signing with the run's own key, and the clone's hooks with
/// `prepare-commit-msg` set to `hook_text` (ADR-022). Refuses a key that is
/// not on disk rather than configure a signing key nothing can sign with.
pub fn configure_clone_identity(
    clone: &str,
    agent: &str,
    key_path: &str,
    hook_text: &str,
) -> Result<()> {
    if !crate::util::file_exists(key_path) {
        return Err(crate::error::errorf(1, &format!("the run's signing key {key_path} does not exist; refusing to configure user.signingkey (ADR-022)"), "", "").into());
    }
    let hooks = Path::new(clone).join(".git").join("hooks");
    let mut identity = String::new();
    for (key, value) in [
        ("user.name", agent.to_string()),
        ("user.email", agent_email(agent)),
        ("gpg.format", "ssh".to_string()),
        ("gpg.ssh.program", "ssh-keygen".to_string()),
        ("user.signingkey", key_path.to_string()),
        ("commit.gpgsign", "true".to_string()),
        ("tag.gpgsign", "false".to_string()),
        ("core.hooksPath", hooks.display().to_string()),
    ] {
        let leaf = key.rsplit_once('.').map(|(_, x)| x).unwrap_or(key);
        let _ = writeln!(
            identity,
            "{}\t{} = {}",
            git_config_section(key),
            leaf,
            git_config_quote(&value)
        );
    }
    let identity_path = Path::new(clone).join(".git").join("pulse-identity");
    std::fs::write(&identity_path, identity)?;
    crate::util::git_check(
        clone,
        &[
            "config",
            "include.path",
            &identity_path.display().to_string(),
        ],
    )?;
    std::fs::create_dir_all(&hooks)?;
    let hook = hooks.join("prepare-commit-msg");
    std::fs::write(&hook, hook_text)?;
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}

/// Removes the run's signing key and its public half.
pub fn remove_run_key(path: &str) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(format!("{path}.pub"));
}

/// Refuses to start a worker whose signing key is missing or empty: its
/// commits could not be signed, and nothing has run yet so nothing is lost.
pub fn preflight_run_key(path: &str) -> Result<()> {
    let meta = std::fs::metadata(path).map_err(|e| crate::error::errorf(1, &format!("setup-error: the run's signing key {path} is missing ({e}); the worker was not started so no work is lost (ADR-022)"), "", ""))?;
    if meta.len() == 0 {
        return Err(crate::error::errorf(1, &format!("setup-error: the run's signing key {path} is empty; the worker was not started so no work is lost (ADR-022)"), "", "").into());
    }
    Ok(())
}

/// The directories a local worker may write outside its clone: the PULSE
/// home, the directory of the running binary, the temp dir and `/tmp`, each
/// once.
pub fn worker_dirs(home: &str) -> Vec<String> {
    let mut dirs = vec![home.to_string()];
    if let Some(parent) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|parent| parent.display().to_string()))
    {
        dirs.push(parent);
    }
    dirs.push(std::env::temp_dir().display().to_string());
    if Path::new("/tmp").is_dir() {
        dirs.push("/tmp".into());
    }
    let mut out = Vec::new();
    for d in dirs {
        if !out.contains(&d) {
            out.push(d);
        }
    }
    out
}

fn short(s: &str, n: usize) -> String {
    if s.len() > n {
        String::from_utf8_lossy(&s.as_bytes()[..n]).to_string()
    } else {
        s.to_string()
    }
}

/// What preparing a job's clone needs to know.
#[derive(Debug, Clone, Default)]
pub struct ClonePrep {
    /// The PULSE home whose trash an old clone is discarded into.
    pub home: String,
    /// The repository to clone from.
    pub root: String,
    /// Where the clone goes.
    pub clone_dir: String,
    /// The branch to clone; empty clones the source's HEAD.
    pub branch: String,
    /// The run's signing key.
    pub key: String,
    pub agent: String,
    pub job_id: String,
    /// The executor's name, for the commits' provenance trailers.
    pub executor: String,
    /// The base of a held run whose clone this run carries on in.
    pub resume_base: Option<String>,
}

/// Prepares the job's clone and returns its base commit. A held run's clone
/// is kept (strays swept, identity set for this run) when it is still
/// there; otherwise any old clone goes with what runs in it, the project is
/// cloned afresh without its origin, and the run's identity is set.
pub fn prepare_clone(
    cancel: &crate::ctx::CancellationToken,
    prep: &ClonePrep,
    log: &RunLog,
) -> Result<String> {
    preflight_run_key(&prep.key)?;
    let hook = crate::provenance::provenance_hook(&prep.agent, &prep.job_id, &prep.executor);
    if let Some(base) = &prep.resume_base {
        if Path::new(&prep.clone_dir).join(".git").exists() {
            // The held run's clone, edits and commits included; strays
            // its worker left go, the identity is set for this run.
            crate::proc::sweep_job_dir(&prep.clone_dir, &mut |line| log.add(line));
            configure_clone_identity(&prep.clone_dir, &prep.agent, &prep.key, &hook)?;
            log.add(format!(
                "clone kept from the held run (base {})",
                short(base, 12)
            ));
            return Ok(base.clone());
        }
        log.add("the held run's clone is gone; cloning anew".to_string());
    }
    // A clone left by an earlier attempt goes, with whatever still runs in it.
    if Path::new(&prep.clone_dir).exists() {
        crate::proc::sweep_job_dir(&prep.clone_dir, &mut |line| log.add(line));
        let _ = crate::trash::discard_dir(Path::new(&prep.home), Path::new(&prep.clone_dir));
    }
    let mut args = vec![
        "git".to_string(),
        "clone".to_string(),
        "-q".to_string(),
        "--no-tags".to_string(),
    ];
    if !prep.branch.is_empty() {
        args.extend(["-b".to_string(), prep.branch.clone()]);
    }
    args.extend([prep.root.clone(), prep.clone_dir.clone()]);
    let out = crate::proc::run_group(
        cancel,
        &args,
        "",
        &crate::util::git_env(&[]),
        &[],
        CLONE_TIMEOUT,
    )?;
    log.add(format!(
        "$ git clone -q --no-tags\n{}{}",
        out.stdout, out.stderr
    ));
    if out.exit_code != 0 {
        return Err(crate::error::errorf(
            1,
            &format!("clone failed: {}", out.stderr.trim()),
            "",
            "",
        )
        .into());
    }
    let _ = crate::util::git(&prep.clone_dir, &["remote", "remove", "origin"]);
    configure_clone_identity(&prep.clone_dir, &prep.agent, &prep.key, &hook)?;
    Ok(
        crate::util::git_check(&prep.clone_dir, &["rev-parse", "HEAD"])?
            .stdout
            .trim()
            .to_string(),
    )
}

/// What running the worker in its clone needs to know.
#[derive(Debug, Clone, Copy)]
pub struct WorkerRun<'a> {
    pub clone_dir: &'a str,
    /// The run's signing key, checked before anything starts.
    pub key: &'a str,
    /// The worker's command line.
    pub cmd: &'a [String],
    /// What the worker's environment adds to this process's.
    pub env: &'a BTreeMap<String, String>,
    /// The run's budget; zero or less gives it a second.
    pub minutes: i32,
}

/// Checks the run key and the worker's TMPDIR. A failure here means the
/// worker was not started, so the caller can retry without losing work.
pub fn preflight_worker(run: &WorkerRun<'_>) -> Result<Vec<String>> {
    preflight_run_key(run.key)?;
    let worker_env = crate::proc::worker_env_with_cli_path(&crate::proc::environ(), run.env);
    let tmp = crate::tmpspace::worker_tmp_dir(&worker_env);
    std::fs::create_dir_all(&tmp).map_err(|e| {
        crate::error::errorf(
            1,
            &format!(
                "setup-error: worker TMPDIR {tmp} cannot be made ({e}); the worker was not started so no work is lost"
            ),
            "",
            "",
        )
    })?;
    crate::tmpspace::preflight_tmp_space(&tmp, crate::tmpspace::MIN_TMP_SPACE)?;
    Ok(worker_env)
}

/// Runs the worker in its clone, in its own process group so a cancel ends
/// its whole tree; its stdout and stderr come back in the [`Outcome`].
/// `observe` is called once the preflight passed, right before the worker
/// starts: it marks the run begun and builds the [`ProcessObserver`] told as
/// the worker starts, beats and stops (the caller's heartbeat), so a run that
/// never starts opens no heartbeat.
pub fn run_worker(
    cancel: &crate::ctx::CancellationToken,
    run: &WorkerRun<'_>,
    observe: impl FnOnce() -> Option<ProcessObserver>,
) -> Result<Outcome> {
    let worker_env = preflight_worker(run)?;
    let observer = observe();
    let budget = if run.minutes <= 0 {
        Duration::from_secs(1)
    } else {
        Duration::from_secs(run.minutes as u64 * 60)
    };
    crate::proc::run_group_observed(
        cancel,
        run.cmd,
        run.clone_dir,
        &worker_env,
        &[],
        budget,
        observer.as_ref(),
    )
    .map_err(|e| {
        let exec_err = e.to_string().to_lowercase();
        crate::error::errorf(
            1,
            &format!(
                "worker command not found: {}: {exec_err}",
                run.cmd.first().cloned().unwrap_or_default()
            ),
            "",
            "",
        )
        .into()
    })
}

/// What a finished run leaves to clean up.
#[derive(Debug, Clone, Default)]
pub struct CloneCleanup {
    /// The PULSE home whose trash the clone goes into.
    pub home: String,
    pub clone_dir: String,
    /// The run's temp dir beside the clone.
    pub run_tmp_dir: String,
    /// The run's signing key.
    pub key: String,
}

/// Ends a run's clone: servers and watchers the worker left behind go
/// before their directory does, worktrees it added are removed and listed,
/// the clone and its temp dir are renamed into the trash in an instant (the
/// low-priority remover deletes them later) and the run key is removed.
pub fn cleanup_clone(c: &CloneCleanup, log: &RunLog) -> Vec<Leftover> {
    crate::proc::sweep_job_dir(&c.clone_dir, &mut |line| log.add(line));
    let left = crate::tidy::clear_leftovers(&c.clone_dir);
    let home = Path::new(&c.home);
    let _ = crate::trash::discard_dir(home, Path::new(&c.clone_dir));
    let _ = crate::trash::discard_dir(home, Path::new(&c.run_tmp_dir));
    crate::trash::empty_trash_later(home);
    remove_run_key(&c.key);
    left
}

/// Holds a run's clone for the run that carries on: only the strays go; the
/// clone with the worker's edits and commits, its temp dir and the signing
/// key its commits carry stay, since the next run must sign with that key.
pub fn hold_clone(clone_dir: &str, log: &RunLog) {
    crate::proc::sweep_job_dir(clone_dir, &mut |line| log.add(line));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{run, serial};

    #[test]
    fn run_log_splits_writes_into_lines_and_text_skips_empty_ones() {
        let mut log = RunLog::default();
        let shared = log.clone();
        write!(log, "one\ntwo\n").unwrap();
        log.add("");
        shared.add("three");
        assert_eq!(log.text(), "one\ntwo\nthree");
    }

    #[test]
    fn git_config_helpers_quote_and_section() {
        assert_eq!(git_config_section("user.name"), "[user]\n");
        assert_eq!(git_config_section("a.b.c"), "[a \"b\"]\n");
        assert_eq!(git_config_quote("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
        assert_eq!(agent_email("iris"), "iris@pulse.local");
    }

    #[test]
    fn preflight_run_key_refuses_a_missing_or_empty_key() {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("run.key");
        let k = key.display().to_string();
        let err = preflight_run_key(&k).unwrap_err().to_string();
        assert!(err.contains("is missing"), "{err}");
        std::fs::write(&key, b"").unwrap();
        let err = preflight_run_key(&k).unwrap_err().to_string();
        assert!(err.contains("is empty"), "{err}");
        std::fs::write(&key, b"k").unwrap();
        preflight_run_key(&k).unwrap();
        std::fs::write(format!("{k}.pub"), b"p").unwrap();
        remove_run_key(&k);
        assert!(!key.exists() && !dir.path().join("run.key.pub").exists());
    }

    #[test]
    fn worker_dirs_start_at_home_and_hold_each_dir_once() {
        let dirs = worker_dirs("/h");
        assert_eq!(dirs[0], "/h");
        let mut uniq = dirs.clone();
        uniq.dedup();
        assert_eq!(uniq.len(), dirs.len());
        assert!(dirs.contains(&std::env::temp_dir().display().to_string()));
    }

    #[test]
    fn prepare_runs_and_cleans_a_clone_without_the_store() {
        let _serial = serial();
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        run(&root, &["git", "init", "-q", "-b", "main"]);
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        run(&root, &["git", "add", "a.txt"]);
        run(&root, &["git", "commit", "-q", "-m", "first"]);
        let head = run(&root, &["git", "rev-parse", "HEAD"]).trim().to_string();
        let home = tmp.path().join("home");
        std::fs::create_dir_all(home.join("work")).unwrap();
        let key = home.join("work").join("job-x.key");
        std::fs::write(&key, b"k").unwrap();
        let clone_dir = home.join("work").join("job-x").display().to_string();
        let log = RunLog::default();
        let prep = ClonePrep {
            home: home.display().to_string(),
            root: root.display().to_string(),
            clone_dir: clone_dir.clone(),
            branch: "main".into(),
            key: key.display().to_string(),
            agent: "iris".into(),
            job_id: "job-x".into(),
            executor: "local".into(),
            resume_base: None,
        };
        let cancel = crate::ctx::CancellationToken::new();
        assert_eq!(prepare_clone(&cancel, &prep, &log).unwrap(), head);
        assert!(log.text().contains("$ git clone -q --no-tags"));
        let remotes = crate::util::git(&clone_dir, &["remote"]);
        assert_eq!(remotes.stdout.trim(), "");
        let email = crate::util::git(&clone_dir, &["config", "user.email"]);
        assert_eq!(email.stdout.trim(), "iris@pulse.local");
        let hook = Path::new(&clone_dir).join(".git/hooks/prepare-commit-msg");
        assert!(std::fs::read_to_string(&hook).unwrap().contains("job-x"));

        // A held run's clone is kept, its base handed back.
        let held = ClonePrep {
            resume_base: Some("0123456789abcdef".into()),
            ..prep.clone()
        };
        assert_eq!(
            prepare_clone(&cancel, &held, &log).unwrap(),
            "0123456789abcdef"
        );
        assert!(
            log.text()
                .contains("clone kept from the held run (base 0123456789ab)")
        );

        let cmd = vec![
            "sh".to_string(),
            "-c".to_string(),
            "pwd; echo err >&2".to_string(),
        ];
        let mut env = BTreeMap::new();
        env.insert("TMPDIR".to_string(), format!("{clone_dir}.tmp"));
        let mut began = false;
        let out = run_worker(
            &cancel,
            &WorkerRun {
                clone_dir: &clone_dir,
                key: &prep.key,
                cmd: &cmd,
                env: &env,
                minutes: 1,
            },
            || {
                began = true;
                None
            },
        )
        .unwrap();
        assert!(began);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.trim().ends_with("job-x"), "{}", out.stdout);
        assert_eq!(out.stderr.trim(), "err");

        hold_clone(&clone_dir, &log);
        assert!(Path::new(&clone_dir).exists());
        let left = cleanup_clone(
            &CloneCleanup {
                home: prep.home.clone(),
                clone_dir: clone_dir.clone(),
                run_tmp_dir: format!("{clone_dir}.tmp"),
                key: prep.key.clone(),
            },
            &log,
        );
        assert!(left.is_empty());
        assert!(!Path::new(&clone_dir).exists());
        assert!(!Path::new(&format!("{clone_dir}.tmp")).exists());
        assert!(!key.exists());
    }

    #[test]
    fn run_worker_names_a_missing_command_and_never_starts_without_a_key() {
        let dir = tempfile::tempdir().unwrap();
        let clone_dir = dir.path().display().to_string();
        let key = dir.path().join("run.key").display().to_string();
        let missing = dir.path().join("no-such-worker").display().to_string();
        let cmd = vec![missing.clone()];
        let env = BTreeMap::new();
        let run_ = WorkerRun {
            clone_dir: &clone_dir,
            key: &key,
            cmd: &cmd,
            env: &env,
            minutes: 1,
        };
        let cancel = crate::ctx::CancellationToken::new();
        let mut began = false;
        let err = run_worker(&cancel, &run_, || {
            began = true;
            None
        })
        .unwrap_err()
        .to_string();
        assert!(err.contains("setup-error"), "{err}");
        assert!(!began);
        std::fs::write(&key, b"k").unwrap();
        let err = run_worker(&cancel, &run_, || {
            began = true;
            None
        })
        .unwrap_err()
        .to_string();
        assert!(began);
        assert!(
            err.contains(&format!("worker command not found: {missing}")),
            "{err}"
        );
    }
}
