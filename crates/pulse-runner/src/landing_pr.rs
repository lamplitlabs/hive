//! Pull requests through `gh`, the forge CLI the owner already logs in to
//! on this machine (its auth never reaches git): the calls the delivery
//! pass (`delivery.rs`) makes to open, find, list and read pull requests.
//!
//! Every call is made as the gh account that can see the repository. The
//! daemon's active gh login is a fine-grained token that answers 404 for
//! the owner's repositories while another login on the machine sees them
//! all (measured 2026-11-04: every `gh pr create` of the first per-task
//! pull requests failed that way), so a call that fails with 401, 403 or
//! 404 is made again as each other logged-in account (`gh_auth`), the one
//! that answers is remembered for the repository, and the next call goes
//! straight to it. Errors name the account and the command, never a token.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

/// `OWNER/REPO` (or `HOST/OWNER/REPO` off github.com) for `gh --repo`, from
/// a GitHub-style ssh or http(s) url; None for anything else.
pub(crate) fn repo_slug(url: &str) -> Option<String> {
    let (host, path) = if let Some(rest) = url.strip_prefix("ssh://") {
        let rest = rest.rsplit('@').next().unwrap_or(rest);
        rest.split_once('/')?
    } else if let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    {
        let (host, path) = rest.split_once('/')?;
        (host.rsplit('@').next().unwrap_or(host), path)
    } else {
        let (userhost, path) = url.split_once(':')?;
        (userhost.rsplit('@').next().unwrap_or(userhost), path)
    };
    let host = host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host);
    if host.is_empty() || host.contains('/') {
        return None;
    }
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut segs = path.split('/');
    let owner = segs.next().unwrap_or_default();
    let repo = segs.next().unwrap_or_default();
    if owner.is_empty() || repo.is_empty() || segs.next().is_some() {
        return None;
    }
    Some(if host == "github.com" {
        format!("{owner}/{repo}")
    } else {
        format!("{host}/{owner}/{repo}")
    })
}

/// The gh account that last answered for each repository (`gh --repo`
/// slug), kept for the daemon's lifetime: the first call of a pass finds
/// it once, the rest go straight to it.
static REPO_ACCOUNTS: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());

fn remembered(repo: &str) -> Option<String> {
    REPO_ACCOUNTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(repo)
        .cloned()
}

fn remember(repo: &str, user: Option<&str>) {
    let mut map = REPO_ACCOUNTS.lock().unwrap_or_else(|e| e.into_inner());
    match user {
        Some(u) => {
            map.insert(repo.to_string(), u.to_string());
        }
        None => {
            map.remove(repo);
        }
    }
}

