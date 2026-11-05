#![allow(dead_code)]

use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use std::process::{Command, Stdio};
use std::sync::{LazyLock, RwLock};
use std::time::{Duration, Instant};

pub(crate) const SELF_UPDATE_AREA: &str = "runner";
pub(crate) const SELF_UPDATE_THREAD: &str = "release";
pub(crate) const SELF_UPDATE_EVENT: &str = "daemon.self_update";
/// How long `cargo build` may go without printing, and how long each smoke
/// command may run. The build is bounded by its silence rather than its
/// length: a one-crate release build measured 246 s here, 186 s of them
/// the final link of `pulse`, which prints nothing while it runs, and a
/// full rebuild after a Cargo.lock bump or a profile change takes longer
/// than ten minutes at the daemon's `-j` (half the cores), so a bound on
/// the length cut one short (`cargo timed out after 10m0s` in the owner's
/// log) and tried again an hour later on the cached half. A build that
/// still prints is still compiling; one that has said nothing for ten
/// minutes is stuck. The smoke tests (`version`, `help`) answer within a
/// second, so their length is the right bound.
pub(crate) const SELF_UPDATE_BUILD_TIMEOUT: Duration = Duration::from_secs(10 * 60);
pub(crate) const RESTORED_MARKER: &str = ".restored";

/// The paths the `pulse` binary is built from, as prefixes of paths from
/// the workspace root. A landing that touches none of them is not built:
/// of the 39 landings on the brain's main over four days, each of which
/// cost a release build and a restart, three changed only docs, the
/// README, a script or the macOS app. The console's sources are here
/// because pulse-hub embeds its static export, `web/hub/out`, into the
/// binary through rust_embed (the `folder` attribute in
/// crates/pulse-hub/src/server.rs), and that export is not committed:
/// crates/pulse-hub/build.rs builds it from exactly these paths under
/// `web/hub` (its `INPUTS` list) whenever one of them changed, so a landing
/// that touches one of them changes `pulse` too, and one that touches only
/// `web/hub/mock` does not. The test `every_embedded_path_is_a_binary_input`
/// walks the crates for `include_str!`, `include_bytes!` and rust_embed
/// folders and fails when one lies outside this list.
pub(crate) const BINARY_INPUTS: &[&str] = &[
    "crates/",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain",
    ".cargo/",
    "web/hub/src/",
    "web/hub/public/",
    "web/hub/package.json",
    "web/hub/package-lock.json",
    "web/hub/next.config.ts",
    "web/hub/tsconfig.json",
    "web/hub/postcss.config.mjs",
    "web/hub/components.json",
];

/// Whether a path from the workspace root is one of [`BINARY_INPUTS`] or
/// under one: `starts_with` covers both the equal path and the children.
pub(crate) fn is_binary_input(path: &str) -> bool {
    BINARY_INPUTS.iter().any(|input| path.starts_with(input))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct SelfUpdateState {
    pub(crate) inflight: bool,
    pub(crate) lines: Vec<String>,
    pub(crate) last_failure_at: String,
    pub(crate) last_skip: String,
    pub(crate) installed: String,
    /// A build is due but waits for the experiment lane to finish its
    /// measurement; the lane starts no new experiment while this is set.
    pub(crate) lane_wait: bool,
    /// A build is due but waits for the machine: saturated, or every slot
    /// under the machine's capacity holds a run. The dispatcher keeps one local
    /// slot free while this is set so the build starts within one run.
    pub(crate) build_wait: bool,
    /// The brain HEAD last seen differing from the running binary and when
    /// it was first seen there (store clock): a build starts only once it
    /// has stood for `daemon.self_update_settle_minutes`.
    pub(crate) settle_rev: String,
    pub(crate) settle_since: String,
    /// Since when (store clock) the build waits for queued work the restart
    /// it ends in would refuse; empty while it does not.
    pub(crate) refused_since: String,
    /// The brain HEAD last found to differ from the running binary only
    /// outside [`BINARY_INPUTS`]: it is not built, and the pass does not
    /// diff it again. The next HEAD is diffed against the running revision,
    /// not against this one, so a later landing that does touch an input
    /// is built with everything since.
    pub(crate) neutral_rev: String,
}

/// How long the build waits for queued work the restart would refuse to
/// start first: past an hour the queue is stuck on something a restart does
/// not change, and the update should not wait behind it. The owner's work
/// is never among the refused - it starts in any restart window - so this
/// wait never holds a build for it.
pub(crate) const REFUSED_WORK_WAIT_MINUTES: f64 = 60.0;

pub(crate) static SELF_UPDATE_STATE: LazyLock<RwLock<SelfUpdateState>> =
    LazyLock::new(|| RwLock::new(SelfUpdateState::default()));

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct SelfUpdateEnv {
    pub binary: String,
    pub cargo_cmd: String,
    pub timeout: std::time::Duration,
    pub revision: String,
    pub actor: String,
    pub inflight: i32,
    /// The experiment lane is measuring; the daemon sets it per pass.
    pub measuring: bool,
    /// Local runs in flight on this machine; the daemon sets it per pass.
    pub local_inflight: i32,
    /// The load governor's limits; None (tests, `pulse work`) builds
    /// whenever the checkout differs, as before the governor.
    #[serde(skip)]
    pub governor: Option<crate::daemon_load::Limits>,
    /// The queued work a restart armed now would refuse to start while the
    /// runs in flight finish ("improve runs on brain"); the daemon sets it
    /// per pass, empty when there is none. The owner's work is not in it:
    /// the restart starts it in any window.
    pub refused_queued: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct SelfUpdateOutcome {
    pub(crate) line: String,
    pub(crate) installed: bool,
}

pub(crate) fn short_rev(rev: String) -> String {
    if rev.len() > 12 {
        rev[..12].to_string()
    } else if rev.is_empty() {
        "unstamped".into()
    } else {
        rev
    }
}

/// True while a self-update is building or waits for the experiment lane
/// or for room on the machine; the lane then measures nothing new and the
/// dispatcher keeps one slot free, so the update is never starved and a
/// measurement is never thrown away by the restart it brings.
pub(crate) fn update_pending() -> bool {
    let st = SELF_UPDATE_STATE
        .read()
        .expect("self-update state poisoned");
    st.inflight || st.lane_wait || st.build_wait
}

/// One daemon pass over the brain's HEAD: reads the state and the settings,
/// reads the checkout ([`Checkout`]) and, once HEAD is due, the machine
/// ([`Bench`]), climbs the ladder ([`gate`]) and applies what it decides -
/// says the wait once, or starts the build on a thread. The lines that
/// thread left are returned first.
pub(crate) fn tend_binary(s: &pulse_store::Store, now: String, env: SelfUpdateEnv) -> Vec<String> {
    let mut st = SELF_UPDATE_STATE
        .write()
        .expect("self-update state poisoned");
    let mut lines = std::mem::take(&mut st.lines);
    if st.inflight {
        return lines;
    }
    // Each pass decides afresh whether a due build waits for the lane or
    // for the machine.
    st.lane_wait = false;
    st.build_wait = false;
    let say = |st: &mut SelfUpdateState, lines: &mut Vec<String>, reason: String| {
        if reason != st.last_skip {
            st.last_skip = reason.clone();
            lines.push(format!("self-update: {reason}"));
        }
    };
    if s.setting("daemon.self_update", "") != "on" {
        return lines;
    }
    if s.setting("stop", "") == "1" {
        say(&mut st, &mut lines, "stop is set; no build".into());
        return lines;
    }
    if !st.last_failure_at.is_empty() {
        let retry = s
            .setting("daemon.self_update_retry_minutes", "60")
            .parse::<i32>()
            .unwrap_or(0);
        let retry = if retry < 1 { 60 } else { retry };
        if crate::util::must_iso(&now)
            < crate::util::must_iso(&crate::util::plus_minutes(
                &st.last_failure_at,
                f64::from(retry),
            ))
        {
            return lines;
        }
    }
    let brain = s
        .conn()
        .ok()
        .map(|conn| crate::board::brain_project(&conn))
        .unwrap_or_default();
    if brain.0.is_empty() {
        say(
            &mut st,
            &mut lines,
            "no brain project; nothing to build from".into(),
        );
        return lines;
    }
    let def = first_non_empty(&[brain.str("default_branch"), "main".into()]);
    let mut co = match Checkout::read(brain.str("path"), def) {
        Ok(co) => co,
        Err(reason) => {
            say(&mut st, &mut lines, reason);
            return lines;
        }
    };
    // On most passes HEAD is what runs, what is installed or a landing
    // already found neutral: the diff and the probes below are spared.
    if !co.due(&st, &env) {
        return lines;
    }
    // The binary the daemon runs next is the installed one when a build or a
    // hand install already waits for its restart: a landing is neutral
    // against that, not against the older running one. On 2026-11-05 a
    // hand-installed 1355fb59 waited for the runs in flight while #76 landed
    // one script; measured from the running 930e1454 the diff spanned
    // every crate and the settle clock started for a build nobody needed.
    let base = if st.installed.is_empty() {
        env.revision.clone()
    } else {
        st.installed.clone()
    };
    co.diff_from(&base);
    let bench = Bench::read(std::path::Path::new(&s.home), &env, &co.rev);
    let settle = s.setting_int("daemon.self_update_settle_minutes").max(0);
    match gate(&mut st, settle, &now, &env, &co, &bench) {
        Gate::Quiet => {}
        Gate::Wait(reason) => say(&mut st, &mut lines, reason),
        Gate::Build => {
            st.last_skip.clear();
            st.inflight = true;
            lines.push(format!(
                "self-update: brain HEAD {} differs from the running {}; building",
                short_rev(co.rev.clone()),
                short_rev(env.revision.clone())
            ));
            let s2 = s.clone();
            let (repo, rev) = (co.repo, co.rev);
            std::thread::spawn(move || {
                let outcome = build_and_install(&s2, repo, rev.clone(), env);
                let mut st = SELF_UPDATE_STATE
                    .write()
                    .expect("self-update state poisoned");
                st.inflight = false;
                st.lines.push(outcome.line);
                if outcome.installed {
                    st.installed = rev;
                    st.last_failure_at.clear();
                } else {
                    st.last_failure_at = s2.now();
                }
            });
        }
    }
    lines
}

/// What the brain checkout says, read once per pass: HEAD, the branch,
/// whether the tree is clean and, once HEAD is due, what changed between
/// the running revision and HEAD.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Checkout {
    pub(crate) repo: String,
    pub(crate) rev: String,
    pub(crate) branch: String,
    pub(crate) default_branch: String,
    pub(crate) dirty: bool,
    /// The paths `git diff --name-only <base> <HEAD>` names, from the
    /// workspace root, when the base revision is known to the checkout.
    /// None when it is not (an unstamped build, rewritten history) or the
    /// diff was not run; the pass then builds as it did before the diff.
    pub(crate) changed: Option<Vec<String>>,
    /// The revision `changed` was measured from: the one installed and
    /// waiting for its restart when there is one, else the running one.
    pub(crate) base: String,
}

impl Checkout {
    /// Reads HEAD, the branch and the tree's state; Err is the line the
    /// pass says when the checkout cannot be read.
    pub(crate) fn read(repo: String, default_branch: String) -> Result<Self, String> {
        let head = crate::util::git(&repo, &["rev-parse", "HEAD"]);
        let branch = crate::util::git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"]);
        let status = crate::util::git(&repo, &["status", "--porcelain"]);
        if !head.ok() || !branch.ok() || !status.ok() {
            return Err(format!(
                "{repo} is not a readable git checkout: {}",
                git_reason(&[head, branch, status])
            ));
        }
        Ok(Self {
            rev: head.stdout.trim().to_string(),
            branch: branch.stdout.trim().to_string(),
            dirty: !status.stdout.trim().is_empty(),
            changed: None,
            base: String::new(),
            repo,
            default_branch,
        })
    }

    /// HEAD is neither the running binary, the one installed and waiting
    /// for its restart, nor a landing already found neutral: the pass has
    /// a decision to make.
    pub(crate) fn due(&self, st: &SelfUpdateState, env: &SelfUpdateEnv) -> bool {
        self.rev != st.installed && self.rev != env.revision && self.rev != st.neutral_rev
    }

    /// Fills `changed` with the paths that differ between `base` and HEAD
    /// when git can tell; an unknown base revision leaves it None.
    pub(crate) fn diff_from(&mut self, base: &str) {
        if base.is_empty() {
            return;
        }
        self.base = base.to_string();
        let diff = crate::util::git(&self.repo, &["diff", "--name-only", base, &self.rev]);
        if diff.ok() {
            self.changed = Some(
                diff.stdout
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(String::from)
                    .collect(),
            );
        }
    }

    /// Whether HEAD changes nothing the binary is built from: every path
    /// in `changed` lies outside [`BINARY_INPUTS`], or none differs at all
    /// (a revert leaves identical trees). Unknown counts as not neutral.
    pub(crate) fn neutral(&self) -> bool {
        self.changed
            .as_ref()
            .is_some_and(|changed| !changed.iter().any(|path| is_binary_input(path)))
    }
}

/// What the machine offers the build, read once HEAD is due and before the
/// decision, so the decision itself runs no process.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Bench {
    /// `<binary> version` already prints HEAD: a human built and installed
    /// it while this process still runs the old one.
    pub(crate) installed_by_hand: bool,
    pub(crate) cargo_on_path: bool,
    pub(crate) bulk_missing: bool,
    /// Why the load governor holds the build, when it does.
    pub(crate) hold: Option<String>,
}

