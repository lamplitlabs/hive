# The runner (how the machine works)

`hive daemon` (the service) delivers verified branches as pull requests and
follows the upstream ([Delivery](landing.md)), plans jobs from the
tally, leases work to executors (this machine, paired Linux hands, or
provisioned compute), and runs the lead of every project so owner tasks and
messages get answers. `hive work` runs one worker here; `hive land`,
`hive plan`, `hive executor`, `hive pair`, `hive hands`, `hive env` and
`hive sweep` are the same pieces by hand. `hive stop` / `hive start` is the kill switch.

## The daemon loop

One tick of the loop, top to bottom, with what a line starts or ends on its
right; the planning pass is the one line a tick may skip, since it runs at
most once per `daemon.pass_seconds`, or at once when you write a task or a
message.

```text
      ┌────────────────────────────────────┐
┌───→ │ collect the runs that ended        │
│     │ stop switch, pending restart       ├──→ the daemon leaves
│     │ write the heartbeat                │
│     │ reap dead agents, idle workers     │
│     │ the planning pass, when it is due  ├──→ maintenance thread
│     │ ask for a chores pass              ├──→ chores thread
│     │ refill free slots: lease, run      ├──→ one thread per run
│     └─────────────────┬──────────────────┘
│                       ↓
│     ┌────────────────────────────────────┐
│     │ wait: a run ends, an event wakes   │
└─────┤ the loop, or the timer runs out    │
      └────────────────────────────────────┘
```

Use `hive daemon --interval S` (or `hive service install --interval S`) to
set the idle wait and the retry pause after an all-error batch. The default
is 300 seconds; zero or negative values use that default. `hive daemon`
takes flags only: a stray word (`hive daemon start`) exits 2 with the usage
instead of starting a loop. It also refuses to start (exit 3, naming the pid)
while the store's heartbeat names another daemon whose pid is alive and whose
beat is fresh; `--force` starts anyway. An `exited` or `restarting` beat, a
stale beat or a dead pid never holds the store. Store events can
wake the loop early; job queue events (`job.add`, `job.queued`, `job.requeue`,
`job.recover`) wake dispatch, and an idle daemon also polls the active queued
count every 15 seconds so a missed event still starts work within one dispatch
tick without multiplying idle log lines. Cancellation stops the wait. Successful
runs and work still in flight keep their existing prompt-dispatch behavior.
The loop has two speeds: every tick (each 15 seconds while runs are in
flight, and whenever a run ends or an event wakes it) reaps, watches the stop
switch and a pending restart, asks the chores thread for a pass and refills
free slots;
the planning pass (the leads, tasks, amendments, recoveries and lessons of
every project, evolution, schedules, the planner, idle notes, executors and
the maintenance thread) runs at most once per `daemon.pass_seconds` (default
60), however often the loop wakes, since planning every fifteen seconds under
four agents cost a quarter of a core for jobs that only queued behind full
slots. Zero plans on every tick, and a shorter `--interval` caps the spacing.
What you write does not wait for the spacing - a task filed or amended, a
task's status changed, a message sent: the tick it wakes runs the planning
pass.
The heartbeat the hub shows counts `loops` and `passes` apart, and carries
the body's own time as `tick` (the last body in milliseconds, the slowest
since the start and how many ran over the fifteen-second dispatch cadence):
`hive status` prints it as `last tick 1200ms (slowest 41000ms, 2 over the
cadence)`. The daemon says `tick N took 40s, over the 15s dispatch cadence
(planning pass); queued work waited that long for a slot` once when a body
crosses the cadence and again only after one fits, so a slow pass is one
line, not one per tick; before this the heartbeat said when the loop beat
and how often, and a body that spent its cadence planning looked like a loop
that merely beat late. The line ends with where the time went - `; longest:
plan 14s, maintenance 9s, leads 3s`, the three longest stages of a second
or more, a per-project tender summed across the projects - and the heartbeat
carries the slowest body's stages as `tick.longest`, which `hive status`
prints after the slowest time (`slowest 31000ms: plan 14000ms, leads
9000ms`): the number alone said that seven passes after one restart took
29-33 s each and nothing about which of the twenty tenders took them.

The stages of one tick in the order the code runs them, by the names the
slow-tick line and `tick.longest` print; the `each project` row runs through
for one project, then for the next.

