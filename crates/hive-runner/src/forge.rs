//! Git and forge plumbing the delivery pass (`delivery.rs`) and the local
//! landings that remain (`hive land --local`, projects without an
//! upstream) share: the default branch's upstream, the daemon's push
//! credential, the fetch of the upstream and the fast-forward of the local
//! default branch, which under pull-request delivery only ever follows the
//! upstream - nothing merges into it on this machine.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The url a push to `remote` uses: its push url, else its fetch url, else
/// the name itself (git resolves it the same way).
pub(crate) fn push_url(root: &str, remote: &str) -> String {
    for args in [
        ["remote", "get-url", "--push", remote].as_slice(),
        ["remote", "get-url", remote].as_slice(),
    ] {
        let r = crate::util::git_timeout(root, Duration::from_secs(10), args);
        if r.ok() && !r.stdout.trim().is_empty() {
            return r.stdout.trim().to_string();
        }
    }
    remote.to_string()
}

/// Fetches the upstream's default branch into its tracking ref, with `env`
/// carrying the daemon's credential (`push_credential_env`).
pub(crate) fn fetch_upstream(root: &str, up: &Upstream, env: &[String]) -> crate::util::GitResult {
    git_upstream(
        root,
        Duration::from_secs(120),
        env,
        &up.url,
        &[
            "fetch",
            "--quiet",
            "--no-tags",
            &up.url,
            &format!("+refs/heads/{}:{}", up.remote_branch, up.upstream_ref),
        ],
    )
}

/// Where a project's pull requests go: the remote the default branch
/// tracks, its branch there and its url, with the `OWNER/REPO` gh names it
/// when the url is a GitHub one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Upstream {
    pub(crate) remote: String,
    pub(crate) remote_branch: String,
    /// `refs/remotes/<remote>/<branch>`, the tracking ref a fetch updates.
    pub(crate) upstream_ref: String,
    pub(crate) url: String,
    /// `OWNER/REPO` for `gh --repo`; empty when the url is not GitHub's
    /// (gh then finds the repository from the checkout's remotes itself).
    pub(crate) repo: String,
}

impl Upstream {
    /// `origin/main`, how the upstream is named in log lines and events.
    pub(crate) fn name(&self) -> String {
        format!("{}/{}", self.remote, self.remote_branch)
    }
}

/// The project's upstream: what the default branch tracks, else the
/// `origin` remote with the branch of the same name (a checkout cloned
/// without `-u` tracks nothing yet pushes there), else None - a project
/// with no remote at all (two of the fifteen live ones on 2026-11-04,
/// both on a feature branch) can have no pull request and lands locally.
pub(crate) fn upstream_of(root: &str, def: &str) -> Option<Upstream> {
    let (remote, remote_branch) = match tracking_of(root, def) {
        Some((remote, _, remote_branch)) => (remote, remote_branch),
        None => {
            let origin = crate::util::git_timeout(
                root,
                Duration::from_secs(10),
                &["remote", "get-url", "origin"],
            );
            if !origin.ok() || origin.stdout.trim().is_empty() {
                return None;
            }
            ("origin".to_string(), def.to_string())
        }
    };
    let url = push_url(root, &remote);
    if url == remote {
        return None;
    }
    Some(Upstream {
        upstream_ref: format!("refs/remotes/{remote}/{remote_branch}"),
        repo: crate::landing_pr::repo_slug(&url).unwrap_or_default(),
        remote,
        remote_branch,
        url,
    })
}

/// The default branch's upstream as git has it: (remote, remote ref,
/// remote branch), or None when the branch tracks nothing.
pub(crate) fn tracking_of(root: &str, def: &str) -> Option<(String, String, String)> {
    let up = crate::util::git_timeout(
        root,
        Duration::from_secs(10),
        &[
            "for-each-ref",
            "--format=%(upstream:remotename) %(upstream:remoteref)",
            &format!("refs/heads/{def}"),
        ],
    )
    .stdout
    .split_whitespace()
    .map(ToOwned::to_owned)
    .collect::<Vec<_>>();
    if up.len() != 2 {
        return None;
    }
    let remote_branch = up[1]
        .strip_prefix("refs/heads/")
        .unwrap_or(&up[1])
        .to_string();
    Some((up[0].clone(), up[1].clone(), remote_branch))
}

/// Whether the default branch's tracking ref holds commits the local
/// default branch lacks (no network: what the last fetch brought).
pub(crate) fn upstream_ahead_of_default(root: &str, def: &str) -> bool {
    let Some((remote, _, remote_branch)) = tracking_of(root, def) else {
        return false;
    };
    crate::util::git_timeout(
        root,
        Duration::from_secs(30),
        &[
            "rev-list",
            "--count",
            &format!("refs/heads/{def}..refs/remotes/{remote}/{remote_branch}"),
        ],
    )
    .stdout
    .trim()
    .parse::<i32>()
    .unwrap_or(0)
        > 0
}

fn fast_forward_failed_note(why: &str, upstream: &str) -> String {
    format!("fast-forward to {upstream} failed: {why}")
}

/// A remote merge (e.g. a human merged on the forge) reached the tracking
/// ref while the local default branch stayed put; move it with the merge's
/// own commits so the brain sees them this pass instead of waiting for a
/// local landing. A clean checkout of the default branch is fast-forwarded;
/// a checkout on another branch (or detached) has the default branch moved
/// by ref, as a landing does, and its working tree is not touched; a dirty
/// checkout of the default branch keeps its HEAD and `sync_upstream`
/// handles the remote commits as on any other pass.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fast_forward_default(
    s: &hive_store::Store,
    root: &str,
    slug: &str,
    def: &str,
    name: &str,
    up_ref: &str,
    actor: &str,
    now: &str,
    behind: i32,
) -> Option<String> {
    let (on_default, clean) = checkout_state(root, def);
    if on_default && !clean {
        return None;
    }
    let rev = |r: &str| {
        crate::util::git_timeout(root, Duration::from_secs(10), &["rev-parse", r])
            .stdout
            .trim()
            .to_string()
    };
    let from = rev(def);
    let to = rev(up_ref);
    let record = |ok: bool, why: &str, how: &str| {
        if let Ok(conn) = s.conn() {
            let mut detail = serde_json::json!({"project": slug, "branch": def, "upstream": name, "behind": behind, "ok": ok});
            if ok {
                detail["from"] = serde_json::json!(crate::recipe::short(&from, 12));
                detail["to"] = serde_json::json!(crate::recipe::short(&to, 12));
                detail["how"] = serde_json::json!(how);
            } else {
                detail["why"] = serde_json::json!(why);
            }
            crate::util::event_at(&conn, now, actor, "land.fast-forward", slug, detail);
        }
    };
    if !on_default {
        if from.is_empty() || to.is_empty() {
            return None;
        }
        // Only a fast-forward moves by ref; a diverged branch is the
        // sync's to merge.
        let ancestor = crate::util::git_timeout(
            root,
            Duration::from_secs(20),
            &["merge-base", "--is-ancestor", &from, &to],
        );
        if !ancestor.ok() {
            return None;
        }
        let moved = crate::util::git_timeout(
            root,
            Duration::from_secs(30),
            &[
                "update-ref",
                "-m",
                &format!("hive: fast-forward of {def} to {name} by ref"),
                &format!("refs/heads/{def}"),
                &to,
                &from,
            ],
        );
        if !moved.ok() {
            let why = hive_cred::redact(&crate::run::squeeze(moved.stderr.trim(), 160), &[]);
            record(false, &why, "");
            return Some(fast_forward_failed_note(&why, name));
        }
        record(true, "", "ref");
        return Some(format!(
            "landing fast-forwarded {slug}: {def} -> {name} ({behind} {}) by ref; the checkout is on another branch and was not touched",
            crate::util::plural(behind, "commit"),
        ));
    }
    let ff = crate::util::git_timeout(
        root,
        Duration::from_secs(120),
        &["merge", "--ff-only", up_ref],
    );
    if !ff.ok() {
        let why = crate::run::squeeze(&format!("{} {}", ff.stderr, ff.stdout), 160);
        let why = hive_cred::redact(&why, &[]);
        record(false, &why, "");
        return Some(fast_forward_failed_note(&why, name));
    }
    record(true, "", "checkout");
    Some(format!(
        "landing fast-forwarded {slug}: {def} -> {name} ({behind} {})",
        crate::util::plural(behind, "commit"),
    ))
}

