//! The lease path between the brain and a hand (ADR-070 step 1c): how one
//! `hive-hands` verb reaches a machine - the `b64` command line sshd's
//! forced command reads, the shared ssh connection it travels on, the
//! retries a connect that never began earns, and the local `hive-hand`
//! pipe (`executor.transport local-hand`) that stands in for sshd on this
//! machine. The brain (`hive-runner`'s `executor`) re-exports these; a hand
//! needs them to answer the same wire format and nothing of the brain.

use anyhow::Result;
use base64::Engine;
use regex::Regex;
use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock, RwLock};
use std::time::Duration;

use crate::ctx::{CancellationToken, ContextError};

pub const SSH_EXIT_UNREACH: i32 = 255;
/// How many more times a session that never began is tried, and the first
/// pause between tries (doubled each time: 2, 4, 8, 16, 32 s, a minute in
/// all). Measured 2026-11-05: the hub's carrier-grade NAT refused bursts of
/// fresh ssh connections for 1 to 14 s at a time; one refusal between the
/// inspection and the apply failed CR-063's pairing and released a machine
/// that had just booted, as CR-055..059 went before it, and three in a row
/// retired hands-cr-062. The burst after a daemon restart is longer: the
/// shared connection's master dies with the daemon and every probe, lead
/// and worker the new daemon starts opens a fresh connection at once, which
/// the NAT refused for 30 to 55 s after the 03:12, 06:14 and 07:03 restarts
/// that day; four tries (30 s) ran out inside it and failed the lead's task
/// three times on a healthy hands-cr-062, so the fifth try carries the
/// window past a minute. A connect that fails runs nothing on the far side,
/// so trying it again repeats no work.
pub const SSH_CONNECT_RETRIES: u32 = 5;
/// The first pause between connect tries, doubled after each.
pub const SSH_CONNECT_PAUSE: Duration = Duration::from_secs(2);

pub static SSH_OPTIONS: &[&str] = &[
    "-o",
    "BatchMode=yes",
    "-o",
    "ConnectTimeout=15",
    "-o",
    "ClearAllForwardings=yes",
];
/// Seconds a shared connection stays open past its last session; the
/// daemon's five-minute tick keeps a machine in use inside it.
pub const SSH_SHARED_PERSIST_SECONDS: u32 = 600;
/// The longest socket path a shared connection is given; a unix socket path
/// is bound at 104 bytes on macOS, and ssh's `%C` hash takes 40 of them.
pub const SSH_SOCKET_DIR_MAX: usize = 60;

