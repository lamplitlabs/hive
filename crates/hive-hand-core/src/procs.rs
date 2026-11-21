//! Process trees for job subprocesses: own process groups, bounded
//! termination (SIGTERM, a grace period, SIGKILL) and a straggler sweep that
//! kills whatever still lives under a job directory before the directory goes.
//!
//! Everything here is best effort and bounded in time; it never signals the
//! calling process, its parent, or anything outside the directory it is given.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// How long a terminated process group or straggler gets to exit on SIGTERM
/// before it is killed.
pub const DEFAULT_KILL_GRACE: Duration = Duration::from_secs(5);

/// How long the straggler sweep waits for SIGKILL to take after the grace.
const KILL_SETTLE: Duration = Duration::from_secs(1);

const POLL: Duration = Duration::from_millis(10);

/// A process the sweep or a group termination found and signalled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KilledProc {
    pub pid: i32,
    pub ppid: i32,
    pub pgid: i32,
    pub name: String,
    pub cwd: PathBuf,
    pub exe: PathBuf,
    /// SIGTERM was not enough within the grace period; SIGKILL followed.
    pub forced: bool,
}

impl std::fmt::Display for KilledProc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "pid {} ({}){}",
            self.pid,
            if self.name.is_empty() {
                "?"
            } else {
                self.name.as_str()
            },
            if self.forced { ", SIGKILL" } else { "" }
        )?;
        if !self.cwd.as_os_str().is_empty() {
            write!(f, " cwd {}", self.cwd.display())?;
        }
        Ok(())
    }
}

/// A snapshot of one process belonging to the current user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcInfo {
    pub pid: i32,
    pub ppid: i32,
    pub pgid: i32,
    pub name: String,
    pub cwd: PathBuf,
    pub exe: PathBuf,
}

impl ProcInfo {
    fn killed(&self, forced: bool) -> KilledProc {
        KilledProc {
            pid: self.pid,
            ppid: self.ppid,
            pgid: self.pgid,
            name: self.name.clone(),
            cwd: self.cwd.clone(),
            exe: self.exe.clone(),
            forced,
        }
    }
}

/// Makes `cmd` start as the leader of a new process group, so the whole tree
/// it spawns (including `cmd &`, `nohup` children and servers) can be
/// signalled with one `killpg`. Stdio handling is untouched.
///
/// Set as a spawn attribute, not a `pre_exec` hook: a hook makes std fork
/// this process and exec in the copy instead of posix_spawn - a page-table
/// copy of a hub holding gigabytes per shell command, every other thread
/// blocked on the allocator's fork lock meanwhile, and on macOS a
/// "MallocStackLogging: can't turn off malloc stack logging" line on the
/// child's stderr per spawn once the system has loaded the stack logger into
/// the process (which it does to a big long-lived one). See also
/// [`resolve_program`] for the other way a spawn ends up forking.
///
/// Off unix this does nothing yet: Windows needs CREATE_NEW_PROCESS_GROUP
/// plus a Job Object for the same guarantee (docs/windows-port.md).
pub fn own_process_group(cmd: &mut Command) {
    sys::own_process_group(cmd);
}

/// `program` as `Command::new` should get it: a name without a path
/// separator becomes the first executable of that name on PATH - the
/// child's PATH when `env` (`KEY=VALUE` lines) has one, else this process's -
/// and anything with a separator comes back as it is. std resolves a bare
/// name itself, but when the child's environment differs from the parent's
/// (`env_clear`, a PATH of its own) it has to do so after a fork rather than
/// a posix_spawn, with the cost [`own_process_group`] describes. A name no
/// PATH entry has comes back unchanged, so the spawn fails with the same
/// "not found" error it always did.
pub fn resolve_program(program: &str, env: &[String]) -> PathBuf {
    let bare = PathBuf::from(program);
    if program.is_empty() || program.contains(std::path::MAIN_SEPARATOR) || program.contains('/') {
        return bare;
    }
    let child_path = env
        .iter()
        .find_map(|kv| kv.strip_prefix("PATH=").map(std::ffi::OsString::from));
    let Some(path) = child_path.or_else(|| std::env::var_os("PATH")) else {
        return bare;
    };
    std::env::split_paths(&path)
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable_file(candidate))
        .unwrap_or(bare)
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

/// True while any process (zombies included) is left in the group.
pub fn group_alive(pgid: i32) -> bool {
    if pgid <= 1 {
        return false;
    }
    sys::group_alive(pgid)
}

