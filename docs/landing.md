# Landing, pipelines, dependencies and credentials

## Workers, landing and the load governor

Workers commit in their own clones and never push; the runner copies each job's
commits to a `pulse/<agent>/<job>` branch in your checkout, and the daemon
lands them into the default branch (ADR-024: verify command first, fast-forward
or a `Land …` merge commit).

A merge that conflicts is not resolved for you: the branch's patch is saved
under `~/.pulse/patches/<slug>/`, the branch is removed, the job is requeued on
the current default branch (`land.conflict` names the patch) and the run is
made again on the moved base. One conflict is an exception, because it is not
a conflict between anyone's sources: the console's generated export under
`web/hub/out/`, which every build of the hub regenerates with content-hashed
chunk names, so two branches that both touched the hub's source always
collided there while the sources merged clean. Of the nine amistio-template
landings that conflicted on 2026-11-02..04, every patch was 80-90%
`web/hub/out`, and each one threw away a finished run and ran its job again
for a conflict nobody wrote. When every conflicted path lies under
`web/hub/out/`, the daemon drops the export from the merge, runs `npm ci` and
`npm run build` in the landing worktree on the merged `web/hub/src`, stages
the rebuilt export and finishes the `Land …` merge commit with it; the landing
line and the `land.landed` event say `web/hub/out rebuilt from the merged src`
(`hub_out_rebuilt`). A build that fails, or a conflict that reaches any other
path, holds as before. The rebuilt bundle's stamp names the merged src tree,
so the bundle test the verify command runs still passes.

### The verify command