/// `HIVE_HOME`, else `~/.hive`, absolute and resolved: the same answer as
/// the store's `resolve_home("")`, without linking the store. An install from
/// before the rename still answers `PULSE_HOME` or `~/.pulse` while no
/// `~/.hive` exists.
pub fn hive_home() -> Option<String> {
    let mut home = std::env::var("HIVE_HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var("PULSE_HOME").ok().filter(|v| !v.is_empty()))
        .unwrap_or_default();
    if home.is_empty() {
        home = default_home_in(&std::env::home_dir()?)
            .display()
            .to_string();
    }
    if home.starts_with('~') {
        let user = std::env::home_dir()?;
        let rest = home
            .trim_start_matches('~')
            .trim_start_matches(std::path::MAIN_SEPARATOR);
        home = user.join(rest).display().to_string();
    }
    let p = std::path::PathBuf::from(&home);
    let abs = p.canonicalize().ok().or_else(|| {
        if p.is_absolute() {
            Some(p.clone())
        } else {
            std::env::current_dir().ok().map(|cwd| cwd.join(&p))
        }
    })?;
    Some(abs.display().to_string())
}

/// `~/.hive`, or `~/.pulse` while that is the only one of the two that
/// exists (an install from before the rename). The store's `default_home_in`
/// gives the same answer; the hand links this crate and not the store.
pub fn default_home_in(user: &std::path::Path) -> std::path::PathBuf {
    let current = user.join(".hive");
    let legacy = user.join(".pulse");
    if !current.exists() && legacy.is_dir() {
        return legacy;
    }
    current
}

/// The directory of the shared connections' sockets, `HIVE_HOME/ssh`,
/// made on the way; None when no home resolves or the path would be too
/// long for a socket, in which case every session connects on its own.
pub fn ssh_socket_dir() -> Option<String> {
    let home = hive_home()?;
    let dir = format!("{}/ssh", home.trim_end_matches('/'));
    if dir.len() > SSH_SOCKET_DIR_MAX {
        return None;
    }
    std::fs::create_dir_all(&dir).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    Some(dir)
}

/// Removes the shared connections' sockets, for a daemon that starts: a
/// master from before a reboot is gone and its socket would turn every
/// session to that machine into a single connection with a warning until
/// someone deleted it; a master that still runs ends itself once idle.
pub fn ssh_sockets_reset() -> usize {
    let Some(dir) = ssh_socket_dir() else {
        return 0;
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| std::fs::remove_file(e.path()).is_ok())
        .count()
}

/// The options every ssh command line to a machine starts with. Every
/// session to one address (user, host, port) shares one connection: the
/// first opens a master that stays [`SSH_SHARED_PERSIST_SECONDS`] past its
/// last session, the rest attach through its socket under
/// [`ssh_socket_dir`]; a session whose master cannot be reached connects
/// on its own, as before. Measured 2026-11-05 on the hub: of twenty fresh
/// connects to hands-cr-064 in 77 s, the carrier-grade NAT the hub sits
/// behind answered eighteen with a reset before the packet left its
/// network (a capture on the machine saw one SYN arrive and sent no
/// reset), and the refusals went on for four minutes after a pairing's
/// twenty-odd short sessions, while the connection that was up kept
/// working. One connection per machine is what the NAT allows for; a
/// probe, a job start, a steps mirror and a fetch then cost no connect at
/// all.
pub fn ssh_options() -> Vec<String> {
    let mut opts: Vec<String> = SSH_OPTIONS.iter().map(|s| s.to_string()).collect();
    if let Some(dir) = ssh_socket_dir() {
        opts.extend(
            [
                "ControlMaster=auto".to_string(),
                format!("ControlPersist={SSH_SHARED_PERSIST_SECONDS}"),
                format!("ControlPath={dir}/%C"),
                "ServerAliveInterval=30".to_string(),
                "ServerAliveCountMax=3".to_string(),
            ]
            .into_iter()
            .flat_map(|o| ["-o".to_string(), o]),
        );
    }
    opts
}

static SSH_ADDRESS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([A-Za-z0-9._-]+)@([A-Za-z0-9._-]+|\[[0-9A-Fa-f:.]+\])(?::([0-9]{1,5}))?$")
        .unwrap()
});

/// A machine the transport could not reach: ssh never got a session, the
/// verb timed out, or no `hive-hand` is installed for the local pipe.
#[derive(Debug, Clone)]
pub struct ExecutorUnreachable {
    pub msg: String,
}
impl std::fmt::Display for ExecutorUnreachable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.msg)
    }
}
impl std::error::Error for ExecutorUnreachable {}

/// The last `n` characters of `s`.
pub fn tail(s: &str, n: usize) -> String {
    let chars = s.chars().collect::<Vec<_>>();
    if chars.len() > n {
        chars[chars.len() - n..].iter().collect()
    } else {
        s.to_string()
    }
}

/// `user@host[:port]` into its parts; port 22 when none is given.
pub fn parse_ssh_address(text: &str) -> Result<(String, String, i32)> {
    let trimmed = text.trim();
    let Some(caps) = SSH_ADDRESS_RE.captures(trimmed) else {
        return Err(crate::error::errorf(
            2,
            &format!("ssh address must be user@host[:port], got {text:?}"),
            "",
            "",
        )
        .into());
    };
    let port = caps
        .get(3)
        .map(|m| m.as_str().parse::<i32>().unwrap_or(22))
        .unwrap_or(22);
    if port <= 0 || port >= 65536 {
        return Err(
            crate::error::errorf(2, &format!("ssh port out of range in {text:?}"), "", "").into(),
        );
    }
    Ok((caps[1].to_string(), caps[2].to_string(), port))
}

/// The `b64 ...` command line sshd's forced command hands `hive-hands`:
/// the verb and its arguments, each NUL-terminated, base64 in one word.
pub fn hands_command_line(args: &[String]) -> String {
    let mut payload = Vec::new();
    for a in args {
        payload.extend_from_slice(a.as_bytes());
        payload.push(0);
    }
    format!(
        "b64 {}",
        base64::engine::general_purpose::STANDARD.encode(payload)
    )
}

