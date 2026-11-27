#![allow(unused_variables, dead_code, unused_imports)]
use crate::proc::Outcome;
use crate::util::Row;
use anyhow::{Result, anyhow};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
pub(crate) trait Executor {
    fn name(&self) -> String;
    fn remote(&self) -> bool;
    fn clone(&self) -> String;
    fn key_path(&self) -> String;
    fn started(&self) -> bool;
    fn worker_dirs(&self) -> Vec<String>;
    fn prepare(&mut self, cancel: &hive_bus::CancellationToken) -> Result<String>;
    fn run_worker(
        &mut self,
        cancel: &hive_bus::CancellationToken,
        cmd: &[String],
        env: &BTreeMap<String, String>,
        minutes: i32,
    ) -> Result<Outcome>;
    fn result_text(&self) -> Option<String>;
    fn commit_count(&self, base: &str) -> i32;
    fn dirty(&self) -> String;
    /// New files the worker left in the clone (untracked, not ignored), one
    /// path per line; an executor that cannot say reports none.
    fn untracked(&self) -> String {
        String::new()
    }
    /// Commits what the worker left staged or modified (tracked files only,
    /// like `git commit -a`) with the clone's own identity and hooks minus
    /// the pre-commit check, which is what killed the worker's commit. The
    /// run's verification judges the commit afterwards. Executors that
    /// cannot do this return an error and the work is discarded as before.
    fn commit_left_work(&self, message: &str) -> Result<()> {
        let _ = message;
        Err(anyhow!(
            "executor {} cannot commit on the worker's behalf",
            self.name()
        ))
    }
    fn owner_paths_touched(&self, base: &str) -> Vec<String>;
    /// Every file the worker's commits changed since `base`, sorted; an
    /// executor that cannot say reports none. The run's evidence line and
    /// the landing use it to tell a change with a face (a page, a view)
    /// from one without.
    fn changed_files(&self, base: &str) -> Vec<String> {
        let _ = base;
        Vec::new()
    }
    /// The run's own `TMPDIR` when the executor gives it one: a sibling of
    /// the clone (`work/<job>.tmp`) that the worker, the pre-commit check
    /// and the run's verification write their temporary files to, and that
    /// goes with the clone at cleanup. What a killed check leaves behind
    /// (a `cargo test` stopped at the step deadline never runs its
    /// `tempfile` destructors) then costs the clone's sweep, not the
    /// machine's temp dir. Empty: the worker inherits the machine's.
    fn run_tmp_dir(&self) -> String {
        String::new()
    }
    fn verify(
        &self,
        cancel: &hive_bus::CancellationToken,
        cmd: &str,
        minutes: i32,
    ) -> (Option<i32>, String);
    /// Installs the clone's verify-before-commit gate. `check_cmd` is what
    /// the hook runs - the project's check inside the verify lock, with the
    /// shared-target-dir touch inside the lock too (`run::wrapped_check`);
    /// `memo_cmd` is the check as every verifier keys its memo, the bare
    /// command inside the lock (`run::verify_memo_key`), so a memo the hook
    /// writes is good for the run's own check, a landing and a base check.
    /// `base` is the commit the job started from: the hook hands it to the
    /// check so a check that can narrow itself to what changed does
    /// (`keys::pre_commit_hook_script`).
    #[allow(clippy::too_many_arguments)]
    fn install_pre_commit_hook(
        &self,
        check_cmd: &str,
        memo_cmd: &str,
        recipe: Option<&crate::recipe::Recipe>,
        pins: &[String],
        hive_home: &str,
        project: &str,
        agent: &str,
        base: &str,
    ) -> Result<()>;
    /// Where git fetches the run's commits from. `cancel` ends whatever
    /// the machine is asked for to make it (a dial hand's bundle).
    fn fetch_source(&self, cancel: &hive_bus::CancellationToken) -> String;
    fn fetch_env(&self) -> Vec<String>;
    /// A second way to the run's commits when the fetch from
    /// [`fetch_source`](Self::fetch_source) failed: a git bundle file the
    /// machine made with its `bundle` verb, or empty when there is none. An
    /// ssh machine has it: a hand whose `git-upload-pack` does not answer
    /// (every hand before TASK-133) still sends its commits this way.
    /// `cancel` ends the machine's bundle call, like the fetch it stands in
    /// for (review of PR 283 item 3: the daemon's pass could not stop it).
    fn fetch_fallback(&self, cancel: &hive_bus::CancellationToken) -> String {
        let _ = cancel;
        String::new()
    }
    /// How a person gets the run's commits from the machine by hand, for a
    /// failure that could not fetch them; empty when the executor cannot say.
    fn fetch_by_hand(&self, root: &str, branch: &str) -> String {
        let _ = (root, branch);
        String::new()
    }
    /// Ends a run whose commits could not be fetched: the clone stays on the
    /// machine for whoever fetches it ([`fetch_by_hand`](Self::fetch_by_hand)),
    /// only what the brain holds for the run goes. Executors whose clone is
    /// not elsewhere clean up as usual.
    fn keep_work(&self) -> Vec<crate::tidy::Leftover> {
        self.cleanup()
    }
    /// Whether the run's accepted commits are pushed from where they were
    /// made ([`push_from_hand`](Self::push_from_hand)) rather than by the
    /// brain's delivery: a local `hive-hand` (`executor.transport
    /// local-hand`) does, every other executor does not.
    fn pushes(&self) -> bool {
        false
    }
    /// Pushes `sha` from the run's clone to `refs/heads/{head}` at `url`
    /// with `key`, the credential the brain leased for this push (empty:
    /// what git there already uses). Says what it did.
    fn push_from_hand(&self, url: &str, sha: &str, head: &str, key: &[u8]) -> Result<String> {
        let _ = (url, sha, head, key);
        Err(anyhow!("executor {} does not push", self.name()))
    }
    /// Removes the run's clone and whatever the agent left registered on it;
    /// returns the leftovers so the run can write them down against the agent.
    fn cleanup(&self) -> Vec<crate::tidy::Leftover>;
    /// Carries a held run on in the clone its earlier run left: `prepare`
    /// keeps that clone instead of making a new one and reports `base`, the
    /// earlier run's base, as its own. Executors that cannot do this ignore
    /// the request and clone anew.
    fn set_resume(&mut self, base: &str) {
        let _ = base;
    }
    /// Ends a held run: stray processes go, the clone, its temp dir and its
    /// signing key stay for the run that carries on. Executors that cannot
    /// keep a clone clean up as usual, and the next run starts over.
    fn hold(&self) -> Vec<crate::tidy::Leftover> {
        self.cleanup()
    }
}
// The run's log, the clone's preparation, the worker's run and the
// clone's cleanup live with the executor path (hive-hand-core, ADR-070
// step 1d); LocalExec feeds them plain values from its rows.
#[allow(unused_imports)]
pub(crate) use hive_hand_core::local::{RunLog, preflight_run_key, worker_dirs};
pub(crate) fn tail_str(s: &str, n: usize) -> String {
    crate::executor::tail(s, n)
}
fn ssh_options() -> Vec<String> {
    crate::executor::ssh_options()
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct LocalExec {
    pub(crate) row: Row,
    #[serde(skip)]
    pub(crate) s: Option<hive_store::Store>,
    pub(crate) home: String,
    pub(crate) job: Row,
    pub(crate) project: Row,
    pub(crate) agent: String,
    #[serde(skip)]
    pub(crate) log: RunLog,
    pub(crate) root: String,
    pub(crate) clone_dir: String,
    pub(crate) key: String,
    pub(crate) begun: bool,
    pub(crate) last_out: Option<String>,
    /// The base of the held run whose clone this run carries on in.
    #[serde(default)]
    pub(crate) resume_base: Option<String>,
    /// `brain.code none` (ADR-070 rule 6): the run gets an empty workspace
    /// instead of a clone of the project ([`prepare_no_code_workspace`]).
    #[serde(default)]
    pub(crate) no_code: bool,
}

/// Prepares the workspace of a run on a brain without code (`brain.code
/// none`): an empty repository with one empty commit (the run's base, so
/// the commit count, the dirty check and the hooks work as in a clone) and
/// the run's identity, and not one file of the project. Returns the base.
pub(crate) fn prepare_no_code_workspace(
    home: &str,
    dir: &str,
    agent: &str,
    key: &str,
    hook: &str,
) -> Result<String> {
    if Path::new(dir).exists() {
        let _ = hive_hand_core::trash::discard_dir(Path::new(home), Path::new(dir));
    }
    std::fs::create_dir_all(dir)?;
    crate::util::git_check(dir, &["init", "-q", "-b", "main"])?;
    crate::util::git_check(
        dir,
        &[
            "-c",
            "user.name=hive",
            "-c",
            "user.email=hive@hive.local",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "workspace (brain.code none: no project checkout on this machine)",
        ],
    )?;
    hive_hand_core::local::configure_clone_identity(dir, agent, key, hook)?;
    Ok(crate::util::git_check(dir, &["rev-parse", "HEAD"])?
        .stdout
        .trim()
        .to_string())
}

/// True when the recipe's `CARGO_TARGET_DIR` points outside the clone: that
/// target dir may hold artifacts built from another tree's checkout, and
/// Cargo would serve them as fresh for this clone's check because its
/// freshness key is path-insensitive (mtime + fingerprint, not the source's
/// canonical path). A relative value lands inside the clone and can only
/// hold this tree's work, so no freshening is needed there.
pub(crate) fn cargo_target_dir_is_outside_clone(target_dir: &str, clone_dir: &str) -> bool {
    if target_dir.trim().is_empty() {
        return false;
    }
    let target = Path::new(target_dir);
    if !target.is_absolute() {
        return false;
    }
    !target.starts_with(Path::new(clone_dir))
}

/// True when the project's check builds into a target dir outside the clone:
/// the recipe's `CARGO_TARGET_DIR`, or the one the verify command names itself
/// (`env CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/shared/land} ... cargo test`,
/// resolved the way the check's shell would). A recipe that leaves the
/// variable to the verify command still shares that dir with every other
/// clone and landing of the project, so either source counts.
pub(crate) fn check_shares_cargo_target_dir(
    recipe_target_dir: Option<&str>,
    verify_cmd: &str,
    clone_dir: &str,
) -> bool {
    let verify_target_dir = crate::sweep::cargo_target_dir_from_verify(verify_cmd);
    [recipe_target_dir, verify_target_dir.as_deref()]
        .into_iter()
        .flatten()
        .any(|dir| cargo_target_dir_is_outside_clone(dir, clone_dir))
}

/// `check` with the touch that keeps a shared Cargo target dir honest in
/// front of it: bump the mtime of every tracked source Cargo could reuse a
/// stale artifact for, so the check rebuilds the workspace crates from the
/// tree it stands in and never runs another tree's binary. Cargo hashes a
/// workspace member by its path relative to the workspace root and judges
/// freshness by mtime, so two clones building into one dir
/// (`~/.hive/cache/cargo-target-land/<slug>`) share artifacts, and the one
/// whose sources are older is served the other's. Relative to the shell's
/// working directory, so one line serves the clone, the base worktree the
/// run checks out beside it (`base_worktree_command`) and the hook, and
/// meant to stand inside the verify lock (`verify_lock::wrap`), after the
/// wait: the install-time touch it replaces ran hours before the hook's
/// check, and the touch before the run-end verify ran before a lock wait of
/// an hour or more; a tree that built in between left newer artifacts, and
/// on 2026-11-04 two brain runs (job-20261104T053139Z-4a52f6 and -f74846)
/// passed their pre-commit check on another clone's test binaries, then
/// failed the run-end check - one with a compile error no flake explains -
/// after a day's work each. Registry dependencies are untouched, so the cost
/// is one workspace rebuild per check, not a cold target dir; a failed touch
/// never blocks the check, one stderr line says so and the check runs as it
/// is. `whose` names the tree in that line ("the clone's", "the base
/// tree's").
pub(crate) fn freshen_before_check(check: &str, whose: &str) -> String {
    format!(
        "(git ls-files -z -- '*.rs' 'Cargo.toml' '**/Cargo.toml' 'build.rs' '**/build.rs' | xargs -0 -r touch -c) 2>/dev/null || echo \"hive: could not touch {whose} tracked sources; the check runs as it is\" >&2\n{check}"
    )
}
pub(crate) fn new_local_exec(
    r: Row,
    s: &hive_store::Store,
    home: &str,
    job: Row,
    project: Row,
    agent: &str,
    log: &mut RunLog,
) -> LocalExec {
    let job_id = job.str("id");
    let root = project.str("path");
    LocalExec {
        row: r,
        s: Some(s.clone()),
        home: home.into(),
        job,
        project,
        agent: agent.into(),
        log: log.clone(),
        root,
        clone_dir: std::path::Path::new(home)
            .join("work")
            .join(&job_id)
            .display()
            .to_string(),
        key: std::path::Path::new(home)
            .join("work")
            .join(format!("{job_id}.key"))
            .display()
            .to_string(),
        ..Default::default()
    }
}
impl LocalExec {
    pub(crate) fn name(&self) -> String {
        self.row.str("name")
    }
    /// Runs git in the clone; a failure is written to the job log with
    /// everything git said - its exit code, stderr, stdout (`commit` reports
    /// "nothing to commit" there) and a timeout or spawn error - so a
    /// failed runner commit is never unexplained in the log.
    pub(crate) fn clone_git(&self, args: &[&str]) -> Result<crate::util::GitResult> {
        let r = crate::util::git(&self.clone_dir, args);
        if r.ok() {
            return Ok(r);
        }
        let cause = git_failure_cause(&r);
        let words = args.first().copied().unwrap_or("");
        let line = format!(
            "git {words} failed in the clone {} (exit {}): {cause}",
            self.clone_dir, r.code
        );
        self.log.add(line.clone());
        Err(anyhow!(line))
    }
}

/// What a failed git said, in one line: its error, stderr and stdout,
/// whichever are there, or that it said nothing.
fn git_failure_cause(r: &crate::util::GitResult) -> String {
    let parts: Vec<String> = [
        r.err.clone().unwrap_or_default(),
        r.stderr.trim().to_string(),
        r.stdout.trim().to_string(),
    ]
    .into_iter()
    .filter(|p| !p.is_empty())
    .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
    .collect();
    if parts.is_empty() {
        "git printed nothing".to_string()
    } else {
        crate::util::trunc(&parts.join("; "), 600)
    }
}
impl LocalExec {
    pub(crate) fn branch_to_clone(&self) -> String {
        let d = self.project.str("default_branch");
        if !d.is_empty() && d != "HEAD" {
            d
        } else {
            String::new()
        }
    }
}
impl Executor for LocalExec {
    fn name(&self) -> String {
        self.row.str("name")
    }
    fn remote(&self) -> bool {
        false
    }
    fn clone(&self) -> String {
        self.clone_dir.clone()
    }
    fn key_path(&self) -> String {
        self.key.clone()
    }
    fn started(&self) -> bool {
        self.begun
    }
    fn worker_dirs(&self) -> Vec<String> {
        worker_dirs(&self.home)
    }
    fn prepare(&mut self, cancel: &hive_bus::CancellationToken) -> Result<String> {
        if self.no_code {
            self.log.add(
                "brain.code is none: an empty workspace, no checkout of the project".to_string(),
            );
            return prepare_no_code_workspace(
                &self.home,
                &self.clone_dir,
                &self.agent,
                &self.key,
                &hive_hand_core::provenance::provenance_hook(
                    &self.agent,
                    &self.job.str("id"),
                    &self.name(),
                ),
            );
        }
        let prep = hive_hand_core::local::ClonePrep {
            home: self.home.clone(),
            root: self.root.clone(),
            clone_dir: self.clone_dir.clone(),
            branch: self.branch_to_clone(),
            key: self.key.clone(),
            agent: self.agent.clone(),
            job_id: self.job.str("id"),
            executor: self.name(),
            resume_base: self.resume_base.take(),
        };
        hive_hand_core::local::prepare_clone(cancel, &prep, &self.log)
    }
    fn run_worker(
        &mut self,
        cancel: &hive_bus::CancellationToken,
        cmd: &[String],
        env: &BTreeMap<String, String>,
        minutes: i32,
    ) -> Result<Outcome> {
        // The clone and key are copied out: the observer closure below
        // borrows the executor mutably while the run borrows these.
        let (clone_dir, key) = (self.clone_dir.clone(), self.key.clone());
        let run = hive_hand_core::local::WorkerRun {
            clone_dir: &clone_dir,
            key: &key,
            cmd,
            env,
            minutes,
        };
        let out = hive_hand_core::local::run_worker(cancel, &run, || {
            self.begun = true;
            let id = self.execution(env.clone());
            self.observer(id)
        })?;
        self.last_out = Some(out.stdout.clone());
        Ok(out)
    }
    fn result_text(&self) -> Option<String> {
        self.last_out.clone()
    }
    fn commit_count(&self, base: &str) -> i32 {
        crate::util::git(
            &self.clone_dir,
            &["rev-list", "--count", &format!("{base}..HEAD")],
        )
        .stdout
        .trim()
        .parse()
        .unwrap_or(0)
    }
    fn dirty(&self) -> String {
        crate::util::git(
            &self.clone_dir,
            &["status", "--porcelain", "--untracked-files=no"],
        )
        .stdout
        .trim()
        .to_string()
    }
    fn untracked(&self) -> String {
        crate::util::git(
            &self.clone_dir,
            &["ls-files", "--others", "--exclude-standard"],
        )
        .stdout
        .trim()
        .to_string()
    }
    fn commit_left_work(&self, message: &str) -> Result<()> {
        // A commit killed mid-hook can leave the index lock behind; the
        // worker's processes are gone by now, so the lock guards nothing.
        let lock = Path::new(&self.clone_dir).join(".git").join("index.lock");
        if lock.exists() {
            let _ = std::fs::remove_file(&lock);
        }
        self.clone_git(&["add", "-u"])?;
        // `--no-verify` skips pre-commit and commit-msg; the clone's
        // prepare-commit-msg hook still adds the provenance trailers and
        // the included identity still signs with the run key (ADR-022).
        self.clone_git(&["commit", "-q", "--no-verify", "-m", message])?;
        Ok(())
    }
    fn owner_paths_touched(&self, base: &str) -> Vec<String> {
        crate::brief::owner_paths_touched(self.clone_dir.clone(), base.to_string())
    }
    fn changed_files(&self, base: &str) -> Vec<String> {
        crate::brief::changed_files(self.clone_dir.clone(), base.to_string())
    }
    fn run_tmp_dir(&self) -> String {
        format!("{}.tmp", self.clone_dir)
    }
    fn verify(
        &self,
        cancel: &hive_bus::CancellationToken,
        cmd: &str,
        minutes: i32,
    ) -> (Option<i32>, String) {
        let secs = (minutes.max(1) as u64 * 60).max(60);
        // The check writes its temporary files beside the clone, like the
        // worker did; a recipe that exports TMPDIR itself still wins.
        let tmp = self.run_tmp_dir();
        let _ = std::fs::create_dir_all(&tmp);
        let env = crate::proc::env_with(
            &crate::proc::environ(),
            &BTreeMap::from([("TMPDIR".to_string(), tmp)]),
        );
        let run = self.beat_during("verify", || {
            crate::proc::run_group(
                cancel,
                &["/bin/sh".into(), "-c".into(), cmd.into()],
                &self.clone_dir,
                &env,
                &[],
                Duration::from_secs(secs),
            )
        });
        match run {
            Ok(out) if out.timeout => (None, String::new()),
            Ok(out) => (
                Some(out.exit_code),
                format!(
                    "{}{}",
                    tail_str(&out.stdout, 20000),
                    tail_str(&out.stderr, 20000)
                ),
            ),
            Err(e) => (Some(127), e.to_string()),
        }
    }
    fn install_pre_commit_hook(
        &self,
        check_cmd: &str,
        memo_cmd: &str,
        recipe: Option<&crate::recipe::Recipe>,
        pins: &[String],
        hive_home: &str,
        project: &str,
        agent: &str,
        base: &str,
    ) -> Result<()> {
        if check_cmd.is_empty() {
            return Ok(());
        }
        crate::keys::write_pre_commit_hook(
            &self.clone_dir,
            &crate::keys::pre_commit_hook_script(
                check_cmd,
                memo_cmd,
                recipe,
                pins,
                &self.clone_dir,
                hive_home,
                project,
                agent,
                base,
            ),
        )
    }
    fn fetch_source(&self, _cancel: &hive_bus::CancellationToken) -> String {
        self.clone_dir.clone()
    }
    fn fetch_env(&self) -> Vec<String> {
        crate::util::git_env(&[])
    }
    fn cleanup(&self) -> Vec<crate::tidy::Leftover> {
        hive_hand_core::local::cleanup_clone(
            &hive_hand_core::local::CloneCleanup {
                home: self.home.clone(),
                clone_dir: self.clone_dir.clone(),
                run_tmp_dir: self.run_tmp_dir(),
                key: self.key.clone(),
            },
            &self.log,
        )
    }
    fn set_resume(&mut self, base: &str) {
        self.resume_base = Some(base.to_string());
    }
    fn hold(&self) -> Vec<crate::tidy::Leftover> {
        hive_hand_core::local::hold_clone(&self.clone_dir, &self.log);
        Vec::new()
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct SshExec {
    pub(crate) local: LocalExec,
    pub(crate) job_dir: String,
    pub(crate) bundle: String,
    /// The commit `prepare` reported: a hand that dials in (ADR-070 step 2)
    /// sends its commits back as a bundle past it.
    #[serde(default)]
    pub(crate) base: String,
}
static SHA_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9a-f]{40,64}$").unwrap());

pub(crate) fn new_ssh_exec(
    r: Row,
    s: &hive_store::Store,
    home: &str,
    job: Row,
    project: Row,
    agent: &str,
    log: &mut RunLog,
) -> SshExec {
    SshExec {
        local: {
            let mut l = new_local_exec(r.clone(), s, home, job.clone(), project, agent, log);
            let job_dir = format!(
                "{}/{}",
                r.str("workdir").trim_end_matches('/'),
                job.str("id")
            );
            l.clone_dir = format!("{job_dir}/clone");
            l
        },
        job_dir: format!(
            "{}/{}",
            r.str("workdir").trim_end_matches('/'),
            job.str("id")
        ),
        bundle: std::path::Path::new(home)
            .join("work")
            .join(format!("{}.bundle", job.str("id")))
            .display()
            .to_string(),
        base: String::new(),
    }
}
impl SshExec {
    pub(crate) fn name(&self) -> String {
        self.local.name()
    }
    /// Asks a dial hand for the clone's commits past the base (`bundle`
    /// verb) and writes them to `<bundle>.result`, the path git fetches.
    /// A failure is logged and leaves no file, so the fetch says so;
    /// `cancel` ends the call.
    fn dial_result_bundle(&self, cancel: &hive_bus::CancellationToken) -> String {
        use base64::Engine;
        let path = format!("{}.result", self.bundle);
        let _ = std::fs::remove_file(&path);
        match self.call_until(
            cancel,
            &["bundle".into(), self.local.job.str("id"), self.base.clone()],
            &[],
            Duration::from_secs(600),
        ) {
            Ok((0, out, _)) => match base64::engine::general_purpose::STANDARD.decode(out.trim()) {
                Ok(data) if !data.is_empty() => {
                    if let Some(dir) = std::path::Path::new(&path).parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    if let Err(e) = std::fs::write(&path, data) {
                        self.local
                            .log
                            .add(format!("result bundle not written: {e}"));
                    }
                }
                Ok(_) => self
                    .local
                    .log
                    .add("the hand reported no commits past the base"),
                Err(e) => self.local.log.add(format!("result bundle unreadable: {e}")),
            },
            Ok((rc, out, err)) => self.local.log.add(format!(
                "bundle on {} failed (exit {rc}): {}",
                self.name(),
                tail_str(if err.trim().is_empty() { &out } else { &err }, 300)
            )),
            Err(e) => self
                .local
                .log
                .add(format!("bundle on {} failed: {e}", self.name())),
        }
        path
    }
    pub(crate) fn call(
        &self,
        args: &[String],
        stdin: &[u8],
        timeout: Duration,
    ) -> Result<(i32, String, String)> {
        self.call_until(&hive_bus::CancellationToken::new(), args, stdin, timeout)
    }
    /// [`call`](Self::call) that `cancel` ends: the ssh session is killed
    /// and the error carries the cancellation. The worker and the check run
    /// through this one, so a restart the owner asked for now, a stall or
    /// the stop grace end a run on a hand like one here; through `call`
    /// they could not, and the daemon's exit waited on the hand's budget
    /// (twenty hours for a task) while the hub showed it restarting.
    pub(crate) fn call_until(
        &self,
        cancel: &hive_bus::CancellationToken,
        args: &[String],
        stdin: &[u8],
        timeout: Duration,
    ) -> Result<(i32, String, String)> {
        let (rc, out, err_text) =
            crate::executor::ssh_run_context(cancel, &self.local.row, args, stdin, timeout)?;
        let shown = if args.len() > 4 { &args[..4] } else { args };
        self.local.log.add(format!(
            "$ ssh {} {}\n{}{}",
            self.local.row.str("address"),
            crate::util::shell_join(shown),
            tail_str(&out, 4000),
            tail_str(&err_text, 4000)
        ));
        let src = if err_text.is_empty() { &out } else { &err_text };
        if rc != 0 && src.contains("verb not allowed") {
            let verb = args.first().map(String::as_str).unwrap_or("?");
            return Err(crate::executor::ExecutorFault {
                msg: format!(
                    "hands on {} refuse `{}` ({}) - they predate this hub; refresh them with `hive pair <admin@host> --name {}`",
                    self.name(),
                    verb,
                    tail_str(&src.split_whitespace().collect::<Vec<_>>().join(" "), 120),
                    self.name()
                ),
            }
            .into());
        }
        Ok((rc, out, err_text))
    }
    pub(crate) fn preflight_remote_run_key(&self, when: &str) -> Result<()> {
        let (rc, out, err) = self.call(
            &[
                "verify".into(),
                self.local.job.str("id"),
                "1".into(),
                "--".into(),
                remote_key_check_cmd(&self.remote_key_path()),
            ],
            &[],
            Duration::from_secs(120),
        )?;
        remote_key_check_result(&self.remote_key_path(), &self.name(), when, rc, &out, &err)
    }
    pub(crate) fn remote_key_path(&self) -> String {
        format!("{}/run.key", self.job_dir)
    }
    pub(crate) fn preflight_remote_cli(&self, path: &str) -> Result<()> {
        let (rc, out, err) = self.call(
            &[
                "verify".into(),
                self.local.job.str("id"),
                "1".into(),
                "--".into(),
                remote_cli_check_cmd(path),
            ],
            &[],
            Duration::from_secs(120),
        )?;
        remote_cli_check_result(path, &self.name(), rc, &out, &err)
    }
}
impl Executor for SshExec {
    fn name(&self) -> String {
        self.local.name()
    }
    fn remote(&self) -> bool {
        true
    }
    fn clone(&self) -> String {
        self.local.clone_dir.clone()
    }
    fn key_path(&self) -> String {
        self.local.key_path()
    }
    fn started(&self) -> bool {
        self.local.started()
    }
    fn worker_dirs(&self) -> Vec<String> {
        vec![self.job_dir.clone(), "/tmp".into()]
    }
    fn prepare(&mut self, cancel: &hive_bus::CancellationToken) -> Result<String> {
        let _ = cancel;
        let mut branch = self.local.branch_to_clone();
        if branch.is_empty() {
            let head =
                crate::util::git(&self.local.root, &["symbolic-ref", "--short", "-q", "HEAD"]);
            if head.ok() {
                branch = head.stdout.trim().to_string();
            }
        }
        let reference = if branch.is_empty() {
            "HEAD".to_string()
        } else {
            branch
        };
        let made = crate::util::git_timeout(
            &self.local.root,
            Duration::from_secs(300),
            &["bundle", "create", "-q", &self.bundle, &reference],
        );
        if !made.ok() {
            return Err(
                crate::util::fail(1, format!("bundle failed: {}", made.stderr.trim())).into(),
            );
        }
        let data = std::fs::read(&self.bundle)?;
        let (rc, out, err_text) = self.call(
            &[
                "prepare".into(),
                self.local.job.str("id"),
                reference,
                self.local.agent.clone(),
            ],
            &data,
            Duration::from_secs(600),
        )?;
        let src = if err_text.is_empty() { &out } else { &err_text };
        if rc != 0 {
            return Err(crate::util::fail(
                1,
                format!(
                    "remote prepare failed on {} (exit {rc}): {}",
                    self.name(),
                    tail_str(&src.split_whitespace().collect::<Vec<_>>().join(" "), 300)
                ),
            )
            .into());
        }
        let base = out
            .split_whitespace()
            .last()
            .unwrap_or_default()
            .to_string();
        if !SHA_RE.is_match(&base) {
            return Err(crate::util::fail(
                1,
                format!(
                    "remote prepare on {} returned no base commit: {}",
                    self.name(),
                    crate::py_repr(tail_str(&out, 200))
                ),
            )
            .into());
        }
        let key_data = std::fs::read(&self.local.key)?;
        let (rc, out, err_text) = self.call(
            &["key".into(), self.local.job.str("id"), self.name()],
            &key_data,
            Duration::from_secs(120),
        )?;
        if rc != 0 {
            let src = if err_text.is_empty() { out } else { err_text };
            return Err(crate::executor::ExecutorFault {
                msg: format!(
                    "remote key failed on {} (exit {rc}): {} - the machine cannot take a signing key; probe it with `hive executor probe {}`",
                    self.name(),
                    tail_str(&src.split_whitespace().collect::<Vec<_>>().join(" "), 300),
                    self.name()
                ),
            }
            .into());
        }
        self.preflight_remote_run_key("after upload")?;
        self.base = base.clone();
        Ok(base)
    }
    fn run_worker(
        &mut self,
        cancel: &hive_bus::CancellationToken,
        cmd: &[String],
        env: &BTreeMap<String, String>,
        minutes: i32,
    ) -> Result<Outcome> {
        preflight_run_key(&self.local.key)?;
        self.preflight_remote_run_key("before the worker starts")?;
        if let Ok((0, out, _)) = self.call(&["probe".into()], &[], Duration::from_secs(120)) {
            let fields = out
                .split_whitespace()
                .filter_map(|f| f.split_once('='))
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<BTreeMap<_, _>>();
            if fields.get("hive").map(String::as_str) == Some("missing") {
                let looked = match fields.get("hive_lookup").map(String::as_str) {
                    Some("") | None | Some("-") => "the executor user's login PATH".to_string(),
                    Some(v) => format!("PATH={}", v.replace("%20", " ")),
                };
                return Err(crate::executor::ExecutorFault {
                    msg: format!(
                        "hive is not on PATH on {} (hands probe reported hive=missing, looked in {looked}) - install the hive CLI for the executor user or fix its login PATH; probe it with `hive executor probe {}`",
                        self.name(),
                        self.name()
                    ),
                }
                .into());
            }
        }
        if let Some(cli) = env.get("HIVE_CLI").filter(|s| !s.is_empty()) {
            self.preflight_remote_cli(cli)?;
        }
        let budget = if minutes <= 0 {
            "1s".to_string()
        } else {
            minutes.to_string()
        };
        let mut args = vec!["run".to_string(), self.local.job.str("id"), budget];
        for (k, v) in env {
            args.push(format!("{k}={v}"));
        }
        args.push("--".into());
        args.extend(cmd.iter().cloned());
        self.local.begun = true;
        let called = self.call_until(
            cancel,
            &args,
            &[],
            Duration::from_secs((minutes.max(1) * 60 + 120) as u64),
        );
        // Stopped by the runner before the cap: the same outcome as a worker
        // here (proc.rs), so the run reads as lost and the job goes back to
        // the queue; `cleanup` then tells the hand to clean the job, which
        // ends what the worker left running there.
        if let Err(e) = &called
            && cancel.is_cancelled()
        {
            return Ok(Outcome {
                timeout: true,
                exit_code: -1,
                stdout: String::new(),
                stderr: e.to_string(),
            });
        }
        let (rc, out, err_text) = called?;
        self.local.last_out = Some(out.clone());
        if rc == crate::executor::REMOTE_EXIT_TIMEOUT {
            return Ok(Outcome {
                timeout: true,
                exit_code: -1,
                stdout: out,
                stderr: err_text,
            });
        }
        Ok(Outcome {
            timeout: false,
            exit_code: rc,
            stdout: out,
            stderr: err_text,
        })
    }
    fn result_text(&self) -> Option<String> {
        match self.call(
            &["result".into(), self.local.job.str("id")],
            &[],
            Duration::from_secs(120),
        ) {
            Ok((0, out, _)) => Some(out),
            _ => self.local.last_out.clone(),
        }
    }
    fn commit_count(&self, base: &str) -> i32 {
        match self.call(
            &["count".into(), self.local.job.str("id"), base.into()],
            &[],
            Duration::from_secs(120),
        ) {
            Ok((0, out, _)) => out
                .split_whitespace()
                .last()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            _ => 0,
        }
    }
    fn dirty(&self) -> String {
        match self.call(
            &["dirty".into(), self.local.job.str("id")],
            &[],
            Duration::from_secs(120),
        ) {
            Ok((0, out, _)) => out.trim().to_string(),
            _ => String::new(),
        }
    }
    fn owner_paths_touched(&self, base: &str) -> Vec<String> {
        let mut hit = self.changed_files(base);
        hit.retain(|f| {
            crate::keys::OWNER_PATHS
                .iter()
                .any(|p| f == p || f.starts_with(p))
        });
        hit
    }
    fn changed_files(&self, base: &str) -> Vec<String> {
        let Ok((0, out, _)) = self.call(
            &["changed".into(), self.local.job.str("id"), base.into()],
            &[],
            Duration::from_secs(120),
        ) else {
            return Vec::new();
        };
        let mut files: Vec<String> = out.split_whitespace().map(ToOwned::to_owned).collect();
        files.sort();
        files
    }
    fn verify(
        &self,
        cancel: &hive_bus::CancellationToken,
        cmd: &str,
        minutes: i32,
    ) -> (Option<i32>, String) {
        let budget = if minutes <= 0 {
            "1s".to_string()
        } else {
            minutes.to_string()
        };
        let call = self.local.beat_during("verify", || {
            self.call_until(
                cancel,
                &[
                    "verify".into(),
                    self.local.job.str("id"),
                    budget,
                    "--".into(),
                    cmd.into(),
                ],
                &[],
                Duration::from_secs((minutes.max(1) * 60 + 120) as u64),
            )
        });
        match call {
            // Stopped by the runner before the cap: no verdict, as for a
            // check here, so run.rs reads the run as lost rather than failed.
            _ if cancel.is_cancelled() => (None, String::new()),
            Ok((crate::executor::REMOTE_EXIT_TIMEOUT, out, err)) => (
                None,
                format!("{}{}", tail_str(&out, 20000), tail_str(&err, 20000)),
            ),
            Ok((rc, out, err)) => (
                Some(rc),
                format!("{}{}", tail_str(&out, 20000), tail_str(&err, 20000)),
            ),
            Err(e) => (Some(-1), e.to_string()),
        }
    }
    fn install_pre_commit_hook(
        &self,
        check_cmd: &str,
        memo_cmd: &str,
        recipe: Option<&crate::recipe::Recipe>,
        pins: &[String],
        hive_home: &str,
        project: &str,
        agent: &str,
        base: &str,
    ) -> Result<()> {
        if check_cmd.is_empty() {
            return Ok(());
        }
        let script = crate::keys::pre_commit_hook_script(
            check_cmd,
            memo_cmd,
            recipe,
            pins,
            &self.local.clone_dir,
            hive_home,
            project,
            agent,
            base,
        );
        let (rc, out, err) = self.call(
            &["hook".into(), self.local.job.str("id")],
            script.as_bytes(),
            Duration::from_secs(120),
        )?;
        if rc != 0 {
            let src = if err.is_empty() { out } else { err };
            return Err(crate::util::fail(
                1,
                format!(
                    "remote pre-commit hook install on {} failed (exit {rc}): {}",
                    self.name(),
                    tail_str(&src.split_whitespace().collect::<Vec<_>>().join(" "), 300)
                ),
            )
            .into());
        }
        Ok(())
    }
    fn pushes(&self) -> bool {
        self.local.row.str("kind") == crate::executor::HAND_KIND
    }
    fn push_from_hand(&self, url: &str, sha: &str, head: &str, key: &[u8]) -> Result<String> {
        let (rc, out, err) = self.call(
            &[
                "push".into(),
                self.local.job.str("id"),
                url.to_string(),
                sha.to_string(),
                head.to_string(),
            ],
            key,
            Duration::from_secs(330),
        )?;
        if rc != 0 {
            let src = if err.trim().is_empty() { out } else { err };
            return Err(anyhow!(
                "{}",
                hive_cred::redact(
                    &format!(
                        "push of {} to {head} from {} failed (exit {rc}): {}",
                        crate::recipe::short(sha, 12),
                        self.name(),
                        tail_str(&src.split_whitespace().collect::<Vec<_>>().join(" "), 300)
                    ),
                    &[],
                )
            ));
        }
        Ok(out.trim().to_string())
    }
    fn fetch_source(&self, cancel: &hive_bus::CancellationToken) -> String {
        // A local hive-hand's clone is on this machine: git reads it as a
        // path, no sshd in between.
        if self.pushes() {
            return self.local.clone_dir.clone();
        }
        // A hand that dials in has no sshd to fetch from: its commits come
        // back as a bundle past the base, which git fetches as a file.
        if self.local.row.str("kind") == crate::dial::DIAL_KIND {
            return self.dial_result_bundle(cancel);
        }
        match crate::executor::parse_ssh_address(&self.local.row.str("address")) {
            Ok((user, host, port)) => {
                format!("ssh://{user}@{host}:{port}{}", self.local.clone_dir)
            }
            Err(_) => format!("ssh://{}", self.local.clone_dir),
        }
    }
    fn fetch_env(&self) -> Vec<String> {
        if self.pushes() {
            return crate::util::git_env(&[]);
        }
        crate::proc::env_with(
            &crate::util::git_env(&[]),
            &BTreeMap::from([(
                "GIT_SSH_COMMAND".to_string(),
                crate::util::shell_join(
                    &std::iter::once("ssh".to_string())
                        .chain(ssh_options())
                        .collect::<Vec<_>>(),
                ),
            )]),
        )
    }
    fn fetch_fallback(&self, cancel: &hive_bus::CancellationToken) -> String {
        // A local hand's clone is a path and a dial hand's answer already is
        // the bundle: neither has a second way.
        if self.pushes()
            || self.local.row.str("kind") == crate::dial::DIAL_KIND
            || !SHA_RE.is_match(&self.base)
        {
            return String::new();
        }
        let path = self.dial_result_bundle(cancel);
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > 0) {
            path
        } else {
            String::new()
        }
    }
    fn fetch_by_hand(&self, root: &str, branch: &str) -> String {
        if self.pushes() || self.local.row.str("kind") == crate::dial::DIAL_KIND {
            return String::new();
        }
        let job = self.local.job.str("id");
        let mut line = format!(
            "the clone stays on {name} at {clone} until it is fetched or {name} is released; get the work with `git -C {root} fetch {src} +HEAD:refs/heads/{branch}`",
            name = self.name(),
            clone = self.local.clone_dir,
            // Neither a local hand nor a dial hand gets here: the source
            // is the ssh address, made without a call to the machine.
            src = self.fetch_source(&hive_bus::CancellationToken::new()),
        );
        // The bundle verb answers on a hand whose upload-pack does not.
        if SHA_RE.is_match(&self.base)
            && let Ok(argv) = crate::executor::ssh_command(
                &self.local.row,
                &["bundle".into(), job, self.base.clone()],
            )
        {
            line.push_str(&format!(
                ", or as a bundle with `{} | base64 -d > work.bundle && git -C {root} fetch $PWD/work.bundle +HEAD:refs/heads/{branch}`",
                crate::util::shell_join(&argv)
            ));
        }
        line
    }
    fn keep_work(&self) -> Vec<crate::tidy::Leftover> {
        let _ = std::fs::remove_file(&self.bundle);
        let _ = std::fs::remove_file(format!("{}.result", self.bundle));
        crate::keys::remove_run_key(&self.local.key);
        if self.pushes() || self.local.row.str("kind") == crate::dial::DIAL_KIND {
            return self.cleanup();
        }
        self.local.log.add(format!(
            "the clone on {} is kept: its commits were not fetched (TASK-133)",
            self.name()
        ));
        Vec::new()
    }
    fn cleanup(&self) -> Vec<crate::tidy::Leftover> {
        let _ = std::fs::remove_file(&self.bundle);
        let _ = std::fs::remove_file(format!("{}.result", self.bundle));
        crate::keys::remove_run_key(&self.local.key);
        match self.call(
            &["clean".into(), self.local.job.str("id")],
            &[],
            Duration::from_secs(120),
        ) {
            Ok((_, out, _)) => crate::tidy::parse_leftover_lines(&out),
            Err(err) => {
                self.local.log.add(format!(
                    "remote clean skipped: {err} (the next probe reports leftovers)"
                ));
                Vec::new()
            }
        }
    }
}
pub(crate) fn remote_key_check_result(
    path: &str,
    host: &str,
    when: &str,
    rc: i32,
    out: &str,
    err_text: &str,
) -> Result<()> {
    if rc == 0 {
        return Ok(());
    }
    let src = if err_text.is_empty() { out } else { err_text };
    Err(crate::util::fail(1, format!("setup-error: the run's signing key {path} on {host} is missing or empty {when} (exit {rc}): {}; the worker was not started so no work is lost (ADR-022)", tail_str(&src.split_whitespace().collect::<Vec<_>>().join(" "), 300))).into())
}
pub(crate) fn remote_key_check_cmd(path: &str) -> String {
    format!("test -s {}", crate::util::shell_quote(path))
}
pub(crate) fn remote_cli_check_cmd(path: &str) -> String {
    format!("test -x {}", crate::util::shell_quote(path))
}
pub(crate) fn remote_cli_check_result(
    path: &str,
    host: &str,
    rc: i32,
    out: &str,
    err_text: &str,
) -> Result<()> {
    if rc == 0 {
        return Ok(());
    }
    let src = if err_text.is_empty() { out } else { err_text };
    Err(crate::util::fail(1, format!("setup-error: HIVE_CLI {path} on {host} is missing or not executable (exit {rc}): {}; the worker was not started so no work is lost - stage the hive binary there or fix the recipe's HIVE_CLI", tail_str(&src.split_whitespace().collect::<Vec<_>>().join(" "), 300))).into())
}
pub(crate) fn make_executor(
    r: Row,
    s: &hive_store::Store,
    home: &str,
    job: Row,
    project: Row,
    agent: &str,
    log: &mut RunLog,
) -> Result<Box<dyn Executor>> {
    // `brain.code none` (ADR-070 rule 6): no run here clones the project.
    // The lease already keeps the other kinds off this machine; a job run
    // by hand (`hive run --job`) is refused the same way.
    if crate::jobs::needs_code_here(s, &r.str("name"), &job.str("kind")) {
        return Err(crate::util::fail(
            2,
            format!(
                "brain.code is none: this machine holds no project code, so the {} job {} runs on a hand, not on {}",
                job.str("kind"),
                job.str("id"),
                r.str("name")
            ),
        )
        .into());
    }
    let no_code =
        r.str("name") == crate::executor::LOCAL_EXECUTOR && crate::jobs::brain_code_none(s);
    let r = crate::executor::transport_row(s, r, &job.str("kind"));
    if r.str("kind") == "local" {
        let mut ex = new_local_exec(r, s, home, job, project, agent, log);
        ex.no_code = no_code;
        return Ok(Box::new(ex));
    }
    if r.str("kind") == "ssh"
        || r.str("kind") == crate::executor::HAND_KIND
        || r.str("kind") == crate::dial::DIAL_KIND
    {
        return Ok(Box::new(new_ssh_exec(r, s, home, job, project, agent, log)));
    }
    Err(crate::util::fail(
        2,
        format!(
            "Executor {} has unknown kind {:?}",
            r.str("name"),
            r.str("kind")
        ),
    )
    .into())
}
/// Points the harness worker at the held run's trajectory: it carries that
/// conversation on and opens with the brief as its continuation message.
pub(crate) fn with_resume(mut cmd: Vec<String>, trajectory: &str) -> Vec<String> {
    cmd.extend(["--resume".to_string(), trajectory.to_string()]);
    cmd
}

/// The harness command with its gated-commit cap set to `seconds`: the value
/// after `--commit-timeout` replaced, or the flag added when the command
/// carries none. For a run starting over its project's narrowed work lanes
/// (`jobs::commit_cap_over_lanes`), whose commit waits for the other checks
/// under the verify lock before its own.
pub(crate) fn with_commit_cap(mut cmd: Vec<String>, seconds: i64) -> Vec<String> {
    let value = seconds.to_string();
    match cmd.iter().position(|a| a == "--commit-timeout") {
        Some(i) if i + 1 < cmd.len() => cmd[i + 1] = value,
        Some(_) => cmd.push(value),
        None => cmd.extend(["--commit-timeout".to_string(), value]),
    }
    cmd
}

pub(crate) fn worker_command(
    s: &hive_store::Store,
    kind: &str,
    brief_text: &str,
    clone: &str,
    worker_cmd: &str,
    remote: bool,
) -> (Vec<String>, bool) {
    worker_command_as(s, kind, "", brief_text, clone, worker_cmd, remote, None)
}

/// Whether the worker gets `--agent NAME` for this spec: a local worker
/// has the store and the owner's home, a remote hand only its clone, so a
/// spec that lives in a record or in `~/.hive/agents` cannot be named to
/// it (its model, step cap and read-only flag still travel as flags).
pub(crate) fn agent_flag_travels(spec: &hive_harness::agents::AgentDef, remote: bool) -> bool {
    !remote || spec.layer.in_clone()
}

/// Tests: the program a store's (by home) native worker runs instead of
/// `hive harness run`, so a test drives `run::run_job` down the native
/// harness's path (its `--model`, its session) with a script that answers.
#[cfg(test)]
pub(crate) static TEST_HARNESS_BIN: std::sync::LazyLock<
    parking_lot::Mutex<BTreeMap<String, String>>,
> = std::sync::LazyLock::new(|| parking_lot::Mutex::new(BTreeMap::new()));

/// `worker_command` for a run whose role asks for a model class of its own
/// (`model_class`, empty for the kind's own model) and whose card an agent
/// file selects (`spec`, see [`crate::agent_specs::spec_for_job`]): the
/// worker runs as that agent, with its model when `models.allowed` lets it
/// through, its step cap when lower than the kind's, read-only when it
/// says so.
#[allow(clippy::too_many_arguments)]
pub(crate) fn worker_command_as(
    s: &hive_store::Store,
    kind: &str,
    model_class: &str,
    brief_text: &str,
    clone: &str,
    worker_cmd: &str,
    remote: bool,
    spec: Option<&hive_harness::agents::AgentDef>,
) -> (Vec<String>, bool) {
    if !worker_cmd.is_empty() {
        return (crate::util::split_shell(worker_cmd), true);
    }
    let bin = if remote {
        "hive".to_string()
    } else {
        std::env::current_exe()
            .ok()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "hive".into())
    };
    #[cfg(test)]
    let bin = TEST_HARNESS_BIN.lock().get(&s.home).cloned().unwrap_or(bin);
    let mut cmd = vec![
        bin,
        "harness".into(),
        "run".into(),
        "-p".into(),
        brief_text.into(),
        "-C".into(),
        clone.into(),
        "--max-steps".into(),
        worker_max_steps(s, kind, spec),
        "--step-timeout".into(),
        s.setting("harness.step_timeout", ""),
        "--commit-timeout".into(),
        s.setting("harness.commit_timeout", ""),
        "--output-limit".into(),
        s.setting("harness.output_limit", ""),
        "--max-cost".into(),
        s.setting("harness.max_cost", ""),
        "--keep-observations".into(),
        s.setting("harness.keep_observations", ""),
    ];
    // The override travels only when set, so a remote hand without
    // the store still honours it and the default argv stays short.
    if crate::util::setting_int(s, "harness.context_window") > 0 {
        cmd.extend([
            "--context-window".into(),
            crate::util::setting_int(s, "harness.context_window").to_string(),
        ]);
    }
    if crate::util::setting_int(s, "minutes_per_job") > 1 {
        cmd.extend([
            "--minutes".into(),
            (crate::util::setting_int(s, "minutes_per_job") - 1).to_string(),
        ]);
    }
    // Likewise the Wave 2 switches: only a departure from the default
    // travels.
    for (setting, flag) in [
        ("harness.skills", "--no-skills"),
        ("harness.hooks", "--no-hooks"),
        ("harness.subagents", "--no-subagents"),
        ("harness.memory", "--no-memory"),
        ("harness.tasks", "--no-tasks"),
    ] {
        if s.setting(setting, "on") == "off" {
            cmd.push(flag.into());
        }
    }
    if s.setting("harness.web_search", "chat") == "on" {
        cmd.push("--web-search".into());
    }
    if s.setting("harness.teach", "off") == "on" {
        cmd.push("--teach".into());
    }
    let fetch_allow = s.setting("harness.fetch_allow", "");
    if !fetch_allow.trim().is_empty() {
        cmd.extend(["--fetch-allow".into(), fetch_allow]);
    }
    for (setting, flag) in [
        ("harness.allow_tools", "--allow-tool"),
        ("harness.deny_tools", "--deny-tool"),
    ] {
        let rules = s.setting(setting, "");
        if !rules.trim().is_empty() {
            cmd.extend([flag.into(), rules.replace('\n', ",")]);
        }
    }
    // The compaction knobs travel only when they depart from the defaults
    // the harness has anyway, so a remote hand without the store compacts
    // the way the hub does.
    for (setting, flag) in [
        ("harness.compact_at", "--compact-at"),
        ("provider.max_request_mb", "--max-request-mb"),
        ("harness.compact_model", "--compact-model"),
    ] {
        let default = hive_store::HARNESS_SETTINGS
            .get(setting)
            .cloned()
            .unwrap_or_default();
        let value = s.setting(setting, "");
        if value.trim() != default.trim() {
            cmd.extend([flag.into(), value]);
        }
    }
    let reasoning = s.run_reasoning_class(kind, model_class);
    if !reasoning.is_empty() && reasoning != "default" {
        cmd.extend(["--reasoning".into(), reasoning]);
    }
    // The jobs that look but do not touch run read-only: no edit or write
    // tool, commands that would change the clone refused.
    if hive_store::settings::READ_ONLY_JOB_KINDS.contains(&kind)
        || spec.is_some_and(|a| a.read_only)
    {
        cmd.push("--read-only".into());
    }
    if let Some(a) = spec
        && agent_flag_travels(a, remote)
    {
        cmd.extend(["--agent".into(), a.name.clone()]);
    }
    let model = worker_model(s, kind, model_class, spec);
    if !model.is_empty() {
        cmd.extend(["--model".into(), model]);
    }
    (cmd, false)
}