fn process_alive(pid: i32) -> bool {
    if pid <= 1 {
        return false;
    }
    sys::process_alive(pid)
}

/// The two signals the termination ladder sends: a polite request first,
/// then the one that cannot be refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sig {
    Term,
    Kill,
}

fn signal_group(pgid: i32, sig: Sig) {
    if pgid > 1 {
        sys::signal_group(pgid, sig);
    }
}

fn signal_process(pid: i32, sig: Sig) {
    if pid > 1 && pid != std::process::id() as i32 {
        sys::signal_process(pid, sig);
    }
}

/// The unix process-group and signal calls behind the functions above.
#[cfg(unix)]
mod sys {
    use super::Sig;
    use nix::sys::signal::{Signal, kill, killpg};
    use nix::unistd::Pid;
    use std::process::Command;

    fn signal(sig: Sig) -> Signal {
        match sig {
            Sig::Term => Signal::SIGTERM,
            Sig::Kill => Signal::SIGKILL,
        }
    }

    pub(super) fn own_process_group(cmd: &mut Command) {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    pub(super) fn group_alive(pgid: i32) -> bool {
        match killpg(Pid::from_raw(pgid), None) {
            Ok(()) => true,
            Err(nix::errno::Errno::ESRCH) => false,
            // EPERM: something in the group exists but is not ours.
            Err(_) => true,
        }
    }

    pub(super) fn process_alive(pid: i32) -> bool {
        !matches!(
            kill(Pid::from_raw(pid), None),
            Err(nix::errno::Errno::ESRCH)
        )
    }

    pub(super) fn signal_group(pgid: i32, sig: Sig) {
        let _ = killpg(Pid::from_raw(pgid), signal(sig));
    }

    pub(super) fn signal_process(pid: i32, sig: Sig) {
        let _ = kill(Pid::from_raw(pid), signal(sig));
    }

    pub(super) fn parent_pid() -> i32 {
        // SAFETY: getppid has no preconditions and cannot fail.
        unsafe { nix::libc::getppid() }
    }
}

/// Off unix there are no process groups or signals to send yet: nothing is
/// grouped, no group or process reads as alive, and signals go nowhere, so
/// callers fall back to `Child::kill` on the direct child. The Windows port
/// replaces this with a Job Object per group (docs/windows-port.md).
#[cfg(not(unix))]
mod sys {
    use super::Sig;
    use std::process::Command;

    pub(super) fn own_process_group(_cmd: &mut Command) {}

    pub(super) fn group_alive(_pgid: i32) -> bool {
        false
    }

    pub(super) fn process_alive(_pid: i32) -> bool {
        false
    }

    pub(super) fn signal_group(_pgid: i32, _sig: Sig) {}

    pub(super) fn signal_process(_pid: i32, _sig: Sig) {}

    pub(super) fn parent_pid() -> i32 {
        0
    }
}

/// The process group a job subprocess was started in. `terminate` ends every
/// member; dropping an armed guard does the same, so a group never outlives
/// the step that created it.
#[derive(Debug)]
pub struct ProcessGroup {
    pgid: i32,
    grace: Duration,
    armed: bool,
}

impl ProcessGroup {
    pub fn new(pgid: i32) -> Self {
        Self::with_grace(pgid, DEFAULT_KILL_GRACE)
    }

    pub fn with_grace(pgid: i32, grace: Duration) -> Self {
        ProcessGroup {
            pgid,
            grace,
            armed: pgid > 1,
        }
    }

    pub fn pgid(&self) -> i32 {
        self.pgid
    }

    pub fn is_alive(&self) -> bool {
        group_alive(self.pgid)
    }

    /// Members of the group right now, as far as the process table tells.
    pub fn members(&self) -> Vec<ProcInfo> {
        if self.pgid <= 1 {
            return Vec::new();
        }
        list_processes()
            .into_iter()
            .filter(|p| p.pgid == self.pgid)
            .collect()
    }

    /// Lets the group live on; the guard then does nothing on drop.
    pub fn disarm(mut self) {
        self.armed = false;
    }

    /// SIGTERM, up to the grace period for the members to leave, then
    /// SIGKILL for whoever stayed. Returns the members that had to be
    /// signalled. `reap` is polled while waiting so a direct child that is
    /// already a zombie can be collected and does not hold the group open.
    pub fn terminate(&mut self) -> Vec<KilledProc> {
        self.terminate_reaping(|| {})
    }

