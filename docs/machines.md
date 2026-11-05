# Machines and paired hands

## Machines: what runs where

Work runs on more than one machine: this one, the Linux hands `pulse pair`
adds, the machines compute rents. They are not alike - the iOS app needs a
Mac with Xcode, a model evaluation wants a GPU, most jobs want nothing in
particular - so a job can say what it **needs** and a machine says what it
**offers**, and the daemon puts each job on a machine that offers everything
it needs, keeping the scarce machines for the work that needs them.

Some jobs never leave this machine whatever they need: lead, review,
reflect and suggestion-review jobs run `pulse` commands against this
machine's store, and a hand has no store, so they lease only here.

Needs and offers are the same plain lowercase words, with no fixed list:
`macos`, `linux`, `windows`; `arm64`, `amd64`; `xcode`, `docker`, `gpu`,
`android`, the name of a tool. The common spellings fold into one
(`darwin`/`osx`/`mac` → `macos`, `x86_64`/`intel` → `amd64`,
`aarch64`/`apple-silicon` → `arm64`, `ubuntu`/`debian` → `linux`). A job
fits a machine when every word it needs is among the words the machine
offers; a job that needs nothing fits every machine that is not reserved.

**What a machine offers** is its OS word, its architecture word and its
labels. This machine reads its own OS and chip and finds `xcode`
(`xcode-select -p` inside `Xcode.app`) and `docker` (on the path) by itself;
you add the rest with the setting `machine.labels` (`pulse settings set
machine.labels gpu,android`). A paired hand reports its OS and chip and what
it found on every probe (`pulse executor probe NAME`; hands paired before
this count as Linux until their next probe, the only platform `pulse pair`
takes), and you label it with `pulse executor label NAME gpu,android`
(replaces your words, keeps the found ones) or `pulse executor label NAME
+cuda,-gpu` (adds and removes); `pulse executor add NAME --labels a,b`
labels it from the start, and so does the console's **Add machine** dialog
(its *Labels* field). `pulse executor list` shows every machine's
`offers …` line, and the hub's Machines page the same.

