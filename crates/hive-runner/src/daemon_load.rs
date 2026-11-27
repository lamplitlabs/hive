//! The load governor: how many runs this machine takes at once and when it
//! leases nothing new because the machine is saturated.
//!
//! The local slots come from the capacity model ([`crate::capacity`]): the
//! smaller of the physical cores and one run per 3 GB of RAM, or the
//! `capacity` override. The governor holds every local slot while the
//! one-minute load average is over [`DEFAULT_LOAD_PER_CPU`] times the CPUs
//! or the memory a new run could use is under [`MIN_FREE_MEM_MB`], halves
//! the runs it starts while the kernel's memory warning stands, and runs
//! the daemon's children at nice [`CHILD_NICE`] so the owner's own work
//! stays responsive. Remote executors size themselves: their load is not
//! this machine's.
//!
//! [`decide`] is the pure decision the loop and the self-update build use;
//! [`Machine::detect`] and [`pressure`] are the only parts that touch the
//! hardware, and each falls back to "no pressure" where the platform has
//! no reading.

use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(test)]
use hive_hand_core::machine::meminfo_field_mb;
#[cfg(target_os = "linux")]
use hive_hand_core::machine::meminfo_mb;
#[cfg(target_os = "macos")]
use hive_hand_core::machine::sysctl_int;
#[allow(unused_imports)]
pub(crate) use hive_hand_core::machine::{
    BUILD_JOBS_VAR, build_env, build_jobs, child_nice, nice_child, renice_pid, set_build_jobs,
    set_child_nice,
};

/// Nice level the daemon's job children run at (`setpriority`); children
/// of children inherit it, so a worker's `npm ci` and `cargo test` yield
/// to the owner's foreground work and to the daemon's own loop.
pub(crate) const CHILD_NICE: i32 = 10;
/// The highest nice level `setpriority` accepts; `daemon.nice` is clamped to it.
pub(crate) const MAX_NICE: i32 = 19;
/// Megabytes of usable memory under which nothing new starts locally.
pub(crate) const MIN_FREE_MEM_MB: i64 = 1024;
/// The load line, in load per CPU. macOS counts every
/// runnable thread in the load average, and a laptop running an emulator
/// or a browser build sits at twice its CPU count for hours while its CPUs
/// are far from busy; a line at the CPU count held the fleet all day.
pub(crate) const DEFAULT_LOAD_PER_CPU: f64 = 2.0;

/// The machine every test runs on (four local runs by default).
#[cfg(test)]
pub(crate) const TEST_MACHINE: Machine = Machine {
    cpus: 12,
    cores: 12,
    mem_mb: 12288,
};

/// The hands probe reports [`TEST_MACHINE`] under test, as
/// [`Machine::detect`] does: the runner's test setup installs it.
#[cfg(test)]
pub(crate) fn install_test_hardware() {
    hive_hand_core::machine::set_hardware_override(Some(hive_hand_core::machine::Hardware {
        cpus: TEST_MACHINE.cpus,
        cores: TEST_MACHINE.cores,
        mem_mb: TEST_MACHINE.mem_mb,
    }));
}

/// What this machine has; read once per process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Machine {
    /// Logical CPUs: the load average counts runnable threads against these.
    pub(crate) cpus: i32,
    /// Physical cores (the logical count where the platform cannot tell).
    pub(crate) cores: i32,
    pub(crate) mem_mb: i64,
}

impl Machine {
    /// The machine the daemon runs on. Under test a fixed 12-core, 16 GB
    /// machine, so a dispatch test leases the same on every test host;
    /// [`Machine::read`] is the hardware reading itself.
    pub(crate) fn detect() -> Self {
        #[cfg(test)]
        {
            TEST_MACHINE
        }
        #[cfg(not(test))]
        {
            Self::read()
        }
    }

    /// What the hardware says.
    pub(crate) fn read() -> Self {
        let hive_hand_core::machine::Hardware {
            cpus,
            cores,
            mem_mb,
        } = hive_hand_core::machine::Hardware::read();
        Self {
            cpus,
            cores,
            mem_mb,
        }
    }

    /// The runs this machine takes from its hardware ([`crate::capacity::machine_slots`]).
    pub(crate) fn default_max_jobs(&self) -> i32 {
        crate::capacity::machine_slots(self.cores, self.mem_mb)
    }

    /// Parallelism for a build the daemon runs for itself (`cargo -j`) and
    /// the default `daemon.build_jobs`: half the cores, so no single build
    /// takes the whole machine from the owner or the other runs.
    pub(crate) fn build_jobs(&self) -> i32 {
        (self.cores / 2).max(1)
    }
}

/// A reading of the machine's pressure now.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Pressure {
    /// The one-minute load average; None where the platform has none.
    pub(crate) load1: Option<f64>,
    /// Megabytes a new run could use without swapping: free, inactive,
    /// speculative and purgeable pages on macOS, `MemAvailable` on Linux;
    /// None elsewhere.
    pub(crate) free_mem_mb: Option<i64>,
    /// The kernel itself reports memory pressure (macOS
    /// `kern.memorystatus_vm_pressure_level` at warning or critical). The
    /// warning alone is a soft signal: it halves the runs that start.
    pub(crate) mem_warning: bool,
    /// The kernel reports critical memory pressure (the level at 4): the
    /// machine is swapping and one more run would only finish later.
    pub(crate) mem_critical: bool,
}

/// The limits in force.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Limits {
    /// Local runs at once.
    pub(crate) max_jobs: i32,
    /// One-minute load average over which nothing new starts.
    pub(crate) max_load: f64,
    /// Logical CPUs the load is read against, for the hold line.
    pub(crate) cpus: i32,
    /// Megabytes of usable memory under which nothing new starts; 0 off.
    pub(crate) min_free_mem_mb: i64,
    /// Nice level the daemon's own process runs at from startup
    /// (`daemon.nice`); 0 leaves the level the service started it at.
    pub(crate) own_nice: i32,
    /// Compile jobs one build may use (`daemon.build_jobs`, else half the
    /// cores): the `CARGO_BUILD_JOBS` the children get and the `-j` of the
    /// self-update build.
    pub(crate) build_jobs: i32,
    /// Where `max_jobs` came from, for the startup line.
    pub(crate) source: &'static str,
}

/// The limits from the flag, the `capacity` override and the machine: a
/// positive `--max-jobs` wins, then a numeric `capacity`, then the
/// machine's own slots; the load line is [`DEFAULT_LOAD_PER_CPU`] times the
/// CPUs and the memory floor [`MIN_FREE_MEM_MB`]; `daemon.build_jobs` 0 is
/// half the cores.
pub(crate) fn limits(s: &hive_store::Store, machine: &Machine, flag_max_jobs: i32) -> Limits {
    let (max_jobs, source) = if flag_max_jobs > 0 {
        (flag_max_jobs, "--max-jobs")
    } else if let Some(n) = crate::capacity::override_slots(s) {
        (n, "capacity")
    } else {
        (machine.default_max_jobs(), "auto")
    };
    let cpus = machine.cpus.max(1);
    let max_load = f64::from(cpus) * DEFAULT_LOAD_PER_CPU;
    let build_jobs = match s.setting_int("daemon.build_jobs") {
        n if n > 0 => n.min(i64::from(i32::MAX)) as i32,
        _ => machine.build_jobs(),
    };
    Limits {
        max_jobs: max_jobs.max(1),
        max_load,
        cpus,
        min_free_mem_mb: MIN_FREE_MEM_MB,
        own_nice: own_nice_level(s.setting_int("daemon.nice")),
        build_jobs,
        source,
    }
}