    pub fn terminate_reaping(&mut self, mut reap: impl FnMut()) -> Vec<KilledProc> {
        self.armed = false;
        if self.pgid <= 1 {
            return Vec::new();
        }
        reap();
        if !group_alive(self.pgid) {
            return Vec::new();
        }
        let members = self.members();
        signal_group(self.pgid, Sig::Term);
        let deadline = Instant::now() + self.grace;
        while Instant::now() < deadline {
            reap();
            if !group_alive(self.pgid) {
                return members.into_iter().map(|p| p.killed(false)).collect();
            }
            std::thread::sleep(POLL);
        }
        let survivors: Vec<i32> = members
            .iter()
            .map(|p| p.pid)
            .filter(|pid| process_alive(*pid))
            .collect();
        signal_group(self.pgid, Sig::Kill);
        let deadline = Instant::now() + KILL_SETTLE;
        while Instant::now() < deadline {
            reap();
            if !group_alive(self.pgid) {
                break;
            }
            std::thread::sleep(POLL);
        }
        members
            .into_iter()
            .map(|p| {
                let forced = survivors.contains(&p.pid);
                p.killed(forced)
            })
            .collect()
    }
}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        if self.armed {
            self.terminate();
        }
    }
}

/// Directory shapes the sweep refuses, so a bad argument can never turn into
/// "kill everything": relative paths, `..`, the root, a top-level directory,
/// or the user's home.
pub fn sweepable_dir(dir: &Path) -> bool {
    if !dir.is_absolute() || dir.components().count() < 3 {
        return false;
    }
    if dir
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return false;
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        if !home.as_os_str().is_empty() && (dir == home || home.starts_with(dir)) {
            return false;
        }
    }
    ![
        "/",
        "/private",
        "/private/tmp",
        "/tmp",
        "/private/var",
        "/var",
        "/var/folders",
    ]
    .iter()
    .any(|p| Path::new(p) == dir)
}

fn dir_variants(dir: &Path) -> Vec<PathBuf> {
    let mut out = vec![dir.to_path_buf()];
    if let Ok(canon) = std::fs::canonicalize(dir)
        && !out.contains(&canon)
    {
        out.push(canon);
    }
    // macOS reports /var/... and /tmp/... as /private/var and /private/tmp.
    for v in out.clone() {
        if let Ok(rest) = v.strip_prefix("/private") {
            let alias = Path::new("/").join(rest);
            if !out.contains(&alias) {
                out.push(alias);
            }
        } else {
            let alias = Path::new("/private").join(v.strip_prefix("/").unwrap_or(&v));
            if !out.contains(&alias) {
                out.push(alias);
            }
        }
    }
    out
}

fn under_any(path: &Path, dirs: &[PathBuf]) -> bool {
    if path.as_os_str().is_empty() {
        return false;
    }
    dirs.iter().any(|d| path.starts_with(d))
}

/// The processes whose working directory or executable lives under `dir`,
/// never including the calling process or its parent.
pub fn processes_under(dir: &Path) -> Vec<ProcInfo> {
    if !sweepable_dir(dir) {
        return Vec::new();
    }
    let dirs = dir_variants(dir);
    let me = std::process::id() as i32;
    let parent = sys::parent_pid();
    list_processes()
        .into_iter()
        .filter(|p| p.pid > 1 && p.pid != me && p.pid != parent)
        .filter(|p| under_any(&p.cwd, &dirs) || under_any(&p.exe, &dirs))
        .collect()
}

/// Kills every process still working under `dir` (by working directory or
/// executable path): SIGTERM, `DEFAULT_KILL_GRACE`, then SIGKILL. Call it for
/// a job directory whose job has finished, right before the directory is
/// deleted, so background servers and watchers do not outlive their job.
/// Never touches the calling process, its parent, or anything outside `dir`;
/// refuses directories that are too broad to be a job directory.
pub fn kill_tree_under(dir: &Path) -> Vec<KilledProc> {
    kill_tree_under_with_grace(dir, DEFAULT_KILL_GRACE)
}

pub fn kill_tree_under_with_grace(dir: &Path, grace: Duration) -> Vec<KilledProc> {
    kill_tree_under_sparing(dir, grace, |_| false)
}

