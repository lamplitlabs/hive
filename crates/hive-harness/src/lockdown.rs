//! Lockdown (sandbox) mode: a conversation whose agent works with every
//! outside thing blocked. The agent still reads the machine, but it cannot
//! reach the network, cannot write outside its worktree (plus the temp and
//! build folders it needs to compile and test), and never sees credentials.
//! Only work inside the sandbox is possible: the tools that leave the
//! machine (`fetch`, `web_search`, MCP servers) are not offered, and every
//! command runs under the OS sandbox so a script the agent runs is bound the
//! same way as the agent itself.
//!
//! On macOS the sandbox is `/usr/bin/sandbox-exec` with a profile built by
//! [`profile`]; the shell of every command is wrapped by [`wrap_command`].
//! Elsewhere there is no OS sandbox to lean on: the harness still withdraws
//! the outside tools and refuses the commands [`outside_refusal`] names, and
//! [`os_enforced`] says which of the two the owner gets.
use std::path::{Path, PathBuf};

/// The environment variable set on every command of a lockdown session, so
/// a tool or a script can tell it runs sandboxed (`HIVE_LOCKDOWN=1`).
pub const LOCKDOWN_ENV: &str = "HIVE_LOCKDOWN";

/// The macOS sandbox runner.
pub const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// What a lockdown sandbox allows writes to and reads from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lockdown {
    /// The worktree the agent works in: the one place it writes.
    pub worktree: PathBuf,
    /// Folders beside the worktree the work needs to write: the shared
    /// Cargo target, the artifacts folder, the git common dir of a worktree
    /// (`.git/worktrees/<name>` and the object store live under the owner's
    /// checkout's `.git`), the store's SQLite files (`hive` commands write
    /// the WAL). Each is a directory or a file; missing ones are skipped.
    pub writable: Vec<PathBuf>,
    /// Folders the agent must not even read: credentials.
    pub unreadable: Vec<PathBuf>,
}

impl Lockdown {
    /// A lockdown around `worktree` with the usual extras: the temp folders,
    /// `/dev`, and the credential paths of `home` (the user's home) and
    /// `hive_home` denied for reading.
    pub fn new(worktree: impl Into<PathBuf>, home: &str, hive_home: &str) -> Self {
        let mut unreadable = Vec::new();
        if !home.is_empty() {
            let h = Path::new(home);
            unreadable.push(h.join(".ssh"));
            unreadable.push(h.join(".netrc"));
            unreadable.push(h.join(".aws"));
            unreadable.push(h.join(".config").join("gh"));
            unreadable.push(h.join(".copilot"));
            unreadable.push(h.join(".gnupg"));
        }
        if !hive_home.is_empty() {
            unreadable.push(Path::new(hive_home).join("private"));
        }
        Lockdown {
            worktree: worktree.into(),
            writable: Vec::new(),
            unreadable,
        }
    }

    /// Adds a folder or file the work may write (skipped when it does not
    /// exist at profile time, so a missing optional dir is no error).
    pub fn allow_write(&mut self, path: impl Into<PathBuf>) -> &mut Self {
        let p = path.into();
        if !p.as_os_str().is_empty() && !self.writable.contains(&p) {
            self.writable.push(p);
        }
        self
    }

    /// One line for the environment summary and the brief: what the
    /// sandbox binds.
    pub fn describe(&self) -> String {
        let enforced = if os_enforced() {
            "the OS sandbox"
        } else {
            "the harness (no OS sandbox on this system)"
        };
        format!(
            "lockdown: network off, writes confined to {} (and temp/build folders), credentials unreadable, no fetch/web_search/MCP; enforced by {enforced}",
            self.worktree.display()
        )
    }
}

/// Whether commands run under an OS sandbox here (macOS `sandbox-exec`).
pub fn os_enforced() -> bool {
    cfg!(target_os = "macos") && Path::new(SANDBOX_EXEC).is_file()
}