/// What the governor says this tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Verdict {
    /// Local runs that may still start under the cap (0 while held; half
    /// the cap, never under one, while the kernel's memory warning stands).
    pub(crate) room: usize,
    /// Why the machine takes nothing new right now, when it is saturated.
    pub(crate) hold: Option<String>,
    /// One local slot stays open to the owner's work alone (the lead's runs,
    /// answers, the jobs of owner tasks), in two cases. Under a soft hold -
    /// load under [`EXPRESS_LOAD_FACTOR`] times the cap - while a slot is
    /// free under `daemon.max_jobs`. And when the fleet's own work fills
    /// `daemon.max_jobs` (or the half of it the kernel's memory warning
    /// leaves) with no owner-driven run among it: one slot over the cap, so
    /// the owner's answer does not wait out the fleet's verifies (never a
    /// second one; never while a self-update build waits). The fleet's own
    /// builds are what saturate the machine; the owner's request should not
    /// queue behind them. Critical memory pressure, a floor breach and a
    /// load over twice the cap shut it.
    pub(crate) express: bool,
}

/// How far over `daemon.max_load` the express lane still opens: a machine at
/// twice the cap is swapping, and one more run would not finish sooner.
pub(crate) const EXPRESS_LOAD_FACTOR: f64 = 2.0;

/// The pure decision: `running` local runs are in flight, `owner_running` of
/// them owner-driven (`jobs::is_owner_driven`), `measuring` measurements
/// (the experiment lane's builds, [`measuring`]) run beside them and take a
/// slot each, `reserved` slots are kept free for something else (a
/// self-update build that waits).
pub(crate) fn decide(
    limits: &Limits,
    pressure: &Pressure,
    running: usize,
    measuring: usize,
    reserved: usize,
    owner_running: usize,
) -> Verdict {
    let hold = hold_reason(limits, pressure);
    let hard = pressure.mem_critical
        || pressure
            .free_mem_mb
            .is_some_and(|free| limits.min_free_mem_mb > 0 && free < limits.min_free_mem_mb)
        || pressure
            .load1
            .is_some_and(|load| load > limits.max_load * EXPRESS_LOAD_FACTOR);
    // The kernel's warning (level 2) stands for hours on a 16 GB laptop with
    // a browser open; as a hold it closed the fleet for a day. It halves the
    // cap instead, never under one, and only the critical level holds.
    let cap = limits.max_jobs.max(0) as usize;
    let cap = if pressure.mem_warning && !hard {
        (cap / 2).max(1)
    } else {
        cap
    };
    let taken = running + measuring + reserved;
    let room = if hold.is_some() {
        0
    } else {
        cap.saturating_sub(taken)
    };
    let under_cap = taken < cap;
    let soft_hold = hold.is_some() && !hard && under_cap;
    let fleet_full = !hard && !under_cap && reserved == 0 && owner_running == 0;
    Verdict {
        room,
        hold,
        express: soft_hold || fleet_full,
    }
}

/// The owner reservation (docs/owner-reservation.md): while an owner-driven
/// job this machine could run is queued, one local slot is kept that the
/// fleet's work cannot take. Under the cap it is one of the verdict's
/// `room`; over it - the cap full, the load over `daemon.max_load` at any
/// height, free memory under the floor - it is the owner's one run over the
/// line, while no owner-driven run is in flight here. Only the kernel's
/// critical memory pressure (the machine swapping) keeps it shut. Returns
/// the local slots the fleet may still take beside it, or None when no slot
/// is kept and the verdict stands as it is.
pub(crate) fn reserve_for_owner(
    verdict: &Verdict,
    pressure: &Pressure,
    owner_queued: bool,
    owner_running: usize,
) -> Option<usize> {
    if !owner_queued || pressure.mem_critical {
        return None;
    }
    if verdict.room > 0 {
        return Some(verdict.room - 1);
    }
    (owner_running == 0).then_some(0)
}

/// The local slots the governor offers the owner's queued work this tick,
/// with no run asked to yield: the verdict's room under the cap, else the
/// one slot over it ([`Verdict::express`], or the owner reservation over a
/// full cap, [`reserve_for_owner`]) - the same slots the dispatch opens. The
/// yield (yield_lane.rs) calls the fleet full when this and the paired
/// machines' free slots are none, not when the jobs in flight reach the
/// fleet's nominal total: the kernel's memory warning halves the cap, and
/// the express slot is one past it.
pub(crate) fn owner_room(
    limits: &Limits,
    pressure: &Pressure,
    running: usize,
    measuring: usize,
    reserved: usize,
    owner_running: usize,
) -> usize {
    let verdict = decide(
        limits,
        pressure,
        running,
        measuring,
        reserved,
        owner_running,
    );
    if verdict.room > 0 {
        verdict.room
    } else {
        usize::from(
            verdict.express || reserve_for_owner(&verdict, pressure, true, owner_running).is_some(),
        )
    }
}

/// Whether the next local run would start under the cap (the cap halved
/// under the kernel's memory warning) with no hard hold standing: `running`,
/// `measuring` and `reserved` as in [`decide`]. A slot under the cap, the
/// express slot under a soft hold among them, is kept back from the owner's
/// work past its project's share when it is the machine's last free one
/// (jobs.rs `LeaseOpts::keep_slot`); the express slot over a full cap is one
/// past it, and nothing is kept there. The slot a waiting self-update keeps
/// (`reserved`) counts as taken, so with an update waiting two slots stay
/// out of the owner's work past its share, the update's and the kept one
/// (docs/owner-reservation.md).
pub(crate) fn under_cap(
    limits: &Limits,
    pressure: &Pressure,
    running: usize,
    measuring: usize,
    reserved: usize,
) -> bool {
    let hard = pressure.mem_critical
        || pressure
            .free_mem_mb
            .is_some_and(|free| limits.min_free_mem_mb > 0 && free < limits.min_free_mem_mb)
        || pressure
            .load1
            .is_some_and(|load| load > limits.max_load * EXPRESS_LOAD_FACTOR);
    let cap = limits.max_jobs.max(0) as usize;
    let cap = if pressure.mem_warning && !hard {
        (cap / 2).max(1)
    } else {
        cap
    };
    !hard && running + measuring + reserved < cap
}

