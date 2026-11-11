//! The git credential helper a GitHub App installation token travels in
//! (ADR-070 rule 3). The brain mints a one-hour token for one repository and
//! leases it; whoever runs the git command that needs it - the hand for its
//! push, the brain for its own fetch and push - installs a [`Helper`] for
//! that command and drops it when the command ends.
//!
//! The helper is a directory of two files outside every worktree: `token`
//! (mode 0600) and `helper` (mode 0700), a shell script that answers git's
//! `get` with `username=x-access-token` and the token for the one host it
//! was installed for, and nothing for any other host. Git finds it through
//! `GIT_CONFIG_COUNT`/`KEY_n`/`VALUE_n` in the command's own environment
//! ([`Helper::env`]), never through a config file: nothing is written into
//! the clone's `.git/config` and nothing can be committed. The list is reset
//! first (`credential.helper=` with an empty value) so a repository's own
//! helper is never asked. Dropping the helper removes the directory.
//!
//! The token never reaches a command line, an argument list, a log line or
//! an error: [`Helper`]'s `Debug` names the host alone.

use anyhow::{Result, anyhow};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// The username GitHub expects beside an installation token.
pub const TOKEN_USER: &str = "x-access-token";

/// What a lease carries on the hand's stdin for a push with an App token:
/// this prefix, a space, the token. Anything else is an SSH private key (the
/// `push.credential` path of ADR-070 step 1b) or nothing.
pub const TOKEN_LEASE_PREFIX: &str = "github-app-token ";

/// The token a lease carries, or None when the lease is something else.
pub fn token_from_lease(lease: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(lease).ok()?;
    let token = text.strip_prefix(TOKEN_LEASE_PREFIX)?.trim();
    valid_token(token).then(|| token.to_string())
}

/// What a lease for `token` looks like on the wire.
pub fn token_lease(token: &str) -> Vec<u8> {
    format!("{TOKEN_LEASE_PREFIX}{token}\n").into_bytes()
}

/// A token is one line of URL-safe characters: anything else could break
/// out of the helper's output (a newline would add a credential field).
pub fn valid_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 512
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
}

/// `host[:port]` of an http(s) url, as git names it to a helper; None for
/// any other url (an ssh or a file url never asks a credential helper).
pub fn http_host(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split('/').next().unwrap_or_default();
    let host = authority.rsplit('@').next().unwrap_or(authority);
    (!host.is_empty()
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b)))
    .then(|| host.to_string())
}

/// One installed helper; removed from disk when dropped.
pub struct Helper {
    dir: PathBuf,
    script: PathBuf,
    host: String,
}

impl std::fmt::Debug for Helper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Helper")
            .field("dir", &self.dir)
            .field("host", &self.host)
            .finish_non_exhaustive()
    }
}

static SEQ: AtomicU64 = AtomicU64::new(0);

impl Helper {
    /// Writes the helper for `host` (`host[:port]`) under `parent`, which
    /// must not be inside a worktree. Errors never carry the token.
    pub fn install(parent: &Path, host: &str, token: &str) -> Result<Helper> {
        if !valid_token(token) {
            return Err(anyhow!("the leased token is not a token"));
        }
        if host.is_empty()
            || !host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b))
        {
            return Err(anyhow!("bad credential host {host:?}"));
        }
        std::fs::create_dir_all(parent)
            .map_err(|e| anyhow!("credential helper parent {}: {e}", parent.display()))?;
        let dir = loop {
            let n = SEQ.fetch_add(1, Ordering::Relaxed);
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0);
            let dir = parent.join(format!("gitcred-{}-{n}-{nanos:08x}", std::process::id()));
            match create_private_dir(&dir) {
                Ok(()) => break dir,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => {
                    return Err(anyhow!("credential helper dir {}: {e}", dir.display()));
                }
            }
        };
        let helper = Helper {
            script: dir.join("helper"),
            dir,
            host: host.to_string(),
        };
        let token_path = helper.dir.join("token");
        write_mode(&token_path, format!("{token}\n").as_bytes(), 0o600)
            .map_err(|e| anyhow!("credential helper token file: {e}"))?;
        let script = format!(
            "#!/bin/sh\n\
             # Hive: one GitHub App installation token for one host, for one command (ADR-070).\n\
             [ \"$1\" = get ] || exit 0\n\
             host=\n\
             while IFS= read -r line; do\n\
             \x20 [ -z \"$line\" ] && break\n\
             \x20 case \"$line\" in host=*) host=${{line#host=}} ;; esac\n\
             done\n\
             [ \"$host\" = {host} ] || exit 0\n\
             printf 'username={TOKEN_USER}\\n'\n\
             printf 'password=%s\\n' \"$(cat {token})\"\n",
            host = crate::util::shell_quote(host),
            token = crate::util::shell_quote(&token_path.display().to_string()),
        );
        write_mode(&helper.script, script.as_bytes(), 0o700)
            .map_err(|e| anyhow!("credential helper script: {e}"))?;
        Ok(helper)
    }

    /// The directory the helper lives in (removed on drop).
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The host the helper answers for.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The environment that has git use this helper and no other, plus any
    /// `extra` config pairs (a `url.<base>.insteadOf` rewrite), as
    /// command-scope config. Callers add it to a hermetic git environment
    /// that holds no `GIT_CONFIG_COUNT` of its own.
    pub fn env(&self, extra: &[(String, String)]) -> Vec<String> {
        let path = self.script.display().to_string();
        let mut pairs: Vec<(String, String)> = vec![
            ("credential.helper".into(), String::new()),
            (
                "credential.helper".into(),
                format!("!{}", crate::util::shell_quote(&path)),
            ),
        ];
        pairs.extend(extra.iter().cloned());
        let mut env = vec![format!("GIT_CONFIG_COUNT={}", pairs.len())];
        for (i, (k, v)) in pairs.into_iter().enumerate() {
            env.push(format!("GIT_CONFIG_KEY_{i}={k}"));
            env.push(format!("GIT_CONFIG_VALUE_{i}={v}"));
        }
        env
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(dir)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir(dir)
    }
}

