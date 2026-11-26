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
        // Only a fast-forward moves by ref; a diverged branch is
        // `diverged_default`'s.
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
/// event at most once an hour. A branch the upstream no longer descends
/// from is not fast-forwarded and never stashed for: `diverged_default`
/// realigns it when it holds no work of its own and tells the owner once
/// when it does. Returns the log line when something was done or could
/// not be.
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
        diverged_over(s, project, &def, &up, None, actor, now);
        return None;
    }
    match is_ancestor(&root, &format!("refs/heads/{def}"), &up.upstream_ref) {
        Some(false) => return diverged_default(s, project, &def, &up, actor, now, behind),
        Some(true) => diverged_over(s, project, &def, &up, None, actor, now),
        // Git could not say; the fast-forward below finds out.
        None => {}
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

/// The owner's question about a default branch that cannot follow its
/// upstream: its coalesce kind and its thread under `runner`.
const DIVERGED_ASK: &str = "default-branch";
/// The `meta` key under which a project remembers what was last recorded
/// for its diverged default branch (`tell_diverged_once`), so a pair is
/// recorded once however many passes see it and the owner is asked once
/// for a divergence.
const DIVERGED_META_PREFIX: &str = "land-diverged:";

/// Whether `a` is `b` or an ancestor of it, so a fast-forward takes `a` to
/// `b`; None when git cannot say (a timeout, a missing object), which is
/// never read as diverged.
fn is_ancestor(root: &str, a: &str, b: &str) -> Option<bool> {
    let r = crate::util::git_timeout(
        root,
        Duration::from_secs(20),
        &["merge-base", "--is-ancestor", a, b],
    );
    match (r.err.is_none(), r.code) {
        (true, 0) => Some(true),
        (true, 1) => Some(false),
        _ => None,
    }
}

/// A local default branch its upstream no longer descends from: an
/// upstream whose history was replaced left the same fast-forward failing
/// on every pass, and nothing told the owner. The branch is realigned to
/// the upstream only when it holds no work of its own, which takes both
/// proofs: its tree is the upstream tip's, so not one file differs, and
/// its tip is a value the tracking ref held before (`was_upstream`), so
/// every commit on it was once the upstream's - a branch rewritten in this
/// checkout and not pushed yet fails the second and is never moved back.
/// A branch a working tree holds is left alone, and never stashed for
/// this (`held_by_a_worktree`). Whatever is not realigned is the owner's to
/// settle and is said once (`tell_diverged_once`).
fn diverged_default(
    s: &hive_store::Store,
    project: &crate::util::Row,
    def: &str,
    up: &Upstream,
    actor: &str,
    now: &str,
    behind: i32,
) -> Option<String> {
    let root = project.str("path");
    let slug = project.str("slug");
    let name = up.name();
    let rev = |r: &str| {
        let out = crate::util::git_timeout(
            &root,
            Duration::from_secs(10),
            &["rev-parse", "--verify", "--quiet", r],
        );
        if out.ok() {
            out.stdout.trim().to_string()
        } else {
            String::new()
        }
    };
    let from = rev(&format!("refs/heads/{def}^{{commit}}"));
    let to = rev(&format!("{}^{{commit}}", up.upstream_ref));
    // Everything below is about these two commits; a branch that moved
    // since the pass looked is judged on the next one.
    if from.is_empty() || to.is_empty() || is_ancestor(&root, &from, &to) != Some(false) {
        return None;
    }
    let tips = Tips {
        from: &from,
        to: &to,
        behind,
    };
    let tree = rev(&format!("{from}^{{tree}}"));
    let same_files = !tree.is_empty() && tree == rev(&format!("{to}^{{tree}}"));
    let why = match (same_files, was_upstream(&root, &up.upstream_ref, &from)) {
        (false, false) => format!(
            "its files differ from {name}'s and the upstream never held its tip, so it holds work of its own"
        ),
        (false, true) => format!(
            "the upstream held its tip before the rewrite, but its files differ from {name}'s now, and Hive realigns only a branch that holds the same files"
        ),
        (true, false) => format!(
            "its files are the same as {name}'s, but the upstream never held its tip (the tracking ref's reflog has no such entry), so the history that was rewritten may be the local one"
        ),
        (true, true) => match held_by_a_worktree(&root, def) {
            Some(held) => format!("it holds no work of its own, but {held}"),
            None => match realign(&root, def, &name, &tips, now) {
                Ok(backup) => {
                    let ahead = tips.ahead(&root);
                    if let Ok(conn) = s.conn() {
                        crate::util::event_at(
                            &conn,
                            now,
                            actor,
                            "land.realigned",
                            &slug,
                            serde_json::json!({"project": slug, "branch": def, "upstream": name, "from": from, "to": to,
                                "backup": backup, "ahead": ahead, "behind": behind}),
                        );
                    }
                    diverged_over(s, project, def, up, Some(&backup), actor, now);
                    return Some(format!(
                        "landing realigned {slug}: {def} -> {name}, whose history was rewritten ({ahead} local {} gave way to {behind}, the files are the same); the old tip {} is kept at {backup}",
                        crate::util::plural(ahead, "commit"),
                        crate::recipe::short(&from, 12),
                    ));
                }
                Err(why) => format!(
                    "it holds no work of its own, but git refused the realignment ({why}); Hive tries again on every pass"
                ),
            },
        },
    };
    tell_diverged_once(s, project, def, up, actor, now, &tips, &why)
}

/// The two tips of a default branch that cannot follow its upstream, and
/// how many commits the upstream holds that the branch lacks.
struct Tips<'a> {
    from: &'a str,
    to: &'a str,
    behind: i32,
}

impl Tips<'_> {
    /// How many commits the branch holds that the upstream lacks.
    fn ahead(&self, root: &str) -> i32 {
        crate::util::git_timeout(
            root,
            Duration::from_secs(30),
            &[
                "rev-list",
                "--count",
                &format!("{}..{}", self.to, self.from),
            ],
        )
        .stdout
        .trim()
        .parse::<i32>()
        .unwrap_or(0)
    }
}

/// Git's own words for a call that failed, short enough for a line and
/// with secrets removed; a call that ran out of time says so.
fn git_said(failed: &crate::util::GitResult) -> String {
    hive_cred::redact(
        &crate::run::squeeze(&crate::land::git_failure_result(failed), 160),
        &[],
    )
}

/// Why the default branch must not be moved under a working tree, or None
/// when none stands in the way. One reason is a worktree that has the
/// branch checked out with uncommitted changes, the project's own checkout
/// or a linked one. The other is an operation in progress on the branch,
/// which a clean `git status` does not show: a rebase and a bisect detach
/// HEAD and name the branch in their own files, and a rebase ends with an
/// update of the branch from the tip it started at, which fails when the
/// branch moved meanwhile; a merge and a cherry-pick run in the worktree
/// that has the branch checked out (`under_way`). A worktree git cannot
/// list or read is a reason too, whichever call fails or runs out of time
/// and whichever of git's notes cannot be read: nothing is moved on a
/// guess. Only a worktree whose folder is gone is passed over.
fn held_by_a_worktree(root: &str, def: &str) -> Option<String> {
    let list = crate::util::git_timeout(
        root,
        Duration::from_secs(30),
        &["worktree", "list", "--porcelain"],
    );
    if !list.ok() {
        return Some(format!(
            "git could not list the worktrees that may have it checked out ({}); Hive tries again on every pass",
            git_said(&list)
        ));
    }
    for wt in hive_hand_core::worktree::parse_worktrees(&list.stdout) {
        let path = wt.path.as_str();
        if wt.bare || !std::path::Path::new(path).is_dir() {
            continue;
        }
        let unread = |why: &str| {
            format!(
                "git could not read the worktree at {path} ({why}); Hive tries again on every pass"
            )
        };
        let dir = crate::util::git_timeout(
            path,
            Duration::from_secs(10),
            &["rev-parse", "--absolute-git-dir"],
        );
        if !dir.ok() || dir.stdout.trim().is_empty() {
            return Some(unread(&git_said(&dir)));
        }
        let on_it = wt.branch == def;
        if on_it {
            let status = crate::util::git_timeout(
                path,
                Duration::from_secs(60),
                &["status", "--porcelain", "--untracked-files=no"],
            );
            if !status.ok() {
                return Some(unread(&git_said(&status)));
            }
            if !status.stdout.trim().is_empty() {
                return Some(format!(
                    "its checkout at {path} has uncommitted changes; Hive realigns it once they are stashed or that checkout is on another branch"
                ));
            }
        }
        match under_way(std::path::Path::new(dir.stdout.trim()), def, on_it) {
            Err(unreadable) => return Some(unread(&unreadable)),
            Ok(Some(what)) => {
                return Some(format!(
                    "a {what} is in progress on it in {path}; Hive realigns it once that is finished or aborted"
                ));
            }
            Ok(None) => {}
        }
    }
    None
}