**What a job needs** comes from its project by default: `pulse init PATH
--needs macos,xcode`, or the `needs:` line of the project's
[`.pulse/pulse.yaml`](board.md#the-projects-own-files-pulse), sets the words
every new job of the project inherits, and changing them moves the project's
queued jobs along (a running job keeps its machine). One job can differ:
`pulse job add --needs linux,gpu …` at creation, `pulse job needs JOB_ID
WORDS` while it waits (`-` clears). A word that is not a word (`x/code`) is
refused where it is typed and is a manifest problem when it is committed.
In the console, the project page's **Runs on** panel shows the words, which
paired machines fit and what the others lack, and a warning when no active
machine fits; **Change** edits the words with the ones the machines offer as
suggestions (`GET`/`POST /api/v1/projects/<slug>/needs`, `{"needs": "macos,
xcode"}`, an empty string for any machine; the answer names the machines
that fit). A task's job rows say what each job needs while it waits.

**Where a job goes.** Each tick the daemon offers the free slots in this
order: the reserved machines first (they take only the jobs that need
them), then the machines kept for certain projects (`--projects`), then the
machine that offers the *least* - a plain Linux hand before the hand with
a GPU before the Mac with Xcode - so the scarce machine stays free for the
work only it can do, and among equals the least loaded one, counting the
slot about to be filled. Each machine then leases the oldest highest-priority
job that fits it, with one change to the usual order: a job that needs
something comes before a job that needs nothing, because fewer machines can
take it. `machine.reserve on` (this machine) or `pulse executor reserve
NAME` (a hand; `--off` to release it) makes a machine take needy jobs only -
the Mac works the iOS app and the Linux hands take everything else. A
machine whose queue holds jobs it cannot take says so in its idle line
(`queued: 2 need macos,xcode, which linux-1 does not offer (it offers linux
amd64; pulse executor list)`).

**When no machine fits.** A job whose needs no active machine offers waits
in the queue. After ten minutes, when the needs are what a rented Linux
machine offers (`linux` and the size's architecture) and
`compute.auto_approve` is on within `compute.max_vms` and
`compute.max_hours_per_day`, the daemon rents one on the spot, whether or
not this machine has free slots (a Mac with idle slots and a `linux` job
used to tell you "machine wanted" and leave the job queued; the rental only
came for a queue with every slot busy). For the needs it cannot rent (never
a Mac, never a label such as `gpu`) or may not (the setting off, a cap
reached), it asks you once per project and set of words, on the brain's
`runner/machines` thread: pair or label a machine that offers them, or
change the needs (`pulse job needs`, the manifest, `pulse init --needs`).
The question stays open until one of those happens; a second group of the
same words does not ask again. While a rented machine is on its way the log
says so (`machine wanted: compute request CR-012 is provisioning; waiting
for the machine`) and nothing else is asked for.

**What a rented machine has.** It starts from the image: git, Python 3,
build tools, mise. Everything else a project's jobs need comes from the
project's environment recipe (`docs/context/environment.md`, `pulse env
set`): the setup installs its `tools:` pins through mise on the machine's
first job (`rust@1.95` took 22 s on a Standard_B2ms), and the registered
check should name any shared directory from `$HOME`, since a hand's home is
not the hub's (an absolute Mac path cannot be created there). A project
whose recipe pins nothing and whose setup needs a tool the image lacks marks
the machine unfit, which then idles and is released; the daemon does not
rent again for that project until the recipe changes - the idle line says
`renting again would meet the same lack: hands-cr-064 lacks cargo for
amistio-template … pin the tool in tools:` - and rents for the next waiting
project instead. Measured before this rule: CR-064 lacked `cargo` for the
brain, was released after 30 idle minutes, and CR-065 was rented for the
same job within the minute.

```bash
pulse settings set machine.labels gpu,android          # what this machine offers, beyond its OS and chip and what it finds
pulse settings set machine.reserve on                  # this machine takes only the jobs that need something it offers
pulse executor add gpu-box --ssh pulse@gpu-box --workdir /srv/pulse --labels gpu
pulse executor label gpu-box +cuda                     # add a word; -word removes one; a plain list replaces yours
pulse executor reserve gpu-box                         # keep it for the jobs that need it; --off to release
pulse executor list                                    # every machine, its slots and an `offers …` line
pulse init ~/code/ios-app --needs macos,xcode          # every job of the project needs a Mac with Xcode
pulse job add --project ml --title 'Train' --brief … --needs linux,gpu
pulse job needs JOB_ID -                               # this one job may run anywhere after all
```

## Pairing hands

`pulse pair ADMIN@HOST --name NAME [--workdir /srv/pulse] [--workers N]
[--projects a,b|*]` turns a Linux machine into an executor in one run, from
the account that runs the daemon. `--workers` is only the starting count: each
probe reads the machine's cores and memory (`cores=` and `mem_mb=` in the
hands' probe line) and sets its slots to the smaller of the cores and one run
per 3 GB, the same rule this machine uses, so the fleet's total grows with every
machine paired. Over the admin's ssh session it reads the
machine first (`uname`, `sudo`, a `pulse` user, installed hands, the workdir,
`authorized_keys`, the sshd drop-in, git, …) and refuses before
changing anything when the machine is not fit: not Linux, no passwordless
sudo, a `pulse` `authorized_keys` with other content, clones with git remotes
under the workdir, an sshd without `Include sshd_config.d` (the manual Match
block is printed). Then, in this order:

- **Here**, idempotently: `~/.ssh/pulse_hands_ed25519` is generated if it is
  missing; a `Host NAME` block (HostName, Port, `User pulse`, that key) is
  appended to `~/.ssh/config` only if an identical block is not there yet (the
  previous file is kept as `~/.ssh/config.bak` on the first change); the
  machine's host keys are added to `~/.ssh/known_hosts` only if absent. When
  HOST is itself an alias in that config (compute pairs through the
  `Host NAME-admin` block it wrote), the block and the known_hosts lines carry
  the address `ssh -G` resolves it to - OpenSSH does not follow a `HostName`
  into another `Host` block, so `ssh NAME` would otherwise fail to resolve the
  alias right after the apply step reached the machine through it. A
  second run changes none of them.
- **There, as root**: the apply script `--dry-run` prints is streamed to `ssh
  -p PORT ADMIN@HOST sudo -n sh -s` (ten minutes at most). It creates the
  `pulse` user, installs the hands binary as `/usr/local/bin/pulse-hands` with
  `/usr/local/bin/pulse` linked to it, makes the workdir, writes the one
  `command="… hands",restrict` line in its `authorized_keys`, a gitconfig and
  `mise`, then the drop-in `/etc/ssh/sshd_config.d/60-pulse-hands.conf` and reloads
  sshd. Every step reports `kept` or `done`, so a re-pair is safe. `sudo -n`
  never prompts: when sudo wants a password the run stops with the dry-run
  output and the way out (a `NOPASSWD` line for the admin, or run the printed
  script as root yourself and then `pulse executor add NAME --ssh pulse@NAME
  --workdir D --workers N --projects P`).
- **In the store**: the executor is registered exactly as `pulse executor add
  NAME --ssh pulse@NAME --workdir D --workers N --projects P` would; a name
  that already exists is updated (address, workdir, workers, projects) instead
  of refused, which is how you re-pair after a reinstall or change the worker
  count.
- **The probe** is `pulse executor probe NAME`, in the same process. When it
  passes, the commit script removes the drop-in's `.prev` copy there, the
  executor is `active` and one screen sums it up (host, user, workdir,
  workers, projects, the probe line, the undo one-liner). When the machine is reached
  but not ready - no harness login, disk under the floor - the pairing
  stays, the executor stays `drained` with the probe note, the summary names
  the next step (`pulse executor login NAME --admin ADMIN@HOST --with-token`,
  or fix it and `pulse executor probe NAME`) and the exit code is 1. The
  daemon pairs a machine it bought the same way, except that the login is
  its own next step: a probe that lacks only the harness login leaves the
  hand paired and waiting (`paired NAME: drained (the harness login comes
  next)`, the report ok, the `pair.applied` event marked `login_next`), the
  daemon places the login and probes again, and that probe decides. The
  login goes through the same `Host NAME-admin` alias the pairing used, and
  that alias is the admin address the executor keeps: the machine answers to
  the hands key alone, and only the alias block names it. When the
  hand does not answer
  at all - ssh refused, the forced command missing, the wrong workdir - the
  undo script runs over the admin session (the drop-in this run wrote is
  restored or removed and sshd reloaded; a drop-in it kept is left alone), a
  row this run created is removed from the store (a re-paired one is
  drained), the local ssh files stay as they are (the block is harmless), and
  the run exits 1 with the probe output and `ssh -v NAME` as the next check.

A run that applied records one `pair.applied` event with the host, the
executor and the probe outcome (`active`, `drained` or `undone`); refusals
and failed applies record nothing. `pulse remove executor NAME` is the undo
for a paired hand.

Every ssh session the hub opens to one address (user, host and port)
shares one connection: the first opens a master that stays ten minutes
past its last session, the rest attach to it through a socket under
`PULSE_HOME/ssh`, and a session whose master cannot be reached connects on
its own, as before (a daemon that starts removes the sockets it finds, since
a master from before a reboot is gone). The measured reason: of twenty
fresh connects the hub made to `hands-cr-064` in 77 seconds, the
carrier-grade NAT it sits behind answered eighteen with a reset before the
packet left its network - a capture on the machine saw one SYN arrive and
sent no reset - and the refusals went on for four minutes after the
pairing's twenty-odd short sessions, while the connection that was up kept
working; a probe, a job start, a steps mirror and a fetch now cost no new
connect. The drop-in a pairing writes raises sshd's sessions per connection
for the hands' user to 64 (the default is ten), so the sessions the hub has
open at once on one hand share the bound comfortably; a session a full
master cannot serve is tried again like a refused connect.

A session that never began - the connect refused or timed out, the banner
exchange cut, as a network behind carrier-grade NAT does to a burst of
fresh connections - or that a shared connection's master could not serve
is tried again before anything counts it: five more times with a doubling
pause (2, 4, 8, 16, 32 s - a minute in all, which outlasts the burst a
daemon restart itself causes when every probe, lead and worker it starts
opens a fresh connection at once), for every ssh the hub makes, the pairing's
inspection and apply included (one such refusal between the two used to
fail the pairing and release a machine that had just booted; four tries used
to run out inside the restart burst and fail a lead's task three times on a
healthy hand). A hand that
still stops answering mid-run - three
transport errors in a row after those tries - goes `unreachable` and gets
no work. The daemon probes it
again after a minute and doubles the wait after each probe that still fails,
up to thirty minutes; the first probe that passes puts it back in rotation
(`executor NAME is reachable again: probe passed, back in rotation: …` in the
daemon's log). `pulse executor probe NAME` does the same at once.

### Adding a machine from the console

The Machines page's **Add machine** button is the same three commands behind
one route, `POST /api/v1/compute/executors`, told apart by `mode`:

- `pair` (the default; `{"mode": "pair", "name": "gpu-box", "admin":
  "ubuntu@10.0.0.9", "workdir": "/srv/pulse", "workers": 1, "projects": "*",
  "labels": "gpu", "dry_run": false}`) runs `pulse pair` on the hub, from
  the account that runs it, with the same checks and the same undo. The name,
  address, workdir, worker count and labels are checked on the request, so a
  bad one answers 400 and starts nothing; a name that is already an executor,
  or already pairing, answers 409. Otherwise the answer is 202 with the
  pairing, and `GET /api/v1/compute` lists it under `pairings` (newest first:
  `name`, `admin`, `by`, `started`, `finished`, `status` `running`/`ok`/
  `failed`, `ok`, `dry_run`, `lines` - the report as it grows - and
  `changed`, the local ssh files it touched). The console shows a finished
  pairing's report above the machines until
  `POST /api/v1/compute/pairings/<name>/dismiss` (409 while it runs); a dry
  run leaves the remote and undo scripts in `lines` and changes nothing.
- `register` (`{"mode": "register", "name": "mac-mini", "ssh":
  "pulse@mac-mini", "workdir": "/srv/pulse", "workers": 2, "projects":
  "ios-app", "labels": "macos arm64 xcode", "probe": true}`) is `pulse
  executor add` for a machine someone prepared by hand - it answers 201 with
  the executor row, `added` (its first `offers …` line) and `probe`
  (`{ok, status, note, line}` when `probe` was true, `null` otherwise). The
  machine stays `drained` until a probe you ask for passes - the dialog's
  probe or `pulse executor probe NAME` after fixing what its note says; the
  runner's own probes wake only a hand that went unreachable.
- `request` (`{"mode": "request", "project": "ml", "size": "small",
  "hours": 4, "reason": "a GPU for the evaluation", "approve": true}`) is
  `pulse compute request` in your name: 201 with the request row, approved
  at once unless `approve` is false, provisioned by `compute.provider` inside
  the caps like any agent's request (the dialog says so when no provider is
  set and points at the setting). `size` falls back to the provider's
  configured size.

Each way records what its command would: `executor.add`, `executor.probe`,
`pair.applied`, `compute.requested`. The request rows `GET /api/v1/compute`
lists carry a `note`: the daemon's last word on each (the provisioning step
in progress, why it failed, why its machine was released; empty once a
machine is ready), which is what the card on the Machines page shows under
the reason. The hub serving one project
(`pulse hub --project`) takes requests for that project only.

### Choosing a cloud

Settings → Machines opens with **Where machines come from**: one card per
cloud, drawn from the cloud registry (`crates/pulse-store/src/clouds/`, one
module per cloud; `GET /api/v1/clouds` answers it with the stored values and
what is still missing). Each cloud names its auth modes and the settings it
needs; picking one writes `compute.provider` and those settings, nothing else.
Azure signs in either as the owner (`compute.azure.auth=login`, a browser
`az login`) or as a service principal (`service-principal`, with
`compute.azure.tenant`, `compute.azure.client_id` and
`compute.azure.client_secret` - a reference such as `keychain://` or
`op://`, never the secret, which PULSE does not read); under a service
principal the runner refuses to provision or to deliver an `az://` backup
while `az` is signed in as any other identity. **Where backups go** reads the
same registry's backup targets (a folder, Azure Blob Storage, a command) and
composes `backup.to` from their parts. A new cloud is one new module listed
in `clouds::clouds()` plus its settings.

`pulse pair ADMIN@HOST --name NAME --inspect` (also `--dry-run`) still reads
the machine and prints the apply, undo and commit scripts and the local
changes without applying any of it. For installed hands it loads or builds
the Linux template locally (`PULSE_HANDS_BINARY`, a `pulse-linux-ARCH` beside
the binary, or `cargo build --release --target ARCH-unknown-linux-musl` from
the checkout - the working directory, the binary's own, or the brain
project's path the daemon builds itself from; `cargo zigbuild` when
`cargo-zigbuild` is installed, which is what links a musl target from macOS),
then reports mismatches as `hands=differs (installed VERSION
SHA7, template VERSION SHA7)`. Versions come from the build metadata embedded
in the binary or a script's `# pulse-hands VERSION` header; `SHA7` is the
first seven hex digits of its SHA-256. Metadata is read, never executed.
Exact hashes of the earlier shipped hands scripts report `hands=older` with
`legacy-COMMIT` versions, including the old `.sh` installation path. Edited
or unrecognized artifacts remain `differs`, even if they claim an older
version; unavailable metadata is `unknown`. These diagnostics do not change
the pairing approval gate or the apply policy.

## Heartbeats and presence

Every agent has a heartbeat, and an agent that stops beating is dead. A beat
is any sign of life recorded under the agent's name: a `pulse` command run as
the agent (`PULSE_AGENT`; read-only commands beat too), the harness post-tool
hook, taking or renewing a job lease, the runner's supervision of its run, or
`pulse agent beat` by hand. Successfully started local workers and the SSH
hands supervisor record an execution heartbeat and refresh it every 60 seconds,
including quiet work and lead jobs; exit, cancellation and timeout stop that
observation. A stop is the execution's last word, not death: the agent stays
`online` while the runner verifies its run and writes its branch, and is `dead` once
`heartbeat.dead_after` has passed since the stop without a fresh beat. While
the runner runs the project's check on the finished worker's clone it beats for
the worker itself (source `verify`, every 60 seconds), so a check longer than
`heartbeat.dead_after` never shows a working run as dead.
Presence is `dead` once the latest beat is older than
`heartbeat.dead_after` seconds (300 by default, never under 30; `pulse settings
set heartbeat.dead_after 600` for slow harnesses) and returns to `online` on
the next beat from any source. A worker or lead with no lease and no running
run is `idle`, not dead; an agent with no sign of life on record is `unknown`;
the owner is not monitored. `pulse agent list`, `pulse agents`, the roster API
and the console show the same presence with the beat's source, age and the
threshold, while retaining their identity/history fields (`--all` still
includes pruned names).

Death has consequences, once, in the daemon's loop: a dead agent's running run
that this daemon is not itself supervising (neither the run nor its job is in
flight here) is closed as `lost` and its job is
queued again (its next lease starts a new generation, so the dead agent's late
writes are fenced; after the usual attempts it fails instead); a job a dead
agent leased without a run is released the same way. A dead session
agent simply shows as dead until it beats again. Heartbeats replace bounded
metadata, not posts or useful-output scores; existing useful-output pruning and
permanently retired names are unchanged.

## Moving a project

`pulse move <slug> <path> [--force]` points a project at the checkout it now
lives in - the folder was moved, the disk it was on is gone, or the
repository was cloned again elsewhere - and keeps its slug, tasks, posts,
runs and records; the `project.moved` event remembers the old path. The
guards are registration's: the path must exist and be a git checkout (its
top level is taken, as `pulse init` does), may not be a checkout the runner
made itself, and may not already be another project's. A checkout whose
`origin` is a different repository than the project's remote is refused -
a different remote usually means the wrong folder - unless `--force`, which
adopts the new remote as the project's. The console's **Change folder…**
(project page menu) and the **Point to its folder** button on a project
marked *Missing on disk* run the same function. Nothing on disk changes,
and jobs that failed with `cannot change to <old path>` run again as the
watchers queue them.

## Removing projects and executors

`pulse remove project <slug>` and `pulse remove executor <name>` show a preview
of what deactivates (nothing is deleted) and ask before applying; `--dry-run`
only previews, `--yes` skips the prompt. `pulse remove project <slug> --forget`
is the hard delete for a registration that is already inactive and whose
checkout no longer exists: it previews the per-table row counts (posts, jobs,
runs, briefs, records, events, harness rows, ...), asks the same way, deletes
them all in one transaction with the project row, and writes a
`project.forgotten` event; it refuses while the project is active, while the
checkout is still on disk or lives on a volume that is not mounted, and while
a run, harness session or leased job of it is in flight. `pulse remove
duplicate-findings [--project SLUG] [--apply] [--yes] [--json]` is dry-run by
default and reports
per-project duplicate groups and rows; with `--apply` it deletes only exact
duplicate system findings (`daemon` or project lead, same
project/area/thread/summary/agent, earliest kept), drops and recreates the post
immutability triggers from their stored definitions, verifies they are present,
and writes a `post.remove.duplicates` event. Human and worker findings are left
alone.