```text
┌──────┐     ┌───────────────────┐     ┌────────┐     ┌──────────┐
│ reap │ ──→ │ the planning pass │ ──→ │ chores │ ──→ │ dispatch │
└──────┘     └─────────┬─────────┘     └────────┘     └──────────┘
                       │ its stages, in order
  ┌────────────────────┴───────────────────────────────────────┐
  │ first          probes, maintenance, adopt                  │
  │ each project   amend, manifest, records, leads, toolkit,   │
  │                tasks, suggestions, recover, lessons        │
  │ then           evolve, schedules, plan, idle, heldout,     │
  │                executors                                   │
  └────────────────────────────────────────────────────────────┘
```

Three chores left the tick for a background thread of their own
(`daemon_chores.rs`): the Dependabot alert watch (`security`), the pipeline
verdicts (`pipeline`) and the compute listing that starts approved machines
and releases idle ones (`compute`). The log had 488 `tick N took ...` lines
over a few weeks - median 21.5 s, p90 40 s, max 395 s against the 15 s
cadence - with these among the longest stages, and a tick that is late leases
late, owner's work included. The tick now asks for a chores pass and goes on
leasing: every tick asks for `compute`, a planning pass also hands over its
projects for `security` and `pipeline`, and asks made while a pass runs merge
into the next one. A stopping daemon still runs the pass it asked last (so an
approval seen by the last tick is provisioned) for up to five minutes before
it cancels it between two projects. So the stages the slow-tick line can name
are the tick's own - the leads, tasks, recoveries, evolution, schedules,
planner - and `chores` only for the moment the ask takes.

Every stage of the two background passes - the landing maintenance pass
(`delivery`, `pull request reviews`, `sweep`, `reclaim`, `board hygiene`,
`landing outcomes`, `revert scan`, `unfit re-probe`, `binary check`,
`backup`) and the chores pass (`security`, `pipeline`, `compute`) - has the
budget `daemon.stage_budget_seconds` sets (default 60, 0 for none;
`stage_budget.rs`). A stage that works project by project (delivery,
reviews, revert scan, security, pipeline) checks it between two projects:
once spent, the stage stops before its next project, writes a
`daemon.slow-stage` event (`stage`, `ms`, `budget_ms`, `ran`, `skipped`) and
says `slow stage: pipeline took 1m12s, over its 1m budget; skipped until the
next pass: a, b`, and its next pass starts with the first project it left,
so the projects at the end of the list are not starved behind a slow one at
the front. A project already started is never cut mid-write, and at least
one project runs every pass. A stage that cannot be split (the sweep, the
backup, compute) runs to its end and is recorded the same way when it went
over. `hive events --json` shows them among the other events.

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
account's token in `GH_TOKEN`; an account whose token `gh auth token`
cannot hand out counts as one that failed and the next is tried. The first
that answers is remembered for
the project (`gh_user` in its watch), read as first from then on, and named
once in the log (`pipeline <slug>: reading <repo> as <account>, since the
active gh login <name> cannot see it`); a remembered account that fails the
same way is forgotten and the accounts tried again. When none can see the
repository the stage leaves it alone for an hour (`unreadable_until` in the
watch), tells the owner once with the accounts it tried, and tries them
again when the hour is over. A branch that is not there is a 404 too but
says so, and no other account is tried for it. The tokens stay in the
process and never reach the log, the board or an event.

When a GitHub App is configured ([landing](landing.md), ADR-070 step 3) every
pipeline read runs as the App first, with its installation token for the
one repository in `GH_TOKEN`, and no machine login is asked for a token or
listed. Only a read the App cannot make (the same 401/403/404 answers: the
App is not installed on the repository or lacks Actions read access) or a
token that cannot be minted (written down once as `gitapp.fallback`) goes
to the remembered login and the rotation above, for the rest of that pass.