/// What git's own notes in a worktree's git directory say is under way on
/// the default branch. A rebase and a bisect name the branch in a file; a
/// merge and a cherry-pick leave their mark in the worktree that has the
/// branch checked out (`on_it`). Err names a note that is there and cannot
/// be read: it may name the branch.
fn under_way(
    dir: &std::path::Path,
    def: &str,
    on_it: bool,
) -> Result<Option<&'static str>, String> {
    let names = |file: &str, branch: &str| match std::fs::read_to_string(dir.join(file)) {
        Ok(text) => Ok(text.trim() == branch),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("{file}: {e}")),
    };
    let branch_ref = format!("refs/heads/{def}");
    Ok(
        if names("rebase-merge/head-name", &branch_ref)?
            || names("rebase-apply/head-name", &branch_ref)?
        {
            Some("rebase")
        } else if names("BISECT_START", def)? {
            Some("bisect")
        } else if on_it && dir.join("MERGE_HEAD").exists() {
            Some("merge")
        } else if on_it && dir.join("CHERRY_PICK_HEAD").exists() {
            Some("cherry-pick")
        } else {
            None
        },
    )
}

/// Whether the tracking ref `up_ref` once held `sha`, by its reflog: as a
/// value a fetch or a push left there, or as the value the oldest entry
/// moved away from (a clone writes no entry for the tip it starts with, so
/// the first forced fetch is the only record of it).
fn was_upstream(root: &str, up_ref: &str, sha: &str) -> bool {
    let git = |args: &[&str]| crate::util::git_timeout(root, Duration::from_secs(30), args);
    let log = git(&["reflog", "show", "--format=%H", up_ref]);
    if !log.ok() {
        return false;
    }
    let held: Vec<&str> = log.stdout.lines().map(str::trim).collect();
    if held.contains(&sha) {
        return true;
    }
    let before = git(&[
        "rev-parse",
        "--verify",
        "--quiet",
        &format!("{up_ref}@{{{}}}", held.len()),
    ]);
    !held.is_empty() && before.ok() && before.stdout.trim() == sha
}

/// Moves the default branch to the upstream's tip by ref, with the old tip
/// kept under `refs/hive/backup/<branch>/<UTC stamp>` first (a backup of
/// that tip an earlier try left is used again, and none is ever
/// overwritten). The two tips hold the same tree, so neither the index nor
/// a file changes whether the branch is checked out or not. Returns the
/// backup ref, or git's words when the old tip could not be kept or the
/// branch moved meanwhile.
fn realign(
    root: &str,
    def: &str,
    name: &str,
    tips: &Tips<'_>,
    now: &str,
) -> Result<String, String> {
    let git = |args: &[&str]| crate::util::git_timeout(root, Duration::from_secs(30), args);
    let kept = format!("refs/hive/backup/{def}/");
    let mut backup = git(&[
        "for-each-ref",
        "--count=1",
        &format!("--points-at={}", tips.from),
        "--format=%(refname)",
        &kept,
    ])
    .stdout
    .trim()
    .to_string();
    if backup.is_empty() {
        backup = format!("{kept}{}", crate::util::stamp_of(now));
        let made = git(&[
            "update-ref",
            "-m",
            &format!("hive: {def} before it was realigned to {name}"),
            &backup,
            tips.from,
            "",
        ]);
        if !made.ok() {
            return Err(format!(
                "the old tip could not be kept at {backup}: {}",
                git_said(&made)
            ));
        }
    }
    let moved = git(&[
        "update-ref",
        "-m",
        &format!(
            "hive: {def} realigned to {name}, whose history was rewritten; the old tip is {backup}"
        ),
        &format!("refs/heads/{def}"),
        tips.to,
        tips.from,
    ]);
    if !moved.ok() {
        return Err(git_said(&moved));
    }
    Ok(backup)
}

/// Records that the default branch cannot follow its upstream, once for
/// each pair of tips: a failed `land.fast-forward` with the two tips and
/// the reason. The first pair of a divergence also asks the owner what to
/// keep (`ask_about_diverged`); a tip that moves after that is a new pair
/// to record, not a new question. An ask the store refused is made again
/// on every pass until it goes through. What was recorded is remembered in
/// `meta` until the branch can follow again (`diverged_over`): the pair,
/// whether the owner was asked, and the line in its parts, so a daemon
/// that restarts says nothing twice. When something else holds the same
/// pair on a later pass (an edit was stashed, a rebase began), the reason
/// alone is rewritten there, for a run by hand to read
/// (`diverged_standing`): no event, no question, no line. Returns the log
/// line the first time a pair is seen, and again when an ask that was owed
/// went through; None otherwise.
#[allow(clippy::too_many_arguments)]
fn tell_diverged_once(
    s: &hive_store::Store,
    project: &crate::util::Row,
    def: &str,
    up: &Upstream,
    actor: &str,
    now: &str,
    tips: &Tips<'_>,
    why: &str,
) -> Option<String> {
    let root = project.str("path");
    let slug = project.str("slug");
    let name = up.name();
    let key = format!("{DIVERGED_META_PREFIX}{slug}");
    let mut told = diverged_told(s, &slug);
    let same_pair = told["from"] == tips.from && told["to"] == tips.to;
    let asked_before = told["asked"] == true;
    if same_pair && asked_before {
        if told["why"] != why {
            told["why"] = why.into();
            let _ = s.set_meta(&key, &told.to_string());
        }
        return None;
    }
    let (ahead, behind) = (tips.ahead(&root), tips.behind);
    if !same_pair && let Ok(conn) = s.conn() {
        crate::util::event_at(
            &conn,
            now,
            actor,
            "land.fast-forward",
            &slug,
            serde_json::json!({"project": slug, "branch": def, "upstream": name, "behind": behind, "ok": false,
                "why": why, "from": crate::recipe::short(tips.from, 12), "to": crate::recipe::short(tips.to, 12),
                "ahead": ahead}),
        );
    }
    let (asked, how) = if asked_before {
        (
            true,
            "the owner was asked when it first diverged".to_string(),
        )
    } else {
        match ask_about_diverged(s, project, def, up, actor, now, tips, ahead, why) {
            Ok(Some(post)) => (true, format!("the owner is asked ({})", post.str("id"))),
            Ok(None) => (
                true,
                "the owner's question about it is still open".to_string(),
            ),
            Err(e) => (
                false,
                format!(
                    "the owner could not be asked ({}); Hive asks again on the next pass",
                    crate::run::squeeze(&e.to_string(), 160)
                ),
            ),
        }
    };
    let head = format!(
        "{slug}: {def} has diverged from {name} ({ahead} ahead, {behind} behind) and was not moved"
    );
    let _ = s.set_meta(
        &key,
        &serde_json::json!({"from": tips.from, "to": tips.to, "asked": asked, "head": head, "why": why, "how": how})
            .to_string(),
    );
    (!same_pair || asked).then(|| diverged_line(&head, why, &how))
}

/// The line a diverged default branch is said with: where it stands
/// (`head`), why it was not moved, and how the owner was asked.
fn diverged_line(head: &str, why: &str, how: &str) -> String {
    format!("{head}: {why}; {how}")
}

/// What `tell_diverged_once` last recorded for the project's default
/// branch: `from`, `to`, `asked`, and the line's `head`, `why` and `how`;
/// null when nothing is recorded.
fn diverged_told(s: &hive_store::Store, slug: &str) -> serde_json::Value {
    s.meta(&format!("{DIVERGED_META_PREFIX}{slug}"))
        .ok()
        .and_then(|(told, _)| serde_json::from_str(&told).ok())
        .unwrap_or_default()
}