impl Bench {
    pub(crate) fn read(home: &std::path::Path, env: &SelfUpdateEnv, rev: &str) -> Self {
        Self {
            installed_by_hand: installed_binary_is(&env.binary, rev),
            cargo_on_path: which::which(&env.cargo_cmd).is_ok(),
            bulk_missing: crate::bulk::state(home).is_missing(),
            hold: env.governor.as_ref().and_then(|limits| {
                crate::daemon_load::build_hold(
                    limits,
                    &crate::daemon_load::pressure(),
                    env.local_inflight.max(0) as usize,
                )
            }),
        }
    }
}

/// What one pass does about the brain's HEAD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Gate {
    /// HEAD is the running, the installed or a neutral revision: nothing.
    Quiet,
    /// The build waits for the named reason; the pass says it once.
    Wait(String),
    /// HEAD is built now.
    Build,
}

/// The ladder of gates a due HEAD climbs, in the order it climbs them,
/// with no I/O of its own: the checkout and the bench were read before,
/// the settle time comes from the settings and `now` from the store's
/// clock. Each rung that waits names why; what it leaves in the state
/// (`installed`, `neutral_rev`, `settle_*`, `refused_since`, `lane_wait`,
/// `build_wait`) is what the next pass and the dispatcher read.
pub(crate) fn gate(
    st: &mut SelfUpdateState,
    settle: i64,
    now: &str,
    env: &SelfUpdateEnv,
    co: &Checkout,
    bench: &Bench,
) -> Gate {
    if !co.due(st, env) {
        return Gate::Quiet;
    }
    let rev = co.rev.clone();
    // A human may have built and installed HEAD by hand while this process
    // still runs the old binary; the file watcher restarts onto it, so a
    // build would only redo that work on a machine the runs need.
    if bench.installed_by_hand {
        st.installed = rev.clone();
        return Gate::Wait(format!(
            "{} is already installed at {}; restarting onto it after the runs in flight",
            short_rev(rev),
            env.binary
        ));
    }
    // A landing that touches nothing the binary is built from - docs, the
    // README, a script, the macOS app - would rebuild the same binary and
    // restart the daemon for nothing; three of the brain's last 39 did.
    if co.neutral() {
        st.neutral_rev = rev.clone();
        let (base, word) = if !co.base.is_empty() && co.base != env.revision {
            (co.base.as_str(), "installed")
        } else {
            (env.revision.as_str(), "running")
        };
        return Gate::Wait(neutral_line(
            &rev,
            base,
            word,
            co.changed.as_deref().unwrap_or_default(),
        ));
    }
    if co.branch != co.default_branch {
        return Gate::Wait(format!(
            "brain checkout is on {}, not {}; waiting",
            co.branch, co.default_branch
        ));
    }
    if co.dirty {
        return Gate::Wait("brain checkout is dirty; waiting for it to be clean".into());
    }
    // A release build is three to seven minutes of half the cores and ends
    // in a restart; a fleet lands a burst of brain branches minutes apart.
    // The build waits until HEAD has stood unchanged for a while, so the
    // burst builds once, from its last commit. A new HEAD restarts the clock.
    if settle > 0 {
        if st.settle_rev != rev {
            st.settle_rev = rev.clone();
            st.settle_since = now.to_string();
        }
        let due = crate::util::plus_minutes(&st.settle_since, settle as f64);
        if crate::util::must_iso(now) < crate::util::must_iso(&due) {
            return Gate::Wait(format!(
                "brain HEAD {} differs from the running {}; building once it has stood for {settle} min (daemon.self_update_settle_minutes), so a burst of landings builds once",
                short_rev(rev.clone()),
                short_rev(env.revision.clone())
            ));
        }
    }
    // The build ends in a restart, and a restart armed with runs in flight
    // starts only the owner's work and the kinds of run that end before
    // them: a fleet improve job queued behind it would wait the whole
    // window. It starts first.
    if !env.refused_queued.is_empty() {
        if st.refused_since.is_empty() {
            st.refused_since = now.to_string();
        }
        let until = crate::util::plus_minutes(&st.refused_since, REFUSED_WORK_WAIT_MINUTES);
        if crate::util::must_iso(now) < crate::util::must_iso(&until) {
            return Gate::Wait(format!(
                "brain HEAD {} differs from the running {}; building once the queued {} have started, as the restart the build ends in would refuse them while the runs in flight finish",
                short_rev(rev.clone()),
                short_rev(env.revision.clone()),
                env.refused_queued
            ));
        }
    } else {
        st.refused_since.clear();
    }
    if !bench.cargo_on_path {
        return Gate::Wait(format!(
            "{} is not on the daemon's PATH; builds are yours until it is",
            env.cargo_cmd
        ));
    }
    // A checkout's build dir may live on the bulk volume (bulk.rs); a build
    // into a dangling link fails, and the restart it ends in can wait.
    if bench.bulk_missing {
        return Gate::Wait("bulk storage away; building once the disk is back".into());
    }
    if env.measuring {
        st.lane_wait = true;
        return Gate::Wait(format!(
            "brain HEAD {} differs from the running {}; building once the experiment lane is idle",
            short_rev(rev.clone()),
            short_rev(env.revision.clone())
        ));
    }
    // The build is a full release compile; on a saturated laptop it would
    // fight the runs for the machine, so it waits for room (daemon_load.rs).
    if let Some(hold) = bench.hold.as_ref() {
        st.build_wait = true;
        return Gate::Wait(format!(
            "brain HEAD {} differs from the running {}; building once the machine has room: {hold}",
            short_rev(rev.clone()),
            short_rev(env.revision.clone())
        ));
    }
    st.refused_since.clear();
    Gate::Build
}

