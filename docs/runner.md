# The runner (how the machine works)

`pulse daemon` (the service) lands verified branches, plans jobs from the
tally, leases work to executors (this machine, paired Linux hands, or
provisioned compute), and runs the lead of every project so owner tasks and
messages get answers. `pulse work` runs one worker here; `pulse land`,
`pulse plan`, `pulse executor`, `pulse pair`, `pulse hands`, `pulse env` and
`pulse sweep` are the same pieces by hand. `pulse stop` / `pulse start` is the kill switch.

## The daemon loop

Use `pulse daemon --interval S` (or `pulse service install --interval S`) to
set the idle wait and the retry pause after an all-error batch. The default
is 300 seconds; zero or negative values use that default. Store events can
wake the loop early; job queue events (`job.add`, `job.queued`, `job.requeue`,
`job.recover`) wake dispatch, and an idle daemon also polls the active queued
count every 15 seconds so a missed event still starts work within one dispatch
tick without multiplying idle log lines. Cancellation stops the wait. Successful
runs and work still in flight keep their existing prompt-dispatch behavior.
The loop has two speeds: every tick (each 15 seconds while runs are in
flight, and whenever a run ends or an event wakes it) reaps, watches the stop
switch and a pending restart, starts approved compute and refills free slots;
the planning pass (the leads, tasks, amendments, recoveries and lessons of
every project, evolution, schedules, the planner, idle notes, executors and
the maintenance thread) runs at most once per `daemon.pass_seconds` (default
60), however often the loop wakes, since planning every fifteen seconds under
four agents cost a quarter of a core for jobs that only queued behind full
slots. Zero plans on every tick, and a shorter `--interval` caps the spacing.
The heartbeat the hub shows counts `loops` and `passes` apart, and carries
the body's own time as `tick` (the last body in milliseconds, the slowest
since the start and how many ran over the fifteen-second dispatch cadence):
`pulse status` prints it as `last tick 1200ms (slowest 41000ms, 2 over the
cadence)`. The daemon says `tick N took 40s, over the 15s dispatch cadence
(planning pass); queued work waited that long for a slot` once when a body
crosses the cadence and again only after one fits, so a slow pass is one
line, not one per tick; before this the heartbeat said when the loop beat
and how often, and a body that spent its cadence planning looked like a loop
that merely beat late. The line ends with where the time went - `; longest:
plan 14s, maintenance 9s, pipeline 3s`, the three longest stages of a second
or more, a per-project tender summed across the projects - and the heartbeat
carries the slowest body's stages as `tick.longest`, which `pulse status`
prints after the slowest time (`slowest 31000ms: plan 14000ms, pipeline
9000ms`): the number alone said that seven passes after one restart took
29-33 s each and nothing about which of the twenty tenders took them.

The pipeline stage reads GitHub as whichever gh account on the machine can
see the project's repository. `gh` keeps one login active per host and
ignores the others, and a machine whose active login was a fine-grained
token that saw none of three watched repositories, beside an account that
saw them all, spent 7.3 s of one tick on 404s and told the owner the
pipelines were unreadable. So when the active login's read of a push's
runs or of the branch head answers 401, 403, 404 or a repository name
GitHub cannot resolve, the stage tries each other account `gh auth status`
lists - read once and kept ten minutes, since the status check reaches
GitHub for every account (1.8 s measured) - running `gh` with that
account's token in `GH_TOKEN`. The first that answers is remembered for
the project (`gh_user` in its watch), read as first from then on, and named
once in the log (`pipeline <slug>: reading <repo> as <account>, since the
active gh login <name> cannot see it`); a remembered account that fails the
same way is forgotten and the accounts tried again. When none can see the
repository the stage leaves it alone for an hour (`unreadable_until` in the
watch), tells the owner once with the accounts it tried, and tries them
again when the hour is over. A branch that is not there is a 404 too but
says so, and no other account is tried for it. The tokens stay in the
process and never reach the log, the board or an event.