The verify command is the project's `verify_cmd` (`pulse init PATH --verify
CMD` sets or replaces it; re-registering keeps everything else); a project
registered without one adopts its recipe's `suggested_check` the first time the
daemon or a run reads a recipe that parses - once, with a `finding` on the
project's `runner/verify` thread and a `verify.adopted` event - so a project is
never left landing unchecked work because nobody passed `--verify` (`pulse env
status` shows both when they differ, and an owner's own command is never
replaced; before this, seven of eleven projects on one machine had no verify
command and landed every job unchecked, three of them with a `make test`
committed in their recipe that the daemon never ran).

The verify command runs in a throwaway worktree that starts bare, so the daemon
first runs the project's setup there - the recipe's `setup:` lines from the
default branch, else the lockfile rules (`corepack pnpm install`, `uv sync`,
`npm ci`, …) that a worker clone gets - within ten minutes (`SETUP_MINUTES`,
the budget every setup gets, whatever `minutes_per_job` says: a hung `pnpm
install` once held a slot for the two hours a job may run); a setup that fails
on the machine defers the landing with a `land.deferred` event naming the file
and the failure, and nothing is charged to the branch.

A setup that fails that way at the base - not a timeout, not a dead network or
a full disk, judged on the output's last lines where the error is, so that
npm's `could not resolve dependency` peer note above a lockfile error never
passes for a DNS failure and the unfit mark and the repair read the same
evidence - is a fault of the default branch itself that no retry and no other
job can get past: every landing of the project runs the same setup first, and
every worker clone does too.

So the daemon plans the fix (`landing.setup_repair`, default `on`): one Tier 1
`task` job with source `setup:SLUG` (the `land.deferred` detail carries its id
as `repair`, the hold note says `repair job ID planned`, and a `setup.repair`
event records the command, the exit code and the last lines), briefed with the
base commit, the recipe file, the command and the output, and told the three
ways such a setup breaks and how to prove each fixed: the recipe names paths or
tools the tree no longer has (`pulse env set --project SLUG --file …`, and
`pulse init PATH --verify` for a verify command that moved with them), a
lockfile out of step with its manifest (regenerated only at the versions the
manifest already pins, since the landing's release-age rule reads every new
version it adds), or the tree itself. That job is the one job of the project
the setup is not run for - the runner says so in its log and the brief's rule 3
tells the worker to install only what the setup would have - and it is leased
even while the project is marked unfit for the machine, which holds everything
else of the project back. The same failure seen in a worker clone plans it too,
at the point the second same-signature failure marks the project unfit, so a
project frozen on every machine by a broken `npm ci` no longer waits a day per
machine for marks to expire - unless the checkout is behind its upstream (the
`origin/main` the last push pass fetched, `origin/pr` in PR mode): then no
repair is planned and the hold note says the sync lands first, because the
recipe may already describe upstream's tree, set by hand after the owner
renamed a solution, and a worker sent to fix it at the stale head would write
the old paths back; the sync's own setup failure, whose base is upstream's tip,
still plans one.

Three repairs a day per project at most; a failing repair stays open and is
briefed afresh at the next failure rather than queued again; and the first
landing whose setup passes on the machine clears that machine's `setup:` mark
for the project at once (an `executor.fit` event), so the held work follows the
repair out. Before this, a project whose `server/package-lock.json` had drifted
from its manifest sat frozen on the hub for a day behind an `npm ci` that no
job was allowed to fix, and a pnpm workspace whose lockfile no longer matched
its `overrides` held every landing the same way.

### Workers and paired hands

Each worker is one `pulse harness run` process (the daemon's own binary here;
on a paired hand the shipped `pulse-hands` binary, which `/usr/local/bin/pulse`
links to) given the brief, the clone, the limits and the model from Settings,
one minute less than `minutes_per_job` so it can stop with a report of its own,
and the run id as its session; its stdout is the report the runner reads. A
paired hand needs the same sign-in once: the native harness reuses the brain's
stored provider login during heal/provision (a machine the daemon buys is
paired first and logged in right after, before the probe that makes it
active), and `pulse executor login NAME
--admin admin@host` copies that login to the hand's `pulse` user
(`--with-token` still reads a token from stdin; `--remove` forgets it). The
hand's probe reports `harness=yes|no` and the `pulse` build it runs. The probe
also refuses a hand whose workdir filesystem has under `daemon.min_free_mb`
(default 10240, ten gigabytes: a build and the clones beside it, since a run
that hits ENOSPC mid-build is lost work) megabytes free and names that path in
its note; 0 turns the check off. A hand that was drained but never active
(added with `pulse executor add`, registered in the console, or paired while
the host was still booting) becomes active on its first passing probe,
including the daemon's own; a hand drained by the owner, by errors or by the
login gate still needs a human probe.

### Disk: the floor and the reclaim ladder

The daemon checks the same floor before it leases new work or starts an
experiment measurement, so a full work filesystem pauses dispatch with `disk
under floor: <free> MB free on <path> (daemon.min_free_mb <floor>)`; after
fifteen minutes it writes one runner finding that says to free space or adjust
the floor. It does not only wait for you: while the disk stays under the floor,
each maintenance pass (at most every thirty minutes) climbs a reclaim ladder
(`crates/pulse-runner/src/reclaim.rs`) and stops at the first rung that puts
the free space back over the floor. Rung one removes what the runner's own
tools left in the OS temp directory (`TMPDIR`, and `/tmp` when that is a
different place): Rust `tempfile` directories (`.tmpXXXXXX`) and
`pulse-*`/`check-site-*`/`distill-ui-*` folders that nothing has written to for
two hours and no live process has its working directory or executable
under - on 2026-10-12 one laptop carried 105,774 of them, 19.7 GB, the test
homes of `cargo test` runs the step deadline had killed before their
destructors ran. Rung two runs the sweep in pressure mode: finished job work
goes at once instead of after its hour of grace, work the store does not know
and run logs after a day instead of the configured keep, build-cache lanes idle
for an hour instead of `daemon.build_cache_keep_days`, and the store gives its
free pages back past 64 MB instead of `daemon.vacuum_free_mb`; the things the
sweep never touches stay untouched under pressure too. A floor still not clear
after both rungs is reported with the five largest entries under the temp roots
and `~/.pulse`, so the owner knows what to look at. Each pass that removed or
listed something writes one `daemon.reclaim` event, and the disk-floor finding
carries the last pass: what it freed, over which rungs, and what is left.
`pulse sweep --reclaim [--dry-run]` climbs every rung by hand, floor or not.

The leak itself is closed at the source: a local run's worker, its pre-commit
hook and its run-end check get `TMPDIR` set to `<clone>.tmp` beside the clone
(a recipe that sets `TMPDIR` wins), and a landing's commands get
`<worktree>.tmp`, so what a killed test leaves behind goes with the clone or
the worktree instead of piling up in the system temp directory.

### Disk headroom per run

The floor alone cannot see that the ten setups one tick is about to start each
need a gigabyte (on 2026-10-02 a restart with a ten-job backlog did exactly
that on 1.1 GB of headroom: every clone and `npm install` ended in ENOSPC, the
failures counted as transient, and the same jobs failed the same way after each
five-minute pause, fifty-four times in an hour), so the daemon also measures
the work directory of every local run as it ends (`runs.work_mb`) and, each
tick, lets a local slot take only a project whose footprint the free space
above the floor still pays for - the largest of the project's last ten measured
local runs, or `daemon.setup_mb` (default 1024) until a project's runs have
measured it. Each start claims its project's footprint, a run still in setup
(no brief written yet) holds its footprint until its brief exists, and a
project the rest no longer pays for waits while a smaller one starts (the first
hour of the admission held every slot behind the fleet's largest footprint:
1455 MB of room and nothing started, while four of five projects took under 900
MB). One run is always admitted while none is in flight so a small disk still
measures itself, and a slot held back this way leaves the queue as short of
hands as a busy one (below).

A held project shows in the idle reason as `N queued wait: N for <project>
(local needs M MB of disk)`, and a slot no project fits says `disk headroom: R
MB above daemon.min_free_mb F with K run(s) setting up; waits: <project> (M
MB), ...`; `daemon.setup_mb` 0 turns the admission off. Hands keep their own
disks.

### The load governor

The machine itself is governed the same way
(`crates/pulse-runner/src/daemon_load.rs`, sized by `capacity.rs`): a 12-core
16 GB laptop once told to run twelve local slots leased seven jobs within a few
seconds and ran their `npm ci`, test suites and `cargo test` builds at once, at a
load average of forty with 2.3 GB of its 3 GB swap in use. There is no slot
setting any more: every machine works out its own slots from its hardware - the
smaller of its physical cores and one run per 3 GB of RAM, never under one (a
12-core 16 GB laptop: five; a 32-core 64 GB box: twenty-one) - and each paired
machine does the same from the cores and memory its hands report on every
probe. The fleet's total is the sum over the active machines, and a project may
have `ceil(total / projects with work)` jobs in flight, so a lone busy project
uses every slot and several share them fairly; `pulse status` shows it as
`slots 18 = this machine 10 + vm-1 8, per project 6`. The one optional override is
`capacity` (auto by default; a number caps this machine, 0 keeps it idle), and
`pulse daemon --max-jobs N` overrides both for one daemon; `pulse stop` pauses.
The startup line `load governor: up to N local run(s) at once
(auto|capacity|--max-jobs; C cores, P CPUs, M MB), nothing new over load L or
under F MB of memory free; children run at nice 10, builds at -jJ, the daemon
itself at nice 5` names what is in force.

Before leasing it also reads the one-minute load average and the memory a new
run could use (free, inactive, speculative and purgeable pages on macOS - the
caches the kernel drops on request, a gigabyte or two on a 16 GB laptop whose
`free` reads a few hundred megabytes - `MemAvailable` on Linux) and leases
nothing new while the load is over the load line (twice the number
of CPUs - the load average counts runnable threads, and a laptop with an
emulator or a browser build sits at its CPU count for hours while its CPUs are
not busy; a line at the CPU count held the fleet all day) or the memory is
under 1024 MB (both internal constants, not settings), or while macOS itself
reports critical memory pressure; the runs in flight finish, the idle line says
`machine saturated: load 40.1 on 12 CPUs is over the line 24` or
`machine saturated: 512 MB of memory free, under the floor 1024`
once per change rather than per tick. A queue behind held slots is as short of
hands as one behind busy slots: once an improve or task job has waited half an
hour that way, the daemon rents a machine itself when `compute.auto_approve` is
on and `compute.max_vms` and `compute.max_hours_per_day` allow (one open
request at a time, four hours, `AUTO_COMPUTE_HOURS`), and otherwise posts
`hands wanted` to the owner once, saying why no machine was rented.

The kernel's memory warning (the level under critical, which a 16 GB laptop
reports for hours with a browser open) is not a hold: while it stands the
daemon starts half as many local runs at once (the machine's slots halved, never
under one), and the experiment lane's measurements in progress count as runs
under the cap, since their builds are load like any other.

### The express lane for the owner's work

A load hold alone (not memory, not a restart window) keeps one local slot open
as the express lane: it leases only the owner's work - lead runs, answers,
`task:` jobs - while the load stays under twice the load line and the run
cap has room; the idle line adds `one express slot stays open for the owner's
work`. The fleet's own builds and landings are what push the load over the
line, and without the lane they held the owner's tasks behind themselves. The
cap itself is the other way they did it: with two local slots and both
slots on fleet verifies, the lead sitting your *Try again another way* queued
waited forty minutes for a slot. So when every slot under the cap is busy with
the fleet's work, none of it is the owner's, no slot is reserved for a restart
and the machine is not hard-pressed (memory, the floor, twice the load line),
the daemon lends one slot over the cap to the owner's work alone - one at a
time, never while an owner run is already in flight - with the line `every slot of this machine is busy
with the fleet's work; one express slot stays open for the owner's work`.

A free slot is not a free lane, though: a project's own work lanes
(the fleet's per-project share, narrowed to what fits the commit cap while its checks
take turns) count the fleet's runs and the owner's alike, so an owner's task
queued behind one long fleet `improve` job in its project waited four hours
while the daemon leased fifty runs in other projects around it. Now the owner's
work goes first in its own project: while an owner-driven work job (`task:`,
`security:`) is queued in a project, the fleet's work jobs of that project stay
in the queue, and when every lane the owner's job needs is held and some by the
fleet's runs, the daemon asks the youngest of those runs to hold at its current
step - the same hold the restart uses: the clone and its commits stay, the job
goes back to the queue costing it no attempt, and the same agent carries it on
from its harness session once the owner's run has a lane. The daemon says
`<project>: <job> (improve) holds after its current step so the owner's <job>
can start; it carries on later`, writes a `job.yield` event on the fleet job
(`project`, `run`, `to`), and the held run's summary reads `held so the owner's
work could start first after N steps`. The owner's own runs are never held, and
a project whose default branch is red is left alone: the owner's task waits
behind the fix job there and holding the fix would only lengthen the wait.

The narrowed lanes are the fleet's, not the owner's. On the brain the check
takes twenty to twenty-seven minutes against a forty-minute commit cap, so one
lane of five is open, and the jobs the lead planned from the owner's tasks -
one and a half to two and a half hours each - serialised behind each other
through that one lane: four of them sat queued ten to thirteen hours with no
attempt, and the yield could not help, since the run in the lane was the
owner's own. So the owner's work (a lead's sitting, an answer, the jobs planned
from the owner's tasks, `task:…`, the security updates) takes a lane beyond the
narrowed count at once, bounded by the plain `parallel_per_project` alone, and
its run's commit cap is widened to cover the checks it shares the lock with -
`n × the verify median + harness.step_timeout` for `n` work runs in the
project, never below the configured cap; with the default five-minute step the
run log says `commit cap widened to 51 min: 2 checks share the verify lock on
amistio-template (verify ~23 min each)`. The fleet's work keeps the narrowed
lanes, and a fleet run in the owner's way is still asked to hold, so the lanes
return to their count and its own commit is not killed waiting on the owner's
check. The owner's work fits every restart window too (the window names the
kinds expected to end before the runs in flight, a task on the brain never was
one, and the same tasks waited through seven windows in a day; see
[runner.md](runner.md)).

The lead's own sitting is outside its project's count altogether. A sitting
reads and posts for a minute, plans the owner's work and directs the fleet's;
with a per-project share of 2 and two long fleet `improve` runs in the project,
the sitting an owner's *continue* queued once waited 54 minutes for a lane the
express slot had offered it every tick. Now a project lead's sitting leases
whatever the project's lanes hold (at most one short run over the limit: one
lead leases one job at a time); a role lead's standing sitting (`lead-role:`)
is the fleet's work and keeps the count.

### Nice, build jobs and probes

Remote hands carry their own load and are not capped by this machine's
governor. Every run the daemon starts here - the worker, its `npm ci`, its test
suite, the clone's removal - runs at nice 10, so the owner's own work and the
daemon's loop stay responsive while the fleet works. Nice keeps the machine
responsive, not cool: four runs each compiling on all twelve cores still ran
the laptop hot for most of a day, so the daemon also exports
`CARGO_BUILD_JOBS=<daemon.build_jobs>` (default 0: half the physical cores) to
everything it spawns - a worker's `cargo test`, the pre-commit verification
that inherits the worker's environment, the landing verification, the
self-update build's `-j`, the experiment lane's measurements (each fixture's
share of the cores, `evolve.measure_parallel`, never over this cap) - whenever
the environment does not set it already, so an owner's own export or a recipe's
`env:` wins and a `-j` on a command line still overrides it. Other toolchains
read their own variables (`GOMAXPROCS`, `MAKEFLAGS`); a recipe sets those where
a project needs them.

The daemon itself lowers its own priority to `daemon.nice` (default 5) as it
starts, so its planning, sweeps and landing passes yield to the owner's
foreground work while the loop still comes before the fleet's builds; 0 leaves
the level the service started it at, and a `renice` by hand does not survive
the restart a self-update brings, which is why the daemon does it itself.

`pulse executor probe local` probes this machine the same way (git, free space
under `~/.pulse/work`), and a passing probe run by the owner clears the
projects a failed setup marked unfit here, instead of waiting a day for the
marks to expire. While queued work waits behind an unfit mark, the maintenance
pass also re-probes after `daemon.unfit_reprobe_minutes` (default 60; 0 off),
then doubles the wait for that executor/project/tool up to one day; a passing
daemon probe clears only that project's mark. A mark about a tool (`lacks
node@22`, `lacks bundle`, `lacks mise`) is cleared only once the tool
answers - the probe runs `mise where PIN` or `TOOL --version` on the machine
first (on remote hands through their `tool` verb; older hands are cleared as
before), since a probe that looked only at git and disk sent the jobs back into
the same failing setup every hour; while the tool is still missing the mark
stays, the re-probe wait doubles as after a failed probe, and the mark expires
a day after the setup failed, so the setup gets one real try a day. A pin the
recipe no longer lists is cleared whether or not it is installed.

Remote hands take a recipe's `tools:` and `env:` the way the hub does - the
worker runs under `mise exec PIN… --` with the recipe's env set, after the
`setup:` lines ran there - once their probe answers `recipe=1` (hands shipped
since this landed do; `pulse pair` refreshes older ones, then `pulse executor
probe NAME`); until then such a project runs on the hub only, as before, which
kept five of eight projects off the hands.

A setup that fails twice the same way marks the project unfit for that executor
and plans the setup repair job described under landing, which that mark lets
through; a failure the machine did not cause - a dead network, a full disk (`no
space left on device`, `disk quota exceeded`) - never does, and the job simply
goes back to the queue. While a mark holds queued jobs, the daemon's idle line
says so (`no queued job; 3 queued wait: 3 for blog (local cannot set up
Gemfile.lock); …`) with the probe command, instead of a bare `no queued job`
over a queue nobody can see. The login check the daemon runs on an executor is
a two-step harness run that must answer `SUMMARY: ok`.

### Landing beside your checkout

While the checkout is on another branch the landing does not touch it: the
merge is verified in the throwaway worktree as always and the default branch
then moves by ref (a compare-and-swap `update-ref` with the same reflog
receipt, so the code delivery evidence is unchanged); your branch, your edits
and your stash stay as you left them, and the next worker clone starts from the
landed commit. Only a second worktree that has the default branch checked out
makes the landing wait, naming that worktree. When the checkout is on the
default branch and has uncommitted changes, `landing.auto_stash` (default `on`)
stashes them with a clear message so the landing can proceed, and a stash that
cannot be made blocks the landing with the reason instead of silently
deferring. Your work in progress is never committed or lost.

A verify failure on the merged result is charged to the branch (patch saved,
job requeued, branch removed) only when the default branch passes the same
command in the same worktree; when the default branch is red itself - a tool
gone from the machine, a broken dependency, a suite that fails on its own - the
branch is kept, the project's landings wait `landing.base_red_retry_minutes`
(default 10 minutes), the hold names the failing line and logs `retry in 10m`,
a `land.base-red` event records the tail, and `pulse land --project X` retries
at once after you fixed it - the branches and the push step's sync of the
upstream alike, which reads the same hold (before this, `pulse land` by hand on
a project whose setup had failed landed nothing, reported nothing waiting, and
left the sync behind the hold its own note said the command lifts). The hold
shows in `pulse status` and the hub as `landing` (how many branches wait and
why), and the daemon logs it and writes `land.held` once per change,
`land.resumed` when the branches land again.

When the branch is charged, the check's whole stdout and stderr are kept next
to its patch as `~/.pulse/patches/<slug>/<branch>.verify.log`, the
`land.failed-verify` event carries that path (`log`), the failing lines
(`failures`) and the last 1500 characters, and the command's own line names
the first three failures: `verify exit 1 on the merged main; branch removed;
patch at …; the check's whole output at …verify.log (failed: error: test
failed, to rerun pass …)`. Before the tail is cut, every command a tool echoed
in backticks is shortened to its first word: cargo repeats the whole `rustc`
invocation of a crate whose build or test binary failed, several thousand
characters that pushed the `error: test failed` line out of the tail, so the
event said `failures: []` and the agent fixing the branch retried blind.

### A red default branch

A red default branch is also a plan, not just a hold: the first run or landing
that finds the base itself failing the verify command records it (`base.red`
event, one `finding` on the project's `runner/verify` thread with the check's
tail), queues one `improve` job to make the check pass - source
`redbase:<slug>`, priority above the planner's improve jobs, a brief that
carries the command, the exit code and the failing lines and forbids loosening
or skipping the check; a corrected job keeps the source if it fails - and holds
the project's other landing work (improve, task, experiment and clean jobs)
with its attempts intact until the default branch moves; research, ballots,
answers and leads keep running, and the idle line counts the held jobs (`2 jobs
of blog wait for a green default branch (3f9c… is red; fix job … leased)`).

A check that did not run to a verdict - the disk under `daemon.min_free_mb`
(exit 71), the verify lock given up (exit 75), a timeout, a worktree that could
not be made - says nothing about either tree: the run or landing waits as
before, but the base is not recorded red and the branch is not charged, and the
base tree is not checked again when the job's own check could not run.

The hold lifts on the tick after the default branch moves (`base.moved`),
whoever moved it, and when the fix job ends with no commit and the daemon's own
check of the default branch as it stands passes (`base.green`, a finding on the
same thread; the red was never the tree's own, or was fixed without moving the
branch); if half an hour passes with no fix job open and the base still red,
the daemon asks the owner once (`default branch main is still red at …`, a
question marked for the owner) with the two ways out - land a fix by hand, or
give the project a check it can pass with `pulse init PATH --verify CMD` - and
answers its own question when the branch moves or the hold lifts. In the week
before this landed, 42 runs across 40 jobs on one machine each spent a worker
to learn the same thing and threw their commits away.

A branch that changes `docs/context/environment.md` alone is yours to land
(ADR-026): the daemon holds it, asks once on the project's `runner/environment`
thread, and then leaves it be for thirty minutes at a time instead of
re-reading it every tick - `pulse land --project X --branch B` lands it
whenever you are ready, and `pulse land --project X` by hand always shows what
waits.

### Verifications take turns

The same verification command runs three times per change - in the worker
clone's pre-commit hook at every commit, once more when the run ends, and in
the landing worktree - and on one machine those runs overlap across clones.
When the clones share a build cache (a `CARGO_TARGET_DIR`, a Gradle or Bazel
cache: the usual way to keep a large build fast and the disk small) two
overlapping runs rewrite each other's artifacts mid-run and both fail for
reasons neither branch contains, so a good branch is charged or the default
branch is judged red. `pulse settings verify.lock project` makes the
verifications of one project take turns on the machine (`machine`: every
verification does); the command runs under an atomic lock in
`$PULSE_HOME/locks/`, a later arrival prints that it is waiting and for whom, a
lock whose holder has died is removed by the next arrival, and a wait longer
than `minutes_per_job` gives up with exit 75 without running the command.
Taking turns does not make a shared cache tell one checkout from another: a
build system that decides freshness by file times still needs the project's own
command to refresh what the checkout owns (for Cargo, touch the workspace
sources before building; the dependency cache stays).

Taking turns also sets how many work jobs a project can usefully run at once:
with a per-project share of 4 and a check that takes eight minutes, a clone's
commit may wait for three other checks before its own and the harness kills the
command at `harness.commit_timeout` (25 min by default; plain commands stop at
`harness.step_timeout`) - the run retries, and the lanes burn their budget
waiting on each other. So the daemon measures the check's own run time at every
landing (`verify_seconds` on the `land.landed` event, the lock's wait excluded)
and leases `improve` and `task` jobs for the project only up to the lanes that
fit that commit cap (`commit_timeout / median of the last five`, never below
one; the cap is `harness.commit_timeout`, never below `harness.step_timeout`,
the same deadline the harness gives the gated commit); research, ballots and
answers keep the plain limit. The project's idle line says so: `2 of 4 work
lanes open: the check takes ~10 min and 2 in a row fit the 25 min commit cap
(harness.commit_timeout)`. A quicker check, a longer commit cap, or
`verify.lock off` opens the lanes again. The lanes are the fleet's: the owner's
work takes one beyond the count at once, with its commit cap widened to cover
the checks it then shares the lock with (see the express lane above).

When the deadline still lands on a worker's `git commit` - the change finished,
the pre-commit check did not - the runner does not discard the tree: if the
worker exited normally, left no commit, and the run's transcript shows a `git
commit` step that timed out, the runner commits what the worker left (tracked
changes and whatever it had staged, under the worker's own `-m` message when it
gave one, signed with the run key and carrying the provenance trailers; only
the pre-commit check is skipped) and the run's own verification judges it like
any other commit. The run log says `the runner committed its work for
verification`; a worker that stopped for a reason of its own keeps its verdict.

### Several branches at once: `landing.batch`

A landing verifies each branch on its own, and on a project whose check takes
long that serializes the queue: on amistio-template one verify took 1,210 s on
average (36 timed landings over three days, the longest 1,642 s) and two to
four finished branches often waited at once, so the last of them sat through
every verify ahead of it. So when a landing pass finds two or more plain
pending branches with a verify command to run (`landing.batch`, default `on`;
`off` lands every branch on its own), it lands them as one: in a throwaway
worktree of the default branch it merges each member in finished order exactly
as its own landing would (the same fast-forward and merge-commit rules, the
`pulse` identity, `out/`-only conflicts resolved by rebuilding the hub bundle,
`PULSE_RESULT.md` dropped), leaves out a member whose merge conflicts - it
lands or is refused on its own afterwards - points a synthetic branch
`pulse/batch/<id>` at the chain and lands that branch like any other: one
verify of the merged whole, one fast-forward, one push, one `land.batch` event
(the chain as subject, the batch id and the member jobs and branches in the
detail). The pass then marks each member landed in it: a `land.landed` event
of its own, with the commits it added to the chain as `before` and `sha`, the
batch id and the chain's verify in the detail, and text, note and result post
on the job's thread that say the branch landed in batch `<id>` with N branches
and was checked once as the merged whole, so the members leave the queue, show
on the task page and count for the push exactly as individually landed jobs
do. Experiments,
sync and upstream landings, held branches, branches in a retry hold, and
branches a landing would refuse or treat specially - owner paths, guarded
files, `docs/context/environment.md`, a lockfile the release-age rule dates -
never join a batch; a batch counts once against `pulse land --limit`, whatever
its size. Whatever goes wrong with the chain costs the pass only that one
attempt: a chain that fails its verify, conflicts, is refused or cannot be set
up records `land.batch-failed` (the reason and the check's tail in the detail)
instead of a retry hold, a repair or a result post, its synthetic branch is
deleted, and the same pass lands the members one by one, where the member's
own verify names the culprit and the good branches still land. A red default
branch holds a batch as it holds any landing.

### Pushing: `landing.push`

Landing moves your local default branch; whether it goes further is your
standing instruction, `landing.push` (ADR-037). `off`, the default: you review
and push. `pulse settings landing.push main`: after each landing the daemon
pushes the project's default branch to its upstream - the one branch, a plain
fast-forward push with the git credentials this machine already has (or with
the daemon's own key, see [Credentials](#credentials)), never a force, never a
prompt - and pushes what landed earlier and was not pushed yet, so turning it
on sends yesterday's landings along with whatever else sits on your `main`.
Before it pushes, the daemon fetches the upstream branch through the same push
url and credential.

When someone else moved the remote, it does not stop at "not fast-forward": it
brings the remote commits in as a landing of their own
(`pulse/sync/<remote>/<branch>`, merged in the throwaway worktree and judged by
the project's check like any other landing, `land.synced` in the log), then
pushes the result. The upstream is the owner's published truth, so a sync is
set up and checked as upstream has it: its recipe, its lockfiles and manifests,
and the paths the check names are the merged tree's, not restored from the
local head the way an agent's branch has them restored so it cannot change what
checks its own work. (Before this, a sync of an owner's rename lost the renamed
solution and csproj files the check named and failed every setup with `MSB1009:
Project file does not exist`, and no retry could pass.)

A plain merge that conflicts is not left to you: the upstream is the published
truth, so the daemon re-applies the local branch's own commits - the ones the
upstream lacks, merges left out, up to 200 - onto the upstream tip in a
throwaway worktree, one by one, the way a pull with a hand resolution would
end. A commit the upstream already holds in another form becomes empty and is
dropped; one that clashes only on files the upstream removed loses those files
and keeps the rest; one that conflicts for real is set aside. The tree that
results becomes one merge commit of both sides (`Sync <remote>/<branch> into
<default>` in the log, listing where each commit went), landed and checked like
any sync, so the push is a fast-forward and the local history still holds every
set-aside commit through the merge's first parent. Each set-aside commit gets
its patch under `~/.pulse/patches/<slug>/resync-<sha>.patch` and one job
(`task`, tier 1, source `resync:<slug>:<sha>`) that re-applies what it meant on
the synced branch, or finishes without a change when the upstream holds it
another way; a `finding` for you names them, and `land.synced` carries `resync`
with each commit's fate. A merged result that fails the project's check lands
all the same when the upstream tip fails it the same way (`land.upstream-red`
in the log): the forge is red already, holding would protect nothing, and only
a checkout that holds the forge's head lets the pipeline watcher's fix start
from it. A merge that fails the check for more than the upstream's own reasons,
or a re-apply that cannot be made (the branch too far ahead, a worktree that
cannot be added), holds the push with the reason and `git pull --no-rebase`
spelled out, and is not tried again until either side moves. A conflict a
daemon remembered before it knew how to re-apply is retried once by the
re-apply rather than left waiting for a move. A credential error that carries
the tool's own timestamp (`[ERROR] 2026/10/31 09:08:24 ...`) is noted without
it, so the hold is logged once, not again every pass.

One remote move is not merged at all: an upstream rewritten by hand into a
squash of history the daemon had already pushed - a root commit, or any commit
the local branch shares no ancestor with, whose tree equals a commit on the
local branch's first-parent line. The daemon copies the local commits made
since that point onto the squash with their trees, authors, dates and messages
(a landing's merge becomes one commit carrying its tree, up to 2000 commits),
points the local branch at the copy - the tree is unchanged, so the working
tree, the index and the check's verdict are untouched - and pushes the
fast-forward, `land.synced` with `replayed` in the log. A remembered conflict
from an earlier merge attempt is retired the same pass.

A push url that is `https://` and takes a password the daemon has none of
(`could not read Username`, `unable to get password from user`) is moved to its
ssh form (`git@github.com:owner/repo.git`) when that form answers to the
daemon's key - push url only, the fetch url stays, `land.push-url` in the
log - and the hold note names the `git remote set-url --push` to run when it
does not.