/// Where a diverged default branch stands, for a pass run by hand (`hive
/// land`): the daemon says the line once for a pair of tips, a person who
/// asks is told every time. It is the recorded line with the reason the
/// last pass found, read and not written again, and None once the branch
/// can follow its upstream.
pub(crate) fn diverged_standing(
    s: &hive_store::Store,
    project: &crate::util::Row,
) -> Option<String> {
    let told = diverged_told(s, &project.str("slug"));
    let said = |part: &str| told[part].as_str().unwrap_or_default();
    if said("head").is_empty() {
        return None;
    }
    let root = project.str("path");
    let def = crate::baseline::default_branch(project.clone());
    let up = upstream_of(&root, &def)?;
    let local = format!("refs/heads/{def}");
    (is_ancestor(&root, &local, &up.upstream_ref) == Some(false)
        && is_ancestor(&root, &up.upstream_ref, &local) == Some(false))
    .then(|| diverged_line(said("head"), said("why"), said("how")))
}

/// Asks the owner what to keep when the default branch cannot follow its
/// upstream: one question marked for the owner on `runner/default-branch`
/// that says what happened, why Hive did not move the branch and the two
/// ways out. It goes through the daemon's coalescing ask, so one that is
/// still open is never asked a second time (None). Err when the board
/// would not take it.
#[allow(clippy::too_many_arguments)]
fn ask_about_diverged(
    s: &hive_store::Store,
    project: &crate::util::Row,
    def: &str,
    up: &Upstream,
    actor: &str,
    now: &str,
    tips: &Tips<'_>,
    ahead: i32,
    why: &str,
) -> anyhow::Result<Option<crate::util::Row>> {
    let root = project.str("path");
    let slug = project.str("slug");
    let name = up.name();
    let behind = tips.behind;
    let unrelated = crate::util::git_timeout(
        &root,
        Duration::from_secs(30),
        &["merge-base", tips.from, tips.to],
    )
    .stdout
    .trim()
    .is_empty();
    let dir = crate::util::shell_quote(&root);
    let body = format!(
        "The local default branch `{def}` of {slug} (the checkout at {root}) cannot follow its upstream `{name}` by a fast-forward. `{def}` is at {} and holds {ahead} {} the upstream lacks; `{name}` is at {} and holds {behind} {} `{def}` lacks{}. Hive only ever fast-forwards `{def}`, so it stays where it is: new work is still cut from it, and a pull request cut from it may be refused by the forge or carry the difference.\n\n\
Why Hive did not move it: {why}.\n\n\
Two ways out:\n\n\
1. Keep the upstream's history. Save the local commits under a name of their own (`git -C {dir} branch {def}-local {def}`), then move `{def}`: `git -C {dir} reset --keep {name}` with `{def}` checked out, or `git -C {dir} branch -f {def} {name}` from another branch.\n\
2. Keep the local commits. Rebase them onto `{name}` and push them or open a pull request with them; or, when the local history is the one to keep, put it in the upstream's place: `git -C {dir} push --force-with-lease {} {def}:{}`.\n\n\
After either, `{def}` follows `{name}` again on the next delivery pass and Hive answers this question itself. It is asked once: the daemon records a tip that moves meanwhile and does not ask again.\n",
        crate::recipe::short(tips.from, 12),
        crate::util::plural(ahead, "commit"),
        crate::recipe::short(tips.to, 12),
        crate::util::plural(behind, "commit"),
        if unrelated {
            "; the two share no commit, as after a history was replaced"
        } else {
            ""
        },
        up.remote,
        up.remote_branch,
    );
    crate::hygiene::post_daemon_owner_question(
        s,
        crate::hygiene::DaemonQuestionAsk {
            project: slug.clone(),
            ask_kind: DIVERGED_ASK.into(),
            area: "runner".into(),
            thread: DIVERGED_ASK.into(),
            summary: crate::util::trunc(
                &format!(
                    "{def} of {slug} has diverged from {name} and Hive did not move it: keep the upstream's history or the local commits?"
                ),
                230,
            ),
            body,
            actor: actor.to_string(),
            now: now.to_string(),
        },
    )
}