Worker selection and leasing agree on occupied capacity: one current lease
per agent, and a provisional completion does not free the worker or project
slot until its controller run ends. Workers are not kept idle: a dispatch
takes a free active worker when one exists (the holder of held work waiting
in the queue first) and otherwise spawns a fresh name, and each tick the
daemon retires every worker whose run has ended and that holds no lease, no
running run and no held conversation - its row turns `pruned` with the
reason `retired after its run; workers are not kept idle` and an
`agent.retire` event; the name is never reused. Leads and sessions are never
retired. A lease takes the owner's work first -
lead runs, answers and the jobs planned from owner tasks (source `task:...`) -
and only then the project that waited longest, by priority and age within it.
Project fairness includes lease reservations before the worker's run thread
records its start, so dispatch can spread work across projects without waiting
for their processes to launch. Without the owner-first key a six-project fleet
on four workers made an urgent task wait a full round of other projects'
improve jobs (ninety minutes, measured) before its first job leased. Between
the owner's work and the fairness comes the work class: the last half of the
local slots of this machine (rounded up: two of four) take a queued
improve, task or experiment job before a reflect, ballot or clean, and every
slot does once such a job has waited two hours (`WORK_WAITED_HOURS`) - the
meta kinds are many and short, and four slots cycling through them for every
project kept an improve job queued for a day while fairness saw each project
served; a kept slot still takes the meta kinds when no work is queued.
Configured concurrency, daily limits and lead priority remain unchanged.

## Services and self-update

`pulse service install` writes two units: `pulse.daemon` (the fleet) and
`pulse.hub` (the console on `127.0.0.1:8765`; `--hub-port N` moves it,
`--no-hub` skips it). Both restart themselves when a new `pulse` binary lands
on disk, so the console you keep open is always the build the daemon runs, and
neither dies with a terminal. Running `pulse service install` again is safe: a
unit whose file did not change and is loaded is left running.

