# Windows port

Hive builds and ships for macOS and Linux today. This page
lists everything that keeps the workspace from compiling for
`x86_64-pc-windows-msvc`, crate by crate, with the Windows replacement for
each site, and tracks how far the port has got. The work is planned in
TASK-091 (`hive task show TASK-091`).

Rule for every step: unix behaviour stays exactly as it is. A Windows
replacement goes in a `#[cfg(windows)]` / `#[cfg(not(unix))]` sibling. The
unix code is not rewritten to suit Windows.

## How the survey was taken

The Windows standard library (`rustup target add x86_64-pc-windows-msvc`) was not
installed on the survey machine, and installing toolchain components is
outside a worker's remit. Two methods were used instead:

1. **A grep of every unix-only API**: `std::os::unix::*` (`PermissionsExt`,
   `MetadataExt`, `CommandExt`, `ExitStatusExt`, `OpenOptionsExt`,
   `DirBuilderExt`, `OsStrExt`, `symlink`, fds), `nix::*`, `libc::*`,
   `signal_hook::iterator`, and the unix programs the code spawns
   (`/bin/sh`, `launchctl`, `codesign`, `ps`, `lsof`, `df`, ...). That gives
   353 sites in 19 of the 31 crates. Each site was then classed as
   production or test, and as already `cfg`-gated or not.
