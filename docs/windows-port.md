# Windows port

Hive builds and ships for macOS and Linux today. On Windows it runs inside
WSL, the Linux that Windows runs, and that needs no port. This page is
about the other way: Hive on Windows itself, as a `hive.exe`.

Where that stands: the whole workspace compiles for
`x86_64-pc-windows-msvc`, and the `hive.exe` it gives starts, makes a store
and serves the console. It does not run work yet: the calls that start a
job, follow it and stop it are stubs off unix. The page lists every
unix-only site, crate by crate, with the Windows replacement for each, and
tracks how far the port has got. The work began as TASK-091
(`hive task show TASK-091`).

Rule for every step: unix behaviour stays exactly as it is. A Windows
replacement goes in a `#[cfg(windows)]` / `#[cfg(not(unix))]` sibling. The
unix code is not rewritten to suit Windows.

## How the survey was taken

The Windows standard library (`rustup target add x86_64-pc-windows-msvc`) was not
installed on the survey machine, and installing toolchain components is
outside a worker's remit. Two methods were used instead, and since the third
step a third one, the real target:

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
   and drops `nix`/`libc`, then runs `cargo check` on them with
   `-D warnings`, so an off-unix warning fails the check as an error would
   and cannot creep back into a layer that is done. Anything that
   fails there will fail on Windows. A pass is strong evidence, not proof:
   it does not exercise Windows-only std APIs or the third-party crates'
   own Windows builds. Those need the real target (see
   [Proposed CI](#proposed-ci-owner-owned)).

3. **The real target**, on one of GitHub's Windows machines
   (`windows-2025`, Rust 1.100): `cargo check --locked --workspace
   --all-targets`, then the programs built and started, then `cargo test`
   crate by crate ([What the real target says](#what-the-real-target-says)).
   It found two things the emulated compile could not. `hive-hand-core`,
   which every crate but three needs, was in no layer of the script, so the
   layers that passed there did not compile on Windows at all. And
   `rustyline` has its `Signal::Interrupt` on unix only, which the script
   cannot see, because it does not look into other crates.

```sh
sh scripts/check-not-unix.sh --leaves --all-targets   # the leaves
sh scripts/check-not-unix.sh --all-targets hive-evolve hive-harness  # the middle layer
sh scripts/check-not-unix.sh --all --all-targets   # every crate
```

The script copies only `Cargo.toml`, `Cargo.lock` and `crates/`, so it sets
`HIVE_HUB_SKIP_BUILD=1` and hive-hub embeds its placeholder page. Code that
names a Windows call (`std::os::windows`) cannot be compiled on a Mac or a
Linux machine, so such a site has three siblings, `cfg(unix)`,
`cfg(windows)` and `cfg(not(any(unix, windows)))`, and the script compiles
the third.

Third-party dependencies do not block the port: every one of them builds on
that machine as it comes, with `AWS_LC_SYS_PREBUILT_NASM=1` set so that
`aws-lc-sys` needs no NASM. `nix` builds there too, as a crate with nothing
in it, so a crate may keep it as a dependency and must gate every use.
`signal-hook` builds without its `iterator`.

## Status

| Layer | Crates | State |
| --- | --- | --- |
| Leaves | hive-gitenv, hive-hand-core, hive-hand, hive-cred, hive-store, hive-bus, hive-cli, hive-cred-setup, hive-cred-command, hive-provider, hive-provider-{anthropic,copilot,gemini,openai,fake}, hive-board, hive-tasks, hive-goals, hive-goals-command, hive-tally, hive-records, hive-codedelivery, hive-mcp, hive-voice, hive-hooks, hive-testutil | **Compile off unix**, libs and tests (`check-not-unix.sh --leaves --all-targets`, no warnings). Several only *compile*: behaviour still to port is listed below. |
| Middle | hive-evolve, hive-harness | **Compile off unix**, libs and tests (`check-not-unix.sh --all-targets hive-evolve hive-harness`), no warnings. |
| Top | hive-hub, hive-runner, hive-inspect, hive-ask, hive (the binary) | **Compile off unix** since the third step, libs and tests, no warnings. hive-runner holds most of the behaviour still to port. |

All 33 crates also compile on the real target
([What the real target says](#what-the-real-target-says)).

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

## Done in the third step

These changes make every crate compile off unix, and on Windows itself.
Unix code paths are unchanged. Each stub below is the careful answer for its
callers, and not the port: the port replaces it.

- **hive-hand-core** holds what hive-cli `procs` held (step one) and stands
  under every crate but three, so it is a leaf now, with hive-hand.
  Off unix: the device key and device file get no mode (`dial.rs`), the
  commit hook no exec bit (`local.rs`), the user's name is `USERNAME` and
  the host's `COMPUTERNAME` (`hands.rs`, `vault.rs`), and the free space of
  the worker's temp dir is not read, so it holds no run back
  (`tmpspace.rs`; `GetDiskFreeSpaceExW` later).
- **hive-runner**. The three modes of `keys.rs` go through one `set_mode`,
  a no-op off unix. `bulk.rs` makes its links through `symlink_dir` and
  `symlink_file`, which name the Windows calls on Windows. The backup's
  host name is `COMPUTERNAME`. The launchd arm of `service.rs` reads no uid
  off unix. Three stubs decide something:
  - `sweep::process_alive` says **alive** for any pid on record. Its
    callers ask before they take a lock from its holder or sweep a
    directory, so no lock is stolen and nothing is removed from under a
    process that may still run; what a crash left behind waits for its age
    rule. The verify lock is a link on unix and was a file written over
    off unix, which took the lock from any holder: it is now made only
    when none is there, and one left behind waits for a hand.
  - `hive daemon` takes a **fresh heartbeat for a live daemon**, as it
    cannot ask the pid, and so does not start a second daemon on one store.
  - The watchdog has **no SIGUSR1** to ask for a stack dump with. Its
    channel is kept open, because a closed one reads as ready for ever.
  The first Ctrl+C cancels the daemon through `signal_hook::flag` and a
  watcher thread, as in hive-evolve.
- **hive-hub**: the host name is `COMPUTERNAME`, the first Ctrl+C cancels
  through `signal_hook::flag`, and `build.rs` starts `npm.cmd` on Windows,
  where a batch file is only found under its full name. With that the
  console is built and embedded on Windows as it is elsewhere.
- **hive-harness**: the arm that reads rustyline's `Signal::Interrupt` is
  unix-only, as that signal is.
- **Tests**: those about a unix thing are `cfg(unix)`: links to a volume
  that is away, the launchd and systemd units, a signal sent to the
  launcher, files aged with `utimes`, the verify lock's link, the token
  read from descriptor 0, a git that hangs on a shell script, and the
  process tree tests of hive-hand-core. Helpers set a mode on unix only.
  The quiet worker of the heartbeat tests waits on a FIFO: off unix the
  helper that makes one stops the test and says why.
- **The check**: hive-hand-core and hive-hand joined the leaves, and
  `scripts/check-not-unix.sh --all --all-targets` passes.

Two stubs for one question now disagree, on purpose: hive-hand-core `procs`
says no process is alive (so a caller that waits for a group falls back to
`Child::kill`), and `sweep::process_alive` says every recorded one is. One
real answer, `OpenProcess` and `GetExitCodeProcess`, replaces both.

## What the real target says

One of GitHub's `windows-2025` machines, Rust 1.100, 24 November 2026, with
`HIVE_HUB_SKIP_BUILD=1` for the check and without it for the build.

| Asked | Before the third step | After it |
| --- | --- | --- |
| `cargo check --workspace`: crates that compile | 3 of 33 (hive-cred, hive-gitenv, hive-testutil); the other 30 waited for hive-hand-core | 33 of 33 |
| `cargo check --workspace --all-targets`: crates whose tests compile | 3 of 33 | 33 of 33 |
| `cargo build -p hive-hand`, then `hive-hand.exe --version` | did not build | prints the version |
| `cargo build -p hive` with the console, then `hive.exe version` | did not build | prints the version |
| `scripts/tests/smoke-installed.sh` on that `hive.exe` | | passes: it starts, makes a store and reads it, and its console answers on 127.0.0.1 |

So the store, the commands that read it and the console are there on
Windows. What is not there is everything that starts a process and looks
after it.

`cargo test`, crate by crate, in the 31 crates other than hive-runner and
hive-hub: 1,373 tests passed and 196 failed. The tests of 18 crates pass
whole (hive, hive-ask, hive-bus, hive-cli, hive-codedelivery, hive-gitenv,
hive-goals, hive-goals-command, hive-provider and its five providers,
hive-records, hive-tasks, hive-testutil, hive-voice). Most of the failures
are in hive-evolve (105 of 228), hive-harness (32 of 382), hive-store (19
of 145) and hive-cred (13 of 16). They have a few causes, and each is a
piece of the port:

- **Paths.** `canonicalize` gives Windows' long form, `\\?\C:\...`, and
  `git` and `sh` do not take it (`git init: cannot mkdir \\?\C:\...`).
  A path written the unix way, `/Users/me/key`, is not absolute on Windows,
  so `file://` references are refused.
- **The shell.** Fake programs in tests are `#!/bin/sh` scripts, and hooks,
  MCP servers and checks are started as such.
- **Modes and links.** Read-only trees, private fixtures and the clones'
  links in hive-evolve.
- **One failure takes its neighbours along.** Most tests of a crate hold
  one lock, and a test that panics leaves it poisoned, so a count of
  failures overstates the causes.

## Every unix-only site, by crate

Counts are grep hits (production / test). "Gated" means the site already sits
behind `cfg(unix)` with a non-unix fallback that compiles.

### Leaves (compile off unix now)

| Crate | Sites | What is unix-only | Windows replacement |
| --- | --- | --- | --- |
| hive-hand-core, hive-hand | gated | `procs.rs`, moved here from hive-cli (its row below still describes it); 0600 on the device key; `statvfs` for the temp dir's free space; `gethostname`; the passwd entry of the user; `df` and `ssh-keygen` as programs | Job Objects as for hive-cli. `GetDiskFreeSpaceExW`. `COMPUTERNAME` and `USERNAME` are read now. The key file needs the DACL helper of hive-cred: **security-relevant, decide before shipping**. `hostname -s` in `dial.rs` falls back to the name `hand` on Windows. |
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

### Middle (compile off unix now)

| Crate | Sites | What is unix-only | Windows replacement |
| --- | --- | --- | --- |
| hive-evolve | 36 / 17 | `process.rs`: `process_group`, `libc::killpg`, `setpriority` (nice), SIGKILL, signal-hook iterator, `MetadataExt`; `readonly.rs`: dev/ino identity and 0o444/0o555 read-only trees; `clones.rs`, `fleet.rs`, `genome.rs`, `commands.rs`, `heldout_auto.rs`, `measure.rs`, `usage.rs`: `symlink` and modes; `private.rs`: 0600/0700 | Process code goes through hive-cli `procs` (Job Objects) instead of its own `killpg`. Nice → `SetPriorityClass(BELOW_NORMAL_PRIORITY_CLASS)`. Identity: `GetFileInformationByHandle` (volume serial + file index) in place of dev/ino. Read-only: `Permissions::set_readonly` (file attribute) in place of mode bits. Symlinks: `std::os::windows::fs::symlink_dir/symlink_file`, which need Developer Mode or admin; fall back to directory junctions or copies. Compiles off unix since the second step. |
| hive-harness | 10 / 16 | `chat.rs`: signal-hook iterator; `conversation.rs`, `localenv.rs`: `libc::kill(pid, 0)` liveness and `ExitStatusExt`; `harness.rs`: tool scripts chmod; `buildenv.rs`: symlink; the `bash` tool runs `/bin/sh -c` | Liveness via `OpenProcess` + `GetExitCodeProcess == STILL_ACTIVE`. Ctrl handler as in hive-tasks. **The `bash` tool is the big product question**: the model writes POSIX shell. Options are Git for Windows' `sh.exe` (closest to today) or a PowerShell tool with its own name and description. That is a decision for the owner, not a mechanical port. Compiles off unix since the second step. |

### Top (compile off unix now)

| Crate | Sites | What is unix-only | Windows replacement |
| --- | --- | --- | --- |
| hive-runner | 77 / 85 | Spread over 40 files. **Service**: `service.rs` writes a launchd plist (macOS) or a systemd user unit (Linux) and runs `launchctl`. **Signals**: `commands.rs`, `daemon_watchdog.rs` (SIGUSR1 stack dump), `jobs.rs`, `util.rs`, `sweep.rs`, `proc.rs` use `kill`/SIGTERM and signal-hook. **Load**: `daemon_load.rs` reads `getloadavg`, Mach `host_statistics`, `vm_stat`, `pmset`, `setpriority`. **Disk**: `tmpspace.rs`, `bulk.rs` use `statvfs`, `df`, `du`, `dev()`. **Locks and identity**: `land_lock.rs`, `selfupdate.rs` compare dev/ino. **Permissions**: about 40 sites write 0600/0700/0755 (tokens, keys, scripts, hooks). **Symlinks**: `bulk.rs`, `verify_lock.rs`, `idle_hold.rs`, `buildcache.rs`, `recovery_api.rs`, `runners.rs`, `daemon.rs`. **Shell**: `/bin/sh -c` for every check, recipe and landing command (`exec.rs`, `executor.rs`, `proc.rs`, `landing_context.rs`, `service.rs`). **macOS tools**: `sign.rs` (`codesign`), `preflight.rs` (`xcode-select`, `xcodebuild`), `awake.rs` (`pmset`, caffeinate). **Users**: `hands.rs` uses `getuid` and `User`. | Compiles since the third step; this is the behaviour still to port. Service: `hive service install` → a Task Scheduler "at logon" task (`schtasks /Create /SC ONLOGON`, no admin needed) or a Windows service via `windows-service` (new dependency, Tier 2, needs admin). Recommend Task Scheduler first because it matches the per-user launchd agent. Signals: Job Objects from hive-cli; SIGUSR1 stack dump → a named event or a hub endpoint. Load: `GlobalMemoryStatusEx`, `GetSystemTimes` (no load average; derive one from CPU time). Disk: `GetDiskFreeSpaceExW`. Identity: volume serial + file index. Permissions: DACL helper (shared with hive-cred) or skip where the file sits under `%LOCALAPPDATA%`. Symlinks: junctions or copies. Shell: Git for Windows `sh.exe` located once and used for every `/bin/sh` site (same decision as the harness `bash` tool). macOS tools: already `cfg(target_os = "macos")` in most places, otherwise skip. Users: `GetUserNameW`, with `hands` (second-user isolation) unsupported on Windows at first. |
| hive-hub | 2 / 6 | `read.rs`: `nix::unistd::gethostname`; `register.rs`: signal-hook iterator; `server.rs`: `open`/`xdg-open`; tests: symlinks, chmod | Compiles since the third step, and serves the console on Windows. The host name is `COMPUTERNAME` now (`GetComputerNameExW` later, if the short name is not enough). Still to port: a Ctrl handler in place of the flag, and `cmd /C start "" URL` to open the browser (as hive-mcp already does). |
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

`.github/` is the owner's, so this page only proposes. There are two pieces.

**A check, now.** `cargo check --locked --workspace --all-targets` on a
Windows machine for every change under `crates/`, so that a unix-only call
without a sibling is seen the day it is made. It is the real-target
counterpart of `scripts/check-not-unix.sh`, and it sees into other crates,
which the script cannot. Proposed as `.github/workflows/windows-check.yml`:

```yaml
  check:
    runs-on: windows-2025
    env:
      HIVE_HUB_SKIP_BUILD: "1"          # the console is not what is checked
      AWS_LC_SYS_PREBUILT_NASM: "1"     # aws-lc-sys then needs no NASM
    steps:
      - uses: actions/checkout@v4
      - run: rustup toolchain install stable --profile minimal --no-self-update && rustup default stable
      - run: cargo check --locked --workspace --all-targets
```

It runs no test: too many do not pass yet
([What the real target says](#what-the-real-target-says)). A crate can be
added to a `cargo test` step there as its tests turn green.

**A release target, later.** A job in the release workflow beside the macOS
one, once work runs on Windows. A `hive.exe` that cannot run a job would
only mislead:

```yaml
  windows:
    runs-on: windows-2025
    env:
      AWS_LC_SYS_PREBUILT_NASM: "1"
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with: { node-version: 22 }      # the build exports the console
      - run: rustup toolchain install 1.95 --profile minimal --no-self-update && rustup default 1.95
      - run: cargo build --release --locked --bin hive
      - run: target\release\hive.exe version
      - run: Compress-Archive target\release\hive.exe hive-${{ github.ref_name }}-x86_64-pc-windows-msvc.zip
```

## Next steps

1. ~~hive-evolve: route `process.rs` through hive-cli `procs`, then gate
   `readonly.rs`/`clones.rs`.~~ Done in the second step.
2. ~~hive-harness: compile (Ctrl handler).~~ Done in the second step.
3. ~~hive-hand-core, hive-runner, hive-hub and the binary: compile.~~ Done
   in the third step. Every crate compiles on Windows, and `hive.exe`
   starts and serves the console.

What is left is behaviour. In the order that unblocks most:

4. **Paths.** One helper for the home (`USERPROFILE` where `HOME` is not
   set), and one that takes the `\\?\` form off a canonical path before it
   goes to `git` or a shell. Many of the tests that fail on Windows fail
   here. It needs no new dependency and no decision.
5. **Processes.** Job Objects in hive-hand-core `procs`, and one
   `process_alive` (`OpenProcess`) for `procs`, `sweep` and the daemon's
   heartbeat. This needs `windows-sys`, which is already in `Cargo.lock`
   transitively. Adding it as a direct dependency is Tier 2: an owner
   decision.
6. **The shell.** Every `/bin/sh -c` site (checks, recipes, landing
   commands, hooks) and the harness's `bash` tool. The choice is Git for
   Windows' `sh.exe`, which is closest to today and comes with the `git`
   Hive needs anyway, or a PowerShell tool with its own name and
   description. An owner decision, not a mechanical port.
7. **Key files.** An owner-only DACL where unix sets 0600, and an ACL check
   where unix refuses a key that others can read. Until then Windows skips
   both: security-relevant, decide before shipping.
8. **The service.** `hive service install` as a Task Scheduler task at
   logon.
9. Then the release target above, an install line for PowerShell, and the
   tests in the Windows check, crate by crate.