That install line is for the first install only. After it, the daemon keeps
itself current (ADR-035): when a landing moves the brain's `main`, it rebuilds
`pulse` from that clean checkout, smoke-tests the new binary, installs it in
place and restarts once its runs in flight finish - the console follows within
a minute and shows the build it runs beside the daemon's under **Settings →
Running**. The restart is a handover, not a pause: a daemon whose binary
changed on disk while runs are in flight keeps dispatching, planning and
landing for up to `daemon.restart_drain_minutes` (default 30), stops starting
the fleet's runs of a kind on a project once less of that window is left than
such a run is expected to take (log line `restarting: the owner's work and
any run on blog, lamplitlabs; ballot, reflect, lead runs on amistio-template
start with <left> left of the restart window; N in flight` while the short
runs still fit, then `restarting: only the owner's work starts with <left>
left of the restart window; N in flight`, each said once per change rather
than per countdown tick; when no fleet run could fit the whole window the
first notice already says `only the owner's work starts`), and exits with
code 3 the moment nothing is in flight; its heartbeat says `restarting: N in
flight` meanwhile. The owner's work - the lead's sittings, answers, the jobs
planned from the owner's tasks, the security updates - is never held by the
window: it starts in every one, and on the express slot the load governor
keeps open inside one. The window is the drain or the time until the slowest
of the runs that were in flight when the binary changed is expected to end,
whichever is longer: the restart waits for those runs anyway, so a run that
ends before the slowest of them is expected to delays nothing, while
stopping at the end of the drain idled every project for as long as that one
run took (the first notice then says `new runs may start for up to <window>
while the slowest run in flight is expected to go on`). The fleet's runs
started inside the window do not extend it; an owner's run started inside
it does, since the restart waits for that run too, so the fleet's short
kinds keep starting while it runs rather than nine slots idling for its
length. Once the drain is over and no fleet run fits, the daemon still stays
while the owner's work is queued, so that work starts under this daemon
rather than waiting for a window the next one never has - for one more
drain at most since the stay began or the owner's last run started, since
work the slots cannot lease here (under a disk floor, behind a project's
marks, needing a hand this machine is not) would otherwise hold the restart
for good, and the next daemon starts it first thing; a console's own
**Restart** (below, without "now") still starts nothing, since the owner
asked for the restart itself. The measured reason: a task on the brain
takes two and a half hours against a twenty-minute drain, so the owner's
tasks never fit a window, and one filed in the console waited thirteen
hours with no attempt through a day of self-update restarts that refused
its kind. The expectation is measured per project and kind of job: the 90th
percentile of
the last fifty real runs of that kind on that project that ran a worker to an
end, plus five minutes of setup and report, capped at `minutes_per_job + 5`,
which also stands in until ten such runs exist (a project with fewer borrows
the measure of the kind over every project, a kind with fewer the measure of
all runs together). A ballot takes a minute and an improve run half an hour,
an improve run on the brain an hour and one on a blog five minutes, so one
measure for all either held the short runs for every window or admitted long
runs against the runs in flight's two-hour budget while those ended within
the half hour - one such run held nine idle slots for forty minutes at each
release. A run the window outlives is waited for, never cut. `0` stops
dispatching at once and waits for the runs, as before. The build itself is a
full release compile, so it keeps out of the runs' way: it runs at nice 10
with `cargo -j` set to `daemon.build_jobs` (half the machine's cores by
default), and it does not start while the load governor (below) says the
machine is saturated or every local slot of this machine holds a run -
the daemon then says `self-update: brain HEAD <rev> differs from the running
<rev>; building once the machine has room: <why>` once, keeps one local slot
free so the build starts within one run, and the experiment lane measures
nothing new meanwhile, as when the build waits for the lane. Nor does it start
until the brain's HEAD has stood unchanged for `daemon.self_update_settle_minutes`
(default 60; 0 builds at once): a fleet lands a burst of brain branches minutes
apart, and five release builds a day, each followed by a restart window that
refuses the long kinds of run, were the price of building on every landing -
the daemon says `building once it has stood for 60 min` once, and a further
landing restarts the clock, so the burst builds once, from its last commit. Nor
does it start while a fleet improve, task or experiment job is queued that the
restart the build ends in would refuse (its run is not expected to end before
the runs in flight): the daemon says `building once the queued improve runs on
<project> have started` once and lets them start first, for at most an hour.
The owner's queued work is never among those: the restart does not refuse
it, so the build does not wait for it.
A landing that touches nothing the binary is built from is not built at all:
the daemon diffs the brain's HEAD against the revision it runs, and when every
changed path lies outside the crates, the workspace `Cargo.toml` and
`Cargo.lock`, the toolchain and `.cargo/` files and the console's built output
under `web/hub/out/` (which pulse-hub embeds into the binary, so a console
build does change `pulse`), it says `self-update: brain HEAD <rev> differs
from the running <rev> only outside what the binary is built from (N file(s):
a, b, c and K more); no build` once and keeps the binary it has. The next
landing is diffed against the running revision again, not against that one,
so a change under `crates/` that follows a docs landing is built with both.
The measured reason: of the 39 landings on the brain's `main` over four days,
each a release build and a restart, three changed only docs, the README, a
script or the macOS app. A running binary that does not know its revision (an
unstamped build) or whose revision the checkout no longer has (rewritten
history) is built as before. The build is bounded by its silence rather than
its length: `cargo` is killed once it has printed nothing for ten minutes, and
the daemon says `self-update: build of <rev> failed: cargo produced no output
for 10m0s`. A bound on the length cut a full rebuild short (`cargo timed out
after 10m0s` in the log, after a Cargo.lock bump) and tried again an hour
later on the cached half, while a one-crate rebuild takes four minutes here,
three of them the final link of `pulse`, which prints nothing: a build that
still prints is still compiling. The smoke tests (`version`, `help`) answer
within a second and keep the ten-minute bound on their length.
A build that then exits three times early is put back to
the previous one by the launcher, which turns `daemon.self_update` off and
leaves a finding. A revision already installed by hand is not built again:
when `~/.pulse/bin/pulse version` prints HEAD, the daemon says so once and
restarts onto it after the runs in flight. Set `daemon.self_update` to `off` to keep the build you
have and `cargo install --path crates/pulse --root ~/.pulse --locked` by hand.