2. **An emulated compile**, `scripts/check-not-unix.sh`. It copies the
   tracked workspace to a scratch directory. In the named crates it switches
   `cfg(unix)` off, renames `std::os::unix`, hides `signal_hook::iterator`
   and drops `nix`/`libc`, then runs `cargo check` on them. Anything that
   fails there will fail on Windows. A pass is strong evidence, not proof:
   it does not exercise Windows-only std APIs or the third-party crates'
   own Windows builds. Those need the real target (see
   [Proposed CI](#proposed-ci-owner-owned)).

```sh
sh scripts/check-not-unix.sh --leaves --all-targets   # the leaves
sh scripts/check-not-unix.sh --all-targets hive-evolve hive-harness  # the middle layer
```

Third-party dependencies are not expected to block the port. `rusqlite`
(bundled), `reqwest`/rustls (`aws-lc-rs`, which needs cmake and NASM on the
Windows runner), `tokio`, `fs2`, `signal-hook` (minus `iterator`) and
`parakeet-rs`/`ort` (`load-dynamic`) all build for windows-msvc. `nix` does
not, and is only a dependency of hive-cli, hive-hub and hive-runner.
`fs2`'s Windows backend (`winapi`) is already in `Cargo.lock`.

## Status

| Layer | Crates | State |
| --- | --- | --- |
| Leaves | hive-gitenv, hive-cred, hive-store, hive-bus, hive-cli, hive-cred-setup, hive-cred-command, hive-provider, hive-provider-{anthropic,copilot,gemini,openai,fake}, hive-board, hive-tasks, hive-goals, hive-goals-command, hive-tally, hive-records, hive-codedelivery, hive-mcp, hive-voice, hive-hooks, hive-testutil | **Compile off unix**, libs and tests (`check-not-unix.sh --leaves --all-targets`, no warnings). Several only *compile*: behaviour still to port is listed below. |
| Middle | hive-evolve, hive-harness | **Compile off unix**, libs and tests (`check-not-unix.sh --all-targets hive-evolve hive-harness`). Off-unix warnings remain for parameters only unix code reads (`private.rs` modes, test helpers). |
| Top | hive-hub, hive-runner, hive-inspect, hive-ask, hive (the binary) | Not reached yet: they depend on the middle layer. hive-runner holds most of the work. |

## Done in the first step

These changes make the leaves compile off unix. Unix code paths are unchanged.

- **hive-cli `procs.rs`** (now `crates/hive-hand-core/src/procs.rs`, re-exported as `hive_cli::procs`): process groups and signals moved behind a
  private `sys` module. On unix it is the same `killpg`/`kill` through nix,
  `Command::process_group(0)` and `getppid`. Off unix it is a stub: nothing
  is grouped, nothing reads as alive, signals go nowhere. Callers then fall
  back to `Child::kill` on the direct child, so grandchildren survive. The
  test module is `cfg(all(test, unix))` because it drives `/bin/sh` and
  `ps`. Process listing already had a `not(macos|linux)` fallback (empty).
- **hive-hooks `exit_text`**: the signal-name table is `cfg(unix)`. Off
  unix an exit without a code prints `ExitStatus`'s own text.
- **hive-tasks `hive watch`**: the SIGINT/SIGTERM iterator is `cfg(unix)`.
  Off unix Ctrl+C keeps its default and ends the process.
- **hive-board `unmounted_volume`**: the device-number comparison is
  `cfg(unix)`. Off unix an existing mount point counts as mounted.
  `volume_root` only recognises unix mount parents anyway.
- **hive-records record lock**: `records.lock` now uses `fs2` on every
  platform: flock on unix (unchanged), LockFileEx on Windows. Before this,
  the non-unix branch failed every record write.
- **Tests**: hive-cred `resolve_file`, `resolve_refused_inside_worker`,
  `read_secret_accepts_tokens_inside_a_run` and `store_token` assert unix
  modes and symlinks, so they are `cfg(unix)`. hive-mcp's stdio-client exit
  test drives `/bin/sh` and is `cfg(unix)`. The mcp test helpers and
  hive-cred / hive-provider-openai mode assertions only `chmod` on unix.

## Done in the second step

These changes make hive-evolve and hive-harness compile off unix. Unix code
paths are unchanged.

- **hive-evolve `process.rs`**: the measured command's process group goes
  through hive-cli `procs` (`own_process_group`, and a new
  `procs::kill_group` for the immediate SIGKILL of what the command left)
  instead of its own `libc::killpg`; it follows `procs` to Job Objects.
  `SignalStop` keeps the signal-hook iterator on unix; off unix a
  `signal_hook::flag` and a watcher thread turn the first Ctrl+C into the
  cancel. `setpriority` (nice) stays unix-only for now.
- **hive-evolve `readonly.rs`, `clones.rs`**: `readonly::mode` and
  `readonly::set_mode` wrap the permission bits. Off unix they map to the
  read-only attribute (`Permissions::set_readonly`): a writable entry reads
  as 0o755/0o644, any mode without a write bit sets read-only. Freezing a
  scored tree and making a managed clone removable need nothing more.
  dev/ino link identity already had a stub; off unix an escaping hardlink is
  not detected yet (volume serial + file index later).
- **hive-harness `chat.rs`**: `Interrupts` keeps the iterator on unix; off
  unix a SIGINT flag and a watcher thread deliver ^C the same way (cancel the
  running turn, else reach the keyboard).
- **Tests**: those that create symlinks (fleet, scorer, held-out ledger and
  request links, the venv in `drop_ignored`, the private-dir alias), count
  hardlinks, or probe pids with `libc::kill(pid, 0)` (kept conversation
  processes) are `cfg(unix)`; hive-harness `localenv`'s test module is
  `cfg(all(test, unix))` like hive-cli `procs`, because it drives `sh` and
  real process groups. Mode assertions in the harness tests only run on unix.

## Every unix-only site, by crate

Counts are grep hits (production / test). "Gated" means the site already sits
behind `cfg(unix)` with a non-unix fallback that compiles.

### Leaves (compile off unix now)

| Crate | Sites | What is unix-only | Windows replacement |
| --- | --- | --- | --- |
| hive-gitenv | 1 / 1, gated | `is_executable_file` checks the `0o111` mode bits; `GIT_CONFIG_GLOBAL=/dev/null` | Already has `is_file()` fallback; `NUL` already chosen by `cfg!(windows)`. Later: honour `PATHEXT` (`git.exe`) when resolving `git` on PATH. |
| hive-store | 1 / 0, gated | `HIVE_HOME` chmod 0700 | Per-user `%LOCALAPPDATA%` already restricts by ACL; optional explicit DACL later. |
| hive-cred | 5 / 3, gated | 0600/0700 on key and token files; key file refused when group/other-readable | Owner-only DACL on write (`SetNamedSecurityInfoW`), and an ACL check in place of the `0o077` test. Until then Windows skips the check: **security-relevant, decide before shipping**. `keychain://` → Windows Credential Manager (`CredReadW`) or refuse with a pointer to `op://`/`file://` as on Linux. |
| hive-cred-setup | 1 / 1, gated | `set_mode` | Same DACL helper as hive-cred. |
| hive-provider (+ anthropic, copilot, gemini, openai, fake) | 1 / 7, gated | `chmod` 0600 on stored API keys | Same DACL helper. |
| hive-mcp | 3 / 4 | OAuth grant files 0600/0700 (gated); test fakes are `/bin/sh` scripts | DACL helper; the stdio-server tests need a `.cmd`/PowerShell fake or stay unix-only. `open_in_browser` already uses `cmd /C start`. Chrome/Edge discovery (`browser.rs`) lists only macOS/Linux paths: add `%ProgramFiles%\Google\Chrome\Application\chrome.exe` and `%ProgramFiles(x86)%\Microsoft\Edge\Application\msedge.exe`. |
| hive-voice | 1 / 0, gated | 0755 on the downloaded runtime | None needed (no exec bit). The runtime download must pick the Windows `onnxruntime.dll` asset. |
| hive-cli | 44 / 7 | `procs.rs`: process groups (`process_group(0)`, `killpg`), `kill(pid, 0)` liveness, SIGTERM→SIGKILL ladder, process listing via `proc_listallpids`/`proc_pidinfo` (macOS) and `/proc` (Linux); `limits.rs`: RLIMIT_NOFILE; `signed.rs`: `codesign` | **Job Objects**: spawn with `CREATE_NEW_PROCESS_GROUP \| CREATE_SUSPENDED`, assign to a Job with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, resume. "Group alive" = `QueryInformationJobObject` active processes > 0. SIGTERM = `GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT)` to the group, SIGKILL = `TerminateJobObject`. Listing: `CreateToolhelp32Snapshot` + `QueryFullProcessImageNameW`. The cwd is not readable without `NtQueryInformationProcess`, so match on exe path only. RLIMIT: nothing to raise (handles are not capped at 256), keep the `None` stub. codesign: already `(false, false)` off macOS; Authenticode is a separate release decision. |
| hive-hooks | 17 / 2 | Exit signal names via `ExitStatusExt::signal`; test hooks are `#!/bin/sh` scripts | Done (stub). Hook scripts run through `Command::new(target)`: on Windows a `.sh` target needs `sh.exe` from Git for Windows, so resolve the interpreter from the shebang or document `.cmd`/`.ps1` hooks. |
| hive-tasks | 2 / 5 | `signal_hook::iterator` for SIGINT/SIGTERM in `hive watch` | Done (default Ctrl+C). Later: `SetConsoleCtrlHandler` (e.g. the `ctrlc` crate, a new dependency, Tier 2) to cancel cleanly. |
| hive-board | 1 / 5 | `MetadataExt::dev` for unmounted removable volumes; test chmods read-only homes | Done (stub). Later: `GetVolumePathNameW` and compare with the drive root. A missing drive letter is the Windows "unmounted volume". |
| hive-records | 1 / 0 | Advisory lock on `records.lock` | Done: fs2 LockFileEx. |
| hive-bus, hive-goals(-command), hive-tally, hive-codedelivery, hive-testutil | 0 | None | None. hive-tally spawns `go` for Go projects, which is portable. |

### Middle (next step)

| Crate | Sites | What is unix-only | Windows replacement |
| --- | --- | --- | --- |
| hive-evolve | 36 / 17 | `process.rs`: `process_group`, `libc::killpg`, `setpriority` (nice), SIGKILL, signal-hook iterator, `MetadataExt`; `readonly.rs`: dev/ino identity and 0o444/0o555 read-only trees; `clones.rs`, `fleet.rs`, `genome.rs`, `commands.rs`, `heldout_auto.rs`, `measure.rs`, `usage.rs`: `symlink` and modes; `private.rs`: 0600/0700 | Process code goes through hive-cli `procs` (Job Objects) instead of its own `killpg`. Nice → `SetPriorityClass(BELOW_NORMAL_PRIORITY_CLASS)`. Identity: `GetFileInformationByHandle` (volume serial + file index) in place of dev/ino. Read-only: `Permissions::set_readonly` (file attribute) in place of mode bits. Symlinks: `std::os::windows::fs::symlink_dir/symlink_file`, which need Developer Mode or admin; fall back to directory junctions or copies. Compiles off unix since the second step. |
| hive-harness | 10 / 16 | `chat.rs`: signal-hook iterator; `conversation.rs`, `localenv.rs`: `libc::kill(pid, 0)` liveness and `ExitStatusExt`; `harness.rs`: tool scripts chmod; `buildenv.rs`: symlink; the `bash` tool runs `/bin/sh -c` | Liveness via `OpenProcess` + `GetExitCodeProcess == STILL_ACTIVE`. Ctrl handler as in hive-tasks. **The `bash` tool is the big product question**: the model writes POSIX shell. Options are Git for Windows' `sh.exe` (closest to today) or a PowerShell tool with its own name and description. That is a decision for the owner, not a mechanical port. Compiles off unix since the second step. |

### Top (after the middle)

| Crate | Sites | What is unix-only | Windows replacement |
| --- | --- | --- | --- |
| hive-runner | 77 / 85 | Spread over 40 files. **Service**: `service.rs` writes a launchd plist (macOS) or a systemd user unit (Linux) and runs `launchctl`. **Signals**: `commands.rs`, `daemon_watchdog.rs` (SIGUSR1 stack dump), `jobs.rs`, `util.rs`, `sweep.rs`, `proc.rs` use `kill`/SIGTERM and signal-hook. **Load**: `daemon_load.rs` reads `getloadavg`, Mach `host_statistics`, `vm_stat`, `pmset`, `setpriority`. **Disk**: `tmpspace.rs`, `bulk.rs` use `statvfs`, `df`, `du`, `dev()`. **Locks and identity**: `land_lock.rs`, `selfupdate.rs` compare dev/ino. **Permissions**: about 40 sites write 0600/0700/0755 (tokens, keys, scripts, hooks). **Symlinks**: `bulk.rs`, `verify_lock.rs`, `idle_hold.rs`, `buildcache.rs`, `recovery_api.rs`, `runners.rs`, `daemon.rs`. **Shell**: `/bin/sh -c` for every check, recipe and landing command (`exec.rs`, `executor.rs`, `proc.rs`, `landing_context.rs`, `service.rs`). **macOS tools**: `sign.rs` (`codesign`), `preflight.rs` (`xcode-select`, `xcodebuild`), `awake.rs` (`pmset`, caffeinate). **Users**: `hands.rs` uses `getuid` and `User`. | Service: `hive service install` → a Task Scheduler "at logon" task (`schtasks /Create /SC ONLOGON`, no admin needed) or a Windows service via `windows-service` (new dependency, Tier 2, needs admin). Recommend Task Scheduler first because it matches the per-user launchd agent. Signals: Job Objects from hive-cli; SIGUSR1 stack dump → a named event or a hub endpoint. Load: `GlobalMemoryStatusEx`, `GetSystemTimes` (no load average; derive one from CPU time). Disk: `GetDiskFreeSpaceExW`. Identity: volume serial + file index. Permissions: DACL helper (shared with hive-cred) or skip where the file sits under `%LOCALAPPDATA%`. Symlinks: junctions or copies. Shell: Git for Windows `sh.exe` located once and used for every `/bin/sh` site (same decision as the harness `bash` tool). macOS tools: already `cfg(target_os = "macos")` in most places, otherwise skip. Users: `GetUserNameW`, with `hands` (second-user isolation) unsupported on Windows at first. |
| hive-hub | 2 / 6 | `read.rs`: `nix::unistd::gethostname`; `register.rs`: signal-hook iterator; `server.rs`: `open`/`xdg-open`; tests: symlinks, chmod | `gethostname` → `std::env::var("COMPUTERNAME")` or `GetComputerNameExW`; Ctrl handler; `cmd /C start "" URL` (as hive-mcp already does). |
| hive-inspect, hive-ask, hive | 0 direct | Only through their dependencies | None. |

### Cross-cutting

- **Paths written as unix strings**: `/dev/null` (already `NUL` behind
  `cfg!(windows)` in gitenv/records), `/tmp` in `sweepable_dir`'s refusal
  list (hive-cli; add `%TEMP%`, `C:\`, `C:\Windows`), and `HOME`. About 100
  production reads of `HOME`. On Windows use `USERPROFILE` through one
  helper, since `HOME` is usually unset there.
- **`git`**: invoked by name everywhere, so it resolves to `git.exe` once
  `PATHEXT` is honoured (gitenv). Git for Windows also brings `sh.exe`, which
  is the cheapest way to keep `/bin/sh -c` sites working.
- **Line endings**: `core.autocrlf` must be off in clones Hive makes, or
  tree hashes and diffs will differ from the macOS ones. `hive_gitenv::env()`
  should set `GIT_CONFIG_PARAMETERS`/`-c core.autocrlf=false`.

## Proposed CI (owner-owned)

`.github/` is the owner's, so this page only proposes the workflow change. On
the release workflow, add a job beside the macOS one:

```yaml
  windows:
    runs-on: windows-2022
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@master
        with: { toolchain: "1.95", targets: x86_64-pc-windows-msvc }
      - uses: ilammy/setup-nasm@v1          # aws-lc-sys
      - run: cargo build --release --locked --bin hive --target x86_64-pc-windows-msvc
      - run: target\x86_64-pc-windows-msvc\release\hive.exe version
      - run: Compress-Archive target\x86_64-pc-windows-msvc\release\hive.exe hive-${{ github.ref_name }}-x86_64-pc-windows-msvc.zip
```

Until every layer builds, a non-release CI job running
`cargo check --locked --target x86_64-pc-windows-msvc -p <the leaves>` would
keep the done layer from regressing. That is the real-target counterpart of
`scripts/check-not-unix.sh --leaves`.

## Next steps

1. ~~hive-evolve: route `process.rs` through hive-cli `procs`, then gate
   `readonly.rs`/`clones.rs`.~~ Done in the second step.
2. hive-harness: ~~compile (Ctrl handler)~~ done in the second step;
   liveness (`OpenProcess`) still to port. The `bash` tool on Windows needs
   an owner decision (Git `sh.exe` vs PowerShell).
3. hive-cli `procs` on Windows: Job Objects. This needs `windows-sys`, which
   is already in `Cargo.lock` transitively. Adding it as a direct dependency
   is Tier 2.
4. hive-runner, file by file, starting with `service.rs` (Task Scheduler)
   and the `/bin/sh` sites.
5. hive-hub, then the binary. Then the CI job above.
