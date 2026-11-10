//! Bounded steady-state retention for what PULSE keeps on this machine:
//! a trash folder that makes removing a clone instant, a size cap for the
//! daemon's own logs, the stale binaries the self-update and the launcher
//! leave in `bin/`, and the harness trajectory files the hub can download.
//! The periodic sweep (`sweep.rs`) calls these; a run's own cleanup uses
//! `discard_dir` so the slow unlinking of a dependency tree never happens on
//! the thread that ends the run.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The trash lives in the hand's core (ADR-070 step 1c): a job's clone is
/// discarded the same way by the brain and by a hand.
#[allow(unused_imports)]
pub use pulse_hand_core::trash::{
    Discarded, TRASH_DIR, TrashReport, discard_dir, empty_trash, empty_trash_later,
    remove_at_low_priority, trash_dir, trash_entries,
};

/// `bin/pulse.*` siblings the runner itself keeps: the binary in use, the
/// previous one the launcher's crash guard falls back to, a build about to be
/// installed, and the marker the launcher writes after a restore.
pub const KEPT_BINARIES: &[&str] = &["pulse", "pulse.prev", "pulse.new", "pulse.restored"];

/// How long another `bin/pulse.*` file (a `pulse.failed` the launcher set
/// aside, a `pulse.rollback-*` or an older build) stays before the sweep
/// removes it; long enough to diagnose the crash that made it.
pub const STALE_BINARY_AFTER: Duration = Duration::from_secs(7 * 24 * 3600);

/// The log files the launcher writes the daemon's and the hub's output to.
pub const CAPPED_LOGS: &[&str] = &["daemon.log", "hub.log"];

/// What capping a log did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CappedLog {
    pub path: PathBuf,
    pub bytes: u64,
    pub kept_as: PathBuf,
}

/// Copies `path` to `<path>.1` (replacing the previous copy) and truncates it
/// in place once it is over `max_bytes`. The launcher opens these logs in
/// append mode, so the live process keeps writing from the new end; the copy
/// keeps one generation of history. Returns `None` when nothing was done.
pub fn cap_log(path: &Path, max_bytes: u64) -> std::io::Result<Option<CappedLog>> {
    if max_bytes == 0 {
        return Ok(None);
    }
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    if !meta.is_file() || meta.len() <= max_bytes {
        return Ok(None);
    }
    let mut kept_as = path.as_os_str().to_owned();
    kept_as.push(".1");
    let kept_as = PathBuf::from(kept_as);
    std::fs::copy(path, &kept_as)?;
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)?
        .set_len(0)?;
    Ok(Some(CappedLog {
        path: path.to_path_buf(),
        bytes: meta.len(),
        kept_as,
    }))
}

/// A stale file under `bin/`: a `pulse.*` sibling the runner does not keep
/// that has sat for `STALE_BINARY_AFTER`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleBinary {
    pub path: PathBuf,
    pub bytes: u64,
    pub age: Duration,
}