The owner need not wait for the drain. **Restart** (under **Settings →
Running** and beside the update notice in the Home page's runner controls;
`POST /api/v1/running/restart`) drains like a self-update by default
(**Restart when they finish**): nothing new starts, the runs in flight are
waited out, the daemon keeps its heartbeat and exits with code 3 once they
end or `daemon.restart_drain_minutes` is up (with nothing in flight it exits
at once). The dialog's **Hold the runs and restart now instead** (a payload
of `{"now": true}`) asks the daemon to restart at once with the
work held rather than cut: every run in flight is asked to hold, its harness
saves the agent's conversation and exits `held` (code 75), the run settles
`held`, and its job goes back to the queue with the attempt refunded and the
run's clone and push key kept. The next run of that job - under the same
agent, which the runner dispatches first once it is free and which the queue
waits for while that agent is active and free - picks
the conversation back up (`pulse harness run --resume`) with a note of where
it was interrupted, in the same clone, and the agent carries on instead of
starting over; the held session is marked `resumed`. A run that cannot hold
(a custom worker command, a run still in its recipe setup or already in its
landing check, a remote executor, an agent gone inactive) is stopped after
`daemon.hold_seconds` (default 120) and settles `lost`: its job returns to
the queue and starts over. A landing or measurement in flight gets the same
cap before it is cancelled; the next daemon lands or measures it again. The
hub does the same for its conversations: a console whose binary
changed stops the turns in flight with a note and the next console carries
each on by itself, so a chat mid-work needs no "continue" after an update
(a turn cut by a crash rather than a stop is left to the owner, so a turn
that brings the hub down cannot loop). A daemon older than this build does
not hear the ask and restarts once its runs finish, as before; the console
shows the ask as pending only while the running daemon started before it.
The same hold carries a run on that the step budget (`harness.max_steps`)
stopped with work in its clone, once per job, with a continuation note of
its own (see **Three budgets** under the harness).

A conversation whose turn a transport `provider_error` cut short (a failed
send or a broken reply body) resumes once by itself after
`chat.resume_after_provider_error_secs` (default 90; `0` leaves it idle for the
next message). `chat.sub_depth` (default 2) and `chat.sub_open` (default 8) cap
how deep sub-conversations nest and how many a conversation keeps open; `0`
lifts a cap.