The life of a job: `queued` when it is added, `leased` while its run goes,
`done` or `failed` when the run ends. A run held for a restart, a lost run
or a runner error puts the job back in the queue; the third attempt that
ends lost or in a runner error fails the job (a hold, or an error that is
the machine's or the recipe's fault, costs no attempt). Recovery then sorts
a failed job on a planning pass
([Failed jobs and recovery](recovery.md#failed-jobs-and-recovery)).

```text
   a lead, the planner, a schedule, hive job add
                         ↓
                    ┌──────────┐
                    │  queued  │ ←────────────────────────────┐
                    └────┬─────┘                              │
                         │ a free slot leases it              │
                         ↓                                    │
                    ┌──────────┐  restart, lost run, runner   │
                    │  leased  ├─ error, lease expired ───────┘
                    └────┬─────┘
               ┌─────────┴──────────┐  the run ends
               ↓                    ↓
          ┌──────────┐         ┌──────────┐
          │   done   │         │  failed  │
          └──────────┘         └────┬─────┘
                                    │ recovery, on a planning pass
      ┌─────────────────┬───────────┴─────┬─────────────────┐
      ↓                 ↓                 ↓                 ↓
 of the moment:    of the work:      another path:     the environment:
 queued again      a corrected       held, with        blocked, with
 after a pause     copy, or the      its reason        its reason
                   lead re-plans
```

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

When one of the owner's jobs (a lead's sitting, an answer, a job planned from
an owner task, a security update) has waited in the queue for ten minutes
(`hive_tasks::OWNER_WAIT_MINUTES`), the daemon writes one `job.owner-wait`
event on it naming every hold it saw while the job waited, short: `lanes
full` (every slot busy with the fleet's work), `load 27 > 24`, `memory 300
MB < 1024 MB`, `restart drain`, `disk floor`, `bulk disk away`, `daily cap
reached` - or the tick's own idle reason when none of those held it - and
logs `owner job <id> (<project>) waiting since 04:40 UTC: lanes full, load 27
> 24`. `hive status` lists the open ones as `waiting` lines, and the task's
wait line on the board and the task page reads `The next step has waited
since 04:40 UTC: lanes full, load 27 > 24.` in place of "waiting for a free
agent". A wait ends when the job is leased or queued again; the next wait
gets its own event. On 2026-11-03 the owner's TASK-055 waited from 04:40 to
15:18 behind full lanes, a load of 26-30 over the line 24 and a restart
window, and only the daemon's deduped idle log said so
(crates/hive-runner/src/owner_wait.rs).

The slot count and the memory floor in those holds are not settings any more.
`daemon.max_jobs` (local runs at once) and `daemon.min_free_mem_mb` (the
memory under which nothing new starts), like `workers`,
`parallel_per_project` and `daemon.max_load`, are automatic now: each machine
works out its slots from its cores and memory (`capacity.rs`), the memory
floor is the internal 1024 MB (`MIN_FREE_MEM_MB` in `daemon_load.rs`), a
stored value is swept at startup with a `setting.retired` event, and `hive
settings set daemon.max_jobs 2` is refused with `daemon.max_jobs is automatic
now`. Set `capacity` to cap this machine (0 keeps it idle), pass `hive daemon
--max-jobs N` for one daemon, or `hive stop` to pause; see "The load
governor" in [landing.md](landing.md).

## Services and self-update

`hive service install` writes two units: `hive.daemon` (the fleet) and
`hive.hub` (the console on `127.0.0.1:8765`; `--hub-port N` moves it,
`--no-hub` skips it). Both restart themselves when a new `hive` binary lands
on disk, so the console you keep open is always the build the daemon runs, and
neither dies with a terminal. Running `hive service install` again is safe: a
unit whose file did not change and is loaded is left running.

A service keeps the `PATH` it was installed with, and every program the
daemon and its runs start is found on it or not at all: `gh` for pull
requests, the helper that reads a push key, the tools a project's recipe
pins. So the units get the `PATH` of the shell that ran `hive service
install`, as it is, and after it each of these directories that exists and
is not on it already: the one `hive` is in, `~/.local/bin`, `~/.cargo/bin`,
`/opt/homebrew/bin`, `/home/linuxbrew/.linuxbrew/bin`, `/usr/local/bin` and
`/snap/bin`. The command says which it added. That covers an install run
from a script, over ssh or from an app, where `PATH` is `/usr/bin:/bin` and
little more. A tool that lives anywhere else must be on the `PATH` of the
shell you run the command from; when the daemon says a tool is missing that
you know is installed, run `hive service install` again from a shell that
finds it.

That install line is for the first install only. After it, the daemon keeps
itself current (ADR-035): when the brain's `main` moves - a pull request
merged on the forge and fast-forwarded into the checkout by the delivery
pass ([Delivery](landing.md#the-local-default-branch-follows-the-upstream)) - it
rebuilds `hive` from that clean checkout, smoke-tests the new binary, installs it in
place and restarts once its runs in flight finish - the console follows within
a minute and shows the build it runs beside the daemon's under **Settings →
Running**. The restart is a handover, not a pause: a daemon whose binary
changed on disk while runs are in flight keeps dispatching, planning and
delivering for up to `daemon.restart_drain_minutes` (default 30), stops starting
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
release. A run the window outlives is asked to hold rather than waited for
(`daemon.restart_hold`, on by default): as under **Restart now** below, the
harness saves its conversation and exits held within moments, the job
returns to the queue with the attempt refunded and the next daemon carries
it on in the same clone, so the handover costs the run a step rather than
the machine the run's length - log lines `restart: N run(s) in flight, M
asked to hold (daemon.restart_hold); ...` then one per run. A held job
waits for the agent that held it while that agent is free, and for this
machine: a hand keeps no clone between runs, so a hand's slot passes it
(after the restart of 2026-11-05 a job held at 161 steps went to
hands-cr-065 and started over). A run past its
harness - in its run-end check, or a custom worker - has no session to hear
the hold and runs to its end, since a check stopped midway loses the run and
its job is done over. The measured reason: on the brain, where a task run
takes hours, ten self-updates in three days waited 0.2 to 4.3 hours each
with nothing dispatched, twelve hours in all. `off` waits for every run, as
before; the console's own **Restart** without "now" holds nothing either, as
the owner chose. `0` stops
dispatching at once and holds the runs the same way.

The whole handover is bounded too: `daemon.restart_max_minutes` (default
90), counted from the moment the restart is armed, caps the drain, the stay
for the owner's queued work and every owner's run that anchors the window
again. Once it is up nothing new starts, the owner's work included; the
daemon says `restart pending for 1h30m0s (daemon.restart_max_minutes):
nothing new starts, the owner's work included; exiting with code 3 after the
N run(s) in flight finish or hold, the next daemon starts the queue first`
and leaves as at the drain's end - the runs in flight hold as
`daemon.restart_hold` says, or are waited out under the console's
**Restart** without "now". The fleet's window never runs past what is left
of the bound either. The measured reason: on 2026-11-26 a restart armed at
05:42Z had not come by 12:07Z, because each run of the owner's work that
started while it was pending anchored the window again and the owner's queue
was never empty; nine merged fixes sat on disk all that time. Ninety minutes
is the thirty-minute drain plus one more drain of stay for the owner's work,
and half an hour past that so an owner's run started late in the window gets
on before it is held. `0` is no bound, the old behaviour. While a restart is
pending the heartbeat's `restart` record carries `since` and `until` (when
the bound is up) beside the runs the new build waits for. What ends the wait
after the bound: the bound itself sets no timer on the runs on this machine.
A run on another machine (a hand or a rented machine) gets its own short
wait, `daemon.restart_remote_minutes` (default 10), counted from the bound:
the daemon says `restart: waiting for N run(s) on other machines (N on
<machine>) at most 10m0s more (daemon.restart_remote_minutes); ...`, and when
that is up it fetches each run's commits as kept work (branch
`hive/<agent>/<job>-failed` and a patch, the job's next run starts from
them), stops the run and returns its job to the queue with the attempt
refunded - the run summary reads `stopped on <machine>: the restart waited
daemon.restart_remote_minutes past its bound ...`. Such a run never holds
the daemon: on 2026-11-27 a daemon waited for seven runs on hands-cr-11116
from 06:21Z with nothing said. `0` stops them as soon as the bound is up.
The runs on this machine are asked to hold (`daemon.restart_hold`) only when
the daemon is about to leave - once no run on another machine is left - not
when the wait begins, so a run that ends in that time ends whole. The daemon
then waits for its runs in flight to end, or to hold as `daemon.restart_hold`
says (a held run exits within moments; a run past its harness - in its
check, or a custom worker - runs to its end); an owner's **Restart now**
written during that wait is heard within two seconds and starts the timer of
`daemon.hold_seconds`, after which the runs still going are stopped; the
console's **Restart** without "now" waits the runs out. A run on another
machine (a paired or rented hand) cannot hold: no harness of this daemon
runs it, so it is stopped with its commits kept when
`daemon.restart_remote_minutes` is up, as above. So the longest wait after
the bound is `daemon.restart_remote_minutes` plus the longest run here in
its check, unless the owner asks **Restart now**. `hive status`
reads them on its daemon line (`restart since <time> (<how long>), waits for
<runs>; nothing new starts after <until> (daemon.restart_max_minutes)`, and
once the daemon waits for its runs `restart waits for 1 run(s) on this
machine since 06:21Z, 7 run(s) on hands-cr-11116 since 06:21Z (stopped at
06:31Z, their commits kept: daemon.restart_remote_minutes); Restart now
(console: Settings -> This machine) ends the wait`) and the console's
**Update waiting** row under **Settings → This machine** says how long the
restart has been pending, when new runs stop starting and, while the daemon
waits, what for and since when, with **Restart now** beside it. The build itself is a
full release compile, so it keeps out of the runs' way: it runs at nice 10
with `cargo -j` set to `daemon.build_jobs` (half the machine's cores by
default), and it does not start while the load governor (below) says the
machine is saturated or every local slot of this machine holds a run -
the daemon then says `self-update: brain HEAD <rev> differs from the running
<rev>; building once the machine has room: <why>` once, keeps one local slot
free so the build starts within one run, and the experiment lane measures
nothing new meanwhile, as when the build waits for the lane. Nor does it start
until the brain's HEAD has stood unchanged for `daemon.self_update_settle_minutes`
(default 60; 0 builds at once): a fleet merges a burst of brain branches
minutes apart, and five release builds a day, each followed by a restart
window that refuses the long kinds of run, were the price of building on every
merge - the daemon says `building once it has stood for 60 min` once, and a
further merge restarts the clock, so the burst builds once, from its last
commit. Merges closer together than the settle time would restart that clock
for ever, so the settle wait gives way once the default branch has been ahead
of the running build for `daemon.restart_max_minutes` (an hour when it is 0),
counted from the first pass that saw it ahead whatever HEAD it then stood at.
Nor
does it start while a fleet improve, task or experiment job is queued that the
restart the build ends in would refuse (its run is not expected to end before
the runs in flight): the daemon says `building once the queued improve runs on
<project> have started, ..., or at <time> (daemon.restart_max_minutes)` once
and lets them start first, for at most `daemon.restart_max_minutes` (90; an
hour when it is 0). The wait counts from the first pass that waited, or from
the arming of a restart already pending when that is earlier, and a queue that
empties for a pass and fills again does not start it over; nor does a pass
that waits for the settle time instead (a new HEAD in between), since with a
bound the wait counts from when the default branch went ahead at the latest. Past it the daemon
says `building past the queued task runs on <project>, waited for since <time>`
and builds: the queued work stays queued and starts under the next daemon. On
2026-11-26 a priority-0 audit queued on the brain behind the owner's tasks held
every build for seven hours.
The owner's queued work is never among those: the restart does not refuse
it, so the build does not wait for it.
A merge that touches nothing the binary is built from is not built at all:
the daemon diffs the brain's HEAD against the revision it runs - or, when a
build or a hand install already waits for its restart, against that installed
revision, since it is the one the daemon runs next - and when every
changed path lies outside the crates, the workspace `Cargo.toml` and
`Cargo.lock`, the toolchain and `.cargo/` files and the console's sources
under `web/hub/` (`src/`, `public/` and the package and config files;
hive-hub's build script builds the export from them and embeds it into the
binary, so a console change does change `hive`; the mock API does not), it
says `self-update: brain HEAD <rev> differs from the running <rev> only
outside what the binary is built from (N file(s):
a, b, c and K more); no build` (`the installed <rev>` when that was the
base) once and keeps the binary it has. The next
merge is diffed against the running or installed revision again, not against
that one, so a change under `crates/` that follows a docs merge is built with
both.
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
three of them the final link of `hive`, which prints nothing: a build that
still prints is still compiling. The smoke tests (`version`, `help`) answer
within a second and keep the ten-minute bound on their length.
A failed build or smoke test leaves the running binary untouched, posts a
finding (`hive <stage> of <rev> failed; the running binary is untouched`)
and waits `daemon.self_update_retry_minutes` (default 60, at least 1) before
the daemon builds again; the console's settings list it under **Updates**, and
the hub's running report carries the same wait as `retry_minutes`
(crates/hive-runner/src/selfupdate.rs, crates/hive-hub/src/running.rs).
The install keeps a copy of the running binary as `hive.prev` and puts the
new file in its place; the running one is never renamed. macOS names a
process by the current path of the file it runs from, so a daemon renamed to
`hive.prev` while it drained its restart hold had no Desktop-folder grant,
and the prompt that raised (unanswered for eighty minutes, 2026-11-05) held
every later hive request behind it. A build that then exits three times early is put back to
the previous one by the launcher, which turns `daemon.self_update` off and
leaves a finding. A revision already installed by hand is not built again:
when `~/.hive/bin/hive version` prints HEAD, the daemon says so once and
restarts onto it after the runs in flight. Set `daemon.self_update` to `off` to keep the build you
have and `cargo install --path crates/hive --root ~/.hive --locked` by hand.

Self-update follows the brain's checkout; it does not say when a newer
published release exists. For that, point `update.latest_url` at a JSON
document naming the latest release - a GitHub `releases/latest` URL
(`https://api.github.com/repos/OWNER/REPO/releases/latest`), a GitHub
releases list (drafts and pre-releases are skipped), or
`{"version": "v1.2.3", "url": "https://..."}`. The maintenance pass then
reads it at most once a day (at once after the setting changes), keeps what
it found in the store's `meta` row `update`, and logs one line when a newer
release first shows up. `hive version` prints a second line,
`A newer Hive is available: v1.2.3 (running v1.2.2) - <release link>`
(`--json` adds an `update` object), and the console shows a banner above
every page except chat, with the release link and a button that hides it
until the next release (`GET /api/v1/update`). Versions compare by semver
precedence, so a pseudo-version build after `v1.2.2` (`v1.2.3-0.<stamp>`)
is behind `v1.2.3`, and a `dev` build is never told it is behind. A failed
read keeps the release it last knew and logs the reason once. The check
itself downloads and installs nothing; `hive update` installs the release,
checksum-verified, on a machine whose `update.channel` is `release`, and
`update.channel release` also stops the source builds above
([Releases and `hive update`](release.md)). The setting is empty by
default: no check and no network request by the daemon. `hive update` and
`hive update --check`, run by hand, then look at Hive's own releases for
that one run (crates/hive-cli/src/update.rs,
crates/hive-runner/src/update_check.rs).

The owner need not wait for the drain. **Restart now** (in the **Update
waiting** row under **Settings → This machine**;
`POST /api/v1/running/restart`) drains like a self-update by default
(**Restart when they finish**): nothing new starts, the runs in flight are
waited out, the daemon keeps its heartbeat and exits with code 3 once they
end or `daemon.restart_drain_minutes` is up (with nothing in flight it exits
at once); `daemon.restart_max_minutes` bounds it the same way. The dialog's **Hold the runs and restart now instead** (a payload
of `{"now": true}`) asks the daemon to restart at once with the
work held rather than cut: every run in flight is asked to hold, its harness
saves the agent's conversation and exits `held` (code 75), the run settles
`held`, and its job goes back to the queue with the attempt refunded and the
run's clone and push key kept. The next run of that job - under the same
agent, which the runner dispatches first once it is free and which the queue
waits for while that agent is active and free - picks
the conversation back up (`hive harness run --resume`) with a note of where
it was interrupted, in the same clone, and the agent carries on instead of
starting over; the held session is marked `resumed`. A hold that reaches the
harness after the worker finished the job (`hive job finish`) is a stop
notice on a job already done: the finish stands, the held exit is not a
verdict, the session is settled and the run is judged by the finish - taken
as a hold, the requeue overwrote the `done` (2026-11-05: an owner's task
finished and verified at 06:13:37Z was requeued at 06:13:50Z and its
161-step work lost to a fresh clone). A run that cannot hold
(a custom worker command, a run still in its recipe setup or already in its
run-end check, a remote executor, an agent gone inactive) is stopped after
`daemon.hold_seconds` (default 120) and settles `lost`: its job returns to
the queue and starts over - unless the run was in its run-end check, its
worker's work finished. Then the stop keeps the commits on the branch
`hive/<agent>/<job>-unchecked` in the project's checkout (not the job's own
`hive/<agent>/<job>`, which the landing refuses when it exists) and writes a
`run.unchecked` event on the job with the base, the signing key and the
worker's report lines; the job returns to the queue at no cost of an attempt,
and its next run on this machine checks out that branch, starts no worker,
and checks and lands the commits as the stopped run would have, then deletes
the branch. A task amended after the work was kept is named in that run's
log, since the kept commits predate the amendment - counted from the stop
that first kept them, not from a later record of the same work. When the commits could
not be fetched onto the branch, the event names the stopped run's clone
instead and the clone stays; the next run fetches the commits from it before
it makes its own clone, and when that fetch fails too it ends with a runner
error and leaves the clone alone (the summary gives the `git fetch` that
gets them by hand). Once the job's attempts are spent and it has failed, the
sweep saves the commits still in that clone as a patch under
`patches/<project>/` before it removes the clone, and keeps a clone whose
commits it cannot save. A job with such kept work is leased to this machine's
slots only, as a job with a session held here is (a hand would do the task
again from nothing), and a starved queue does not rent a machine for it -
but only while this machine could take the job: one that needs something
this machine lacks, or a plain job while this machine is kept for the work
that needs something (`machine.reserve`), goes to a hand and starts over
rather than wait for a lease that never comes. A stop that is the owner's
cancel - of the run, of its task or of the job itself (`hive job finish ...
cancelled` while it ran) - keeps nothing and does not put the job back in
the queue; nor does a
stop during a card's proof (`evolve.require_check`) count as the card's check
failing - it is a stopped check like any other (crates/hive-runner/src/unchecked.rs).
On a hand the stop ends the ssh session the
worker or the check runs in and the run's clean-up tells the hand to clean
the job, which ends what the worker left running there; until 2026-11-05 a
hand's run could not be stopped at all, and a restart with one in flight
waited on the hand's budget (twenty hours for a task). A delivery pass or measurement in flight gets the
same cap before it is cancelled; the next daemon delivers or measures again. The
ask is heard while the daemon waits for its runs too: a daemon whose drain
is over and that waits for a run past its harness reads the ask every two
seconds (log line `restart asked by <who> at <when> while this daemon waits
for N run(s) in flight: M asked to hold; stopping the rest after <cap>`) -
until 2026-11-04 only the loop read it, so an owner pressing **Restart now**
during that wait was not heard until the run ended, seven times in a day. The
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
how deep sub-sessions nest and how many a session keeps open; `0`
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
(`hive start` in a terminal). It does not start a missing service, remove
file STOPs or change caps. **Silent** means the loop heartbeat is stale, not
that the daemon is disabled: it may still be running or waiting. A loop that
stays silent for half an hour (six intervals when the interval is longer) is
treated as hung: the daemon writes its own thread's stack and thread count to
`~/.hive/logs/daemon-stall-<time>.log`, notes the stall in its heartbeat and
event log, cancels the runs in flight and exits 3, so the service starts it
again; `kill -USR1 <pid>` writes the same dump without a restart. The controls
show the recorded PID, service state and diagnostic commands rather than
starting a duplicate daemon. `daemon.self_update = off` only disables
automatic rebuilds; it is not the daemon's enable switch.

