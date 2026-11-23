//! `hive update [--check]` (TASK-090): installs the newer published
//! release that `update.latest_url` names. The release document's `assets`
//! give the archive for the running target
//! (`hive-vX.Y.Z-<target>.tar.gz`) and `SHA256SUMS`; both are downloaded,
//! the archive must match its line in `SHA256SUMS`, and the `hive` inside it
//! goes the way the daemon's own build goes (selfupdate.rs): written beside
//! the running binary as `<binary>.new`, signed (sign.rs), smoke-tested with
//! `version --json` (the version it prints must be the release's) and
//! `help`, the running binary copied aside as `<binary>.prev`, and the new
//! file renamed into place. The daemon sees its binary change on disk and
//! restarts on it after its runs in flight (restart.rs); nothing here stops
//! a run.
//!
//! While `update.latest_url` is empty, the command the owner runs reads the
//! project's own releases for that one run and says so first. The setting
//! stays empty, so the daily check, the daemon's self-update and the
//! console still do not look anywhere. Under an agent's name or inside a
//! job the command refuses, as it always did, and reads nothing.
//!
//! A program a package manager installed (Homebrew, for one) is that
//! manager's to replace: it leaves a marker beside the program
//! (`hive_cli::managed`), and with one there nothing here writes the file.
//! `hive update` and the console's Update now refuse and name the manager's
//! command, [`install`] itself refuses whoever calls it, and `hive update
//! --check` still says whether a release is out.
//!
//! A private repository answers 404 to an anonymous request, so a GitHub
//! address that fails over HTTP is read again through `gh api` when gh is
//! on PATH (the account the owner is signed in with); the daily check
//! (update_check.rs) reads the document the same way.
//!
//! What is pure - asset choice, checksums, the channel and the
//! once-per-release prompt - lives in `hive_cli::update` with its tests.

use anyhow::{Result, anyhow, bail};
use hive_cli::update::{
    Asset, CHECK_HOURS, CHECKSUMS_ASSET, Channel, LATEST_URL_SETTING, META_KEY, Release,
    UpdateState, check_digest, parse_checksums, parse_release, pick_asset, resolve_channel,
};
use serde_json::Value;
use sha2::Digest;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The event an installed release writes.
pub(crate) const UPDATE_EVENT: &str = "hive.update";
const DOC_TIMEOUT: Duration = Duration::from_secs(20);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const SMOKE_TIMEOUT: Duration = Duration::from_secs(60);
const USER_AGENT: &str = "hive-update";
const GH_API: &str = "https://api.github.com/";

/// Reads the JSON document at `url`: over HTTP(S), and for an
/// api.github.com address that answers an error, through `gh api` when gh
/// is on PATH (a private repository).
pub(crate) fn fetch_json(url: &str) -> Result<Value, String> {
    let http = http_get(url, "application/json", DOC_TIMEOUT).and_then(|bytes| {
        serde_json::from_slice::<Value>(&bytes).map_err(|e| format!("{url}: {e}"))
    });
    match http {
        Ok(v) => Ok(v),
        Err(err) => match url.strip_prefix(GH_API) {
            Some(path) if gh_on_path() => gh_api(path, "application/json", DOC_TIMEOUT)
                .and_then(|bytes| {
                    serde_json::from_slice::<Value>(&bytes)
                        .map_err(|e| format!("gh api {path}: {e}"))
                })
                .map_err(|gh| format!("{err}; {gh}")),
            _ => Err(err),
        },
    }
}

/// Downloads one release file: its download address over HTTP(S), else its
/// GitHub API address through `gh api` (a private repository's assets).
pub(crate) fn fetch_asset(asset: &Asset) -> Result<Vec<u8>, String> {
    match http_get(&asset.url, "application/octet-stream", DOWNLOAD_TIMEOUT) {
        Ok(bytes) => Ok(bytes),
        Err(err) => match asset.api_url.strip_prefix(GH_API) {
            Some(path) if gh_on_path() => {
                gh_api(path, "application/octet-stream", DOWNLOAD_TIMEOUT)
                    .map_err(|gh| format!("{err}; {gh}"))
            }
            _ => Err(err),
        },
    }
}

fn http_get(url: &str, accept: &str, timeout: Duration) -> Result<Vec<u8>, String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(timeout)
        .build()
        .map_err(|e| format!("no HTTP client: {e}"))?;
    let resp = client
        .get(url)
        .header("accept", accept)
        .send()
        .map_err(|e| format!("{url}: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("{url}: HTTP {}", status.as_u16()));
    }
    resp.bytes()
        .map(|b| b.to_vec())
        .map_err(|e| format!("{url}: {e}"))
}

fn gh_on_path() -> bool {
    which::which("gh").is_ok()
}

/// `gh api -H 'Accept: <accept>' <path>`, its stdout written to a file so a
/// large asset never fills a pipe; killed after `timeout`. The error carries
/// gh's own words, never a token (gh does not print one).
fn gh_api(path: &str, accept: &str, timeout: Duration) -> Result<Vec<u8>, String> {
    let dir = Scratch::new("gh").map_err(|e| format!("gh api {path}: {e}"))?;
    let out_path = dir.path().join("out");
    let err_path = dir.path().join("err");
    let out = std::fs::File::create(&out_path).map_err(|e| format!("gh api {path}: {e}"))?;
    let err = std::fs::File::create(&err_path).map_err(|e| format!("gh api {path}: {e}"))?;
    let mut child = Command::new("gh")
        .args(["api", "-H", &format!("Accept: {accept}"), path])
        .env("GH_PROMPT_DISABLED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err))
        .spawn()
        .map_err(|e| format!("gh api {path}: {e}"))?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "gh api {path}: timed out after {}s",
                    timeout.as_secs()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(format!("gh api {path}: {e}")),
        }
    };
    if !status.success() {
        let words = std::fs::read_to_string(&err_path).unwrap_or_default();
        let words = words.lines().next().unwrap_or("").trim().to_string();
        return Err(format!(
            "gh api {path}: exit {}{}",
            status.code().unwrap_or(-1),
            if words.is_empty() {
                String::new()
            } else {
                format!(": {words}")
            }
        ));
    }
    std::fs::read(&out_path).map_err(|e| format!("gh api {path}: {e}"))
}

