//! The trash a job's clone goes into: removing a clone is one rename, the
//! same instant for a 1.6 GB `node_modules` as for an empty folder, and a
//! background remover deletes it at low priority. A run's own cleanup uses
//! `discard_dir` so the slow unlinking of a dependency tree never happens on
//! the thread that ends the run; the hand clears its clones the same way.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Where discarded folders wait for the background remover, under
/// `HIVE_HOME/work` so a rename from a clone is one directory operation on
/// the same file system.
pub const TRASH_DIR: &str = "work/.trash";

static TRASH_WORKER: AtomicBool = AtomicBool::new(false);

/// Homes whose trash has a discard waiting; the single remover thread drains
/// every one of them, so a discard that lands while a pass runs (or under a
/// different `HIVE_HOME`, as the tests do) is never left behind.
static TRASH_PENDING: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

fn trash_pending() -> std::sync::MutexGuard<'static, Vec<PathBuf>> {
    TRASH_PENDING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn take_pending_home() -> Option<PathBuf> {
    let mut pending = trash_pending();
    (!pending.is_empty()).then(|| pending.remove(0))
}

pub fn trash_dir(home: &Path) -> PathBuf {
    home.join(TRASH_DIR)
}

/// What `discard_dir` did with a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Discarded {
    /// Renamed into the trash; the background remover deletes it.
    Trashed(PathBuf),
    /// Removed in place (the rename failed, so the trash is not on the same
    /// file system or cannot be created).
    Removed,
    /// Nothing was there.
    Missing,
}

/// Moves `path` into `HIVE_HOME/work/.trash` with one rename, which takes
/// the same instant for a 1.6 GB `node_modules` as for an empty folder, and
/// wakes the background remover. Files and symlinks are removed directly.
/// When the rename cannot work the folder is removed in place, so the
/// caller's folder is gone either way when this returns `Ok`.
pub fn discard_dir(home: &Path, path: &Path) -> std::io::Result<Discarded> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Discarded::Missing),
        Err(err) => return Err(err),
    };
    if !meta.is_dir() {
        std::fs::remove_file(path)?;
        return Ok(Discarded::Removed);
    }
    let trash = trash_dir(home);
    if !path_is_under(path, &trash) && std::fs::create_dir_all(&trash).is_ok() {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "dir".into());
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() % 1_000_000_000)
            .unwrap_or(0);
        let dest = trash.join(format!("{name}-{:x}-{nonce:x}", std::process::id()));
        if std::fs::rename(path, &dest).is_ok() {
            empty_trash_later(home);
            return Ok(Discarded::Trashed(dest));
        }
    }
    std::fs::remove_dir_all(path)?;
    Ok(Discarded::Removed)
}

/// Starts one background thread (never two) that deletes everything in the
/// trash at low priority, then exits. Cheap to call after every discard. A
/// home queued while the thread runs is taken by that thread; a home queued
/// as it exits starts a new one, so no discard waits for the next call.
pub fn empty_trash_later(home: &Path) {
    if !trash_dir(home).is_dir() {
        return;
    }
    {
        let mut pending = trash_pending();
        if !pending.iter().any(|h| h == home) {
            pending.push(home.to_path_buf());
        }
    }
    spawn_trash_worker();
}