/// The real path the kernel sees (`/tmp` is `/private/tmp` on macOS); a path
/// that cannot be resolved is kept as given.
fn real(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// A string inside a profile's double quotes: the Scheme reader takes `\"`
/// and `\\`.
fn quote(p: &Path) -> String {
    let s = p.display().to_string();
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// The sandbox profile (Apple's Scheme-like SBPL): everything allowed but
/// the network (loopback stays, for a dev server or a test the work runs),
/// file writes (the worktree, the extras, the temp folders, `/dev`) and
/// reads of the credential paths. The temp folders are macOS's
/// `/private/var/folders` (what `TMPDIR` points under) and `/private/tmp`.
pub fn profile(l: &Lockdown) -> String {
    let mut p = String::new();
    p.push_str("(version 1)\n(allow default)\n");
    p.push_str("(deny network*)\n");
    p.push_str("(allow network* (local ip \"localhost:*\"))\n");
    p.push_str("(allow network-outbound (remote ip \"localhost:*\"))\n");
    p.push_str("(deny file-write*)\n");
    let mut dirs: Vec<PathBuf> = vec![real(&l.worktree)];
    for w in &l.writable {
        if w.exists() {
            dirs.push(real(w));
        }
    }
    dirs.push(PathBuf::from("/private/var/folders"));
    dirs.push(PathBuf::from("/private/tmp"));
    if let Ok(t) = std::env::var("TMPDIR")
        && !t.is_empty()
    {
        dirs.push(real(Path::new(&t)));
    }
    dirs.sort();
    dirs.dedup();
    for d in dirs {
        if d.is_file() {
            p.push_str(&format!("(allow file-write* (literal {}))\n", quote(&d)));
            // SQLite's -wal/-shm and an editor's temp beside a file.
            p.push_str(&format!(
                "(allow file-write* (regex #\"^{}[-.]\"))\n",
                regex_escape(&d.display().to_string())
            ));
        } else {
            p.push_str(&format!("(allow file-write* (subpath {}))\n", quote(&d)));
        }
    }
    p.push_str("(allow file-write* (regex #\"^/dev/\"))\n");
    for u in &l.unreadable {
        let u = real(u);
        p.push_str(&format!(
            "(deny file-read* (subpath {}))\n(deny file-read* (literal {}))\n",
            quote(&u),
            quote(&u)
        ));
    }
    p
}

fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if "\\.+*?()|[]{}^$#\"".contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// The shell command wrapped to run under the sandbox: `sandbox-exec -p
/// PROFILE /bin/sh -c COMMAND`, with the profile passed inline (a profile
/// file would need a place to write first). On a system without the OS
/// sandbox the command comes back as it was.
pub fn wrap_command(l: &Lockdown, command: &str) -> String {
    if !os_enforced() {
        return command.to_string();
    }
    format!(
        "{SANDBOX_EXEC} -p {} /bin/sh -c {}",
        shell_quote(&profile(l)),
        shell_quote(command)
    )
}

/// Single-quoted for `/bin/sh`.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Patterns of commands that leave the machine or the sandbox whatever the
/// OS does: refused outright in lockdown with the reason, before the OS
/// sandbox would fail them with a less helpful error. `curl`/`wget` to a
/// host that is not loopback, `ssh`/`scp`/`rsync` to a host, `git
/// push/fetch/pull/clone` of a remote, `hive land`, package publishing,
/// `sandbox-exec` itself (a nested profile cannot widen, but the command is
/// a tell), and `sudo`.
const OUTSIDE: &[(&str, &str)] = &[
    (
        r"(?i)(^|[;&|(]\s*)(curl|wget)\s+(?:.*\s)?(?:https?://|ftp://|)(?!(?:https?://)?(?:localhost|127\.0\.0\.1|\[::1\]|0\.0\.0\.0))[a-z0-9][a-z0-9.-]*\.[a-z]{2,}",
        "reaches a host outside this machine",
    ),
    (
        r"(?i)(^|[;&|(]\s*)(ssh|scp|sftp|rsync|telnet|nc|ncat|netcat)\s+(?:.*\s)?[a-z0-9._-]+@|(^|[;&|(]\s*)(ssh|scp|sftp|telnet)\s+(?:-\S+\s+)*[a-z0-9.-]+\.[a-z]{2,}",
        "opens a connection to another machine",
    ),
    (
        r"(?i)(^|[;&|(]\s*)git\s+(?:-C\s+\S+\s+)?(push|fetch|pull|clone|ls-remote|submodule\s+update)\b",
        "talks to a git remote",
    ),
    (
        r"(?i)(^|[;&|(]\s*)hive\s+(land|login|logout)\b",
        "lands or signs in from inside a lockdown",
    ),
    (
        r"(?i)(^|[;&|(]\s*)(cargo\s+publish|npm\s+publish|pnpm\s+publish|yarn\s+publish|gem\s+push|twine\s+upload|docker\s+push|gh\s+release)\b",
        "publishes outside",
    ),
    (
        r"(?i)(^|[;&|(]\s*)(sudo|doas|su)\s",
        "asks for another user's rights",
    ),
];

/// Why `command` is refused in lockdown, if it is: the reason to show the
/// agent. The OS sandbox stays the real wall; this is the plain-words one in
/// front of it, so the agent learns why instead of reading an EPERM.
pub fn outside_refusal(command: &str) -> Option<String> {
    use std::sync::OnceLock;
    static RULES: OnceLock<Vec<(regex::Regex, &'static str)>> = OnceLock::new();
    let rules = RULES.get_or_init(|| {
        OUTSIDE
            .iter()
            .filter_map(|(p, why)| {
                // Look-around is not in the regex crate: the curl rule is
                // compiled without it and the loopback hosts are checked
                // by hand below.
                let p = p.replace(
                    r"(?!(?:https?://)?(?:localhost|127\.0\.0\.1|\[::1\]|0\.0\.0\.0))",
                    "",
                );
                regex::Regex::new(&p).ok().map(|re| (re, *why))
            })
            .collect()
    });
    for (re, why) in rules {
        if re.is_match(command) {
            if why.starts_with("reaches a host") && only_loopback(command) {
                continue;
            }
            return Some(format!(
                "this is a lockdown (sandbox) session and the command {why}; only work inside the sandbox is possible: no network, no remotes, no publishing, no sign-in"
            ));
        }
    }
    None
}

/// True when every host a `curl`/`wget` command names is loopback.
fn only_loopback(command: &str) -> bool {
    let host_re = regex::Regex::new(r"(?i)(?:https?://|ftp://)?([a-z0-9][a-z0-9.-]*\.[a-z]{2,}|localhost|127\.0\.0\.1|\[::1\]|0\.0\.0\.0)(?::\d+)?(?:/|\s|$)")
        .expect("host regex");
    let mut saw = false;
    for cap in host_re.captures_iter(command) {
        let h = cap[1].to_ascii_lowercase();
        saw = true;
        if !matches!(h.as_str(), "localhost" | "127.0.0.1" | "[::1]" | "0.0.0.0") {
            return false;
        }
    }
    saw
}

/// The environment lines a lockdown session's commands carry: the marker,
/// git kept from prompting or signing through an agent outside the sandbox
/// (the owner's 1Password or gpg-agent socket is outside it), and the proxy
/// variables cleared so a tool does not wait on one.
pub fn env_lines() -> Vec<String> {
    vec![
        format!("{LOCKDOWN_ENV}=1"),
        "GIT_TERMINAL_PROMPT=0".to_string(),
        "GIT_CONFIG_COUNT=2".to_string(),
        "GIT_CONFIG_KEY_0=commit.gpgsign".to_string(),
        "GIT_CONFIG_VALUE_0=false".to_string(),
        "GIT_CONFIG_KEY_1=tag.gpgsign".to_string(),
        "GIT_CONFIG_VALUE_1=false".to_string(),
        "HTTP_PROXY=".to_string(),
        "HTTPS_PROXY=".to_string(),
        "http_proxy=".to_string(),
        "https_proxy=".to_string(),
        "ALL_PROXY=".to_string(),
        "NO_PROXY=*".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_names_the_worktree_the_extras_and_the_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let wt = dir.path().join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        let target = dir.path().join("target");
        std::fs::create_dir_all(&target).unwrap();
        let db = dir.path().join("hive.sqlite");
        std::fs::write(&db, b"").unwrap();
        let mut l = Lockdown::new(&wt, "/Users/someone", "/Users/someone/.hive");
        l.allow_write(&target)
            .allow_write(&db)
            .allow_write(dir.path().join("missing"));
        let p = profile(&l);
        assert!(
            p.starts_with("(version 1)\n(allow default)\n(deny network*)\n"),
            "{p}"
        );
        assert!(p.contains("(deny file-write*)"), "{p}");
        let wt_real = real(&wt).display().to_string();
        assert!(
            p.contains(&format!("(allow file-write* (subpath \"{wt_real}\"))")),
            "{p}"
        );
        assert!(
            p.contains(&format!(
                "(allow file-write* (subpath \"{}\"))",
                real(&target).display()
            )),
            "{p}"
        );
        assert!(
            p.contains(&format!(
                "(allow file-write* (literal \"{}\"))",
                real(&db).display()
            )),
            "the store file is writable: {p}"
        );
        assert!(!p.contains("missing"), "a missing extra is skipped: {p}");
        assert!(
            p.contains("(allow file-write* (subpath \"/private/tmp\"))"),
            "{p}"
        );
        assert!(
            p.contains("(deny file-read* (subpath \"/Users/someone/.ssh\"))"),
            "{p}"
        );
        assert!(
            p.contains("(deny file-read* (subpath \"/Users/someone/.hive/private\"))"),
            "{p}"
        );
        assert!(
            p.contains("(allow network* (local ip \"localhost:*\"))"),
            "{p}"
        );
    }

    #[test]
    fn quoting_escapes_what_the_profile_reader_takes() {
        assert_eq!(quote(Path::new("/a/b")), "\"/a/b\"");
        assert_eq!(quote(Path::new("/a/\"b\\c")), "\"/a/\\\"b\\\\c\"");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(regex_escape("/a.b+c"), "/a\\.b\\+c");
    }

    #[test]
    fn outside_commands_are_named_and_loopback_is_not() {
        for cmd in [
            "curl https://example.com",
            "wget -q http://pkg.example.org/x.tgz",
            "cd x && curl -s api.github.com/repos",
            "ssh user@host.example.com ls",
            "scp a.txt deploy@10.0.0.1:/tmp",
            "git push origin main",
            "git fetch origin",
            "git -C /x pull",
            "git clone https://github.com/a/b",
            "hive land --project p --branch b",
            "cargo publish",
            "npm publish --access public",
            "sudo rm -rf /",
        ] {
            let why = outside_refusal(cmd);
            assert!(why.is_some(), "{cmd:?} should be refused");
            assert!(why.unwrap().contains("lockdown"), "{cmd:?}");
        }
        for cmd in [
            "curl -s http://localhost:3000/",
            "curl http://127.0.0.1:8787/api/v1/org",
            "cargo test -p hive-store",
            "git status && git commit -m x",
            "git log --oneline -3",
            "hive status",
            "hive ask --all --no-model \"lockdown\"",
            "ls ~/.hive",
            "echo ssh is a word",
        ] {
            assert!(outside_refusal(cmd).is_none(), "{cmd:?} should run");
        }
    }

    #[test]
    fn env_lines_mark_the_session_and_disarm_signing() {
        let env = env_lines();
        assert!(env.contains(&"HIVE_LOCKDOWN=1".to_string()));
        assert!(env.contains(&"GIT_CONFIG_KEY_0=commit.gpgsign".to_string()));
        assert!(env.contains(&"GIT_CONFIG_VALUE_0=false".to_string()));
        assert!(env.contains(&"HTTPS_PROXY=".to_string()));
    }

    #[test]
    fn wrap_command_goes_through_sandbox_exec_where_there_is_one() {
        let dir = tempfile::tempdir().unwrap();
        let l = Lockdown::new(dir.path(), "", "");
        let wrapped = wrap_command(&l, "echo 'hi there'");
        if os_enforced() {
            assert!(
                wrapped.starts_with(&format!("{SANDBOX_EXEC} -p '(version 1)")),
                "{wrapped}"
            );
            assert!(
                wrapped.ends_with(" /bin/sh -c 'echo '\\''hi there'\\'''"),
                "{wrapped}"
            );
        } else {
            assert_eq!(wrapped, "echo 'hi there'");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_sandbox_binds_writes_and_the_network_on_macos() {
        if !os_enforced() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let wt = dir.path().join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        // The tempdir itself is under TMPDIR, which the profile allows, so
        // the "outside" probe uses the home directory instead.
        let home = std::env::var("HOME").unwrap_or_default();
        let mut l = Lockdown::new(&wt, &home, "");
        l.unreadable.clear();
        let run = |cmd: &str| {
            let out = std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(wrap_command(&l, cmd))
                .current_dir(&wt)
                .env("TMPDIR", dir.path())
                .output()
                .unwrap();
            (
                out.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&out.stdout).to_string()
                    + &String::from_utf8_lossy(&out.stderr),
            )
        };
        let (rc, out) = run("echo inside > here.txt && cat here.txt");
        assert_eq!(rc, 0, "{out}");
        assert_eq!(out.trim(), "inside");
        if !home.is_empty() {
            let probe = Path::new(&home).join(".hive-lockdown-probe-should-not-exist");
            let (rc, out) = run(&format!("echo x > {}", probe.display()));
            let _ = std::fs::remove_file(&probe);
            assert_ne!(rc, 0, "a write to the home went through: {out}");
            assert!(out.contains("Operation not permitted"), "{out}");
        }
        let (rc, _) = run("curl -s -m 5 https://example.com/");
        assert_ne!(rc, 0, "the network was reachable");
    }
}