/// Why the owner's next run could not start on this machine once a fleet
/// run yielded its lane, or None when it could: the yield (yield_lane.rs)
/// asks a fleet run to hold only then. It is the governor's own answer for
/// the tick after the yield - [`decide`] with the yielding run gone, plus
/// the reservation ([`reserve_for_owner`]) - so the yield and the dispatch
/// never disagree. The kernel's critical memory pressure refuses the owner;
/// so does a load or memory hold while the owner's one run over the line is
/// already in flight here and the cap leaves no express slot. A full cap
/// alone does not: the run that yields frees its slot.
pub(crate) fn owner_refusal(
    limits: &Limits,
    pressure: &Pressure,
    running: usize,
    measuring: usize,
    reserved: usize,
    owner_running: usize,
) -> Option<String> {
    let after = decide(
        limits,
        pressure,
        running.saturating_sub(1),
        measuring,
        reserved,
        owner_running,
    );
    if after.room > 0
        || after.express
        || reserve_for_owner(&after, pressure, true, owner_running).is_some()
    {
        return None;
    }
    Some(match after.hold {
        Some(hold) if owner_running > 0 => {
            format!("{hold}, and the owner's one run over the line is already in flight")
        }
        Some(hold) => hold,
        None => format!(
            "{} local slot(s) taken of this machine's {}",
            running + measuring + reserved,
            limits.max_jobs
        ),
    })
}

/// The prefix every hold line carries, so a lane that was held can tell
/// the hold from a failure and retry soon rather than in twenty minutes.
pub(crate) const HOLD_PREFIX: &str = "machine saturated: ";

/// Whether `note` is a governor hold ([`hold_reason`]) rather than a failure.
pub(crate) fn is_hold_note(note: &str) -> bool {
    note.contains(HOLD_PREFIX)
}

/// Why nothing new should start on this machine, if the pressure says so.
/// The numbers that drift (load, megabytes) are normalised by the idle log,
/// so the line is said once per change and summarised hourly, not per tick.
pub(crate) fn hold_reason(limits: &Limits, pressure: &Pressure) -> Option<String> {
    if let Some(load) = pressure.load1
        && load > limits.max_load
    {
        return Some(format!(
            "{HOLD_PREFIX}load {load:.1} on {} CPUs is over the line {:.0}",
            limits.cpus.max(1),
            limits.max_load
        ));
    }
    if pressure.mem_critical {
        return Some(format!(
            "{HOLD_PREFIX}the kernel reports critical memory pressure"
        ));
    }
    if let Some(free) = pressure.free_mem_mb
        && limits.min_free_mem_mb > 0
        && free < limits.min_free_mem_mb
    {
        return Some(format!(
            "{HOLD_PREFIX}{free} MB of memory free, under the floor {}",
            limits.min_free_mem_mb
        ));
    }
    None
}

/// Why a self-update build should wait, if it should: the machine is
/// saturated, or every slot under the cap holds a run (the dispatcher keeps
/// one slot free while the build waits, so it starts within one run).
pub(crate) fn build_hold(limits: &Limits, pressure: &Pressure, running: usize) -> Option<String> {
    if let Some(hold) = hold_reason(limits, pressure) {
        return Some(hold);
    }
    if running >= limits.max_jobs.max(1) as usize {
        return Some(format!(
            "{running} local run(s) in flight fill this machine's {} slot(s)",
            limits.max_jobs
        ));
    }
    None
}

/// Keeps at most `room` local slots in `slots` (remote slots stay: their
/// machines carry their own load); returns how many local slots went.
pub(crate) fn trim_local_slots(slots: &mut Vec<String>, room: usize) -> usize {
    let mut kept = 0usize;
    let before = slots.len();
    slots.retain(|name| {
        if name != crate::executor::LOCAL_EXECUTOR {
            return true;
        }
        kept += 1;
        kept <= room
    });
    before - slots.len()
}

/// The startup line naming the limits in force.
pub(crate) fn describe(machine: &Machine, limits: &Limits) -> String {
    let floor = if limits.min_free_mem_mb > 0 {
        format!("under {} MB of memory free", limits.min_free_mem_mb)
    } else {
        "never for memory".to_string()
    };
    let own = if limits.own_nice > 0 {
        format!(", the daemon itself at nice {}", limits.own_nice)
    } else {
        String::new()
    };
    format!(
        "load governor: up to {} local run(s) at once ({}; {} cores, {} CPUs, {} MB), nothing new over load {:.0} or {floor}; children run at nice {CHILD_NICE}, builds at -j{}{own}",
        limits.max_jobs,
        limits.source,
        machine.cores,
        machine.cpus,
        machine.mem_mb,
        limits.max_load,
        limits.build_jobs
    )
}

/// Lowers the daemon's own scheduling priority to `level` once, at startup,
/// so its planning, sweeps and landing passes yield to the owner's foreground
/// work while the loop still comes before its children at [`CHILD_NICE`];
/// 0 leaves the level the service started it at. A process cannot raise its
/// priority back, so under test (where the daemon runs in the test host's
/// process) nothing is applied.
pub(crate) fn renice_self(level: i32) {
    #[cfg(test)]
    {
        let _ = level;
    }
    #[cfg(not(test))]
    {
        renice_pid(0, own_nice_level(i64::from(level)));
    }
}

/// `daemon.nice` as a level `setpriority` accepts.
pub(crate) fn own_nice_level(setting: i64) -> i32 {
    setting.clamp(0, i64::from(MAX_NICE)) as i32
}

/// Where a measurement in progress leaves its mark under the hive home:
/// one file per measurement, named by the process running it. The
/// experiment lane runs inside the daemon, but `hive land` and `hive
/// evolve measure` run in their own processes, and the governor must count
/// every measurement's builds against the cap - two fixture builds at
/// `-j6` beside four runs were the biggest load this machine put on itself.
pub(crate) const MEASURING_DIR: &str = "measuring";

static MEASURING_SEQ: AtomicUsize = AtomicUsize::new(0);

/// A measurement in progress; dropping it removes the mark.
#[derive(Debug)]
pub(crate) struct MeasuringMark {
    path: std::path::PathBuf,
}

impl MeasuringMark {
    pub(crate) fn new(home: &str) -> Self {
        let dir = std::path::Path::new(home).join(MEASURING_DIR);
        let _ = std::fs::create_dir_all(&dir);
        let seq = MEASURING_SEQ.fetch_add(1, Ordering::SeqCst);
        let path = dir.join(format!("{}-{seq}", std::process::id()));
        let _ = std::fs::write(&path, format!("{}\n", std::process::id()));
        Self { path }
    }
}

