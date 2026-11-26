//! The one rule every push of Hive's follows on an https url when the
//! credential it holds is an ssh key: git offers a key over ssh only, so an
//! upstream cloned over https (`https://github.com/OWNER/REPO`) never sees
//! the key and the push fails with `could not read Username for
//! 'https://github.com': terminal prompts disabled`, however good the key
//! is. When that is all git says, the command is made once more against
//! the url's ssh form, where the key counts.
//!
//! The daemon's own pushes and fetches (`hive-runner`'s `forge`), the push
//! a paired machine makes with the key the brain leased it (`hive-hand`)
//! and the probe `hive cred set` runs before it saves a key all read the
//! url and git's answer through these two functions, so the three cannot
//! drift apart. Both read strictly, because a push must never go anywhere
//! but where the url points.

/// GitHub's web address: the forge whose ssh form is known, and the one a
/// caller names unless a test points it at its own server.
pub const GITHUB_WEB: &str = "https://github.com";

/// The ssh form of an https url on the forge whose web address is `web`
/// ([`GITHUB_WEB`] outside tests): `https://github.com/OWNER/REPO`, with
/// `.git` or without, is `git@github.com:OWNER/REPO.git`. None for anything
/// else. Git ends the host at the first `/`, `?` or `#`, so
/// `https://HOST?@github.com/OWNER/REPO` goes to HOST while a reader that
/// looks past the `@` sees GitHub. So the url must be the forge's own
/// address, one `/` and the two names, with nothing else in it - no user, no
/// port, no query, no fragment, no further path - and the names hold
/// letters, digits, `.`, `_` and `-` alone.
pub fn ssh_form(web: &str, url: &str) -> Option<String> {
    let path = url
        .strip_prefix(web.trim_end_matches('/'))?
        .strip_prefix('/')?;
    let (owner, repo) = path.strip_suffix(".git").unwrap_or(path).split_once('/')?;
    let plain = |name: &str| {
        !name.is_empty()
            && !name.starts_with('-')
            && name.bytes().any(|b| b != b'.')
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    };
    let host = git_host(web)?;
    (plain(owner) && plain(repo)).then(|| format!("git@{host}:{owner}/{repo}.git"))
}

/// The host of `web`, without a user or a port: where its ssh lives.
fn git_host(web: &str) -> Option<String> {
    let rest = web
        .strip_prefix("https://")
        .or_else(|| web.strip_prefix("http://"))?;
    let authority = rest.split('/').next().unwrap_or_default();
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = host.split(':').next().unwrap_or(host);
    (!host.is_empty()
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b)))
    .then(|| host.to_string())
}

/// Whether all git said (`said`, its stderr) is that the forge at `web`
/// asked for a credential git had none to give: its last line is `fatal:
/// could not read Username for 'https://github.com': terminal prompts
/// disabled`, and before it stands at most its notice that the forge sent
/// it to another address of the forge's (`warning: redirecting to
/// https://github.com/...`).
///
/// One such line among others proves nothing, because the host git
/// reached writes part of what git prints: a server's error comes out as
/// `fatal: remote error:` and the server's own words, line breaks
/// included, so a host can end git's answer with this very line. And the
/// host is not always the url's: the repository's own config may send the
/// url elsewhere (`url.*.insteadOf`), and a server may redirect it. An
/// answer of nothing but these lines is git's own, about the forge. Words
/// that name no address, another address, or the address with the
/// repository's path (`credential.useHttpPath`) are not that answer.
pub fn credential_asked(web: &str, said: &str) -> bool {
    let web = web.trim_end_matches('/');
    let asked = format!("fatal: could not read Username for '{web}': terminal prompts disabled");
    let redirected = format!("warning: redirecting to {web}/");
    let mut lines = said.lines().filter(|line| !line.trim().is_empty());
    lines.next_back().is_some_and(|last| last == asked)
        && lines.all(|line| line.starts_with(&redirected))
}

/// The forge's web address a hand and `hive cred set` read urls against:
/// [`GITHUB_WEB`], unless `HIVE_FORGE_WEB` names another (a test's git
/// server). The daemon reads its own from the GitHub App endpoint. Another
/// address changes only which urls have an ssh form; that form keeps the
/// url's host, so no push is sent anywhere the url does not point.
pub fn web() -> String {
    std::env::var("HIVE_FORGE_WEB")
        .ok()
        .map(|w| w.trim().trim_end_matches('/').to_string())
        .filter(|w| !w.is_empty())
        .unwrap_or_else(|| GITHUB_WEB.to_string())
}

/// The url to make a failed git command again against with an ssh key, or
/// None to leave it where it failed: when all git said (`said`) is that
/// the forge ([`web`]) asked for a credential git had none to give
/// ([`credential_asked`]), `url` has an ssh form ([`ssh_form`]) and git in
/// `repo` asks `url` as it is given - the repository's own config does not
/// put another in its place (`url.*.insteadOf`), which `git ls-remote
/// --get-url` tells without calling anybody. A host such a url is sent to
/// could send git on to the forge, and the forge's question would then be
/// git's whole answer although the command went elsewhere first.
pub fn ssh_retry_url(repo: &str, url: &str, said: &str) -> Option<String> {
    let web = web();
    if !credential_asked(&web, said) {
        return None;
    }
    let ssh = ssh_form(&web, url)?;
    let asked = std::process::Command::new(crate::git_program())
        .args(["-C", repo, "ls-remote", "--get-url", url])
        .env_clear()
        .envs(crate::env().into_iter().filter_map(|kv| {
            kv.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
        }))
        .output()
        .ok()?;
    (asked.status.success() && String::from_utf8_lossy(&asked.stdout).trim() == url).then_some(ssh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ssh_form_keeps_to_the_forge_the_url_names() {
        assert_eq!(
            ssh_form(GITHUB_WEB, "https://github.com/owner/repo").as_deref(),
            Some("git@github.com:owner/repo.git")
        );
        assert_eq!(
            ssh_form("http://127.0.0.1:8080/", "http://127.0.0.1:8080/o/r.git").as_deref(),
            Some("git@127.0.0.1:o/r.git")
        );
        for url in [
            "https://github.com//repo",
            "https://github.com/owner/",
            "https://example.org?@github.com/owner/repo",
            "https://github.com.example.org/owner/repo",
        ] {
            assert_eq!(ssh_form(GITHUB_WEB, url), None, "{url}");
        }
    }

    #[test]
    fn only_the_forges_own_request_counts() {
        let asked =
            "fatal: could not read Username for 'https://github.com': terminal prompts disabled";
        assert!(credential_asked(GITHUB_WEB, asked));
        assert!(credential_asked(
            "https://github.com/",
            &format!("{asked}\n")
        ));
        assert!(!credential_asked("https://example.org", asked));
        assert!(!credential_asked(
            GITHUB_WEB,
            &format!("fatal: remote error: no\n{asked}")
        ));
    }
}
