//! `pulse update [--check]` (TASK-090): installs the newer published
//! release that `update.latest_url` names. The release document's `assets`
//! give the archive for the running target
//! (`pulse-vX.Y.Z-<target>.tar.gz`) and `SHA256SUMS`; both are downloaded,
//! the archive must match its line in `SHA256SUMS`, and the `pulse` inside it
//! goes the way the daemon's own build goes (selfupdate.rs): written beside
//! the running binary as `<binary>.new`, signed (sign.rs), smoke-tested with
//! `version --json` (the version it prints must be the release's) and
//! `help`, the running binary copied aside as `<binary>.prev`, and the new
//! file renamed into place. The daemon sees its binary change on disk and
//! restarts on it after its runs in flight (restart.rs); nothing here stops
//! a run.
//!
//! A private repository answers 404 to an anonymous request, so a GitHub
//! address that fails over HTTP is read again through `gh api` when gh is
//! on PATH (the account the owner is signed in with); the daily check
//! (update_check.rs) reads the document the same way.
//!
//! What is pure - asset choice, checksums, the channel and the
//! once-per-release prompt - lives in `pulse_cli::update` with its tests.

use anyhow::{Result, anyhow, bail};
use pulse_cli::update::{
    Asset, CHECK_HOURS, CHECKSUMS_ASSET, Channel, LATEST_URL_SETTING, META_KEY, Release,
    UpdateState, check_digest, parse_checksums, parse_release, pick_asset, resolve_channel,
};
use serde_json::Value;
use sha2::Digest;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The event an installed release writes.
pub(crate) const UPDATE_EVENT: &str = "pulse.update";
const DOC_TIMEOUT: Duration = Duration::from_secs(20);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const SMOKE_TIMEOUT: Duration = Duration::from_secs(60);
const USER_AGENT: &str = "pulse-update";
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
            "pulse-{what}-{}-{}",
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

/// Everything one `pulse update` works with; tests build their own.
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

