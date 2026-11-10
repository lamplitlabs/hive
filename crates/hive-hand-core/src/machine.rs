//! What a machine is and how its children run: the nice level and build
//! parallelism the daemon gives its job children, the hardware a hand
//! reports in its probe, and the labels a machine finds on itself. The brain
//! (hive-runner's daemon_load) and the hand (hive-hand) share these.

use std::sync::atomic::{AtomicI32, Ordering};

/// The nice level [`nice_child`] puts each job child at; 0 (nothing) until
/// the daemon sets one, so `hive work`, `hive-hand` and the other commands
/// run as before.
static CHILD_NICE_LEVEL: AtomicI32 = AtomicI32::new(0);

/// The environment variable Cargo reads its `build.jobs` from; every build a
/// child of the daemon runs (a worker's `cargo test`, the pre-commit
/// verification, the landing verification) gets it from [`build_env`].
pub const BUILD_JOBS_VAR: &str = "CARGO_BUILD_JOBS";

/// The parallelism [`build_env`] exports; 0 until the daemon sets it, so
/// `hive work` and the other commands leave the environment alone.
static BUILD_JOBS_LEVEL: AtomicI32 = AtomicI32::new(0);

/// The daemon sets the nice level its job children get; [`nice_child`]
/// applies to each child once spawned. Other commands leave it at 0.
pub fn set_child_nice(level: i32) {
    CHILD_NICE_LEVEL.store(level.clamp(0, 19), Ordering::SeqCst);
}

pub fn child_nice() -> i32 {
    CHILD_NICE_LEVEL.load(Ordering::SeqCst)
}

/// The daemon sets the compile parallelism its children's builds get;
/// [`build_env`] exports it. Re-set each pass so a changed
/// `daemon.build_jobs` applies without a restart; 0 exports nothing.
pub fn set_build_jobs(jobs: i32) {
    BUILD_JOBS_LEVEL.store(jobs.max(0), Ordering::SeqCst);
}

pub fn build_jobs() -> i32 {
    BUILD_JOBS_LEVEL.load(Ordering::SeqCst)
}

/// Adds `CARGO_BUILD_JOBS=<daemon.build_jobs>` to a child's environment
/// (`KEY=VALUE` lines) when the daemon set a level and the environment does
/// not carry the variable already: an owner who exports it, or a recipe
/// that sets it, wins. Before this every `cargo test` a worker ran and every
/// landing verification compiled on all the cores at once, four runs deep,
/// and the laptop ran hot for most of the day; Cargo reads this variable as
/// `build.jobs`, and a `-j` on the command line still overrides it.
pub fn build_env(env: &mut Vec<String>) {
    let jobs = build_jobs();
    if jobs <= 0 {
        return;
    }
    let prefix = format!("{BUILD_JOBS_VAR}=");
    if env.iter().any(|kv| kv.starts_with(&prefix)) {
        return;
    }
    env.push(format!("{prefix}{jobs}"));
}

/// Puts `child`, just spawned, at the nice level the daemon set (nothing
/// changes under `hive work` and the other commands, which set none): the
/// one line a spawn site adds after `spawn`, with `own_process_group` before
/// it. Done from the parent rather than a `pre_exec` hook so the spawn stays
/// a posix_spawn (a hook forks the whole process; `own_process_group` says
/// what that costs); the child runs at the parent's level for the moment
/// until the call lands. A child already gone, or one the parent may not
/// lower further (a process cannot raise its priority back), stays as it is.
pub fn nice_child(child: &std::process::Child) {
    renice_pid(child.id(), child_nice());
}

/// `setpriority` on one process (0: the calling one) with a bounded level;
/// a failure (the process gone, or already at a higher nice, which a process
/// cannot undo) changes nothing.
#[cfg(unix)]
pub fn renice_pid(pid: u32, level: i32) {
    if level > 0 {
        // SAFETY: one setpriority system call on a process id with a
        // bounded level.
        unsafe {
            libc::setpriority(libc::PRIO_PROCESS, pid as libc::id_t, level);
        }
    }
}

#[cfg(not(unix))]
pub fn renice_pid(_pid: u32, _level: i32) {}