impl Drop for MeasuringMark {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// How many measurements are in progress under `home`: the marks whose
/// process is still alive. A measurement that died with its process left
/// its mark behind; the count drops it rather than hold a slot for good.
pub(crate) fn measuring(home: &str) -> usize {
    let dir = std::path::Path::new(home).join(MEASURING_DIR);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0;
    };
    let mut n = 0;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let pid = name.split('-').next().and_then(|p| p.parse::<u32>().ok());
        match pid {
            Some(pid) if process_alive(pid) => n += 1,
            _ => {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    n
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: kill with signal 0 only asks whether the process exists.
    let rc = unsafe { libc::kill(pid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
fn process_alive(_pid: u32) -> bool {
    true
}

/// Puts `child` at nice [`CHILD_NICE`] whatever process runs it (the
/// self-update build, which only the daemon runs).
pub(crate) fn low_priority(child: &std::process::Child) {
    renice_pid(child.id(), CHILD_NICE);
}

/// The machine's pressure right now, as the daemon sees it. Under test the
/// pinned pressure or a calm machine, so a loaded test host never turns a
/// dispatch test into a hold; [`read_pressure`] is the reading itself.
pub(crate) fn pressure() -> Pressure {
    #[cfg(test)]
    {
        (*TEST_PRESSURE.lock().unwrap_or_else(|e| e.into_inner())).unwrap_or(CALM)
    }
    #[cfg(not(test))]
    {
        read_pressure()
    }
}

/// What the hardware says; each part is None where the platform has no
/// reading, which the decision treats as no pressure.
pub(crate) fn read_pressure() -> Pressure {
    let level = mem_pressure_level();
    Pressure {
        load1: load1(),
        free_mem_mb: free_mem_mb(),
        mem_warning: level.is_some_and(|level| level >= 2),
        mem_critical: level.is_some_and(|level| level >= 4),
    }
}

/// An idle machine, the pressure tests see unless they pin another.
#[cfg(test)]
pub(crate) const CALM: Pressure = Pressure {
    load1: Some(0.5),
    free_mem_mb: Some(8192),
    mem_warning: false,
    mem_critical: false,
};

/// Tests pin the pressure the daemon sees; [`pin_pressure`] sets it and the
/// guard it returns resets it.
#[cfg(test)]
pub(crate) static TEST_PRESSURE: std::sync::Mutex<Option<Pressure>> = std::sync::Mutex::new(None);

#[cfg(test)]
pub(crate) struct PressureGuard;

#[cfg(test)]
impl Drop for PressureGuard {
    fn drop(&mut self) {
        *TEST_PRESSURE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// The pressure the daemon sees until the guard drops.
#[cfg(test)]
pub(crate) fn pin_pressure(p: Pressure) -> PressureGuard {
    *TEST_PRESSURE.lock().unwrap_or_else(|e| e.into_inner()) = Some(p);
    PressureGuard
}

#[cfg(any(
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
))]
fn load1() -> Option<f64> {
    let mut loads = [0f64; 3];
    // SAFETY: getloadavg writes at most three doubles into the array given.
    let n = unsafe { libc::getloadavg(loads.as_mut_ptr(), 3) };
    (n >= 1 && loads[0].is_finite()).then_some(loads[0])
}

#[cfg(target_os = "linux")]
fn load1() -> Option<f64> {
    std::fs::read_to_string("/proc/loadavg")
        .ok()?
        .split_whitespace()
        .next()?
        .parse::<f64>()
        .ok()
        .filter(|l| l.is_finite())
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "linux"
)))]
fn load1() -> Option<f64> {
    None
}

// libc points at the mach2 crate for this call; the workspace does not carry
// it for one function that libc still binds correctly.
#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn host_port() -> libc::mach_port_t {
    static PORT: std::sync::OnceLock<libc::mach_port_t> = std::sync::OnceLock::new();
    // SAFETY: mach_host_self has no preconditions; the port is kept for the
    // process's life rather than leaked once per reading.
    *PORT.get_or_init(|| unsafe { libc::mach_host_self() })
}

#[cfg(target_os = "macos")]
fn free_mem_mb() -> Option<i64> {
    let page = sysctl_int("hw.pagesize").filter(|p| *p > 0)? as u64;
    // SAFETY: vm_statistics64 is plain data; host_statistics64 fills at most
    // `count` integers of it.
    let mut stat: libc::vm_statistics64 = unsafe { std::mem::zeroed() };
    let mut count = libc::HOST_VM_INFO64_COUNT;
    let rc = unsafe {
        libc::host_statistics64(
            host_port(),
            libc::HOST_VM_INFO64,
            (&mut stat as *mut libc::vm_statistics64).cast(),
            &mut count,
        )
    };
    if rc != libc::KERN_SUCCESS {
        return vm_stat_text().and_then(|text| vm_stat_free_mb(&text, page));
    }
    Some(usable_mb(
        [
            u64::from(stat.free_count),
            u64::from(stat.inactive_count),
            u64::from(stat.speculative_count),
            u64::from(stat.purgeable_count),
        ],
        page,
    ))
}

/// `vm_stat`'s report, the reading's fallback when the mach call fails.
#[cfg(target_os = "macos")]
fn vm_stat_text() -> Option<String> {
    let out = std::process::Command::new("vm_stat").output().ok()?;
    String::from_utf8(out.stdout).ok()
}

/// Megabytes in `pages` of `page` bytes each.
#[cfg(any(target_os = "macos", test))]
fn usable_mb(pages: [u64; 4], page: u64) -> i64 {
    let pages = pages.iter().fold(0u64, |sum, n| sum.saturating_add(*n));
    (pages.saturating_mul(page) / (1024 * 1024)) as i64
}

/// The memory a new run could use, from `vm_stat`'s text: the free,
/// inactive and speculative pages, and the purgeable ones - caches the
/// kernel drops the moment something asks, which a 16 GB laptop holds a
/// gigabyte or two of while `free` alone reads a few hundred megabytes and
/// the floor holds the fleet for nothing.
#[cfg(any(target_os = "macos", test))]
fn vm_stat_free_mb(text: &str, page: u64) -> Option<i64> {
    let field = |name: &str| -> Option<u64> {
        text.lines().find_map(|line| {
            let (key, rest) = line.split_once(':')?;
            (key.trim() == name)
                .then(|| rest.trim().trim_end_matches('.').parse::<u64>().ok())
                .flatten()
        })
    };
    let free = field("Pages free")?;
    Some(usable_mb(
        [
            free,
            field("Pages inactive").unwrap_or(0),
            field("Pages speculative").unwrap_or(0),
            field("Pages purgeable").unwrap_or(0),
        ],
        page,
    ))
}

/// The kernel's own memory pressure level: 1 normal, 2 warning, 4 critical;
/// None on older kernels.
#[cfg(target_os = "macos")]
fn mem_pressure_level() -> Option<i64> {
    sysctl_int("kern.memorystatus_vm_pressure_level")
}

#[cfg(target_os = "linux")]
fn free_mem_mb() -> Option<i64> {
    meminfo_mb("MemAvailable")
}