/// How long a fetch of the upstream is trusted before the next pass
/// fetches again: the maintenance pass runs about every two minutes, and a
/// merge on the forge is followed up by its own fetch (`settle_merged`), so
/// the per-pass fetch is for what others push.
const FETCH_MEMO: Duration = Duration::from_secs(120);
/// A dirty checkout of the default branch that cannot be fast-forwarded is
/// said once an hour per project, not every pass.
const SYNC_SKIP_MEMO: Duration = Duration::from_secs(3600);

/// When each project root was last fetched, and when each project was last
/// told its checkout holds back the fast-forward.
static SYNC_MEMO: Mutex<BTreeMap<String, Instant>> = Mutex::new(BTreeMap::new());

fn memo_elapsed(key: &str) -> Option<Duration> {
    SYNC_MEMO
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(key)
        .map(Instant::elapsed)
}

fn memo_now(key: &str) {
    SYNC_MEMO
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key.to_string(), Instant::now());
}

/// Forgets the fetch and skip memos, so a test's second pass fetches.
#[cfg(test)]
pub(crate) fn forget_sync_memo() {
    SYNC_MEMO.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// Brings the local default branch up to its upstream: fetches the
/// upstream (skipped when the last fetch was under `FETCH_MEMO` ago and
/// `force` is false) and fast-forwards the local branch to it. Under
/// pull-request delivery nothing merges into the local default branch on
/// this machine, so this is the only way merged work reaches the checkout,
/// and the brain project's self-update reads from it. A clean checkout
/// of the default branch is fast-forwarded in place; a checkout on another
/// branch has the default branch moved by ref; a dirty checkout of the
/// default branch is stashed around the fast-forward when
/// `landing.auto_stash` is on, else left alone with a `land.sync-skipped`
/// event at most once an hour. Returns the log line when something was
/// done or could not be.
pub(crate) fn sync_default_from_upstream(
    s: &hive_store::Store,
    project: &crate::util::Row,
    actor: &str,
    now: &str,
    force: bool,
) -> Option<String> {
    let root = project.str("path");
    let slug = project.str("slug");
    let def = crate::baseline::default_branch(project.clone());
    let up = upstream_of(&root, &def)?;
    let fetch_key = format!("fetch:{root}");
    if !force && memo_elapsed(&fetch_key).is_some_and(|age| age < FETCH_MEMO) {
        return None;
    }
    memo_now(&fetch_key);
    let (env, cleanup, why) = remote_credential_env(s, root.clone(), &up.url);
    if !why.is_empty() {
        return Some(format!("{slug}: fetch of {} skipped: {why}", up.name()));
    }
    let fetched = fetch_upstream(&root, &up, &env);
    cleanup("", &[]);
    if !fetched.ok() {
        return Some(format!(
            "{slug}: fetch of {} failed: {}",
            up.name(),
            hive_cred::redact(&crate::run::squeeze(&fetched.stderr, 160), &[])
        ));
    }
    let behind = crate::util::git_timeout(
        &root,
        Duration::from_secs(30),
        &[
            "rev-list",
            "--count",
            &format!("refs/heads/{def}..{}", up.upstream_ref),
        ],
    )
    .stdout
    .trim()
    .parse::<i32>()
    .unwrap_or(0);
    if behind == 0 {
        return None;
    }
    let name = up.name();
    let (on_default, clean) = checkout_state(&root, &def);
    if on_default && !clean {
        if s.setting("landing.auto_stash", "on") == "off" {
            let skip_key = format!("skip:{slug}");
            if memo_elapsed(&skip_key).is_some_and(|age| age < SYNC_SKIP_MEMO) {
                return None;
            }
            memo_now(&skip_key);
            if let Ok(conn) = s.conn() {
                crate::util::event_at(
                    &conn,
                    now,
                    actor,
                    "land.sync-skipped",
                    &slug,
                    serde_json::json!({"project": slug, "branch": def, "upstream": name, "behind": behind,
                        "why": "the checkout has uncommitted changes and landing.auto_stash is off"}),
                );
            }
            return Some(format!(
                "{slug}: {def} is {behind} {} behind {name} and the checkout at {root} has uncommitted changes; commit or stash them (landing.auto_stash is off)",
                crate::util::plural(behind, "commit")
            ));
        }
        let mut stash = match crate::land::AutoStash::push(
            &root,
            &format!("hive auto-stash before fast-forward of {def} to {name}"),
        ) {
            Ok(st) => st,
            Err(why) => {
                return Some(format!(
                    "{slug}: {def} is {behind} {} behind {name}; the checkout's uncommitted changes could not be stashed: {why}",
                    crate::util::plural(behind, "commit")
                ));
            }
        };
        let line = fast_forward_default(
            s,
            &root,
            &slug,
            &def,
            &name,
            &up.upstream_ref,
            actor,
            now,
            behind,
        );
        let popped = stash.pop();
        return Some(match (line, popped) {
            (Some(l), Some(note)) => format!("{l}; {note}"),
            (Some(l), None) => l,
            (None, Some(note)) => note,
            (None, None) => format!("{slug}: {def} not fast-forwarded to {name}"),
        });
    }
    fast_forward_default(
        s,
        &root,
        &slug,
        &def,
        &name,
        &up.upstream_ref,
        actor,
        now,
        behind,
    )
}

/// Whether HEAD is the default branch and whether the tracked files are
/// clean (untracked files do not hold a fast-forward back).
pub(crate) fn checkout_state(root: &str, def: &str) -> (bool, bool) {
    let head = crate::util::git_timeout(
        root,
        Duration::from_secs(10),
        &["rev-parse", "--abbrev-ref", "HEAD"],
    );
    let status = crate::util::git_timeout(
        root,
        Duration::from_secs(60),
        &["status", "--porcelain", "--untracked-files=no"],
    );
    (
        head.ok() && head.stdout.trim() == def,
        status.ok() && status.stdout.trim().is_empty(),
    )
}

#[allow(clippy::type_complexity)]
pub(crate) fn push_credential_env(
    s: &hive_store::Store,
    root: String,
) -> (
    Vec<String>,
    Box<dyn Fn(&str, &[serde_json::Value]) + Send + Sync>,
    String,
) {
    let ref_ = s.setting("push.credential", "").trim().to_string();
    if ref_.is_empty() {
        return (push_env(root), Box::new(|_, _| {}), String::new());
    }
    let parsed = match hive_cred::parse(&ref_) {
        Ok(v) => v,
        Err(e) => return (Vec::new(), Box::new(|_, _| {}), e.to_string()),
    };
    let resolver = hive_cred::new(&s.home);
    let secret = match resolver.resolve(parsed) {
        Ok(v) => v,
        Err(e) => {
            let why = hive_cred::redact(&e.to_string(), &[]).replace(
                ": No such file or directory (os error 2)",
                ": no such file or directory",
            );
            return (Vec::new(), Box::new(|_, _| {}), why);
        }
    };
    let material = match resolver.materialize(&secret) {
        Ok(v) => v,
        Err(e) => {
            return (
                Vec::new(),
                Box::new(|_, _| {}),
                format!(
                    "could not write the key for the push: {}",
                    hive_cred::redact(&e.to_string(), &[])
                ),
            );
        }
    };
    let env = vec![format!("GIT_SSH_COMMAND={}", material.ssh_command)];
    (env, Box::new(move |_, _| material.cleanup()), String::new())
}

/// The environment a brain-side fetch or push of `url` runs with: the
/// GitHub App's installation token for that one repository through a
/// credential helper that lives until the cleanup is called (ADR-070
/// step 3), when an App is configured and `url` is on its host; else
/// `push_credential_env`, today's path. A token that cannot be minted is
/// written down as `gitapp.fallback` (the reason, never a secret) and the
/// command falls back to `push.credential` and this machine's logins.
#[allow(clippy::type_complexity)]
pub(crate) fn remote_credential_env(
    s: &hive_store::Store,
    root: String,
    url: &str,
) -> (
    Vec<String>,
    Box<dyn Fn(&str, &[serde_json::Value]) + Send + Sync>,
    String,
) {
    match crate::gitapp::token_for_url(s, url) {
        None => {}
        Some(Ok((repo, token))) => match crate::gitapp::helper_env(s, &token, &repo) {
            Ok((helper, env)) => {
                let helper = Mutex::new(Some(helper));
                return (
                    env,
                    Box::new(move |_, _| {
                        helper.lock().unwrap_or_else(|e| e.into_inner()).take();
                    }),
                    String::new(),
                );
            }
            Err(why) => app_fallback(s, &repo, &why),
        },
        Some(Err(why)) => {
            let repo = crate::gitapp::app_remote(url)
                .map(|(_, r)| r)
                .unwrap_or_default();
            app_fallback(s, &repo, &why);
        }
    }
    push_credential_env(s, root)
}

fn app_fallback(s: &hive_store::Store, repo: &str, why: &str) {
    let _ = s.event(
        "daemon",
        "gitapp.fallback",
        repo,
        serde_json::json!({
            "repo": repo,
            "why": hive_cred::redact(&crate::executor::redact_tokens(why), &[]),
        }),
    );
}

/// The ssh form of an https url on GitHub, the one forge whose ssh form is
/// known here: `https://github.com/OWNER/REPO`, with `.git` or without, is
/// `git@github.com:OWNER/REPO.git`. None for anything else, and the url is
/// read strictly, because a push must never go anywhere but where the url
/// points. Git ends the host at the first `/`, `?` or `#`, so
/// `https://HOST?@github.com/OWNER/REPO` goes to HOST while a reader that
/// looks past the `@` sees GitHub. So the url must be GitHub's own address,
/// one `/` and the two names, with nothing else in it - no user, no port,
/// no query, no fragment, no further path - and the names hold letters,
/// digits, `.`, `_` and `-` alone.
pub(crate) fn ssh_url(url: &str) -> Option<String> {
    let forge = crate::gitapp::endpoint();
    let path = url
        .strip_prefix(forge.web.trim_end_matches('/'))?
        .strip_prefix('/')?;
    let (owner, repo) = path.strip_suffix(".git").unwrap_or(path).split_once('/')?;
    let plain = |name: &str| {
        !name.starts_with('-')
            && name.bytes().any(|b| b != b'.')
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    };
    let host = forge.git_host();
    let host = host.split(':').next().unwrap_or(&host);
    (plain(owner) && plain(repo)).then(|| format!("git@{host}:{owner}/{repo}.git"))
}

/// Whether `env` holds the daemon's own key: `push_credential_env` with
/// `push.credential` set is the one environment here whose ssh command
/// names an identity file.
fn has_push_key(env: &[String]) -> bool {
    env.iter().any(|kv| {
        kv.strip_prefix("GIT_SSH_COMMAND=")
            .is_some_and(|ssh| ssh.contains(" -i "))
    })
}

/// Whether `env` holds a GitHub App token, as `gitapp::helper_env` hands
/// it out: the one environment here that names a credential helper.
fn has_app_token(env: &[String]) -> bool {
    env.iter()
        .any(|kv| kv.starts_with("GIT_CONFIG_KEY_") && kv.ends_with("=credential.helper"))
}

/// Whether git's words say an http(s) remote asked for a credential and
/// git had none to give: it may not prompt here (`hive_gitenv::env`), so it
/// could not read a username or a password. Which remote asked is
/// `github_asked`'s to say.
fn no_https_credential(why: &str) -> bool {
    [
        "could not read Username",
        "could not read Password",
        "unable to get password from user",
    ]
    .iter()
    .any(|said| why.contains(said))
}

/// Whether all git said is that GitHub asked for a credential git had
/// none to give: its last line is `fatal: could not read Username for
/// 'https://github.com': terminal prompts disabled`, and before it stands
/// at most its notice that GitHub sent it to another address of GitHub's
/// (`warning: redirecting to https://github.com/...`).
///
/// One such line among others proves nothing, because the host git
/// reached writes part of what git prints: a server's error comes out as
/// `fatal: remote error:` and the server's own words, line breaks
/// included, so a host can end git's answer with this very line. And the
/// host is not always the url's: the checkout's own config may send the
/// url elsewhere (`url.*.insteadOf`), and a server may redirect it. An
/// answer of nothing but these lines is git's own, about GitHub. Words
/// that name no address, another address, or the address with the
/// repository's path (`credential.useHttpPath`) are not that answer.
fn github_asked(why: &str) -> bool {
    let web = crate::gitapp::endpoint().web;
    let web = web.trim_end_matches('/');
    let asked = format!("fatal: could not read Username for '{web}': terminal prompts disabled");
    let redirected = format!("warning: redirecting to {web}/");
    let mut lines = why.lines().filter(|line| !line.trim().is_empty());
    lines.next_back().is_some_and(|last| last == asked)
        && lines.all(|line| line.starts_with(&redirected))
}

/// Whether git asks `url` as it is given, and not another the checkout's
/// own config puts in its place (`url.*.insteadOf`): what `git ls-remote
/// --get-url` answers, which reads the config and calls nobody. A host
/// the url is sent to can send git on to GitHub, and GitHub's question for
/// a credential is then git's own answer, whole (`github_asked`), although
/// the command went elsewhere first.
fn asks_as_given(root: &str, env: &[String], url: &str) -> bool {
    let asked = crate::util::git_timeout_env(
        root,
        Duration::from_secs(10),
        env,
        &["ls-remote", "--get-url", url],
    );
    asked.ok() && asked.stdout.trim() == url
}

/// Runs one git command against the upstream `url`, which is one of
/// `args`, with `env` as `remote_credential_env` chose it. Git offers a key
/// only over ssh, so with the daemon's own key (`push.credential`) a push
/// or fetch of an https url fails for want of a credential however good
/// the key is: nine deliveries of one project were held that way over
/// three days. When all git says is that GitHub asked for a credential it
/// has not got (`github_asked`), the url is GitHub's and git asks it as it
/// is given (`asks_as_given`), the command is made once more against the
/// url's ssh form (`ssh_url`), where the key counts. A command that
/// succeeds over https is not touched, and nothing is written to the
/// checkout's config.
pub(crate) fn git_upstream(
    root: &str,
    timeout: Duration,
    env: &[String],
    url: &str,
    args: &[&str],
) -> crate::util::GitResult {
    git_upstream_said(root, timeout, env, url, args).0
}

/// [`git_upstream`], with what git said over https when the command was
/// made again over ssh, for the note of a retry that fails too
/// (`failed_after`).
pub(crate) fn git_upstream_said(
    root: &str,
    timeout: Duration,
    env: &[String],
    url: &str,
    args: &[&str],
) -> (crate::util::GitResult, Option<String>) {
    let done = crate::util::git_timeout_env(root, timeout, env, args);
    if done.ok() || !has_push_key(env) || !github_asked(&done.stderr) {
        return (done, None);
    }
    let Some(ssh) = ssh_url(url).filter(|_| asks_as_given(root, env, url)) else {
        return (done, None);
    };
    let args: Vec<&str> = args
        .iter()
        .map(|a| if *a == url { ssh.as_str() } else { *a })
        .collect();
    (
        crate::util::git_timeout_env(root, timeout, env, &args),
        Some(done.stderr),
    )
}

/// How a note says a command on the upstream failed: `failed`, and when it
/// was made again over ssh (`https` is what git said first), both ways
/// with https's words. The owner knows the remote as an https one, and a
/// note with ssh's words alone - a port that is closed, say - would name a
/// way the remote does not show.
pub(crate) fn failed_after(https: Option<&str>) -> String {
    match https {
        Some(said) => format!(
            "failed over https ({}), then over ssh with the daemon's key",
            crate::run::squeeze(said, 120)
        ),
        None => "failed".into(),
    }
}

/// How to fix a push that failed for an authentication reason, as one
/// sentence that ends its hold's note; empty when git's words (`why`) name
/// another cause. `env` is what the push ran with.
pub(crate) fn auth_fix(up: &Upstream, env: &[String], why: &str) -> String {
    // Over https the forge wants a credential git has not got, or refuses
    // the one it has; over ssh it does not know the key, or the key may
    // not write here.
    let https = no_https_credential(why)
        || why.contains("Authentication failed")
        || why.contains("returned error: 403");
    let key = why.contains("Permission denied (publickey")
        || (why.contains("Permission to ") && why.contains(" denied to "));
    let fix = if https && has_app_token(env) {
        "GitHub refused the App's token: install the GitHub App on this repository with read and write access to its contents".into()
    } else if https {
        format!(
            "git has no credential that opens this https url: give {remote} an ssh push url (git remote set-url --push {remote} {})",
            ssh_url(&up.url).unwrap_or_else(|| "SSH-URL".into()),
            remote = up.remote
        )
    } else if key {
        if has_push_key(env) {
            "the forge refused the key push.credential names: add its public key to the repository as a deploy key with write access, or name a key that may push with hive cred set".into()
        } else {
            "the forge refused this machine's ssh keys: give the daemon a key of its own with hive cred set".into()
        }
    } else if why.contains("Host key verification failed") {
        "ssh does not know the forge's host key: connect to the forge over ssh once by hand, as the user the daemon runs as, and accept it".into()
    } else {
        return String::new();
    };
    format!("; {fix}")
}

/// The push credential leased to a hand for one push (ADR-070 step 1b):
/// the key `push.credential` names, resolved here on the brain, or empty
/// when the setting is unset (the hand pushes with what git there already
/// uses). Err says why the reference could not be read, never the key.
pub(crate) fn push_credential_key(s: &hive_store::Store) -> Result<Vec<u8>, String> {
    let ref_ = s.setting("push.credential", "").trim().to_string();
    if ref_.is_empty() {
        return Ok(Vec::new());
    }
    let parsed = hive_cred::parse(&ref_).map_err(|e| e.to_string())?;
    hive_cred::new(&s.home)
        .resolve(parsed)
        .map(|secret| secret.key)
        .map_err(|e| hive_cred::redact(&e.to_string(), &[]))
}

pub(crate) fn push_env(root: String) -> Vec<String> {
    if std::env::var("GIT_SSH_COMMAND").unwrap_or_default() != ""
        || !crate::util::git_timeout(
            &root,
            Duration::from_secs(10),
            &["config", "--get", "core.sshCommand"],
        )
        .stdout
        .trim()
        .is_empty()
    {
        Vec::new()
    } else {
        vec!["GIT_SSH_COMMAND=ssh -oBatchMode=yes".into()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::*;
    use crate::test_helpers_daemon::*;
    use std::path::Path;

    /// A project whose local main is one commit behind its origin: the
    /// commit is pushed and then dropped locally.
    fn behind_fixture(s: &hive_store::Store) -> (crate::util::Row, String, String) {
        let (proj, repo, bare) = push_fixture(s);
        std::fs::write(Path::new(&repo).join("upstream.txt"), "from upstream\n").unwrap();
        run(&repo, &["git", "add", "-A"]);
        run(
            &repo,
            &["git", "commit", "-q", "-m", "pushed by someone else"],
        );
        run(&repo, &["git", "push", "-q", "origin", "main"]);
        run(&repo, &["git", "reset", "-q", "--hard", "HEAD~1"]);
        (proj, repo, bare)
    }

    fn count_events(s: &hive_store::Store, action: &str) -> i32 {
        count(
            s,
            "SELECT COUNT(*) AS n FROM events WHERE action = ?",
            &[&action],
        )
    }

    #[test]
    fn a_clean_checkout_of_the_default_branch_follows_its_upstream_once_per_memo() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = new_store();
        let (proj, repo, _bare) = behind_fixture(&s);
        forget_sync_memo();
        let line = sync_default_from_upstream(&s, &proj, "daemon", TEST_NOW, false);
        assert!(
            line.as_deref()
                .is_some_and(|l| l.contains("fast-forwarded proj: main -> origin/main (1 commit)")),
            "{line:?}"
        );
        assert!(Path::new(&repo).join("upstream.txt").exists());
        assert_eq!(count_events(&s, "land.fast-forward"), 1);
        assert_eq!(count_events(&s, "land.sync-skipped"), 0);

        // Another commit upstream within the memo window waits for the
        // next fetch; a forced sync (after a merge) fetches at once.
        std::fs::write(Path::new(&repo).join("second.txt"), "second\n").unwrap();
        run(&repo, &["git", "add", "-A"]);
        run(&repo, &["git", "commit", "-q", "-m", "second"]);
        run(&repo, &["git", "push", "-q", "origin", "main"]);
        run(&repo, &["git", "reset", "-q", "--hard", "HEAD~1"]);
        assert_eq!(
            sync_default_from_upstream(&s, &proj, "daemon", TEST_NOW, false),
            None
        );
        assert!(!Path::new(&repo).join("second.txt").exists());
        let line = sync_default_from_upstream(&s, &proj, "daemon", TEST_NOW, true);
        assert!(line.is_some(), "forced sync");
        assert!(Path::new(&repo).join("second.txt").exists());
        assert_eq!(count_events(&s, "land.fast-forward"), 2);
    }

    #[test]
    fn a_dirty_checkout_is_told_once_an_hour_unless_auto_stash_is_on() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = new_store();
        let (proj, repo, _bare) = behind_fixture(&s);
        std::fs::write(Path::new(&repo).join("README.md"), "# edited by hand\n").unwrap();
        set_setting(&s, "landing.auto_stash", "off");
        forget_sync_memo();
        let line = sync_default_from_upstream(&s, &proj, "daemon", TEST_NOW, false);
        assert!(
            line.as_deref()
                .is_some_and(|l| l.contains("main is 1 commit behind origin/main")
                    && l.contains("uncommitted changes")),
            "{line:?}"
        );
        assert!(!Path::new(&repo).join("upstream.txt").exists());
        assert_eq!(count_events(&s, "land.sync-skipped"), 1);
        assert_eq!(
            sync_default_from_upstream(&s, &proj, "daemon", TEST_NOW, true),
            None,
            "said twice within the hour"
        );
        assert_eq!(count_events(&s, "land.sync-skipped"), 1);

        set_setting(&s, "landing.auto_stash", "on");
        let line = sync_default_from_upstream(&s, &proj, "daemon", TEST_NOW, true);
        assert!(
            line.as_deref()
                .is_some_and(|l| l.contains("fast-forwarded proj: main -> origin/main")),
            "{line:?}"
        );
        assert!(Path::new(&repo).join("upstream.txt").exists());
        assert_eq!(
            std::fs::read_to_string(Path::new(&repo).join("README.md")).unwrap(),
            "# edited by hand\n",
            "the hand edit did not survive the stash"
        );
        assert_eq!(count_events(&s, "land.fast-forward"), 1);
        assert!(
            run(&repo, &["git", "stash", "list"]).trim().is_empty(),
            "stash left behind"
        );
    }

    #[test]
    fn a_checkout_on_another_branch_has_its_default_branch_moved_by_ref() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = new_store();
        let (proj, repo, _bare) = behind_fixture(&s);
        run(&repo, &["git", "checkout", "-q", "-b", "work/elsewhere"]);
        std::fs::write(
            Path::new(&repo).join("README.md"),
            "# dirty on another branch\n",
        )
        .unwrap();
        forget_sync_memo();
        let line = sync_default_from_upstream(&s, &proj, "daemon", TEST_NOW, false);
        assert!(
            line.as_deref()
                .is_some_and(|l| l.contains("fast-forwarded proj: main -> origin/main")),
            "{line:?}"
        );
        assert_eq!(
            run(&repo, &["git", "rev-parse", "refs/heads/main"]).trim(),
            run(&repo, &["git", "rev-parse", "refs/remotes/origin/main"]).trim()
        );
        assert_eq!(
            run(&repo, &["git", "rev-parse", "--abbrev-ref", "HEAD"]).trim(),
            "work/elsewhere"
        );
        assert_eq!(
            std::fs::read_to_string(Path::new(&repo).join("README.md")).unwrap(),
            "# dirty on another branch\n"
        );
    }

    /// A project whose `origin` is an http url that answers as GitHub's
    /// does, with nothing unless a credential comes along, while the same
    /// repository takes git over ssh: the fake `ssh` on PATH runs git's
    /// command where the repository is served.
    struct HttpsOrigin {
        proj: crate::util::Row,
        repo: String,
        url: String,
        /// The bare repository behind both urls.
        served: String,
        /// Where the fake ssh writes the arguments of its last call.
        ssh_record: String,
        http: hive_testutil::githttp::Server,
        _keep: (
            tempfile::TempDir,
            crate::gitapp::tests::EndpointGuard,
            Vec<hive_testutil::SetenvGuard>,
        ),
    }

    /// None where `git http-backend` is not installed.
    fn https_origin(s: &hive_store::Store) -> Option<HttpsOrigin> {
        if !hive_testutil::githttp::available() {
            return None;
        }
        let (proj, repo, bare) = push_fixture(s);
        let dir = tempfile::tempdir().unwrap();
        let served = dir.path().join("owner").join("repo.git");
        std::fs::create_dir_all(served.parent().unwrap()).unwrap();
        let served = served.display().to_string();
        run(".", &["git", "clone", "-q", "--bare", &bare, &served]);
        let http = hive_testutil::githttp::Server::new(dir.path(), "Basic nothing-has-this");
        let endpoint = crate::gitapp::tests::point_at(&http.url(), &http.url());
        let url = format!("{}/owner/repo.git", http.url());
        run(&repo, &["git", "remote", "set-url", "origin", &url]);
        let (ssh_record, mut env) = fake_ssh();
        env.push(hive_testutil::setenv("HIVE_TEST_SSH_ROOT", dir.path()));
        Some(HttpsOrigin {
            proj,
            repo,
            url,
            served,
            ssh_record,
            http,
            _keep: (dir, endpoint, env),
        })
    }

    /// The daemon's own key cannot travel over https. An upstream whose
    /// url is an https one is reached over ssh with it instead: branches
    /// are pushed and their pull requests open, the heads and the default
    /// branch are fetched, a merged branch is deleted, and the checkout's
    /// remote stays as it was.
    #[test]
    fn an_https_upstream_is_reached_over_ssh_with_the_daemons_key() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = fake_gh();
        let s = new_store();
        let Some(origin) = https_origin(&s) else {
            return;
        };
        set_setting(
            &s,
            "push.credential",
            &format!("file://{}", key_file(0o600)),
        );
        let job = push_branch(&s, &origin.proj, &origin.repo, "hive/worker/a");
        let tip = ref_of(&origin.repo, "refs/heads/hive/worker/a");

        forget_sync_memo();
        let lines = crate::delivery::tend_project(&s, &origin.proj, TEST_NOW);
        assert_eq!(
            lines,
            [format!(
                "job pr proj job {}: #7 opened https://github.com/o/r/pull/7",
                job.str("id")
            )]
        );
        assert_eq!(bare_ref(&origin.served, "refs/heads/hive/worker/a"), tip);
        // The url was tried as it is first and no credential went over
        // http; ssh was handed the key for the seconds of the call.
        let seen = origin.http.seen();
        assert!(
            !seen.is_empty() && seen.iter().all(String::is_empty),
            "{seen:?}"
        );
        let ssh = std::fs::read_to_string(&origin.ssh_record).unwrap_or_default();
        assert!(
            ssh.contains("-i\n") && ssh.contains("git@127.0.0.1\n"),
            "{ssh}"
        );
        assert_eq!(
            std::fs::read_to_string(format!("{}.mode", origin.ssh_record))
                .unwrap_or_default()
                .trim(),
            "600"
        );

        // The next branch stacks on the open pull request, whose head is
        // fetched the same way before the push.
        let next = push_branch(&s, &origin.proj, &origin.repo, "hive/worker/b");
        forget_sync_memo();
        let lines = crate::delivery::tend_project(&s, &origin.proj, TEST_NOW);
        assert_eq!(
            lines,
            [format!(
                "job pr proj job {}: #8 opened on #7 https://github.com/o/r/pull/8",
                next.str("id")
            )]
        );
        assert_eq!(
            bare_ref(&origin.served, "refs/heads/hive/worker/b"),
            ref_of(&origin.repo, "refs/heads/hive/worker/b")
        );
        assert_eq!(count_events(&s, "land.pr-held"), 0);

        // The one below merges: the default branch follows the upstream
        // and the merged branch goes from it, the same way.
        merge_on_forge(&gh, &origin.served, 7);
        forget_sync_memo();
        let lines = crate::delivery::tend_project(&s, &origin.proj, TEST_NOW);
        assert!(
            lines
                .iter()
                .any(|l| l == "pr proj: hive/worker/a deleted on origin")
                && !lines.iter().any(|l| l.contains("failed")),
            "{lines:?}"
        );
        assert!(
            !run(
                ".",
                &["git", "--git-dir", &origin.served, "branch", "--list"]
            )
            .contains("hive/worker/a"),
            "the merged branch is still on the upstream"
        );
        assert_eq!(
            ref_of(&origin.repo, "refs/heads/main"),
            bare_ref(&origin.served, "refs/heads/main"),
            "main follows the upstream"
        );
        assert_eq!(
            run(
                &origin.repo,
                &["git", "remote", "get-url", "--push", "origin"]
            )
            .trim(),
            origin.url,
            "the checkout's remote is not rewritten"
        );
    }

    /// Without a key of the daemon's own nothing is tried over ssh, and
    /// the hold says how to get the branch out.
    #[test]
    fn a_push_git_has_no_credential_for_is_held_with_the_way_to_fix_it() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let _gh = fake_gh();
        let s = new_store();
        let Some(origin) = https_origin(&s) else {
            return;
        };
        let job = push_branch(&s, &origin.proj, &origin.repo, "hive/worker/a");

        forget_sync_memo();
        let lines = crate::delivery::tend_project(&s, &origin.proj, TEST_NOW);
        let held = lines
            .iter()
            .find(|l| l.starts_with(&format!("job pr proj {}: held: ", job.str("id"))))
            .unwrap_or_else(|| panic!("{lines:?}"));
        assert!(
            held.contains("push of hive/worker/a to hive/worker/a on origin/main failed: ")
                && held.ends_with(
                    "; git has no credential that opens this https url: give origin an ssh push url (git remote set-url --push origin git@127.0.0.1:owner/repo.git)"
                ),
            "{held}"
        );
        assert!(
            !std::path::Path::new(&origin.ssh_record).exists(),
            "ssh was tried without the daemon's key"
        );
    }

    /// Only "git has no credential" sends a command over ssh. A credential
    /// the checkout's own config holds and the forge refuses is the
    /// owner's to repair: nothing is tried over ssh, and the hold says what
    /// to do.
    #[test]
    fn a_refused_https_credential_is_not_tried_over_ssh() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let _gh = fake_gh();
        let s = new_store();
        let Some(origin) = https_origin(&s) else {
            return;
        };
        set_setting(
            &s,
            "push.credential",
            &format!("file://{}", key_file(0o600)),
        );
        run(
            &origin.repo,
            &[
                "git",
                "config",
                "credential.helper",
                "!f() { test \"$1\" = get && printf 'username=someone\\npassword=wrong\\n'; }; f",
            ],
        );
        let job = push_branch(&s, &origin.proj, &origin.repo, "hive/worker/a");

        forget_sync_memo();
        let lines = crate::delivery::tend_project(&s, &origin.proj, TEST_NOW);
        let held = lines
            .iter()
            .find(|l| l.starts_with(&format!("job pr proj {}: held: ", job.str("id"))))
            .unwrap_or_else(|| panic!("{lines:?}"));
        assert!(
            held.contains("on origin/main failed: ")
                && held.contains("Authentication failed")
                && held.ends_with(
                    "; git has no credential that opens this https url: give origin an ssh push url (git remote set-url --push origin git@127.0.0.1:owner/repo.git)"
                ),
            "{held}"
        );
        assert!(
            origin
                .http
                .seen()
                .iter()
                .any(|auth| auth.starts_with("Basic ")),
            "the checkout's credential was not offered: {:?}",
            origin.http.seen()
        );
        assert!(
            !std::path::Path::new(&origin.ssh_record).exists(),
            "ssh was tried although https had a credential"
        );
    }

    /// The ssh form is GitHub's, so a command is made again only when it
    /// was GitHub that asked for the credential. A url the checkout's own
    /// config sends to another host (`url.*.insteadOf`) reads like GitHub's
    /// and is not: git's answer is then about that host, and the command
    /// is not made on GitHub.
    #[test]
    fn a_command_git_sent_to_another_host_is_not_made_again_on_github() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = new_store();
        let Some(origin) = https_origin(&s) else {
            return;
        };
        set_setting(
            &s,
            "push.credential",
            &format!("file://{}", key_file(0o600)),
        );
        let elsewhere = tempfile::tempdir().unwrap();
        let mirror =
            hive_testutil::githttp::Server::new(elsewhere.path(), "Basic nothing-has-this");
        run(
            &origin.repo,
            &[
                "git",
                "config",
                &format!("url.{}/.insteadOf", mirror.url()),
                &format!("{}/", origin.http.url()),
            ],
        );

        let (env, cleanup, why) = remote_credential_env(&s, origin.repo.clone(), &origin.url);
        assert!(why.is_empty(), "{why}");
        let listed = git_upstream(
            &origin.repo,
            Duration::from_secs(60),
            &env,
            &origin.url,
            &["ls-remote", &origin.url],
        );
        cleanup("", &[]);
        assert!(
            !listed.ok()
                && listed
                    .stderr
                    .contains(&format!("could not read Username for '{}'", mirror.url())),
            "{listed:?}"
        );
        assert!(
            !mirror.seen().is_empty() && origin.http.seen().is_empty(),
            "git did not go where the config sends the url"
        );
        assert!(
            !std::path::Path::new(&origin.ssh_record).exists(),
            "the command was made on GitHub over ssh"
        );
    }

    /// A host the checkout's config sends the url to (`url.*.insteadOf`)
    /// can send git on to GitHub, which asks for a credential. Git's
    /// answer is then its own, whole, and about GitHub - and the command
    /// went somewhere else first. A url git puts another in the place of
    /// is not made again.
    #[test]
    fn a_url_the_config_rewrites_is_not_made_again_though_github_asked() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = new_store();
        let Some(origin) = https_origin(&s) else {
            return;
        };
        set_setting(
            &s,
            "push.credential",
            &format!("file://{}", key_file(0o600)),
        );
        let github = origin.http.url();
        let detour = hive_testutil::httptest::Server::new(move |req| {
            hive_testutil::httptest::Response::new(302)
                .header("Location", &format!("{github}{}", req.uri))
        });
        run(
            &origin.repo,
            &[
                "git",
                "config",
                &format!("url.{}/.insteadOf", detour.url()),
                &format!("{}/", origin.http.url()),
            ],
        );

        let (env, cleanup, why) = remote_credential_env(&s, origin.repo.clone(), &origin.url);
        assert!(why.is_empty(), "{why}");
        let (listed, https) = git_upstream_said(
            &origin.repo,
            Duration::from_secs(60),
            &env,
            &origin.url,
            &["ls-remote", &origin.url],
        );
        cleanup("", &[]);
        assert!(
            !listed.ok()
                && https.is_none()
                && listed.stderr.trim_end()
                    == format!(
                        "fatal: could not read Username for '{}': terminal prompts disabled",
                        origin.http.url()
                    ),
            "{listed:?} after {https:?}"
        );
        assert!(
            !origin.http.seen().is_empty(),
            "git was not sent on to GitHub"
        );
        assert!(
            !std::path::Path::new(&origin.ssh_record).exists(),
            "the command was made over ssh"
        );
    }

    /// The host git reached writes part of what git prints: a server's
    /// error comes out as `fatal: remote error:` and the server's own
    /// words, line breaks included. So a host can put the line that asks
    /// for a user name into git's answer, after a line of its own. Such an
    /// answer is not git's alone, and the command is not made again over
    /// ssh.
    #[test]
    fn an_answer_the_host_wrote_part_of_does_not_send_a_command_over_ssh() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = new_store();
        let Some(origin) = https_origin(&s) else {
            return;
        };
        set_setting(
            &s,
            "push.credential",
            &format!("file://{}", key_file(0o600)),
        );
        // A forge that answers git's first request with an error packet
        // of two lines, the second one worded as git words its own.
        let forged = hive_testutil::httptest::Server::new(|req| {
            let packet = |line: &str| format!("{:04x}{line}", line.len() + 4);
            let body = format!(
                "{}0000{}",
                packet("# service=git-upload-pack\n"),
                packet(&format!(
                    "ERR no such repository\nfatal: could not read Username for 'http://{}': terminal prompts disabled\n",
                    req.header("host").unwrap_or_default()
                ))
            );
            hive_testutil::httptest::Response::new(200)
                .header(
                    "Content-Type",
                    "application/x-git-upload-pack-advertisement",
                )
                .body(body.into_bytes())
        });
        let _forge = crate::gitapp::tests::point_at(&forged.url(), &forged.url());
        let url = format!("{}/owner/repo.git", forged.url());

        let (env, cleanup, why) = remote_credential_env(&s, origin.repo.clone(), &url);
        assert!(why.is_empty(), "{why}");
        let (listed, https) = git_upstream_said(
            &origin.repo,
            Duration::from_secs(60),
            &env,
            &url,
            &["ls-remote", &url],
        );
        cleanup("", &[]);
        assert!(
            !listed.ok()
                && https.is_none()
                && listed
                    .stderr
                    .starts_with("fatal: remote error: no such repository\n")
                && listed.stderr.lines().any(|line| line
                    == format!(
                        "fatal: could not read Username for '{}': terminal prompts disabled",
                        forged.url()
                    )),
            "{listed:?} after {https:?}"
        );
        assert!(
            !std::path::Path::new(&origin.ssh_record).exists(),
            "the command was made over ssh on the host's word"
        );
    }

    /// When the retry over ssh fails too, the hold says what https said
    /// and what ssh said, since the owner knows the remote as an https
    /// one, and the way to fix it follows ssh's words.
    #[test]
    fn a_retry_over_ssh_that_fails_is_held_with_what_https_said_too() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let _gh = fake_gh();
        let s = new_store();
        let Some(origin) = https_origin(&s) else {
            return;
        };
        set_setting(
            &s,
            "push.credential",
            &format!("file://{}", key_file(0o600)),
        );
        let _refuse = hive_testutil::setenv(
            "HIVE_TEST_SSH_REFUSE",
            "git@127.0.0.1: Permission denied (publickey).",
        );
        let job = push_branch(&s, &origin.proj, &origin.repo, "hive/worker/a");

        forget_sync_memo();
        let lines = crate::delivery::tend_project(&s, &origin.proj, TEST_NOW);
        let held = lines
            .iter()
            .find(|l| l.starts_with(&format!("job pr proj {}: held: ", job.str("id"))))
            .unwrap_or_else(|| panic!("{lines:?}"));
        assert!(
            held.contains(
                "push of hive/worker/a to hive/worker/a on origin/main failed over https (fatal: could not read Username for 'http://127.0.0.1:"
            ) && held.contains(
                "': terminal prompts disabled), then over ssh with the daemon's key: git@127.0.0.1: Permission denied (publickey)."
            ) && held.ends_with(
                "; the forge refused the key push.credential names: add its public key to the repository as a deploy key with write access, or name a key that may push with hive cred set"
            ),
            "{held}"
        );
        assert!(
            std::path::Path::new(&origin.ssh_record).exists(),
            "ssh was not tried"
        );
    }

    #[test]
    fn only_an_http_url_on_github_has_an_ssh_form() {
        let _serial = crate::test_helpers::serial();
        let wrong: Vec<_> = [
            (
                "https://github.com/owner/repo.git",
                Some("git@github.com:owner/repo.git"),
            ),
            (
                "https://github.com/owner/repo",
                Some("git@github.com:owner/repo.git"),
            ),
            (
                "https://github.com/owner/.github",
                Some("git@github.com:owner/.github.git"),
            ),
            // Git ends the host at `?` and `#`: these two go to example.org.
            ("https://example.org?@github.com/owner/repo", None),
            ("https://example.org#@github.com/owner/repo", None),
            // Shapes nobody vouches for stay on the url they have.
            ("https://me@github.com/owner/repo.git", None),
            ("https://github.com:443/owner/repo.git", None),
            ("https://github.com/owner/repo.git/", None),
            ("https://github.com/owner/repo/deeper", None),
            ("https://github.com/owner/repo?ref=main", None),
            ("https://github.com/owner/repo#main", None),
            ("https://github.com/owner/..", None),
            ("https://github.com/-owner/repo", None),
            ("http://github.com/owner/repo.git", None),
            ("https://gitlab.com/owner/repo.git", None),
            ("git@github.com:owner/repo.git", None),
            ("ssh://git@github.com/owner/repo.git", None),
            ("/tmp/origin.git", None),
        ]
        .into_iter()
        .filter(|(url, want)| ssh_url(url).as_deref() != *want)
        .map(|(url, _)| (url, ssh_url(url)))
        .collect();
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    /// A command is made again over ssh on git's own word alone: its
    /// last line says it could not read a user name for GitHub's address,
    /// and before it stands at most its notice that GitHub sent it to
    /// another address of its own. Any other line before or after,
    /// another host, a port, the address with a path, or words that name
    /// no address do not count.
    #[test]
    fn only_githubs_own_request_for_a_credential_counts() {
        let _serial = crate::test_helpers::serial();
        let asked =
            "fatal: could not read Username for 'https://github.com': terminal prompts disabled";
        let wrong: Vec<_> = [
            (asked.to_string(), true),
            (format!("{asked}\n"), true),
            (
                format!("warning: redirecting to https://github.com/owner/repo.git/\n{asked}\n"),
                true,
            ),
            // A line of the host's before git's, as an error packet puts
            // it, and one after.
            (format!("fatal: remote error: no\n{asked}"), false),
            (
                format!("{asked}\nfatal: the remote end hung up unexpectedly"),
                false,
            ),
            (
                format!("remote: {asked}\nfatal: the remote end hung up unexpectedly"),
                false,
            ),
            (
                format!("warning: redirecting to https://example.org/owner/repo.git/\n{asked}"),
                false,
            ),
            (
                "warning: redirecting to https://github.com/owner/repo.git/".to_string(),
                false,
            ),
            (
                format!("{asked}\nwarning: redirecting to https://github.com/owner/repo.git/"),
                false,
            ),
            (format!("{asked}\n{asked}"), false),
            (String::new(), false),
            (format!("{asked} "), false),
            (asked.replace("github.com'", "github.com/owner/repo.git'"), false),
            (asked.replace("github.com'", "github.com.example.org'"), false),
            (asked.replace("github.com'", "github.com:8443'"), false),
            (asked.replace("https://", "http://"), false),
            (asked.replace("github.com", "example.org"), false),
            (
                "fatal: could not read Password for 'https://someone@github.com': terminal prompts disabled".to_string(),
                false,
            ),
            ("fatal: unable to get password from user".to_string(), false),
            (
                "fatal: Authentication failed for 'https://github.com/owner/repo.git/'".to_string(),
                false,
            ),
        ]
        .into_iter()
        .filter(|(said, want)| github_asked(said) != *want)
        .collect();
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    /// The sentence a hold's note ends with follows what refused the push:
    /// https for want of a credential, ssh for the key or the host, and
    /// nothing for a refusal that is not about who is asking.
    #[test]
    fn a_refused_push_is_told_apart_by_what_refused_it() {
        let _serial = crate::test_helpers::serial();
        let up = |url: &str| Upstream {
            remote: "origin".into(),
            remote_branch: "main".into(),
            upstream_ref: "refs/remotes/origin/main".into(),
            url: url.into(),
            repo: String::new(),
        };
        let key = [format!(
            "GIT_SSH_COMMAND={}",
            hive_cred::ssh_command("/keys/push")
        )];
        let own = ["GIT_SSH_COMMAND=ssh -oBatchMode=yes".to_string()];
        let app = [
            "GIT_CONFIG_COUNT=2".to_string(),
            "GIT_CONFIG_KEY_0=credential.helper".to_string(),
            "GIT_CONFIG_VALUE_0=".to_string(),
            "GIT_CONFIG_KEY_1=credential.helper".to_string(),
            "GIT_CONFIG_VALUE_1=!'/home/tmp/gitcred/helper'".to_string(),
        ];
        let forbidden = "remote: Permission to owner/repo.git denied to someone.\nfatal: unable to access 'https://github.com/owner/repo.git/': The requested URL returned error: 403";
        let denied = "git@github.com: Permission denied (publickey).\nfatal: Could not read from remote repository.";
        for (url, env, why, want) in [
            (
                "https://git.example/owner/repo.git",
                &key[..],
                "fatal: unable to get password from user",
                "; git has no credential that opens this https url: give origin an ssh push url (git remote set-url --push origin SSH-URL)",
            ),
            (
                "https://github.com/owner/repo.git",
                &key[..],
                denied,
                "; the forge refused the key push.credential names: add its public key to the repository as a deploy key with write access, or name a key that may push with hive cred set",
            ),
            (
                "https://github.com/owner/repo.git",
                &key[..],
                "ERROR: Permission to owner/repo.git denied to deploy key\nfatal: Could not read from remote repository.",
                "; the forge refused the key push.credential names: add its public key to the repository as a deploy key with write access, or name a key that may push with hive cred set",
            ),
            (
                "git@github.com:owner/repo.git",
                &own[..],
                denied,
                "; the forge refused this machine's ssh keys: give the daemon a key of its own with hive cred set",
            ),
            (
                "https://github.com/owner/repo.git",
                &own[..],
                forbidden,
                "; git has no credential that opens this https url: give origin an ssh push url (git remote set-url --push origin git@github.com:owner/repo.git)",
            ),
            (
                "https://github.com/owner/repo.git",
                &app[..],
                forbidden,
                "; GitHub refused the App's token: install the GitHub App on this repository with read and write access to its contents",
            ),
            (
                "git@github.com:owner/repo.git",
                &key[..],
                "Host key verification failed.\nfatal: Could not read from remote repository.",
                "; ssh does not know the forge's host key: connect to the forge over ssh once by hand, as the user the daemon runs as, and accept it",
            ),
            (
                "git@github.com:owner/repo.git",
                &key[..],
                " ! [rejected] hive/a -> hive/a (stale info)",
                "",
            ),
        ] {
            assert_eq!(auth_fix(&up(url), env, why), want, "{url}: {why}");
        }
    }

    #[test]
    fn a_project_without_a_remote_has_nothing_to_follow() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        let (proj, repo) = new_bare_repo(&s);
        assert!(upstream_of(&repo, "main").is_none());
        forget_sync_memo();
        assert_eq!(
            sync_default_from_upstream(&s, &proj, "daemon", TEST_NOW, true),
            None
        );
    }
}