/// The line a neutral landing is said with: the revision it was measured
/// from (`word` says whether that one runs or waits installed), how many
/// paths changed and the first three of them, so the log shows what the
/// landing was.
fn neutral_line(rev: &str, base: &str, word: &str, changed: &[String]) -> String {
    let files = match changed.len() {
        0 => "the trees are identical".to_string(),
        n => {
            let shown = changed
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            if n > 3 {
                format!("{n} file(s): {shown} and {} more", n - 3)
            } else {
                format!("{n} file(s): {shown}")
            }
        }
    };
    format!(
        "brain HEAD {} differs from the {word} {} only outside what the binary is built from ({files}); no build",
        short_rev(rev.to_string()),
        short_rev(base.to_string())
    )
}

/// Whether the binary at `path` already prints `rev`'s short form as its
/// version, the same check the install smoke test applies to a new build.
fn installed_binary_is(path: &str, rev: &str) -> bool {
    if path.is_empty() || rev.is_empty() {
        return false;
    }
    std::process::Command::new(path)
        .arg("version")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .is_some_and(|out| {
            String::from_utf8_lossy(&out.stdout).contains(&short_rev(rev.to_string()))
        })
}

pub(crate) fn build_and_install(
    s: &pulse_store::Store,
    repo: String,
    rev: String,
    env: SelfUpdateEnv,
) -> SelfUpdateOutcome {
    let timeout = if env.timeout.is_zero() {
        SELF_UPDATE_BUILD_TIMEOUT
    } else {
        env.timeout
    };
    let new_binary = format!("{}.new", env.binary);
    let _ = std::fs::remove_file(&new_binary);
    let target_dir = std::path::Path::new(&repo).join("target");
    // daemon.build_jobs, half the cores by default: the build never takes
    // the whole machine from the runs.
    let jobs = env
        .governor
        .as_ref()
        .map(|limits| limits.build_jobs)
        .filter(|jobs| *jobs > 0)
        .unwrap_or_else(|| crate::daemon_load::Machine::detect().build_jobs());
    let build_args = vec![
        "build".to_string(),
        "--release".to_string(),
        "--locked".to_string(),
        "--bin".to_string(),
        "pulse".to_string(),
        "--target-dir".to_string(),
        target_dir.display().to_string(),
        "-j".to_string(),
        jobs.to_string(),
    ];
    // The build is bounded by its silence, the smoke tests by their length
    // (SELF_UPDATE_BUILD_TIMEOUT says why).
    let (out, err) = run_in(
        repo.clone(),
        Bound::Quiet(timeout),
        env.cargo_cmd.clone(),
        &build_args,
    );
    if let Some(err) = err {
        return self_update_failure(s, rev, env, "build".into(), out, err);
    }
    let built = target_dir.join("release").join("pulse");
    if let Err(err) = std::fs::copy(&built, &new_binary) {
        return self_update_failure(s, rev, env, "build".into(), String::new(), err.into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(mut perms) = std::fs::metadata(&new_binary).map(|m| m.permissions()) {
            perms.set_mode(0o755);
            let _ = std::fs::set_permissions(&new_binary, perms);
        }
    }
    if let Err(err) = crate::sign::sign_new_build(s, &new_binary) {
        return self_update_failure(s, rev, env, "sign".into(), String::new(), err);
    }
    let version_args = vec!["version".to_string()];
    let (version, err) = run_in(
        repo.clone(),
        Bound::Total(timeout),
        new_binary.clone(),
        &version_args,
    );
    if let Some(err) = err {
        return self_update_failure(s, rev, env, "smoke".into(), version, err);
    }
    if !version.contains(&short_rev(rev.clone())) {
        return self_update_failure(
            s,
            rev.clone(),
            env,
            "smoke".into(),
            version.clone(),
            anyhow!(
                "pulse.new version printed {:?}, not the revision {}",
                version.trim(),
                short_rev(rev)
            ),
        );
    }
    let help_args = vec!["help".to_string()];
    let (out, err) = run_in(
        repo.clone(),
        Bound::Total(timeout),
        new_binary.clone(),
        &help_args,
    );
    if let Some(err) = err {
        return self_update_failure(s, rev, env, "smoke".into(), out, err);
    }
    let prev = format!("{}.prev", env.binary);
    let _ = std::fs::remove_file(&prev);
    if let Err(err) = std::fs::rename(&env.binary, &prev) {
        return self_update_failure(s, rev, env, "install".into(), String::new(), err.into());
    }
    if let Err(err) = std::fs::rename(&new_binary, &env.binary) {
        let _ = std::fs::rename(&prev, &env.binary);
        return self_update_failure(s, rev, env, "install".into(), String::new(), err.into());
    }
    let now = s.now();
    let detail = serde_json::json!({"from": env.revision, "to": rev, "binary": env.binary, "inflight": env.inflight});
    if let Err(err) = pulse_bus::emit(s, &env.actor, SELF_UPDATE_EVENT, "pulse", detail) {
        return SelfUpdateOutcome {
            installed: true,
            line: format!(
                "self-update: installed {}; event not written: {}",
                short_rev(rev),
                crate::run::err_message(&err)
            ),
        };
    }
    let summary = format!(
        "pulse rebuilt from {}: restarts after {} run(s) in flight",
        short_rev(rev.clone()),
        env.inflight
    );
    let body = format!(
        "The brain's default branch landed at `{rev}` while the daemon ran `{}`. The daemon built `{}` from the clean checkout at {repo}, checked that it prints its revision and answers `help`, kept the running binary as `{prev}` and installed the new one at {now}.\n\nThe daemon and the hub restart on the changed file: the daemon after the {} run(s) in flight, the hub within a minute. Roll back with `mv {prev} {}` (the daemon restarts again); `pulse settings set daemon.self_update off` stops these builds (ADR-035).\n",
        first_non_empty(&[env.revision.clone(), "an unstamped build".into()]),
        env.binary,
        env.inflight,
        env.binary,
    );
    if let Err(err) = self_update_finding(s, env.actor, summary.clone(), body) {
        return SelfUpdateOutcome {
            installed: true,
            line: format!(
                "self-update: installed {}; finding not posted: {}",
                short_rev(rev),
                crate::run::err_message(&err)
            ),
        };
    }
    SelfUpdateOutcome {
        installed: true,
        line: format!("self-update: {summary}"),
    }
}

pub(crate) fn self_update_failure(
    s: &pulse_store::Store,
    rev: String,
    env: SelfUpdateEnv,
    stage: String,
    output: String,
    err: anyhow::Error,
) -> SelfUpdateOutcome {
    let tail =
        crate::compute::redact_secrets(crate::executor::last_lines(&output, 20).join("\n").trim());
    let mut line = format!(
        "self-update: {} of {} failed: {}",
        stage,
        short_rev(rev.clone()),
        crate::run::err_message(&err)
    );
    let retry = s.setting("daemon.self_update_retry_minutes", "60");
    let summary = format!(
        "pulse {} of {} failed; the running binary is untouched",
        stage,
        short_rev(rev.clone())
    );
    let body = format!(
        "The daemon tried to rebuild itself from the brain's `{rev}` and the {stage} step failed: {}\n\n`{}` was not touched; `{}.new` is kept for reading when it exists. The daemon tries again in {retry} minutes (daemon.self_update_retry_minutes); `pulse settings set daemon.self_update off` stops it (ADR-035).\n\n```\n{tail}\n```\n",
        crate::run::err_message(&err),
        env.binary,
        env.binary,
    );
    if let Err(perr) = self_update_finding(s, env.actor, summary, body) {
        line.push_str("; finding not posted: ");
        line.push_str(&crate::run::err_message(&perr));
    }
    SelfUpdateOutcome {
        line,
        installed: false,
    }
}

pub(crate) fn self_update_finding(
    s: &pulse_store::Store,
    actor: String,
    summary: String,
    body: String,
) -> anyhow::Result<()> {
    let brain = s
        .conn()
        .ok()
        .map(|conn| crate::board::brain_project(&conn))
        .unwrap_or_default();
    if brain.0.is_empty() {
        return Err(anyhow!("no brain project"));
    }
    crate::board::create_post(
        s,
        s.now(),
        crate::board::PostOpts {
            kind: "finding".into(),
            project: brain.str("slug"),
            area: SELF_UPDATE_AREA.into(),
            thread: SELF_UPDATE_THREAD.into(),
            summary: crate::util::trunc(&summary, 230),
            body,
            agent: crate::board::actor_agent(s, actor),
            ..Default::default()
        },
    )?;
    Ok(())
}

/// How long a child of [`run_in`] may go on. `Total` bounds its length;
/// `Quiet` the time since it last wrote a byte to either pipe, which is the
/// bound for a build: a release build prints a line per crate and is
/// silent only while it links, and its length varies with how much of the
/// target dir is warm, from four minutes to more than the ten a length
/// bound allowed (SELF_UPDATE_BUILD_TIMEOUT).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Bound {
    Total(Duration),
    Quiet(Duration),
}

