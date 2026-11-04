//! Pull requests through `gh`, the forge CLI the owner already logs in to
//! on this machine (its auth never reaches git): the helpers the per-task
//! pull requests use (`task_pr.rs`), and `pulse land --propose`, which
//! pushes one branch and opens a pull request for it.

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

/// Runs `gh` in the project checkout (so it finds the repository from the
/// remotes when `repo` is empty) with the daemon's own environment: `gh`
/// keeps its login in its own config, nothing of it reaches git. Returns
/// stdout, or the failure put into words.
pub(crate) fn gh(
    root: &str,
    repo: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<String, String> {
    let mut words = vec!["gh".to_string()];
    words.extend(args.iter().map(|a| crate::util::shell_quote(a)));
    if !repo.is_empty() {
        words.push("--repo".into());
        words.push(crate::util::shell_quote(repo));
    }
    let what = format!("gh {} {}", args[0], args[1]);
    let (code, out, err) = crate::land::sh_with_timeout(root.to_string(), words.join(" "), timeout);
    match code.as_deref() {
        Some(0) => Ok(out),
        Some(_) => Err(gh_failure(&what, &format!("{err} {out}"))),
        None => Err(format!(
            "{what} did not finish within {}s",
            timeout.as_secs()
        )),
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

/// `pulse land --branch B --propose`: the branch itself pushed to the
/// project's upstream and a pull request opened from it into the upstream
/// default branch - nothing is merged locally. The review lead (or the
/// owner) reviews and merges it on the forge; the daemon's next push pass
/// fast-forwards the local default branch from the upstream. Asked for by
/// the owner on 2026-11-04: land through a reviewed pull request, not
/// straight into main. Returns the pull request url, or the failure in words.
pub(crate) fn propose_branch(
    s: &pulse_store::Store,
    project: &crate::util::Row,
    branch: &str,
    actor: &str,
    now: &str,
) -> Result<String, String> {
    let root = project.str("path");
    let def = {
        let d = project.str("default_branch");
        if d.is_empty() { "main".to_string() } else { d }
    };
    if branch.is_empty() || branch == def {
        return Err(format!("--propose needs a branch other than {def}"));
    }
    let Some((remote, _, remote_branch)) = crate::forge::tracking_of(&root, &def) else {
        return Err(format!(
            "{def} has no upstream (git branch --set-upstream-to=origin/{def} {def})"
        ));
    };
    let git = |timeout: u64, env: &[String], args: &[&str]| {
        crate::util::git_timeout_env(&root, Duration::from_secs(timeout), env, args)
    };
    let local = format!("refs/heads/{branch}");
    if !git(10, &[], &["rev-parse", "--verify", "-q", &local]).ok() {
        return Err(format!("no branch {branch} in {root}"));
    }
    let ahead = git(
        30,
        &[],
        &[
            "rev-list",
            "--count",
            &format!("refs/remotes/{remote}/{remote_branch}..{local}"),
        ],
    );
    if ahead.stdout.trim() == "0" {
        return Err(format!(
            "{branch} has nothing {remote}/{remote_branch} lacks"
        ));
    }
    let (env, cleanup, why) = crate::forge::push_credential_env(s, root.clone());
    if !why.is_empty() {
        return Err(format!("push credential: {why}"));
    }
    let pushed = git(
        180,
        &env,
        &[
            "push",
            "--force-with-lease",
            &remote,
            &format!("{local}:{local}"),
        ],
    );
    cleanup("", &[]);
    if !pushed.ok() {
        let why = pushed.err.clone().unwrap_or_else(|| {
            crate::run::squeeze(&format!("{} {}", pushed.stderr, pushed.stdout), 300)
        });
        return Err(format!(
            "push of {branch} failed: {}",
            pulse_cred::redact(&why, &[])
        ));
    }
    let url = git(10, &[], &["remote", "get-url", &remote])
        .stdout
        .trim()
        .to_string();
    let repo = repo_slug(&url).unwrap_or_default();
    if let Some((_, url)) = open_pull_request(&root, &repo, branch, &remote_branch)? {
        return Ok(url);
    }
    let log = git(
        30,
        &[],
        &[
            "log",
            "--format=- %s",
            &format!("refs/remotes/{remote}/{remote_branch}..{local}"),
        ],
    )
    .stdout;
    let title = log
        .lines()
        .last()
        .map(|l| l.trim_start_matches("- ").to_string())
        .unwrap_or_else(|| branch.to_string());
    let body = format!(
        "Branch `{branch}` of {} proposed by PULSE for review.\n\n{}\nThe review lead reviews it, runs the project's check and merges it; nothing reached `{def}` yet.\n",
        project.str("slug"),
        log
    );
    let out = gh(
        &root,
        &repo,
        &[
            "pr",
            "create",
            "--head",
            branch,
            "--base",
            &remote_branch,
            "--title",
            &crate::util::trunc(&title, 120),
            "--body",
            &body,
        ],
        Duration::from_secs(90),
    )?;
    let url = out
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| l.starts_with("http"))
        .unwrap_or_default()
        .to_string();
    if let Ok(conn) = s.conn() {
        crate::util::event_at(
            &conn,
            now,
            actor,
            "land.proposed",
            &project.str("slug"),
            serde_json::json!({"project": project.str("slug"), "branch": branch, "base": remote_branch, "url": url}),
        );
    }
    Ok(url)
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
}
