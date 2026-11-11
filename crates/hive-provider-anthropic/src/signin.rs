//! The Anthropic sign-in: Anthropic's own CLI (`ant`) holds the OAuth
//! profile, Hive only starts `ant auth login` and asks
//! `ant auth print-credentials --access-token` for a short-lived token.
//!
//! The CLI sits behind [`AntCli`] so the tests drive a fake one; the real
//! [`SystemCli`] spawns the binary with a timeout. A browser sign-in in
//! flight lives in a process-wide table keyed by its device code, so it
//! survives the hub opening the provider again between the start and a poll.

use anyhow::Result;
use hive_provider::{EnvFn, ProviderError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::NAME;

/// The environment variable that names the `ant` binary outright.
pub const ANT_BIN_ENV: &str = "HIVE_ANT_BIN";
/// The environment variable `ant` (and the SDKs) read to pick a profile;
/// the one in force at sign-in is recorded so the daemon keeps using it.
pub const PROFILE_ENV: &str = "ANTHROPIC_PROFILE";
/// The private file stem of the sign-in marker.
pub const MARKER_FILE: &str = "anthropic-signin";
/// How long a browser sign-in may take before it counts as expired.
pub const LOGIN_TTL: Duration = Duration::from_secs(600);
/// The install line the "not installed" sentence offers.
pub const INSTALL_HINT: &str =
    "brew install anthropics/tap/ant, or a release from github.com/anthropics/anthropic-cli";

/// Where the binary is looked for after `HIVE_ANT_BIN` and `PATH`: a
/// daemon started by launchd or systemd runs with a thin `PATH`.
const FALLBACK_DIRS: [&str; 2] = ["/opt/homebrew/bin", "/usr/local/bin"];

/// The marker of an Anthropic sign-in: which `ant` profile to ask (empty
/// means the active one) and the label the console shows. No token.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SignInMarker {
    #[serde(rename = "profile", default, skip_serializing_if = "String::is_empty")]
    pub profile: String,
    #[serde(rename = "user", default)]
    pub user: String,
    #[serde(rename = "saved_at", default)]
    pub saved_at: String,
}

impl SignInMarker {
    /// The label of a sign-in to `profile`.
    pub fn user_for(profile: &str) -> String {
        if profile.trim().is_empty() {
            "Anthropic Console".to_string()
        } else {
            format!("Anthropic Console (profile {})", profile.trim())
        }
    }
}

/// What one finished run of the CLI answered.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CliOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// A run of the CLI that may still be going (the browser sign-in).
pub trait CliChild: Send {
    /// The exit code once the run ended; `None` while it runs.
    fn try_wait(&mut self) -> Result<Option<i32>>;
    /// Everything the run printed to stdout so far.
    fn stdout(&self) -> String;
    /// Everything the run printed to stderr so far.
    fn stderr(&self) -> String;
    /// Stops the run; a run that already ended is left alone.
    fn kill(&mut self);
}

/// Anthropic's CLI as Hive uses it.
pub trait AntCli: Send + Sync {
    /// Starts `ant <args>` and returns at once.
    fn spawn(&self, args: &[String]) -> Result<Box<dyn CliChild>>;