fn spawn_trash_worker() {
    if TRASH_WORKER.swap(true, Ordering::SeqCst) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("trash-remover".into())
        .spawn(move || {
            while let Some(home) = take_pending_home() {
                // A discard that lands while a pass runs is taken by the next pass.
                loop {
                    let report = empty_trash(&home);
                    if report.entries == 0 || !report.errors.is_empty() {
                        break;
                    }
                }
            }
            TRASH_WORKER.store(false, Ordering::SeqCst);
            // A home queued between the last take and the store would wait
            // for the next discard; start another thread for it instead.
            if !trash_pending().is_empty() {
                spawn_trash_worker();
            }
        });
    if spawned.is_err() {
        TRASH_WORKER.store(false, Ordering::SeqCst);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrashReport {
    pub entries: usize,
    pub errors: Vec<String>,
}

/// Deletes every entry of the trash, one at a time, each through
/// `nice -n 15 rm -rf` (on macOS under `taskpolicy -b`, which also throttles
/// its I/O) so the laptop stays responsive while gigabytes unlink; a child
/// that cannot start or fails is finished in-process. Blocks until done.
pub fn empty_trash(home: &Path) -> TrashReport {
    let trash = trash_dir(home);
    let mut report = TrashReport::default();
    let Ok(entries) = std::fs::read_dir(&trash) else {
        return report;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path_is_under(&path, &trash) || path == trash {
            continue;
        }
        report.entries += 1;
        if let Err(err) = remove_at_low_priority(&path) {
            report
                .errors
                .push(format!("cannot empty {}: {err}", path.display()));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    report
}

pub fn remove_at_low_priority(path: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_dir() {
        return std::fs::remove_file(path);
    }
    let mut argv: Vec<String> = Vec::new();
    if cfg!(target_os = "macos") && Path::new("/usr/sbin/taskpolicy").exists() {
        argv.extend(["/usr/sbin/taskpolicy".into(), "-b".into()]);
    }
    argv.extend([
        "nice".into(),
        "-n".into(),
        "15".into(),
        "rm".into(),
        "-rf".into(),
        "--".into(),
        path.display().to_string(),
    ]);
    let status = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    if matches!(status, Ok(st) if st.success()) && !path.exists() {
        return Ok(());
    }
    match std::fs::remove_dir_all(path) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

/// Bytes the trash still holds, counted the cheap way (entries only; their
/// trees are being removed).
pub fn trash_entries(home: &Path) -> usize {
    std::fs::read_dir(trash_dir(home))
        .map(|d| d.flatten().count())
        .unwrap_or(0)
}
fn path_is_under(path: &Path, base: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let (path, base) = (canon(path), canon(base));
    path.starts_with(&base)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mkfile(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn empty_trash_later_drains_every_home_queued_while_a_pass_runs() {
        // Two homes discarded back to back (the test suite does this across
        // tempdirs) must both be emptied without a further call.
        let homes: Vec<_> = (0..4).map(|_| tempfile::tempdir().unwrap()).collect();
        for home in &homes {
            for n in 0..20 {
                mkfile(&trash_dir(home.path()).join(format!("c{n}/deep/file")), "x");
            }
        }
        for home in &homes {
            empty_trash_later(home.path());
        }
        for home in &homes {
            for _ in 0..400 {
                if trash_entries(home.path()) == 0 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            assert_eq!(trash_entries(home.path()), 0, "trash left in {home:?}");
        }
    }

    #[test]
    fn discard_renames_into_trash_and_empties_it() {
        let home = tempfile::tempdir().unwrap();
        let clone = home.path().join("work/job-20260101T000000Z-abcdef");
        mkfile(&clone.join("node_modules/a/index.js"), "x");
        mkfile(&clone.join("src/main.rs"), "fn main() {}");
        let got = discard_dir(home.path(), &clone).unwrap();
        let Discarded::Trashed(dest) = got else {
            panic!("expected a trashed folder, got {got:?}");
        };
        assert!(!clone.exists(), "clone still there");
        assert!(dest.starts_with(trash_dir(home.path())));
        // The background thread may have taken it already; emptying by hand is idempotent.
        let report = empty_trash(home.path());
        assert!(report.errors.is_empty(), "{report:?}");
        for _ in 0..200 {
            if trash_entries(home.path()) == 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert_eq!(trash_entries(home.path()), 0, "trash not emptied");
        assert!(!dest.exists());
        assert_eq!(
            discard_dir(home.path(), &clone).unwrap(),
            Discarded::Missing
        );
        let key = home.path().join("work/job.key");
        mkfile(&key, "k");
        assert_eq!(discard_dir(home.path(), &key).unwrap(), Discarded::Removed);
        assert!(!key.exists());
    }
}