## macOS folder access

On macOS, a checkout under `Desktop`, `Documents` or `Downloads` makes the
system ask once whether `hive` may access that folder the first time the
daemon touches it; answer Allow (the loop waits on the dialog, and `hive
status` reports the daemon as silent until then). macOS ties that answer to
the binary's code-signing requirement, and a plain `cargo build`'s requirement
is its own hash - so every rebuild would ask again. `hive service install`
therefore re-signs the installed `hive` with a requirement every build meets,
`identifier "hive"`, and the daemon signs each build it installs the same way:
no certificate, no password, and the answer holds. Checkouts elsewhere never
ask. A `hive` that Homebrew installed is the exception: the file is
Homebrew's, so Hive does not sign it again, and every version is a new file
in a new place, so macOS may ask again after `brew upgrade`
([release.md](release.md#homebrew)). The services run hive through small `/bin/sh` launchers
(`~/.hive/bin/hive-daemon.sh`, `hive-hub.sh`) so that a restart is a loop
and the crash guard has a home.

## Bulk storage on another disk

Most of what Hive writes is regenerable bulk: the clones under `work/`, the
build caches, the worktrees, and the `target/` of a checkout that builds
itself. On a laptop with a small internal disk that bulk can live on an
external drive; the board and its history never do.

```sh
hive bulk attach /Volumes/Kingston   # stop the service, move the bulk, link it, start again
hive bulk status                     # mounted at /Volumes/Kingston/hive (103201 MB free); ...
hive bulk detach                     # bring everything back onto this disk
hive bulk detach --lost              # the drive is gone for good: start with empty dirs
```

`attach` formats nothing. It takes a mounted volume (a plain folder only with
`--any-folder`), creates `<volume>/hive` with a marker file carrying an id
that `~/.hive/bulk.id` repeats, moves `work`, `cache` and `worktrees`
there, and leaves one symlink behind - `~/.hive/bulk` - that the three names
go through (`work -> bulk/work`). It also makes `<volume>/hive/cargo-target/`
for a checkout's build dir: `ln -s ~/.hive/bulk/cargo-target/<name>
<checkout>/target` keeps a self-building brain off the internal disk too.
Everything else stays home: `hive.sqlite` and its backups, patches,
measurements, the harness, the services' logs and launchers, and `tmp/` with
the short-lived credential files - a removable disk has no reliable ownership
bits, so secrets never go there. `attach` and `detach` refuse while a daemon
runs outside the service, stop the service for the move and start it again;
`--dry-run` lists what would move.

Pulling the drive, or logging in without it, is safe at any moment and needs
no `launchctl` first. The link simply dangles, and a dangling link creates
nothing: `mkdir -p`, `git clone` and every `create_dir_all` fail on it instead
of filling the internal disk. The daemon notices before each pass, holds every
dispatch, delivery, sweep, reclaim, experiment lane and self-update with one
line - `idle: bulk storage away: /Volumes/Kingston/hive is not mounted; all
work waits for the disk` - and the console shows the same hold in red on the
Overview with the one remedy, plug it back in. It never exits, so the service's
crash guard never mistakes the missing disk for a bad build and rolls the binary
back. A run that the disk vanished under ends as an executor fault: the job
keeps its attempt and runs again once the disk is back. When the volume returns
the next pass logs `bulk storage back; resuming` and work continues; when
macOS mounts it under a new name (`Kingston 1`) the daemon finds the marker on
the sibling volume and repoints the link itself. `hive status` prints a
`bulk` line while anything is attached, and `hive executor probe` names the
missing disk instead of measuring a 0 MB floor.

## Backups of the store

The board, its history, every task and every job live in one file,
`~/.hive/hive.sqlite`, and nothing above regenerates it. When a disk starts
failing the only copy must already be somewhere else, so the runner can keep
one there. Name a place with `backup.to` and the maintenance pass copies the
store to it every `backup.every_hours` (24 by default), keeping the newest
`backup.keep` copies (14) and removing older ones of this host's:

```bash
hive settings set backup.to az://myaccount/hive-backups/store   # an Azure blob container, with an optional prefix
hive settings set backup.to /Volumes/Backup/hive                 # a folder that already exists (a mounted disk, a synced folder)
hive settings set backup.to 'run:rclone copyto {file} remote:hive/{name}'   # any command; {file} and {name} are filled in
hive backup                       # one now, to backup.to
hive backup --to ~/Desktop --keep 3   # one now, somewhere else, pruning to 3 there
hive backup --status              # where copies go, the last outcome, when the next is due (also --json)
```

Each copy is a `VACUUM INTO` snapshot (writers are never blocked), checked with
`PRAGMA quick_check` before it leaves the machine, gzipped, hashed, and named
`hive-<host>-<stamp>.sqlite.gz`; the folder destination writes a `.part` file
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

Old ledger rows nothing reads any more are not deleted but moved, once a month,
to `~/.hive/hive-archive.sqlite` next to the store (`daemon.archive_days`,
90 by default; `hive store archive --dry-run` shows what would go); see
"Store archive" in [recovery.md](recovery.md). A backup copies `hive.sqlite`
only, so the side file is a separate copy to keep if old history matters.

## Keeping the machine awake

A laptop that dozes off takes its agents with it: a run that worked for an
hour loses the whole hour when the lid closes or the idle timer fires, the
timeline shows a gap, and the trends dip. `daemon.keep_awake` is Hive's own
caffeination mode. `off` (the default) leaves the power settings alone. `on`
holds the machine awake while the daemon runs and the machine is plugged in;
`always` holds it on battery too. The hold is a child of the daemon - on
macOS `caffeinate -d -i -s -w <daemon pid>`, the same assertions Copilot
CLI's `/keep-alive` takes (`-d` keeps the display awake, `-i` stops idle
sleep, `-s` stops system sleep on AC power); on Linux a `systemd-inhibit
--what=sleep:idle --mode=block` that waits on the daemon's pid - so it ends
the moment the daemon exits, is paused with `hive stop`, or restarts under a
new build, and it never outlives Hive. A closed lid still sleeps a Mac
unless an external display is attached; the daemon's log line says so
(`keeping this machine awake while Hive runs (caffeinate -d -i -s -w <pid>,
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
Hive used before - stopped sleep but let the saver come on every three
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