/// [`kill_tree_under_with_grace`] that leaves alone every process `spare`
/// says yes to: the harness's own managed background processes work under
/// the clone too and must survive the sweep after each command.
pub fn kill_tree_under_sparing(
    dir: &Path,
    grace: Duration,
    spare: impl Fn(&ProcInfo) -> bool,
) -> Vec<KilledProc> {
    let found: Vec<ProcInfo> = processes_under(dir)
        .into_iter()
        .filter(|p| !spare(p))
        .collect();
    if found.is_empty() {
        return Vec::new();
    }
    for p in &found {
        signal_process(p.pid, Sig::Term);
    }
    let deadline = Instant::now() + grace;
    let mut alive: Vec<i32> = found.iter().map(|p| p.pid).collect();
    while Instant::now() < deadline {
        alive.retain(|pid| process_alive(*pid));
        if alive.is_empty() {
            break;
        }
        std::thread::sleep(POLL);
    }
    alive.retain(|pid| process_alive(*pid));
    for pid in &alive {
        signal_process(*pid, Sig::Kill);
    }
    if !alive.is_empty() {
        let deadline = Instant::now() + KILL_SETTLE;
        while Instant::now() < deadline && alive.iter().any(|pid| process_alive(*pid)) {
            std::thread::sleep(POLL);
        }
    }
    found
        .into_iter()
        .map(|p| {
            let forced = alive.contains(&p.pid);
            p.killed(forced)
        })
        .collect()
}

/// SIGKILL for every member of the group at once, with no grace period and
/// no wait: for a group whose leader has already exited or is being killed
/// anyway (a measured command's leftovers). Off unix it does nothing yet,
/// like the rest of the group calls.
pub fn kill_group(pgid: i32) {
    signal_group(pgid, Sig::Kill);
}