An open conversation's agent holds a provider connection and one connection
or child process per MCP server. A conversation nobody has written to for
`chat.idle_release_minutes` (default 15; `0` never) lets its agent go: the
connections and child processes close, the trajectory stays on disk, and the
next message resumes the conversation from it, the way a hub restart does.
The binary also raises its own open-file limit at start to what the system
allows (launchd gives a service 256, a handful of conversations' worth); when
that falls short the hub's startup line says how many it has, and git
failing to start is logged instead of reading as a project missing from disk.

## Stall detection

If the owner's pause is on, **Resume daemon** clears only the `stop` setting
(`pulse start` in a terminal). It does not start a missing service, remove
file STOPs or change caps. **Silent** means the loop heartbeat is stale, not
that the daemon is disabled: it may still be running or waiting. A loop that
stays silent for half an hour (six intervals when the interval is longer) is
treated as hung: the daemon writes its own thread's stack and thread count to
`~/.pulse/logs/daemon-stall-<time>.log`, notes the stall in its heartbeat and
event log, cancels the runs in flight and exits 3, so the service starts it
again; `kill -USR1 <pid>` writes the same dump without a restart. The controls
show the recorded PID, service state and diagnostic commands rather than
starting a duplicate daemon. `daemon.self_update = off` only disables
automatic rebuilds; it is not the daemon's enable switch.

## macOS folder access

On macOS, a checkout under `Desktop`, `Documents` or `Downloads` makes the
system ask once whether `pulse` may access that folder the first time the
daemon touches it; answer Allow (the loop waits on the dialog, and `pulse
status` reports the daemon as silent until then). macOS ties that answer to
the binary's code-signing requirement, and a plain `cargo build`'s requirement
is its own hash - so every rebuild would ask again. `pulse service install`
therefore re-signs the installed `pulse` with a requirement every build meets,
`identifier "pulse"`, and the daemon signs each build it installs the same way:
no certificate, no password, and the answer holds. Checkouts elsewhere never
ask. The services run pulse through small `/bin/sh` launchers
(`~/.pulse/bin/pulse-daemon.sh`, `pulse-hub.sh`) so that a restart is a loop
and the crash guard has a home.

## Bulk storage on another disk

Most of what PULSE writes is regenerable bulk: the clones under `work/`, the
build caches, the worktrees, and the `target/` of a checkout that builds
itself. On a laptop with a small internal disk that bulk can live on an
external drive; the board and its history never do.

```sh
pulse bulk attach /Volumes/Kingston   # stop the service, move the bulk, link it, start again
pulse bulk status                     # mounted at /Volumes/Kingston/pulse (103201 MB free); ...
pulse bulk detach                     # bring everything back onto this disk
pulse bulk detach --lost              # the drive is gone for good: start with empty dirs
```

`attach` formats nothing. It takes a mounted volume (a plain folder only with
`--any-folder`), creates `<volume>/pulse` with a marker file carrying an id
that `~/.pulse/bulk.id` repeats, moves `work`, `cache` and `worktrees`
there, and leaves one symlink behind - `~/.pulse/bulk` - that the three names
go through (`work -> bulk/work`). It also makes `<volume>/pulse/cargo-target/`
for a checkout's build dir: `ln -s ~/.pulse/bulk/cargo-target/<name>
<checkout>/target` keeps a self-building brain off the internal disk too.
Everything else stays home: `pulse.sqlite` and its backups, patches,
measurements, the harness, the services' logs and launchers, and `tmp/` with
the short-lived credential files - a removable disk has no reliable ownership
bits, so secrets never go there. `attach` and `detach` refuse while a daemon
runs outside the service, stop the service for the move and start it again;
`--dry-run` lists what would move.

Pulling the drive, or logging in without it, is safe at any moment and needs
no `launchctl` first. The link simply dangles, and a dangling link creates
nothing: `mkdir -p`, `git clone` and every `create_dir_all` fail on it instead
of filling the internal disk. The daemon notices before each pass, holds every
dispatch, landing, sweep, reclaim, experiment lane and self-update with one
line - `idle: bulk storage away: /Volumes/Kingston/pulse is not mounted; all
work waits for the disk` - and the console shows the same hold in red on the
Overview with the one remedy, plug it back in. It never exits, so the service's
crash guard never mistakes the missing disk for a bad build and rolls the binary
back. A run that the disk vanished under ends as an executor fault: the job
keeps its attempt and runs again once the disk is back. When the volume returns
the next pass logs `bulk storage back; resuming` and work continues; when
macOS mounts it under a new name (`Kingston 1`) the daemon finds the marker on
the sibling volume and repoints the link itself. `pulse status` prints a
`bulk` line while anything is attached, and `pulse executor probe` names the
missing disk instead of measuring a 0 MB floor.

## Backups of the store

The board, its history, every task and every job live in one file,
`~/.pulse/pulse.sqlite`, and nothing above regenerates it. When a disk starts
failing the only copy must already be somewhere else, so the runner can keep
one there. Name a place with `backup.to` and the maintenance pass copies the
store to it every `backup.every_hours` (24 by default), keeping the newest
`backup.keep` copies (14) and removing older ones of this host's:

```bash
pulse settings set backup.to az://myaccount/pulse-backups/store   # an Azure blob container, with an optional prefix
pulse settings set backup.to /Volumes/Backup/pulse                 # a folder that already exists (a mounted disk, a synced folder)
pulse settings set backup.to 'run:rclone copyto {file} remote:pulse/{name}'   # any command; {file} and {name} are filled in
pulse backup                       # one now, to backup.to
pulse backup --to ~/Desktop --keep 3   # one now, somewhere else, pruning to 3 there
pulse backup --status              # where copies go, the last outcome, when the next is due (also --json)
```

Each copy is a `VACUUM INTO` snapshot (writers are never blocked), checked with
`PRAGMA quick_check` before it leaves the machine, gzipped, hashed, and named
`pulse-<host>-<stamp>.sqlite.gz`; the folder destination writes a `.part` file
and renames it only after the size matches, so a half-written copy is never
mistaken for a whole one. A folder destination must already exist: a missing
mount must never turn into a new folder on the boot disk. The Azure destination
uses the `az` CLI you are logged in to; it fetches the account key for the
upload and passes it to `az` through the environment only, never on a command
line, in the store, or in a log, and falls back to `--auth-mode login` when the
key is not readable. When the account lives in a subscription other than the
CLI's default, name it with `backup.subscription` (name or id): every `az`
call then carries `--subscription`, so the key is found and the login's token
is cut for that tenant. The `run:` destination gets no pruning - the command owns
its remote. The outcome (size, digest, location, what was pruned, or the error)
is recorded as `backup.done`/`backup.failed` on the bus and under the Cleanup
panel in the console; a failed backup is tried again within the hour regardless
of the cadence. The Settings page has the three caps under **Backups**.

## Keeping the machine awake

A laptop that dozes off takes its agents with it: a run that worked for an
hour loses the whole hour when the lid closes or the idle timer fires, the
timeline shows a gap, and the trends dip. `daemon.keep_awake` is PULSE's own
caffeination mode. `off` (the default) leaves the power settings alone. `on`
holds the machine awake while the daemon runs and the machine is plugged in;
`always` holds it on battery too. The hold is a child of the daemon - on
macOS `caffeinate -d -i -s -w <daemon pid>`, the same assertions Copilot
CLI's `/keep-alive` takes (`-d` keeps the display awake, `-i` stops idle
sleep, `-s` stops system sleep on AC power); on Linux a `systemd-inhibit
--what=sleep:idle --mode=block` that waits on the daemon's pid - so it ends
the moment the daemon exits, is paused with `pulse stop`, or restarts under a
new build, and it never outlives PULSE. A closed lid still sleeps a Mac
unless an external display is attached; the daemon's log line says so
(`keeping this machine awake while PULSE runs (caffeinate -d -i -s -w <pid>,
pid …), while plugged in; the display stays on and the screen saver stays
off; closing the lid still sleeps a Mac unless an external display is
attached`). When the holder cannot start (no `caffeinate`, no systemd) the
daemon logs why and tries again ten minutes later instead of giving up.

The daemon reads the power source every twenty seconds while the mode is on
(`pmset -g batt` on macOS, `/sys/class/power_supply` on Linux). Because `-d`
and `-i` would keep a battery awake, `on` on a Mac running on battery holds
nothing at all until the charger is back - the log says `keep-awake: on
battery, so the hold waits for the charger (daemon.keep_awake=always holds on
battery too)`, then `keep-awake: plugged in again; taking the hold` and the
usual holding line when the cable returns. The heartbeat carries `power`
(`ac`, `battery`, `unknown`), `effective` and `waiting` next to the mode, so
the console says "Staying awake" only when the hold is in force and "Awake
once plugged in" while it waits. `always` keeps the display on while on
battery too, which is exactly the drain `on` avoids.

The screen saver stays off while the hold is in force. macOS starts the saver
from the Lock Screen idle timer, but before launching it the screen-saver
daemon asks power management whether idle display sleep is blocked
(`PMDisplaySleepIsBlocked`), and `caffeinate -d` is that block: the system
log reads `IS blocked, by assertionType: PreventUserIdleDisplaySleep` …
`do not launch screen saver`. A hold without `-d` - the plain `caffeinate -s`
PULSE used before - stopped sleep but let the saver come on every three
minutes while the agents worked behind it. With the hold waiting for the
charger or the setting off, the display rests as the Lock Screen settings
say.

The daemon also notices when the machine slept under it anyway - the wall
clock jumps while the monotonic clock does not - and writes a `daemon.slept`
event with the gap and the runs that were in flight. The heartbeat carries
`awake` (mode, whether the hold is held and effective or waiting, since when,
how, the power source, the display and lid notes) and `slept`;
`/api/v1/runner` and `/api/v1/running` pass them to the console. The setting
lives under Settings → This machine → Staying awake; the Running strip shows
"Staying awake" and "Last sleep"; the home page adds a line under the
headline. While the hold is held a thin amber line runs along the top of the
console and a steaming cup sits in the shell's footer and header - the
palette itself does not change - so a glance says the machine is staying up
for its agents. Pip, the mascot on the home page, works at a laptop instead
of waving: typing when the daemon is up, typing with a coffee in hand while
the hold is in force, and with the laptop on its charger when the machine is
plugged in as well; a hold that waits for the charger shows the plain laptop
and the "Awake once plugged in" label.