/// What `pulse update` found or did.
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
pub(crate) fn read_release(s: &pulse_store::Store, env: &UpdateEnv) -> Result<Release> {
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

/// `pulse update --check` reads the stored state when the daily check is
/// fresh and the document otherwise; `pulse update` always reads it (the
/// stored state keeps no assets).
pub(crate) fn check(s: &pulse_store::Store, env: &UpdateEnv) -> Result<Outcome> {
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
    if pulse_cli::update::is_newer(&latest, &env.running) {
        Ok(Outcome::Available { latest, url })
    } else {
        Ok(Outcome::Current { latest })
    }
}

/// Downloads, verifies and installs the newer release, or says the running
/// build is current.
pub(crate) fn install(s: &pulse_store::Store, env: &UpdateEnv) -> Result<Outcome> {
    let release = read_release(s, env)?;
    if !pulse_cli::update::is_newer(&release.version, &env.running) {
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
        .ok_or_else(|| anyhow!("{} holds no file named pulse", archive.name))?;

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
    let _ = pulse_bus::emit(s, &env.actor, UPDATE_EVENT, "pulse", detail);
    Ok(Outcome::Installed {
        latest: release.version,
        binary: env.binary.clone(),
        prev,
    })
}

/// The `pulse` file in an unpacked archive: at its top or one folder down.
fn find_binary(dir: &Path) -> Option<PathBuf> {
    let name = if cfg!(windows) { "pulse.exe" } else { "pulse" };
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
        pulse_cli::update::parse_semver(&got),
        pulse_cli::update::parse_semver(want),
    ) {
        (Some(a), Some(b)) => {
            pulse_cli::update::compare_semver(&a, &b) == std::cmp::Ordering::Equal
        }
        _ => false,
    };
    if !same {
        bail!(
            "the downloaded pulse says it is {:?}, not the release {want}; nothing installed",
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
pub(crate) fn channel(s: &pulse_store::Store) -> Channel {
    let has_brain = s
        .conn()
        .ok()
        .map(|conn| !crate::board::brain_project(&conn).str("slug").is_empty())
        .unwrap_or(false);
    let has_cargo = which::which("cargo").is_ok();
    resolve_channel(
        &s.setting(pulse_cli::update::CHANNEL_SETTING, "auto"),
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
pub fn apply_state(s: &pulse_store::Store) -> ApplyState {
    let mut st = match s.meta(APPLY_KEY) {
        Ok((raw, true)) => serde_json::from_str::<ApplyState>(&raw).unwrap_or_default(),
        _ => ApplyState::default(),
    };
    if st.state.is_empty() {
        st.state = "idle".into();
    }
    if st.state == "installing" && !APPLYING.load(std::sync::atomic::Ordering::SeqCst) {
        st.state = "failed".into();
        st.error = "the console stopped while it was installing; pulse version says which build is in place. Try again.".into();
    }
    st
}

/// Where `update.channel` points on this machine (`auto` resolved).
pub fn update_channel(s: &pulse_store::Store) -> Channel {
    channel(s)
}

/// Why the console's "Update now" did not start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyRefusal {
    /// `update.latest_url` is empty: PULSE does not know where releases are.
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
}

impl std::fmt::Display for ApplyRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApplyRefusal::NoSource => write!(
                f,
                "{LATEST_URL_SETTING} is empty, so PULSE does not know where its releases are"
            ),
            ApplyRefusal::SourceChannel => write!(
                f,
                "update.channel resolves to source here: the daemon rebuilds pulse from the brain project's checkout after each landing"
            ),
            ApplyRefusal::NoTarget => write!(
                f,
                "no release is built for {}-{}",
                std::env::consts::ARCH,
                std::env::consts::OS
            ),
            ApplyRefusal::Busy => write!(f, "an update is already being installed"),
            ApplyRefusal::Spawn(e) => write!(f, "could not start the install: {e}"),
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
            binary: crate::service::pulse_binary(),
            running: pulse_cli::version(),
        }
    }
}

/// The console's "Update now" (TASK-093): checks what `pulse update` checks
/// (a release source, the release channel, a target), records `installing`
/// in the `update.apply` meta row and runs [`install`] on a thread of its
/// own - downloads take minutes - which records how it ended. The handle
/// is for tests; the hub lets the thread run.
pub fn start_apply(
    s: &pulse_store::Store,
    actor: &str,
    target: ApplyTarget,
) -> std::result::Result<std::thread::JoinHandle<ApplyState>, ApplyRefusal> {
    use std::sync::atomic::Ordering;
    let url = s.setting(LATEST_URL_SETTING, "").trim().to_string();
    if url.is_empty() {
        return Err(ApplyRefusal::NoSource);
    }
    if channel(s) == Channel::Source {
        return Err(ApplyRefusal::SourceChannel);
    }
    let triple = pulse_cli::update::running_target().ok_or(ApplyRefusal::NoTarget)?;
    if APPLYING.swap(true, Ordering::SeqCst) {
        return Err(ApplyRefusal::Busy);
    }
    let started = ApplyState {
        state: "installing".into(),
        at: s.now(),
        actor: actor.to_string(),
        from: target.running.clone(),
        latest: pulse_cli::update::stored_state(s)
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
        .name("pulse-update-apply".into())
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
fn finish_apply(s: &pulse_store::Store, env: &UpdateEnv, started: ApplyState) -> ApplyState {
    let result = install(s, env);
    let mut st = ApplyState {
        at: s.now(),
        ..started
    };
    match result {
        Ok(Outcome::Installed { latest, .. }) => {
            st.state = "installed".into();
            let _ = s.set_meta(pulse_cli::update::PROMPTED_KEY, &latest);
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
/// `update.prompted` meta row, so the next command stays quiet.
pub fn release_prompt(s: &pulse_store::Store) -> Option<String> {
    let state = pulse_cli::update::stored_state(s)?;
    let running = pulse_cli::version();
    if !state.newer_than(&running) {
        return None;
    }
    let prompted = s
        .meta(pulse_cli::update::PROMPTED_KEY)
        .map(|(v, _)| v)
        .unwrap_or_default();
    let line = pulse_cli::update::prompt(&running, &state, channel(s), &prompted)?;
    s.set_meta(pulse_cli::update::PROMPTED_KEY, &state.latest)
        .ok()?;
    Some(line)
}

pub(crate) fn update_flags(fs: &mut pulse_cli::FlagSet) {
    fs.bool_var(
        "check",
        false,
        "say whether a newer release exists; download and install nothing",
    );
}

/// `pulse update [--check]`.
pub(crate) fn cmd_update(
    c: &mut pulse_cli::Context,
    fs: &pulse_cli::FlagSet,
    _args: &[String],
) -> Result<()> {
    let s = c
        .store
        .clone()
        .ok_or_else(|| anyhow!("pulse update needs the store"))?;
    let url = s.setting(LATEST_URL_SETTING, "").trim().to_string();
    if url.is_empty() {
        bail!(
            "{LATEST_URL_SETTING} is empty, so PULSE does not know where its releases are; set it first: pulse settings set {LATEST_URL_SETTING} https://api.github.com/repos/OWNER/REPO/releases/latest"
        );
    }
    let only_check = fs.get_bool("check");
    let channel = channel(&s);
    if channel == Channel::Source && !only_check {
        bail!(
            "update.channel resolves to source here: the daemon rebuilds pulse from the brain project's checkout after each landing (daemon.self_update). To install published releases instead: pulse settings set update.channel release"
        );
    }
    let target = pulse_cli::update::running_target().ok_or_else(|| {
        anyhow!(
            "no release is built for {}-{}",
            std::env::consts::ARCH,
            std::env::consts::OS
        )
    })?;
    let env = UpdateEnv {
        url,
        running: pulse_cli::version(),
        target,
        binary: crate::service::pulse_binary(),
        actor: if c.agent.is_empty() {
            crate::util::HUMAN_AGENT.to_string()
        } else {
            c.agent.clone()
        },
        fetch_json,
        fetch_asset,
    };
    let outcome = if only_check {
        check(&s, &env)?
    } else {
        install(&s, &env)?
    };
    if !only_check || matches!(outcome, Outcome::Available { .. }) {
        let _ = s.set_meta(
            pulse_cli::update::PROMPTED_KEY,
            match &outcome {
                Outcome::Current { latest }
                | Outcome::Available { latest, .. }
                | Outcome::Installed { latest, .. } => latest,
            },
        );
    }
    if c.json {
        let v = match &outcome {
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
        pulse_cli::write_json(c.out.as_mut(), &v)?;
        return Ok(());
    }
    for line in outcome_lines(&outcome, &env.running, channel) {
        c.println(line)?;
    }
    Ok(())
}

/// What `pulse update` prints for `outcome`.
pub(crate) fn outcome_lines(outcome: &Outcome, running: &str, channel: Channel) -> Vec<String> {
    match outcome {
        Outcome::Current { latest } => vec![format!(
            "pulse {running} is up to date (latest release {latest})"
        )],
        Outcome::Available { latest, url } => {
            let mut lines = vec![format!(
                "A newer PULSE is available: {latest} (running {running}){}",
                if url.is_empty() {
                    String::new()
                } else {
                    format!(" - {url}")
                }
            )];
            lines.push(match channel {
                Channel::Release => "Install it with: pulse update".into(),
                Channel::Source => "This machine follows update.channel source: the daemon builds from the brain's checkout. pulse settings set update.channel release to install releases.".into(),
            });
            lines
        }
        Outcome::Installed {
            latest,
            binary,
            prev,
        } => vec![
            format!("Installed PULSE {latest} at {binary} (was {running})."),
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

    /// A release folder: a fake `pulse` that answers `version --json` with
    /// `prints`, packed as the target's archive, SHA256SUMS (with
    /// `bad_sum` a wrong digest) and the release document `latest.json`.
    fn release_dir(base: &str, dir: &Path, prints: &str, bad_sum: bool) -> String {
        let stage = dir.join("stage");
        std::fs::create_dir_all(&stage).unwrap();
        let pulse = stage.join("pulse");
        std::fs::write(
            &pulse,
            format!(
                "#!/bin/sh\ncase \"$1\" in version) echo '{{\"version\":\"{prints}\"}}';; help) exit 0;; *) exit 1;; esac\n"
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&pulse, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let name = pulse_cli::update::archive_name("v0.2.0", TARGET);
        let st = Command::new("tar")
            .arg("-czf")
            .arg(dir.join(&name))
            .arg("-C")
            .arg(&stage)
            .arg("pulse")
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
                "{digest}  {name}\n{}  pulse-v0.2.0-x86_64-apple-darwin.tar.gz\n",
                "1".repeat(64)
            ),
        )
        .unwrap();
        let doc = serde_json::json!({
            "tag_name": "v0.2.0",
            "html_url": format!("{base}/page"),
            "assets": [
                {"name": "pulse-v0.2.0-x86_64-apple-darwin.tar.gz", "browser_download_url": format!("{base}/missing.tar.gz")},
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
        let path = bin.join("pulse");
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
        let lines = outcome_lines(&out, "v0.1.0", Channel::Release);
        assert!(
            lines[0].starts_with("Installed PULSE v0.2.0 at "),
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
    /// cargo; a hosted brain holds no PULSE source to build from.
    #[test]
    fn self_update_release_mode_installs_the_release_and_never_runs_cargo() {
        let _serial = serial();
        let _env = pulse_testutil::env_lock();
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
        let _path = pulse_testutil::setenv(
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

    #[test]
    fn apply_refuses_without_a_source_or_on_source_and_a_stale_install_reads_failed() {
        let _serial = serial();
        let s = new_store();
        let target = ApplyTarget {
            binary: "/nonexistent/pulse".into(),
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
        s.set_setting("human", pulse_cli::update::CHANNEL_SETTING, "source")
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