/// One line per killed process, for logs and observations.
pub fn describe_killed(killed: &[KilledProc]) -> String {
    killed
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// Every process of the current user the platform lets us inspect, with its
/// working directory and executable. Processes we may not inspect (other
/// users, zombies, kernel tasks) are left out.
pub fn list_processes() -> Vec<ProcInfo> {
    platform::list_processes()
}

#[cfg(target_os = "macos")]
mod platform {
    use super::ProcInfo;
    use nix::libc;
    use std::path::PathBuf;

    fn c_bytes_to_path(bytes: &[u8]) -> PathBuf {
        let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
        use std::os::unix::ffi::OsStrExt;
        PathBuf::from(std::ffi::OsStr::from_bytes(&bytes[..end]))
    }

    fn c_chars_to_string(chars: &[libc::c_char]) -> String {
        let bytes: Vec<u8> = chars
            .iter()
            .take_while(|c| **c != 0)
            .map(|c| *c as u8)
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    pub(super) fn list_pids() -> Vec<i32> {
        // libproc's proc_listallpids returns a count of pids (the kernel's
        // byte count divided by sizeof(int)), both when asked for the size
        // and when filling the buffer; over-allocating is harmless.
        let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
        if count <= 0 {
            return Vec::new();
        }
        let cap = count as usize + 64;
        let mut pids: Vec<libc::pid_t> = vec![0; cap];
        let got = unsafe {
            libc::proc_listallpids(
                pids.as_mut_ptr() as *mut libc::c_void,
                (cap * std::mem::size_of::<libc::pid_t>()) as libc::c_int,
            )
        };
        if got <= 0 {
            return Vec::new();
        }
        pids.truncate((got as usize).min(cap));
        pids.retain(|p| *p > 0);
        pids
    }

    pub(super) fn info(pid: i32) -> Option<ProcInfo> {
        let mut bsd: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        let got = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                &mut bsd as *mut _ as *mut libc::c_void,
                size,
            )
        };
        if got != size {
            return None;
        }
        let mut vpi: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
        let got = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDVNODEPATHINFO,
                0,
                &mut vpi as *mut _ as *mut libc::c_void,
                size,
            )
        };
        let cwd = if got == size {
            let raw = unsafe {
                std::slice::from_raw_parts(
                    vpi.pvi_cdir.vip_path.as_ptr() as *const u8,
                    std::mem::size_of_val(&vpi.pvi_cdir.vip_path),
                )
            };
            c_bytes_to_path(raw)
        } else {
            PathBuf::new()
        };
        let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        let len = unsafe {
            libc::proc_pidpath(pid, buf.as_mut_ptr() as *mut libc::c_void, buf.len() as u32)
        };
        let exe = if len > 0 {
            c_bytes_to_path(&buf[..len as usize])
        } else {
            PathBuf::new()
        };
        let mut name = c_chars_to_string(&bsd.pbi_name);
        if name.is_empty() {
            name = c_chars_to_string(&bsd.pbi_comm);
        }
        Some(ProcInfo {
            pid,
            ppid: bsd.pbi_ppid as i32,
            pgid: bsd.pbi_pgid as i32,
            name,
            cwd,
            exe,
        })
    }

    pub(super) fn list_processes() -> Vec<ProcInfo> {
        list_pids().into_iter().filter_map(info).collect()
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::ProcInfo;
    use std::path::{Path, PathBuf};

    fn link(path: &Path) -> PathBuf {
        match std::fs::read_link(path) {
            Ok(p) => {
                let s = p.to_string_lossy();
                match s.strip_suffix(" (deleted)") {
                    Some(stripped) => PathBuf::from(stripped),
                    None => p,
                }
            }
            Err(_) => PathBuf::new(),
        }
    }

    pub(super) fn info(pid: i32) -> Option<ProcInfo> {
        let proc_dir = Path::new("/proc").join(pid.to_string());
        let stat = std::fs::read_to_string(proc_dir.join("stat")).ok()?;
        // "pid (comm) state ppid pgrp ..." - comm may contain spaces and parens.
        let close = stat.rfind(')')?;
        let open = stat.find('(')?;
        let name = stat.get(open + 1..close)?.to_string();
        let mut rest = stat.get(close + 1..)?.split_whitespace();
        let _state = rest.next()?;
        let ppid: i32 = rest.next()?.parse().ok()?;
        let pgid: i32 = rest.next()?.parse().ok()?;
        Some(ProcInfo {
            pid,
            ppid,
            pgid,
            name,
            cwd: link(&proc_dir.join("cwd")),
            exe: link(&proc_dir.join("exe")),
        })
    }

    pub(super) fn list_processes() -> Vec<ProcInfo> {
        let Ok(rd) = std::fs::read_dir("/proc") else {
            return Vec::new();
        };
        rd.filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_string_lossy().parse::<i32>().ok())
            .filter(|pid| *pid > 0)
            .filter_map(info)
            .collect()
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod platform {
    pub(super) fn list_processes() -> Vec<super::ProcInfo> {
        Vec::new()
    }
}

// The tests drive /bin/sh, `ps` and real process groups: unix only until the
// Windows port gives them Job Object counterparts.
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use nix::libc;
    use std::process::Stdio;

    fn spawn_sh(dir: &Path, script: &str) -> std::process::Child {
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c")
            .arg(script)
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        own_process_group(&mut cmd);
        cmd.spawn().unwrap()
    }

    fn wait_for(mut cond: impl FnMut() -> bool, what: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !cond() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(POLL);
        }
    }

    #[test]
    fn own_process_group_makes_the_child_a_group_leader() {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "ps -o pgid= -p $$"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        own_process_group(&mut cmd);
        let child = cmd.spawn().unwrap();
        let pid = child.id();
        let out = child.wait_with_output().unwrap();
        let pgid: u32 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
        assert_eq!(pgid, pid, "the child leads its own group");
    }

    #[test]
    fn resolve_program_follows_the_child_path_and_keeps_paths_as_they_are() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let tool = bin.join("hive-test-tool");
        std::fs::write(&tool, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let child_env = vec![format!("PATH={}", bin.display()), "HOME=/nowhere".into()];
        assert_eq!(resolve_program("hive-test-tool", &child_env), tool);
        assert_eq!(
            resolve_program("hive-test-tool", &[]),
            PathBuf::from("hive-test-tool"),
            "not on this process's PATH: the bare name, so the spawn's error names it"
        );
        assert_eq!(
            resolve_program("hive-missing-tool", &child_env),
            PathBuf::from("hive-missing-tool")
        );
        let explicit = tool.display().to_string();
        assert_eq!(resolve_program(&explicit, &child_env), tool);
        assert_eq!(
            resolve_program("./relative/tool", &child_env),
            PathBuf::from("./relative/tool")
        );
        assert_eq!(resolve_program("", &child_env), PathBuf::from(""));
        let sh = resolve_program("sh", &["PATH=/usr/bin:/bin".into()]);
        assert!(sh.is_absolute() && sh.ends_with("sh"), "{sh:?}");
    }

    #[test]
    fn lists_this_process_with_its_cwd() {
        let me = std::process::id() as i32;
        let procs = list_processes();
        let mine = procs
            .iter()
            .find(|p| p.pid == me)
            .unwrap_or_else(|| panic!("own pid {me} missing from {} entries", procs.len()));
        let cwd = std::env::current_dir().unwrap();
        let canon = std::fs::canonicalize(&cwd).unwrap();
        assert!(
            mine.cwd == cwd || mine.cwd == canon,
            "cwd {:?} != {:?}",
            mine.cwd,
            cwd
        );
        assert!(!mine.exe.as_os_str().is_empty());
        assert!(mine.pgid > 0 && mine.ppid > 0);
    }

    #[test]
    fn listing_is_cheap_enough_for_every_step() {
        let start = Instant::now();
        for _ in 0..5 {
            let _ = list_processes();
        }
        let per = start.elapsed() / 5;
        let seen = list_processes().len();
        eprintln!("process table listing: {per:?} ({seen} processes)");
        assert!(
            per < Duration::from_millis(250),
            "listing the process table took {per:?}"
        );
        // Everything of ours that `ps` shows must be in the listing, or the
        // sweep would miss stragglers (an earlier cut listed a quarter).
        // "Shows" means before the listing and after it: a process that
        // ended in between (this `ps` itself, a neighbour test's `sh`) is not
        // one the listing missed, and neither is one that has exited and
        // waits to be reaped. On a machine with few processes of ours, such
        // as a fresh VM, two of those were already more than the tenth this
        // test allows.
        let uid = unsafe { libc::getuid() };
        let ours_now = || -> std::collections::HashSet<i32> {
            let ps = Command::new("ps")
                .args(["-axo", "pid=,uid=,state="])
                .output()
                .unwrap();
            String::from_utf8_lossy(&ps.stdout)
                .lines()
                .filter_map(|l| {
                    let mut it = l.split_whitespace();
                    let pid: i32 = it.next()?.parse().ok()?;
                    let owner: u32 = it.next()?.parse().ok()?;
                    let exited = it.next().is_some_and(|state| state.starts_with('Z'));
                    (owner == uid && pid > 1 && !exited).then_some(pid)
                })
                .collect()
        };
        let before = ours_now();
        let listed: std::collections::HashSet<i32> =
            list_processes().into_iter().map(|p| p.pid).collect();
        let after = ours_now();
        let ours: Vec<i32> = before.intersection(&after).copied().collect();
        let missing = ours.iter().filter(|p| !listed.contains(p)).count();
        assert!(
            ours.len() >= 2 && missing * 10 <= ours.len(),
            "process listing misses {missing} of our {} processes",
            ours.len()
        );
    }

    #[test]
    fn terminate_takes_background_children_with_the_shell() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("started");
        let mut child = spawn_sh(
            dir.path(),
            &format!(
                "sleep 300 & (sleep 300 &) ; echo $! > {}; exit 0",
                marker.display()
            ),
        );
        let pgid = child.id() as i32;
        let status = child.wait().unwrap();
        assert!(status.success());
        wait_for(|| marker.exists(), "marker");
        let mut group = ProcessGroup::with_grace(pgid, Duration::from_secs(2));
        assert!(
            group.is_alive(),
            "sleep children should keep the group alive"
        );
        let members = group.members();
        assert!(
            members.iter().all(|p| p.pgid == pgid) && !members.is_empty(),
            "members {members:?}"
        );
        let start = Instant::now();
        let killed = group.terminate();
        assert!(!killed.is_empty(), "nothing reported killed");
        assert!(
            killed.iter().all(|k| !k.forced),
            "sleep ignored SIGTERM: {killed:?}"
        );
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "{:?}",
            start.elapsed()
        );
        assert!(!group_alive(pgid), "survivors in group {pgid}");
    }

    #[test]
    fn terminate_escalates_to_sigkill() {
        let dir = tempfile::tempdir().unwrap();
        let armed = dir.path().join("armed");
        let mut child = spawn_sh(
            dir.path(),
            &format!("trap '' TERM; echo > {}; sleep 300", armed.display()),
        );
        let pgid = child.id() as i32;
        wait_for(|| armed.exists(), "trap to be installed");
        let mut group = ProcessGroup::with_grace(pgid, Duration::from_millis(300));
        let killed = group.terminate_reaping(|| {
            let _ = child.try_wait();
        });
        assert!(
            killed.iter().any(|k| k.forced),
            "shell ignoring TERM must be SIGKILLed: {killed:?}"
        );
        wait_for(|| !group_alive(pgid), "group to die");
    }

    #[test]
    fn dropping_an_armed_group_kills_it_and_disarm_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = spawn_sh(dir.path(), "sleep 300");
        let pgid = child.id() as i32;
        {
            // Without a reaper the zombie child holds the group until the
            // grace runs out; keep it short.
            let _group = ProcessGroup::with_grace(pgid, Duration::from_millis(100));
        }
        let status = child.wait().unwrap();
        assert!(!status.success());
        assert!(!group_alive(pgid));

        let mut child = spawn_sh(dir.path(), "sleep 300");
        let pgid = child.id() as i32;
        ProcessGroup::new(pgid).disarm();
        assert!(
            child.try_wait().unwrap().is_none(),
            "disarmed group was killed"
        );
        let mut group = ProcessGroup::with_grace(pgid, Duration::from_secs(2));
        group.terminate_reaping(|| {
            let _ = child.try_wait();
        });
        let _ = child.wait();
    }

    #[test]
    fn kill_tree_under_only_touches_the_directory() {
        let inside = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let mut escaped = spawn_sh(inside.path(), "sleep 300");
        let mut bystander = spawn_sh(outside.path(), "sleep 300");
        let in_pid = escaped.id() as i32;
        let out_pid = bystander.id() as i32;
        wait_for(
            || {
                processes_under(inside.path())
                    .iter()
                    .any(|p| p.pid == in_pid)
            },
            "sleep to show up under the directory",
        );
        let killed = kill_tree_under_with_grace(inside.path(), Duration::from_secs(2));
        assert!(
            killed.iter().any(|k| k.pid == in_pid),
            "process in dir not killed: {killed:?}"
        );
        assert!(
            killed.iter().all(|k| k.pid != out_pid),
            "bystander killed: {killed:?}"
        );
        assert!(!escaped.wait().unwrap().success());
        std::thread::sleep(Duration::from_millis(100));
        assert!(
            bystander.try_wait().unwrap().is_none(),
            "bystander outside the directory died"
        );
        let mut group = ProcessGroup::with_grace(out_pid, Duration::from_secs(2));
        group.terminate_reaping(|| {
            let _ = bystander.try_wait();
        });
        let _ = bystander.wait();
        assert!(kill_tree_under(inside.path()).is_empty());
    }

    #[test]
    fn sweep_refuses_broad_directories() {
        for bad in [
            "/",
            "/Users",
            "/home",
            "/tmp",
            "/private/tmp",
            "relative/dir",
            "/a/b/../c",
        ] {
            assert!(!sweepable_dir(Path::new(bad)), "{bad} should be refused");
            assert!(kill_tree_under(Path::new(bad)).is_empty());
        }
        if let Some(home) = std::env::var_os("HOME") {
            assert!(!sweepable_dir(Path::new(&home)));
        }
        assert!(sweepable_dir(Path::new("/Users/me/.hive/work/job-1")));
        assert!(sweepable_dir(Path::new("/var/folders/ab/T/hive-x/clone")));
    }

    #[test]
    fn directory_variants_cover_the_private_alias() {
        let vs = dir_variants(Path::new("/var/folders/x/T/job"));
        assert!(vs.contains(&PathBuf::from("/private/var/folders/x/T/job")));
        let vs = dir_variants(Path::new("/private/tmp/job"));
        assert!(vs.contains(&PathBuf::from("/tmp/job")));
        assert!(under_any(
            Path::new("/private/tmp/job/clone/src"),
            &dir_variants(Path::new("/tmp/job"))
        ));
        assert!(!under_any(
            Path::new("/tmp/jobs"),
            &dir_variants(Path::new("/tmp/job"))
        ));
    }

    #[test]
    fn group_alive_is_false_for_empty_groups() {
        assert!(!group_alive(0));
        assert!(!group_alive(-5));
        let dir = tempfile::tempdir().unwrap();
        let mut child = spawn_sh(dir.path(), "exit 0");
        let pgid = child.id() as i32;
        child.wait().unwrap();
        assert!(!group_alive(pgid));
    }
}
