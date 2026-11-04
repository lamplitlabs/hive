//! Git and forge plumbing the per-task pull requests (`task_pr.rs`) and the
//! landings that remain (evolve's experiment lane, `pulse land --branch`)
//! share: the default branch's upstream, the daemon's push credential, the
//! fetch of the upstream and the fast-forward of the local default branch
//! once the forge merged something.

use std::time::Duration;

/// Where a push goes: the default branch's upstream, the url the push uses
/// and the environment that carries the daemon's credential.
pub(crate) struct PushTarget {
    pub(crate) root: String,
    pub(crate) remote: String,
    pub(crate) remote_branch: String,
    /// `refs/remotes/<remote>/<branch>`, the tracking ref the fetch updates.
    pub(crate) upstream: String,
    pub(crate) url: String,
    pub(crate) env: Vec<String>,
}

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

pub(crate) fn fetch_upstream(t: &PushTarget) -> crate::util::GitResult {
    crate::util::git_timeout_env(
        &t.root,
        Duration::from_secs(120),
        &t.env,
        &[
            "fetch",
            "--quiet",
            "--no-tags",
            &t.url,
            &format!("+refs/heads/{}:{}", t.remote_branch, t.upstream),
        ],
    )
}

/// Directory prefix of the hub's generated static export.
pub(crate) const HUB_OUT_PREFIX: &str = "web/hub/out/";

/// Rebuild the hub's static export from the merged `web/hub/src` when a
/// landing's merge conflicted only under the generated export, and commit
/// the merge with the rebuilt bundle. Two branches that both touch the hub's
/// source always collide there - every build names its chunks by content
/// hash - so without this a finished run was thrown away and its job run
/// again for a conflict no one wrote: of the nine amistio-template landings
/// that conflicted on 2026-11-02..04, every patch was 80-90% `web/hub/out`
/// and the sources had merged clean. Any branch gets the rebuild, not only
/// an upstream sync. The export leaves the index and the disk before the
/// build: the stamp the bundle test reads hashes the staged `src` with
/// `git write-tree`, which refuses an index that still holds unmerged
/// entries, and a chunk the losing side left on disk would otherwise ride
/// into the commit. Returns false, with the merge aborted, when the conflict
/// reaches outside the export or any step failed, so the usual hold follows.
pub(crate) fn resolve_hub_out_conflict(worktree: &str, branch: &str) -> bool {
    let unmerged = crate::util::git_timeout(
        worktree,
        Duration::from_secs(30),
        &["diff", "--name-only", "--diff-filter=U", "-z"],
    );
    if !unmerged.ok()
        || unmerged
            .stdout
            .split('\0')
            .any(|p| !p.is_empty() && !p.starts_with(HUB_OUT_PREFIX))
    {
        return false;
    }
    let abort = || {
        crate::util::git_timeout(worktree, Duration::from_secs(60), &["merge", "--abort"]);
        false
    };
    let out_dir = HUB_OUT_PREFIX.trim_end_matches('/');
    if !crate::util::git_timeout(
        worktree,
        Duration::from_secs(120),
        &[
            "rm",
            "-r",
            "-f",
            "-q",
            "--cached",
            "--ignore-unmatch",
            "--",
            out_dir,
        ],
    )
    .ok()
    {
        return abort();
    }
    let _ = std::fs::remove_dir_all(std::path::Path::new(worktree).join(out_dir));
    // Rebuild the export in place: the landing worktree's disk already holds
    // the merged web/hub/src (only generated out/ paths conflicted, so src
    // merged clean), which is exactly the tree the stamp gate names. Build
    // over it with npm, then stage the result and finish the merge commit.
    let hub = std::path::Path::new(worktree).join("web/hub");
    let npm = |args: &[&str]| {
        std::process::Command::new("npm")
            .args(args)
            .current_dir(&hub)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_QUARANTINE_PATH")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|st| st.success())
    };
    if !npm(&["ci", "--no-audit", "--no-fund"]) || !npm(&["run", "build"]) {
        return abort();
    }
    if !crate::util::git_timeout(
        worktree,
        Duration::from_secs(120),
        &["add", "-A", "--", "web/hub"],
    )
    .ok()
    {
        return abort();
    }
    let msg = format!("Land {branch}: rebuild web/hub/out from the merged src");
    let committed = crate::util::git_timeout(
        worktree,
        Duration::from_secs(120),
        &[
            "-c",
            "user.name=pulse",
            "-c",
            "user.email=pulse@pulse.local",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--no-verify",
            "-m",
            &msg,
        ],
    )
    .ok();
    if !committed {
        crate::util::git_timeout(worktree, Duration::from_secs(60), &["merge", "--abort"]);
        return false;
    }
    true
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
    let (on_default, clean) = {
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
    };
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