/// The child runs at nice [`crate::daemon_load::CHILD_NICE`]: it is the
/// daemon's own build or smoke test, never the owner's work. Its output is
/// drained while it runs (a build with many warnings once filled the pipe
/// and hung until the timeout), and the wait polls ten times a second
/// against `bound`; a child past it is killed, and the error says which
/// bound it passed (`timed out after` or `produced no output for`).
pub(crate) fn run_in(
    dir: String,
    bound: Bound,
    name: String,
    args: &[String],
) -> (String, Option<anyhow::Error>) {
    let mut cmd = Command::new(&name);
    cmd.args(args)
        .current_dir(dir)
        .envs(
            crate::util::strip_hook_git_env(
                &std::env::vars()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>(),
            )
            .into_iter()
            .filter_map(|kv| {
                kv.split_once('=')
                    .map(|(k, v)| (k.to_string(), v.to_string()))
            }),
        )
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::daemon_load::low_priority(&mut cmd);
    let base = std::path::Path::new(&name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&name)
        .to_string();
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => return (String::new(), Some(err.into())),
    };
    let started = Instant::now();
    let stdout = Drained::read(child.stdout.take());
    let stderr = Drained::read(child.stderr.take());
    let text = |grace: Duration| {
        let mut text = String::from_utf8_lossy(&stdout.bytes(grace)).into_owned();
        text.push_str(&String::from_utf8_lossy(&stderr.bytes(grace)));
        text
    };
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                let passed = match bound {
                    Bound::Total(limit) => started.elapsed() >= limit,
                    Bound::Quiet(limit) => {
                        stdout.last_output().max(stderr.last_output()).elapsed() >= limit
                    }
                };
                if passed {
                    let _ = child.kill();
                    let _ = child.wait();
                    let why = match bound {
                        Bound::Total(limit) => format!(
                            "{base} timed out after {}",
                            pulse_cli::format_duration(limit)
                        ),
                        Bound::Quiet(limit) => format!(
                            "{base} produced no output for {}",
                            pulse_cli::format_duration(limit)
                        ),
                    };
                    return (text(Duration::ZERO), Some(anyhow!(why)));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                return (text(Duration::ZERO), Some(err.into()));
            }
        }
    };
    let text = text(Duration::from_secs(5));
    if status.success() {
        (text, None)
    } else {
        (text, Some(anyhow!(pulse_hooks::exit_text(status))))
    }
}

/// A child's pipe read to its end on a thread, so a chatty build never
/// fills the pipe and stalls; the bytes are taken once the child is gone,
/// and the time of the last byte is kept for the silence bound.
struct Drained {
    buf: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    closed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// When the pipe last yielded a byte; the spawn, until it does.
    last: std::sync::Arc<std::sync::Mutex<Instant>>,
}