/// Ends what `tell_diverged_once` began once the branch can follow its
/// upstream again - it was realigned (`backup` names where its old tip is
/// kept), or the owner moved it or pushed it: the owner's open question is
/// answered, and only then is what was recorded forgotten, so an answer
/// the store refused is given on the next pass. For a project that was
/// never told it is one `meta` read; for one that was, the branch must
/// hold the upstream's tip or be an ancestor of it, by git's own word.
fn diverged_over(
    s: &hive_store::Store,
    project: &crate::util::Row,
    def: &str,
    up: &Upstream,
    backup: Option<&str>,
    actor: &str,
    now: &str,
) {
    let slug = project.str("slug");
    let key = format!("{DIVERGED_META_PREFIX}{slug}");
    if !s.meta(&key).is_ok_and(|(_, found)| found) {
        return;
    }
    let (root, local) = (project.str("path"), format!("refs/heads/{def}"));
    if is_ancestor(&root, &local, &up.upstream_ref) != Some(true)
        && is_ancestor(&root, &up.upstream_ref, &local) != Some(true)
    {
        return;
    }
    let name = up.name();
    let outcome = match backup {
        Some(backup) => format!(
            "Hive realigned {def} to {name}: it held no work of its own, and its old tip is kept at {backup}"
        ),
        None => format!("{def} can follow {name} again; nothing is left to decide"),
    };
    crate::idle_hold::answer(s, project, DIVERGED_ASK, &outcome, &outcome, now, actor);
    if crate::hygiene::open_daemon_questions_of_kind(s, &slug, DIVERGED_ASK).is_empty()
        && let Ok(conn) = s.conn()
    {
        let _ = conn.execute("DELETE FROM meta WHERE key = ?", [&key]);
    }
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
    hive_gitenv::forge::ssh_form(&crate::gitapp::endpoint().web, url)
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
    hive_gitenv::forge::credential_asked(&crate::gitapp::endpoint().web, why)
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

    /// A project whose local main and its origin hold the same three
    /// commits, the last two pushed from this checkout.
    fn followed_fixture(s: &hive_store::Store) -> (crate::util::Row, String, String) {
        let (proj, repo, bare) = push_fixture(s);
        for name in ["first.txt", "second.txt"] {
            std::fs::write(Path::new(&repo).join(name), format!("{name}\n")).unwrap();
            run(&repo, &["git", "add", "-A"]);
            run(&repo, &["git", "commit", "-q", "-m", name]);
        }
        run(&repo, &["git", "push", "-q", "origin", "main"]);
        (proj, repo, bare)
    }

    fn bare_git(bare: &str, args: &[&str]) -> String {
        let mut argv = vec!["git", "--git-dir", bare];
        argv.extend_from_slice(args);
        run(".", &argv).trim().to_string()
    }

    /// The upstream's owner replaces main with one commit that holds the
    /// same files, as a force-push from another clone does; returns it.
    fn squash_upstream(bare: &str) -> String {
        let tree = bare_git(bare, &["rev-parse", "refs/heads/main^{tree}"]);
        let squashed = bare_git(
            bare,
            &["commit-tree", &tree, "-m", "the history as one commit"],
        );
        bare_git(bare, &["update-ref", "refs/heads/main", &squashed]);
        squashed
    }

    /// The owner's questions about a default branch that cannot follow its
    /// upstream, asked so far and still open.
    fn diverged_questions(s: &hive_store::Store) -> (i32, usize) {
        (
            count(
                s,
                "SELECT COUNT(*) AS n FROM posts WHERE project = 'proj' AND kind = 'question' AND area = 'runner' AND thread = 'default-branch' AND json_extract(extra, '$.for_owner') = 1",
                &[],
            ),
            crate::hygiene::open_daemon_questions_of_kind(s, "proj", "default-branch").len(),
        )
    }

    fn backup_refs(repo: &str) -> String {
        run(
            repo,
            &["git", "for-each-ref", "--format=%(refname)", "refs/hive/"],
        )
    }

    fn head_branch(repo: &str) -> String {
        run(repo, &["git", "rev-parse", "--abbrev-ref", "HEAD"])
            .trim()
            .to_string()
    }

    #[test]
    fn a_default_branch_with_no_work_of_its_own_is_realigned_to_a_rewritten_upstream() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = new_store();
        let (proj, repo, bare) = followed_fixture(&s);
        let old = ref_of(&repo, "refs/heads/main");
        let squashed = squash_upstream(&bare);
        std::fs::write(Path::new(&repo).join("untracked.txt"), "not git's\n").unwrap();
        let written = |name: &str| {
            std::fs::metadata(Path::new(&repo).join(name))
                .and_then(|m| m.modified())
                .unwrap()
        };
        let before = (written("README.md"), written("second.txt"));
        forget_sync_memo();
        let line = sync_default_from_upstream(&s, &proj, "daemon", TEST_NOW, false);
        assert!(
            line.as_deref().is_some_and(|l| l
                .contains("landing realigned proj: main -> origin/main")
                && l.contains("refs/hive/backup/main/20260906T100000Z")),
            "{line:?}"
        );
        assert_eq!(ref_of(&repo, "refs/heads/main"), squashed);
        assert_eq!(head_branch(&repo), "main");
        let d = last_detail(&s, "land.realigned");
        assert_eq!(d["from"], old.as_str(), "{d}");
        assert_eq!(d["to"], squashed.as_str(), "{d}");
        assert_eq!(d["backup"], "refs/hive/backup/main/20260906T100000Z", "{d}");
        assert_eq!(
            (d["ahead"].as_i64(), d["behind"].as_i64()),
            (Some(3), Some(1)),
            "{d}"
        );
        assert_eq!(
            ref_of(&repo, "refs/hive/backup/main/20260906T100000Z"),
            old,
            "the old tip is not kept"
        );
        assert_eq!(
            run(&repo, &["git", "status", "--porcelain"]),
            "?? untracked.txt\n",
            "the checkout is not as it was"
        );
        assert_eq!(
            (written("README.md"), written("second.txt")),
            before,
            "a file was written"
        );
        assert_eq!(count_events(&s, "land.realigned"), 1);
        assert_eq!(count_events(&s, "land.fast-forward"), 0);
        assert_eq!(diverged_questions(&s), (0, 0));
        assert_eq!(
            sync_default_from_upstream(&s, &proj, "daemon", TEST_NOW, true),
            None,
            "nothing is left to follow"
        );
    }

    #[test]
    fn a_default_branch_that_is_not_checked_out_is_realigned_by_ref() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = new_store();
        let (proj, repo, bare) = followed_fixture(&s);
        let old = ref_of(&repo, "refs/heads/main");
        let squashed = squash_upstream(&bare);
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
                .is_some_and(|l| l.contains("landing realigned proj: main -> origin/main")),
            "{line:?}"
        );
        assert_eq!(ref_of(&repo, "refs/heads/main"), squashed);
        let d = last_detail(&s, "land.realigned");
        assert_eq!(
            ref_of(&repo, d["backup"].as_str().unwrap_or("no-backup")),
            old,
            "{d}"
        );
        assert_eq!(head_branch(&repo), "work/elsewhere");
        assert_eq!(ref_of(&repo, "HEAD"), old);
        assert_eq!(
            std::fs::read_to_string(Path::new(&repo).join("README.md")).unwrap(),
            "# dirty on another branch\n"
        );
    }

    #[test]
    fn a_default_branch_with_work_of_its_own_is_not_moved_and_the_owner_is_asked_once() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = new_store();
        let (proj, repo, bare) = followed_fixture(&s);
        std::fs::write(Path::new(&repo).join("mine.txt"), "never pushed\n").unwrap();
        run(&repo, &["git", "add", "-A"]);
        run(&repo, &["git", "commit", "-q", "-m", "local work"]);
        let own = ref_of(&repo, "refs/heads/main");
        let squashed = squash_upstream(&bare);
        forget_sync_memo();
        let line = sync_default_from_upstream(&s, &proj, "daemon", TEST_NOW, false);
        assert!(
            line.as_deref().is_some_and(|l| l
                .contains("proj: main has diverged from origin/main (4 ahead, 1 behind)")
                && l.contains("not moved")),
            "{line:?}"
        );
        assert_eq!(ref_of(&repo, "refs/heads/main"), own);
        let d = last_detail(&s, "land.fast-forward");
        assert_eq!(d["ok"], false, "{d}");
        assert_eq!(d["from"], &own[..12], "{d}");
        assert_eq!(d["to"], &squashed[..12], "{d}");
        assert_eq!(
            (d["ahead"].as_i64(), d["behind"].as_i64()),
            (Some(4), Some(1)),
            "{d}"
        );
        assert!(
            d["why"]
                .as_str()
                .is_some_and(|w| w.contains("files differ")),
            "{d}"
        );
        assert_eq!(count_events(&s, "land.fast-forward"), 1);
        assert_eq!(diverged_questions(&s), (1, 1));
        let asked = crate::hygiene::open_daemon_questions_of_kind(&s, "proj", "default-branch");
        let (summary, body) = (asked[0].str("summary"), asked[0].str("body"));
        assert!(
            summary.contains("main of proj has diverged from origin/main"),
            "{summary}"
        );
        assert!(
            body.contains("reset --keep origin/main")
                && body.contains("push --force-with-lease origin main:main"),
            "the two ways out are not both named: {body}"
        );

        // The same two commits on the passes that follow: nothing is
        // written and nothing is said again.
        for minutes in [2, 4] {
            let later = crate::util::plus_minutes(TEST_NOW, minutes);
            assert_eq!(
                sync_default_from_upstream(&s, &proj, "daemon", &later, true),
                None,
                "said again after {minutes} minutes"
            );
        }
        assert_eq!(count_events(&s, "land.fast-forward"), 1);
        assert_eq!(diverged_questions(&s), (1, 1));

        // The upstream moves on: the new pair is recorded once, and the
        // question that is open stands for it too.
        let tip = bare_git(&bare, &["rev-parse", "refs/heads/main"]);
        let tree = bare_git(&bare, &["rev-parse", "refs/heads/main^{tree}"]);
        let next = bare_git(&bare, &["commit-tree", &tree, "-p", &tip, "-m", "more"]);
        bare_git(&bare, &["update-ref", "refs/heads/main", &next]);
        for minutes in [6, 8] {
            let later = crate::util::plus_minutes(TEST_NOW, minutes);
            let line = sync_default_from_upstream(&s, &proj, "daemon", &later, true);
            assert_eq!(line.is_some(), minutes == 6, "{line:?}");
        }
        assert_eq!(count_events(&s, "land.fast-forward"), 2);
        assert_eq!(diverged_questions(&s), (1, 1));
        assert_eq!(ref_of(&repo, "refs/heads/main"), own);
        assert_eq!(count_events(&s, "land.realigned"), 0);
        assert_eq!(backup_refs(&repo), "");

        // The owner keeps the upstream's history: the branch follows
        // again and the daemon answers its own question.
        run(&repo, &["git", "reset", "-q", "--keep", "origin/main"]);
        let later = crate::util::plus_minutes(TEST_NOW, 10);
        assert_eq!(
            sync_default_from_upstream(&s, &proj, "daemon", &later, true),
            None
        );
        assert_eq!(diverged_questions(&s), (1, 0));
        let answer = must_one(
            &s,
            "SELECT * FROM posts WHERE project = 'proj' AND kind = 'answer' AND thread = 'default-branch'",
            &[],
        );
        assert_eq!(answer.str("resolution"), "answered", "{answer:?}");
        assert_eq!(answer.str("on_post"), asked[0].str("id"), "{answer:?}");
    }

    #[test]
    fn a_dirty_checkout_of_a_rewritten_default_branch_is_not_touched() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = new_store();
        let (proj, repo, bare) = followed_fixture(&s);
        let old = ref_of(&repo, "refs/heads/main");
        let squashed = squash_upstream(&bare);
        let readme = Path::new(&repo).join("README.md");
        std::fs::write(&readme, "# edited by hand\n").unwrap();
        let written = || {
            std::fs::metadata(&readme)
                .and_then(|m| m.modified())
                .unwrap()
        };
        let before = written();
        forget_sync_memo();
        // landing.auto_stash is on, the default: a fast-forward would
        // stash the edit; a realignment leaves the checkout alone.
        let line = sync_default_from_upstream(&s, &proj, "daemon", TEST_NOW, false);
        assert!(
            line.as_deref()
                .is_some_and(|l| l.contains("proj: main has diverged from origin/main")
                    && l.contains("uncommitted changes")),
            "{line:?}"
        );
        assert_eq!(ref_of(&repo, "refs/heads/main"), old);
        assert_eq!(
            std::fs::read_to_string(&readme).unwrap(),
            "# edited by hand\n"
        );
        assert_eq!(written(), before, "the edit was stashed and put back");
        assert_eq!(run(&repo, &["git", "stash", "list"]), "");
        assert_eq!(backup_refs(&repo), "");
        assert_eq!(count_events(&s, "land.realigned"), 0);
        assert_eq!(count_events(&s, "land.fast-forward"), 1);
        assert_eq!(diverged_questions(&s), (1, 1));
        let later = crate::util::plus_minutes(TEST_NOW, 2);
        assert_eq!(
            sync_default_from_upstream(&s, &proj, "daemon", &later, true),
            None,
            "said twice"
        );
        assert_eq!(count_events(&s, "land.fast-forward"), 1);

        // The owner stashes the edit: the branch is realigned on the next
        // pass and the question is answered.
        run(&repo, &["git", "stash", "-q"]);
        let later = crate::util::plus_minutes(TEST_NOW, 4);
        let line = sync_default_from_upstream(&s, &proj, "daemon", &later, true);
        assert!(
            line.as_deref()
                .is_some_and(|l| l.contains("landing realigned proj: main -> origin/main")),
            "{line:?}"
        );
        assert_eq!(ref_of(&repo, "refs/heads/main"), squashed);
        assert_eq!(diverged_questions(&s), (1, 0));
        run(&repo, &["git", "stash", "pop", "-q"]);
        assert_eq!(
            std::fs::read_to_string(&readme).unwrap(),
            "# edited by hand\n"
        );
    }

    #[test]
    fn a_default_branch_rewritten_in_this_checkout_is_not_realigned() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = new_store();
        let (proj, repo, bare) = followed_fixture(&s);
        // The owner squashes main here and has not pushed it yet: the same
        // files as the upstream's, under a tip the upstream never held.
        let tree = ref_of(&repo, "refs/heads/main^{tree}");
        let squashed = run(&repo, &["git", "commit-tree", &tree, "-m", "squashed here"])
            .trim()
            .to_string();
        run(&repo, &["git", "reset", "-q", "--hard", &squashed]);
        forget_sync_memo();
        let line = sync_default_from_upstream(&s, &proj, "daemon", TEST_NOW, false);
        assert!(
            line.as_deref().is_some_and(|l| l
                .contains("proj: main has diverged from origin/main (1 ahead, 3 behind)")
                && l.contains("not moved")
                && l.contains("the upstream never held its tip")),
            "{line:?}"
        );
        assert_eq!(ref_of(&repo, "refs/heads/main"), squashed);
        assert_eq!(backup_refs(&repo), "");
        assert_eq!(count_events(&s, "land.realigned"), 0);
        assert_eq!(diverged_questions(&s), (1, 1));

        // The push the owner meant to make ends it.
        run(&repo, &["git", "push", "-q", "--force", "origin", "main"]);
        assert_eq!(bare_ref(&bare, "refs/heads/main"), squashed);
        let later = crate::util::plus_minutes(TEST_NOW, 2);
        assert_eq!(
            sync_default_from_upstream(&s, &proj, "daemon", &later, true),
            None
        );
        assert_eq!(diverged_questions(&s), (1, 0));
    }

    #[test]
    fn a_rewritten_upstream_that_holds_other_files_is_left_to_the_owner() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = new_store();
        let (proj, repo, bare) = followed_fixture(&s);
        let old = ref_of(&repo, "refs/heads/main");
        // The upstream is replaced by one commit that holds other files
        // (here the ones before `second.txt`): nothing local was ever
        // unpushed, yet following would change the checkout's files.
        let tree = bare_git(&bare, &["rev-parse", "refs/heads/main~1^{tree}"]);
        let other = bare_git(&bare, &["commit-tree", &tree, "-m", "rewritten"]);
        bare_git(&bare, &["update-ref", "refs/heads/main", &other]);
        forget_sync_memo();
        let line = sync_default_from_upstream(&s, &proj, "daemon", TEST_NOW, false);
        assert!(
            line.as_deref().is_some_and(|l| l
                .contains("proj: main has diverged from origin/main (3 ahead, 1 behind)")
                && l.contains("the upstream held its tip before the rewrite")),
            "{line:?}"
        );
        assert_eq!(ref_of(&repo, "refs/heads/main"), old);
        assert!(Path::new(&repo).join("second.txt").exists());
        assert_eq!(backup_refs(&repo), "");
        assert_eq!(count_events(&s, "land.realigned"), 0);
        assert_eq!(count_events(&s, "land.fast-forward"), 1);
        assert_eq!(diverged_questions(&s), (1, 1));

        // The owner closes the question without settling it and the
        // upstream moves on: the new pair is recorded, and the owner is
        // not asked a second time.
        let asked = crate::hygiene::open_daemon_questions_of_kind(&s, "proj", "default-branch");
        let later = crate::util::plus_minutes(TEST_NOW, 1);
        crate::board::create_post(
            &s,
            later.clone(),
            crate::board::PostOpts {
                kind: "answer".into(),
                project: "proj".into(),
                area: "runner".into(),
                thread: "default-branch".into(),
                summary: "I will look at it next week".into(),
                body: "Not now.\n".into(),
                on: asked[0].str("id"),
                agent: crate::board::actor_agent(&s, crate::util::HUMAN_AGENT.to_string()),
                date: later,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(diverged_questions(&s), (1, 0));
        let next = bare_git(&bare, &["commit-tree", &tree, "-p", &other, "-m", "more"]);
        bare_git(&bare, &["update-ref", "refs/heads/main", &next]);
        let later = crate::util::plus_minutes(TEST_NOW, 2);
        let line = sync_default_from_upstream(&s, &proj, "daemon", &later, true);
        assert!(
            line.as_deref()
                .is_some_and(|l| l.contains("(3 ahead, 2 behind)")
                    && l.contains("the owner was asked when it first diverged")),
            "{line:?}"
        );
        assert_eq!(count_events(&s, "land.fast-forward"), 2);
        assert_eq!(diverged_questions(&s), (1, 0));
        assert_eq!(ref_of(&repo, "refs/heads/main"), old);
    }

    #[test]
    fn a_realignment_git_refuses_is_said_once_and_made_on_a_later_pass() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = new_store();
        let (proj, repo, bare) = followed_fixture(&s);
        let old = ref_of(&repo, "refs/heads/main");
        let squashed = squash_upstream(&bare);
        // A hook of the owner's that refuses every update of main, as a
        // lock left behind does.
        let hook = Path::new(&repo).join(".git/hooks/reference-transaction");
        std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
        std::fs::write(
            &hook,
            "#!/bin/sh\n[ \"$1\" = prepared ] || exit 0\nwhile read -r old new name; do\n    [ \"$name\" = refs/heads/main ] && exit 1\ndone\nexit 0\n",
        )
        .unwrap();
        run(&repo, &["chmod", "+x", ".git/hooks/reference-transaction"]);
        forget_sync_memo();
        let line = sync_default_from_upstream(&s, &proj, "daemon", TEST_NOW, false);
        assert!(
            line.as_deref()
                .is_some_and(|l| l.contains("proj: main has diverged from origin/main")
                    && l.contains("git refused the realignment")),
            "{line:?}"
        );
        assert_eq!(ref_of(&repo, "refs/heads/main"), old);
        let kept = "refs/hive/backup/main/20260906T100000Z";
        assert_eq!(backup_refs(&repo).trim(), kept);
        assert_eq!(count_events(&s, "land.realigned"), 0);
        assert_eq!(count_events(&s, "land.fast-forward"), 1);
        assert_eq!(diverged_questions(&s), (1, 1));
        let later = crate::util::plus_minutes(TEST_NOW, 2);
        assert_eq!(
            sync_default_from_upstream(&s, &proj, "daemon", &later, true),
            None,
            "said twice"
        );
        assert_eq!(
            backup_refs(&repo).trim(),
            kept,
            "the same tip kept a second time"
        );

        // The hook is gone: the next pass moves the branch, with the
        // backup the first try made.
        std::fs::remove_file(&hook).unwrap();
        let later = crate::util::plus_minutes(TEST_NOW, 4);
        let line = sync_default_from_upstream(&s, &proj, "daemon", &later, true);
        assert!(
            line.as_deref().is_some_and(|l| l
                .contains("landing realigned proj: main -> origin/main")
                && l.contains(kept)),
            "{line:?}"
        );
        assert_eq!(ref_of(&repo, "refs/heads/main"), squashed);
        assert_eq!(backup_refs(&repo).trim(), kept);
        assert_eq!(ref_of(&repo, kept), old);
        assert_eq!(count_events(&s, "land.fast-forward"), 1);
        assert_eq!(diverged_questions(&s), (1, 0));
    }

    #[test]
    fn a_checkout_whose_tracking_ref_never_moved_before_the_rewrite_is_realigned() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = new_store();
        let (proj, repo, bare) = followed_fixture(&s);
        // As after a clone: the tracking ref holds the tip and has no
        // reflog, so the first entry is the forced fetch itself.
        run(
            &repo,
            &[
                "git",
                "reflog",
                "expire",
                "--expire=all",
                "refs/remotes/origin/main",
            ],
        );
        assert_eq!(
            run(
                &repo,
                &["git", "reflog", "show", "refs/remotes/origin/main"]
            ),
            ""
        );
        let old = ref_of(&repo, "refs/heads/main");
        let squashed = squash_upstream(&bare);
        forget_sync_memo();
        let line = sync_default_from_upstream(&s, &proj, "daemon", TEST_NOW, false);
        assert!(
            line.as_deref()
                .is_some_and(|l| l.contains("landing realigned proj: main -> origin/main")),
            "{line:?}"
        );
        assert_eq!(ref_of(&repo, "refs/heads/main"), squashed);
        assert_eq!(ref_of(&repo, "refs/hive/backup/main/20260906T100000Z"), old);
    }

    fn realigned(line: &Option<String>) -> bool {
        line.as_deref()
            .is_some_and(|l| l.contains("landing realigned proj: main -> origin/main"))
    }

    /// A commit of the owner's on main that was never pushed.
    fn commit_local_work(repo: &str, name: &str) -> String {
        std::fs::write(Path::new(repo).join(name), "never pushed\n").unwrap();
        run(repo, &["git", "add", "-A"]);
        run(repo, &["git", "commit", "-q", "-m", "local work"]);
        ref_of(repo, "refs/heads/main")
    }

    /// One more commit on the upstream's main, with the files it had.
    fn commit_upstream(bare: &str) -> String {
        let tip = bare_git(bare, &["rev-parse", "refs/heads/main"]);
        let tree = bare_git(bare, &["rev-parse", "refs/heads/main^{tree}"]);
        let next = bare_git(bare, &["commit-tree", &tree, "-p", &tip, "-m", "more"]);
        bare_git(bare, &["update-ref", "refs/heads/main", &next]);
        next
    }

    /// The store refuses every new post, as a board that cannot be written
    /// does; `accept_posts` ends it.
    fn refuse_posts(s: &hive_store::Store) {
        exec1(
            s,
            "CREATE TRIGGER no_posts BEFORE INSERT ON posts BEGIN SELECT RAISE(ABORT, 'the board cannot be written'); END",
            &[],
        );
    }

    fn accept_posts(s: &hive_store::Store) {
        exec1(s, "DROP TRIGGER no_posts", &[]);
    }

    #[test]
    fn a_dirty_worktree_that_has_the_default_branch_is_not_moved_under() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = new_store();
        let (proj, repo, bare) = followed_fixture(&s);
        let old = ref_of(&repo, "refs/heads/main");
        let squashed = squash_upstream(&bare);
        // The project's own checkout is on another branch and clean; main
        // is checked out in a linked worktree, with an edit not committed.
        run(&repo, &["git", "checkout", "-q", "-b", "work/elsewhere"]);
        let wt = Path::new(&s.home)
            .join("tmp")
            .join(format!("main-{}", uuid::Uuid::new_v4()));
        let wt_dir = wt.display().to_string();
        run(&repo, &["git", "worktree", "add", "-q", &wt_dir, "main"]);
        let readme = wt.join("README.md");
        std::fs::write(&readme, "# edited in a worktree\n").unwrap();
        forget_sync_memo();
        let line = sync_default_from_upstream(&s, &proj, "daemon", TEST_NOW, false);
        assert!(
            line.as_deref()
                .is_some_and(|l| l.contains("proj: main has diverged from origin/main")
                    && l.contains("has uncommitted changes")),
            "{line:?}"
        );
        assert_eq!(ref_of(&repo, "refs/heads/main"), old);
        assert_eq!(backup_refs(&repo), "");
        assert_eq!(count_events(&s, "land.realigned"), 0);
        assert_eq!(count_events(&s, "land.fast-forward"), 1);
        assert_eq!(diverged_questions(&s), (1, 1));

        // The edit is put away, and a rebase of main stops in that
        // worktree: it is clean now and still not moved under.
        run(&wt_dir, &["git", "stash", "-q"]);
        run(
            &wt_dir,
            &[
                "sh",
                "-c",
                "git rebase -q -x 'test -f go' HEAD~1 >/dev/null 2>&1 || true",
            ],
        );
        assert_eq!(head_branch(&wt_dir), "HEAD", "the rebase did not stop");
        let later = crate::util::plus_minutes(TEST_NOW, 2);
        assert_eq!(
            sync_default_from_upstream(&s, &proj, "daemon", &later, true),
            None,
            "the same pair said twice"
        );
        assert_eq!(ref_of(&repo, "refs/heads/main"), old);
        assert_eq!(count_events(&s, "land.realigned"), 0);

        // The rebase ends: the next pass realigns.
        std::fs::write(wt.join("go"), "").unwrap();
        run(&wt_dir, &["git", "rebase", "--continue"]);
        let later = crate::util::plus_minutes(TEST_NOW, 4);
        let line = sync_default_from_upstream(&s, &proj, "daemon", &later, true);
        assert!(realigned(&line), "{line:?}");
        assert_eq!(ref_of(&repo, "refs/heads/main"), squashed);
        assert_eq!(ref_of(&wt_dir, "HEAD"), squashed);
        assert_eq!(
            run(&wt_dir, &["git", "status", "--porcelain"]),
            "?? go\n",
            "the worktree is not as it was"
        );
        assert_eq!(diverged_questions(&s), (1, 0));
        run(&wt_dir, &["git", "stash", "pop", "-q"]);
        assert_eq!(
            std::fs::read_to_string(&readme).unwrap(),
            "# edited in a worktree\n"
        );
    }

    #[test]
    fn a_rebase_of_the_default_branch_in_progress_is_not_moved_under() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = new_store();
        let (proj, repo, bare) = followed_fixture(&s);
        let old = ref_of(&repo, "refs/heads/main");
        let squashed = squash_upstream(&bare);
        // The owner's rebase of main stops at a step that fails: HEAD is
        // detached, the checkout is clean, and git ends the rebase with an
        // update of main from the tip it started at.
        run(
            &repo,
            &[
                "sh",
                "-c",
                "git rebase -q -x 'test -f go' HEAD~1 >/dev/null 2>&1 || true",
            ],
        );
        assert_eq!(head_branch(&repo), "HEAD", "the rebase did not stop");
        forget_sync_memo();
        let line = sync_default_from_upstream(&s, &proj, "daemon", TEST_NOW, false);
        assert!(
            line.as_deref()
                .is_some_and(|l| l.contains("proj: main has diverged from origin/main")
                    && l.contains("a rebase is in progress on it")),
            "{line:?}"
        );
        assert_eq!(ref_of(&repo, "refs/heads/main"), old);
        assert_eq!(backup_refs(&repo), "");
        assert_eq!(count_events(&s, "land.realigned"), 0);

        // The step passes now and the rebase ends as it would have without
        // Hive; the next pass realigns.
        std::fs::write(Path::new(&repo).join("go"), "").unwrap();
        run(&repo, &["git", "rebase", "--continue"]);
        assert_eq!(head_branch(&repo), "main");
        let later = crate::util::plus_minutes(TEST_NOW, 2);
        let line = sync_default_from_upstream(&s, &proj, "daemon", &later, true);
        assert!(realigned(&line), "{line:?}");
        assert_eq!(ref_of(&repo, "refs/heads/main"), squashed);
    }

    #[test]
    fn a_merge_cherry_pick_or_bisect_on_the_default_branch_is_not_moved_under() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        // What is under way, and how the line names it. The first three
        // leave the checkout clean, so only git's own files say so; the
        // last is a rebase by the apply backend, stopped on a conflict.
        for (begun, what) in [
            ("merge", "merge"),
            ("cherry-pick", "cherry-pick"),
            ("bisect", "bisect"),
            ("rebase --apply", "rebase"),
        ] {
            let s = new_store();
            let (proj, repo, bare) = followed_fixture(&s);
            let old = ref_of(&repo, "refs/heads/main");
            let squashed = squash_upstream(&bare);
            let git = |args: &[&str]| {
                let mut argv = vec!["git"];
                argv.extend_from_slice(args);
                run(&repo, &argv);
            };
            let side_branch = |from: &str, file: &str| {
                git(&["checkout", "-q", "-b", "side", from]);
                std::fs::write(Path::new(&repo).join(file), "side\n").unwrap();
                git(&["add", "-A"]);
                git(&["commit", "-q", "-m", "side"]);
                git(&["checkout", "-q", "main"]);
            };
            match begun {
                "merge" => {
                    side_branch("main", "side.txt");
                    git(&["merge", "-q", "-s", "ours", "--no-commit", "side"]);
                }
                "cherry-pick" => {
                    run(
                        &repo,
                        &["sh", "-c", "git cherry-pick HEAD >/dev/null 2>&1 || true"],
                    );
                }
                "bisect" => {
                    git(&["bisect", "start"]);
                    git(&["bisect", "bad"]);
                    git(&["bisect", "good", "HEAD~2"]);
                }
                _ => {
                    side_branch("main~1", "second.txt");
                    run(
                        &repo,
                        &[
                            "sh",
                            "-c",
                            "git rebase --apply --onto side HEAD~1 >/dev/null 2>&1 || true",
                        ],
                    );
                }
            }
            if begun != "rebase --apply" {
                assert_eq!(
                    run(&repo, &["git", "status", "--porcelain"]),
                    "",
                    "{begun}: the checkout is not clean"
                );
            }
            forget_sync_memo();
            let line = sync_default_from_upstream(&s, &proj, "daemon", TEST_NOW, false);
            assert!(
                line.as_deref()
                    .is_some_and(|l| l.contains(&format!("a {what} is in progress on it"))),
                "{begun}: {line:?}"
            );
            assert_eq!(ref_of(&repo, "refs/heads/main"), old, "{begun}");
            assert_eq!(count_events(&s, "land.realigned"), 0, "{begun}");

            match begun {
                "merge" => git(&["merge", "--abort"]),
                "cherry-pick" => git(&["cherry-pick", "--abort"]),
                "bisect" => git(&["bisect", "reset"]),
                _ => git(&["rebase", "--abort"]),
            }
            let later = crate::util::plus_minutes(TEST_NOW, 2);
            let line = sync_default_from_upstream(&s, &proj, "daemon", &later, true);
            assert!(realigned(&line), "{begun}: {line:?}");
            assert_eq!(ref_of(&repo, "refs/heads/main"), squashed, "{begun}");
        }
    }

    #[test]
    fn a_worktree_git_cannot_read_is_not_moved_under() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = new_store();
        let (proj, repo, bare) = followed_fixture(&s);
        let old = ref_of(&repo, "refs/heads/main");
        let squashed = squash_upstream(&bare);
        // A linked worktree on a branch of its own whose `.git` file
        // points nowhere: its folder is there, and git cannot say whether
        // a rebase or a bisect of main is under way in it.
        let wt = Path::new(&s.home)
            .join("tmp")
            .join(format!("other-{}", uuid::Uuid::new_v4()));
        let wt_dir = wt.display().to_string();
        run(
            &repo,
            &[
                "git",
                "worktree",
                "add",
                "-q",
                "-b",
                "work/other",
                &wt_dir,
                "main",
            ],
        );
        let link = wt.join(".git");
        let was = std::fs::read(&link).unwrap();
        std::fs::write(&link, "gitdir: /nowhere/at/all\n").unwrap();
        forget_sync_memo();
        let line = sync_default_from_upstream(&s, &proj, "daemon", TEST_NOW, false);
        assert!(
            line.as_deref()
                .is_some_and(|l| l.contains("proj: main has diverged from origin/main")
                    && l.contains("git could not read the worktree at")),
            "{line:?}"
        );
        assert_eq!(ref_of(&repo, "refs/heads/main"), old);
        assert_eq!(backup_refs(&repo), "");
        assert_eq!(count_events(&s, "land.realigned"), 0);

        // The worktree is mended: the next pass realigns.
        std::fs::write(&link, was).unwrap();
        let later = crate::util::plus_minutes(TEST_NOW, 2);
        let line = sync_default_from_upstream(&s, &proj, "daemon", &later, true);
        assert!(realigned(&line), "{line:?}");
        assert_eq!(ref_of(&repo, "refs/heads/main"), squashed);
    }

    #[test]
    fn a_checkout_git_cannot_read_is_not_said_to_have_uncommitted_changes() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = new_store();
        let (proj, repo, bare) = followed_fixture(&s);
        let old = ref_of(&repo, "refs/heads/main");
        let squashed = squash_upstream(&bare);
        // Main is checked out in a linked worktree whose index git cannot
        // read: `git status` fails there, which says nothing about changes.
        run(&repo, &["git", "checkout", "-q", "-b", "work/elsewhere"]);
        let wt = Path::new(&s.home)
            .join("tmp")
            .join(format!("main-{}", uuid::Uuid::new_v4()));
        let wt_dir = wt.display().to_string();
        run(&repo, &["git", "worktree", "add", "-q", &wt_dir, "main"]);
        let index = Path::new(run(&wt_dir, &["git", "rev-parse", "--absolute-git-dir"]).trim())
            .join("index");
        let was = std::fs::read(&index).unwrap();
        std::fs::write(&index, "not an index\n").unwrap();
        forget_sync_memo();
        let line = sync_default_from_upstream(&s, &proj, "daemon", TEST_NOW, false);
        assert!(
            line.as_deref()
                .is_some_and(|l| l.contains("git could not read the worktree at")
                    && !l.contains("uncommitted changes")),
            "{line:?}"
        );
        assert_eq!(ref_of(&repo, "refs/heads/main"), old);
        assert_eq!(count_events(&s, "land.realigned"), 0);

        std::fs::write(&index, was).unwrap();
        let later = crate::util::plus_minutes(TEST_NOW, 2);
        let line = sync_default_from_upstream(&s, &proj, "daemon", &later, true);
        assert!(realigned(&line), "{line:?}");
        assert_eq!(ref_of(&repo, "refs/heads/main"), squashed);
    }

    #[test]
    fn a_note_of_gits_that_cannot_be_read_is_not_moved_under() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = new_store();
        let (proj, repo, bare) = followed_fixture(&s);
        let old = ref_of(&repo, "refs/heads/main");
        let squashed = squash_upstream(&bare);
        // A rebase stopped in the checkout, and the file in which git
        // names the branch it rebases cannot be read: it may be main.
        run(
            &repo,
            &[
                "sh",
                "-c",
                "git rebase -q -x 'test -f go' HEAD~1 >/dev/null 2>&1 || true",
            ],
        );
        assert_eq!(head_branch(&repo), "HEAD", "the rebase did not stop");
        run(&repo, &["chmod", "000", ".git/rebase-merge/head-name"]);
        forget_sync_memo();
        let line = sync_default_from_upstream(&s, &proj, "daemon", TEST_NOW, false);
        // (A superuser reads the file all the same and sees the rebase.)
        assert!(
            line.as_deref()
                .is_some_and(|l| l.contains("proj: main has diverged from origin/main")
                    && (l.contains("git could not read the worktree at")
                        || l.contains("a rebase is in progress on it"))),
            "{line:?}"
        );
        assert_eq!(ref_of(&repo, "refs/heads/main"), old);
        assert_eq!(count_events(&s, "land.realigned"), 0);

        run(&repo, &["chmod", "644", ".git/rebase-merge/head-name"]);
        run(&repo, &["git", "rebase", "--abort"]);
        let later = crate::util::plus_minutes(TEST_NOW, 2);
        let line = sync_default_from_upstream(&s, &proj, "daemon", &later, true);
        assert!(realigned(&line), "{line:?}");
        assert_eq!(ref_of(&repo, "refs/heads/main"), squashed);
    }

    #[test]
    fn an_ask_the_store_refuses_is_made_on_a_later_pass() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = new_store();
        let (proj, repo, bare) = followed_fixture(&s);
        commit_local_work(&repo, "mine.txt");
        squash_upstream(&bare);
        refuse_posts(&s);
        forget_sync_memo();
        let line = sync_default_from_upstream(&s, &proj, "daemon", TEST_NOW, false);
        assert!(
            line.as_deref()
                .is_some_and(|l| l.contains("proj: main has diverged from origin/main")
                    && l.contains("the owner could not be asked")),
            "{line:?}"
        );
        assert_eq!(count_events(&s, "land.fast-forward"), 1);
        assert_eq!(diverged_questions(&s), (0, 0));

        // Still refused: the pair is not recorded a second time and
        // nothing is said.
        let later = crate::util::plus_minutes(TEST_NOW, 2);
        assert_eq!(
            sync_default_from_upstream(&s, &proj, "daemon", &later, true),
            None
        );
        assert_eq!(count_events(&s, "land.fast-forward"), 1);

        // The board can be written again: the owner is asked, once.
        accept_posts(&s);
        let later = crate::util::plus_minutes(TEST_NOW, 4);
        let line = sync_default_from_upstream(&s, &proj, "daemon", &later, true);
        assert!(
            line.as_deref()
                .is_some_and(|l| l.contains("the owner is asked")),
            "{line:?}"
        );
        assert_eq!(diverged_questions(&s), (1, 1));
        assert_eq!(count_events(&s, "land.fast-forward"), 1);
        let later = crate::util::plus_minutes(TEST_NOW, 6);
        assert_eq!(
            sync_default_from_upstream(&s, &proj, "daemon", &later, true),
            None
        );
        assert_eq!(diverged_questions(&s), (1, 1));
    }

    #[test]
    fn an_answer_the_store_refuses_is_given_on_a_later_pass() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = new_store();
        let (proj, repo, bare) = followed_fixture(&s);
        commit_local_work(&repo, "mine.txt");
        squash_upstream(&bare);
        forget_sync_memo();
        assert!(sync_default_from_upstream(&s, &proj, "daemon", TEST_NOW, false).is_some());
        assert_eq!(diverged_questions(&s), (1, 1));

        // The owner settles it while the board cannot be written: the
        // question stays open, and so does what Hive owes it.
        run(&repo, &["git", "reset", "-q", "--keep", "origin/main"]);
        refuse_posts(&s);
        let later = crate::util::plus_minutes(TEST_NOW, 2);
        assert_eq!(
            sync_default_from_upstream(&s, &proj, "daemon", &later, true),
            None
        );
        assert_eq!(diverged_questions(&s), (1, 1));

        accept_posts(&s);
        let later = crate::util::plus_minutes(TEST_NOW, 4);
        assert_eq!(
            sync_default_from_upstream(&s, &proj, "daemon", &later, true),
            None
        );
        assert_eq!(diverged_questions(&s), (1, 0), "the answer was never given");
    }

    #[test]
    fn a_second_divergence_after_the_first_ended_is_recorded_and_asked_again() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = new_store();
        let (proj, repo, bare) = followed_fixture(&s);
        commit_local_work(&repo, "mine.txt");
        squash_upstream(&bare);
        forget_sync_memo();
        assert!(sync_default_from_upstream(&s, &proj, "daemon", TEST_NOW, false).is_some());
        assert_eq!(count_events(&s, "land.fast-forward"), 1);
        assert_eq!(diverged_questions(&s), (1, 1));

        // The owner keeps the upstream's history: the first one is over.
        run(&repo, &["git", "reset", "-q", "--keep", "origin/main"]);
        let later = crate::util::plus_minutes(TEST_NOW, 2);
        assert_eq!(
            sync_default_from_upstream(&s, &proj, "daemon", &later, true),
            None
        );
        assert_eq!(diverged_questions(&s), (1, 0));

        // Weeks later the same happens again: a commit here that is never
        // pushed, and an upstream that moves on without it.
        commit_local_work(&repo, "again.txt");
        commit_upstream(&bare);
        let later = crate::util::plus_minutes(TEST_NOW, 4);
        let line = sync_default_from_upstream(&s, &proj, "daemon", &later, true);
        assert!(
            line.as_deref().is_some_and(
                |l| l.contains("(1 ahead, 1 behind)") && l.contains("the owner is asked")
            ),
            "{line:?}"
        );
        assert_eq!(count_events(&s, "land.fast-forward"), 2);
        assert_eq!(diverged_questions(&s), (2, 1));
    }

    #[test]
    fn realign_refuses_a_branch_that_moved_since_it_was_read() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = new_store();
        let (_proj, repo, _bare) = followed_fixture(&s);
        // What a pass read, and the commit the owner made right after it.
        let read = ref_of(&repo, "refs/heads/main");
        let tip = commit_local_work(&repo, "mine.txt");
        let tree = ref_of(&repo, &format!("{read}^{{tree}}"));
        let upstream = run(&repo, &["git", "commit-tree", &tree, "-m", "rewritten"])
            .trim()
            .to_string();
        let tips = Tips {
            from: &read,
            to: &upstream,
            behind: 1,
        };
        let refused = realign(&repo, "main", "origin/main", &tips, TEST_NOW);
        assert!(refused.is_err(), "{refused:?}");
        assert_eq!(
            ref_of(&repo, "refs/heads/main"),
            tip,
            "the owner's commit is off the branch"
        );
    }

    #[test]
    fn realign_never_overwrites_a_backup() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = new_store();
        let (_proj, repo, _bare) = followed_fixture(&s);
        let first = ref_of(&repo, "refs/heads/main");
        let tree = ref_of(&repo, "refs/heads/main^{tree}");
        let rewritten = |message: &str| {
            run(&repo, &["git", "commit-tree", &tree, "-m", message])
                .trim()
                .to_string()
        };
        let (second, third) = (rewritten("rewritten"), rewritten("rewritten again"));
        let tips = Tips {
            from: &first,
            to: &second,
            behind: 1,
        };
        let kept = realign(&repo, "main", "origin/main", &tips, TEST_NOW).unwrap();
        assert_eq!(kept, "refs/hive/backup/main/20260906T100000Z");
        assert_eq!(ref_of(&repo, &kept), first);
        assert_eq!(ref_of(&repo, "refs/heads/main"), second);

        // Another tip in the same second: the name is taken, so the tip
        // cannot be kept and the branch stays where it is.
        let tips = Tips {
            from: &second,
            to: &third,
            behind: 1,
        };
        let refused = realign(&repo, "main", "origin/main", &tips, TEST_NOW);
        assert!(
            refused
                .as_ref()
                .is_err_and(|why| why.contains("could not be kept")),
            "{refused:?}"
        );
        assert_eq!(ref_of(&repo, &kept), first, "the backup was overwritten");
        assert_eq!(
            ref_of(&repo, "refs/heads/main"),
            second,
            "the branch moved without its tip kept"
        );
    }
}