/// A folder under the system's temporary directory, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(what: &str) -> std::io::Result<Self> {
        let dir = std::env::temp_dir().join(format!(
            "hive-{what}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir)?;
        Ok(Scratch(dir))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Everything one `hive update` works with; tests build their own.
pub(crate) struct UpdateEnv {
    /// The release document's address (`update.latest_url`).
    pub url: String,
    /// The running build's version.
    pub running: String,
    /// The running build's target triple.
    pub target: String,
    /// The binary to replace.
    pub binary: String,
    pub actor: String,
    pub fetch_json: fn(&str) -> Result<Value, String>,
    pub fetch_asset: fn(&Asset) -> Result<Vec<u8>, String>,
}

/// What `hive update` found or did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The running build is the latest release (or newer).
    Current { latest: String },
    /// `--check`: a newer release exists and was not installed.
    Available { latest: String, url: String },
    /// The release was installed at `binary`; the old one is `prev`.
    Installed {
        latest: String,
        binary: String,
        prev: String,
    },
}

/// Reads the release document, keeps what it says in the `update` meta row
/// (as the daily check does) and returns the release.
pub(crate) fn read_release(s: &hive_store::Store, env: &UpdateEnv) -> Result<Release> {
    let doc = (env.fetch_json)(&env.url);
    let mut state = UpdateState {
        checked_at: s.now(),
        source: env.url.clone(),
        ..UpdateState::default()
    };
    let release = doc.and_then(|d| parse_release(&d));
    match &release {
        Ok(r) => {
            state.latest = r.version.clone();
            state.url = r.url.clone();
        }
        Err(e) => {
            if let Ok((raw, true)) = s.meta(META_KEY)
                && let Some(before) = UpdateState::from_json(&raw)
                && before.source == env.url
            {
                state.latest = before.latest;
                state.url = before.url;
            }
            state.error = e.clone();
        }
    }
    let _ = s.set_meta(META_KEY, &serde_json::to_string(&state).unwrap_or_default());
    release.map_err(|e| anyhow!("could not read {LATEST_URL_SETTING} ({}): {e}", env.url))
}

/// `hive update --check` reads the stored state when the daily check is
/// fresh and the document otherwise; `hive update` always reads it (the
/// stored state keeps no assets).
pub(crate) fn check(s: &hive_store::Store, env: &UpdateEnv) -> Result<Outcome> {
    let stored = match s.meta(META_KEY) {
        Ok((raw, true)) => UpdateState::from_json(&raw),
        _ => None,
    };
    let fresh = stored.as_ref().filter(|st| {
        st.source == env.url
            && st.error.is_empty()
            && !st.latest.is_empty()
            && crate::util::plus_hours(&st.checked_at, CHECK_HOURS) > s.now()
    });
    let (latest, url) = match fresh {
        Some(st) => (st.latest.clone(), st.url.clone()),
        None => {
            let r = read_release(s, env)?;
            (r.version, r.url)
        }
    };
    if hive_cli::update::is_newer(&latest, &env.running) {
        Ok(Outcome::Available { latest, url })
    } else {
        Ok(Outcome::Current { latest })
    }
}

/// Downloads, verifies and installs the newer release, or says the running
/// build is current.
pub(crate) fn install(s: &hive_store::Store, env: &UpdateEnv) -> Result<Outcome> {
    // Whoever asks: a package manager's file is never written here.
    if let Some(m) = hive_cli::managed::of(Path::new(&env.binary)) {
        bail!("{}", m.refusal(&env.binary));
    }
    let release = read_release(s, env)?;
    if !hive_cli::update::is_newer(&release.version, &env.running) {
        return Ok(Outcome::Current {
            latest: release.version,
        });
    }
    let archive = pick_asset(&release.assets, &release.version, &env.target)
        .map_err(|e| anyhow!(e))?
        .clone();
    let sums_asset = release
        .assets
        .iter()
        .find(|a| a.name == CHECKSUMS_ASSET)
        .ok_or_else(|| {
            anyhow!(
                "release {} carries no {CHECKSUMS_ASSET}; refusing to install an archive that cannot be checked",
                release.version
            )
        })?;
    let sums =
        (env.fetch_asset)(sums_asset).map_err(|e| anyhow!("download {CHECKSUMS_ASSET}: {e}"))?;
    let sums = parse_checksums(&String::from_utf8_lossy(&sums));
    let bytes =
        (env.fetch_asset)(&archive).map_err(|e| anyhow!("download {}: {e}", archive.name))?;
    let digest = hex::encode(sha2::Sha256::digest(&bytes));
    check_digest(&sums, &archive.name, &digest).map_err(|e| anyhow!(e))?;

    let work = Scratch::new("update")?;
    let tarball = work.path().join(&archive.name);
    std::fs::write(&tarball, &bytes)?;
    let unpacked = work.path().join("unpacked");
    std::fs::create_dir_all(&unpacked)?;
    let tar = Command::new("tar")
        .arg("-xzf")
        .arg(&tarball)
        .arg("-C")
        .arg(&unpacked)
        .output()
        .map_err(|e| anyhow!("tar: {e}"))?;
    if !tar.status.success() {
        bail!(
            "could not unpack {}: {}",
            archive.name,
            String::from_utf8_lossy(&tar.stderr).trim()
        );
    }
    let found = find_binary(&unpacked)
        .ok_or_else(|| anyhow!("{} holds no file named hive", archive.name))?;

    let new_binary = format!("{}.new", env.binary);
    let _ = std::fs::remove_file(&new_binary);
    std::fs::copy(&found, &new_binary)?;
    let cleanup = |e: anyhow::Error| {
        let _ = std::fs::remove_file(&new_binary);
        e
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&new_binary, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| cleanup(e.into()))?;
    }
    crate::sign::sign_new_build(s, &new_binary)
        .map_err(|e| cleanup(anyhow!("sign {new_binary}: {e}")))?;
    smoke(&new_binary, &release.version).map_err(cleanup)?;

    // Copied aside, never renamed: a daemon running from this path keeps
    // its name (selfupdate.rs says why).
    let prev = format!("{}.prev", env.binary);
    let _ = std::fs::remove_file(&prev);
    std::fs::copy(&env.binary, &prev).map_err(|e| cleanup(anyhow!("keep {prev}: {e}")))?;
    if let Err(e) = std::fs::rename(&new_binary, &env.binary) {
        let _ = std::fs::remove_file(&prev);
        return Err(cleanup(anyhow!("install {}: {e}", env.binary)));
    }
    let detail = serde_json::json!({
        "from": env.running,
        "to": release.version,
        "binary": env.binary,
        "asset": archive.name,
        "sha256": digest,
    });
    let _ = hive_bus::emit(s, &env.actor, UPDATE_EVENT, "hive", detail);
    Ok(Outcome::Installed {
        latest: release.version,
        binary: env.binary.clone(),
        prev,
    })
}

/// The `hive` file in an unpacked archive: at its top or one folder down.
fn find_binary(dir: &Path) -> Option<PathBuf> {
    let name = if cfg!(windows) { "hive.exe" } else { "hive" };
    let top = dir.join(name);
    if top.is_file() {
        return Some(top);
    }
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path().join(name))
        .find(|p| p.is_file())
}

/// Runs the new binary's `version --json` (its version must be `want`) and
/// `help`.
fn smoke(binary: &str, want: &str) -> Result<()> {
    let out = run_bounded(binary, &["version", "--json"])?;
    let got = serde_json::from_str::<Value>(out.trim())
        .ok()
        .and_then(|v| v.get("version").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default();
    let same = match (
        hive_cli::update::parse_semver(&got),
        hive_cli::update::parse_semver(want),
    ) {
        (Some(a), Some(b)) => hive_cli::update::compare_semver(&a, &b) == std::cmp::Ordering::Equal,
        _ => false,
    };
    if !same {
        bail!(
            "the downloaded hive says it is {:?}, not the release {want}; nothing installed",
            if got.is_empty() { out.trim() } else { &got }
        );
    }
    run_bounded(binary, &["help"])?;
    Ok(())
}

fn run_bounded(binary: &str, args: &[&str]) -> Result<String> {
    let dir = Scratch::new("smoke")?;
    let out_path = dir.path().join("out");
    let out = std::fs::File::create(&out_path)?;
    let mut child = Command::new(binary)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out))
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| anyhow!("smoke test {binary} {}: {e}", args.join(" ")))?;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= SMOKE_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "smoke test {binary} {} timed out after {}s; nothing installed",
                args.join(" "),
                SMOKE_TIMEOUT.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let text = std::fs::read_to_string(&out_path).unwrap_or_default();
    if !status.success() {
        bail!(
            "smoke test {binary} {} exited {}; nothing installed",
            args.join(" "),
            status.code().unwrap_or(-1)
        );
    }
    Ok(text)
}

/// The channel this machine follows, from `update.channel` and what the
/// machine has: a brain project and cargo on PATH.
pub(crate) fn channel(s: &hive_store::Store) -> Channel {
    let has_brain = s
        .conn()
        .ok()
        .map(|conn| !crate::board::brain_project(&conn).str("slug").is_empty())
        .unwrap_or(false);
    let has_cargo = which::which("cargo").is_ok();
    resolve_channel(
        &s.setting(hive_cli::update::CHANNEL_SETTING, "auto"),
        has_brain,
        has_cargo,
    )
}

