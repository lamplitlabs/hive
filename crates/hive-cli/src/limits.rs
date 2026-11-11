//! The process's ceiling on open files.
//!
//! launchd starts an agent with 256 open files allowed (the hard limit is
//! unlimited). A hub holds, for every open conversation, a provider
//! connection and one connection or child process per MCP server, so a
//! handful of conversations reach 256 - after which the hub cannot even
//! spawn `git`, and every project reads as missing from disk. The Go
//! runtime raised the soft limit to the hard one at start by itself; the
//! Rust binary does it here, once, before any command runs.

/// The open-file limit before and after [`raise_open_files`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenFiles {
    pub before: u64,
    pub after: u64,
}

/// Raises the soft open-file limit to the hard one - on macOS to
/// `kern.maxfilesperproc` when the hard limit is unlimited, since
/// `setrlimit` refuses more than that. Some(limits) when it raised the
/// limit; None when it was already as high as it goes or the system
/// refused.
#[cfg(unix)]
#[allow(clippy::unnecessary_cast)] // rlim_t is u64 here, not on every platform
pub fn raise_open_files() -> Option<OpenFiles> {
    use nix::libc::{RLIMIT_NOFILE, getrlimit, rlimit, setrlimit};
    let mut lim = rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit fills the struct it is handed and nothing else.
    if unsafe { getrlimit(RLIMIT_NOFILE, &mut lim) } != 0 {
        return None;
    }
    let before = lim.rlim_cur as u64;
    let after = raised(before, lim.rlim_max as u64, per_process_cap());
    if after <= before {
        return None;
    }
    let next = rlimit {
        rlim_cur: after as nix::libc::rlim_t,
        rlim_max: lim.rlim_max,
    };
    // SAFETY: setrlimit reads the struct it is handed.
    if unsafe { setrlimit(RLIMIT_NOFILE, &next) } != 0 {
        return None;
    }
    Some(OpenFiles { before, after })
}

#[cfg(not(unix))]
pub fn raise_open_files() -> Option<OpenFiles> {
    None
}

/// How many files this process may have open right now (the soft limit),
/// for the hub's startup line; None when the system cannot say.
#[cfg(unix)]
#[allow(clippy::unnecessary_cast)] // rlim_t is u64 here, not on every platform
pub fn open_files_allowed() -> Option<u64> {
    use nix::libc::{RLIMIT_NOFILE, getrlimit, rlimit};
    let mut lim = rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit fills the struct it is handed and nothing else.
    if unsafe { getrlimit(RLIMIT_NOFILE, &mut lim) } != 0 {
        return None;
    }
    Some(lim.rlim_cur as u64)
}

#[cfg(not(unix))]
pub fn open_files_allowed() -> Option<u64> {
    None
}

/// Fewer open files than this and a hub is going to run short: the
/// startup line says so, with the number.
pub const OPEN_FILES_SHORT: u64 = 1024;

/// The soft limit to ask for: the hard limit, held under the system's
/// per-process cap when it names one (an unlimited hard limit means "as
/// many as the kernel allows", and the kernel says how many that is), and
/// never below what the process already has.
#[cfg_attr(not(unix), allow(dead_code))]
fn raised(soft: u64, hard: u64, cap: Option<u64>) -> u64 {
    let want = match cap {
        Some(cap) => hard.min(cap),
        None => hard,
    };
    want.max(soft)
}

/// macOS's `OPEN_MAX`, the per-process cap `setrlimit` falls back to when
/// the kernel does not say otherwise.
#[cfg(target_os = "macos")]
const OPEN_MAX: u64 = 10_240;

/// macOS: `kern.maxfilesperproc`, the most any one process may open;
/// [`OPEN_MAX`] when the kernel does not answer.
#[cfg(target_os = "macos")]
fn per_process_cap() -> Option<u64> {
    use nix::libc::{c_int, c_void, sysctlbyname};
    let mut value: c_int = 0;
    let mut len = std::mem::size_of::<c_int>();
    // SAFETY: sysctlbyname writes at most `len` bytes into `value`, a
    // c_int of exactly that size, and is given no new value to set.
    let rc = unsafe {
        sysctlbyname(
            c"kern.maxfilesperproc".as_ptr(),
            (&raw mut value).cast::<c_void>(),
            &raw mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    match u64::try_from(value) {
        Ok(cap) if rc == 0 && cap > 0 => Some(cap),
        _ => Some(OPEN_MAX),
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn per_process_cap() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raised_goes_to_the_hard_limit_under_the_kernel_cap() {
        // launchd's agent: 256 soft, unlimited hard, the kernel allows 61440.
        assert_eq!(raised(256, u64::MAX, Some(61_440)), 61_440);
        // A shell with a finite hard limit and no cap to speak of.
        assert_eq!(raised(1024, 1_048_576, None), 1_048_576);
        // A hard limit under the cap is the ceiling.
        assert_eq!(raised(256, 4096, Some(61_440)), 4096);
        // Already at the hard limit: nothing to raise.
        assert_eq!(raised(4096, 4096, Some(61_440)), 4096);
        // Never below what the process already has.
        assert_eq!(raised(8192, 8192, Some(1024)), 8192);
    }

    #[cfg(unix)]
    #[test]
    fn the_process_ends_up_at_its_ceiling() {
        let first = raise_open_files();
        let now = open_files_allowed().expect("getrlimit answers");
        if let Some(r) = first {
            assert_eq!(r.after, now);
            assert!(r.after > r.before, "{r:?}");
        }
        // A second call finds nothing left to raise.
        assert_eq!(raise_open_files(), None);
        assert_eq!(open_files_allowed(), Some(now));
    }
}
