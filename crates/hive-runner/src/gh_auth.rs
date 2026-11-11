//! Which GitHub account `gh` reads a repository as. `gh` keeps one login
//! active per host and ignores the others, so a machine whose active login
//! is a fine-grained token that sees none of the watched repositories,
//! beside an account that sees them all, read nothing: every poll of the
//! pipeline stage answered 404, 7.3 s of one daemon tick spent on three
//! repositories for nothing. This module lists the logged-in accounts and
//! lends the stage the environment that runs `gh` as a named one of them:
//! `GH_TOKEN` from `gh auth token --user`, which gh prefers over its
//! keyring. `gh auth status` tests every account over the network (1.8 s
//! on the machine this was measured on) and `gh auth token` reads the
//! keyring (70 ms), so both are kept for `TTL` and asked once in it, never
//! per poll. A token is never put in words: nothing here prints the
//! environment or an error with stdout in it, and the cache that holds
//! tokens has no Debug.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long the accounts and their tokens are kept before gh is asked
/// again, so a login added or refreshed on the machine is seen within it.
const TTL: Duration = Duration::from_secs(10 * 60);
/// `gh auth status` reaches GitHub once per account; `gh auth token` reads
/// the keyring, which on macOS may wait for the user to unlock it.
const STATUS_TIMEOUT: Duration = Duration::from_secs(30);
const TOKEN_TIMEOUT: Duration = Duration::from_secs(15);
const HOST: &str = "github.com";

/// The accounts `gh auth status` lists as logged in to github.com.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Logins {
    /// The account gh uses when none is named; empty when gh did not say.
    pub active: String,
    /// Every logged-in account in the order gh lists them, the active one
    /// among them. An account gh could not log in to (its keyring token
    /// invalid) is left out: it would answer nothing.
    pub accounts: Vec<String>,
}

impl Logins {
    /// The logged-in accounts other than `user` and the active one, in
    /// gh's order: what to try by name when `user` cannot see a repository
    /// (the active login is tried as itself, without a token).
    pub(crate) fn others(&self, user: &str) -> Vec<String> {
        self.accounts
            .iter()
            .filter(|a| a.as_str() != user && a.as_str() != self.active)
            .cloned()
            .collect()
    }
}

/// Reads the accounts out of `gh auth status`'s text, which gh writes to
/// stderr (2.102 does, and whenever an account has a problem) or stdout
/// (older releases), one block per account: `Logged in to github.com
/// account NAME (keyring)` and below it `Active account: true` for the one
/// gh uses. Releases before 2.40 knew one account per host and wrote
/// `Logged in to github.com as NAME`; that one is the active login. A name
/// listed twice (from `GH_TOKEN` and from the keyring) counts once, and an
/// account gh `Failed to log in to` is left out.
pub(crate) fn parse_status(stdout: &str, stderr: &str) -> Logins {
    let mut logins = Logins::default();
    let mut current = String::new();
    for line in stdout.lines().chain(stderr.lines()) {
        let line = line.trim();
        if let Some(rest) = line.split_once("Logged in to ").map(|x| x.1) {
            current.clear();
            let words: Vec<&str> = rest.split_whitespace().collect();
            if words.first() == Some(&HOST)
                && matches!(words.get(1), Some(&"account") | Some(&"as"))
                && let Some(name) = words.get(2)
                && !name.is_empty()
            {
                current = name.to_string();
                if !logins.accounts.iter().any(|a| a == &current) {
                    logins.accounts.push(current.clone());
                }
            }
        } else if line.contains("Failed to log in to") {
            current.clear();
        } else if line.ends_with("Active account: true") && !current.is_empty() {
            logins.active = current.clone();
        }
    }
    if logins.active.is_empty() && logins.accounts.len() == 1 {
        logins.active = logins.accounts[0].clone();
    }
    logins
}

