//! Worktrees and the disk they take: what `git worktree list` says, whether
//! two paths are one place, and how big a tree is. The daemon's sweep and the
//! hand's leftover tidy both read these.

use std::path::Path;

/// One entry of `git worktree list --porcelain`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: String,
    pub head: String,
    /// Short branch name; empty when the worktree is detached or bare.
    pub branch: String,
    pub detached: bool,
    pub bare: bool,
    /// `Some(reason)` when locked; the reason may be empty.
    pub locked: Option<String>,
    pub prunable: bool,
}

/// Parses `git worktree list --porcelain`; the first entry is the main worktree.
pub fn parse_worktrees(porcelain: &str) -> Vec<WorktreeEntry> {
    let mut out: Vec<WorktreeEntry> = Vec::new();
    let mut cur: Option<WorktreeEntry> = None;
    for line in porcelain.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            out.extend(cur.take());
            cur = Some(WorktreeEntry {
                path: path.to_string(),
                ..Default::default()
            });
            continue;
        }
        let Some(e) = cur.as_mut() else { continue };
        if let Some(head) = line.strip_prefix("HEAD ") {
            e.head = head.trim().to_string();
        } else if let Some(branch) = line.strip_prefix("branch ") {
            e.branch = branch
                .trim()
                .strip_prefix("refs/heads/")
                .unwrap_or(branch.trim())
                .to_string();
        } else if line == "detached" {
            e.detached = true;
        } else if line == "bare" {
            e.bare = true;
        } else if line == "locked" {
            e.locked = Some(String::new());
        } else if let Some(reason) = line.strip_prefix("locked ") {
            e.locked = Some(reason.trim().to_string());
        } else if line == "prunable" || line.starts_with("prunable ") {
            e.prunable = true;
        }
    }
    out.extend(cur);
    out
}

/// True when both paths name the same place on disk.
pub fn same_path(a: &str, b: &str) -> bool {
    if a.is_empty() || b.is_empty() {
        return false;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// Bytes under `path`, symlinks not followed.
pub fn disk_usage(path: String) -> i64 {
    fn walk(path: &Path, n: &mut i64) {
        let Ok(meta) = std::fs::symlink_metadata(path) else {
            return;
        };
        if meta.is_file() {
            *n += meta.len() as i64;
        } else if meta.is_dir()
            && let Ok(kids) = std::fs::read_dir(path)
        {
            for k in kids.flatten() {
                walk(&k.path(), n);
            }
        }
    }
    let mut n = 0;
    walk(Path::new(&path), &mut n);
    n
}

/// A byte count the way a person reads it.
pub fn size_text(n: i64) -> String {
    if n >= 1 << 30 {
        format!("{:.1} GB", n as f64 / (1 << 30) as f64)
    } else if n >= 1 << 20 {
        format!("{:.1} MB", n as f64 / (1 << 20) as f64)
    } else if n >= 1 << 10 {
        format!("{:.0} KB", n as f64 / (1 << 10) as f64)
    } else {
        format!("{n} B")
    }
}