A push that is still refused (no credentials, no upstream) never undoes the
landing: it is a `landing` note in `pulse status` and the hub and a `land.held`
event once per change, retried every tick until it goes through as
`land.pushed`; the note ends with the way to a pull request instead. `pulse
land` by hand does the same. Workers never push, whatever the setting.

### Pull requests

`pulse settings landing.push pr`: the daemon pushes the default branch to the
PULSE-owned `pulse/landed` branch on the upstream instead - a plain push, never
a force; a `pulse/landed` someone else moved holds with the reason - and keeps
one pull request open from it into the upstream branch (`gh`, the GitHub CLI,
logged in as the user the daemon runs as; its login never reaches git), so
every landing waits for your merge on the forge. Later landings push to the
same branch and update the open pull request; a merged one is followed by a
fresh pull request with only what is new (merge with a merge commit: a squash
or rebase merge lists the old commits again). When the upstream moved and
merging it conflicts, the pull request still opens and shows the conflict for
you to resolve there, nothing holds here. The pull request is `land.pr` in the
log, with its number and url.

Whatever the setting says, a task's conversation in the hub offers **Open a
pull request** once its work landed and is not on the forge yet (the Task API:
`POST /api/v1/tasks/{id}/pull-request`), and `pulse land --project <slug> --pr`
asks the same way: the daemon's next landing pass delivers that project's
landed work as a pull request (`land.pr-asked`, answered by `land.pr`, or by
`land.pr-answered` when the upstream already has everything). The sheet's
delivery line says where a task's code is - landed here only, held with the
push's reason, on the upstream, or on pull request #N - and the task's activity
feed carries the same, so the lead agent reads at close whether the work
reached GitHub, not only that it landed.