/// Runs `gh` in the project checkout (so it finds the repository from the
/// remotes when `repo` is empty) as the account that can see `repo`:
/// first the one remembered for it, else the active login; when that one
/// cannot see the repository, every other logged-in account once. Returns
/// stdout, or the failure put into words.
pub(crate) fn gh(
    root: &str,
    repo: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<String, String> {
    gh_with(root, repo, args, timeout, &crate::gh_auth::logins)
}

/// `gh` with the machine's logins supplied by the caller, so tests name
/// them instead of asking `gh auth status`.
pub(crate) fn gh_with(
    root: &str,
    repo: &str,
    args: &[&str],
    timeout: Duration,
    logins: &dyn Fn() -> crate::gh_auth::Logins,
) -> Result<String, String> {
    let first = remembered(repo);
    let err = match run_gh(root, repo, args, timeout, first.as_deref()) {
        Ok(out) => return Ok(out),
        Err(e) if !crate::pipeline::account_failed(&e) => return Err(e),
        Err(e) => e,
    };
    let logins = logins();
    let active = if logins.active.is_empty() {
        "the active gh login".to_string()
    } else {
        logins.active.clone()
    };
    let mut tried = vec![first.clone().unwrap_or_else(|| active.clone())];
    let mut candidates: Vec<Option<String>> = Vec::new();
    if let Some(user) = &first {
        remember(repo, None);
        if *user != logins.active {
            candidates.push(None);
        }
    }
    candidates.extend(
        logins
            .others(first.as_deref().unwrap_or_default())
            .into_iter()
            .map(Some),
    );
    for candidate in candidates {
        match run_gh(root, repo, args, timeout, candidate.as_deref()) {
            Ok(out) => {
                remember(repo, candidate.as_deref());
                return Ok(out);
            }
            Err(e) if crate::pipeline::account_failed(&e) => {
                tried.push(candidate.unwrap_or_else(|| active.clone()));
            }
            Err(e) => return Err(e),
        }
    }
    Err(format!(
        "{err} (none of the gh logins on this machine can see {}: tried {})",
        if repo.is_empty() {
            "the repository"
        } else {
            repo
        },
        tried.join(", ")
    ))
}

fn run_gh(
    root: &str,
    repo: &str,
    args: &[&str],
    timeout: Duration,
    user: Option<&str>,
) -> Result<String, String> {
    let mut argv = vec!["gh".to_string()];
    argv.extend(args.iter().map(|a| a.to_string()));
    if !repo.is_empty() {
        argv.push("--repo".into());
        argv.push(repo.to_string());
    }
    let what = match user {
        Some(u) => format!("gh {} {} as {u}", args[0], args[1]),
        None => format!("gh {} {}", args[0], args[1]),
    };
    let env = match user {
        Some(u) => crate::gh_auth::env_as(u).map_err(|e| format!("{what}: {e}"))?,
        None => crate::proc::environ(),
    };
    match crate::proc::run_group(
        &pulse_bus::CancellationToken::new(),
        &argv,
        root,
        &env,
        &[],
        timeout,
    ) {
        Err(e) => Err(gh_failure(&what, &e.to_string())),
        Ok(out) if out.timeout => Err(format!(
            "{what} did not finish within {}s",
            timeout.as_secs()
        )),
        Ok(out) if out.exit_code != 0 => Err(gh_failure(
            &what,
            &pulse_cred::redact(&format!("{} {}", out.stderr, out.stdout), &[]),
        )),
        Ok(out) => Ok(out.stdout),
    }
}

/// The pull request `gh` reports open from `head` into `base`, as
/// (number, url).
pub(crate) fn open_pull_request(
    root: &str,
    repo: &str,
    head: &str,
    base: &str,
) -> Result<Option<(i64, String)>, String> {
    let out = gh(
        root,
        repo,
        &[
            "pr",
            "list",
            "--head",
            head,
            "--base",
            base,
            "--state",
            "open",
            "--json",
            "number,url",
            "--limit",
            "1",
        ],
        Duration::from_secs(60),
    )?;
    let list: Vec<serde_json::Value> =
        serde_json::from_str(out.trim()).map_err(|e| format!("gh pr list answered oddly: {e}"))?;
    Ok(list.first().map(|v| {
        (
            v.get("number").and_then(|n| n.as_i64()).unwrap_or(0),
            v.get("url")
                .and_then(|u| u.as_str())
                .unwrap_or_default()
                .to_string(),
        )
    }))
}

/// Opens the pull request from `head` into `base` and returns its
/// (number, url) as gh prints them.
pub(crate) fn create_pull_request(
    root: &str,
    repo: &str,
    head: &str,
    base: &str,
    title: &str,
    body: &str,
) -> Result<(i64, String), String> {
    let out = gh(
        root,
        repo,
        &[
            "pr",
            "create",
            "--head",
            head,
            "--base",
            base,
            "--title",
            &crate::util::trunc(title, 200),
            "--body",
            body,
        ],
        Duration::from_secs(120),
    )?;
    let url = out
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| l.starts_with("http"))
        .unwrap_or_default()
        .to_string();
    let number = url
        .rsplit('/')
        .next()
        .and_then(|n| n.parse::<i64>().ok())
        .unwrap_or(0);
    if number == 0 {
        return Err(format!("gh did not name the pull request for {head}"));
    }
    Ok((number, url))
}