fn write_mode(path: &Path, data: &[u8], mode: u32) -> std::io::Result<()> {
    use std::io::Write;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    let mut f = o.open(path)?;
    f.write_all(data)?;
    f.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    fn fill(script: &Path, input: &str) -> String {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "{} get",
                crate::util::shell_quote(&script.display().to_string())
            ))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap()
    }

    #[test]
    fn the_helper_answers_its_host_only_and_is_gone_when_dropped() {
        let _serial = crate::test_helpers::serial();
        let parent = tempfile::tempdir().unwrap();
        let h = Helper::install(parent.path(), "github.com", "ghs_abc123").unwrap();
        let dir = h.dir().to_path_buf();
        assert!(dir.starts_with(parent.path()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&dir), 0o700);
            assert_eq!(mode(&dir.join("token")), 0o600);
            assert_eq!(mode(&dir.join("helper")), 0o700);
        }
        // The script names the token's file, never the token.
        let script = std::fs::read_to_string(dir.join("helper")).unwrap();
        assert!(!script.contains("ghs_abc123"), "{script}");
        assert_eq!(
            fill(&dir.join("helper"), "protocol=https\nhost=github.com\n\n"),
            "username=x-access-token\npassword=ghs_abc123\n"
        );
        assert_eq!(
            fill(&dir.join("helper"), "protocol=https\nhost=evil.example\n\n"),
            ""
        );
        // The env names the helper as command-scope config after a reset.
        let env = h.env(&[(
            "url.https://github.com/.insteadOf".into(),
            "git@github.com:".into(),
        )]);
        assert_eq!(env[0], "GIT_CONFIG_COUNT=3");
        assert!(env.contains(&"GIT_CONFIG_KEY_0=credential.helper".to_string()));
        assert!(env.contains(&"GIT_CONFIG_VALUE_0=".to_string()));
        assert!(env.iter().all(|kv| !kv.contains("ghs_abc123")), "{env:?}");
        assert!(!format!("{h:?}").contains("ghs_abc123"));
        drop(h);
        assert!(!dir.exists(), "the helper outlived its command");
    }

    #[test]
    fn leases_and_tokens_are_checked() {
        let _serial = crate::test_helpers::serial();
        assert_eq!(
            token_from_lease(&token_lease("ghs_x-1.y")).as_deref(),
            Some("ghs_x-1.y")
        );
        assert_eq!(
            token_from_lease(b"-----BEGIN OPENSSH PRIVATE KEY-----"),
            None
        );
        assert_eq!(token_from_lease(b"github-app-token a\nb"), None);
        assert_eq!(token_from_lease(b""), None);
        assert!(!valid_token("a b"));
        assert!(!valid_token("a'b"));
        let parent = tempfile::tempdir().unwrap();
        let err = Helper::install(parent.path(), "github.com", "x\ny").unwrap_err();
        assert!(!err.to_string().contains('y'), "{err}");
        assert!(Helper::install(parent.path(), "a b", "tok").is_err());
        assert_eq!(
            http_host("https://github.com/o/r.git").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            http_host("http://x-access-token@127.0.0.1:8080/o/r.git").as_deref(),
            Some("127.0.0.1:8080")
        );
        assert_eq!(http_host("git@github.com:o/r.git"), None);
        assert_eq!(http_host("/tmp/origin.git"), None);
    }

    /// Clone and push over smart HTTP through the helper: the server takes
    /// only `x-access-token:<token>`, git gets it from the helper alone,
    /// and nothing of it lands in the clone's config or files.
    #[test]
    fn a_clone_and_a_push_authenticate_through_the_helper() {
        let _serial = crate::test_helpers::serial();
        if !hive_testutil::githttp::available() {
            return;
        }
        use base64::Engine;
        let token = "ghs_helperTest123";
        let auth = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{TOKEN_USER}:{token}"))
        );
        let root = tempfile::tempdir().unwrap();
        let bare = root.path().join("owner").join("repo.git");
        std::fs::create_dir_all(bare.parent().unwrap()).unwrap();
        crate::test_helpers::run(
            root.path(),
            &[
                "git",
                "init",
                "-q",
                "--bare",
                "-b",
                "main",
                bare.to_str().unwrap(),
            ],
        );
        let seed = tempfile::tempdir().unwrap();
        crate::test_helpers::run(seed.path(), &["git", "init", "-q", "-b", "main"]);
        std::fs::write(seed.path().join("a.txt"), "a\n").unwrap();
        crate::test_helpers::run(seed.path(), &["git", "add", "a.txt"]);
        crate::test_helpers::run(seed.path(), &["git", "commit", "-q", "-m", "a"]);
        crate::test_helpers::run(
            seed.path(),
            &["git", "push", "-q", bare.to_str().unwrap(), "main"],
        );
        let srv = hive_testutil::githttp::Server::new(root.path(), &auth);
        let url = format!("{}/owner/repo.git", srv.url());
        let host = http_host(&url).unwrap();
        let work = tempfile::tempdir().unwrap();
        let clone = work.path().join("clone");
        let creds = tempfile::tempdir().unwrap();

        // Without the helper the server refuses.
        let r = crate::util::git_timeout_env(
            "",
            std::time::Duration::from_secs(60),
            &[],
            &["clone", "-q", &url, clone.to_str().unwrap()],
        );
        assert!(!r.ok(), "cloned without a credential");
        let _ = std::fs::remove_dir_all(&clone);

        let helper = Helper::install(creds.path(), &host, token).unwrap();
        let env = helper.env(&[]);
        let r = crate::util::git_timeout_env(
            "",
            std::time::Duration::from_secs(60),
            &env,
            &["clone", "-q", &url, clone.to_str().unwrap()],
        );
        assert!(r.ok(), "clone through the helper: {}", r.stderr);
        let clone_s = clone.display().to_string();
        crate::test_helpers::run(&clone, &["git", "checkout", "-q", "-b", "hive/a/job"]);
        std::fs::write(clone.join("b.txt"), "b\n").unwrap();
        crate::test_helpers::run(&clone, &["git", "add", "b.txt"]);
        crate::test_helpers::run(&clone, &["git", "commit", "-q", "-m", "b"]);
        let r = crate::util::git_timeout_env(
            &clone_s,
            std::time::Duration::from_secs(60),
            &env,
            &["push", "-q", &url, "hive/a/job"],
        );
        assert!(r.ok(), "push through the helper: {}", r.stderr);
        let pushed = crate::util::git(
            bare.to_str().unwrap(),
            &["rev-parse", "refs/heads/hive/a/job"],
        );
        assert!(pushed.ok(), "the branch did not reach the remote");
        assert!(srv.seen().contains(&auth));
        drop(helper);
        assert_eq!(std::fs::read_dir(creds.path()).unwrap().count(), 0);
        let config = std::fs::read_to_string(clone.join(".git").join("config")).unwrap();
        assert!(!config.contains("credential"), "{config}");
        assert!(!config.contains(token), "{config}");
        let tracked = crate::util::git(&clone_s, &["grep", "-q", token, "HEAD"]);
        assert!(!tracked.ok(), "the token is in a committed file");
        let status = crate::util::git(&clone_s, &["status", "--porcelain"]);
        assert_eq!(
            status.stdout.trim(),
            "",
            "the helper left files in the clone"
        );
    }
}