/// The ssh command line for one hands verb at `address` (`user@host[:port]`).
pub fn ssh_command(address: &str, args: &[String]) -> Result<Vec<String>> {
    let (user, host, port) = parse_ssh_address(address)?;
    let mut cmd = vec!["ssh".to_string()];
    cmd.extend(ssh_options());
    cmd.extend([
        "-p".to_string(),
        port.to_string(),
        format!("{user}@{host}"),
        hands_command_line(args),
    ]);
    Ok(cmd)
}

/// The ssh runner tests swap for a fake. Tests install closures that
/// capture state, so the hook is a shared trait object rather than a
/// plain fn pointer.
pub type SshRunner = Arc<
    dyn Fn(&CancellationToken, &[String], &[u8], Duration) -> Result<(i32, String, String)>
        + Send
        + Sync,
>;
pub static RUN_SSH: LazyLock<RwLock<SshRunner>> =
    LazyLock::new(|| RwLock::new(Arc::new(run_process)));

/// run_ssh_hook returns the current `runSSH` implementation.
pub fn run_ssh_hook() -> SshRunner {
    RUN_SSH.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// set_run_ssh installs `f` as `runSSH` and returns the previous hook so a
/// test can restore it.
pub fn set_run_ssh<F>(f: F) -> SshRunner
where
    F: Fn(&CancellationToken, &[String], &[u8], Duration) -> Result<(i32, String, String)>
        + Send
        + Sync
        + 'static,
{
    std::mem::replace(
        &mut *RUN_SSH.write().unwrap_or_else(|e| e.into_inner()),
        Arc::new(f),
    )
}

/// restore_run_ssh puts a hook returned by [`set_run_ssh`] or [`run_ssh_hook`] back.
pub fn restore_run_ssh(old: SshRunner) {
    *RUN_SSH.write().unwrap_or_else(|e| e.into_inner()) = old;
}

/// The first transport to an open hand (ADR-070 step 1): the brain drives a
/// `hive-hand` binary on this machine over a pipe, exactly as it drives
/// `hive hands` behind sshd - one verb per call, the `b64 ...` (or
/// `git-upload-pack ...`) command line in `SSH_ORIGINAL_COMMAND`, the
/// payload on stdin, the verb's exit code and output back. `env` is the
/// hand's own environment (its workdir, home, PATH) on top of this
/// process's, without the brain's HIVE_JOB/HIVE_AGENT/HIVE_SESSION. The
/// dial-out TLS channel replaces the pipe in step 2; `executor.transport
/// local-hand` picks this transport for this machine's own work
/// (`hive-runner`'s `transport_row`, [`HAND_KIND`]).
pub fn local_hand_runner(hand: std::path::PathBuf, env: Vec<(String, String)>) -> SshRunner {
    Arc::new(
        move |cancel: &CancellationToken, argv: &[String], stdin: &[u8], timeout: Duration| {
            let mut vars: BTreeMap<String, String> = std::env::vars()
                .filter(|(k, _)| !matches!(k.as_str(), "HIVE_JOB" | "HIVE_AGENT" | "HIVE_SESSION"))
                .collect();
            vars.extend(env.iter().cloned());
            vars.insert(
                "SSH_ORIGINAL_COMMAND".into(),
                argv.last().cloned().unwrap_or_default(),
            );
            let environ: Vec<String> = vars.into_iter().map(|(k, v)| format!("{k}={v}")).collect();
            let out = crate::proc::run_group(
                cancel,
                &[hand.display().to_string()],
                "",
                &environ,
                stdin,
                timeout,
            )?;
            if out.timeout {
                if cancel.is_cancelled() {
                    return Err(ContextError::Canceled.into());
                }
                return Err(ContextError::DeadlineExceeded.into());
            }
            Ok((out.exit_code, out.stdout, out.stderr))
        },
    )
}

/// The kind of an executor row that is this machine's own `hive-hand`
/// driven over a pipe (`executor.transport local-hand`): the row keeps the
/// local executor's name, so its slots and accounting stay this machine's,
/// and every hands verb goes to the binary instead of through sshd.
pub const HAND_KIND: &str = "hand";

/// The setting that routes this machine's own work through its `hive-hand`.
pub const TRANSPORT_SETTING: &str = "executor.transport";
/// [`TRANSPORT_SETTING`]'s value for the local `hive-hand` pipe.
pub const TRANSPORT_LOCAL_HAND: &str = "local-hand";

/// The `hive-hand` binary a [`HAND_KIND`] row drives: the one installed
/// beside this `hive`, else the first on PATH.
pub fn local_hand_binary() -> Option<std::path::PathBuf> {
    let name = format!("hive-hand{}", std::env::consts::EXE_SUFFIX);
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join(&name)))
        .filter(|p| p.is_file())
        .or_else(|| which::which("hive-hand").ok())
}