/// What is kept between asks: the accounts, and the token of each account
/// that was asked for, as `gh auth token --user` answered.
struct Cache {
    read: Instant,
    logins: Logins,
    tokens: BTreeMap<String, String>,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

fn with_cache<T>(f: impl FnOnce(&mut Cache) -> T) -> T {
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if guard.as_ref().is_none_or(|c| c.read.elapsed() > TTL) {
        *guard = Some(Cache {
            read: Instant::now(),
            logins: read_status(),
            tokens: BTreeMap::new(),
        });
    }
    f(guard.as_mut().expect("filled above"))
}

/// The accounts this machine is logged in to, as gh last said within
/// `TTL`; none when gh is not installed or `gh auth status` did not answer,
/// which leaves the active login as the only account.
pub(crate) fn logins() -> Logins {
    with_cache(|c| c.logins.clone())
}

/// The environment that runs gh as `user`: the daemon's own with
/// `GH_TOKEN` set to the account's token, which gh prefers over its
/// keyring. The error names the account, never the token.
pub(crate) fn env_as(user: &str) -> Result<Vec<String>, String> {
    let token = with_cache(|c| -> Result<String, String> {
        match c.tokens.get(user) {
            Some(t) => Ok(t.clone()),
            None => {
                let t = read_token(user)?;
                c.tokens.insert(user.to_string(), t.clone());
                Ok(t)
            }
        }
    })?;
    let mut extra = BTreeMap::new();
    extra.insert("GH_TOKEN".to_string(), token);
    Ok(crate::proc::env_with(&crate::proc::environ(), &extra))
}

fn read_status() -> Logins {
    let argv = ["gh", "auth", "status"].map(String::from);
    match run(&argv, STATUS_TIMEOUT) {
        // gh exits 1 when any account has a problem and still lists the
        // others, so the text is read whatever the exit code.
        Ok(out) if !out.timeout => parse_status(&out.stdout, &out.stderr),
        _ => Logins::default(),
    }
}

fn read_token(user: &str) -> Result<String, String> {
    let argv = ["gh", "auth", "token", "--user", user, "--hostname", HOST].map(String::from);
    match run(&argv, TOKEN_TIMEOUT) {
        Err(e) => Err(format!("gh auth token for {user}: {e}")),
        Ok(out) if out.timeout => Err(format!(
            "gh auth token for {user} did not answer within {}s",
            TOKEN_TIMEOUT.as_secs()
        )),
        Ok(out) if out.exit_code != 0 => Err(format!(
            "gh auth token for {user} exited {}: {}",
            out.exit_code,
            crate::util::trunc(out.stderr.trim(), 200)
        )),
        Ok(out) => {
            let token = out.stdout.trim().to_string();
            if token.is_empty() {
                Err(format!("gh auth token for {user} answered nothing"))
            } else {
                Ok(token)
            }
        }
    }
}

fn run(argv: &[String], timeout: Duration) -> anyhow::Result<crate::proc::Outcome> {
    crate::proc::run_group(
        &hive_bus::CancellationToken::new(),
        argv,
        "",
        &crate::proc::environ(),
        &[],
        timeout,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = "github.com
  ✓ Logged in to github.com account auth0-6a9ed6c22202028697a5ee8d_TEFGER (keyring)
  - Active account: true
  - Git operations protocol: https
  - Token: github_pat_****

  ✓ Logged in to github.com account manishtiwari25 (keyring)
  - Active account: false
  - Git operations protocol: https
  - Token: gho_****
  - Token scopes: 'delete:packages', 'delete_repo', 'gist', 'read:org', 'repo', 'workflow', 'write:packages'

  X Failed to log in to github.com account manish-tiwari_TEFGER (keyring)
  - Active account: false
  - The token in keyring is invalid.
  - To re-authenticate, run: gh auth refresh -h github.com
  - To forget about this account, run: gh auth logout -h github.com -u manish-tiwari_TEFGER
";

    #[test]
    fn the_status_lists_the_accounts_and_the_active_one_from_either_stream() {
        let want = Logins {
            active: "auth0-6a9ed6c22202028697a5ee8d_TEFGER".into(),
            accounts: vec![
                "auth0-6a9ed6c22202028697a5ee8d_TEFGER".into(),
                "manishtiwari25".into(),
            ],
        };
        assert_eq!(parse_status("", STATUS), want, "gh 2.102 writes to stderr");
        assert_eq!(parse_status(STATUS, ""), want, "older gh wrote to stdout");
        assert_eq!(want.others(""), ["manishtiwari25"]);
        assert_eq!(
            want.others("manishtiwari25"),
            Vec::<String>::new(),
            "the active login is tried as itself, not by name"
        );
        assert_eq!(want.others("someone-logged-out"), ["manishtiwari25"]);
    }

    #[test]
    fn an_environment_token_lists_its_account_once_and_a_lone_account_is_the_active_one() {
        let env_token = "github.com
  ✓ Logged in to github.com account manishtiwari25 (GH_TOKEN)
  - Active account: true
  - Git operations protocol: https
  - Token: gho_****

  ✓ Logged in to github.com account manishtiwari25 (keyring)
  - Active account: false

  ✓ Logged in to github.com account auth0-6a9ed6c22202028697a5ee8d_TEFGER (keyring)
  - Active account: false
";
        let l = parse_status("", env_token);
        assert_eq!(l.active, "manishtiwari25");
        assert_eq!(
            l.accounts,
            ["manishtiwari25", "auth0-6a9ed6c22202028697a5ee8d_TEFGER"]
        );

        let old = "github.com
  ✓ Logged in to github.com as manishtiwari25 (keyring)
  ✓ Git operations for github.com configured to use https protocol.
  ✓ Token: gho_****
ghe.example.com
  ✓ Logged in to ghe.example.com as someone (keyring)
";
        let l = parse_status(old, "");
        assert_eq!(l.active, "manishtiwari25", "one account: the active one");
        assert_eq!(
            l.accounts,
            ["manishtiwari25"],
            "other hosts are not github.com"
        );

        assert_eq!(parse_status("", ""), Logins::default());
        assert_eq!(
            parse_status(
                "",
                "You are not logged into any GitHub hosts. To log in, run: gh auth login"
            ),
            Logins::default()
        );
    }
}