/// Points pull request `number` at `base`: a pull request stacked on one
/// that merged or closed moves onto that one's base.
pub(crate) fn retarget_pull_request(
    root: &str,
    repo: &str,
    number: i64,
    base: &str,
) -> Result<(), String> {
    gh(
        root,
        repo,
        &["pr", "edit", &number.to_string(), "--base", base],
        Duration::from_secs(60),
    )
    .map(|_| ())
}

/// The fields `pr list` and `pr view` are asked for: the state and head of
/// each pull request, and what stands between an open one and its merge.
const PR_FIELDS: &str =
    "number,state,mergedAt,url,headRefName,headRefOid,mergeable,mergeStateStatus,statusCheckRollup";

/// What the forge says of one pull request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PrState {
    pub(crate) number: i64,
    /// `OPEN`, `MERGED` or `CLOSED`.
    pub(crate) state: String,
    pub(crate) merged_at: String,
    pub(crate) url: String,
    pub(crate) head: String,
    /// The head branch's tip as the forge has it (`headRefOid`).
    pub(crate) head_sha: String,
    /// `MERGEABLE`, `CONFLICTING` or `UNKNOWN` (the forge has not judged
    /// the head yet).
    pub(crate) mergeable: String,
    /// `CLEAN`, `DIRTY` (conflicts), `BLOCKED`, `BEHIND`, `UNSTABLE`,
    /// `HAS_HOOKS` or `UNKNOWN`.
    pub(crate) merge_state: String,
    /// The names of the checks on the head that failed, were cancelled,
    /// timed out or want action, from `statusCheckRollup`.
    pub(crate) failing_checks: Vec<String>,
}

impl PrState {
    /// The head conflicts with its base as the forge sees it.
    pub(crate) fn conflicting(&self) -> bool {
        self.mergeable == "CONFLICTING" || self.merge_state == "DIRTY"
    }
}

fn pr_state(v: &serde_json::Value) -> PrState {
    let field = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string()
    };
    PrState {
        number: v.get("number").and_then(|n| n.as_i64()).unwrap_or(0),
        state: field("state"),
        merged_at: field("mergedAt"),
        url: field("url"),
        head: field("headRefName"),
        head_sha: field("headRefOid"),
        mergeable: field("mergeable"),
        merge_state: field("mergeStateStatus"),
        failing_checks: failing_checks(v.get("statusCheckRollup")),
    }
}

/// The checks of a `statusCheckRollup` that did not pass: a check run
/// (`name`, `conclusion`) or a commit status (`context`, `state`). A check
/// still running, skipped or neutral is not failing.
fn failing_checks(rollup: Option<&serde_json::Value>) -> Vec<String> {
    let Some(list) = rollup.and_then(|r| r.as_array()) else {
        return Vec::new();
    };
    let text = |e: &serde_json::Value, k: &str| {
        e.get(k)
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string()
    };
    list.iter()
        .filter(|e| {
            matches!(
                text(e, "conclusion").as_str(),
                "FAILURE" | "CANCELLED" | "TIMED_OUT" | "ACTION_REQUIRED" | "STARTUP_FAILURE"
            ) || matches!(text(e, "state").as_str(), "ERROR" | "FAILURE")
        })
        .map(|e| {
            let name = text(e, "name");
            if name.is_empty() {
                text(e, "context")
            } else {
                name
            }
        })
        .filter(|n| !n.is_empty())
        .collect()
}

/// The repository's newest 200 pull requests in every state, one process
/// per repository per pass instead of one `gh pr view` per open pull
/// request (the daemon keeps a dozen open at a time; each gh call is
/// 0.4-1 s).
pub(crate) fn list_prs(root: &str, repo: &str) -> Result<BTreeMap<i64, PrState>, String> {
    let out = gh(
        root,
        repo,
        &[
            "pr", "list", "--state", "all", "--json", PR_FIELDS, "--limit", "200",
        ],
        Duration::from_secs(90),
    )?;
    let list: Vec<serde_json::Value> =
        serde_json::from_str(out.trim()).map_err(|e| format!("gh pr list answered oddly: {e}"))?;
    Ok(list
        .iter()
        .map(pr_state)
        .filter(|p| p.number > 0)
        .map(|p| (p.number, p))
        .collect())
}

