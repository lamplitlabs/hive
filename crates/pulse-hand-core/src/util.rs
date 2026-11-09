//! The small helpers the executor path needs on both sides: git with the
//! hermetic environment and a real timeout, shell quoting, the time texts.
//! pulse-runner's `util` re-exports these, so the brain and the hand run the
//! very same git.

use anyhow::{Result, anyhow};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;
use std::sync::LazyLock;
use std::time::Duration;

pub fn trunc(text: &str, n: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() > n && n > 0 {
        chars[..n - 1].iter().collect::<String>() + "~"
    } else {
        text.to_string()
    }
}

pub fn plural(n: i32, word: &str) -> String {
    if n == 1 {
        word.into()
    } else {
        format!("{word}s")
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitResult {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
    #[serde(skip)]
    pub err: Option<String>,
}
impl GitResult {
    pub fn ok(&self) -> bool {
        self.err.is_none() && self.code == 0
    }
}

pub fn git_timeout(repo: &str, timeout: Duration, args: &[&str]) -> GitResult {
    git_timeout_env(repo, timeout, &[], args)
}

/// Settings every git the runner spawns through [`git_timeout_env`] runs
/// with, as `-c` pairs ahead of `-C` and the subcommand. `core.fsmonitor`
/// is off so an owner's repo-local `core.fsmonitor = true` cannot stall
/// PULSE on a filesystem monitor daemon that never answers its cookie
/// (refresh_fsmonitor -> fsmonitor_ipc__send_query hung a landing's
/// `merge --ff-only` for three hours). Command-line `-c`, not
/// GIT_CONFIG_COUNT: the harness lockdown already exports
/// GIT_CONFIG_COUNT/KEY_n/VALUE_n for the signing settings, and a second
/// writer of that list would overwrite them. A caller's own `-c` comes
/// later on the line and still wins.
pub const GIT_FORCED_CONFIG: &[&str] = &["-c", "core.fsmonitor=false"];

/// The argv (without the program) [`git_timeout_env`] runs.
pub fn git_argv(repo: &str, args: &[&str]) -> Vec<String> {
    let mut full: Vec<String> = GIT_FORCED_CONFIG.iter().map(|s| s.to_string()).collect();
    if !repo.is_empty() {
        full.extend(["-C".into(), repo.into()]);
    }
    full.extend(args.iter().map(|s| s.to_string()));
    full
}

/// Runs git with the hermetic environment plus `env`, bounded by `timeout`
/// (zero: unbounded). Past the timeout git and everything it started (hooks,
/// helpers, an fsmonitor client) is killed by process group and the result
/// carries `err` naming the timeout, so a caller's budget is a real one.
pub fn git_timeout_env(repo: &str, timeout: Duration, env: &[String], args: &[&str]) -> GitResult {
    let full = git_argv(repo, args);
    let mut cmd = Command::new(pulse_gitenv::git_program());
    cmd.args(&full);
    // Strip the hook-exported variables that redirect this command at the
    // repository of a concurrent pulse job's pre-commit hook (GIT_DIR,
    // GIT_INDEX_FILE, GIT_QUARANTINE_PATH, ...). `.envs` alone does not remove
    // an inherited variable, so without this the recipe read in
    // pulse-inspect::recipes flaked whenever another job ran git. We remove
    // only the redirecting vars rather than clearing the whole environment so
    // that intentional ambient credentials (e.g. GIT_SSH_COMMAND) still apply.
    for kv in pulse_gitenv::HOOK_VARS {
        if let Some(var) = kv.strip_suffix('=') {
            cmd.env_remove(var);
        }
    }
    cmd.envs(git_env(env).into_iter().filter_map(|kv| {
        kv.split_once('=')
            .map(|(k, v)| (k.to_string(), v.to_string()))
    }));
    match crate::proc::output_within(&mut cmd, timeout) {
        Ok(o) if o.timed_out => GitResult {
            code: o.status.and_then(|st| st.code()).unwrap_or(-1),
            stdout: o.stdout,
            stderr: o.stderr,
            err: Some(format!(
                "git {} timed out after {} in {}; killed",
                trunc(&args.join(" "), 120),
                format_duration(timeout),
                if repo.is_empty() { "." } else { repo }
            )),
        },
        Ok(o) => GitResult {
            code: o.status.and_then(|st| st.code()).unwrap_or(-1),
            stdout: o.stdout,
            stderr: o.stderr,
            err: None,
        },
        Err(e) => GitResult {
            code: -1,
            err: Some(e.to_string()),
            ..GitResult::default()
        },
    }
}

pub fn git_env(extra: &[String]) -> Vec<String> {
    let mut e = pulse_gitenv::env();
    e.extend_from_slice(extra);
    e
}
pub fn strip_hook_git_env(env: &[String]) -> Vec<String> {
    pulse_gitenv::strip_hook(env.to_vec())
}
pub fn git(repo: &str, args: &[&str]) -> GitResult {
    git_timeout(repo, Duration::from_secs(60), args)
}
pub fn git_check(repo: &str, args: &[&str]) -> Result<GitResult> {
    let r = git(repo, args);
    if r.ok() {
        Ok(r)
    } else {
        Err(anyhow!(
            "git {} failed in {}: {}",
            args.join(" "),
            repo,
            r.stderr.trim()
        ))
    }
}
pub fn git_branch(repo: &str) -> String {
    git_timeout(
        repo,
        Duration::from_secs(10),
        &["rev-parse", "--abbrev-ref", "HEAD"],
    )
    .stdout
    .trim()
    .to_string()
}

pub fn file_exists(path: &str) -> bool {
    Path::new(path).exists()
}

static SAFE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9@%+=:,./_-]+$").unwrap());
pub fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        "''".into()
    } else if SAFE.is_match(s) {
        s.into()
    } else {
        format!("'{}'", s.replace('\'', "'\"'\"'"))
    }
}
pub fn shell_join(args: &[String]) -> String {
    args.iter()
        .map(|a| shell_quote(a))
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn split_shell(s: &str) -> Vec<String> {
    shlex_split(s)
}
fn shlex_split(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        match c {
            '\'' => {
                in_word = true;
                i += 1;
                while i < b.len() && b[i] as char != '\'' {
                    cur.push(b[i] as char);
                    i += 1;
                }
            }
            '"' => {
                in_word = true;
                i += 1;
                while i < b.len() && b[i] as char != '"' {
                    if b[i] as char == '\\'
                        && i + 1 < b.len()
                        && "\"\\$`\n".contains(b[i + 1] as char)
                    {
                        i += 1;
                    }
                    cur.push(b[i] as char);
                    i += 1;
                }
            }
            '\\' if i + 1 < b.len() => {
                in_word = true;
                i += 1;
                cur.push(b[i] as char)
            }
            ' ' | '\t' | '\n' => {
                if in_word {
                    out.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            _ => {
                in_word = true;
                cur.push(c)
            }
        }
        i += 1;
    }
    if in_word {
        out.push(cur)
    }
    out
}

/// A duration the way Go prints one (`1h30m0s`, `1.5ms`, `0s`).
pub fn format_duration(d: Duration) -> String {
    if d.is_zero() {
        return "0s".to_string();
    }
    let mut n = d.as_nanos();
    if n < 1_000_000_000 {
        if n < 1_000 {
            return format!("{n}ns");
        }
        if n < 1_000_000 {
            return decimal_unit(n, 1_000, "µs");
        }
        return decimal_unit(n, 1_000_000, "ms");
    }
    let mut out = String::new();
    let h = n / 3_600_000_000_000;
    if h > 0 {
        out.push_str(&format!("{h}h"));
        n -= h * 3_600_000_000_000;
    }
    let m = n / 60_000_000_000;
    if m > 0 {
        out.push_str(&format!("{m}m"));
        n -= m * 60_000_000_000;
    }
    let s = n / 1_000_000_000;
    n -= s * 1_000_000_000;
    if n == 0 {
        out.push_str(&format!("{s}s"));
    } else {
        let frac = format!("{n:09}");
        out.push_str(&format!("{s}.{}s", frac.trim_end_matches('0')));
    }
    out
}

fn decimal_unit(nanos: u128, unit: u128, suffix: &str) -> String {
    let whole = nanos / unit;
    let rem = nanos % unit;
    if rem == 0 {
        return format!("{whole}{suffix}");
    }
    let width = match unit {
        1_000 => 3,
        1_000_000 => 6,
        _ => 0,
    };
    let frac = format!("{rem:0width$}");
    format!("{whole}.{}{suffix}", frac.trim_end_matches('0'))
}

/// RFC 3339 in UTC with a variable-width fraction: only the significant
/// digits are kept and the fraction disappears entirely on whole seconds
/// (`2026-01-02T03:04:05.5Z`, `2026-01-02T03:04:05Z`), unlike chrono's
/// `SecondsFormat::Nanos`, which always prints nine digits.
pub fn rfc3339_nano<Tz: chrono::TimeZone>(t: chrono::DateTime<Tz>) -> String {
    let t = t.with_timezone(&chrono::Utc);
    let mut s = t.format("%Y-%m-%dT%H:%M:%S").to_string();
    let nanos = t.timestamp_subsec_nanos();
    if nanos != 0 {
        let frac = format!("{nanos:09}");
        s.push('.');
        s.push_str(frac.trim_end_matches('0'));
    }
    s.push('Z');
    s
}