/// The hardware a machine reports: logical CPUs, physical cores (the
/// logical count where the platform cannot tell) and memory in MB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hardware {
    pub cpus: i32,
    pub cores: i32,
    pub mem_mb: i64,
}

/// A fixed machine in place of the hardware reading: the brain's tests set
/// one so a hand's probe reports the same cores and memory on every test
/// host (hive-runner's `TEST_MACHINE`). Never set outside tests.
static HARDWARE_OVERRIDE: std::sync::RwLock<Option<Hardware>> = std::sync::RwLock::new(None);

/// Sets (or, with `None`, clears) the fixed machine [`Hardware::probed`]
/// reports in place of the hardware.
pub fn set_hardware_override(hw: Option<Hardware>) {
    *HARDWARE_OVERRIDE
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = hw;
}

impl Hardware {
    /// What a hand reports in its probe: the fixed machine a test set, else
    /// [`Hardware::read`].
    pub fn probed() -> Self {
        HARDWARE_OVERRIDE
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .unwrap_or_else(Self::read)
    }

    /// What the hardware says.
    pub fn read() -> Self {
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get() as i32)
            .unwrap_or(1)
            .max(1);
        let cores = physical_cores().unwrap_or(cpus).clamp(1, cpus.max(1));
        let mem_mb = total_mem_mb().unwrap_or(0).max(0);
        Self {
            cpus,
            cores,
            mem_mb,
        }
    }
}

#[cfg(target_os = "macos")]
pub fn sysctl_int(name: &str) -> Option<i64> {
    let cname = std::ffi::CString::new(name).ok()?;
    let mut buf = [0u8; 8];
    let mut len = buf.len();
    // SAFETY: the kernel writes at most `len` bytes into `buf` and updates `len`.
    let rc = unsafe {
        libc::sysctlbyname(
            cname.as_ptr(),
            buf.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return None;
    }
    match len {
        4 => Some(i64::from(i32::from_ne_bytes(buf[..4].try_into().ok()?))),
        8 => Some(i64::from_ne_bytes(buf)),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
fn physical_cores() -> Option<i32> {
    sysctl_int("hw.physicalcpu")
        .filter(|n| *n > 0)
        .map(|n| n.min(i64::from(i32::MAX)) as i32)
}

#[cfg(target_os = "macos")]
fn total_mem_mb() -> Option<i64> {
    sysctl_int("hw.memsize").map(|b| b / (1024 * 1024))
}

// libc points at the mach2 crate for this call; the workspace does not carry

#[cfg(target_os = "linux")]
pub fn meminfo_mb(key: &str) -> Option<i64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    meminfo_field_mb(&text, key)
}

/// One `/proc/meminfo` field in MB.
pub fn meminfo_field_mb(text: &str, key: &str) -> Option<i64> {
    text.lines().find_map(|line| {
        let (name, rest) = line.split_once(':')?;
        if name.trim() != key {
            return None;
        }
        let kb: i64 = rest.split_whitespace().next()?.parse().ok()?;
        Some(kb / 1024)
    })
}

#[cfg(target_os = "linux")]
fn physical_cores() -> Option<i32> {
    None
}

#[cfg(target_os = "linux")]
fn total_mem_mb() -> Option<i64> {
    meminfo_mb("MemTotal")
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn physical_cores() -> Option<i32> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn total_mem_mb() -> Option<i64> {
    None
}

/// The labels a machine finds on itself, so nobody types them: `docker`
/// when the docker client is on PATH, `xcode` when a full Xcode is the
/// selected developer directory (the command line tools alone build no app).
pub const FOUND_LABELS: [&str; 2] = ["docker", "xcode"];

/// What this machine finds on itself (FOUND_LABELS), looked up once.
pub fn found_labels() -> &'static [String] {
    static FOUND: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    FOUND.get_or_init(|| {
        if let Ok(fake) = std::env::var("HIVE_MACHINE_FOUND") {
            return crate::needs::stored(&fake);
        }
        let mut found = Vec::new();
        if which::which("docker").is_ok() {
            found.push("docker".to_string());
        }
        if crate::needs::this_os() == "macos"
            && std::process::Command::new("xcode-select")
                .arg("-p")
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .is_some_and(|p| p.contains("Xcode.app"))
        {
            found.push("xcode".to_string());
        }
        found
    })
}