/// The `meta` row that holds what the console's "Update now" (TASK-093,
/// POST /api/v1/update/apply) is doing or last did.
pub const APPLY_KEY: &str = "update.apply";

/// Set while this process installs a release for the console, so a second
/// click is refused and a stored `installing` left by a hub that stopped
/// mid-install reads as failed.
static APPLYING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// What the console's "Update now" is doing or last did: `state` is `idle`
/// (never asked), `installing`, `installed` (the release is in place; the
/// daemon and the hub restart on it once their runs in flight end),
/// `current` (the release document named nothing newer) or `failed`
/// (`error` says why; nothing was swapped in).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ApplyState {
    pub state: String,
    /// When it started (`installing`) or ended.
    pub at: String,
    pub actor: String,
    /// The version that ran when the install started.
    pub from: String,
    /// The release being installed, or the one the document named.
    pub latest: String,
    pub error: String,
}

/// The console's apply state from the meta row; `idle` when there is none.
/// A stored `installing` no thread of this process runs any more (the hub
/// stopped mid-install) reads as `failed`.
pub fn apply_state(s: &hive_store::Store) -> ApplyState {
    use std::sync::atomic::Ordering;
    // The flag is read before the row: the install thread writes how it
    // ended and only then clears the flag, so a row read after a set flag
    // was still in flight. Read the other way round, an install ending
    // between the two reads left the old `installing` row beside a cleared
    // flag and a finished install read as failed.
    let busy = APPLYING.load(Ordering::SeqCst);
    let mut st = match s.meta(APPLY_KEY) {
        Ok((raw, true)) => serde_json::from_str::<ApplyState>(&raw).unwrap_or_default(),
        _ => ApplyState::default(),
    };
    if st.state.is_empty() {
        st.state = "idle".into();
    }
    // Checked again after the read: an install that started between the
    // two reads wrote its own `installing` row.
    if st.state == "installing" && !busy && !APPLYING.load(Ordering::SeqCst) {
        st.state = "failed".into();
        st.error = "the console stopped while it was installing; hive version says which build is in place. Try again.".into();
    }
    st
}

/// Where `update.channel` points on this machine (`auto` resolved).
pub fn update_channel(s: &hive_store::Store) -> Channel {
    channel(s)
}

/// Why the console's "Update now" did not start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyRefusal {
    /// `update.latest_url` is empty: Hive does not know where releases are.
    NoSource,
    /// `update.channel` resolves to source: the daemon rebuilds from the
    /// brain's checkout instead.
    SourceChannel,
    /// No release is built for this platform.
    NoTarget,
    /// An install is already running.
    Busy,
    /// The install thread could not start.
    Spawn(String),
    /// A package manager installed the program and replaces it itself; the
    /// words say who, and the command to run.
    Managed(String),
}

impl std::fmt::Display for ApplyRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApplyRefusal::NoSource => write!(
                f,
                "{LATEST_URL_SETTING} is empty, so Hive does not know where its releases are"
            ),
            ApplyRefusal::SourceChannel => write!(
                f,
                "update.channel resolves to source here: the daemon rebuilds hive from the brain project's checkout after each landing"
            ),
            ApplyRefusal::NoTarget => write!(
                f,
                "no release is built for {}-{}",
                std::env::consts::ARCH,
                std::env::consts::OS
            ),
            ApplyRefusal::Busy => write!(f, "an update is already being installed"),
            ApplyRefusal::Spawn(e) => write!(f, "could not start the install: {e}"),
            ApplyRefusal::Managed(why) => write!(f, "{why}"),
        }
    }
}

/// The binary an apply replaces and the version it runs; tests name a
/// scratch binary and a fixed version instead of this process's own.
#[derive(Debug, Clone)]
pub struct ApplyTarget {
    pub binary: String,
    pub running: String,
}

impl ApplyTarget {
    pub fn this_process() -> Self {
        Self {
            binary: crate::service::hive_binary(),
            running: hive_cli::version(),
        }
    }
}

/// Why nothing installs a release over `binary`, when a package manager
/// installed it: who owns the file and the command that upgrades it. None
/// for a program Hive may replace.
pub fn managed_refusal(binary: &str) -> Option<String> {
    hive_cli::managed::of(Path::new(binary)).map(|m| m.refusal(binary))
}

/// The console's "Update now" (TASK-093): checks what `hive update` checks
/// (a release source, the release channel, a target), records `installing`
/// in the `update.apply` meta row and runs [`install`] on a thread of its
/// own - downloads take minutes - which records how it ended. The handle
/// is for tests; the hub lets the thread run.
pub fn start_apply(
    s: &hive_store::Store,
    actor: &str,
    target: ApplyTarget,
) -> std::result::Result<std::thread::JoinHandle<ApplyState>, ApplyRefusal> {
    use std::sync::atomic::Ordering;
    if let Some(why) = managed_refusal(&target.binary) {
        return Err(ApplyRefusal::Managed(why));
    }
    let url = s.setting(LATEST_URL_SETTING, "").trim().to_string();
    if url.is_empty() {
        return Err(ApplyRefusal::NoSource);
    }
    if channel(s) == Channel::Source {
        return Err(ApplyRefusal::SourceChannel);
    }
    let triple = hive_cli::update::running_target().ok_or(ApplyRefusal::NoTarget)?;
    if APPLYING.swap(true, Ordering::SeqCst) {
        return Err(ApplyRefusal::Busy);
    }
    let started = ApplyState {
        state: "installing".into(),
        at: s.now(),
        actor: actor.to_string(),
        from: target.running.clone(),
        latest: hive_cli::update::stored_state(s)
            .map(|st| st.latest)
            .unwrap_or_default(),
        error: String::new(),
    };
    let _ = s.set_meta(
        APPLY_KEY,
        &serde_json::to_string(&started).unwrap_or_default(),
    );
    let env = UpdateEnv {
        url,
        running: target.running,
        target: triple,
        binary: target.binary,
        actor: actor.to_string(),
        fetch_json,
        fetch_asset,
    };
    let store = s.clone();
    let begun = started.clone();
    let spawned = std::thread::Builder::new()
        .name("hive-update-apply".into())
        .spawn(move || {
            let st = finish_apply(&store, &env, begun);
            APPLYING.store(false, Ordering::SeqCst);
            st
        });
    spawned.map_err(|e| {
        let failed = ApplyState {
            state: "failed".into(),
            at: s.now(),
            error: format!("could not start the install: {e}"),
            ..started
        };
        let _ = s.set_meta(
            APPLY_KEY,
            &serde_json::to_string(&failed).unwrap_or_default(),
        );
        APPLYING.store(false, Ordering::SeqCst);
        ApplyRefusal::Spawn(e.to_string())
    })
}

/// Runs the install and writes how it ended into the meta row (before the
/// caller clears [`APPLYING`], so a reader never sees a stale `installing`).
fn finish_apply(s: &hive_store::Store, env: &UpdateEnv, started: ApplyState) -> ApplyState {
    let result = install(s, env);
    let mut st = ApplyState {
        at: s.now(),
        ..started
    };
    match result {
        Ok(Outcome::Installed { latest, .. }) => {
            st.state = "installed".into();
            let _ = s.set_meta(hive_cli::update::PROMPTED_KEY, &latest);
            st.latest = latest;
        }
        Ok(Outcome::Current { latest }) => {
            st.state = "current".into();
            st.latest = latest;
        }
        Ok(Outcome::Available { latest, .. }) => {
            st.state = "failed".into();
            st.error = format!("{latest} was found but not installed");
            st.latest = latest;
        }
        Err(e) => {
            st.state = "failed".into();
            st.error = format!("{e:#}");
        }
    }
    let _ = s.set_meta(APPLY_KEY, &serde_json::to_string(&st).unwrap_or_default());
    st
}

