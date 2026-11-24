//! The worker's temp dir and the room it needs before a run starts: a
//! worker that fills its TMPDIR mid-run loses its work, so the run refuses
//! to start below [`MIN_TMP_SPACE`].

use anyhow::Result;
use std::sync::{LazyLock, RwLock};

pub const MIN_TMP_SPACE: u64 = 512 << 20;
pub type FreeSpaceFn = fn(&str) -> Result<u64>;
pub static FREE_SPACE: LazyLock<RwLock<FreeSpaceFn>> =
    LazyLock::new(|| RwLock::new(real_free_space));
pub fn worker_tmp_dir(env: &[String]) -> String {
    env.iter()
        .rev()
        .find_map(|kv| kv.strip_prefix("TMPDIR=").map(ToOwned::to_owned))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| std::env::temp_dir().display().to_string())
}
pub fn free_space(dir: &str) -> Result<u64> {
    (*FREE_SPACE.read().unwrap())(dir)
}
// statvfs counts blocks in a u32 on macOS and a u64 on Linux, so the widening
// is needed on one and a no-op clippy flags on the other.
#[cfg(unix)]
#[allow(clippy::useless_conversion)]
pub fn real_free_space(dir: &str) -> Result<u64> {
    let st = nix::sys::statvfs::statvfs(std::path::Path::new(dir))?;
    Ok(u64::from(st.blocks_available()) * st.fragment_size())
}

/// Off unix the free space is not read yet, so it never holds a run back.
/// The Windows port reads it with `GetDiskFreeSpaceExW`
/// (docs/windows-port.md).
#[cfg(not(unix))]
pub fn real_free_space(_dir: &str) -> Result<u64> {
    Ok(u64::MAX)
}
pub fn preflight_tmp_space(dir: &str, min: u64) -> Result<()> {
    let free = free_space(dir).map_err(|e| crate::error::errorf(1, &format!("setup-error: worker TMPDIR {dir} is unusable ({e}); the worker was not started so no work is lost"), "", ""))?;
    if free < min {
        return Err(crate::error::errorf(1, &format!("setup-error: worker TMPDIR {dir} has {} free, below the {} the worker needs; the worker was not started so no work is lost", human_bytes(free), human_bytes(min)), "", "").into());
    }
    Ok(())
}
pub fn human_bytes(n: u64) -> String {
    if n >= 1 << 30 {
        format!("{:.1} GiB", n as f64 / (1u64 << 30) as f64)
    } else if n >= 1 << 20 {
        format!("{:.1} MiB", n as f64 / (1u64 << 20) as f64)
    } else {
        format!("{n} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    fn free_10m(_: &str) -> Result<u64> {
        Ok(10 << 20)
    }
    fn free_err(_: &str) -> Result<u64> {
        Err(anyhow!("no such file"))
    }
    fn free_min(_: &str) -> Result<u64> {
        Ok(MIN_TMP_SPACE)
    }

    /// The tests swap the process-wide `FREE_SPACE` seam; one at a time.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
        SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn preflight_tmp_space_full_disk_is_setup_error() {
        let _serial = serial();
        let _guard = hive_testutil::env_lock();
        let orig = *FREE_SPACE.read().unwrap();
        *FREE_SPACE.write().unwrap() = free_10m;
        let err = preflight_tmp_space("/scratch", MIN_TMP_SPACE)
            .unwrap_err()
            .to_string();
        for want in ["setup-error", "/scratch", "10.0 MiB", "512.0 MiB"] {
            assert!(err.contains(want), "{err:?} should contain {want:?}");
        }
        *FREE_SPACE.write().unwrap() = free_err;
        let err = preflight_tmp_space("/gone", MIN_TMP_SPACE)
            .unwrap_err()
            .to_string();
        assert!(err.contains("unusable"), "{err}");
        *FREE_SPACE.write().unwrap() = free_min;
        preflight_tmp_space("/ok", MIN_TMP_SPACE).unwrap();
        *FREE_SPACE.write().unwrap() = orig;
    }

    #[test]
    fn preflight_tmp_space_uses_worker_tmpdir() {
        let _serial = serial();
        assert_eq!(
            worker_tmp_dir(&["PATH=/bin".into(), "TMPDIR=/w/tmp".into()]),
            "/w/tmp"
        );
        assert!(!worker_tmp_dir(&["PATH=/bin".into()]).is_empty());
        preflight_tmp_space(
            &tempfile::tempdir().unwrap().path().display().to_string(),
            1,
        )
        .unwrap();
    }
}