    /// Runs `ant <args>` to its end, giving up after `timeout`.
    fn run(&self, args: &[String], timeout: Duration) -> Result<CliOutput> {
        let mut child = self.spawn(args)?;
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(code) = child.try_wait()? {
                return Ok(CliOutput {
                    code,
                    stdout: child.stdout(),
                    stderr: child.stderr(),
                });
            }
            if Instant::now() >= deadline {
                child.kill();
                return Err(ProviderError::new(
                    NAME,
                    0,
                    "cli_timeout",
                    format!(
                        "the Anthropic CLI (ant) did not answer within {} seconds",
                        timeout.as_secs().max(1)
                    ),
                    true,
                )
                .into());
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}

/// The error for a machine without the CLI.
pub fn cli_missing_error() -> ProviderError {
    ProviderError::new(
        NAME,
        0,
        "cli_missing",
        format!(
            "Signing in with Anthropic needs the Anthropic CLI (ant), and it isn't installed on this machine. Install it ({INSTALL_HINT}), or paste an API key instead"
        ),
        false,
    )
}

/// True when an error chain says the CLI is missing.
pub fn is_cli_missing(err: &anyhow::Error) -> bool {
    hive_provider::provider_error(err).is_some_and(|e| e.code == "cli_missing")
}

/// The real `ant` on this machine.
pub struct SystemCli {
    env: Option<EnvFn>,
}

impl SystemCli {
    pub fn new(env: Option<EnvFn>) -> Self {
        Self { env }
    }

    fn env(&self, key: &str) -> String {
        self.env
            .as_ref()
            .map(|f| f(key))
            .unwrap_or_else(|| std::env::var(key).unwrap_or_default())
    }

    /// `HIVE_ANT_BIN`, then `PATH`, then `~/.local/bin` and the Homebrew
    /// and `/usr/local` folders. `None` when no `ant` is found.
    pub fn binary(&self) -> Option<PathBuf> {
        let named = self.env(ANT_BIN_ENV);
        if !named.trim().is_empty() {
            let path = PathBuf::from(named.trim());
            return path.is_file().then_some(path);
        }
        let mut dirs: Vec<PathBuf> = std::env::split_paths(&self.env("PATH")).collect();
        let home = self.env("HOME");
        if !home.trim().is_empty() {
            dirs.push(PathBuf::from(home.trim()).join(".local").join("bin"));
        }
        dirs.extend(FALLBACK_DIRS.iter().map(PathBuf::from));
        dirs.into_iter()
            .filter(|d| !d.as_os_str().is_empty())
            .map(|d| d.join("ant"))
            .find(|p| p.is_file())
    }
}

impl AntCli for SystemCli {
    fn spawn(&self, args: &[String]) -> Result<Box<dyn CliChild>> {
        let bin = self.binary().ok_or_else(cli_missing_error)?;
        let mut child = Command::new(bin)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| {
                if err.kind() == std::io::ErrorKind::NotFound {
                    anyhow::Error::from(cli_missing_error())
                } else {
                    ProviderError::new(
                        NAME,
                        0,
                        "cli_failed",
                        format!("the Anthropic CLI (ant) did not start: {err}"),
                        false,
                    )
                    .into()
                }
            })?;
        let out = Arc::new(Mutex::new(String::new()));
        let err = Arc::new(Mutex::new(String::new()));
        let mut readers = Vec::new();
        if let Some(pipe) = child.stdout.take() {
            readers.push(drain(pipe, out.clone()));
        }
        if let Some(pipe) = child.stderr.take() {
            readers.push(drain(pipe, err.clone()));
        }
        Ok(Box::new(ProcessChild {
            child,
            out,
            err,
            readers,
            code: None,
        }))
    }
}

fn drain(mut pipe: impl Read + Send + 'static, into: Arc<Mutex<String>>) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match pipe.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => into
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push_str(&String::from_utf8_lossy(&buf[..n])),
            }
        }
    })
}

struct ProcessChild {
    child: Child,
    out: Arc<Mutex<String>>,
    err: Arc<Mutex<String>>,
    readers: Vec<JoinHandle<()>>,
    code: Option<i32>,
}

impl CliChild for ProcessChild {
    fn try_wait(&mut self) -> Result<Option<i32>> {
        if let Some(code) = self.code {
            return Ok(Some(code));
        }
        let Some(status) = self.child.try_wait()? else {
            return Ok(None);
        };
        // The pipes close with the process; the readers finish the output.
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
        let code = status.code().unwrap_or(-1);
        self.code = Some(code);
        Ok(Some(code))
    }

    fn stdout(&self) -> String {
        self.out.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    fn stderr(&self) -> String {
        self.err.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    fn kill(&mut self) {
        if self.code.is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.code = Some(-1);
        }
    }
}

impl Drop for ProcessChild {
    fn drop(&mut self) {
        self.kill();
    }
}

/// A browser sign-in in flight.
pub struct LoginRun {
    pub child: Box<dyn CliChild>,
    pub started: Instant,
    pub profile: String,
}

/// The browser sign-ins in flight, by device code.
pub static LOGIN_RUNS: LazyLock<Mutex<HashMap<String, LoginRun>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

static LOGIN_SEQ: AtomicU64 = AtomicU64::new(0);

/// A fresh device code for a browser sign-in: unguessable enough for a
/// handle the console only echoes back to this machine.
pub fn new_device_code() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let seq = LOGIN_SEQ.fetch_add(1, Ordering::SeqCst);
    format!("ant-{nanos:x}-{:x}-{seq:x}", std::process::id())
}