/// The `bin/pulse.*` files older than `STALE_BINARY_AFTER` that nothing
/// keeps: `pulse.failed` and `pulse.rollback-*` the launcher and the
/// self-update set aside, older builds copied by hand. Never the binary in
/// use, `pulse.prev`, `pulse.new`, the restore marker or the launch scripts.
pub fn stale_binaries(bin: &Path, now: std::time::SystemTime) -> Vec<StaleBinary> {
    let kept: BTreeSet<&str> = KEPT_BINARIES.iter().copied().collect();
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(bin) else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("pulse.") || kept.contains(name.as_str()) {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let Some(age) = meta
            .modified()
            .ok()
            .and_then(|m| now.duration_since(m).ok())
        else {
            continue;
        };
        if age < STALE_BINARY_AFTER {
            continue;
        }
        out.push(StaleBinary {
            path: entry.path(),
            bytes: meta.len(),
            age,
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// A harness trajectory file (`PULSE_HOME/harness/<session>.json`) the sweep
/// may remove: older than the transcript retention and not a running
/// session's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleTrajectory {
    pub path: PathBuf,
    pub session: String,
    pub bytes: u64,
    pub age: Duration,
}

/// The trajectory files under `harness/` whose last write is older than
/// `keep`, skipping every session in `running` (ids or paths). The file is
/// a copy of the session's `harness_steps` rows the hub offers for download
/// and tolerates missing; the rows themselves are pruned by the store.
pub fn stale_trajectories(
    harness: &Path,
    keep: Duration,
    now: std::time::SystemTime,
    running: &[String],
) -> Vec<StaleTrajectory> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(harness) else {
        return out;
    };
    let running: BTreeSet<&str> = running.iter().map(|s| s.as_str()).collect();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(session) = name.strip_suffix(".json") else {
            continue;
        };
        let path = entry.path();
        if running.contains(session) || running.contains(path.display().to_string().as_str()) {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let Some(age) = meta
            .modified()
            .ok()
            .and_then(|m| now.duration_since(m).ok())
        else {
            continue;
        };
        if age < keep {
            continue;
        }
        out.push(StaleTrajectory {
            path,
            session: session.to_string(),
            bytes: meta.len(),
            age,
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use filetime::FileTime;

    fn mkfile(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn age_path(path: &Path, days: i64) {
        let t = FileTime::from_unix_time(FileTime::now().unix_seconds() - days * 24 * 3600, 0);
        filetime::set_file_mtime(path, t).unwrap();
    }

    #[test]
    fn cap_log_keeps_one_generation_and_truncates_in_place() {
        let home = tempfile::tempdir().unwrap();
        let log = home.path().join("logs/daemon.log");
        mkfile(&log, &"line\n".repeat(1000));
        assert_eq!(cap_log(&log, 10_000).unwrap(), None, "under the cap");
        assert_eq!(cap_log(&log, 0).unwrap(), None, "0 never caps");
        let got = cap_log(&log, 1000).unwrap().unwrap();
        assert_eq!(got.bytes, 5000);
        assert_eq!(std::fs::metadata(&log).unwrap().len(), 0);
        assert_eq!(std::fs::metadata(&got.kept_as).unwrap().len(), 5000);
        assert_eq!(got.kept_as.file_name().unwrap(), "daemon.log.1");
        assert_eq!(
            cap_log(&home.path().join("logs/missing.log"), 10).unwrap(),
            None
        );
    }

    #[test]
    fn stale_binaries_keep_the_runner_s_own() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        for name in [
            "pulse",
            "pulse.prev",
            "pulse.new",
            "pulse.restored",
            "pulse-daemon.sh",
            "pulse.failed",
            "pulse.rollback-20260913T073142Z",
            "pulse.go-28f35b8",
            "pulse.fresh",
        ] {
            mkfile(&bin.join(name), name);
            if name != "pulse.fresh" {
                age_path(&bin.join(name), 10);
            }
        }
        std::fs::create_dir_all(bin.join("pulse.dir")).unwrap();
        age_path(&bin.join("pulse.dir"), 10);
        let got: Vec<String> = stale_binaries(&bin, std::time::SystemTime::now())
            .into_iter()
            .map(|b| b.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(
            got,
            vec![
                "pulse.failed".to_string(),
                "pulse.go-28f35b8".into(),
                "pulse.rollback-20260913T073142Z".into()
            ]
        );
    }

    #[test]
    fn stale_trajectories_skip_running_and_recent() {
        let home = tempfile::tempdir().unwrap();
        let harness = home.path().join("harness");
        for name in [
            "run-old.json",
            "run-running.json",
            "run-new.json",
            "notes.txt",
        ] {
            mkfile(&harness.join(name), "{}");
        }
        for name in ["run-old.json", "run-running.json", "notes.txt"] {
            age_path(&harness.join(name), 20);
        }
        let running = vec![
            "run-running".to_string(),
            harness.join("run-running.json").display().to_string(),
        ];
        let got = stale_trajectories(
            &harness,
            Duration::from_secs(14 * 24 * 3600),
            std::time::SystemTime::now(),
            &running,
        );
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].session, "run-old");
        assert_eq!(got[0].bytes, 2);
    }
}