/// The lowercase hex SHA-256 of `bytes`, the digest SHA256SUMS lists (for
/// callers building release fixtures without a hashing crate of their own).
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(sha2::Sha256::digest(bytes))
}

/// The line every command prints on stderr once per release (TASK-090),
/// from the stored state with no network: None when nothing newer is
/// known, this machine follows the source channel, or the line for this
/// release was already shown. Showing it records the release in the
/// `update.prompted` meta row, so the next command stays quiet. A program
/// a package manager installed is told that manager's command.
pub fn release_prompt(s: &hive_store::Store) -> Option<String> {
    let state = hive_cli::update::stored_state(s)?;
    let running = hive_cli::version();
    if !state.newer_than(&running) {
        return None;
    }
    let prompted = s
        .meta(hive_cli::update::PROMPTED_KEY)
        .map(|(v, _)| v)
        .unwrap_or_default();
    let line = hive_cli::update::prompt(
        &running,
        &state,
        channel(s),
        &prompted,
        hive_cli::managed::this_program(),
    )?;
    s.set_meta(hive_cli::update::PROMPTED_KEY, &state.latest)
        .ok()?;
    Some(line)
}

pub(crate) fn update_flags(fs: &mut hive_cli::FlagSet) {
    fs.bool_var(
        "check",
        false,
        "say whether a newer release exists; download and install nothing",
    );
}

/// Where Hive's own releases are listed. `hive update`, run by the owner
/// while `update.latest_url` is empty, reads it for that one run. Nothing
/// else reads it: to the daily check, the daemon's self-update, the console,
/// an agent and a job an empty setting still means "do not look".
const PROJECT_RELEASES_URL: &str = "https://api.github.com/repos/lamplitlabs/hive/releases/latest";

/// What `hive update` says before it reads [`PROJECT_RELEASES_URL`] in place
/// of an empty `update.latest_url`.
fn one_run_notice() -> String {
    format!(
        "{LATEST_URL_SETTING} is empty: looking at {PROJECT_RELEASES_URL} for this run. To hear of new releases without asking, set it: hive settings set {LATEST_URL_SETTING} {PROJECT_RELEASES_URL}"
    )
}

/// What `hive update` takes from the build and the machine it runs on;
/// tests name their own.
pub(crate) struct UpdateHost {
    /// The running build's version.
    pub running: String,
    /// The running build's target triple; None where no release is built.
    pub target: Option<String>,
    /// The binary to replace.
    pub binary: String,
    pub fetch_json: fn(&str) -> Result<Value, String>,
    pub fetch_asset: fn(&Asset) -> Result<Vec<u8>, String>,
}

impl UpdateHost {
    fn this_process() -> Self {
        Self {
            running: hive_cli::version(),
            target: hive_cli::update::running_target(),
            binary: crate::service::hive_binary(),
            fetch_json,
            fetch_asset,
        }
    }
}

/// `hive update [--check]`.
pub(crate) fn cmd_update(
    c: &mut hive_cli::Context,
    fs: &hive_cli::FlagSet,
    _args: &[String],
) -> Result<()> {
    update_on(c, fs, UpdateHost::this_process())
}

/// Whether the command runs under an agent's name or inside a job, and so
/// not for the owner: the agent as `hive settings set` tells one from the
/// owner for the owner's settings, the job by `HIVE_JOB`.
fn agent_or_job(c: &hive_cli::Context) -> bool {
    (!c.agent.is_empty() && c.agent != hive_store::HUMAN_AGENT)
        || std::env::var("HIVE_JOB").is_ok_and(|job| !job.is_empty())
}

/// `hive update [--check]` on `host`.
pub(crate) fn update_on(
    c: &mut hive_cli::Context,
    fs: &hive_cli::FlagSet,
    host: UpdateHost,
) -> Result<()> {
    use std::io::Write;
    let s = c
        .store
        .clone()
        .ok_or_else(|| anyhow!("hive update needs the store"))?;
    let only_check = fs.get_bool("check");
    // A package manager's program is that manager's to replace: the command
    // says so before it reads anything. `--check` goes on and reports.
    let managed = hive_cli::managed::of(Path::new(&host.binary));
    if let Some(m) = managed.as_ref().filter(|_| !only_check) {
        bail!("{}", m.refusal(&host.binary));
    }
    let mut url = s.setting(LATEST_URL_SETTING, "").trim().to_string();
    // Looking at the project's own releases is for the owner who runs the
    // command. An agent and a job get the refusal an empty setting always
    // gave, before anything else, and nothing is read.
    if url.is_empty() && agent_or_job(c) {
        bail!(
            "{LATEST_URL_SETTING} is empty, so Hive does not know where its releases are; set it first: hive settings set {LATEST_URL_SETTING} https://api.github.com/repos/OWNER/REPO/releases/latest"
        );
    }
    let channel = channel(&s);
    if channel == Channel::Source && !only_check {
        bail!(
            "update.channel resolves to source here: the daemon rebuilds hive from the brain project's checkout after each landing (daemon.self_update). To install published releases instead: hive settings set update.channel release"
        );
    }
    let target = host.target.ok_or_else(|| {
        anyhow!(
            "no release is built for {}-{}",
            std::env::consts::ARCH,
            std::env::consts::OS
        )
    })?;
    if url.is_empty() {
        // Asked by the owner, the command looks at the project's own
        // releases for this run and says so first; the setting stays empty.
        writeln!(c.err, "{}", one_run_notice())?;
        url = PROJECT_RELEASES_URL.to_string();
    }
    let env = UpdateEnv {
        url,
        running: host.running,
        target,
        binary: host.binary,
        actor: if c.agent.is_empty() {
            crate::util::HUMAN_AGENT.to_string()
        } else {
            c.agent.clone()
        },
        fetch_json: host.fetch_json,
        fetch_asset: host.fetch_asset,
    };
    let outcome = if only_check {
        check(&s, &env)?
    } else {
        install(&s, &env)?
    };
    if !only_check || matches!(outcome, Outcome::Available { .. }) {
        let _ = s.set_meta(
            hive_cli::update::PROMPTED_KEY,
            match &outcome {
                Outcome::Current { latest }
                | Outcome::Available { latest, .. }
                | Outcome::Installed { latest, .. } => latest,
            },
        );
    }
    if c.json {
        let mut v = match &outcome {
            Outcome::Current { latest } => serde_json::json!({
                "state": "current", "running": env.running, "latest": latest,
                "channel": channel.as_str(),
            }),
            Outcome::Available { latest, url } => serde_json::json!({
                "state": "available", "running": env.running, "latest": latest, "url": url,
                "channel": channel.as_str(),
            }),
            Outcome::Installed {
                latest,
                binary,
                prev,
            } => serde_json::json!({
                "state": "installed", "running": env.running, "latest": latest,
                "binary": binary, "prev": prev, "channel": channel.as_str(),
            }),
        };
        // Only a managed program carries these two, so every other
        // answer is what it was.
        if let Some(m) = &managed {
            v["managed_by"] = serde_json::json!(m.owner());
            v["upgrade_command"] = serde_json::json!(m.upgrade);
        }
        hive_cli::write_json(c.out.as_mut(), &v)?;
        return Ok(());
    }
    for line in outcome_lines(&outcome, &env.running, channel, managed.as_ref()) {
        c.println(line)?;
    }
    Ok(())
}