/// One pull request as the forge has it, for a number the list missed.
pub(crate) fn view_pr(root: &str, repo: &str, number: i64) -> Result<PrState, String> {
    let out = gh(
        root,
        repo,
        &["pr", "view", &number.to_string(), "--json", PR_FIELDS],
        Duration::from_secs(60),
    )?;
    let v: serde_json::Value =
        serde_json::from_str(out.trim()).map_err(|e| format!("gh pr view answered oddly: {e}"))?;
    let mut st = pr_state(&v);
    if st.number == 0 {
        st.number = number;
    }
    Ok(st)
}

fn gh_failure(what: &str, why: &str) -> String {
    let why = why.trim();
    if why.contains("No such file or directory")
        || (why.contains("not found") && why.contains("gh"))
    {
        return "gh (the GitHub CLI) is not installed where the daemon can run it; install it and run gh auth login as the user the daemon runs as".to_string();
    }
    if why.contains("gh auth login") || why.contains("not logged in") {
        return format!(
            "{what} failed: gh is not logged in for the user the daemon runs as; run gh auth login there"
        );
    }
    let squeezed: String = why.split_whitespace().collect::<Vec<_>>().join(" ");
    let squeezed = if squeezed.chars().count() > 200 {
        squeezed.chars().take(199).collect::<String>() + "~"
    } else {
        squeezed
    };
    format!("{what} failed: {squeezed}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_slug_reads_github_style_urls() {
        for (url, want) in [
            ("git@github.com:owner/repo.git", Some("owner/repo")),
            ("git@github.com:owner/repo", Some("owner/repo")),
            ("https://github.com/owner/repo.git", Some("owner/repo")),
            ("https://me@github.com/owner/repo/", Some("owner/repo")),
            ("ssh://git@github.com/owner/repo.git", Some("owner/repo")),
            (
                "ssh://git@ghe.corp:2222/owner/repo.git",
                Some("ghe.corp/owner/repo"),
            ),
            ("https://ghe.corp/owner/repo", Some("ghe.corp/owner/repo")),
            ("/tmp/origin.git", None),
            ("https://github.com/owner", None),
            ("https://github.com/owner/repo/deeper", None),
        ] {
            assert_eq!(repo_slug(url).as_deref(), want, "{url}");
        }
    }

    /// A gh that can see `rot/r` only as the account whose token it hands
    /// out for `seer-x`, cannot see `rot/closed` as anyone, and writes
    /// which account made each pull-request call (the token's user, never
    /// the token).
    fn rotating_gh() -> (tempfile::TempDir, pulse_testutil::SetenvGuard) {
        let bin = tempfile::tempdir().expect("fake gh dir");
        let script = format!(
            r#"#!/bin/sh
d='{d}'
case "$1 $2" in
  "auth status") echo "github.com: Logged in to github.com account pat-x (keyring)"; exit 0 ;;
  "auth token") echo "tok-$4"; exit 0 ;;
esac
echo "$1 $2 as=${{GH_TOKEN#tok-}}" >> "$d/calls"
repo=""; prev=""
for a in "$@"; do [ "$prev" = "--repo" ] && repo="$a"; prev="$a"; done
if [ "$repo" = rot/r ] && [ "$GH_TOKEN" = tok-seer-x ]; then echo '[]'; exit 0; fi
echo "GraphQL: Could not resolve to a Repository with the name '$repo'. (repository)" >&2
exit 1
"#,
            d = bin.path().display()
        );
        let path = bin.path().join("gh");
        std::fs::write(&path, script).expect("fake gh script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod gh");
        }
        let guard = pulse_testutil::setenv(
            "PATH",
            format!(
                "{}:{}",
                bin.path().display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        );
        (bin, guard)
    }

    #[test]
    fn a_call_the_active_login_cannot_make_is_made_as_the_login_that_can() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
        let (bin, _path) = rotating_gh();
        let calls = || {
            std::fs::read_to_string(bin.path().join("calls"))
                .unwrap_or_default()
                .lines()
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        let logins = || crate::gh_auth::Logins {
            active: "pat-x".into(),
            accounts: vec!["pat-x".into(), "seer-x".into()],
        };
        remember("rot/r", None);
        remember("rot/closed", None);
        let root = std::env::temp_dir().display().to_string();
        let args = ["pr", "list", "--state", "open"];
        let out = gh_with(&root, "rot/r", &args, Duration::from_secs(20), &logins);
        assert_eq!(out.as_deref().map(str::trim), Ok("[]"), "{out:?}");
        assert_eq!(
            calls(),
            vec!["pr list as=".to_string(), "pr list as=seer-x".to_string()],
            "the active login first, then the other account by token"
        );
        assert_eq!(remembered("rot/r").as_deref(), Some("seer-x"));

        // The next call goes straight to the account that answered.
        let out = gh_with(&root, "rot/r", &args, Duration::from_secs(20), &logins);
        assert_eq!(out.as_deref().map(str::trim), Ok("[]"), "{out:?}");
        assert_eq!(calls().len(), 3);
        assert_eq!(calls()[2], "pr list as=seer-x");

        // A repository no account can see names them all and no token.
        let err = gh_with(&root, "rot/closed", &args, Duration::from_secs(20), &logins)
            .expect_err("rot/closed is visible to nobody");
        assert!(
            err.contains("gh pr list failed")
                && err.contains("Could not resolve to a Repository")
                && err.contains("can see rot/closed: tried pat-x, seer-x"),
            "{err}"
        );
        assert!(!err.contains("tok-"), "token in the error: {err}");
        assert_eq!(remembered("rot/closed"), None);
    }

    #[test]
    fn gh_failure_names_a_missing_gh() {
        assert!(
            gh_failure("gh pr list", "No such file or directory (os error 2)")
                .contains("gh auth login")
        );
        assert_eq!(
            gh_failure(
                "gh pr create",
                "  HTTP 403: Resource not accessible  \n by integration "
            ),
            "gh pr create failed: HTTP 403: Resource not accessible by integration"
        );
    }

    /// The forge's judgement of an open pull request is read from both
    /// shapes a `statusCheckRollup` entry takes, and an answer without the
    /// fields (an older gh, a fake) reads as not judged.
    #[test]
    fn pr_state_reads_what_stands_between_a_pull_request_and_its_merge() {
        let st = pr_state(&serde_json::json!({
            "number": 7, "state": "OPEN", "mergedAt": null, "url": "u", "headRefName": "h",
            "headRefOid": "abc", "mergeable": "CONFLICTING", "mergeStateStatus": "DIRTY",
            "statusCheckRollup": [
                {"__typename": "CheckRun", "name": "build", "status": "COMPLETED", "conclusion": "FAILURE"},
                {"__typename": "CheckRun", "name": "lint", "status": "COMPLETED", "conclusion": "SUCCESS"},
                {"__typename": "CheckRun", "name": "slow", "status": "IN_PROGRESS", "conclusion": null},
                {"__typename": "CheckRun", "name": "late", "status": "COMPLETED", "conclusion": "TIMED_OUT"},
                {"__typename": "StatusContext", "context": "ci/legacy", "state": "ERROR"},
                {"__typename": "StatusContext", "context": "ci/ok", "state": "SUCCESS"}
            ]
        }));
        assert_eq!(st.head_sha, "abc");
        assert!(st.conflicting());
        assert_eq!(st.failing_checks, ["build", "late", "ci/legacy"]);

        let st = pr_state(&serde_json::json!({
            "number": 8, "state": "OPEN", "mergedAt": null, "url": "u", "headRefName": "h"
        }));
        assert_eq!(st.head_sha, "");
        assert!(!st.conflicting());
        assert!(st.failing_checks.is_empty());
        let st = pr_state(&serde_json::json!({
            "number": 9, "state": "OPEN", "headRefOid": "def", "mergeable": "MERGEABLE",
            "mergeStateStatus": "BLOCKED", "statusCheckRollup": []
        }));
        assert!(!st.conflicting(), "blocked on review is not a conflict");
        assert!(st.failing_checks.is_empty());
    }
}