/// What tests put in place of the installed `hive-hand`: the binary and
/// the environment it runs in on top of the row's own.
pub type HandOverride = (std::path::PathBuf, Vec<(String, String)>);
pub static LOCAL_HAND: LazyLock<RwLock<Option<HandOverride>>> = LazyLock::new(|| RwLock::new(None));

/// Installs `hand` as the local `hive-hand` and returns what was there.
pub fn set_local_hand(hand: Option<HandOverride>) -> Option<HandOverride> {
    std::mem::replace(
        &mut *LOCAL_HAND.write().unwrap_or_else(|e| e.into_inner()),
        hand,
    )
}

/// One hands verb on a [`HAND_KIND`] row whose workdir is `workdir`: the
/// `b64` command line the sshd forced command would get, run by the local
/// `hive-hand`. A pipe has no `authorized_keys` to check, so the hand is
/// told its keys are fine.
pub fn hand_run(
    cancel: &CancellationToken,
    workdir: &str,
    args: &[String],
    stdin: &[u8],
    timeout: Duration,
) -> Result<(i32, String, String)> {
    let (bin, extra) = match LOCAL_HAND.read().unwrap_or_else(|e| e.into_inner()).clone() {
        Some(v) => v,
        None => match local_hand_binary() {
            Some(bin) => (bin, Vec::new()),
            None => {
                return Err(ExecutorUnreachable {
                    msg: format!(
                        "{TRANSPORT_SETTING} is {TRANSPORT_LOCAL_HAND} but no hive-hand is installed beside hive or on PATH (cargo build -p hive-hand --release)"
                    ),
                }
                .into());
            }
        },
    };
    let mut env = vec![
        ("HIVE_HANDS_WORKDIR".to_string(), workdir.to_string()),
        ("HIVE_HANDS_KEYS_OK".to_string(), "yes".to_string()),
    ];
    // The worker the hand runs is `hive harness run`: this hive, found
    // first on the hand's PATH even when the daemon's own PATH lacks it.
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf))
    {
        let mut dirs = vec![dir];
        dirs.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        if let Ok(path) = std::env::join_paths(dirs) {
            env.push(("PATH".to_string(), path.to_string_lossy().into_owned()));
        }
    }
    env.extend(extra);
    let argv = vec![bin.display().to_string(), hands_command_line(args)];
    match local_hand_runner(bin, env)(cancel, &argv, stdin, timeout) {
        Err(err) if !cancel.is_cancelled() && err.to_string().contains("deadline exceeded") => {
            let verb = args.first().map(String::as_str).unwrap_or("");
            Err(ExecutorUnreachable {
                msg: format!("hive-hand {verb} timed out after {}s", timeout.as_secs()),
            }
            .into())
        }
        other => other,
    }
}

/// Whether ssh's stderr says the session never began: the connect itself
/// failed (refused, timed out, no route, network unreachable), the
/// banner exchange was cut before any authentication, or the shared
/// connection's master took the session request and could not serve it
/// (the master's connection died under it, or the machine's sshd has its
/// ten sessions per connection open, which a later try finds freed).
/// Everything after that (a host key change, a refused key, the remote
/// command's own exit) is a different failure that a second try would
/// only repeat.
pub fn ssh_connect_failed(err_text: &str) -> bool {
    [
        "ssh: connect to host ",
        "kex_exchange_identification:",
        "ssh_exchange_identification:",
        "banner exchange",
        "mux_client_request_session:",
        "mux_client_hello_exchange:",
        "mux_client_read_exactly:",
        "mux_client_request_alive:",
    ]
    .iter()
    .any(|mark| err_text.contains(mark))
}