/// What `hive update` prints for `outcome`. For a program a package
/// manager installed (`managed`), the line under a newer release names
/// that manager's command.
pub(crate) fn outcome_lines(
    outcome: &Outcome,
    running: &str,
    channel: Channel,
    managed: Option<&hive_cli::managed::Managed>,
) -> Vec<String> {
    match outcome {
        Outcome::Current { latest } => vec![format!(
            "hive {running} is up to date (latest release {latest})"
        )],
        Outcome::Available { latest, url } => {
            let mut lines = vec![format!(
                "A newer Hive is available: {latest} (running {running}){}",
                if url.is_empty() {
                    String::new()
                } else {
                    format!(" - {url}")
                }
            )];
            lines.push(match (managed, channel) {
                (Some(m), _) => m.upgrade_line(),
                (None, Channel::Release) => "Install it with: hive update".into(),
                (None, Channel::Source) => "This machine follows update.channel source: the daemon builds from the brain's checkout. hive settings set update.channel release to install releases.".into(),
            });
            lines
        }
        Outcome::Installed {
            latest,
            binary,
            prev,
        } => vec![
            format!("Installed Hive {latest} at {binary} (was {running})."),
            format!(
                "The daemon and the hub restart on it once their runs in flight end. Roll back with: mv {prev} {binary}"
            ),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{new_store, serial};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Mutex;

    /// A tiny HTTP server over a folder: GET /NAME answers the file, a
    /// missing one 404. It serves until the test ends.
    fn serve(dir: PathBuf) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .trim_start_matches('/')
                    .to_string();
                let (status, body) = match std::fs::read(dir.join(&path)) {
                    Ok(body) if !path.is_empty() && !path.contains("..") => ("200 OK", body),
                    _ => ("404 Not Found", b"not found".to_vec()),
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(&body);
            }
        });
        format!("http://{addr}")
    }

    const TARGET: &str = "aarch64-apple-darwin";

    /// A release folder: a fake `hive` that answers `version --json` with
    /// `prints`, packed as the target's archive, SHA256SUMS (with
    /// `bad_sum` a wrong digest) and the release document `latest.json`.
    fn release_dir(base: &str, dir: &Path, prints: &str, bad_sum: bool) -> String {
        let stage = dir.join("stage");
        std::fs::create_dir_all(&stage).unwrap();
        let hive = stage.join("hive");
        std::fs::write(
            &hive,
            format!(
                "#!/bin/sh\ncase \"$1\" in version) echo '{{\"version\":\"{prints}\"}}';; help) exit 0;; *) exit 1;; esac\n"
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hive, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let name = hive_cli::update::archive_name("v0.2.0", TARGET);
        let st = Command::new("tar")
            .arg("-czf")
            .arg(dir.join(&name))
            .arg("-C")
            .arg(&stage)
            .arg("hive")
            .status()
            .unwrap();
        assert!(st.success());
        let bytes = std::fs::read(dir.join(&name)).unwrap();
        let digest = if bad_sum {
            "0".repeat(64)
        } else {
            hex::encode(sha2::Sha256::digest(&bytes))
        };
        std::fs::write(
            dir.join(CHECKSUMS_ASSET),
            format!(
                "{digest}  {name}\n{}  hive-v0.2.0-x86_64-apple-darwin.tar.gz\n",
                "1".repeat(64)
            ),
        )
        .unwrap();
        let doc = serde_json::json!({
            "tag_name": "v0.2.0",
            "html_url": format!("{base}/page"),
            "assets": [
                {"name": "hive-v0.2.0-x86_64-apple-darwin.tar.gz", "browser_download_url": format!("{base}/missing.tar.gz")},
                {"name": name, "browser_download_url": format!("{base}/{name}")},
                {"name": CHECKSUMS_ASSET, "browser_download_url": format!("{base}/{CHECKSUMS_ASSET}")},
            ]
        });
        std::fs::write(dir.join("latest.json"), doc.to_string()).unwrap();
        format!("{base}/latest.json")
    }

    fn installed_binary(dir: &Path) -> String {
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let path = bin.join("hive");
        std::fs::write(&path, b"old binary").unwrap();
        path.display().to_string()
    }

    struct SigningLinux(String);
    impl Drop for SigningLinux {
        fn drop(&mut self) {
            *crate::sign::SIGNING_SYSTEM.write().unwrap() = self.0.clone();
        }
    }
    fn signing_linux() -> SigningLinux {
        let old = crate::sign::SIGNING_SYSTEM.read().unwrap().clone();
        *crate::sign::SIGNING_SYSTEM.write().unwrap() = "linux".into();
        SigningLinux(old)
    }

    fn env_for(url: String, binary: String) -> UpdateEnv {
        UpdateEnv {
            url,
            running: "v0.1.0".into(),
            target: TARGET.into(),
            binary,
            actor: "human".into(),
            fetch_json,
            fetch_asset,
        }
    }

    #[test]
    fn update_installs_a_verified_release_from_a_served_folder() {
        let _serial = serial();
        let _signing = signing_linux();
        let s = new_store();
        let served = tempfile::tempdir().unwrap();
        let base = serve(served.path().to_path_buf());
        let url = release_dir(&base, served.path(), "v0.2.0", false);
        let home = tempfile::tempdir().unwrap();
        let binary = installed_binary(home.path());
        let env = env_for(url.clone(), binary.clone());

        assert_eq!(
            check(&s, &env).unwrap(),
            Outcome::Available {
                latest: "v0.2.0".into(),
                url: format!("{base}/page")
            }
        );
        let st = UpdateState::from_json(&s.meta(META_KEY).unwrap().0).unwrap();
        assert_eq!(
            (st.latest.as_str(), st.source.as_str()),
            ("v0.2.0", url.as_str())
        );

        let out = install(&s, &env).unwrap();
        assert_eq!(
            out,
            Outcome::Installed {
                latest: "v0.2.0".into(),
                binary: binary.clone(),
                prev: format!("{binary}.prev"),
            }
        );
        let now = std::fs::read_to_string(&binary).unwrap();
        assert!(now.contains("\"version\":\"v0.2.0\""), "{now}");
        assert_eq!(
            std::fs::read_to_string(format!("{binary}.prev")).unwrap(),
            "old binary"
        );
        assert!(!Path::new(&format!("{binary}.new")).exists());
        let n: i64 = s
            .conn()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM events WHERE action = ?",
                [UPDATE_EVENT],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
        let lines = outcome_lines(&out, "v0.1.0", Channel::Release, None);
        assert!(
            lines[0].starts_with("Installed Hive v0.2.0 at "),
            "{lines:?}"
        );

        // The installed build is the latest: nothing more to do.
        let current = UpdateEnv {
            running: "v0.2.0".into(),
            ..env_for(url, binary)
        };
        assert_eq!(
            install(&s, &current).unwrap(),
            Outcome::Current {
                latest: "v0.2.0".into()
            }
        );
    }

    /// ADR-070 step 6: `daemon.self_update release` replaces the binary from
    /// a published release - here a local release folder - and never runs
    /// cargo; a hosted brain holds no Hive source to build from.
    #[test]
    fn self_update_release_mode_installs_the_release_and_never_runs_cargo() {
        let _serial = serial();
        let _env = hive_testutil::env_lock();
        let _signing = signing_linux();
        let _reset = crate::test_helpers_selfupdate::reset_guard();
        let s = new_store();
        let served = tempfile::tempdir().unwrap();
        let base = serve(served.path().to_path_buf());
        let url = release_dir(&base, served.path(), "v0.2.0", false);
        let home = tempfile::tempdir().unwrap();
        let binary = installed_binary(home.path());
        // A cargo on PATH that leaves a mark when anything runs it.
        let tools = home.path().join("tools");
        std::fs::create_dir_all(&tools).unwrap();
        let mark = home.path().join("cargo-ran");
        std::fs::write(
            tools.join("cargo"),
            format!("#!/bin/sh\ntouch '{}'\nexit 1\n", mark.display()),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(tools.join("cargo"), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        let _path = hive_testutil::setenv(
            "PATH",
            format!(
                "{}:{}",
                tools.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        );
        let env = crate::selfupdate::SelfUpdateEnv {
            binary: binary.clone(),
            cargo_cmd: "cargo".into(),
            revision: "0000000000000000000000000000000000000000".into(),
            actor: "daemon".into(),
            inflight: 1,
            version: "v0.1.0".into(),
            target: TARGET.into(),
            ..Default::default()
        };
        let set = |name: &str, value: &str| s.set_setting("human", name, value).unwrap();

        // Release mode without a release source says so once and installs nothing.
        set("daemon.self_update", "release");
        let lines = crate::selfupdate::tend_binary(&s, s.now(), env.clone());
        assert!(
            lines.len() == 1 && lines[0].contains("update.latest_url is empty"),
            "{lines:?}"
        );
        assert!(
            crate::selfupdate::tend_binary(&s, s.now(), env.clone()).is_empty(),
            "the same wait is said once"
        );

        set(LATEST_URL_SETTING, &url);
        let lines = crate::selfupdate::tend_binary(&s, s.now(), env.clone());
        assert!(
            lines.is_empty(),
            "the install runs on its thread: {lines:?}"
        );
        crate::test_helpers_selfupdate::wait_self_update_idle();
        let lines = crate::selfupdate::tend_binary(&s, s.now(), env.clone());
        assert!(
            lines
                .iter()
                .any(|l| l.contains("installed release v0.2.0") && l.contains(&binary)),
            "{lines:?}"
        );
        // The release's own binary is in place, the old one kept beside it.
        let installed = std::fs::read_to_string(&binary).unwrap();
        assert!(installed.contains("v0.2.0"), "{installed}");
        assert_eq!(
            std::fs::read_to_string(format!("{binary}.prev")).unwrap(),
            "old binary"
        );
        assert!(!mark.exists(), "release mode ran cargo");
        let detail: String = s
            .conn()
            .unwrap()
            .query_row(
                "SELECT detail FROM events WHERE action = ? ORDER BY seq DESC LIMIT 1",
                [crate::selfupdate::SELF_UPDATE_EVENT],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            detail.contains("\"mode\": \"release\"") && detail.contains("\"to\": \"v0.2.0\""),
            "{detail}"
        );
        // Until the restart, the daemon waits on what it installed (said
        // once, on the pass that reported the install) and reads nothing
        // more.
        assert!(
            lines
                .iter()
                .any(|l| l.contains("release v0.2.0 is installed")),
            "{lines:?}"
        );
        let lines = crate::selfupdate::tend_binary(&s, s.now(), env);
        assert!(lines.is_empty(), "the wait is said once: {lines:?}");
        assert!(!mark.exists(), "release mode ran cargo");
    }

    #[test]
    fn update_refuses_a_bad_checksum_a_wrong_binary_or_a_missing_target() {
        let _serial = serial();
        let _signing = signing_linux();
        let s = new_store();
        let home = tempfile::tempdir().unwrap();
        let binary = installed_binary(home.path());

        let bad = tempfile::tempdir().unwrap();
        let base = serve(bad.path().to_path_buf());
        let url = release_dir(&base, bad.path(), "v0.2.0", true);
        let err = install(&s, &env_for(url.clone(), binary.clone()))
            .unwrap_err()
            .to_string();
        assert!(err.contains("does not match SHA256SUMS"), "{err}");
        assert_eq!(std::fs::read_to_string(&binary).unwrap(), "old binary");

        let other = UpdateEnv {
            target: "x86_64-unknown-linux-gnu".into(),
            ..env_for(url, binary.clone())
        };
        let err = install(&s, &other).unwrap_err().to_string();
        assert!(
            err.contains("no archive for x86_64-unknown-linux-gnu"),
            "{err}"
        );

        let wrong = tempfile::tempdir().unwrap();
        let base = serve(wrong.path().to_path_buf());
        let url = release_dir(&base, wrong.path(), "v0.1.9", false);
        let err = install(&s, &env_for(url, binary.clone()))
            .unwrap_err()
            .to_string();
        assert!(err.contains("not the release v0.2.0"), "{err}");
        assert_eq!(std::fs::read_to_string(&binary).unwrap(), "old binary");
        assert!(!Path::new(&format!("{binary}.new")).exists());
        assert!(!Path::new(&format!("{binary}.prev")).exists());

        // A document that cannot be read keeps the last good release.
        let gone = env_for(format!("{base}/nothing.json"), binary);
        let err = install(&s, &gone).unwrap_err().to_string();
        assert!(err.contains("HTTP 404"), "{err}");
        let st = UpdateState::from_json(&s.meta(META_KEY).unwrap().0).unwrap();
        assert!(st.error.contains("HTTP 404"), "{st:?}");
    }

    #[test]
    fn check_reads_a_fresh_stored_state_without_the_network() {
        let _serial = serial();
        let s = new_store();
        static ASKED: Mutex<usize> = Mutex::new(0);
        fn never(_: &str) -> Result<Value, String> {
            *ASKED.lock().unwrap() += 1;
            Err("offline".into())
        }
        let url = "https://releases.test/latest.json".to_string();
        let st = UpdateState {
            checked_at: s.now(),
            source: url.clone(),
            latest: "v0.3.0".into(),
            url: "https://releases.test/v0.3.0".into(),
            error: String::new(),
        };
        s.set_meta(META_KEY, &serde_json::to_string(&st).unwrap())
            .unwrap();
        let env = UpdateEnv {
            fetch_json: never,
            ..env_for(url, String::new())
        };
        assert_eq!(
            check(&s, &env).unwrap(),
            Outcome::Available {
                latest: "v0.3.0".into(),
                url: "https://releases.test/v0.3.0".into()
            }
        );
        assert_eq!(*ASKED.lock().unwrap(), 0);
    }

    /// The addresses `hive update` asked its fake for, and what the fake
    /// answers; the tests that use them hold `serial()`.
    static HAND_ASKED: Mutex<Vec<String>> = Mutex::new(Vec::new());
    static HAND_ANSWER: Mutex<Option<Value>> = Mutex::new(None);

    fn hand_answers(doc: Value) {
        HAND_ASKED.lock().unwrap().clear();
        *HAND_ANSWER.lock().unwrap() = Some(doc);
    }

    fn hand_asked() -> Vec<String> {
        HAND_ASKED.lock().unwrap().clone()
    }

    fn canned(url: &str) -> Result<Value, String> {
        HAND_ASKED.lock().unwrap().push(url.to_string());
        HAND_ANSWER
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| "offline".to_string())
    }

    fn no_download(asset: &Asset) -> Result<Vec<u8>, String> {
        Err(format!("{} must not be downloaded here", asset.name))
    }

    /// A host that runs v0.1.0, replaces `binary`, and reads documents from
    /// the fake.
    fn hand_host(binary: &str) -> UpdateHost {
        UpdateHost {
            running: "v0.1.0".into(),
            target: Some(TARGET.into()),
            binary: binary.to_string(),
            fetch_json: canned,
            fetch_asset: no_download,
        }
    }

    /// `hive update ARGS` on `host`, run by the owner with no name given.
    fn update(
        s: &hive_store::Store,
        args: &[&str],
        json: bool,
        host: UpdateHost,
    ) -> (String, String, i32) {
        update_as(s, "", args, json, host)
    }

    /// `hive update ARGS` on `host` under the name `agent` (`--agent` or
    /// `HIVE_AGENT`; empty for none): stdout, stderr and the exit status, a
    /// failure reported the way the binary reports it.
    fn update_as(
        s: &hive_store::Store,
        agent: &str,
        args: &[&str],
        json: bool,
        host: UpdateHost,
    ) -> (String, String, i32) {
        let mut fs = hive_cli::FlagSet::new("update");
        update_flags(&mut fs);
        let args: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
        fs.parse(&args).unwrap();
        let out = hive_cli::Buffer::default();
        let err = hive_cli::Buffer::default();
        let mut c = hive_cli::Context {
            store: Some(s.clone()),
            home: s.home.clone(),
            out: Box::new(out.clone()),
            err: Box::new(err.clone()),
            json,
            quiet: false,
            color: false,
            agent: agent.to_string(),
            project: String::new(),
            session: String::new(),
            now: crate::test_helpers::TEST_NOW.to_string(),
            args: Vec::new(),
        };
        let code = match update_on(&mut c, &fs, host) {
            Ok(()) => 0,
            Err(e) => {
                let _ = writeln!(c.err, "hive update: {e}");
                1
            }
        };
        (out.string(), err.string(), code)
    }

    const ONE_RUN_NOTICE: &str = "update.latest_url is empty: looking at https://api.github.com/repos/lamplitlabs/hive/releases/latest for this run. To hear of new releases without asking, set it: hive settings set update.latest_url https://api.github.com/repos/lamplitlabs/hive/releases/latest\n";

    fn project_release() -> Value {
        serde_json::json!({
            "tag_name": "v0.3.0",
            "html_url": "https://github.com/lamplitlabs/hive/releases/tag/v0.3.0",
        })
    }

    #[test]
    fn update_by_hand_with_no_latest_url_looks_at_the_projects_releases_for_that_run() {
        let _serial = serial();
        let s = new_store();
        s.set_setting("human", hive_cli::update::CHANNEL_SETTING, "release")
            .unwrap();
        hand_answers(project_release());

        let (out, err, code) = update(&s, &["--check"], false, hand_host(""));
        assert_eq!(code, 0, "{err}");
        assert_eq!(err, ONE_RUN_NOTICE);
        assert_eq!(
            out,
            "A newer Hive is available: v0.3.0 (running v0.1.0) - https://github.com/lamplitlabs/hive/releases/tag/v0.3.0\nInstall it with: hive update\n"
        );
        assert_eq!(hand_asked(), vec![PROJECT_RELEASES_URL.to_string()]);

        // For that run only: the setting is as empty as it was, and what
        // reads it still does not look anywhere.
        assert_eq!(s.setting(LATEST_URL_SETTING, ""), "");
        assert!(hive_cli::update::stored_state(&s).is_none());
        assert_eq!(release_prompt(&s), None);
        let daily = crate::update_check::set_fetch(canned);
        let lines = crate::update_check::tend_update_check(&s, &s.now(), "v0.1.0");
        crate::update_check::set_fetch(daily);
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(
            start_apply(
                &s,
                "human",
                ApplyTarget {
                    binary: "/nonexistent/hive".into(),
                    running: "v0.1.0".into(),
                },
            )
            .err(),
            Some(ApplyRefusal::NoSource)
        );
        assert_eq!(hand_asked().len(), 1, "{:?}", hand_asked());

        // Asked again the same day it answers from what the first run kept,
        // as it does with the setting set, and --json keeps stdout for the
        // document.
        let (out, err, code) = update(&s, &["--check"], true, hand_host(""));
        assert_eq!(code, 0, "{err}");
        assert_eq!(err, ONE_RUN_NOTICE);
        let doc: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            (doc["state"].as_str(), doc["latest"].as_str()),
            (Some("available"), Some("v0.3.0")),
            "{doc}"
        );
        assert_eq!(hand_asked().len(), 1, "{:?}", hand_asked());

        // A setting that holds an address is the one it reads, with no word
        // about the project's.
        let own = "https://releases.test/latest.json";
        s.set_setting("human", LATEST_URL_SETTING, own).unwrap();
        let (out, err, code) = update(&s, &["--check"], false, hand_host(""));
        assert_eq!((code, err.as_str()), (0, ""), "{out}");
        assert_eq!(
            hand_asked(),
            vec![PROJECT_RELEASES_URL.to_string(), own.to_string()]
        );
    }

    /// A package manager's file is that manager's to replace
    /// (hive_cli::managed): `hive update` refuses before it reads anything
    /// and names the manager's command, `--check` still reports, the
    /// console's Update now refuses, and `install` refuses whoever calls it.
    #[test]
    fn a_program_a_package_manager_installed_is_never_replaced() {
        let _serial = serial();
        let _signing = signing_linux();
        let s = new_store();
        s.set_setting("human", hive_cli::update::CHANNEL_SETTING, "release")
            .unwrap();
        let served = tempfile::tempdir().unwrap();
        let base = serve(served.path().to_path_buf());
        let url = release_dir(&base, served.path(), "v0.2.0", false);
        s.set_setting("human", LATEST_URL_SETTING, &url).unwrap();
        let home = tempfile::tempdir().unwrap();
        let binary = installed_binary(home.path());
        let marker = format!("{binary}{}", hive_cli::managed::SUFFIX);
        std::fs::write(&marker, "by=Homebrew\nupgrade=brew upgrade hivefleet\n").unwrap();
        let refusal = format!(
            "Homebrew installed this hive and owns {binary}, so Hive does not replace that file. Upgrade it with: brew upgrade hivefleet"
        );
        let untouched = || {
            assert_eq!(std::fs::read_to_string(&binary).unwrap(), "old binary");
            assert!(!Path::new(&format!("{binary}.new")).exists());
            assert!(!Path::new(&format!("{binary}.prev")).exists());
        };

        hand_answers(project_release());
        let (out, err, code) = update(&s, &[], false, hand_host(&binary));
        assert_eq!((code, out.as_str()), (1, ""), "{err}");
        assert_eq!(err, format!("hive update: {refusal}\n"));
        assert!(hand_asked().is_empty(), "{:?}", hand_asked());
        // The source channel's own refusal does not come first: the daemon
        // builds nothing over this file either.
        s.set_setting("human", hive_cli::update::CHANNEL_SETTING, "source")
            .unwrap();
        let (_, err, code) = update(&s, &[], false, hand_host(&binary));
        assert_eq!((code, err), (1, format!("hive update: {refusal}\n")));
        // An agent is told the same, and nothing is read for it either.
        let (_, err, code) = update_as(&s, "worker-1", &[], false, hand_host(&binary));
        assert_eq!((code, err), (1, format!("hive update: {refusal}\n")));
        assert!(hand_asked().is_empty(), "{:?}", hand_asked());

        let (out, err, code) = update(&s, &["--check"], false, hand_host(&binary));
        assert_eq!((code, err.as_str()), (0, ""));
        assert_eq!(
            out,
            "A newer Hive is available: v0.3.0 (running v0.1.0) - https://github.com/lamplitlabs/hive/releases/tag/v0.3.0\nUpgrade it with: brew upgrade hivefleet\n"
        );
        let (out, _, code) = update(&s, &["--check"], true, hand_host(&binary));
        assert_eq!(code, 0);
        let doc: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            (
                doc["state"].as_str(),
                doc["managed_by"].as_str(),
                doc["upgrade_command"].as_str()
            ),
            (
                Some("available"),
                Some("Homebrew"),
                Some("brew upgrade hivefleet")
            ),
            "{doc}"
        );
        s.set_setting("human", hive_cli::update::CHANNEL_SETTING, "release")
            .unwrap();

        let err = install(&s, &env_for(url.clone(), binary.clone()))
            .unwrap_err()
            .to_string();
        assert_eq!(err, refusal);
        assert_eq!(
            start_apply(
                &s,
                "human",
                ApplyTarget {
                    binary: binary.clone(),
                    running: "v0.1.0".into(),
                },
            )
            .err(),
            Some(ApplyRefusal::Managed(refusal.clone()))
        );
        assert_eq!(apply_state(&s).state, "idle");
        assert_eq!(managed_refusal(&binary), Some(refusal));
        untouched();

        // A marker that names nobody is a marker all the same.
        std::fs::write(&marker, "").unwrap();
        let err = install(&s, &env_for(url.clone(), binary.clone()))
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            format!(
                "A package manager installed this hive and owns {binary}, so Hive does not replace that file. Upgrade it with the package manager that installed it."
            )
        );
        untouched();

        // Without the marker the same machine installs the same release.
        std::fs::remove_file(&marker).unwrap();
        assert_eq!(managed_refusal(&binary), None);
        assert!(matches!(
            install(&s, &env_for(url, binary.clone())).unwrap(),
            Outcome::Installed { .. }
        ));
    }

    #[test]
    fn update_by_hand_with_no_latest_url_installs_the_projects_release() {
        let _serial = serial();
        let _signing = signing_linux();
        let s = new_store();
        s.set_setting("human", hive_cli::update::CHANNEL_SETTING, "release")
            .unwrap();
        // The project's address answers with the document of a served
        // folder, whose files the real download then fetches from it.
        let served = tempfile::tempdir().unwrap();
        let base = serve(served.path().to_path_buf());
        release_dir(&base, served.path(), "v0.2.0", false);
        let doc = std::fs::read(served.path().join("latest.json")).unwrap();
        hand_answers(serde_json::from_slice(&doc).unwrap());
        let home = tempfile::tempdir().unwrap();
        let binary = installed_binary(home.path());
        let host = UpdateHost {
            fetch_asset,
            ..hand_host(&binary)
        };

        let (out, err, code) = update(&s, &[], false, host);
        assert_eq!(code, 0, "{err}");
        assert_eq!(err, ONE_RUN_NOTICE);
        assert_eq!(
            out,
            format!(
                "Installed Hive v0.2.0 at {binary} (was v0.1.0).\nThe daemon and the hub restart on it once their runs in flight end. Roll back with: mv {binary}.prev {binary}\n"
            )
        );
        assert_eq!(hand_asked(), vec![PROJECT_RELEASES_URL.to_string()]);
        let now = std::fs::read_to_string(&binary).unwrap();
        assert!(now.contains("\"version\":\"v0.2.0\""), "{now}");
        assert_eq!(
            std::fs::read_to_string(format!("{binary}.prev")).unwrap(),
            "old binary"
        );
        assert_eq!(s.setting(LATEST_URL_SETTING, ""), "");
    }

    /// The fallback is for the owner. Under an agent's name or inside a job
    /// an empty setting is the refusal it always was, and nothing is read.
    #[test]
    fn update_under_an_agents_name_or_inside_a_job_keeps_the_old_refusal() {
        let _serial = serial();
        let s = new_store();
        s.set_setting("human", hive_cli::update::CHANNEL_SETTING, "release")
            .unwrap();
        hand_answers(project_release());
        let refusal = "hive update: update.latest_url is empty, so Hive does not know where its releases are; set it first: hive settings set update.latest_url https://api.github.com/repos/OWNER/REPO/releases/latest\n";
        let refused = |agent: &str, args: &[&str], what: &str| {
            let (out, err, code) = update_as(&s, agent, args, false, hand_host(""));
            assert_eq!(
                (code, out.as_str(), err.as_str()),
                (1, "", refusal),
                "{what}"
            );
        };

        refused("aurora", &["--check"], "an agent, --check");
        refused("aurora", &[], "an agent");
        {
            let _job = hive_testutil::setenv("HIVE_JOB", "job-1");
            refused("", &["--check"], "a job, --check");
            refused("", &[], "a job");
            refused("human", &[], "a job under the owner's name");
            refused("aurora", &[], "an agent's job");
        }
        // As before the fallback existed, the refusal comes before the one
        // of the source channel.
        s.set_setting("human", hive_cli::update::CHANNEL_SETTING, "source")
            .unwrap();
        refused("aurora", &[], "an agent on the source channel");
        s.set_setting("human", hive_cli::update::CHANNEL_SETTING, "release")
            .unwrap();
        // Nothing was read and nothing kept.
        assert!(hand_asked().is_empty(), "{:?}", hand_asked());
        assert!(!s.meta(META_KEY).unwrap().1);

        // The owner, named or not, gets the project's releases.
        let (_, err, code) = update_as(&s, "human", &["--check"], false, hand_host(""));
        assert_eq!((code, err.as_str()), (0, ONE_RUN_NOTICE));
        assert_eq!(hand_asked(), vec![PROJECT_RELEASES_URL.to_string()]);

        // A setting that holds an address is read by an agent and by a job
        // as it always was.
        let own = "https://releases.test/latest.json";
        s.set_setting("human", LATEST_URL_SETTING, own).unwrap();
        let (out, err, code) = update_as(&s, "aurora", &["--check"], false, hand_host(""));
        assert_eq!((code, err.as_str()), (0, ""), "{out}");
        assert!(
            out.starts_with("A newer Hive is available: v0.3.0"),
            "{out}"
        );
        assert_eq!(
            hand_asked(),
            vec![PROJECT_RELEASES_URL.to_string(), own.to_string()]
        );
        let _job = hive_testutil::setenv("HIVE_JOB", "job-1");
        let (out, err, code) = update(&s, &["--check"], false, hand_host(""));
        assert_eq!((code, err.as_str()), (0, ""), "{out}");
        assert!(
            out.starts_with("A newer Hive is available: v0.3.0"),
            "{out}"
        );
    }

    #[test]
    fn update_on_the_source_channel_still_refuses_to_install_and_check_still_reports() {
        let _serial = serial();
        let s = new_store();
        s.set_setting("human", hive_cli::update::CHANNEL_SETTING, "source")
            .unwrap();
        hand_answers(project_release());

        // It refuses before it looks anywhere, so it names no address.
        let (out, err, code) = update(&s, &[], false, hand_host(""));
        assert_eq!((code, out.as_str()), (1, ""), "{err}");
        assert_eq!(
            err,
            "hive update: update.channel resolves to source here: the daemon rebuilds hive from the brain project's checkout after each landing (daemon.self_update). To install published releases instead: hive settings set update.channel release\n"
        );
        assert!(hand_asked().is_empty(), "{:?}", hand_asked());

        let (out, err, code) = update(&s, &["--check"], false, hand_host(""));
        assert_eq!(code, 0, "{err}");
        assert_eq!(err, ONE_RUN_NOTICE);
        assert_eq!(
            out,
            "A newer Hive is available: v0.3.0 (running v0.1.0) - https://github.com/lamplitlabs/hive/releases/tag/v0.3.0\nThis machine follows update.channel source: the daemon builds from the brain's checkout. hive settings set update.channel release to install releases.\n"
        );
        assert_eq!(hand_asked(), vec![PROJECT_RELEASES_URL.to_string()]);
    }

    #[test]
    fn apply_refuses_without_a_source_or_on_source_and_a_stale_install_reads_failed() {
        let _serial = serial();
        let s = new_store();
        let target = ApplyTarget {
            binary: "/nonexistent/hive".into(),
            running: "v0.1.0".into(),
        };
        assert_eq!(apply_state(&s).state, "idle");
        assert_eq!(
            start_apply(&s, "human", target.clone()).err(),
            Some(ApplyRefusal::NoSource)
        );
        s.set_setting(
            "human",
            LATEST_URL_SETTING,
            "https://releases.test/latest.json",
        )
        .unwrap();
        s.set_setting("human", hive_cli::update::CHANNEL_SETTING, "source")
            .unwrap();
        assert_eq!(
            start_apply(&s, "human", target).err(),
            Some(ApplyRefusal::SourceChannel)
        );
        assert_eq!(apply_state(&s).state, "idle");
        // A hub that stopped mid-install left `installing` behind.
        let left = ApplyState {
            state: "installing".into(),
            at: s.now(),
            ..ApplyState::default()
        };
        s.set_meta(APPLY_KEY, &serde_json::to_string(&left).unwrap())
            .unwrap();
        let st = apply_state(&s);
        assert_eq!(st.state, "failed");
        assert!(st.error.contains("stopped"), "{st:?}");
    }
}