#[cfg(target_os = "linux")]
fn mem_pressure_level() -> Option<i64> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn free_mem_mb() -> Option<i64> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn mem_pressure_level() -> Option<i64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(max_jobs: i32, max_load: f64, min_free_mem_mb: i64) -> Limits {
        Limits {
            max_jobs,
            max_load,
            cpus: 12,
            min_free_mem_mb,
            own_nice: 0,
            build_jobs: 6,
            source: "test",
        }
    }

    fn calm() -> Pressure {
        Pressure {
            load1: Some(2.0),
            free_mem_mb: Some(6000),
            mem_warning: false,
            mem_critical: false,
        }
    }

    #[test]
    fn default_max_jobs_follows_cores_memory_and_the_cap() {
        for (name, cpus, cores, mem_mb, want) in [
            ("12 GB, 12 cores: four", 12, 12, 12288, 4),
            ("16 GB, 12 cores: five", 12, 12, 16384, 5),
            ("8 GB, 8 cores: two", 8, 8, 8192, 2),
            ("2 GB, 2 cores: never under one", 2, 2, 2048, 1),
            ("64 GB, 32 cores: no cap of four", 64, 32, 65536, 21),
            ("16 GB, 4 cores: the cores", 8, 4, 16384, 4),
            ("one core", 1, 1, 16384, 1),
            ("memory unknown: cores alone", 12, 12, 0, 12),
        ] {
            let m = Machine {
                cpus,
                cores,
                mem_mb,
            };
            assert_eq!(m.default_max_jobs(), want, "{name}");
        }
        assert_eq!(
            Machine {
                cpus: 12,
                cores: 12,
                mem_mb: 16384
            }
            .build_jobs(),
            6
        );
        assert_eq!(
            Machine {
                cpus: 1,
                cores: 1,
                mem_mb: 1024
            }
            .build_jobs(),
            1
        );
    }

    #[test]
    fn decide_admits_the_room_under_the_cap_and_nothing_under_pressure() {
        let l = limits(4, 12.0, 1024);
        assert_eq!(
            decide(&l, &calm(), 0, 0, 0, 0),
            Verdict {
                room: 4,
                hold: None,
                express: false
            }
        );
        assert_eq!(decide(&l, &calm(), 3, 0, 0, 0).room, 1);
        assert_eq!(decide(&l, &calm(), 4, 0, 0, 0).room, 0, "the cap is full");
        assert_eq!(
            decide(&l, &calm(), 7, 0, 0, 0).room,
            0,
            "over the cap never underflows"
        );
        assert_eq!(
            decide(&l, &calm(), 2, 0, 1, 0).room,
            1,
            "a reserved slot is not offered"
        );
        assert_eq!(decide(&l, &calm(), 3, 0, 1, 0).room, 0);
        let loaded = Pressure {
            load1: Some(40.1),
            ..calm()
        };
        let v = decide(&l, &loaded, 0, 0, 0, 0);
        assert_eq!(v.room, 0, "load over the cap holds every slot: {v:?}");
        assert_eq!(
            v.hold.as_deref(),
            Some("machine saturated: load 40.1 on 12 CPUs is over the line 12")
        );
        assert!(
            !v.express,
            "a load over twice the cap keeps the express lane shut: {v:?}"
        );
        let tight = Pressure {
            free_mem_mb: Some(512),
            ..calm()
        };
        let v = decide(&l, &tight, 0, 0, 0, 0);
        assert_eq!(v.room, 0);
        assert_eq!(
            v.hold.as_deref(),
            Some("machine saturated: 512 MB of memory free, under the floor 1024")
        );
        assert!(!v.express, "memory under the floor has no express lane");
        let critical = Pressure {
            mem_warning: true,
            mem_critical: true,
            ..calm()
        };
        let v = decide(&l, &critical, 0, 0, 0, 0);
        assert_eq!(v.room, 0);
        assert_eq!(
            v.hold.as_deref(),
            Some("machine saturated: the kernel reports critical memory pressure")
        );
        assert!(!v.express, "critical memory pressure has no express lane");
    }

    #[test]
    fn decide_halves_the_cap_under_the_kernels_memory_warning() {
        let l = limits(4, 12.0, 1024);
        let warned = Pressure {
            mem_warning: true,
            ..calm()
        };
        let v = decide(&l, &warned, 0, 0, 0, 0);
        assert_eq!(v.hold, None, "the warning alone is not a hold: {v:?}");
        assert_eq!(v.room, 2, "half the cap starts: {v:?}");
        assert!(!v.express, "room under the halved cap needs no lane");
        let v = decide(&l, &warned, 2, 0, 0, 0);
        assert_eq!(v.room, 0, "the halved cap is full");
        assert!(
            v.express,
            "the fleet filling the halved cap lends the owner one slot: {v:?}"
        );
        assert!(
            !decide(&l, &warned, 2, 0, 0, 1).express,
            "not while an owner's run is among them"
        );
        assert_eq!(
            decide(&limits(1, 12.0, 1024), &warned, 0, 0, 0, 0).room,
            1,
            "never under one"
        );
        assert_eq!(
            decide(&limits(3, 12.0, 1024), &warned, 0, 0, 0, 0).room,
            1,
            "three halves down to one"
        );
        let warned_and_tight = Pressure {
            mem_warning: true,
            free_mem_mb: Some(512),
            ..calm()
        };
        let v = decide(&l, &warned_and_tight, 0, 0, 0, 0);
        assert_eq!(v.room, 0, "the floor still holds under the warning");
        assert!(v.hold.is_some_and(|h| h.contains("512 MB of memory free")));
        assert!(!v.express, "memory under the floor shuts the lane");
        let warned_and_loaded = Pressure {
            mem_warning: true,
            load1: Some(15.0),
            ..calm()
        };
        let v = decide(&l, &warned_and_loaded, 0, 0, 0, 0);
        assert_eq!(v.room, 0, "a load hold still closes the slots");
        assert!(v.express, "a soft load hold keeps the lane: {v:?}");
    }

    #[test]
    fn owner_room_is_the_slots_the_dispatch_offers_the_owner_not_the_nominal_cap() {
        let l = limits(4, 12.0, 1024);
        let warned = Pressure {
            mem_warning: true,
            ..calm()
        };
        // Under the cap: the room.
        assert_eq!(owner_room(&l, &calm(), 1, 0, 0, 0), 3);
        // The memory warning halves four to two: two runs, one the owner's,
        // leave no slot although two of the four are not taken.
        assert_eq!(owner_room(&l, &warned, 2, 0, 0, 1), 0);
        // With none of them the owner's, the express slot is one.
        assert_eq!(owner_room(&l, &warned, 2, 0, 0, 0), 1);
        // A full cap with the owner's run among it: none; a self-update
        // build waiting takes the slot it keeps.
        assert_eq!(owner_room(&l, &calm(), 4, 0, 0, 1), 0);
        assert_eq!(owner_room(&l, &calm(), 3, 0, 1, 1), 0);
        // Critical memory pressure: nothing.
        let critical = Pressure {
            mem_critical: true,
            ..calm()
        };
        assert_eq!(owner_room(&l, &critical, 0, 0, 0, 0), 0);
    }

    #[test]
    fn decide_counts_measurements_in_flight_against_the_cap() {
        let l = limits(4, 12.0, 1024);
        assert_eq!(
            decide(&l, &calm(), 2, 1, 0, 0).room,
            1,
            "a measurement takes a slot like a run"
        );
        assert_eq!(decide(&l, &calm(), 2, 2, 0, 0).room, 0);
        let v = decide(&l, &calm(), 3, 1, 0, 0);
        assert_eq!(v.room, 0);
        assert!(
            v.express,
            "runs and a measurement filling the cap are the fleet's work: {v:?}"
        );
        assert_eq!(
            decide(&l, &calm(), 0, 9, 0, 0).room,
            0,
            "measurements over the cap never underflow"
        );
        assert_eq!(
            decide(&l, &calm(), 1, 1, 1, 0).room,
            1,
            "a reserved slot counts beside them"
        );
    }

    #[test]
    fn decide_keeps_an_express_slot_for_the_owner_while_load_alone_holds() {
        let l = limits(4, 12.0, 1024);
        let busy = Pressure {
            load1: Some(15.0),
            ..calm()
        };
        let v = decide(&l, &busy, 2, 0, 0, 0);
        assert_eq!(v.room, 0, "the hold still closes the ordinary slots");
        assert!(v.hold.is_some());
        assert!(v.express, "load alone, under the cap: {v:?}");
        assert!(
            decide(&l, &busy, 4, 0, 0, 0).express,
            "a full cap under a soft hold lends one slot over it"
        );
        assert!(
            !decide(&l, &busy, 4, 0, 0, 1).express,
            "and never a second while an owner's run is in flight"
        );
        assert!(
            !decide(&l, &busy, 3, 0, 1, 0).express,
            "a reserved slot counts against the cap and keeps the lane shut"
        );
        let swamped = Pressure {
            load1: Some(24.1),
            ..calm()
        };
        assert!(!decide(&l, &swamped, 0, 0, 0, 0).express);
        let loaded_and_tight = Pressure {
            load1: Some(15.0),
            free_mem_mb: Some(512),
            ..calm()
        };
        assert!(!decide(&l, &loaded_and_tight, 0, 0, 0, 0).express);
        assert!(
            !decide(&l, &calm(), 0, 0, 0, 0).express,
            "no hold, room under the cap: no lane needed"
        );
    }

    #[test]
    fn the_owner_reservation_keeps_one_slot_and_runs_over_the_load_line() {
        let l = limits(4, 24.0, 1024);
        let owner =
            |v: &Verdict, p: &Pressure, owner_running| reserve_for_owner(v, p, true, owner_running);
        // Room under the cap: one of it is the owner's, the fleet the rest.
        let v = decide(&l, &calm(), 1, 0, 0, 0);
        assert_eq!(owner(&v, &calm(), 0), Some(2));
        assert_eq!(
            reserve_for_owner(&v, &calm(), false, 0),
            None,
            "nothing queued"
        );
        // The fleet fills the cap: the owner's one run goes over it.
        let full = decide(&l, &calm(), 4, 0, 0, 0);
        assert_eq!(owner(&full, &calm(), 0), Some(0));
        // Load far over daemon.max_load (past the express lane's twice) and
        // memory under the floor: the owner's one run still starts...
        for p in [
            Pressure {
                load1: Some(60.0),
                ..calm()
            },
            Pressure {
                free_mem_mb: Some(300),
                ..calm()
            },
        ] {
            let v = decide(&l, &p, 4, 0, 0, 0);
            assert!(v.hold.is_some() && !v.express, "{v:?}");
            assert_eq!(owner(&v, &p, 0), Some(0), "{p:?}");
            // ...one only: with it in flight the line holds the owner too.
            assert_eq!(owner(&v, &p, 1), None, "{p:?}");
        }
        // The kernel's critical pressure is the machine swapping: nobody.
        let critical = Pressure {
            mem_critical: true,
            ..calm()
        };
        assert_eq!(
            owner(&decide(&l, &critical, 0, 0, 0, 0), &critical, 0),
            None
        );
    }

    #[test]
    fn the_owner_is_refused_only_where_the_governor_would_refuse_it_after_a_yield() {
        let l = limits(4, 24.0, 1024);
        // A full cap alone is no refusal: the yielding run frees its slot.
        assert_eq!(owner_refusal(&l, &calm(), 4, 0, 0, 0), None);
        let loaded = Pressure {
            load1: Some(30.0),
            ..calm()
        };
        // Over the line with no owner run in flight: the reservation admits
        // the owner's one run over it.
        assert_eq!(owner_refusal(&l, &loaded, 4, 0, 0, 0), None);
        // Under twice the line the express lane takes a second under the
        // cap, as the governor itself would.
        assert_eq!(owner_refusal(&l, &loaded, 4, 0, 0, 1), None);
        // Past twice the line, with the owner's one run already over it,
        // the load refuses the next, and the reason names both.
        let swamped = Pressure {
            load1: Some(60.0),
            ..calm()
        };
        assert_eq!(owner_refusal(&l, &swamped, 4, 0, 0, 0), None);
        let why = owner_refusal(&l, &swamped, 4, 0, 0, 1).expect("refused");
        assert!(
            why.contains("load 60.0") && why.contains("already in flight"),
            "{why}"
        );
        let critical = Pressure {
            mem_critical: true,
            ..calm()
        };
        assert!(
            owner_refusal(&l, &critical, 1, 0, 0, 0)
                .is_some_and(|why| why.contains("critical memory pressure"))
        );
    }

    #[test]
    fn decide_lends_one_slot_over_a_full_cap_to_the_owner_alone() {
        let l = limits(2, 12.0, 1024);
        // The fleet's own work fills daemon.max_jobs on a calm machine: one
        // express slot opens over the cap, restricted to owner-driven jobs.
        let v = decide(&l, &calm(), 2, 0, 0, 0);
        assert_eq!(v.room, 0, "nothing ordinary starts over the cap");
        assert_eq!(v.hold, None, "a full cap is not a hold");
        assert!(v.express, "{v:?}");
        assert!(
            !decide(&l, &calm(), 1, 0, 0, 0).express,
            "room under the cap needs no lane"
        );
        assert!(
            !decide(&l, &calm(), 3, 0, 0, 1).express,
            "the owner's run in flight is the lane in use: no second one"
        );
        assert!(
            !decide(&l, &calm(), 2, 0, 0, 1).express,
            "an owner-driven run among the two is the owner served"
        );
        assert!(
            !decide(&l, &calm(), 1, 0, 1, 0).express,
            "a self-update build waiting for room keeps the lane shut"
        );
        assert!(
            decide(&l, &calm(), 5, 0, 0, 0).express,
            "over the cap (lowered while runs were in flight) the owner still gets one"
        );
        let swamped = Pressure {
            load1: Some(24.1),
            ..calm()
        };
        assert!(!decide(&l, &swamped, 2, 0, 0, 0).express);
        let critical = Pressure {
            mem_critical: true,
            ..calm()
        };
        assert!(!decide(&l, &critical, 2, 0, 0, 0).express);
        let tight = Pressure {
            free_mem_mb: Some(512),
            ..calm()
        };
        assert!(!decide(&l, &tight, 2, 0, 0, 0).express);
    }

    #[test]
    fn decide_treats_a_missing_reading_and_a_zero_floor_as_no_pressure() {
        let l = limits(2, 8.0, 1024);
        let blind = Pressure::default();
        assert_eq!(
            decide(&l, &blind, 0, 0, 0, 0),
            Verdict {
                room: 2,
                hold: None,
                express: false
            }
        );
        let off = limits(2, 8.0, 0);
        let tight = Pressure {
            free_mem_mb: Some(10),
            ..calm()
        };
        assert_eq!(
            decide(&off, &tight, 0, 0, 0, 0).hold,
            None,
            "floor 0 is off"
        );
        let at_the_line = Pressure {
            load1: Some(8.0),
            ..calm()
        };
        assert_eq!(
            decide(&l, &at_the_line, 0, 0, 0, 0).hold,
            None,
            "a load equal to the cap is not over it"
        );
    }

    #[test]
    fn a_hold_note_is_told_from_a_failure() {
        let l = limits(4, 12.0, 1024);
        let loaded = Pressure {
            load1: Some(30.0),
            ..calm()
        };
        assert!(is_hold_note(&hold_reason(&l, &loaded).unwrap()));
        assert!(is_hold_note(
            "branch not landed: measuring main deferred: machine saturated: load 30.0 on 12 CPUs is over the line 12"
        ));
        assert!(!is_hold_note("verify failed: 3 tests"));
    }

    #[test]
    fn build_hold_waits_for_pressure_or_a_free_slot() {
        let l = limits(4, 12.0, 1024);
        assert_eq!(build_hold(&l, &calm(), 3), None);
        assert_eq!(
            build_hold(&l, &calm(), 4).as_deref(),
            Some("4 local run(s) in flight fill this machine's 4 slot(s)")
        );
        assert_eq!(
            build_hold(&l, &calm(), 9).as_deref(),
            Some("9 local run(s) in flight fill this machine's 4 slot(s)")
        );
        let loaded = Pressure {
            load1: Some(30.0),
            ..calm()
        };
        assert!(
            build_hold(&l, &loaded, 0)
                .is_some_and(|h| h.starts_with("machine saturated: load 30.0")),
            "pressure holds the build even with every slot free"
        );
    }

    #[test]
    fn trim_local_slots_keeps_remote_slots_and_the_room() {
        let local = crate::executor::LOCAL_EXECUTOR.to_string();
        let mut slots = vec![
            "hand-a".to_string(),
            local.clone(),
            local.clone(),
            "hand-b".to_string(),
            local.clone(),
        ];
        assert_eq!(trim_local_slots(&mut slots, 1), 2);
        assert_eq!(
            slots,
            vec!["hand-a".to_string(), local.clone(), "hand-b".to_string()]
        );
        assert_eq!(trim_local_slots(&mut slots, 0), 1);
        assert_eq!(slots, vec!["hand-a".to_string(), "hand-b".to_string()]);
        assert_eq!(
            trim_local_slots(&mut slots, 0),
            0,
            "nothing local left to trim"
        );
        let mut plenty = vec![local.clone(), local.clone()];
        assert_eq!(
            trim_local_slots(&mut plenty, 5),
            0,
            "room beyond the slots trims nothing"
        );
        assert_eq!(plenty.len(), 2);
    }

    #[test]
    fn describe_names_the_limits_and_their_source() {
        let m = Machine {
            cpus: 12,
            cores: 12,
            mem_mb: 16384,
        };
        let line = describe(&m, &limits(4, 12.0, 1024));
        assert!(
            line.contains("up to 4 local run(s) at once (test; 12 cores, 12 CPUs, 16384 MB)")
                && line.contains("over load 12 or under 1024 MB of memory free")
                && line.ends_with("children run at nice 10, builds at -j6"),
            "{line}"
        );
        let off = describe(&m, &limits(2, 12.0, 0));
        assert!(off.contains("never for memory"), "{off}");
        let own = describe(
            &m,
            &Limits {
                own_nice: 5,
                ..limits(4, 12.0, 1024)
            },
        );
        assert!(
            own.ends_with("children run at nice 10, builds at -j6, the daemon itself at nice 5"),
            "{own}"
        );
    }

    #[test]
    fn own_nice_level_is_clamped_to_what_setpriority_accepts() {
        assert_eq!(own_nice_level(5), 5);
        assert_eq!(own_nice_level(40), MAX_NICE);
        assert_eq!(own_nice_level(-3), 0);
    }

    #[test]
    fn limits_prefer_the_flag_then_the_setting_then_the_machine() {
        let s = crate::test_helpers::new_store();
        crate::test_helpers_daemon::set_setting(&s, "capacity", "auto");
        let m = Machine {
            cpus: 12,
            cores: 12,
            mem_mb: 16384,
        };
        let l = limits_from(&s, &m, 0);
        assert_eq!((l.max_jobs, l.source), (5, "auto"), "16 GB at 3 GB a run");
        assert_eq!(
            l.max_load, 24.0,
            "the load line is twice the CPU count: the load average counts runnable threads"
        );
        assert_eq!(l.cpus, 12);
        assert_eq!(l.min_free_mem_mb, 1024, "the documented default floor");
        assert_eq!(l.own_nice, 5, "the documented default daemon.nice");
        assert_eq!(l.build_jobs, 6, "daemon.build_jobs 0 is half the cores");
        crate::test_helpers_daemon::set_setting(&s, "capacity", "2");
        for retired in [
            "workers",
            "parallel_per_project",
            "daemon.max_jobs",
            "daemon.max_load",
            "daemon.min_free_mem_mb",
        ] {
            let err = s
                .set_setting("human", retired, "3")
                .unwrap_err()
                .to_string();
            assert!(err.contains("is automatic now"), "{retired}: {err}");
        }
        crate::test_helpers_daemon::set_setting(&s, "daemon.nice", "0");
        crate::test_helpers_daemon::set_setting(&s, "daemon.build_jobs", "3");
        let l = limits_from(&s, &m, 0);
        assert_eq!((l.max_jobs, l.source), (2, "capacity"));
        assert_eq!(l.own_nice, 0, "daemon.nice 0 leaves the priority alone");
        assert_eq!(l.build_jobs, 3);
        let l = limits_from(&s, &m, 3);
        assert_eq!((l.max_jobs, l.source), (3, "--max-jobs"));
        assert!(
            s.set_setting("human", "daemon.build_jobs", "-1").is_err(),
            "the catalogue refuses a negative parallelism"
        );
    }

    #[test]
    fn build_env_exports_cargo_build_jobs_once_the_daemon_sets_it() {
        let _serial = crate::test_helpers::serial();
        set_build_jobs(0);
        let mut env = vec!["PATH=/bin".to_string()];
        build_env(&mut env);
        assert_eq!(env, ["PATH=/bin"], "nothing is exported under hive work");
        set_build_jobs(6);
        build_env(&mut env);
        assert_eq!(env, ["PATH=/bin", "CARGO_BUILD_JOBS=6"]);
        // The owner's own value, or a recipe's, is left alone.
        let mut own = vec!["CARGO_BUILD_JOBS=2".to_string()];
        build_env(&mut own);
        assert_eq!(own, ["CARGO_BUILD_JOBS=2"]);
        // A second call does not stack a second entry.
        build_env(&mut env);
        assert_eq!(env.len(), 2);
        // The daemon's own environment for children carries it from here on
        // (unless the test host exports its own, which wins).
        if std::env::var_os(BUILD_JOBS_VAR).is_none() {
            assert!(
                crate::proc::environ().contains(&"CARGO_BUILD_JOBS=6".to_string()),
                "proc::environ() folds the level in"
            );
            set_build_jobs(0);
            assert!(
                !crate::proc::environ()
                    .iter()
                    .any(|kv| kv.starts_with("CARGO_BUILD_JOBS=")),
                "cleared again"
            );
        }
        set_build_jobs(0);
    }

    fn limits_from(s: &crate::test_helpers::TestStore, m: &Machine, flag: i32) -> Limits {
        super::limits(s, m, flag)
    }

    #[test]
    fn meminfo_field_reads_kilobytes_as_megabytes() {
        let text = "MemTotal:       16318940 kB\nMemFree:          512000 kB\nMemAvailable:    8388608 kB\n";
        assert_eq!(meminfo_field_mb(text, "MemAvailable"), Some(8192));
        assert_eq!(meminfo_field_mb(text, "MemTotal"), Some(15936));
        assert_eq!(meminfo_field_mb(text, "SwapFree"), None);
    }

    #[test]
    fn vm_stat_text_counts_purgeable_pages_as_usable_memory() {
        // A 16 GB laptop under the kernel's warning: 300 MB free, but a
        // gigabyte and a half of caches the kernel drops on request.
        let text = "Mach Virtual Memory Statistics: (page size of 16384 bytes)\n\
                    Pages free:                               19200.\n\
                    Pages active:                            400000.\n\
                    Pages inactive:                           32000.\n\
                    Pages speculative:                         6400.\n\
                    Pages throttled:                              0.\n\
                    Pages wired down:                        120000.\n\
                    Pages purgeable:                          96000.\n\
                    \"Translation faults\":                 123456789.\n";
        // (19200 + 32000 + 6400 + 96000) pages * 16 KiB = 2400 MB.
        assert_eq!(vm_stat_free_mb(text, 16384), Some(2400));
        assert_eq!(
            vm_stat_free_mb("Pages free: 65536.\n", 16384),
            Some(1024),
            "a report without the other lines counts the free pages alone"
        );
        assert_eq!(vm_stat_free_mb("Pages active: 1.\n", 16384), None);
        assert_eq!(usable_mb([0, 0, 0, 0], 4096), 0);
    }

    #[test]
    fn measuring_marks_count_live_measurements_and_drop_stale_ones() {
        let home = crate::test_helpers::new_store().home.clone();
        assert_eq!(measuring(&home), 0, "no marks, no measurements");
        let first = MeasuringMark::new(&home);
        assert_eq!(measuring(&home), 1);
        let second = MeasuringMark::new(&home);
        assert_eq!(measuring(&home), 2, "one mark per measurement");
        drop(first);
        assert_eq!(
            measuring(&home),
            1,
            "a finished measurement takes its mark away"
        );
        // A measurement whose process is gone (no pid is this large) left
        // its mark: the count drops it and removes the file.
        let dir = std::path::Path::new(&home).join(MEASURING_DIR);
        let stale = dir.join("99999999-0");
        std::fs::write(&stale, "99999999\n").unwrap();
        std::fs::write(dir.join("not-a-pid"), "").unwrap();
        assert_eq!(measuring(&home), 1, "only the live one counts");
        assert!(!stale.exists(), "the stale mark is removed");
        assert!(!dir.join("not-a-pid").exists());
        drop(second);
        assert_eq!(measuring(&home), 0);
    }

    #[test]
    fn the_machine_and_its_pressure_read_without_panicking() {
        let m = Machine::read();
        assert!(m.cpus >= 1 && m.cores >= 1 && m.cores <= m.cpus, "{m:?}");
        assert!(m.default_max_jobs() >= 1 && m.default_max_jobs() <= m.cores);
        let p = read_pressure();
        assert!(p.load1.is_none_or(|l| l >= 0.0), "{p:?}");
        assert!(p.free_mem_mb.is_none_or(|mb| mb >= 0), "{p:?}");
        #[cfg(target_os = "macos")]
        {
            assert!(
                m.mem_mb > 0 && p.load1.is_some() && p.free_mem_mb.is_some(),
                "{m:?} {p:?}"
            );
        }
        assert_eq!(
            Machine::detect(),
            TEST_MACHINE,
            "tests see the fixed machine"
        );
    }

    #[test]
    fn tests_see_a_calm_machine_unless_they_pin_the_pressure() {
        let _serial = crate::test_helpers::serial();
        assert_eq!(pressure(), CALM);
        {
            let _pinned = pin_pressure(Pressure {
                load1: Some(40.0),
                ..CALM
            });
            assert_eq!(pressure().load1, Some(40.0));
        }
        assert_eq!(pressure(), CALM, "the guard reset the pin");
    }

    #[test]
    fn child_nice_is_zero_until_the_daemon_sets_it() {
        let _serial = crate::test_helpers::serial();
        set_child_nice(0);
        assert_eq!(child_nice(), 0);
        set_child_nice(CHILD_NICE);
        assert_eq!(child_nice(), CHILD_NICE);
        set_child_nice(99);
        assert_eq!(child_nice(), 19, "clamped to the nice range");
        set_child_nice(0);
    }

    /// The nice level a child shell reports for itself once `renice` has
    /// had its say: the shell waits on stdin, so the parent's call lands
    /// before the shell looks.
    #[cfg(unix)]
    fn child_nice_of(renice: impl FnOnce(&std::process::Child)) -> i32 {
        use std::io::Write;
        let mut child = std::process::Command::new("/bin/sh")
            .args([
                "-c",
                "read go; ps -o nice= -p $$ 2>/dev/null || ps -o ni= -p $$",
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("sh runs");
        renice(&child);
        child
            .stdin
            .take()
            .expect("piped stdin")
            .write_all(b"\n")
            .expect("wake the shell");
        let out = child.wait_with_output().expect("sh ends");
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("nice column: {out:?}"))
    }

    #[cfg(unix)]
    #[test]
    fn low_priority_runs_the_child_at_a_higher_nice() {
        let nice = child_nice_of(low_priority);
        assert!(
            nice >= CHILD_NICE,
            "child nice {nice}, want at least {CHILD_NICE}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn nice_child_follows_the_level_the_daemon_set() {
        let _serial = crate::test_helpers::serial();
        set_child_nice(0);
        let own = child_nice_of(|_| {});
        let plain = child_nice_of(nice_child);
        assert_eq!(
            plain, own,
            "no level set: the child runs at the parent's nice"
        );
        set_child_nice(CHILD_NICE);
        let niced = child_nice_of(nice_child);
        set_child_nice(0);
        assert!(
            niced >= CHILD_NICE && (niced > own || own >= CHILD_NICE),
            "child nice {niced} with the level set, parent {own}"
        );
    }
}