impl Drained {
    fn read<R: std::io::Read + Send + 'static>(pipe: Option<R>) -> Self {
        let buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let closed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(pipe.is_none()));
        let last = std::sync::Arc::new(std::sync::Mutex::new(Instant::now()));
        if let Some(mut pipe) = pipe {
            let sink = buf.clone();
            let done = closed.clone();
            let seen = last.clone();
            let spawned = std::thread::Builder::new()
                .name("selfupdate-drain".into())
                .spawn(move || {
                    let mut chunk = [0u8; 8192];
                    while let Ok(n) = std::io::Read::read(&mut pipe, &mut chunk) {
                        if n == 0 {
                            break;
                        }
                        sink.lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .extend_from_slice(&chunk[..n]);
                        *seen.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
                    }
                    done.store(true, std::sync::atomic::Ordering::SeqCst);
                });
            if spawned.is_err() {
                closed.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        Self { buf, closed, last }
    }

    /// When the pipe last yielded a byte, or when the read began.
    fn last_output(&self) -> Instant {
        *self.last.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The bytes read so far, after waiting up to `grace` for the pipe to
    /// close (a grandchild that keeps it open bounds the wait, not the daemon).
    fn bytes(&self, grace: Duration) -> Vec<u8> {
        let deadline = std::time::Instant::now() + grace;
        while !self.closed.load(std::sync::atomic::Ordering::SeqCst)
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        self.buf.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

pub(crate) fn git_reason(results: &[crate::util::GitResult]) -> String {
    for r in results {
        if r.ok() {
            continue;
        }
        let stderr = r.stderr.trim();
        if !stderr.is_empty() {
            return stderr.into();
        }
        if let Some(err) = &r.err {
            return err.clone();
        }
        if r.code < 0 {
            return "git was killed before it answered (a folder still held behind a dialog, or a timeout)".into();
        }
        return format!("git exited {} without a message", r.code);
    }
    String::new()
}

pub(crate) fn first_non_empty(values: &[String]) -> String {
    values
        .iter()
        .find(|v| !v.trim().is_empty())
        .cloned()
        .unwrap_or_default()
}

pub(crate) fn check_restored(s: &pulse_store::Store, env: SelfUpdateEnv) -> Vec<String> {
    let marker = format!("{}{}", env.binary, RESTORED_MARKER);
    let Ok(data) = std::fs::read_to_string(&marker) else {
        return Vec::new();
    };
    let when = data.trim().to_string();
    let mut failed = "an unknown build".to_string();
    if let Ok(conn) = s.conn() {
        let detail: rusqlite::Result<String> = conn.query_row(
            "SELECT detail FROM events WHERE action = ? ORDER BY seq DESC LIMIT 1",
            [SELF_UPDATE_EVENT],
            |r| r.get(0),
        );
        if let Ok(detail) = detail {
            let to = json_field(detail, "to".into());
            if !to.is_empty() {
                failed = format!("the build of {}", short_rev(to));
            }
        }
    }
    let mut lines = Vec::new();
    if let Err(err) = s.set_setting(&env.actor, "daemon.self_update", "off") {
        lines.push(format!(
            "self-update: could not turn itself off: {}",
            crate::run::err_message(&err)
        ));
    }
    let summary = format!(
        "restored the previous pulse after 3 early exits: {failed} did not start; self-update is off"
    );
    let body = format!(
        "The launcher saw pulse exit three times in a row within a minute of starting (at {when}) and put `{}.prev` back over `{}`; the failed build is kept as `{}.failed`. The daemon now running is the previous build, `{}`.\n\n`daemon.self_update` is off until you turn it on in Settings: read the daemon log for the failure, fix the brain's default branch, then `pulse settings set daemon.self_update on` (ADR-035).\n",
        env.binary,
        env.binary,
        env.binary,
        first_non_empty(&[env.revision.clone(), "unstamped".into()])
    );
    if let Err(err) = self_update_finding(s, env.actor, summary.clone(), body) {
        lines.push(format!(
            "self-update: restore finding not posted: {}",
            crate::run::err_message(&err)
        ));
    }
    let _ = std::fs::remove_file(marker);
    lines.push(format!("self-update: {summary}"));
    lines
}

pub(crate) fn json_field(text: String, key: String) -> String {
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| v.get(&key).and_then(|v| v.as_str()).map(str::to_string))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{TEST_NOW, count, run, serial};
    use crate::test_helpers_daemon::exec1;
    use crate::test_helpers_daemon::set_setting;
    use crate::test_helpers_daemon::test_wait;
    use crate::test_helpers_selfupdate::{
        head_of, reset_self_update, self_update_fixture, wait_self_update_idle,
    };
    use crate::util::{file_exists, query_one};

    /// Writes `path` under `repo` and commits it; returns the new HEAD.
    fn land(repo: &str, path: &str, text: &str, message: &str) -> String {
        let file = std::path::Path::new(repo).join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
        run(repo, &["git", "add", "-A"]);
        run(repo, &["git", "commit", "-q", "-m", message]);
        head_of(repo)
    }

    #[test]
    fn self_update_builds_smokes_installs_and_reports() {
        let _serial = serial();
        let _guard = pulse_testutil::env_lock();
        reset_self_update();
        let (s, repo, env, _fixture) = self_update_fixture();
        let rev = head_of(&repo);
        let lines = tend_binary(&s, TEST_NOW.into(), env.clone());
        assert!(
            lines.len() == 1
                && lines[0].contains(&format!(
                    "brain HEAD {} differs from the running 000000000000; building",
                    &rev[..12]
                )),
            "first tick: {lines:?}"
        );
        let lines = tend_binary(&s, TEST_NOW.into(), env.clone());
        assert!(
            lines.is_empty(),
            "a second tick during the build said {lines:?}"
        );
        wait_self_update_idle();
        let lines = tend_binary(&s, TEST_NOW.into(), env.clone());
        assert!(
            lines.len() == 1
                && lines[0]
                    == format!(
                        "self-update: pulse rebuilt from {}: restarts after 2 run(s) in flight",
                        &rev[..12]
                    ),
            "after the build: {lines:?}"
        );
        let data = std::fs::read(format!("{}.prev", env.binary)).unwrap_or_default();
        assert_eq!(
            String::from_utf8_lossy(&data),
            "old binary",
            "pulse.prev = {data:?}"
        );
        let data = std::fs::read(&env.binary).unwrap_or_default();
        assert!(
            String::from_utf8_lossy(&data).contains(&format!("dev-{}", &rev[..12])),
            "pulse was not replaced by the build:\n{}",
            String::from_utf8_lossy(&data)
        );
        assert!(
            !file_exists(&format!("{}.new", env.binary)),
            "pulse.new left behind"
        );
        let n = count(
            &s,
            "SELECT count(*) AS n FROM events WHERE action = ? AND detail LIKE ?",
            &[&SELF_UPDATE_EVENT, &format!("%{rev}%")],
        );
        assert_eq!(n, 1, "self_update events = {n}");
        let post = query_one(
            &*s.conn().unwrap(),
            "SELECT * FROM posts WHERE kind = 'finding' AND area = ? AND thread = ? ORDER BY id DESC LIMIT 1",
            &[&SELF_UPDATE_AREA, &SELF_UPDATE_THREAD],
        );
        assert!(
            matches!(&post, Ok(Some(post)) if post.str("project") == "proj"
                && post
                    .str("summary")
                    .contains(&format!("pulse rebuilt from {}", &rev[..12]))
                && post.str("body").contains(&format!("{}.prev", env.binary))),
            "finding: {post:?}"
        );
        let lines = tend_binary(&s, TEST_NOW.into(), env.clone());
        assert!(
            lines.is_empty() && !file_exists(&format!("{}.new", env.binary)),
            "rebuilt the installed revision: {lines:?}"
        );
    }

    #[test]
    fn self_update_skips_the_build_when_head_is_already_installed_by_hand() {
        let _serial = serial();
        let _guard = pulse_testutil::env_lock();
        reset_self_update();
        let (s, repo, env, _fixture) = self_update_fixture();
        let rev = head_of(&repo);
        std::fs::write(
            &env.binary,
            format!("#!/bin/sh\necho v0.0.0-20261009-{}\n", &rev[..12]),
        )
        .unwrap();
        let lines = tend_binary(&s, TEST_NOW.into(), env.clone());
        assert!(
            lines.len() == 1
                && lines[0].contains(&format!(
                    "{} is already installed at {}; restarting onto it",
                    &rev[..12],
                    env.binary
                )),
            "installed by hand: {lines:?}"
        );
        let lines = tend_binary(&s, TEST_NOW.into(), env.clone());
        assert!(
            lines.is_empty() && !file_exists(&format!("{}.new", env.binary)),
            "second tick built anyway: {lines:?}"
        );
        assert!(!file_exists(&format!("{}.prev", env.binary)));
        assert!(installed_binary_is(&env.binary, &rev));
        assert!(!installed_binary_is(&env.binary, "0000000000000000"));
        assert!(!installed_binary_is("", &rev));
        assert!(!installed_binary_is("/nonexistent/pulse", &rev));
    }

    #[test]
    fn self_update_waits_out_every_other_state_and_says_so_once() {
        let _serial = serial();
        let _guard = pulse_testutil::env_lock();
        reset_self_update();
        let (s, repo, env, _fixture) = self_update_fixture();
        let rev = head_of(&repo);
        let mut same = env.clone();
        same.revision = rev;
        let lines = tend_binary(&s, TEST_NOW.into(), same);
        assert!(lines.is_empty(), "same revision: {lines:?}");
        std::fs::write(std::path::Path::new(&repo).join("scratch.txt"), b"x").unwrap();
        let lines = tend_binary(&s, TEST_NOW.into(), env.clone());
        assert!(
            lines.len() == 1 && lines[0].contains("brain checkout is dirty"),
            "dirty: {lines:?}"
        );
        let lines = tend_binary(&s, TEST_NOW.into(), env.clone());
        assert!(lines.is_empty(), "dirty again: {lines:?}");
        std::fs::remove_file(std::path::Path::new(&repo).join("scratch.txt")).unwrap();
        run(&repo, &["git", "checkout", "-q", "-b", "feature"]);
        let lines = tend_binary(&s, TEST_NOW.into(), env.clone());
        assert!(
            lines.len() == 1
                && lines[0].contains("brain checkout is on feature, not main; waiting"),
            "branch: {lines:?}"
        );
        run(&repo, &["git", "checkout", "-q", "main"]);
        set_setting(&s, "stop", "1");
        let lines = tend_binary(&s, TEST_NOW.into(), env.clone());
        assert!(
            lines.len() == 1 && lines[0].contains("stop is set; no build"),
            "stop: {lines:?}"
        );
        set_setting(&s, "stop", "0");
        set_setting(&s, "daemon.self_update", "off");
        let lines = tend_binary(&s, TEST_NOW.into(), env.clone());
        assert!(lines.is_empty(), "off: {lines:?}");
        set_setting(&s, "daemon.self_update", "on");
        let mut no_cargo = env.clone();
        no_cargo.cargo_cmd = "cargo-that-does-not-exist".into();
        let lines = tend_binary(&s, TEST_NOW.into(), no_cargo);
        assert!(
            lines.len() == 1
                && lines[0].contains("cargo-that-does-not-exist is not on the daemon's PATH"),
            "no go: {lines:?}"
        );
        let gone = std::path::Path::new(&s.home).join("gone");
        exec1(
            &s,
            "UPDATE projects SET path = ? WHERE slug = 'proj'",
            &[&gone.display().to_string()],
        );
        let lines = tend_binary(&s, TEST_NOW.into(), env.clone());
        assert!(
            lines.len() == 1
                && lines[0].contains("is not a readable git checkout: ")
                && !lines[0].trim().ends_with(':'),
            "unreadable checkout: {lines:?}"
        );
        let got = git_reason(&[crate::util::GitResult {
            code: -1,
            stdout: String::new(),
            stderr: String::new(),
            err: None,
        }]);
        assert!(
            got.contains("killed before it answered"),
            "a killed git needs a reason: {got:?}"
        );
        let got = git_reason(&[crate::util::GitResult {
            code: 128,
            stdout: String::new(),
            stderr: String::new(),
            err: None,
        }]);
        assert_eq!(
            got, "git exited 128 without a message",
            "a silent exit needs a reason: {got:?}"
        );
        exec1(
            &s,
            "UPDATE projects SET path = ? WHERE slug = 'proj'",
            &[&repo],
        );
        assert!(
            !file_exists(&format!("{}.new", env.binary))
                && !file_exists(&format!("{}.prev", env.binary)),
            "something was built while waiting"
        );
        let data = std::fs::read(&env.binary).unwrap_or_default();
        assert_eq!(
            String::from_utf8_lossy(&data),
            "old binary",
            "pulse was touched while waiting"
        );
    }

    #[test]
    fn self_update_waits_for_room_on_the_machine_and_says_so_once() {
        let _serial = serial();
        let _guard = pulse_testutil::env_lock();
        reset_self_update();
        let (s, repo, env, _fixture) = self_update_fixture();
        let rev = head_of(&repo);
        // A cap of one with one local run in flight: the build waits and the
        // dispatcher sees the update pending, so it keeps a slot free.
        let mut full = env.clone();
        full.local_inflight = 1;
        full.governor = Some(crate::daemon_load::Limits {
            max_jobs: 1,
            max_load: f64::MAX,
            min_free_mem_mb: 0,
            own_nice: 0,
            build_jobs: 6,
            cpus: 12,
            source: "test",
        });
        let lines = tend_binary(&s, TEST_NOW.into(), full.clone());
        assert!(
            lines.len() == 1
                && lines[0].contains(&format!(
                    "self-update: brain HEAD {} differs from the running 000000000000; building once the machine has room: 1 local run(s) in flight fill this machine's 1 slot(s)",
                    &rev[..12]
                )),
            "full: {lines:?}"
        );
        assert!(update_pending(), "a build waiting for room is pending");
        let lines = tend_binary(&s, TEST_NOW.into(), full.clone());
        assert!(lines.is_empty(), "said twice: {lines:?}");
        assert!(
            !file_exists(&format!("{}.new", env.binary)),
            "something was built while the machine was full"
        );
        // The run finished: the slot the dispatcher kept free admits the build.
        full.local_inflight = 0;
        let lines = tend_binary(&s, TEST_NOW.into(), full.clone());
        assert!(
            lines.len() == 1 && lines[0].ends_with("; building"),
            "room: {lines:?}"
        );
        wait_self_update_idle();
        let lines = tend_binary(&s, TEST_NOW.into(), full);
        assert!(
            lines.len() == 1 && lines[0].contains(&format!("pulse rebuilt from {}", &rev[..12])),
            "built once the machine had room: {lines:?}"
        );
        assert!(!update_pending(), "nothing pending after the install");
    }

    #[test]
    fn self_update_waits_for_head_to_settle_so_a_burst_of_landings_builds_once() {
        let _serial = serial();
        let _guard = pulse_testutil::env_lock();
        reset_self_update();
        let (s, repo, env, _fixture) = self_update_fixture();
        set_setting(&s, "daemon.self_update_settle_minutes", "15");
        let first = head_of(&repo);
        let at = |minutes: f64| crate::util::plus_minutes(TEST_NOW, minutes);
        let lines = tend_binary(&s, at(0.0), env.clone());
        assert!(
            lines.len() == 1
                && lines[0].contains(&format!(
                    "brain HEAD {} differs from the running 000000000000; building once it has stood for 15 min",
                    &first[..12]
                )),
            "a new HEAD waits: {lines:?}"
        );
        assert!(
            !update_pending(),
            "settling reserves no slot and holds no lane"
        );
        let lines = tend_binary(&s, at(5.0), env.clone());
        assert!(lines.is_empty(), "the wait is said once: {lines:?}");
        // Another landing ten minutes in: the clock restarts for the new HEAD.
        run(
            &repo,
            &[
                "git",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "second landing",
            ],
        );
        let second = head_of(&repo);
        assert_ne!(second, first);
        let lines = tend_binary(&s, at(10.0), env.clone());
        assert!(
            lines.len() == 1 && lines[0].contains(&second[..12]),
            "the new HEAD is named: {lines:?}"
        );
        let lines = tend_binary(&s, at(20.0), env.clone());
        assert!(
            lines.is_empty(),
            "fifteen minutes count from the second HEAD, not the first: {lines:?}"
        );
        let lines = tend_binary(&s, at(25.0), env.clone());
        assert!(
            lines.len() == 1
                && lines[0]
                    == format!(
                        "self-update: brain HEAD {} differs from the running 000000000000; building",
                        &second[..12]
                    ),
            "settled: {lines:?}"
        );
        wait_self_update_idle();
        let lines = tend_binary(&s, at(26.0), env);
        assert!(
            lines.len() == 1 && lines[0].contains(&format!("pulse rebuilt from {}", &second[..12])),
            "one build for two landings: {lines:?}"
        );
    }

    #[test]
    fn self_update_lets_queued_work_the_restart_would_refuse_start_first() {
        let _serial = serial();
        let _guard = pulse_testutil::env_lock();
        reset_self_update();
        let (s, repo, env, _fixture) = self_update_fixture();
        let rev = head_of(&repo);
        let at = |minutes: f64| crate::util::plus_minutes(TEST_NOW, minutes);
        let mut waiting = env.clone();
        waiting.refused_queued = "improve runs on proj".into();
        let lines = tend_binary(&s, at(0.0), waiting.clone());
        assert!(
            lines.len() == 1
                && lines[0].contains(&format!(
                    "brain HEAD {} differs from the running 000000000000; building once the queued improve runs on proj have started",
                    &rev[..12]
                )),
            "queued work first: {lines:?}"
        );
        assert!(
            !update_pending(),
            "the wait keeps no slot free: the work needs it"
        );
        let lines = tend_binary(&s, at(5.0), waiting.clone());
        assert!(lines.is_empty(), "said once: {lines:?}");
        assert!(
            !file_exists(&format!("{}.new", env.binary)),
            "something was built while the work waited"
        );
        // An hour on, the queue is stuck on something else: the build goes ahead.
        let lines = tend_binary(&s, at(REFUSED_WORK_WAIT_MINUTES), waiting);
        assert!(
            lines.len() == 1 && lines[0].ends_with("; building"),
            "an hour is the most it waits: {lines:?}"
        );
        wait_self_update_idle();
        let lines = tend_binary(&s, at(61.0), env);
        assert!(
            lines.len() == 1 && lines[0].contains(&format!("pulse rebuilt from {}", &rev[..12])),
            "built after the wait: {lines:?}"
        );
    }

    #[test]
    fn self_update_failure_leaves_pulse_alone_and_retries_later() {
        let _serial = serial();
        let _guard = pulse_testutil::env_lock();
        reset_self_update();
        let (s, repo, env, _fixture) = self_update_fixture();
        let rev = head_of(&repo);
        let _fail = pulse_testutil::setenv("FAKE_CARGO_FAIL", "1");
        let lines = tend_binary(&s, TEST_NOW.into(), env.clone());
        assert!(
            lines.len() == 1 && lines[0].contains("building"),
            "start: {lines:?}"
        );
        wait_self_update_idle();
        let lines = tend_binary(&s, TEST_NOW.into(), env.clone());
        assert!(
            lines.len() == 1
                && lines[0].contains(&format!("self-update: build of {} failed", &rev[..12])),
            "after the failed build: {lines:?}"
        );
        let data = std::fs::read(&env.binary).unwrap_or_default();
        assert!(
            String::from_utf8_lossy(&data) == "old binary"
                && !file_exists(&format!("{}.prev", env.binary)),
            "pulse was touched by a failed build"
        );
        let post = query_one(
            &*s.conn().unwrap(),
            "SELECT * FROM posts WHERE kind = 'finding' AND thread = ? ORDER BY id DESC LIMIT 1",
            &[&SELF_UPDATE_THREAD],
        );
        assert!(
            matches!(&post, Ok(Some(post)) if post.str("summary").contains(&format!(
                    "build of {} failed; the running binary is untouched",
                    &rev[..12]
                ))
                && post.str("body").contains("cannot find value `nope`")),
            "failure finding: {post:?}"
        );
        drop(_fail);
        let lines = tend_binary(&s, crate::util::plus_minutes(TEST_NOW, 30.0), env.clone());
        assert!(lines.is_empty(), "retried early: {lines:?}");
        set_setting(&s, "daemon.self_update_retry_minutes", "15");
        let lines = tend_binary(&s, crate::util::plus_minutes(TEST_NOW, 30.0), env.clone());
        assert!(
            lines.len() == 1 && lines[0].contains("building"),
            "retry after the window: {lines:?}"
        );
        wait_self_update_idle();
        let lines = tend_binary(&s, crate::util::plus_minutes(TEST_NOW, 31.0), env.clone());
        assert!(
            lines.len() == 1 && lines[0].contains(&format!("pulse rebuilt from {}", &rev[..12])),
            "retry outcome: {lines:?}"
        );
        reset_self_update();
        run(
            &repo,
            &["git", "commit", "-q", "--allow-empty", "-m", "next"],
        );
        let next = head_of(&repo);
        let _bad = pulse_testutil::setenv("FAKE_CARGO_BADVERSION", "1");
        let _ = tend_binary(&s, TEST_NOW.into(), env.clone());
        wait_self_update_idle();
        let lines = tend_binary(&s, TEST_NOW.into(), env.clone());
        assert!(
            lines.len() == 1
                && lines[0].contains(&format!("smoke of {} failed", &next[..12]))
                && lines[0].contains(&format!("not the revision {}", &next[..12])),
            "smoke: {lines:?}"
        );
        assert!(
            file_exists(&format!("{}.new", env.binary)),
            "pulse.new was not kept after the smoke failure"
        );
        let data = std::fs::read(&env.binary).unwrap_or_default();
        assert!(
            String::from_utf8_lossy(&data).contains(&format!("dev-{}", &rev[..12])),
            "pulse changed after a failed smoke"
        );
    }

    #[test]
    fn check_restored_turns_self_update_off_and_names_the_build() {
        let _serial = serial();
        let _guard = pulse_testutil::env_lock();
        reset_self_update();
        let (s, _, env, _fixture) = self_update_fixture();
        let lines = check_restored(&s, env.clone());
        assert!(lines.is_empty(), "no marker: {lines:?}");
        exec1(
            &s,
            "INSERT INTO events (at, actor, action, subject, detail) VALUES (?, 'daemon', ?, 'pulse', ?)",
            &[
                &TEST_NOW,
                &SELF_UPDATE_EVENT,
                &r#"{"from":"aaa","to":"deadbeefcafe0000"}"#,
            ],
        );
        std::fs::write(
            format!("{}{}", env.binary, RESTORED_MARKER),
            b"2026-09-08T10:00:00Z\n",
        )
        .unwrap();
        let lines = check_restored(&s, env.clone());
        assert!(
            lines.len() == 1
                && lines[0].contains(
                    "restored the previous pulse after 3 early exits: the build of deadbeefcafe did not start; self-update is off"
                ),
            "restored: {lines:?}"
        );
        assert_eq!(
            s.setting("daemon.self_update", ""),
            "off",
            "self_update = {:?} after a restore",
            s.setting("daemon.self_update", "")
        );
        assert!(
            !file_exists(&format!("{}{}", env.binary, RESTORED_MARKER)),
            "marker kept"
        );
        let post = query_one(
            &*s.conn().unwrap(),
            "SELECT * FROM posts WHERE kind = 'finding' AND thread = ? ORDER BY id DESC LIMIT 1",
            &[&SELF_UPDATE_THREAD],
        );
        assert!(
            matches!(&post, Ok(Some(post)) if post.str("body").contains("2026-09-08T10:00:00Z")
                && post
                    .str("body")
                    .contains("pulse settings set daemon.self_update on")),
            "restore finding: {post:?}"
        );
        let lines = tend_binary(&s, TEST_NOW.into(), env);
        assert!(lines.is_empty(), "after the restore: {lines:?}");
    }

    #[test]
    fn self_update_skips_a_landing_that_touches_nothing_the_binary_is_built_from() {
        let _serial = serial();
        let _guard = pulse_testutil::env_lock();
        reset_self_update();
        let (s, repo, mut env, _fixture) = self_update_fixture();
        let base = head_of(&repo);
        env.revision = base.clone();
        let at = |minutes: f64| crate::util::plus_minutes(TEST_NOW, minutes);
        // A change under crates/ taken back: the trees are identical, so the
        // landing is neutral even though HEAD moved twice.
        let code = land(&repo, "crates/x/src/lib.rs", "fn x() {}\n", "code");
        run(&repo, &["git", "rm", "-q", "crates/x/src/lib.rs"]);
        run(&repo, &["git", "commit", "-q", "-m", "take it back"]);
        let reverted = head_of(&repo);
        assert!(reverted != code && reverted != base);
        let lines = tend_binary(&s, at(0.0), env.clone());
        assert_eq!(
            lines,
            vec![format!(
                "self-update: brain HEAD {} differs from the running {} only outside what the binary is built from (the trees are identical); no build",
                &reverted[..12],
                &base[..12]
            )],
            "a revert: {lines:?}"
        );
        assert!(!update_pending(), "a neutral landing holds nothing");
        let lines = tend_binary(&s, at(1.0), env.clone());
        assert!(
            lines.is_empty(),
            "the neutral HEAD is judged once: {lines:?}"
        );
        // Docs only: named, counted, not built.
        let docs = land(&repo, "docs/runner.md", "# runner\n", "docs");
        std::fs::write(
            std::path::Path::new(&repo).join("README.md"),
            "# proj, again\n",
        )
        .unwrap();
        run(&repo, &["git", "commit", "-q", "-am", "readme"]);
        let readme = head_of(&repo);
        assert_ne!(readme, docs);
        let lines = tend_binary(&s, at(2.0), env.clone());
        assert_eq!(
            lines,
            vec![format!(
                "self-update: brain HEAD {} differs from the running {} only outside what the binary is built from (2 file(s): README.md, docs/runner.md); no build",
                &readme[..12],
                &base[..12]
            )],
            "docs and README: {lines:?}"
        );
        let lines = tend_binary(&s, at(3.0), env.clone());
        assert!(lines.is_empty(), "said twice: {lines:?}");
        // More than three paths are counted, not listed.
        land(&repo, "scripts/verify-check.sh", "#!/bin/sh\n", "script");
        land(&repo, "apps/macos/Info.plist", "<plist/>\n", "app");
        let five = land(
            &repo,
            "web/hub/mock/api/status.json",
            "{}\n",
            "console mock fixture",
        );
        let lines = tend_binary(&s, at(4.0), env.clone());
        assert!(
            lines.len() == 1
                && lines[0].contains(&format!("brain HEAD {} differs", &five[..12]))
                && lines[0].contains("(5 file(s): README.md, apps/macos/Info.plist, docs/runner.md and 2 more); no build"),
            "five paths: {lines:?}"
        );
        assert!(
            !file_exists(&format!("{}.new", env.binary))
                && !file_exists(&format!("{}.prev", env.binary)),
            "something was built for a neutral landing"
        );
        // The console's source is built into the binary (pulse-hub's build
        // script exports it and rust_embed embeds the export): it builds,
        // diffed against the running revision, so the docs come along.
        let out = land(
            &repo,
            "web/hub/src/app/page.tsx",
            "export default function Page() {}\n",
            "console source",
        );
        let lines = tend_binary(&s, at(5.0), env.clone());
        assert_eq!(
            lines,
            vec![format!(
                "self-update: brain HEAD {} differs from the running {}; building",
                &out[..12],
                &base[..12]
            )],
            "an input: {lines:?}"
        );
        wait_self_update_idle();
        let lines = tend_binary(&s, at(6.0), env.clone());
        assert!(
            lines.len() == 1 && lines[0].contains(&format!("pulse rebuilt from {}", &out[..12])),
            "built: {lines:?}"
        );
        // A docs landing on top of the build that waits for its restart is
        // measured from that installed revision, not the running one: no
        // build.
        let on_top = land(&repo, "docs/on-top.md", "# on top\n", "docs on top");
        let lines = tend_binary(&s, at(6.5), env.clone());
        assert!(
            lines.len() == 1
                && lines[0].contains(&format!(
                    "brain HEAD {} differs from the installed {} only outside",
                    &on_top[..12],
                    &out[..12]
                )),
            "docs on top of the installed build: {lines:?}"
        );
        // An unstamped running binary with nothing installed cannot be
        // diffed against: a docs landing builds, as before the diff.
        crate::test_helpers_selfupdate::reset_self_update();
        let unstamped = land(&repo, "docs/more.md", "# more\n", "more docs");
        env.revision.clear();
        let lines = tend_binary(&s, at(7.0), env.clone());
        assert_eq!(
            lines,
            vec![format!(
                "self-update: brain HEAD {} differs from the running unstamped; building",
                &unstamped[..12]
            )],
            "unstamped: {lines:?}"
        );
        wait_self_update_idle();
        let lines = tend_binary(&s, at(8.0), env);
        assert!(
            lines.len() == 1
                && lines[0].contains(&format!("pulse rebuilt from {}", &unstamped[..12])),
            "built unstamped: {lines:?}"
        );
    }

    #[test]
    fn the_gate_ladder_decides_without_a_machine() {
        let env = SelfUpdateEnv {
            binary: "/nonexistent/pulse".into(),
            cargo_cmd: "cargo".into(),
            revision: "a".repeat(40),
            ..Default::default()
        };
        let bench = Bench {
            cargo_on_path: true,
            ..Default::default()
        };
        let co = Checkout {
            repo: "/nonexistent".into(),
            rev: "b".repeat(40),
            branch: "main".into(),
            default_branch: "main".into(),
            dirty: false,
            changed: Some(vec!["README.md".into(), "scripts/verify-check.sh".into()]),
            base: "a".repeat(40),
        };
        let mut st = SelfUpdateState::default();
        // Neutral: remembered, said, and quiet from then on.
        let got = gate(&mut st, 15, TEST_NOW, &env, &co, &bench);
        assert_eq!(
            got,
            Gate::Wait(format!(
                "brain HEAD {} differs from the running {} only outside what the binary is built from (2 file(s): README.md, scripts/verify-check.sh); no build",
                "b".repeat(12),
                "a".repeat(12)
            ))
        );
        assert_eq!(st.neutral_rev, co.rev);
        assert!(
            st.settle_rev.is_empty(),
            "a neutral HEAD starts no settle clock"
        );
        assert_eq!(gate(&mut st, 15, TEST_NOW, &env, &co, &bench), Gate::Quiet);
        // Already installed by hand comes before the diff.
        let mut co = Checkout {
            rev: "c".repeat(40),
            ..co
        };
        let by_hand = Bench {
            installed_by_hand: true,
            ..bench.clone()
        };
        let got = gate(&mut st, 15, TEST_NOW, &env, &co, &by_hand);
        assert_eq!(
            got,
            Gate::Wait(format!(
                "{} is already installed at /nonexistent/pulse; restarting onto it after the runs in flight",
                "c".repeat(12)
            ))
        );
        assert_eq!(st.installed, co.rev);
        assert_eq!(gate(&mut st, 15, TEST_NOW, &env, &co, &bench), Gate::Quiet);
        // A landing on top of the installed one that touches no input is
        // neutral against the installed revision, not the running one: no
        // settle clock, no build, and the line names what it was measured
        // from.
        let installed = co.rev.clone();
        co.rev = "f".repeat(40);
        co.base = installed.clone();
        co.changed = Some(vec!["scripts/verify-check.sh".into()]);
        assert_eq!(
            gate(&mut st, 15, TEST_NOW, &env, &co, &bench),
            Gate::Wait(format!(
                "brain HEAD {} differs from the installed {} only outside what the binary is built from (1 file(s): scripts/verify-check.sh); no build",
                "f".repeat(12),
                "c".repeat(12)
            ))
        );
        assert!(
            st.settle_rev.is_empty(),
            "no settle clock for a neutral landing"
        );
        co.base = env.revision.clone();
        // An input among the paths: the settle clock runs from the first
        // sight of this HEAD, and the build goes once it has stood.
        co.rev = "d".repeat(40);
        co.changed = Some(vec!["docs/runner.md".into(), "Cargo.lock".into()]);
        let at = |minutes: f64| crate::util::plus_minutes(TEST_NOW, minutes);
        let settling = Gate::Wait(format!(
            "brain HEAD {} differs from the running {}; building once it has stood for 15 min (daemon.self_update_settle_minutes), so a burst of landings builds once",
            "d".repeat(12),
            "a".repeat(12)
        ));
        assert_eq!(gate(&mut st, 15, &at(0.0), &env, &co, &bench), settling);
        assert_eq!(st.settle_rev, co.rev);
        assert_eq!(st.settle_since, at(0.0));
        assert_eq!(gate(&mut st, 15, &at(14.0), &env, &co, &bench), settling);
        assert_eq!(gate(&mut st, 15, &at(15.0), &env, &co, &bench), Gate::Build);
        // Unknown to git (an unstamped build, rewritten history): not
        // neutral, so the ladder goes on to the checkout's state.
        co.rev = "e".repeat(40);
        co.changed = None;
        co.dirty = true;
        assert_eq!(
            gate(&mut st, 0, TEST_NOW, &env, &co, &bench),
            Gate::Wait("brain checkout is dirty; waiting for it to be clean".into())
        );
        co.dirty = false;
        assert_eq!(gate(&mut st, 0, TEST_NOW, &env, &co, &bench), Gate::Build);
        // The ladder's tail, in order: cargo, the disk, the lane, the room.
        let no_cargo = Bench {
            cargo_on_path: false,
            bulk_missing: true,
            ..bench.clone()
        };
        assert_eq!(
            gate(&mut st, 0, TEST_NOW, &env, &co, &no_cargo),
            Gate::Wait("cargo is not on the daemon's PATH; builds are yours until it is".into())
        );
        let full = Bench {
            hold: Some("load 9.0 over daemon.max_load 8.0".into()),
            ..bench.clone()
        };
        assert_eq!(
            gate(&mut st, 0, TEST_NOW, &env, &co, &full),
            Gate::Wait(format!(
                "brain HEAD {} differs from the running {}; building once the machine has room: load 9.0 over daemon.max_load 8.0",
                "e".repeat(12),
                "a".repeat(12)
            ))
        );
        assert!(st.build_wait, "a build held for room is pending");
        for (path, input) in [
            ("crates/pulse-hub/src/server.rs", true),
            ("Cargo.toml", true),
            ("Cargo.lock", true),
            ("rust-toolchain.toml", true),
            (".cargo/config.toml", true),
            ("web/hub/src/app/page.tsx", true),
            ("web/hub/public/fonts/a.woff2", true),
            ("web/hub/package.json", true),
            ("web/hub/package-lock.json", true),
            ("web/hub/next.config.ts", true),
            ("web/hub/out/index.html", false),
            ("web/hub/mock/api/status.json", false),
            ("web/hub/src/lib/api.test.mjs", true),
            ("README.md", false),
            ("docs/runner.md", false),
            ("scripts/verify-check.sh", false),
            ("apps/macos/Info.plist", false),
        ] {
            assert_eq!(is_binary_input(path), input, "{path}");
        }
    }

    #[test]
    fn run_in_bounds_a_build_by_its_silence_and_a_smoke_by_its_length() {
        // Read when the test runs: a test binary served from a shared target
        // dir carries the compiling checkout's path, which may be gone.
        let here = pulse_testutil::manifest_dir(env!("CARGO_MANIFEST_DIR"))
            .to_string_lossy()
            .into_owned();
        let sh = |script: &str| vec!["-c".to_string(), script.to_string()];
        // A child that keeps printing is working, however long it takes:
        // eight lines a tenth of a second apart under a bound ten times the
        // gap (the ratio is generous for a loaded test host).
        let (out, err) = run_in(
            here.clone(),
            Bound::Quiet(test_wait(Duration::from_millis(1000))),
            "sh".into(),
            &sh("for i in 1 2 3 4 5 6 7 8; do echo tick $i; sleep 0.1; done"),
        );
        assert!(err.is_none(), "a chatty child was cut: {err:?}\n{out}");
        assert_eq!(out.lines().count(), 8, "{out}");
        // A silent one is killed once it has said nothing for the bound,
        // long before it would have ended.
        let started = Instant::now();
        let (out, err) = run_in(
            here.clone(),
            Bound::Quiet(Duration::from_millis(300)),
            "sh".into(),
            &sh("sleep 2"),
        );
        let took = started.elapsed();
        let why = err.map(|e| e.to_string()).unwrap_or_default();
        assert_eq!(why, "sh produced no output for 300ms", "{out}");
        assert!(
            took < test_wait(Duration::from_millis(1500)),
            "the silent child ran {took:?} against a 300 ms silence bound"
        );
        // The smoke bound is on length: a child that goes on printing is
        // cut all the same, and the error says which bound it passed.
        let started = Instant::now();
        let (_, err) = run_in(
            here,
            Bound::Total(Duration::from_millis(300)),
            "sh".into(),
            &sh("while :; do echo still here; sleep 0.05; done"),
        );
        let took = started.elapsed();
        let why = err.map(|e| e.to_string()).unwrap_or_default();
        assert_eq!(why, "sh timed out after 300ms");
        assert!(
            took < test_wait(Duration::from_millis(1500)),
            "the endless child ran {took:?} against a 300 ms length bound"
        );
    }

    /// Every file a crate embeds into the binary must lie under a prefix in
    /// BINARY_INPUTS, or a landing that changes it would not be rebuilt. The
    /// console's export is the exception: pulse-hub's build script writes
    /// it from the web/hub inputs, so those must be in the list instead.
    #[test]
    fn every_embedded_path_is_a_binary_input() {
        use std::path::{Component, Path, PathBuf};
        const HUB_EXPORT: &str = "web/hub/out/";
        let root = fold(&pulse_testutil::manifest_dir(env!("CARGO_MANIFEST_DIR")).join("../.."));
        let mut files = Vec::new();
        sources(&root.join("crates"), &mut files);
        assert!(
            files.len() > 10,
            "found {} sources under {}",
            files.len(),
            root.display()
        );
        // The needles are assembled here so this file's own strings are not
        // read as embeds.
        let includes = ["include_str", "include_bytes"].map(|name| format!("{name}!("));
        let folder = format!("#[{} = ", "folder");
        let mut hits: Vec<(String, String)> = Vec::new();
        for file in &files {
            let text = std::fs::read_to_string(file).unwrap_or_default();
            let at = |idx: usize| {
                format!(
                    "{}:{}",
                    file.strip_prefix(&root).unwrap_or(file).display(),
                    text[..idx].lines().count().max(1)
                )
            };
            for needle in &includes {
                for (idx, _) in text.match_indices(needle.as_str()) {
                    // concat!(env!("OUT_DIR"), ...) is a generated file, not a path in the tree.
                    let Some(path) = quoted(&text[idx + needle.len()..]) else {
                        continue;
                    };
                    let embedded = file.parent().unwrap().join(path);
                    hits.push((relative(&root, &embedded, false), at(idx)));
                }
            }
            for (idx, _) in text.match_indices(folder.as_str()) {
                let Some(path) = quoted(&text[idx + folder.len()..]) else {
                    continue;
                };
                if path.starts_with('$') || Path::new(path).is_absolute() {
                    continue;
                }
                let manifest_dir = file
                    .ancestors()
                    .find(|dir| dir.join("Cargo.toml").is_file())
                    .expect("a crate's manifest dir");
                hits.push((relative(&root, &manifest_dir.join(path), true), at(idx)));
            }
        }
        let paths: Vec<&str> = hits.iter().map(|(path, _)| path.as_str()).collect();
        assert!(
            paths.contains(&"crates/pulse-records/seed/bundle.json") && paths.contains(&HUB_EXPORT),
            "the scan missed the embeds it is known to find: {paths:?}"
        );
        for (path, at) in &hits {
            // A build product, not an input: the web/hub paths the list
            // names must be the ones the script builds it from (its INPUTS
            // list), no more and no fewer.
            if path == HUB_EXPORT {
                let script = std::fs::read_to_string(root.join("crates/pulse-hub/build.rs"))
                    .expect("read crates/pulse-hub/build.rs");
                let list = script
                    .split_once("const INPUTS: &[&str] = &[")
                    .and_then(|(_, rest)| rest.split_once("];"))
                    .map(|(list, _)| list)
                    .expect("crates/pulse-hub/build.rs declares INPUTS");
                let mut script_inputs: Vec<&str> = list.split('"').skip(1).step_by(2).collect();
                script_inputs.sort_unstable();
                let mut listed: Vec<&str> = BINARY_INPUTS
                    .iter()
                    .filter_map(|input| input.strip_prefix("web/hub/"))
                    .map(|input| input.trim_end_matches('/'))
                    .collect();
                listed.sort_unstable();
                assert_eq!(
                    listed, script_inputs,
                    "the web/hub paths in BINARY_INPUTS and the INPUTS crates/pulse-hub/build.rs builds the export from differ: keep the two lists the same"
                );
                continue;
            }
            assert!(
                is_binary_input(path),
                "{at} embeds {path}, which no prefix in BINARY_INPUTS covers: add its prefix to BINARY_INPUTS in selfupdate.rs so a landing that changes it is rebuilt"
            );
        }

        /// Every `*.rs` and `Cargo.toml` under `dir`, build dirs aside.
        fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                if path.is_dir() {
                    if name != "target" && name != "node_modules" {
                        sources(&path, out);
                    }
                } else if name.ends_with(".rs") || name == "Cargo.toml" {
                    out.push(path);
                }
            }
        }

        /// The string literal `text` opens with, after any whitespace.
        fn quoted(text: &str) -> Option<&str> {
            let rest = text.trim_start().strip_prefix('"')?;
            Some(&rest[..rest.find('"')?])
        }

        /// `path` with its `.` and `..` folded, lexically: the embedded
        /// path need not exist on this machine for the list to be checked.
        fn fold(path: &Path) -> PathBuf {
            let mut out = PathBuf::new();
            for part in path.components() {
                match part {
                    Component::ParentDir => {
                        out.pop();
                    }
                    Component::CurDir => {}
                    other => out.push(other.as_os_str()),
                }
            }
            out
        }

        /// `path` from the workspace root with `/` separators; a folder
        /// ends in `/`, as the prefixes of BINARY_INPUTS do.
        fn relative(root: &Path, path: &Path, is_dir: bool) -> String {
            let folded = fold(path);
            let rel = folded
                .strip_prefix(root)
                .unwrap_or(&folded)
                .to_string_lossy()
                .replace('\\', "/");
            if is_dir { format!("{rel}/") } else { rel }
        }
    }
}