### Signatures and parallel landings

Before you push, `pulse verify <ref>` checks that a landed commit's signature
and its `Pulse-Agent`/`Pulse-Job`/`Pulse-Executor` trailers match the run that
made it (ADR-022); `pulse signers` lists the keys it trusts.

The daemon can tend multiple projects' finished branches at once with
`landing.parallel` (default `4`); landings within one project remain serial
behind that project's landing lock, and `landing.parallel=1` keeps the old
single-project maintenance-pass behaviour.

## Pipelines: what a push set running on GitHub

A push is not the end of a landing: the repository's GitHub Actions
workflows run on it, and a red one is a regression PULSE made. So the daemon
watches what its own pushes set off - and, since a project's default branch
is PULSE's to keep green whoever pushed to it, the head of that branch on
the forge as well. Each tick, for every active project
whose remote is on GitHub, it takes the pushes of the last 48 hours that
still have no verdict - `land.pushed` to the default branch and `land.pr`
to `pulse/landed` alike - and asks GitHub for the runs on that commit (`gh
run list --commit`, the GitHub CLI logged in as the user the daemon runs
as, with read access to the repository's Actions). It counts the runs the
push itself caused (`push`, `pull_request`, `pull_request_target`,
`merge_group`, and the `dynamic` runs of the repository's own workflows,
not GitHub's **Dependabot Updates** and **Dependency Graph**) and records
one verdict per push as an event with the runs in its detail:
`pipeline.passed` when every counted run finished green (`skipped` and
`neutral` count as green); `pipeline.failed` when one failed, timed out,
did not start or wants your approval; `pipeline.none` when nothing ran for
the push ten minutes after it, or every run was cancelled; `pipeline.gave-up`
when a run is still going `pipeline.wait_hours` (default `3`) after the push.
While runs are still going the daemon looks again every three minutes - a
failure beside a run that is still going is reported once the push is half
an hour old, so one slow deployment does not hide a failed test suite.

A `failed` or `gave-up` verdict is a `finding` for you on the project's
`runner/pipeline` thread: which workflow failed, the step that failed in
each failed job, the last lines of its log (secrets redacted), what the push
landed, and the command that re-runs the failed jobs (`gh run rerun <id>
--failed`); `none` is only recorded, since a push that ran nothing broke
nothing. With `pipeline.watch` at `fix`, the default, a red pipeline also
queues one fix job for the project (`task`, tier 1, priority 60, source
`pipeline:<slug>`) whose brief carries the same evidence and the project's
verify command: a worker takes it like any other job, and its landing is
pushed and watched in turn. A worker has no `gh` of its own, so the brief is
what it knows of the failure; when GitHub no longer has the failed log
(HTTP 410 or 404: the repository's log retention passed, or someone deleted
it) the daemon does not plan a job around a brief with no error in it. It
reruns the failed jobs instead (`gh run rerun <id> --failed`, once per
commit, recorded as `pipeline.rerun` with the reason), polls the rerun like
a push of its own and judges that: green means there was nothing to fix and
the verdict says so; red again gives a fresh log, and the brief carries it
with a line that says the first attempt's log had expired. A rerun GitHub
refuses, and a log the daemon could not read for another reason (a timeout,
say), plan the job at once with the error named in the brief and the
finding, so a worker knows the log is missing rather than empty. One such
job is open per project at a time -
a second red push while it waits refreshes the queued job's brief instead
of queueing a second one - and after three fix jobs in 24 hours the daemon
plans no more and says so in the finding, so a pipeline that is red for a
reason outside the code (a secret that expired, a runner that is off) does
not turn into a day of workers. When several pushes of one project are
waiting for a verdict, only the newest gets the finding and the job; the
older ones are recorded as superseded. `watch` keeps the findings and skips
the jobs; `off` reads nothing. A repository whose runs the login cannot read
(HTTP 403 or 404) is reported once, as `pipeline.unreadable`; the daemon
keeps trying and says nothing more until the runs can be read again.

The branch head is read every ten minutes (`gh api
repos/<owner>/<repo>/branches/<branch>`, the branch the local default branch
tracks, `origin/<default>` when it tracks none). A head that is one of
PULSE's own pushes, or a commit already judged, is left alone; any other
is judged by the same rules, its verdict recorded with `head: true` and the
login or name that pushed it (`by`), and a red one is a finding and a fix job
like a push's - with one difference: a worker starts from the project
checkout, so the job is planned only once the checkout's default branch
holds the commit. The landing pass brings it there: with `landing.push` on,
the daemon fetches each project's upstream every ten minutes whether or not
anything landed - a quiet project gets a landing worker for it (`landing
<slug>: started (upstream refresh)` in the log) - because a tracking ref
nobody fetches never says the forge moved: before this, a merge made on
GitHub never reached the checkout, and the red head waited its two hours
for nothing. A checkout that is behind then follows: a clean checkout on the default branch is fast-forwarded, a
checkout on another branch or detached has its default branch moved by ref
and its working tree left alone (`land.fast-forward` with `how`), a dirty
checkout on the default branch keeps its HEAD and the sync merges the remote
commits in as above. Until the checkout holds the commit the daemon looks
again every three minutes and the log says it is waiting; when the checkout
still lacks the commit two hours after the daemon first saw the failure, the
finding goes out without a job and names what holds the checkout behind (the
push's hold note, when there is one), so you can see what to clear. The
brief of a head's fix job says that the push was not PULSE's and that PULSE
looks after the branch all the same.

`pulse pipeline` shows, per project, the branch head last read (who pushed
it, and since when a red one waits for the checkout), the pushes still being
watched and the last verdicts (`(pushed by X)` on a head's; a red one names
its fix job and how that job stands - `(queued)`, `(leased)`, `(done)`, or
`(failed: blocked: ...)` with the worker's first words, so a red pipeline
whose fix came back blocked reads as yours, not as looked after), with `--project
<slug>` for one, `--all` for every active project, `--limit N` for more
history and `--json` for the records (`head` holds the sha, `by`, `read`,
`next` and `waiting_since`); `pulse pipeline --check` reads GitHub now
instead of waiting for the daemon's pass. A task's conversation in the hub
shows the verdict on its own push on the delivery line
(`delivery.pipeline` and `delivery.pipeline_job` in `GET /api/v1/tasks/<id>`).

## Dependencies: security alerts and the release-age rule

One rule governs every dependency change in every project PULSE manages: **a
version is adopted only once it is `dependencies.min_release_age_days` old**
(default `3` days, a store-wide setting, `0` turns the wait off). A package
that is hijacked or shipped broken is usually pulled within days; waiting
that long costs little against a known vulnerability and nothing against a
routine bump. The publish dates come from the package's own registry - npm,
crates.io, PyPI, the Go module proxy, RubyGems - and are cached in the store
once known (`release_age:<ecosystem>:<name>:<version>` in `meta`), since a
publish date never changes. The rule is applied twice, so neither an agent
nor a branch can slip a younger version past it.

**The watcher.** Once per `dependencies.alert_check_minutes` (default `60`)
the daemon reads each active project's open Dependabot alerts through the
`gh` CLI it already uses (`gh api repos/<owner>/<repo>/dependabot/alerts`,
GitHub remotes only), groups them by manifest and asks the registry, for
every vulnerable package, for the newest patched version that stays within
the installed version's compatible range - same major, same minor below
`1.0` - and is at least the rule's age. Those become **one Tier 1 `task` job
per manifest** (source `security:<slug>:<manifest>:<alert numbers>`, priority
`90`, `security.queued` in the log) whose brief is the exact versions and the
package-manager commands that adopt them - `npm install next@16.4.2`, `cargo
update -p x --precise 1.2.3`, `uv lock --upgrade-package`, `go get`, `bundle
update --conservative` - and nothing else: change no other package, never a
blanket update, rebuild a committed bundle when the repository has one, run
the project's check, one commit. Because the owner is waiting on it, a
security job leases with the owner's own work, ahead of the fleet's and
alone through the load governor's express lane; its worker brief names the
lockfile change as the job instead of forbidding it (every other job still
reads "never change a lockfile - dependencies are Tier 2"). A patched version
younger than the rule is not queued yet: the daemon logs the date it becomes
eligible (`security.waiting`) and queues it on the first pass after that.
Alerts a job already covers - queued, running, or finished within the last
24 hours, while its branch may still be landing - are not queued again.
What the watcher cannot take it hands to you, once each, as a `finding` marked
for the owner on the project's `security/dependabot` thread, where it waits in
the console's For you inbox with two buttons: a fix that
needs a **major upgrade** (`security.major`; a Tier 2 change by the
constitution, so the finding asks "Yes: Upgrade it / No: Leave it to me" and
your yes queues the upgrade as a security job - or set
`dependencies.major_updates auto` and the watcher queues major upgrades like
any other fix, to the newest eligible version of the patched line, and
answers the finding that asked you itself (`security.auto_major`, naming the
job) so the question leaves your inbox the moment the setting decides it), an alert
whose advisory names **no patched version** yet (`security.unfixable`, "Yes:
Look for a way around it / No: Keep watching"; the update is queued the
moment one is published and old enough), an **ecosystem the watcher does
not read** (`security.unsupported`: it reads npm, Cargo, pip, Go and
RubyGems), and alerts **switched off** on GitHub or unreadable to the
signed-in `gh` account (`security.disabled`, with the setting to turn on).
When an alert behind a finding is closed on GitHub - fixed by hand, dismissed,
or gone with the dependency - the watcher answers the finding itself
(`security.closed`) so nothing stale waits for you.
`dependencies.security_updates off` stops the watcher; the watcher's memory
per project is the `meta` row `dependabot:<slug>`.

**The landing.** A branch that changes a lockfile together with its manifest
(`package.json`, or `pnpm-workspace.yaml`, where pnpm 10 keeps `overrides`
and `catalog:` pins - before this a branch changing those two alone was no
dependency bump to the landing, main's lockfile was restored over the
branch's workspace file, and every landing of the project then failed its
setup with `ERR_PNPM_LOCKFILE_CONFIG_MISMATCH`)
used to wait for a human landing in every case. Now the landing diffs the
pins between the default branch and the branch (`Cargo.lock`,
`package-lock.json`, `pnpm-lock.yaml` in every shape pnpm has written,
`yarn.lock` classic and berry, `uv.lock`, `Gemfile.lock`) and asks the
registry how old every new version is. When each one is at least the rule's age the
branch lands like any other, with its lockfile and manifests kept for the
check (the recipe's `cargo fetch --locked` or `npm ci` must see the branch's
lockfile, not main's) and a `land.release-age` event naming the versions
adopted. When a version is too young the branch is held with reason
`release-age` and the date it can land from (`can land from
2026-10-26T13:40Z`); the daemon lands it on the first tick after that, no
human needed. A jump past the compatible range (`next 16.3.8 -> 17.0.0`), a
lockfile the landing does not read (`poetry.lock`, `go.sum`, `bun.lock`) or
a registry that cannot be asked keep the human hold as before, with why in the `land.held` event, and `pulse land --project
X --branch B` by hand still lands whatever you ask for, recording the verdict
only. One exception to the major-jump hold: a branch the **security
watcher** queued (job source `security:…`) lands past the compatible range
when `dependencies.major_updates` is `auto` - you authorized those upgrades
in advance, so the landing does not ask again; the `land.release-age` event
then carries `authorized: dependencies.major_updates`. An agent's own major
jump still waits, and a security major held under `ask` says in its hold
note which setting would let it land. With the rule at `0` days no registry is asked and only the
compatible-range check remains.

## Credentials

"With the git credentials this machine already has" means the daemon is only
as unattended as your credentials are. An SSH key that lives in an agent
which asks you before each use (1Password's SSH agent, a hardware key) makes
every push wait for a click that never comes at 3 a.m.: `push failed:
sign_and_send_pubkey: signing failed ... communication with agent failed`
lines in the daemon log, landings piling up unpushed. No vault whitelists a
process for good - 1Password's agent remembers an approval only until the app
locks - so the honest answer is a key of the daemon's own (ADR-041):

- Make the daemon a key GitHub knows and that can do nothing but push this
  repository: a **deploy key with write access** on the repository (or a
  machine user's key). In 1Password, keep it as an SSH Key item in a vault of
  its own, say `PULSE`, and create a **Service Account** (Developer > Service
  Accounts) with **read-only** access to that vault only - service accounts
  cannot see your Private vault and never need the app to be unlocked.
- `pulse cred token` - paste the service-account token once. It goes to the
  login Keychain on macOS (service `pulse`, account
  `OP_SERVICE_ACCOUNT_TOKEN`), to `$PULSE_HOME/secrets/` as a 0600 file
  elsewhere, never on a command line, never in the store.
- `pulse cred set "op://PULSE/pulse push key/private key?ssh-format=openssh"`
  - the setting `push.credential` holds a **reference**, never the key. `set`
  reads the key, prints its fingerprint and public key, probes every
  project's upstream with a dry-run push through that key alone, and saves
  the reference only when the remote answers. A key the remote refuses names
  the public key to add as a deploy key and saves nothing.
- `pulse cred status` (also `--json`) says what is set and where the token was
  found; `pulse cred clear [--token]` returns the daemon to your credentials.

The same steps live in the hub (ADR-042): Settings > **Push key** > *Choose a
key…* opens a picker that lists the providers this machine has (1Password,
the macOS Keychain, a key file), takes the service-account token once, lets
you browse the vaults and SSH Key items the token can see and pick one, then
**Test and save** runs the same read-and-probe as `pulse cred set`. A refused
key shows its public key to add as a deploy key and saves nothing; *Remove*
is `pulse cred clear`. The hub route (`/api/v1/cred`) answers only on
loopback, only with the `X-Pulse-Cred` header, and records the change as
`human:dashboard`, so the browser gets no more than the CLI does - the token
and the key never pass through the settings store or the page.

Without 1Password, `keychain://SERVICE/ACCOUNT` names a macOS Keychain item
holding the key, and `file:///ABSOLUTE/PATH` a 0600 file (a symlink or a
looser mode is refused). Whatever the reference, the daemon resolves it at
push time, writes the key 0600 into a 0700 directory under `$PULSE_HOME/tmp`
for the seconds of the push, runs `ssh -oBatchMode=yes -oIdentitiesOnly=yes
-oIdentityAgent=none -i <that file>` for exactly that push, and removes it.
A key that cannot be resolved holds the landing with `push credential:
<why>` in `pulse status` and the hub; the daemon never falls back to your
credentials, and no reference is resolved inside a worker - the one exception
is an MCP server's secret, which `pulse mcp` resolves on the local executor
for the request it sends and never prints (see [Connecting the outside
world](board.md#connecting-the-outside-world-pulse-mcp)). Only the owner runs `pulse
cred token|set|clear` or sets `push.credential`; agents may read `pulse cred
status`. Rolling back is `pulse cred clear` (ADR-037's behaviour
returns at once), then revoke the service account and the deploy key.

Where this runs: the daemon is a launchd agent on macOS and a `systemd
--user` service on Linux, and the credential behaves the same on both - the
one difference is where the service-account token rests (the login Keychain
on macOS, a 0600 file under `$PULSE_HOME/secrets/` on Linux, or the
`OP_SERVICE_ACCOUNT_TOKEN` environment variable on either) and that
`keychain://` is refused on Linux with a pointer to `op://` or `file://`.
1Password's Linux agent asks the same questions as the macOS one, and a
`systemd --user` service does not even see its socket, so the reasons for a
key of the daemon's own hold there too. Windows is not a host for the daemon
today - the module does not build there (process groups, signals and disk
statistics are POSIX-only), so nothing in this section applies until that is
a decision of its own.