/// Stops and forgets the sign-ins older than [`LOGIN_TTL`].
pub fn prune_runs(runs: &mut HashMap<String, LoginRun>) {
    runs.retain(|_, run| {
        if run.started.elapsed() <= LOGIN_TTL {
            return true;
        }
        run.child.kill();
        false
    });
}

/// The first `https://` link in `text`, or empty.
pub fn first_url(text: &str) -> String {
    let Some(start) = text.find("https://") else {
        return String::new();
    };
    text[start..]
        .chars()
        .take_while(|c| !c.is_whitespace() && !matches!(c, '"' | '\'' | '<' | '>' | ')' | ']'))
        .collect::<String>()
        .trim_end_matches(['.', ','])
        .to_string()
}

/// The last few lines of a run's output, for an error sentence.
pub fn tail(text: &str) -> String {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let from = lines.len().saturating_sub(3);
    lines[from..].join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_url_takes_the_link_alone() {
        assert_eq!(
            first_url("Opening https://console.anthropic.com/oauth/authorize?x=1&y=2. Waiting"),
            "https://console.anthropic.com/oauth/authorize?x=1&y=2"
        );
        assert_eq!(first_url("<https://a.example/b>"), "https://a.example/b");
        assert_eq!(first_url("no link here"), "");
    }

    #[test]
    fn tail_keeps_the_last_three_lines() {
        assert_eq!(tail("a\n\nb\nc\nd\n"), "b c d");
        assert_eq!(tail(""), "");
    }

    #[test]
    fn marker_user_names_the_profile() {
        assert_eq!(SignInMarker::user_for(""), "Anthropic Console");
        assert_eq!(
            SignInMarker::user_for(" work "),
            "Anthropic Console (profile work)"
        );
    }

    #[test]
    fn binary_honours_the_override_and_reports_a_missing_one() {
        let dir = tempfile::tempdir().unwrap();
        let ant = dir.path().join("ant");
        std::fs::write(&ant, "#!/bin/sh\n").unwrap();
        let named = ant.to_string_lossy().into_owned();
        let cli = SystemCli::new(Some(Arc::new(move |k: &str| {
            if k == ANT_BIN_ENV {
                named.clone()
            } else {
                String::new()
            }
        })));
        assert_eq!(cli.binary(), Some(ant.clone()));
        let path = dir.path().to_string_lossy().into_owned();
        let cli = SystemCli::new(Some(Arc::new(move |k: &str| {
            if k == "PATH" {
                path.clone()
            } else {
                String::new()
            }
        })));
        assert_eq!(cli.binary(), Some(ant));
        let cli = SystemCli::new(Some(Arc::new(|k: &str| {
            if k == ANT_BIN_ENV {
                "/nonexistent/ant".to_string()
            } else {
                String::new()
            }
        })));
        assert_eq!(cli.binary(), None);
        let err = cli.spawn(&["auth".to_string()]).err().unwrap();
        assert!(is_cli_missing(&err));
    }

    #[cfg(unix)]
    #[test]
    fn system_cli_runs_a_script_and_times_out() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let ant = dir.path().join("ant");
        std::fs::write(
            &ant,
            "#!/bin/sh\nif [ \"$1\" = slow ]; then sleep 5; fi\necho \"out $*\"\necho err >&2\nexit 3\n",
        )
        .unwrap();
        std::fs::set_permissions(&ant, std::fs::Permissions::from_mode(0o755)).unwrap();
        let named = ant.to_string_lossy().into_owned();
        let cli = SystemCli::new(Some(Arc::new(move |k: &str| {
            if k == ANT_BIN_ENV {
                named.clone()
            } else {
                String::new()
            }
        })));
        let out = cli
            .run(
                &["auth".to_string(), "status".to_string()],
                Duration::from_secs(5),
            )
            .unwrap();
        assert_eq!(out.code, 3);
        assert_eq!(out.stdout.trim(), "out auth status");
        assert_eq!(out.stderr.trim(), "err");
        let err = cli
            .run(&["slow".to_string()], Duration::from_millis(200))
            .unwrap_err();
        assert_eq!(
            hive_provider::provider_error(&err).unwrap().code,
            "cli_timeout"
        );
    }
}
