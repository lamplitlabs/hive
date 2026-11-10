//! Git and forge plumbing the delivery pass (`delivery.rs`) and the local
//! landings that remain (`pulse land --local`, projects without an
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
    crate::util::git_timeout_env(
        root,
        Duration::from_secs(120),
        env,
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
    s: &pulse_store::Store,
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
                &format!("pulse: fast-forward of {def} to {name} by ref"),
                &format!("refs/heads/{def}"),
                &to,
                &from,
            ],
        );
        if !moved.ok() {
            let why = pulse_cred::redact(&crate::run::squeeze(moved.stderr.trim(), 160), &[]);
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
        let why = pulse_cred::redact(&why, &[]);
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
    s: &pulse_store::Store,
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
            pulse_cred::redact(&crate::run::squeeze(&fetched.stderr, 160), &[])
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
            &format!("pulse auto-stash before fast-forward of {def} to {name}"),
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
    s: &pulse_store::Store,
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
    let parsed = match pulse_cred::parse(&ref_) {
        Ok(v) => v,
        Err(e) => return (Vec::new(), Box::new(|_, _| {}), e.to_string()),
    };
    let resolver = pulse_cred::new(&s.home);
    let secret = match resolver.resolve(parsed) {
        Ok(v) => v,
        Err(e) => {
            let why = pulse_cred::redact(&e.to_string(), &[]).replace(
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
                    pulse_cred::redact(&e.to_string(), &[])
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
    s: &pulse_store::Store,
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

fn app_fallback(s: &pulse_store::Store, repo: &str, why: &str) {
    let _ = s.event(
        "daemon",
        "gitapp.fallback",
        repo,
        serde_json::json!({
            "repo": repo,
            "why": pulse_cred::redact(&crate::executor::redact_tokens(why), &[]),
        }),
    );
}

/// The push credential leased to a hand for one push (ADR-070 step 1b):
/// the key `push.credential` names, resolved here on the brain, or empty
/// when the setting is unset (the hand pushes with what git there already
/// uses). Err says why the reference could not be read, never the key.
pub(crate) fn push_credential_key(s: &pulse_store::Store) -> Result<Vec<u8>, String> {
    let ref_ = s.setting("push.credential", "").trim().to_string();
    if ref_.is_empty() {
        return Ok(Vec::new());
    }
    let parsed = pulse_cred::parse(&ref_).map_err(|e| e.to_string())?;
    pulse_cred::new(&s.home)
        .resolve(parsed)
        .map(|secret| secret.key)
        .map_err(|e| pulse_cred::redact(&e.to_string(), &[]))
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
    fn behind_fixture(s: &pulse_store::Store) -> (crate::util::Row, String, String) {
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

    fn count_events(s: &pulse_store::Store, action: &str) -> i32 {
        count(
            s,
            "SELECT COUNT(*) AS n FROM events WHERE action = ?",
            &[&action],
        )
    }

    #[test]
    fn a_clean_checkout_of_the_default_branch_follows_its_upstream_once_per_memo() {
        let _serial = crate::test_helpers::serial();
        let _env = pulse_testutil::env_lock();
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
        let _env = pulse_testutil::env_lock();
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
        let _env = pulse_testutil::env_lock();
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