/// The step cap a worker gets: the kind's (`harness.max_steps`,
/// `harness.task_max_steps`), lowered to the agent's `max_steps` when the
/// job runs as one; an agent's cap also applies when the kind has none.
fn worker_max_steps(
    s: &hive_store::Store,
    kind: &str,
    spec: Option<&hive_harness::agents::AgentDef>,
) -> String {
    let kinds = s.run_max_steps(kind);
    let kind_cap = hive_harness::parse_max_steps(&kinds).unwrap_or(hive_harness::DEFAULT_MAX_STEPS);
    match spec.and_then(|a| a.max_steps) {
        Some(cap) if i64::from(cap) < i64::from(kind_cap) => cap.to_string(),
        _ => kinds,
    }
}

/// The model a worker gets: the agent's when the job runs as one and
/// `models.allowed` lets it through, else the kind's and class's
/// (`run_model_class`).
pub(crate) fn worker_model(
    s: &hive_store::Store,
    kind: &str,
    model_class: &str,
    spec: Option<&hive_harness::agents::AgentDef>,
) -> String {
    if let Some(m) = spec.and_then(|a| a.model.as_deref())
        && hive_harness::agents::model_allowed(&s.setting("models.allowed", ""), m)
    {
        return m.to_string();
    }
    s.run_model_class(kind, model_class)
}
pub(crate) fn local_cli(home: &str) -> String {
    std::env::current_exe()
        .ok()
        .map(|p| p.display().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| local_cli_fallback(home))
}
pub(crate) fn local_cli_fallback(home: &str) -> String {
    for cand in [
        std::env::var("HIVE_CLI").unwrap_or_default(),
        std::path::Path::new(home)
            .join("bin")
            .join("hive")
            .display()
            .to_string(),
    ] {
        if cand.is_empty() {
            continue;
        }
        if std::fs::metadata(&cand).is_ok_and(|m| m.is_file()) {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if std::fs::metadata(&cand).is_ok_and(|m| m.permissions().mode() & 0o111 != 0) {
                    return cand;
                }
            }
        }
    }
    String::new()
}
pub(crate) use crate::proc::worker_env_with_cli_path;

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup_env(env: &[String], key: &str) -> String {
        env.iter()
            .find_map(|kv| kv.strip_prefix(&format!("{key}=")).map(ToOwned::to_owned))
            .unwrap_or_default()
    }

    #[test]
    fn verify_git_failures_are_written_to_the_job_log() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().display().to_string();
        // Not a repository: `add -u` fails and says why on stderr.
        let e = LocalExec {
            log: RunLog::default(),
            clone_dir: root.clone(),
            ..Default::default()
        };
        let err = e.commit_left_work("left work").unwrap_err().to_string();
        let log = e.log.text();
        assert!(log.contains("git add failed in the clone"), "{log}");
        assert!(log.contains("not a git repository"), "{log}");
        assert!(err.contains("not a git repository"), "{err}");

        // A clean repository: `commit` fails with an empty stderr and says
        // "nothing to commit" on stdout; the log still names the cause.
        hive_testutil::git(&["-C", root.as_str(), "init", "-q"]).unwrap();
        hive_testutil::git(&["-C", root.as_str(), "config", "user.name", "Fixture"]).unwrap();
        hive_testutil::git(&[
            "-C",
            root.as_str(),
            "config",
            "user.email",
            "fixture@hive.local",
        ])
        .unwrap();
        std::fs::write(dir.path().join("a.txt"), "a\n").unwrap();
        hive_testutil::git(&["-C", root.as_str(), "add", "a.txt"]).unwrap();
        hive_testutil::git(&[
            "-C",
            root.as_str(),
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "base",
        ])
        .unwrap();
        let e = LocalExec {
            log: RunLog::default(),
            clone_dir: root.clone(),
            ..Default::default()
        };
        let err = e.commit_left_work("left work").unwrap_err().to_string();
        let log = e.log.text();
        assert!(log.contains("git commit failed in the clone"), "{log}");
        assert!(log.contains("nothing to commit"), "{log}");
        assert!(err.contains("nothing to commit"), "{err}");
    }

    #[test]
    fn git_failure_cause_says_when_git_printed_nothing() {
        let r = crate::util::GitResult {
            code: 1,
            ..Default::default()
        };
        assert_eq!(git_failure_cause(&r), "git printed nothing");
        let r = crate::util::GitResult {
            code: -1,
            err: Some("timed out after 60s".into()),
            stderr: "fatal:\n  lock".into(),
            ..Default::default()
        };
        assert_eq!(git_failure_cause(&r), "timed out after 60s; fatal: lock");
    }

    #[test]
    fn cargo_target_dir_is_outside_clone_gating() {
        let clone = "/tmp/fleet/a";
        // Shared lane outside the clone: freshen.
        assert!(cargo_target_dir_is_outside_clone(
            "/Users/work/.hive/cache/cargo-target-land/amistio-template",
            clone
        ));
        // Inside the clone (it can only hold this tree's work): no-op.
        assert!(!cargo_target_dir_is_outside_clone(
            "/tmp/fleet/a/target",
            clone
        ));
        assert!(!cargo_target_dir_is_outside_clone(clone, clone));
        // Relative values land inside the clone when the check runs there.
        assert!(!cargo_target_dir_is_outside_clone("target", clone));
        assert!(!cargo_target_dir_is_outside_clone("../target", clone));
        // Unset or blank: no-op.
        assert!(!cargo_target_dir_is_outside_clone("", clone));
        assert!(!cargo_target_dir_is_outside_clone("   ", clone));
        // A sibling whose name shares a prefix is not inside the clone.
        assert!(cargo_target_dir_is_outside_clone("/tmp/fleet/abc", clone));
    }

    #[test]
    fn check_shares_cargo_target_dir_reads_the_recipe_and_the_verify_command() {
        let clone = "/tmp/fleet/a";
        // The registered check of a project whose recipe leaves the target
        // dir to it: the default it names is shared by every clone.
        let land = "env CARGO_TARGET_DIR=${HIVE_TEST_FRESHEN_UNSET:-/h/.hive/cache/cargo-target-land/p} CARGO_INCREMENTAL=0 sh -c 'cargo test --workspace'";
        assert!(check_shares_cargo_target_dir(None, land, clone));
        // The recipe's own shared dir still counts without one in the check.
        assert!(check_shares_cargo_target_dir(
            Some("/h/.hive/cache/cargo-target/p/0"),
            "cargo test",
            clone
        ));
        // Either one outside the clone is enough.
        assert!(check_shares_cargo_target_dir(
            Some("/tmp/fleet/a/target"),
            land,
            clone
        ));
        // Inside the clone, relative, or not named at all: no freshening.
        assert!(!check_shares_cargo_target_dir(None, "cargo test", clone));
        assert!(!check_shares_cargo_target_dir(
            Some("target"),
            "env CARGO_TARGET_DIR=/tmp/fleet/a/target cargo test",
            clone
        ));
        assert!(!check_shares_cargo_target_dir(Some(""), "", clone));
    }

    #[test]
    fn freshen_before_check_touches_only_tracked_sources_where_the_check_stands() {
        let cmd = freshen_before_check("cargo test", "the clone's");
        let (touch, check) = cmd.split_once('\n').unwrap();
        assert_eq!(check, "cargo test");
        assert_eq!(
            touch,
            "(git ls-files -z -- '*.rs' 'Cargo.toml' '**/Cargo.toml' 'build.rs' '**/build.rs' | xargs -0 -r touch -c) 2>/dev/null || echo \"hive: could not touch the clone's tracked sources; the check runs as it is\" >&2"
        );
        // No `cd`: the same line serves the clone, the base worktree beside
        // it and the hook, each from its own working directory.
        assert!(!touch.contains("cd "), "{touch}");
    }

    /// The touch really bumps the tracked sources and leaves the rest alone,
    /// and a tracked file that is gone from the tree is not recreated.
    #[test]
    fn freshen_before_check_bumps_tracked_sources_and_recreates_nothing() {
        let dir = tempfile::Builder::new()
            .prefix("hive-freshen-")
            .tempdir()
            .unwrap();
        let root = dir.path().display().to_string();
        hive_testutil::git(&["-C", root.as_str(), "init", "-q"]).unwrap();
        std::fs::write(dir.path().join("lib.rs"), "// lib\n").unwrap();
        std::fs::write(dir.path().join("gone.rs"), "// gone\n").unwrap();
        std::fs::write(dir.path().join("notes.md"), "notes\n").unwrap();
        hive_testutil::git(&["-C", root.as_str(), "add", "lib.rs", "gone.rs", "notes.md"]).unwrap();
        std::fs::remove_file(dir.path().join("gone.rs")).unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        for name in ["lib.rs", "notes.md"] {
            std::fs::File::options()
                .write(true)
                .open(dir.path().join(name))
                .unwrap()
                .set_modified(old)
                .unwrap();
        }
        let out = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(freshen_before_check("exit 0", "the clone's"))
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let mtime = |name: &str| {
            std::fs::metadata(dir.path().join(name))
                .unwrap()
                .modified()
                .unwrap()
        };
        assert!(
            mtime("lib.rs") > old,
            "the tracked source kept its old mtime"
        );
        assert_eq!(mtime("notes.md"), old, "a non-source file was touched");
        assert!(
            !dir.path().join("gone.rs").exists(),
            "touch -c recreated a removed file"
        );
    }

    #[test]
    fn run_log_clone_shares_lines() {
        let _serial = crate::test_helpers::serial();
        let log = RunLog::default();
        let clone = log.clone();
        clone.add("presence unavailable: test");
        assert_eq!(log.text(), "presence unavailable: test");
    }

    #[test]
    fn preflight_run_key_missing_is_setup_error() {
        let _serial = crate::test_helpers::serial();
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("job.key").display().to_string();
        let err = preflight_run_key(&missing).unwrap_err().to_string();
        assert!(
            err.contains("setup-error") && err.contains(&missing),
            "{err}"
        );
        std::fs::write(&missing, []).unwrap();
        let err = preflight_run_key(&missing).unwrap_err().to_string();
        assert!(err.contains("empty"), "{err}");
        std::fs::write(&missing, b"k").unwrap();
        preflight_run_key(&missing).unwrap();
    }

    #[test]
    fn preflight_run_key_remote_check_cmd() {
        let _serial = crate::test_helpers::serial();
        let _deadline =
            crate::test_helpers_daemon::test_deadline(std::time::Duration::from_secs(60));
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("run.key");
        if std::process::Command::new("/bin/sh")
            .args(["-c", &remote_key_check_cmd(key.to_str().unwrap())])
            .status()
            .unwrap()
            .success()
        {
            panic!("missing remote key must fail test -s");
        }
        std::fs::write(&key, []).unwrap();
        if std::process::Command::new("/bin/sh")
            .args(["-c", &remote_key_check_cmd(key.to_str().unwrap())])
            .status()
            .unwrap()
            .success()
        {
            panic!("empty remote key must fail test -s");
        }
        std::fs::write(&key, b"k").unwrap();
        let status = std::process::Command::new("/bin/sh")
            .args(["-c", &remote_key_check_cmd(key.to_str().unwrap())])
            .status()
            .unwrap();
        if !status.success() {
            panic!("non-empty remote key must pass: {status}");
        }
        let cmd = remote_key_check_cmd("/w/job 1/run.key");
        assert!(
            cmd.contains("'/w/job 1/run.key'"),
            "path must be shell-quoted, got {cmd:?}"
        );
    }

    #[test]
    fn preflight_run_key_remote_result() {
        let _serial = crate::test_helpers::serial();
        remote_key_check_result("/w/j/run.key", "box", "before the worker starts", 0, "", "")
            .unwrap();
        let err = remote_key_check_result(
            "/w/j/run.key",
            "box",
            "before the worker starts",
            1,
            "",
            "no such file",
        )
        .unwrap_err()
        .to_string();
        for want in [
            "setup-error",
            "/w/j/run.key",
            "box",
            "before the worker starts",
            "no such file",
        ] {
            assert!(err.contains(want), "{err:?} should contain {want:?}");
        }
    }

    #[test]
    fn preflight_run_key_after_make_run_key() {
        let _serial = crate::test_helpers::serial();
        let path = tempfile::tempdir()
            .unwrap()
            .path()
            .join("run.key")
            .display()
            .to_string();
        if let Err(err) = crate::keys::make_run_key(&path, "juniper", "run-1") {
            if err.to_string().contains("ssh-keygen is not installed") {
                return;
            }
            panic!("{err}");
        }
        preflight_run_key(&path).unwrap();
    }

    #[test]
    fn preflight_remote_cli_check_cmd() {
        let _serial = crate::test_helpers::serial();
        let _deadline =
            crate::test_helpers_daemon::test_deadline(std::time::Duration::from_secs(60));
        let dir = tempfile::tempdir().unwrap();
        let cli = dir.path().join("hive");
        if std::process::Command::new("/bin/sh")
            .args(["-c", &remote_cli_check_cmd(cli.to_str().unwrap())])
            .status()
            .unwrap()
            .success()
        {
            panic!("missing HIVE_CLI must fail test -x");
        }
        std::fs::write(&cli, b"#!/bin/sh\n").unwrap();
        if std::process::Command::new("/bin/sh")
            .args(["-c", &remote_cli_check_cmd(cli.to_str().unwrap())])
            .status()
            .unwrap()
            .success()
        {
            panic!("non-executable HIVE_CLI must fail test -x");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let status = std::process::Command::new("/bin/sh")
            .args(["-c", &remote_cli_check_cmd(cli.to_str().unwrap())])
            .status()
            .unwrap();
        if !status.success() {
            panic!("executable HIVE_CLI must pass: {status}");
        }
        let cmd = remote_cli_check_cmd("/usr/local/bin/my hive");
        assert!(
            cmd.contains("'/usr/local/bin/my hive'"),
            "path must be shell-quoted, got {cmd:?}"
        );
    }

    #[test]
    fn preflight_remote_cli_result() {
        let _serial = crate::test_helpers::serial();
        remote_cli_check_result("/usr/local/bin/hive", "box", 0, "", "").unwrap();
        let err = remote_cli_check_result("/usr/local/bin/hive", "box", 1, "", "no such file")
            .unwrap_err()
            .to_string();
        for want in [
            "setup-error",
            "HIVE_CLI",
            "/usr/local/bin/hive",
            "box",
            "no such file",
            "not started",
        ] {
            assert!(err.contains(want), "{err:?} should contain {want:?}");
        }
    }

    #[test]
    fn local_exec_worker_path() {
        let _serial = crate::test_helpers::serial();
        let sep = if cfg!(windows) { ";" } else { ":" };
        let base = vec![format!("PATH=/usr/bin{sep}/bin"), "HOME=/h".into()];
        let mut extra = BTreeMap::new();
        extra.insert("HIVE_CLI".into(), "/opt/hive/bin/hive".into());
        let got = worker_env_with_cli_path(&base, &extra);
        assert_eq!(
            lookup_env(&got, "PATH"),
            format!("/opt/hive/bin{sep}/usr/bin{sep}/bin")
        );
        assert_eq!(lookup_env(&got, "HIVE_CLI"), "/opt/hive/bin/hive");

        extra.insert("HIVE_CLI".into(), "/usr/bin/hive".into());
        let got = worker_env_with_cli_path(&base, &extra);
        assert_eq!(lookup_env(&got, "PATH"), format!("/usr/bin{sep}/bin"));

        let mut extra = BTreeMap::new();
        extra.insert("X".into(), "1".into());
        let got = worker_env_with_cli_path(&base, &extra);
        assert_eq!(lookup_env(&got, "PATH"), format!("/usr/bin{sep}/bin"));

        let mut extra = BTreeMap::new();
        extra.insert("HIVE_CLI".into(), "/opt/p/hive".into());
        let got = worker_env_with_cli_path(&["HOME=/h".into()], &extra);
        assert_eq!(lookup_env(&got, "PATH"), "/opt/p");
    }

    #[test]
    fn ssh_exec_worker_path() {
        let _serial = crate::test_helpers::serial();
        let dir = tempfile::tempdir().unwrap();
        let cli = dir.path().join("hive");
        std::fs::write(&cli, b"#!/bin/sh\necho ok\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut out = Vec::new();
        let mut h = crate::hands::new_hands(std::io::empty(), &mut out, Vec::<u8>::new());
        let mut extra = BTreeMap::new();
        extra.insert("HIVE_CLI".to_string(), cli.display().to_string());
        let rc = h.with_timeout(
            Duration::from_secs(60),
            &dir.path().display().to_string(),
            &[
                "/bin/sh".to_string(),
                "-c".to_string(),
                "printf %s \"$PATH\"".to_string(),
            ],
            &extra,
        );
        assert_eq!(rc, 0, "out={}", String::from_utf8_lossy(&out));
        assert!(
            String::from_utf8_lossy(&out).starts_with(
                &(dir.path().display().to_string() + if cfg!(windows) { ";" } else { ":" })
            ),
            "remote worker PATH not prefixed with dir(HIVE_CLI): {:?}",
            String::from_utf8_lossy(&out)
        );
    }

    #[test]
    fn local_exec_default_hive_cli() {
        let _serial = crate::test_helpers::serial();
        let exe = std::env::current_exe().unwrap();
        let exe = exe.display().to_string();
        let mut got = BTreeMap::from([("HIVE_JOB".to_string(), "j".to_string())]);
        crate::run::worker_cli_env(
            &mut got,
            &local_cli(&tempfile::tempdir().unwrap().path().display().to_string()),
        );
        assert_eq!(got.get("HIVE_CLI").map(String::as_str), Some(exe.as_str()));
        let env = worker_env_with_cli_path(&["PATH=/usr/bin".into()], &got);
        assert_eq!(lookup_env(&env, "HIVE_CLI"), exe);
        assert!(
            lookup_env(&env, "PATH").starts_with(
                &(std::env::current_exe()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .display()
                    .to_string()
                    + if cfg!(windows) { ";" } else { ":" })
            )
        );
        let keep = worker_env_with_cli_path(
            &["PATH=/usr/bin".into()],
            &BTreeMap::from([("HIVE_CLI".to_string(), "/opt/hive/bin/hive".to_string())]),
        );
        assert_eq!(lookup_env(&keep, "HIVE_CLI"), "/opt/hive/bin/hive");
    }

    #[test]
    fn cli_on_path() {
        let _serial = crate::test_helpers::serial();
        let bin = tempfile::tempdir().unwrap();
        let exe = bin.path().join("hive");
        std::fs::write(&exe, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let other = tempfile::tempdir().unwrap();
        let sep = if cfg!(windows) { ";" } else { ":" };
        assert_eq!(
            crate::run::cli_on_path(
                &other.path().display().to_string(),
                &exe.display().to_string()
            ),
            format!("{}{sep}{}", bin.path().display(), other.path().display())
        );
        assert_eq!(
            crate::run::cli_on_path("", &exe.display().to_string()),
            bin.path().display().to_string()
        );
        assert_eq!(
            crate::run::cli_on_path(
                &format!("{}{sep}{}", bin.path().display(), other.path().display()),
                &exe.display().to_string()
            ),
            ""
        );
        assert_eq!(
            crate::run::cli_on_path(&other.path().display().to_string(), ""),
            ""
        );
    }

    #[test]
    fn cli_on_path_executable_dir() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        let exe = std::env::current_exe().unwrap();
        let inherited = tempfile::tempdir().unwrap();
        let _env_1 = hive_testutil::setenv("PATH", inherited.path());
        let mut env = BTreeMap::new();
        crate::run::worker_cli_env(&mut env, &exe.display().to_string());
        assert_eq!(
            env.get("HIVE_CLI").map(String::as_str),
            Some(exe.to_str().unwrap())
        );
        let got = worker_env_with_cli_path(&crate::proc::environ(), &env);
        let path = lookup_env(&got, "PATH");
        let dirs = std::env::split_paths(&path).collect::<Vec<_>>();
        assert!(dirs.len() >= 2 && dirs[0] == exe.parent().unwrap());
        assert_eq!(dirs.last().unwrap(), inherited.path());
    }

    #[test]
    fn cli_on_path_fallback() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        let home = tempfile::tempdir().unwrap();
        let _env_2 = hive_testutil::setenv("HIVE_CLI", "");
        assert_eq!(local_cli_fallback(&home.path().display().to_string()), "");
        let bin = home.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let home_cli = bin.join("hive");
        std::fs::write(&home_cli, b"#!/bin/sh\n").unwrap();
        assert_eq!(local_cli_fallback(&home.path().display().to_string()), "");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&home_cli, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(
            local_cli_fallback(&home.path().display().to_string()),
            home_cli.display().to_string()
        );
        let other_dir = tempfile::tempdir().unwrap();
        let other = other_dir.path().join("hive");
        std::fs::write(&other, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&other, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let _env_3 = hive_testutil::setenv("HIVE_CLI", &other);
        assert_eq!(
            local_cli_fallback(&home.path().display().to_string()),
            other.display().to_string()
        );
        let p = crate::run::cli_on_path(
            &tempfile::tempdir().unwrap().path().display().to_string(),
            &other.display().to_string(),
        );
        assert_eq!(
            std::env::split_paths(&p).next().unwrap(),
            other.parent().unwrap()
        );
        assert_eq!(
            local_cli(&home.path().display().to_string()),
            std::env::current_exe().unwrap().display().to_string()
        );
    }

    #[test]
    fn cli_on_path_worker_process() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        let exe = std::env::current_exe().unwrap();
        let inherited = tempfile::tempdir().unwrap();
        let _env_4 = hive_testutil::setenv("PATH", inherited.path());
        let s = crate::test_helpers::new_store();
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("run.key");
        std::fs::write(&key, b"key").unwrap();
        let mut e = LocalExec {
            s: Some((*s).clone()),
            job: {
                let mut r = Row::new();
                r.insert("id", serde_json::json!("job-cli"));
                r
            },
            project: Row::new(),
            clone_dir: dir.path().display().to_string(),
            key: key.display().to_string(),
            ..Default::default()
        };
        let mut env = BTreeMap::new();
        crate::run::worker_cli_env(&mut env, &exe.display().to_string());
        let out = e
            .run_worker(
                &hive_bus::CancellationToken::new(),
                &[
                    "/bin/sh".into(),
                    "-c".into(),
                    "printf 'PATH=%s\\n' \"$PATH\"".into(),
                ],
                &env,
                1,
            )
            .unwrap();
        assert_eq!(out.exit_code, 0, "{out:?}");
        let got = out
            .stdout
            .lines()
            .find_map(|l| l.strip_prefix("PATH="))
            .unwrap_or("");
        let dirs = std::env::split_paths(got).collect::<Vec<_>>();
        assert!(!dirs.is_empty() && dirs[0] == exe.parent().unwrap());
        assert_eq!(dirs.last().unwrap(), inherited.path());
    }

    fn model_arg(cmd: &[String]) -> String {
        cmd.windows(2)
            .find_map(|w| (w[0] == "--model").then(|| w[1].clone()))
            .unwrap_or_default()
    }

    fn flag_arg(cmd: &[String], flag: &str) -> String {
        cmd.windows(2)
            .find_map(|w| (w[0] == flag).then(|| w[1].clone()))
            .unwrap_or_default()
    }

    fn set_setting(s: &hive_store::Store, key: &str, value: &str) {
        s.set_setting(crate::util::HUMAN_AGENT, key, value).unwrap();
    }

    fn insert_test_project(s: &hive_store::Store) -> Row {
        let root = std::path::Path::new(&s.home)
            .join("exec-test-project")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&root).unwrap();
        let root = root.display().to_string();
        let now = s.now();
        s.conn()
            .unwrap()
            .execute("INSERT OR REPLACE INTO projects (slug, name, path, remote, brain, verify_cmd, default_branch, status, added_at, added_by, last_seen) VALUES ('proj', 'Proj', ?, '', 0, '', 'main', 'active', ?, ?, ?)", rusqlite::params![root, now, crate::util::HUMAN_AGENT, now])
            .unwrap();
        let mut project = Row::new();
        project.insert("slug", serde_json::json!("proj"));
        project.insert("name", serde_json::json!("Proj"));
        project.insert("path", serde_json::json!(root));
        project
    }

    fn insert_git_project(s: &hive_store::Store) -> Row {
        let root = std::path::Path::new(&s.home)
            .join("exec-git-project")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&root).unwrap();
        let run_git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&root)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@x")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@x")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_TERMINAL_PROMPT", "0")
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr)
            );
        };
        run_git(&["init", "-q", "-b", "main"]);
        run_git(&["config", "commit.gpgsign", "false"]);
        std::fs::write(root.join("README.md"), "# proj\n").unwrap();
        run_git(&["add", "-A"]);
        run_git(&["commit", "-q", "-m", "init"]);
        let root = std::fs::canonicalize(root).unwrap().display().to_string();
        let now = s.now();
        s.conn()
            .unwrap()
            .execute("INSERT OR REPLACE INTO projects (slug, name, path, remote, brain, verify_cmd, default_branch, status, added_at, added_by, last_seen) VALUES ('proj', 'Proj', ?, '', 0, '', 'main', 'active', ?, ?, ?)", rusqlite::params![root, now, crate::util::HUMAN_AGENT, now])
            .unwrap();
        let mut project = Row::new();
        project.insert("slug", serde_json::json!("proj"));
        project.insert("name", serde_json::json!("Proj"));
        project.insert("path", serde_json::json!(root));
        project
    }

    #[test]
    fn worker_command_model_follows_the_job_kind() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        set_setting(&s, "worker_model", "heavy-test");
        set_setting(&s, "light_model", "light-test");
        for kind in hive_board::JOB_KINDS {
            let (cmd, custom) = worker_command(&s, kind, "brief", "/tmp/clone", "", false);
            assert!(!custom, "{kind}: a custom command without worker_cmd");
            let want = if hive_store::IMPLEMENTATION_JOB_KINDS.contains(kind) {
                "heavy-test"
            } else {
                "light-test"
            };
            assert_eq!(model_arg(&cmd), want, "{kind}");
        }
        let (cmd, custom) =
            worker_command(&s, "task", "brief", "/tmp/clone", "sh -c 'echo hi'", false);
        assert!(custom);
        assert_eq!(model_arg(&cmd), "");
    }

    #[test]
    fn worker_command_runs_as_the_agent_file_that_selects_the_job() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        set_setting(&s, "worker_model", "heavy-test");
        set_setting(&s, "light_model", "light-test");
        set_setting(&s, "harness.max_steps", "60");
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".hive/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("rust-refactorer.md"),
            "---\ndescription: Refactors Rust.\ntools: [read, grep, glob, bash, edit]\nmodel: kimi-k3\nmaxTurns: 40\nhive:\n  kinds: [task]\n  select:\n    tags: [refactor]\n---\nKeep every public signature.\n",
        )
        .unwrap();
        std::fs::write(
            agents.join("auditor.md"),
            "---\ndescription: Audits.\nreadonly: true\nhive:\n  kinds: [task, answer]\n  select:\n    tags: [security]\n---\nLook, do not touch.\n",
        )
        .unwrap();
        let checkout = dir.path().to_str().unwrap();
        let job = |brief: &str| {
            let mut j = crate::util::Row::new();
            j.insert("id", serde_json::json!("J-1"));
            j.insert("kind", serde_json::json!("task"));
            j.insert("brief", serde_json::json!(brief));
            j
        };
        let refactor = job("Tidy run.rs\nClass: refactor\nFile: crates/hive-runner/src/run.rs\n");
        let spec = crate::agent_specs::spec_for_job(&s, &refactor, "proj", checkout)
            .spec
            .unwrap();
        assert_eq!(spec.name, "rust-refactorer");
        let feature = job("Add a feature\nClass: feature\n");
        let chosen = crate::agent_specs::spec_for_job(&s, &feature, "proj", checkout);
        assert!(chosen.spec.is_none() && chosen.warning.is_none());

        // A job pinned to an agent runs as it whatever the card says and
        // whatever the spec's kinds say; a pin to a name that is gone is a
        // warning, and the card chooses as usual.
        let mut pinned = feature.clone();
        pinned.insert("agent", serde_json::json!("auditor"));
        let chosen = crate::agent_specs::spec_for_job(&s, &pinned, "proj", checkout);
        assert_eq!(chosen.spec.unwrap().name, "auditor");
        assert!(chosen.warning.is_none());
        let mut pinned = refactor.clone();
        pinned.insert("agent", serde_json::json!("vanished"));
        let chosen = crate::agent_specs::spec_for_job(&s, &pinned, "proj", checkout);
        assert_eq!(chosen.spec.unwrap().name, "rust-refactorer");
        let warning = chosen.warning.unwrap();
        assert!(
            warning.starts_with("warning: job J-1: it was pinned to the agent vanished, but "),
            "{warning}"
        );

        // models.allowed empty: the agent's model travels, with --agent and
        // its lower step cap.
        let (cmd, custom) = worker_command_as(
            &s,
            "task",
            "",
            "brief",
            "/tmp/clone",
            "",
            false,
            Some(&spec),
        );
        assert!(!custom);
        assert_eq!(flag_arg(&cmd, "--agent"), "rust-refactorer");
        assert_eq!(model_arg(&cmd), "kimi-k3");
        assert_eq!(flag_arg(&cmd, "--max-steps"), "40");
        assert!(!cmd.contains(&"--read-only".to_string()));

        // models.allowed lists it: the same.
        set_setting(&s, "models.allowed", "claude-*, kimi-k3");
        let (cmd, _) = worker_command_as(
            &s,
            "task",
            "",
            "brief",
            "/tmp/clone",
            "",
            false,
            Some(&spec),
        );
        assert_eq!(model_arg(&cmd), "kimi-k3");
        assert_eq!(flag_arg(&cmd, "--agent"), "rust-refactorer");

        // models.allowed leaves it out: the class model, the agent still.
        set_setting(&s, "models.allowed", "claude-*");
        let (cmd, _) = worker_command_as(
            &s,
            "task",
            "",
            "brief",
            "/tmp/clone",
            "",
            false,
            Some(&spec),
        );
        assert_eq!(model_arg(&cmd), "heavy-test");
        assert_eq!(flag_arg(&cmd, "--agent"), "rust-refactorer");
        let (cmd, _) = worker_command_as(
            &s,
            "task",
            "light",
            "brief",
            "/tmp/clone",
            "",
            false,
            Some(&spec),
        );
        assert_eq!(
            model_arg(&cmd),
            "light-test",
            "the role's class still routes"
        );
        assert_eq!(
            worker_model(&s, "task", "", Some(&spec)),
            "heavy-test",
            "the board's agent row shows the model the worker gets"
        );

        // A remote hand reads its clone, so a repository-file agent travels
        // by name; the kind's step cap holds when the agent's is higher.
        set_setting(&s, "models.allowed", "");
        set_setting(&s, "harness.max_steps", "30");
        let (cmd, _) =
            worker_command_as(&s, "task", "", "brief", "/tmp/clone", "", true, Some(&spec));
        assert_eq!(flag_arg(&cmd, "--agent"), "rust-refactorer");
        assert_eq!(flag_arg(&cmd, "--max-steps"), "30");
        // No kind cap at all: the worker hears none, an agent's cap still holds.
        set_setting(&s, "harness.max_steps", "none");
        let (cmd, _) = worker_command_as(&s, "task", "", "brief", "/tmp/clone", "", true, None);
        assert_eq!(flag_arg(&cmd, "--max-steps"), "none");
        let (cmd, _) =
            worker_command_as(&s, "task", "", "brief", "/tmp/clone", "", true, Some(&spec));
        assert_eq!(flag_arg(&cmd, "--max-steps"), "40");
        set_setting(&s, "harness.max_steps", "30");
        assert!(agent_flag_travels(&spec, true));
        let mut record = spec.clone();
        record.layer = hive_harness::agents::AgentLayer::Store;
        assert!(!agent_flag_travels(&record, true) && agent_flag_travels(&record, false));
        let (cmd, _) = worker_command_as(
            &s,
            "task",
            "",
            "brief",
            "/tmp/clone",
            "",
            true,
            Some(&record),
        );
        assert_eq!(flag_arg(&cmd, "--agent"), "");
        assert_eq!(model_arg(&cmd), "kimi-k3", "the model travels all the same");

        // A read-only agent makes a task run read-only.
        let audit = job("Check the login\nTags: security\n");
        let spec = crate::agent_specs::spec_for_job(&s, &audit, "proj", checkout)
            .spec
            .unwrap();
        assert_eq!(spec.name, "auditor");
        let (cmd, _) = worker_command_as(
            &s,
            "task",
            "",
            "brief",
            "/tmp/clone",
            "",
            false,
            Some(&spec),
        );
        assert!(cmd.contains(&"--read-only".to_string()), "{cmd:?}");
        assert_eq!(flag_arg(&cmd, "--agent"), "auditor");
        assert_eq!(model_arg(&cmd), "heavy-test", "no model of its own");
        // Without a spec nothing changes.
        let (cmd, _) = worker_command_as(&s, "task", "", "brief", "/tmp/clone", "", false, None);
        assert_eq!(flag_arg(&cmd, "--agent"), "");
        assert_eq!(flag_arg(&cmd, "--max-steps"), "30");
    }

    #[test]
    fn worker_command_task_max_steps_only_widens_the_implementation_kinds() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        set_setting(&s, "harness.max_steps", "150");
        set_setting(&s, "harness.task_max_steps", "300");
        // A task may spend more steps without the short kinds getting them.
        for kind in ["task", "improve", "experiment"] {
            let (cmd, _) = worker_command(&s, kind, "brief", "/tmp/clone", "", false);
            assert_eq!(flag_arg(&cmd, "--max-steps"), "300", "{kind}");
        }
        for kind in [
            "answer", "ballot", "onboard", "reflect", "lead", "clean", "review",
        ] {
            let (cmd, _) = worker_command(&s, kind, "brief", "/tmp/clone", "", false);
            assert_eq!(flag_arg(&cmd, "--max-steps"), "150", "{kind}");
        }
        // none/off (and the empty default) fall back to harness.max_steps for every kind.
        for off in ["none", "off"] {
            set_setting(&s, "harness.task_max_steps", off);
            let (cmd, _) = worker_command(&s, "task", "brief", "/tmp/clone", "", false);
            assert_eq!(flag_arg(&cmd, "--max-steps"), "150", "{off}");
        }
    }

    #[test]
    fn light_model_falls_back_to_worker_model() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        assert_eq!(s.run_model("task"), "");
        assert_eq!(s.run_model("lead"), "");
        set_setting(&s, "worker_model", "heavy-test");
        for kind in [
            "task", "lead", "answer", "ballot", "reflect", "onboard", "clean", "review",
        ] {
            assert_eq!(s.run_model(kind), "heavy-test", "{kind}");
        }
        assert_eq!(s.light_model(), "heavy-test");
        assert_eq!(
            model_arg(&crate::executor::login_check_command(&s)),
            "heavy-test"
        );
        set_setting(&s, "light_model", "light-test");
        assert_eq!(s.light_model(), "light-test");
        assert_eq!(
            model_arg(&crate::executor::login_check_command(&s)),
            "light-test"
        );
        assert_eq!(s.run_model("task"), "heavy-test");
    }

    #[test]
    fn worker_command_argv() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        set_setting(&s, "worker_model", "heavy-test");
        let (cmd, custom) = worker_command(&s, "task", "brief text", "/tmp/clone", "", false);
        assert!(!custom);
        let exe = std::env::current_exe().unwrap().display().to_string();
        let want = vec![
            exe,
            "harness".into(),
            "run".into(),
            "-p".into(),
            "brief text".into(),
            "-C".into(),
            "/tmp/clone".into(),
            "--max-steps".into(),
            "60".into(),
            "--step-timeout".into(),
            "300".into(),
            "--commit-timeout".into(),
            "1500".into(),
            "--output-limit".into(),
            "20000".into(),
            "--max-cost".into(),
            "0".into(),
            "--keep-observations".into(),
            "5".into(),
            "--minutes".into(),
            "9".into(),
            "--model".into(),
            "heavy-test".into(),
        ];
        assert_eq!(cmd, want);
        let (cmd, custom) = worker_command(&s, "lead", "brief text", "/tmp/clone", "", true);
        assert!(
            !custom && cmd.len() >= 3 && cmd[0] == "hive" && cmd[1] == "harness" && cmd[2] == "run"
        );
        assert!(!cmd.contains(&"--context-window".to_string()));
        assert!(
            !cmd.contains(&"--read-only".to_string()),
            "a lead run may change the clone: {cmd:?}"
        );
        // The jobs that only look run read-only.
        for kind in hive_store::settings::READ_ONLY_JOB_KINDS {
            let (cmd, _) = worker_command(&s, kind, "brief text", "/tmp/clone", "", true);
            assert!(cmd.contains(&"--read-only".to_string()), "{kind}: {cmd:?}");
        }
        for kind in hive_store::settings::IMPLEMENTATION_JOB_KINDS {
            let (cmd, _) = worker_command(&s, kind, "brief text", "/tmp/clone", "", true);
            assert!(!cmd.contains(&"--read-only".to_string()), "{kind}: {cmd:?}");
        }
        // A context window override rides along once set.
        set_setting(&s, "harness.context_window", "1000000");
        let (cmd, _) = worker_command(&s, "task", "brief text", "/tmp/clone", "", true);
        assert!(
            cmd.windows(2).any(|w| w == ["--context-window", "1000000"]),
            "{cmd:?}"
        );
        set_setting(&s, "harness.context_window", "0");
        // The Wave 2 switches travel only when they leave their defaults.
        for flag in [
            "--no-skills",
            "--no-hooks",
            "--no-subagents",
            "--no-memory",
            "--no-tasks",
            "--web-search",
            "--fetch-allow",
            "--compact-at",
            "--max-request-mb",
            "--compact-model",
            "--reasoning",
            "--allow-tool",
            "--deny-tool",
            "--teach",
        ] {
            assert!(!cmd.contains(&flag.to_string()), "{cmd:?}");
        }
        set_setting(&s, "harness.teach", "on");
        set_setting(&s, "harness.skills", "off");
        set_setting(&s, "harness.hooks", "off");
        set_setting(&s, "harness.subagents", "off");
        set_setting(&s, "harness.memory", "off");
        set_setting(&s, "harness.tasks", "off");
        set_setting(&s, "harness.web_search", "on");
        set_setting(&s, "harness.fetch_allow", "docs.rs, *.rust-lang.org");
        set_setting(&s, "harness.compact_at", "0");
        set_setting(&s, "provider.max_request_mb", "0.5");
        set_setting(&s, "harness.compact_model", "cheap-1");
        set_setting(&s, "harness.reasoning", "high");
        set_setting(&s, "harness.deny_tools", "bash(rm *)\nwrite(*.lock)");
        let (cmd, _) = worker_command(&s, "task", "brief text", "/tmp/clone", "", true);
        assert!(
            cmd.windows(2)
                .any(|w| w == ["--deny-tool", "bash(rm *),write(*.lock)"]),
            "{cmd:?}"
        );
        assert!(
            cmd.windows(2).any(|w| w == ["--reasoning", "high"]),
            "{cmd:?}"
        );
        // A role's own effort wins over harness.reasoning for its kinds.
        set_setting(&s, "light_reasoning", "low");
        let (light, _) = worker_command(&s, "lead", "brief text", "/tmp/clone", "", true);
        assert!(
            light.windows(2).any(|w| w == ["--reasoning", "low"]),
            "{light:?}"
        );
        let (task, _) = worker_command(&s, "task", "brief text", "/tmp/clone", "", true);
        assert!(
            task.windows(2).any(|w| w == ["--reasoning", "high"]),
            "{task:?}"
        );
        set_setting(&s, "worker_reasoning", "default");
        let (task, _) = worker_command(&s, "task", "brief text", "/tmp/clone", "", true);
        assert!(!task.contains(&"--reasoning".to_string()), "{task:?}");
        set_setting(&s, "worker_reasoning", "");
        set_setting(&s, "light_reasoning", "");
        for flag in [
            "--no-skills",
            "--no-hooks",
            "--no-subagents",
            "--no-memory",
            "--no-tasks",
            "--web-search",
            "--teach",
        ] {
            assert!(cmd.contains(&flag.to_string()), "{cmd:?}");
        }
        assert!(
            cmd.windows(2)
                .any(|w| w == ["--fetch-allow", "docs.rs, *.rust-lang.org"]),
            "{cmd:?}"
        );
        assert!(
            cmd.windows(2).any(|w| w == ["--compact-at", "0"]),
            "{cmd:?}"
        );
        assert!(
            cmd.windows(2).any(|w| w == ["--max-request-mb", "0.5"]),
            "{cmd:?}"
        );
        assert!(
            cmd.windows(2).any(|w| w == ["--compact-model", "cheap-1"]),
            "{cmd:?}"
        );
        for key in [
            "harness.skills",
            "harness.hooks",
            "harness.subagents",
            "harness.memory",
            "harness.tasks",
        ] {
            set_setting(&s, key, "on");
        }
        set_setting(&s, "harness.web_search", "chat");
        set_setting(&s, "harness.teach", "off");
        set_setting(&s, "harness.fetch_allow", "");
        // The registry's own defaults, set explicitly, travel no more than
        // an unset row does.
        set_setting(&s, "harness.compact_at", "50");
        set_setting(&s, "provider.max_request_mb", "1");
        set_setting(&s, "harness.compact_model", "");
        set_setting(&s, "harness.reasoning", "default");
        set_setting(&s, "harness.deny_tools", "");
        // Back at the defaults a remote hand gets the same argv behind `hive`.
        let (cmd, _) = worker_command(&s, "task", "brief text", "/tmp/clone", "", true);
        let mut remote = want.clone();
        remote[0] = "hive".into();
        assert_eq!(cmd, remote);
    }

    #[test]
    fn login_fault_matches_native_harness_errors() {
        let _serial = crate::test_helpers::serial();
        assert!(crate::executor::login_fault(
            "not logged in: sign in with GitHub"
        ));
        assert!(crate::executor::login_fault("not_logged_in"));
    }

    #[test]
    fn local_run_worker_reports_exec_error() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("run.key");
        std::fs::write(&key, b"key").unwrap();
        let mut job = Row::new();
        job.insert("id", serde_json::json!("job-nf"));
        let mut project = Row::new();
        project.insert("slug", serde_json::json!("proj"));
        let mut e = LocalExec {
            s: Some((*s).clone()),
            job,
            project,
            log: RunLog::default(),
            clone_dir: dir.path().display().to_string(),
            key: key.display().to_string(),
            ..Default::default()
        };
        let missing = dir
            .path()
            .join("no-such-worker-binary")
            .display()
            .to_string();
        let err = e
            .run_worker(
                &hive_bus::CancellationToken::new(),
                std::slice::from_ref(&missing),
                &BTreeMap::new(),
                1,
            )
            .unwrap_err()
            .to_string();
        assert!(
            err.contains(&format!("worker command not found: {missing}")),
            "{err}"
        );
        assert!(err.contains("no such file or directory"), "{err}");
    }

    #[test]
    fn with_resume_points_the_worker_at_the_held_trajectory() {
        let cmd = with_resume(
            vec![
                "hive".into(),
                "harness".into(),
                "run".into(),
                "-p".into(),
                "go".into(),
            ],
            "/h/traj.json",
        );
        assert_eq!(
            cmd,
            vec![
                "hive",
                "harness",
                "run",
                "-p",
                "go",
                "--resume",
                "/h/traj.json"
            ]
        );
    }

    #[test]
    fn with_commit_cap_replaces_the_value_or_adds_the_flag() {
        let cmd = with_commit_cap(
            vec![
                "hive".into(),
                "harness".into(),
                "run".into(),
                "--step-timeout".into(),
                "900".into(),
                "--commit-timeout".into(),
                "2400".into(),
                "--max-cost".into(),
                "0".into(),
            ],
            3700,
        );
        assert_eq!(
            cmd,
            vec![
                "hive",
                "harness",
                "run",
                "--step-timeout",
                "900",
                "--commit-timeout",
                "3700",
                "--max-cost",
                "0"
            ]
        );
        let cmd = with_commit_cap(vec!["hive".into(), "harness".into(), "run".into()], 3700);
        assert_eq!(
            cmd,
            vec!["hive", "harness", "run", "--commit-timeout", "3700"]
        );
    }

    #[test]
    fn a_held_run_keeps_its_clone_and_key_for_the_run_that_carries_on() {
        let _serial = crate::test_helpers::serial();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = crate::test_helpers::new_store();
        let project = insert_git_project(&s);
        let home = tempfile::tempdir().unwrap();
        let mut r = Row::new();
        r.insert("name", serde_json::json!("local"));
        let mut job = Row::new();
        job.insert("id", serde_json::json!("job-held"));
        let mut e = new_local_exec(
            r,
            &s,
            &home.path().display().to_string(),
            job,
            project,
            "cinder",
            &mut RunLog::default(),
        );
        let key = e.key_path();
        let first_pub = crate::keys::make_run_key(&key, "cinder", "run-1").unwrap();
        let base = e.prepare(&hive_bus::CancellationToken::new()).unwrap();
        assert_eq!(base.len(), 40, "{base}");
        let edit = std::path::Path::new(&e.clone_dir).join("work-in-progress.txt");
        std::fs::write(&edit, "half done\n").unwrap();

        // Held: strays go, the clone with its edit and the key stay.
        assert!(e.hold().is_empty());
        assert!(edit.is_file(), "the held run's edit was removed");
        assert_eq!(crate::keys::reuse_run_key(&key).unwrap(), first_pub);

        // The run that carries on: same clone, same base, same key.
        e.set_resume(&base);
        let again = e.prepare(&hive_bus::CancellationToken::new()).unwrap();
        assert_eq!(again, base);
        assert!(edit.is_file(), "the edit did not survive the resume");
        assert!(
            e.log.text().contains("clone kept from the held run"),
            "{}",
            e.log.text()
        );

        // Without a clone to carry on, the request falls through to a fresh clone.
        let _ = crate::retention::discard_dir(home.path(), std::path::Path::new(&e.clone_dir));
        e.set_resume(&base);
        let fresh = e.prepare(&hive_bus::CancellationToken::new()).unwrap();
        assert_eq!(fresh, base);
        assert!(!edit.exists(), "a fresh clone carried the old edit");
        assert!(
            e.log
                .text()
                .contains("the held run's clone is gone; cloning anew"),
            "{}",
            e.log.text()
        );

        // A plain cleanup removes the key, so the next run makes a new one.
        e.cleanup();
        let err = crate::keys::reuse_run_key(&key).unwrap_err().to_string();
        assert!(
            err.contains("the held run's signing key is not at"),
            "{err}"
        );
    }

    #[test]
    fn preflight_run_key_local_prepare_fails_early() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        let project = insert_test_project(&s);
        let home = tempfile::tempdir().unwrap();
        let mut r = Row::new();
        r.insert("name", serde_json::json!("local"));
        let mut job = Row::new();
        job.insert("id", serde_json::json!("job-prep"));
        let mut e = new_local_exec(
            r,
            &s,
            &home.path().display().to_string(),
            job,
            project,
            "cinder",
            &mut RunLog::default(),
        );
        std::fs::create_dir_all(std::path::Path::new(&e.key).parent().unwrap()).unwrap();
        let err = e
            .prepare(&hive_bus::CancellationToken::new())
            .unwrap_err()
            .to_string();
        assert!(err.contains("setup-error") && err.contains(&e.key), "{err}");
        assert!(!e.started());
    }

    #[test]
    fn hive_on_path() {
        let _serial = crate::test_helpers::serial();
        assert_eq!(
            crate::recipe::recipe_hive_cli(&crate::recipe::Recipe::default()),
            ""
        );
        let (r, problems) = crate::recipe::parse_recipe(&crate::test_helpers::recipe_with(
            None,
            None,
            None,
            Some(&[("HIVE_CLI", "/opt/hive/bin/hive ")]),
            "",
        ));
        assert!(problems.is_empty(), "{problems:?}");
        let r = r.unwrap();
        assert_eq!(crate::recipe::recipe_hive_cli(&r), "/opt/hive/bin/hive");
        assert!(
            crate::hands::HANDS_ENV_RE
                .is_match(&format!("HIVE_CLI={}", crate::recipe::recipe_hive_cli(&r)))
        );
        let (plain, _) = crate::recipe::parse_recipe(&crate::test_helpers::recipe_with(
            None,
            None,
            None,
            Some(&[("OTHER", "x")]),
            "",
        ));
        assert_eq!(crate::recipe::recipe_hive_cli(&plain.unwrap()), "");
    }

    #[test]
    fn remote_hive_cli() {
        let _serial = crate::test_helpers::serial();
        assert_eq!(
            crate::run::remote_hive_cli(None),
            crate::pair::PAIR_HIVE_PATH
        );
        let (r, problems) = crate::recipe::parse_recipe(&crate::test_helpers::recipe_with(
            None,
            None,
            None,
            Some(&[("HIVE_CLI", "/opt/hive/bin/hive")]),
            "",
        ));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            crate::run::remote_hive_cli(r.as_ref()),
            "/opt/hive/bin/hive"
        );
    }

    static HIVE_REMOTE_RAN: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);

    fn fake_hive_missing_ssh(
        _cancel: &hive_bus::CancellationToken,
        argv: &[String],
        _stdin: &[u8],
        _timeout: Duration,
    ) -> Result<(i32, String, String)> {
        let args = argv
            .last()
            .and_then(|s| s.strip_prefix("b64 "))
            .and_then(|s| {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.decode(s).ok()
            })
            .map(|v| {
                v.split(|b| *b == 0)
                    .filter(|p| !p.is_empty())
                    .map(|p| String::from_utf8_lossy(p).to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| argv.to_vec());
        match args.first().map(String::as_str) {
            Some("verify") => Ok((0, String::new(), String::new())),
            Some("probe") => Ok((
                0,
                "hands=ok hive=missing hive_lookup=/usr/bin:/bin heartbeat=1".into(),
                String::new(),
            )),
            Some("run") => {
                HIVE_REMOTE_RAN.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok((0, String::new(), String::new()))
            }
            _ => Ok((0, String::new(), String::new())),
        }
    }

    #[test]
    fn hive_on_path_remote_preflight() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("run.key");
        std::fs::write(&key, b"key").unwrap();
        let mut row = Row::new();
        row.insert("name", serde_json::json!("hands"));
        row.insert("address", serde_json::json!("runner@example.invalid"));
        let mut job = Row::new();
        job.insert("id", serde_json::json!("job-remote"));
        let local = LocalExec {
            s: Some((*s).clone()),
            row,
            job,
            clone_dir: dir.path().join("clone").display().to_string(),
            key: key.display().to_string(),
            ..Default::default()
        };
        let mut e = SshExec {
            local,
            job_dir: dir.path().display().to_string(),
            bundle: String::new(),
            base: String::new(),
        };
        let old = crate::executor::run_ssh_hook();
        crate::executor::set_run_ssh(fake_hive_missing_ssh);
        HIVE_REMOTE_RAN.store(false, std::sync::atomic::Ordering::SeqCst);
        let err = e
            .run_worker(
                &hive_bus::CancellationToken::new(),
                &["copilot".into()],
                &BTreeMap::new(),
                1,
            )
            .unwrap_err();
        crate::executor::restore_run_ssh(old);
        assert!(
            err.downcast_ref::<crate::executor::ExecutorFault>()
                .is_some(),
            "{err}"
        );
        let text = err.to_string();
        assert!(text.contains("hive is not on PATH"), "{text}");
        assert!(text.contains("looked in PATH=/usr/bin:/bin"), "{text}");
        assert!(!HIVE_REMOTE_RAN.load(std::sync::atomic::Ordering::SeqCst));
    }

    /// A fake hand that answers every verb at once and, like a real ssh
    /// child, ends with the cancellation when the caller's token is
    /// cancelled.
    fn fake_hand_ssh(
        cancel: &hive_bus::CancellationToken,
        argv: &[String],
        _stdin: &[u8],
        _timeout: Duration,
    ) -> Result<(i32, String, String)> {
        let verb = argv
            .last()
            .and_then(|s| s.strip_prefix("b64 "))
            .and_then(|s| {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.decode(s).ok()
            })
            .and_then(|v| {
                v.split(|b| *b == 0)
                    .find(|p| !p.is_empty())
                    .map(|p| String::from_utf8_lossy(p).to_string())
            })
            .unwrap_or_default();
        if cancel.is_cancelled() {
            return Err(hive_bus::ContextError::Canceled.into());
        }
        match verb.as_str() {
            "probe" => Ok((0, "hands=ok hive=ok heartbeat=1".into(), String::new())),
            _ => Ok((0, String::new(), String::new())),
        }
    }

    #[test]
    fn a_hand_run_the_runner_stops_ends_as_a_stop_not_as_a_result() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("run.key");
        std::fs::write(&key, b"key").unwrap();
        let mut row = Row::new();
        row.insert("name", serde_json::json!("hands"));
        row.insert("address", serde_json::json!("runner@example.invalid"));
        let mut job = Row::new();
        job.insert("id", serde_json::json!("job-remote"));
        let mut e = SshExec {
            local: LocalExec {
                s: Some((*s).clone()),
                row,
                job,
                home: s.home.clone(),
                agent: "aurora".into(),
                clone_dir: dir.path().join("clone").display().to_string(),
                key: key.display().to_string(),
                ..Default::default()
            },
            job_dir: dir.path().display().to_string(),
            bundle: String::new(),
            base: String::new(),
        };
        let old = crate::executor::run_ssh_hook();
        crate::executor::set_run_ssh(fake_hand_ssh);
        // Stopped by the runner (a restart asked for now, the stop grace):
        // the worker reads as cut before its cap, so run.rs loses the run
        // and requeues the job, and the check gives no verdict.
        let stopped = hive_bus::CancellationToken::new();
        stopped.cancel();
        let out = e
            .run_worker(&stopped, &["copilot".into()], &BTreeMap::new(), 1)
            .unwrap();
        assert!(out.timeout, "{out:?}");
        assert_eq!(out.exit_code, -1, "{out:?}");
        assert!(out.stderr.contains("cancel"), "{out:?}");
        let (rc, _) = e.verify(&stopped, "true", 1);
        assert_eq!(rc, None);
        // Not stopped: the hand's answers stand.
        let live = hive_bus::CancellationToken::new();
        let out = e
            .run_worker(&live, &["copilot".into()], &BTreeMap::new(), 1)
            .unwrap();
        assert!(!out.timeout, "{out:?}");
        assert_eq!(out.exit_code, 0, "{out:?}");
        let (rc, _) = e.verify(&live, "true", 1);
        assert_eq!(rc, Some(0));
        crate::executor::restore_run_ssh(old);
    }

    #[test]
    fn lead_runs_with_the_light_model() {
        let _serial = crate::test_helpers::serial();
        hive_bus::reset_default();
        let s = crate::test_helpers::new_store();
        insert_test_project(&s);
        set_setting(&s, "worker_model", "heavy-test");
        let (lead, created) = crate::lead::ensure_lead(&s, "proj".to_string()).unwrap();
        assert!(created, "lead: {lead:?}");
        assert_eq!(lead.as_ref().map(|a| a.model.as_str()), Some("heavy-test"));
        set_setting(&s, "light_model", "light-test");
        let (cmd, _) = worker_command(&s, "lead", "brief", "/tmp/clone", "", false);
        assert_eq!(model_arg(&cmd), "light-test");
        let (lead2, _) = crate::board::join_agent(
            &s,
            "other-lead".to_string(),
            String::new(),
            s.run_model("lead"),
            String::new(),
            String::new(),
            crate::lead::LEAD_KIND.to_string(),
            String::new(),
        )
        .unwrap();
        assert_eq!(lead2.str("model"), "light-test");
    }

    #[test]
    fn pick_worker_keeps_workers_of_every_model() {
        let _serial = crate::test_helpers::serial();
        hive_bus::reset_default();
        let s = crate::test_helpers::new_store();
        set_setting(&s, "quorum", "1");
        set_setting(&s, "worker_model", "gpt-5-mini");
        let mut old = Vec::new();
        for _ in 0..2 {
            let r = crate::board::spawn_agent(
                &s,
                hive_store::HARNESS_RUNNER.to_string(),
                "gpt-5-mini".to_string(),
                "worker".to_string(),
            )
            .unwrap();
            old.push(r.str("name"));
        }
        set_setting(&s, "worker_model", "gpt-6-astra");
        set_setting(&s, "light_model", "gpt-5-mini");
        for _ in 0..3 {
            let name = crate::run::pick_worker(&s, crate::test_helpers::TEST_NOW, "", &[]).unwrap();
            assert!(
                old.iter().any(|want| want == &name),
                "picked {name}, want one of the pool {old:?}"
            );
        }
        let conn = s.conn().unwrap();
        let n = crate::util::must_one(
            &conn,
            "SELECT COUNT(*) AS n FROM agents WHERE status = 'active' AND kind = 'worker'",
            &[],
        )
        .int("n");
        assert_eq!(n, 2);
    }
}