/// Runs an ssh command line through the hook, trying again while the
/// session cannot begin ([`ssh_connect_failed`]): up to
/// [`SSH_CONNECT_RETRIES`] more tries with a doubling pause, which a cancel
/// cuts short. Returns the last try's result unchanged, so the callers'
/// exit-255 handling reads as before.
pub fn run_ssh_connecting(
    cancel: &CancellationToken,
    argv: &[String],
    stdin: &[u8],
    timeout: Duration,
) -> Result<(i32, String, String)> {
    run_ssh_connecting_paused(cancel, argv, stdin, timeout, SSH_CONNECT_PAUSE)
}

/// [`run_ssh_connecting`] with `pause` as the first pause between tries
/// (a test suite takes it down to a millisecond).
pub fn run_ssh_connecting_paused(
    cancel: &CancellationToken,
    argv: &[String],
    stdin: &[u8],
    timeout: Duration,
    first_pause: Duration,
) -> Result<(i32, String, String)> {
    let runner = run_ssh_hook();
    let mut pause = first_pause;
    let mut tries = 0;
    loop {
        let res = runner(cancel, argv, stdin, timeout);
        let again = matches!(
            &res,
            Ok((rc, _, err_text)) if *rc == SSH_EXIT_UNREACH && ssh_connect_failed(err_text)
        ) && tries < SSH_CONNECT_RETRIES
            && !cancel.is_cancelled();
        if !again {
            return res;
        }
        tries += 1;
        let _ = cancel.done().recv_timeout(pause);
        pause *= 2;
    }
}

/// The default `runSSH`: the command line in its own process group, its
/// exit code and output back; a timeout is a context error.
pub fn run_process(
    cancel: &CancellationToken,
    argv: &[String],
    stdin: &[u8],
    timeout: Duration,
) -> Result<(i32, String, String)> {
    let out =
        match crate::proc::run_group(cancel, argv, "", &crate::proc::environ(), stdin, timeout) {
            Ok(o) => o,
            Err(e) => {
                if e.downcast_ref::<std::io::Error>()
                    .is_some_and(|ioe| ioe.kind() == std::io::ErrorKind::NotFound)
                {
                    return Err(
                        std::io::Error::new(std::io::ErrorKind::NotFound, "not found").into(),
                    );
                }
                return Err(e);
            }
        };
    if out.timeout {
        if cancel.is_cancelled() {
            return Err(ContextError::Canceled.into());
        }
        return Err(ContextError::DeadlineExceeded.into());
    }
    Ok((out.exit_code, out.stdout, out.stderr))
}

/// The `key=value` words of a probe's answer.
pub fn parse_probe(text: &str) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::new();
    for tok in text.split_whitespace() {
        if let Some((k, v)) = tok.split_once('=') {
            fields.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_command_wraps_the_verb_in_one_b64_word() {
        let argv =
            ssh_command("hive@hands-one:2222", &["count".into(), "JOB-1".into()]).expect("argv");
        let n = argv.len();
        assert_eq!(argv[0], "ssh");
        assert_eq!(&argv[n - 4..n - 1], ["-p", "2222", "hive@hands-one"]);
        let b64 = argv[n - 1].strip_prefix("b64 ").expect("b64 word");
        let raw = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .expect("base64");
        assert_eq!(raw, b"count\0JOB-1\0");
        assert!(ssh_command("no-at-sign", &[]).is_err());
        assert!(parse_ssh_address("a@b:70000").is_err());
        assert_eq!(
            parse_ssh_address("a@[::1]").unwrap(),
            ("a".into(), "[::1]".into(), 22)
        );
    }

    #[test]
    fn parse_probe_reads_key_value_words() {
        let got = parse_probe("hands=1 git=yes\nfree_mb=12 noise");
        assert_eq!(got.get("hands").map(String::as_str), Some("1"));
        assert_eq!(got.get("free_mb").map(String::as_str), Some("12"));
        assert_eq!(got.len(), 3);
    }

    #[test]
    fn a_missing_hive_hand_is_unreachable_not_a_crash() {
        let _serial = crate::test_helpers::serial();
        let old = set_local_hand(Some((
            std::path::PathBuf::from("/nonexistent/hive-hand"),
            Vec::new(),
        )));
        let res = hand_run(
            &CancellationToken::new(),
            "/tmp",
            &["probe".into()],
            &[],
            Duration::from_secs(5),
        );
        set_local_hand(old);
        assert!(res.is_err(), "{res:?}");
    }
}
