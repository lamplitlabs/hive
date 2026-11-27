# Machines and paired hands

## Machines: what runs where

Work runs on more than one machine: this one, the Linux hands `hive pair`
adds, the machines compute rents, and the ones that enrol themselves with a
join code. They are not alike - the iOS app needs a
Mac with Xcode, a model evaluation wants a GPU, most jobs want nothing in
particular - so a job can say what it **needs** and a machine says what it
**offers**, and the daemon puts each job on a machine that offers everything
it needs, keeping the scarce machines for the work that needs them.

Some jobs never leave this machine: lead, review, reflect and
suggestion-review jobs run `hive` commands against this machine's store, and
a hand has no store, so they lease only here. They take the project's needs
like every other job of it, so a project whose needs this machine does not
offer has its lead and its reviews wait in the queue, and nothing plans its
tasks. Give a project only words this machine offers too, and put a
narrower need on the one job that has it (`hive job add --needs`).

The machines a job can go to, and how the brain reaches each:

```text
                 ┌────────────────────────────────────┐
                 │ queued jobs, each with its needs   │
                 └──────────────────┬─────────────────┘
                                    │ each job goes to a machine
                                    │ that offers what it needs
          ┌─────────────────────────┼────────────────────────┐
          ↓                         ↓                        ↓
┌────────────────────┐  ┌────────────────────────┐  ┌──────────────────┐
│ this machine,      │  │ a hand over SSH        │  │ an enrolled hand │
│ the brain          │  │ paired by hive pair,   │  │ hive-hand join,  │
│                    │  │ or prepared yourself   │  │ with a join code │
│ lead, review and   │  │                        │  │                  │
│ reflect: only here │  │ the brain reaches in   │  │ dials the brain  │
└────────────────────┘  └────────────────────────┘  └──────────────────┘
```

A machine compute rents is paired the way `hive pair` pairs one, so it is a
hand over SSH.

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
you add the rest with the setting `machine.labels` (`hive settings set
machine.labels gpu,android`). A paired hand reports its OS and chip and what
it found on every probe (`hive executor probe NAME`; hands paired before
this count as Linux until their next probe, the only platform `hive pair`
takes), and you label it with `hive executor label NAME gpu,android`
(replaces your words, keeps the found ones), `hive executor label NAME
+cuda,-gpu` (adds and removes) or `hive executor label NAME -gpu` (removes
one); `hive executor add NAME --labels a,b`
labels it from the start, and so does the console's **Add machine** dialog
(its *Labels* field). `hive executor list` shows every machine's
`offers …` line, and the hub's Machines page the same.

**What a job needs** comes from its project by default: `hive init PATH
--needs macos,xcode`, or the `needs:` line of the project's
[`.hive/hive.yaml`](board.md#the-projects-own-files-hive), sets the words
every new job of the project inherits, and changing them moves the project's
queued jobs along (a running job keeps its machine). One job can differ:
`hive job add --needs linux,gpu …` at creation, `hive job needs JOB_ID
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
take it. `machine.reserve on` (this machine) or `hive executor reserve
NAME` (a hand; `--off` to release it) makes a machine take needy jobs only -
the Mac works the iOS app and the Linux hands take everything else. A
machine whose queue holds jobs it cannot take says so in its idle line
(`queued: 2 need macos,xcode, which linux-1 does not offer (it offers linux
amd64; hive executor list)`).

**A project's lanes are a machine's.** The work lanes a project gets (its
checks taking turns under the verify lock) and its share of the slots are
counted on each machine apart: a run on a paired or rented machine holds that
machine's lane, never one here, and the owner's work is not made to wait
"lanes full" here, nor a run elsewhere asked to hold, while a lane here is
open.

**A check red on a class of machine.** When the end check of a run on a
paired or rented machine finds the project's check red on the base tree
there, the run writes it down for that class of machine (OS and
architecture, such as `linux/amd64`; event `class.base-red`), and the
project's code jobs (task, improve, experiment) keep off every machine of the
class: nothing they did there could land. The jobs that only read still go,
and so does a job whose needs name the class's system (`hive job needs JOB
linux`), such as the one that fixes the check there. A check that passes on
a machine of the class lifts it (`class.base-green`); with no run to say so
it lapses after 24 hours. A compute request is not made for such a project
either, and its reason counts only the jobs the rented machine could take
(`4 job(s) waited 31 min …; 1 of them a rented linux/amd64 machine can run`).

**Stopping a run anywhere.** `hive job stop JOB` stops the job's run on
whatever machine it runs - this one, a paired or a rented one - within a few
seconds, and the job ends cancelled by the owner; `--requeue` puts it back in
the queue instead, the attempt not counted.

**When no machine fits.** A job whose needs no active machine offers waits
in the queue. After ten minutes, when the needs are what a rented Linux
machine offers (`linux` and the size's architecture) and
`compute.auto_approve` is on within `compute.max_vms` and
`compute.max_hours_per_day` (`none` for either is no limit), the daemon rents one on the spot, whether or
not this machine has free slots (a Mac with idle slots and a `linux` job
used to tell you "machine wanted" and leave the job queued; the rental only
came for a queue with every slot busy). For the needs it cannot rent (never
a Mac, never a label such as `gpu`) or may not (the setting off, a cap
reached), it asks you once per project and set of words, on the brain's
`runner/machines` thread: pair or label a machine that offers them, or
change the needs (`hive job needs`, the manifest, `hive init --needs`).
The question stays open until one of those happens; a second group of the
same words does not ask again. While a rented machine is on its way the log
says so (`machine wanted: compute request CR-012 is provisioning; waiting
for the machine`) and nothing else is asked for.

**When a request fails.** A request that fails before its machine is ready
is not followed by another on the next tick. The daemon waits first: 5
minutes after one failure, then 15, 45 and 135 minutes, then 6 hours, for as
long as requests fail one after another. Each failed request counts, whoever
made it and whatever its reason, back to 48 hours before the newest one. A
request that got its machine ends the run; one you denied changes nothing.
The log says until when, and you are asked nothing, since the daemon asks
again by itself: `the last 2 request(s) failed before a machine was ready
(CR-012: …); the daemon asks again after …`. A `compute.max_hours_per_day`
of zero stops it before any of this: it rents nothing at all, waits for
nothing, and the log says `compute.max_hours_per_day 0 would be passed`.
After the wait the daemon asks what the compute worker checks before it
calls the provider: a Linux `hive` for the size's architecture, room under
the caps, a provider this build has. While one of them would fail the next request it
makes none, and the log says why: `the daemon rents none (CR-012 failed
before a machine was ready and the next request would fail too: …)`. It
tells you too, as for any machine it may not rent, and it asks on the first
tick the checks pass. Both lines begin `machine wanted:` when a job needs a
machine no paired one is, and `hands wanted:` when every slot is busy.

The wait and the look at the worker's checks hold only the daemon's own
requests. One you make is filed whatever the last one did. From the console
it goes to the compute worker, which fails it for the same reason while the
cause stands. `hive compute request` has a check of its own: it is refused
before it is filed while no Linux `hive` can be sent (`compute request
refused before a machine: …`). Measured before this rule: 11,034 requests in
under two hours, up to 243 a minute, each failed for want of a Linux `hive`
and replaced on the next tick; and 56 in a row that each bought a machine,
failed 5.6 minutes later on average and released it, 2.8 machine-hours for
nothing.

**What a rented machine has.** It starts from the image: git, Python 3,
build tools, mise. Everything else a project's jobs need comes from the
project's environment recipe (`docs/context/environment.md`, `hive env
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

**Renting only for work that can come back.** A full queue rents a machine
only for the jobs that machine could take: the waiting work jobs whose needs
a rented Linux machine of the size's class (`linux/amd64`) meets and whose
session is not held on this machine. The log and the request count those
(`1 job(s) a rented linux/amd64 machine could run waited 33 min ... (21
queued in all)`); a queue of `needs macos` work rents nothing. And once a
rented machine has run a project's work, the daemon rents no other for that
project until work a rented machine finished there has landed: `no rented
machine has landed work for amistio-template yet (79 run(s) on CR-11116,
CR-11115, CR-11113 and 8 more, none landed), so the daemon rents no other
...`. The first rental for a project is free, another project still gets
its machine, and one you ask for by hand is yours to make. Measured before
these rules (26 and 27 November): eight machines in a row were rented for
the brain while its queued code jobs needed macOS or its check was red on
Linux, and none of their work landed. A project whose work does not build
on Linux says so in its `.hive/hive.yaml` (`needs: macos`), as Hive's own
does.

**When a rented machine goes.** The daemon releases it when the hours it
was asked for are up, counted from the moment the provider made it (the
spin-up is paid for too), or after 30 minutes without a run since it was
ready, whichever comes first. It does not release a machine
with a run in flight, and it stops no run. So once its hours are up a rented
machine takes no new work: the runs in flight on it are its last, and it is
released when they end. Until then the daemon log and the request's card say
so (`compute CR-012: its 4 hour(s) are up: hands-cr-012 takes no new work
and is released when its last run ends`), and no second machine is rented:
the one that is finishing still counts against `compute.max_vms` and as the
open request. A machine can still run past its hours by the length of that
last run, and the day's `compute.max_hours_per_day` counts the hours it ran:
each machine counts the time from when it was made to when it was released
(or until now while it runs), inside the UTC day, rounded up to whole hours;
a machine the sweep releases later as a leftover keeps the time it was made,
so its hours count as well;
an open request also holds the asked hours it has not run yet, and a
pending one the hours it asks. `0` means no machine-hours at all, both when
a request would be approved on its own and when the worker provisions it.
One request asks for at most `compute.max_hours_per_day` hours, and never
more than 24: `hive compute request --hours 2147483647` is refused before it
is filed, with the most it may ask in the error. A sum past what the count
holds is past every cap.
Measured before this rule: two machines approved for 4 hours ran 5.2 and
8.0, and the day counted 4 for each.

**The limits are for agents, not for you.** `compute.max_vms` and
`compute.max_hours_per_day` bound what agents and the daemon rent by
themselves. Your approval lifts them: **Extra machine** in the console
approves your request once you answered its dialog, and a request you approve
(`hive compute approve`, Approve or **Rent it anyway** on the Machines page)
is rented whatever the limits say. Who filed a request does not lift them:
over a limit every request waits for your approval, yours too, and
`compute.auto_approve` approves only a request under them. So
`hive compute request` typed by you over a limit prints `CR-012 pending: ...`
and one `hive compute approve CR-012` rents it. A request or approval from a
run of Hive's - a console conversation's agent, a job's worker, a schedule's
script, a harness session (`HIVE_CONVERSATION`, `HIVE_JOB`, `HIVE_SCHEDULE`
or `HIVE_HARNESS` set), even one typed `--agent human` - is filed under that
run's name, never as yours, and its `hive compute approve` is not your
approval. An agent's or the daemon's request stays capped. `0` is a limit of nothing - Hive rents nothing
by itself, your own requests still go through - and `none` (also `off` or
`unlimited`; **No limit** beside the field in Settings) lifts the limit
altogether; `hive compute list` prints `none` for it. Each field says what
its value means beside it (`0 - Hive rents nothing`), and a change that
leaves no rental possible for agents says so before it is saved.

Over a limit the rent dialog asks before it files anything, with three
answers and no free text: **Rent it anyway** (recommended: your request, the
limit unchanged for agents), **Rent it and raise the limit to N** (the
number that lets this request through), or **Do not rent**. Before that it
reads the waiting jobs (`GET /api/v1/compute/preview?project=P&hours=N`):
`2 waiting job(s) can run on it`, or `No waiting job can run on this
machine: brain needs macos`, and then asks **Do not rent** (recommended),
**Rent it anyway**, or **Let project jobs run on Linux** - which drops the
OS word from the project's needs and its queued jobs, with the risk named:
work that only builds or passes its check on macOS fails on Linux and lands
nothing, and a `needs:` line in the project's `.hive/hive.yaml` sets it
back when it changes.

An agent's or the daemon's request a limit refuses is not a failure. Its row
reads `Not rented: today's limit of 24 hours is used (12 used, this asks
12)` (or `Not rented: the limit of 2 machines at once is reached (2 in use,
this asks 1 more)`) under **Waiting for you**, with the same three answers;
**Rent it anyway** files a new request of yours for the same project, size
and hours (`POST /api/v1/compute/CR-012/rent`, `{"raise": true}` to raise
the limit as well), **Do not rent** ends the question (`.../keep-off`).
After a limit change, Settings offers to rent the requests a limit refused
today (`POST /api/v1/compute/refile-today`). A raised limit is written only
once the request was filed; a request refused before that leaves the limit
as it was. Through the hosted front these writes - `rent`, `approve`,
`refile-today`, and the rent dialog's `{"raise": true}` - take the team
owner's passkey; a member who is not the owner gets 403, and a member's own
**Extra machine** over a limit waits for the owner's approval.

A rented machine released unused says why in its row and on the Machines
page: `Released after 30 idle minutes: no waiting job could run there
(brain needs macos)`, or `... no job was waiting`. Before, every idle
release read `the executor sat idle for 30 minutes`, and a request over a
limit read `Failed: cap: ...` even when you had asked for it yourself.

Once a machine that ran past its hours is released, the daemon's next
request waits as after a failed one: 5 minutes, then 15, 45 and 135, then
6 hours while each released machine ran past its hours or each request
failed (`CR-012's machine ran 8.0 hour(s) on 4 approved and was released
(…); the daemon asks again after …`). One released on time - its release
began within 5 minutes of its hours; the provider's delete may take longer
and is not counted - is followed at once. A queue that still waits gets
a fresh machine on the daemon's usual terms, for another four hours, until
the day's `compute.max_hours_per_day` is reached. So a queue that used to keep one
machine busy long past its hours now gets a new machine each time the hours
are up: over a day fewer hours are rented once the cap binds, and more
machines are bought, each with its own start. Measured before this rule: a
machine asked for 4 hours was released after 21.9, and another after 12.1;
17 of the first one's 22 runs began after its hours were up, because the
queue leased to it again each time a run ended.

```bash
hive settings set machine.labels gpu,android          # what this machine offers, beyond its OS and chip and what it finds
hive settings set machine.reserve on                  # this machine takes only the jobs that need something it offers
hive settings set executor.transport local-hand      # this machine's jobs go to its hive-hand, which pushes their branch
hive executor add gpu-box --ssh hive@gpu-box --workdir /srv/hive --labels gpu
hive executor label gpu-box +cuda                     # add a word; a plain list replaces yours
hive executor label gpu-box -cuda                     # remove a word
hive executor reserve gpu-box                         # keep it for the jobs that need it; --off to release
hive executor list                                    # every machine, its slots and an `offers …` line
hive init ~/code/ios-app --needs macos,xcode          # every job of the project needs a Mac with Xcode
hive job add --project ml --title 'Train' --brief … --needs linux,gpu
hive job needs JOB_ID -                               # this one job may run anywhere after all
```

## Pairing hands

`hive pair ADMIN@HOST --name NAME [--workdir /srv/hive] [--workers N]
[--projects a,b|*]` sets up a Linux machine as an executor over the SSH
login of an admin there, who needs passwordless sudo. Run it from the
account that runs the daemon. The name is required. `hive pair` refuses
anything but Linux: a Mac, or any other machine, joins with `hive-hand`
([Enrolling a hand](#enrolling-a-hand)). On a new setup it is not the only
step:

- **It copies a Linux `hive` to the machine, and needs one to copy.** A
  Linux brain with the same processor sends its own program. Otherwise it
  needs one from you: set `HIVE_HANDS_BINARY` to a Linux `hive` for that
  machine's processor (the `hive` inside the release's Linux archive), or
  put that file beside your `hive` as `hive-linux-amd64` or
  `hive-linux-arm64`. From a checkout of Hive's source it can cross-build
  one. With none of these it stops and says so. A file you name with
  `HIVE_HANDS_BINARY`, or one beside your `hive`, is taken before the
  brain's own program. The run says which it took:
  `hands binary: linux/arm64, ... from PATH`, where a Linux brain names its
  own `hive`, and `from HIVE_HANDS_BINARY=PATH` when the variable is set.
- **A newly paired machine has no model login** and takes no work until it
  has one. So the first run on a new machine ends with
  `paired NAME: drained (the probe did not pass)` and
  `probe FAILED: harness login missing`, names the next step and exits with
  status 1. The next step is `hive executor login NAME --admin ADMIN@HOST`,
  then `hive executor probe NAME`.

`--workers` is only the starting count: each
probe reads the machine's cores and memory (`cores=` and `mem_mb=` in the
hands' probe line) and sets its slots to the smaller of the cores and one run
per 3 GB, the same rule this machine uses, so the fleet's total grows with every
machine paired. Over the admin's ssh session it reads the
machine first (`uname`, `sudo`, a `hive` user, installed hands, the workdir,
`authorized_keys`, the sshd drop-in, git, …) and refuses before
changing anything when the machine is not fit: not Linux, no passwordless
sudo, a `hive` `authorized_keys` with other content, clones with git remotes
under the workdir, an sshd without `Include sshd_config.d` (the manual Match
block is printed). Then, in this order:

- **Here**, idempotently: `~/.ssh/hive_hands_ed25519` is generated if it is
  missing; a `Host NAME` block (HostName, Port, `User hive`, that key) is
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
  `hive` user, installs the hands binary as `/usr/local/bin/hive-hands` with
  `/usr/local/bin/hive` linked to it, makes the workdir, writes the one
  `command="… hands",restrict` line in its `authorized_keys`, a gitconfig and
  `mise`, then the drop-in `/etc/ssh/sshd_config.d/60-hive-hands.conf` and reloads
  sshd. The steps it reports, in order, are `user`, `hands`, `hive`,
  `workdir`, `authorized_keys`, `gitconfig`, `mise`, `dropin` and
  `sshd_reload`. Every step reports `kept` or `done` (`sshd_reload` says
  `skipped` when the drop-in did not change), so a re-pair is safe. `sudo -n`
  never prompts: when sudo wants a password the run stops with the dry-run
  output and the way out (a `NOPASSWD` line for the admin, or run the printed
  script as root yourself and then `hive executor add NAME --ssh hive@NAME
  --workdir D --workers N --projects P`).
- **In the store**: the executor is registered exactly as `hive executor add
  NAME --ssh hive@NAME --workdir D --workers N --projects P` would; a name
  that already exists is updated (address, workdir, workers, projects) instead
  of refused, which is how you re-pair after a reinstall or change the worker
  count.
- **The probe** is `hive executor probe NAME`, in the same process. When it
  passes, the commit script removes the drop-in's `.prev` copy there, the
  executor is `active` and one screen sums it up (host, user, workdir,
  workers, projects, the probe line, the undo one-liner). When the machine is reached
  but not ready - no harness login, disk under the floor - the pairing
  stays, the executor stays `drained` with the probe note, the summary names
  the next step (`hive executor login NAME --admin ADMIN@HOST`, with
  `--with-token` to paste a token, then `hive executor probe NAME`; or fix
  it and `hive executor probe NAME`) and the exit code is 1. The
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
and failed applies record nothing. `hive remove executor NAME` is the undo
for a paired hand.

Every ssh session the hub opens to one address (user, host and port)
shares one connection: the first opens a master that stays ten minutes
past its last session, the rest attach to it through a socket under
`HIVE_HOME/ssh`, and a session whose master cannot be reached connects on
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
daemon's log). `hive executor probe NAME` does the same at once.

### Adding a machine from the console

The Machines page's **Add a machine** button offers four ways. The first,
**Run one command on it**, issues a join code for a machine that dials in
([Enrolling a hand](#enrolling-a-hand)). The other three are the shell's
commands behind one route, `POST /api/v1/compute/executors`, told apart by
`mode`:

- `pair` (the default; `{"mode": "pair", "name": "gpu-box", "admin":
  "ubuntu@10.0.0.9", "workdir": "/srv/hive", "workers": 1, "projects": "*",
  "labels": "gpu", "dry_run": false}`) runs `hive pair` on the hub, from
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
  "hive@mac-mini", "workdir": "/srv/hive", "workers": 2, "projects":
  "ios-app", "labels": "macos arm64 xcode", "probe": true}`) is `hive
  executor add` for a machine someone prepared by hand - it answers 201 with
  the executor row, `added` (its first `offers …` line) and `probe`
  (`{ok, status, note, line}` when `probe` was true, `null` otherwise). The
  machine stays `drained` until a probe you ask for passes - the dialog's
  probe or `hive executor probe NAME` after fixing what its note says; the
  runner's own probes wake only a hand that went unreachable.
- `request` (`{"mode": "request", "project": "ml", "size": "small",
  "hours": 4, "reason": "a GPU for the evaluation", "approve": true}`) is
  `hive compute request` in your name: 201 with the request row, approved
  at once unless `approve` is false, provisioned by `compute.provider`; no
  rental limit refuses it, and `"raise": true` also raises the limit it
  passes (the dialog says so when no provider is set and points at the
  setting). `size` falls back to the provider's
  configured size. `project` says whose work the machine takes: a project's
  slug keeps it for that project, and `commons` - **Any project** at the top
  of the dialog's project list, `--project commons` on the command line -
  asks for a machine every project can use. That request is filed on the
  shared board and its machine is paired for `*`, like a machine you add
  for **Every project**. The daemon's notices about it (ready, released,
  failed) are findings on the shared board's `runner/compute` thread; the
  brain's triage of that board leaves them alone, as they are the
  daemon's log, not wishes. The hub refuses **Any project** (409, *Any
  project needs the daemon of this build*) while the daemon's heartbeat
  names another build than the hub's: the two are separate processes and
  differ while one of them waits to restart, and a daemon from before this
  choice would pair the machine for the shared board, which runs no job.
  Restart Hive, or ask for one project.

  A machine for every project runs every project's jobs as one user, so
  one project's job can read what another's left there. That includes a
  job's clone the daemon keeps on the machine when its commits could not
  be fetched back (`the clone on hands-cr-012 is kept: its commits were
  not fetched`): it stays under the machine's workdir until the work is
  recovered, readable by a later job of any project. The same holds for
  any machine you add for **Every project**; keep a machine to the
  projects that may read each other's work when that matters.

Each way records what its command would: `executor.add`, `executor.probe`,
`pair.applied`, `compute.requested`. The request rows `GET /api/v1/compute`
lists carry a `note`: the daemon's last word on each (the provisioning step
in progress, why it failed, that its hours are up while a last run
finishes, why its machine was released; empty while a ready machine is
inside its hours), which is what the card on the Machines page shows under
the reason. The hub serving one project
(`hive hub --project`) takes requests for that project only, so it refuses
one for any project.

### Choosing a cloud

Settings → Machines opens with **Where machines come from**: one card per
cloud, drawn from the cloud registry (`crates/hive-store/src/clouds/`, one
module per cloud; `GET /api/v1/clouds` answers it with the stored values and
what is still missing). Each cloud names its auth modes and the settings it
needs; picking one writes `compute.provider` and those settings, nothing else.
Azure signs in either as the owner (`compute.azure.auth=login`, a browser
`az login`) or as a service principal (`service-principal`, with
`compute.azure.tenant`, `compute.azure.client_id` and
`compute.azure.client_secret` - a reference such as `keychain://` or
`op://`, never the secret, which Hive does not read); under a service
principal the runner refuses to provision or to deliver an `az://` backup
while `az` is signed in as any other identity. **Where backups go** reads the
same registry's backup targets (a folder, Azure Blob Storage, a command) and
composes `backup.to` from their parts. A new cloud is one new module listed
in `clouds::clouds()` plus its settings.

The settings each cloud reads when the runner rents a machine (the wizard
writes them; Settings → Machines shows them; an empty default means "set it
yourself" unless the row says otherwise). `size` on a request falls back to
the provider's size setting - `compute.azure.vm_size`,
`compute.aws.instance_type`, `compute.gcp.machine_type` or
`compute.multipass.size`.

| Setting | Default | What it is |
|---|---|---|
| `compute.azure.subscription` | empty | Azure subscription id or name the runner provisions in (auth is `az login` on this machine) |
| `compute.azure.resource_group` | empty | resource group that holds the provisioned hands |
| `compute.azure.location` | empty | Azure region, e.g. `westeurope` |
| `compute.azure.vm_size` | `Standard_B2ms` | VM size for a provisioned hand |
| `compute.azure.image` | `Ubuntu2404` | VM image alias or URN |
| `compute.aws.profile` | empty | AWS CLI profile the runner provisions with; empty means the CLI's default (auth is `aws configure` or `aws sso login` on this machine) |
| `compute.aws.region` | empty | AWS region the instances are created in, e.g. `eu-west-1` |
| `compute.aws.instance_type` | `t3.medium` | EC2 instance type for a provisioned hand |
| `compute.aws.image` | `/aws/service/canonical/ubuntu/server/24.04/stable/current/amd64/hvm/ebs-gp3/ami-id` | AMI id (`ami-...`) or an SSM public parameter path resolved at launch; the default is Canonical's current Ubuntu 24.04 amd64 image |
| `compute.aws.subnet` | empty | subnet id the instances join; empty means the default VPC's default subnet in the region |
| `compute.aws.security_group` | empty | security group id the instances get; empty means a group named `hive-hands` that allows ssh in, created when missing |
| `compute.gcp.project` | empty | Google Cloud project the instances are created in; empty means gcloud's current project (auth is `gcloud auth login` on this machine) |
| `compute.gcp.zone` | empty | Compute Engine zone, e.g. `europe-west4-a` |
| `compute.gcp.machine_type` | `e2-medium` | machine type for a provisioned hand |
| `compute.gcp.image` | `ubuntu-2404-lts-amd64` | Ubuntu image family in `ubuntu-os-cloud`, or a full image path `projects/P/global/images/I` |
| `compute.gcp.network` | empty | VPC network the instances join; empty means `default`. A firewall rule `hive-hands-ssh` (tcp 22 to the tag `hive-hands`) is created on it when missing |
| `compute.multipass.size` | `cpus=2,mem=4G,disk=32G` | size of a local multipass hand as `cpus=N,mem=SIZE,disk=SIZE` |
| `compute.multipass.image` | `24.04` | multipass image a local hand launches from |

`hive pair ADMIN@HOST --name NAME --inspect` (also `--dry-run`) still reads
the machine and prints the apply, undo and commit scripts and the local
changes without applying any of it. For installed hands it loads or builds
the Linux template locally (`HIVE_HANDS_BINARY`, a `hive-linux-ARCH` beside
the binary, or `cargo build --release --target ARCH-unknown-linux-musl` from
the checkout - the working directory, the binary's own, or the brain
project's path the daemon builds itself from; `cargo zigbuild` when
`cargo-zigbuild` is installed, which is what links a musl target from macOS),
then reports mismatches as `hands=differs (installed VERSION
SHA7, template VERSION SHA7)`. Versions come from the build metadata embedded
in the binary or a script's `# hive-hands VERSION` header; `SHA7` is the
first seven hex digits of its SHA-256. Metadata is read, never executed.
Exact hashes of the earlier shipped hands scripts report `hands=older` with
`legacy-COMMIT` versions, including the old `.sh` installation path. Edited
or unrecognized artifacts remain `differs`, even if they claim an older
version; unavailable metadata is `unknown`. These diagnostics do not change
the pairing approval gate or the apply policy.

## The hand

`hive-hand` is the executor on its own: a second, separately built binary
that holds only what a machine needs to run the jobs it is handed, and none
of what decides which jobs exist (ADR-070). It answers the same
`hive-hands` verbs a paired machine's `hive hands` answers - `probe`,
`prepare` (the job's clone and branch), `key` and `hook` (the run's signing
key and provenance trailers), `brief`, `run` (the worker, inside its budget,
heartbeat and steps written as it goes), `result`, `count`/`dirty`/`changed`,
`verify`, `tool`, `heartbeat` and `steps` (the run's presence snapshot and
its harness steps by offset), `clean`, and `git-upload-pack` so the brain
fetches the branch - one verb per call, the command line in
`SSH_ORIGINAL_COMMAND` (`b64 …` or `git-upload-pack …`) and the payload on
stdin, exactly as behind sshd.

One job on a hand, as the main verbs in the order the brain sends them; each
arrow points the way the data travels:

```text
┌───────────┐                                           ┌──────────────┐
│ the brain │ ── prepare: the branch, a git bundle ───→ │ a hand       │
│           │ ── key: the run's signing key ──────────→ │              │
│           │ ── hook: the pre-commit hook ───────────→ │              │
│           │ ── brief: the brief, on stdin ──────────→ │              │
│           │ ── run: the worker, --task-file ────────→ │ hive harness │
│           │ ←── heartbeat, steps: while it runs ───── │ run, in the  │
│           │ ←── the agent's report ────────────────── │ job's clone  │
│           │ ── verify: the project's check ─────────→ │              │
│           │ ←── the job's commits, by git fetch ───── │              │
│           │ ── clean: removes the job's clone ──────→ │              │
└───────────┘                                           └──────────────┘
```

```
cargo build -p hive-hand --release    # target/release/hive-hand, about 2 MB
HIVE_HANDS_WORKDIR=/srv/hive target/release/hive-hand probe
```

The probe answers `trailers=2` when the hand writes the commit hook of
pull request 299 and later (`addIfDifferent`, read back, `--no-divider`).
Hands that answer less run an older hive: their hook can let a commit out
without its trailers and their harness does not refuse `pkill`. Jobs keep
running there; the brain posts one finding per machine on the
`runner/older-hands` thread, said again only after a probe found the hands
current and a later one finds them older again.

**What it links.** The executor pieces live in `crates/hive-hand-core`,
which `hive-runner` (the brain) and `hive-hand` both build from - the
hands protocol (`hands.rs`), process groups and budgets (`proc.rs`,
`procs.rs`), git helpers, the worktree tidy, the machine's nice level,
build parallelism and hardware, the heartbeat clock, the recipe's name
rules and the provenance hook, the lease path (`transport.rs`), the pairing
protocol helpers (`pair.rs`), the worker TMPDIR preflight, the clone trash,
and a job's local run (`local.rs`: the clone's preparation and signing
identity, the worker's run with its output captured, the run log, the
cleanup or hold after). Nothing there plans, leads, scores or briefs, and
nothing there reads the store: the brain's `LocalExec` hands `local.rs`
small parameter structs (`ClonePrep`, `WorkerRun`, `CloneCleanup`) built
from its job and project rows. `scripts/check-hand-deps.sh` holds that line in the verify command:

- `deps` fails when `cargo tree -p hive-hand -e normal` lists a brain
  crate - hive-runner (with its lead, plan, evolve, tally, research and
  idle modules), hive-evolve, hive-tally, hive-hub, hive-goals,
  hive-goals-command, hive-ask, hive-harness, or the brain's store and
  board;
- `self-test` proves `deps` fails on a scratch manifest whose hand depends
  on a crate named hive-hub, and passes once the line is gone;
- `strings target/release/hive-hand` must find none of three sentinel
  phrases of the worker brief (`crates/hive-runner/src/brief.rs`). The
  `hive-hand` crate's own test (`tests/no_brain.rs`) checks the same
  three, and that the brief still says them.

When a change to the hand needs something of the brain, move the piece into
`hive-hand-core` and have the brain re-export it (`pub use
hive_hand_core::…`), as `hive_store::ctx`, `hive_cli::procs` and
`hive_runner::hands` do now; never copy it.

**How the brain reaches it.** In this first step the brain drives a
`hive-hand` on its own machine over a pipe: `executor::local_hand_runner`
spawns the binary once per verb with the hand's environment (its workdir,
its home) and `SSH_ORIGINAL_COMMAND` set, in place of `ssh`.
`jobs::tests::remote_run_through_a_local_hive_hand` leases one job that
way and checks it finishes as a remote run through `hive hands` does:
the recipe's env reaches the worker, the run is `done` on executor `hands`,
the job dir is cleaned and every verb went to the binary.

**Routing this machine's work through it.** `hive settings set
executor.transport local-hand` sends this machine's own jobs to the
`hive-hand` installed beside `hive` (or the first on PATH) instead of a
clone under the hive home. The run keeps the `local` executor's name and
slots; the hand makes its clone in `HIVE_HOME/hands`, runs the worker
there, and the brain fetches the commits from that path. Jobs that work
the store (lead, review, reflect, suggestion review) stay local, since a hand has
a clone and no store. `executor.transport local` (the default) puts things
back.

**The push, with a leased credential.** Once the brain's identity check
accepts the commits (ADR-022), it resolves `push.credential` itself, as
for its own pushes (ADR-041 D4: never inside a run), and hands the key to
the hand on the stdin of one `push JOB URL SHA BRANCH` verb. The hand
writes it beside the clone with mode 0600, pushes the job's `hive/`
branch to the upstream with `ssh -i` that key (`--force-with-lease` on an
absent branch: it never moves a branch the remote already holds), and
removes the key whatever the push did. Git offers a key over ssh only, so
on a GitHub https url the hand pushes once more over the url's ssh form
when GitHub asking for a credential is all git says, the daemon's own rule
([An upstream with an https url](landing.md#an-upstream-with-an-https-url)).
An unset `push.credential` pushes with what git on the hand already uses. The run log says `pushed … with
the leased push credential`, a `hand.push` event records it, and a push
that fails is left to the delivery pass, which pushes the same branch
from the checkout. Hands that answer `push=1` in their probe take the
verb. `jobs::tests::a_job_leased_to_a_local_hive_hand_pushes_with_the_leased_credential`
proves the path end to end.

With a GitHub App configured ([Credentials](landing.md#one-git-login-for-every-machine-a-github-app-adr-070-step-3))
and an upstream on its host, the lease is an installation token for the
job's one repository instead of a key: the brain mints it, the hand
installs a credential helper beside the clone that answers only for that
host, pushes over https, and removes the helper whatever the push did. The
run log says `pushed … with the leased GitHub App token` and the
`hand.push` event's `via` names it; a token GitHub will not mint falls
back to the key path above. `jobs::tests::a_job_leased_to_a_local_hive_hand_pushes_with_the_app_token`
proves it against a git server that accepts nothing but that token.

A remote machine can now enrol itself and dial in (next section); `hive
pair` and `hive hands` behind sshd stay as the by-hand path for a machine
you would rather prepare yourself.

## Enrolling a hand

A machine with `hive-hand` installed joins the brain once and then calls
out to it for work: no open port, no sshd drop-in, no `authorized_keys`,
and enrolling puts no key of the brain's on it (ADR-070 step 2). On the
Machines page, **Add a machine** -> **Run one command on it** -> **Get a
join code** shows a six-word code, good for ten minutes and one join, and
the line to run on the machine. Here it is with an address the machine can
reach in place of the one the console shows:

```
hive-hand join --brain https://hive.example.com amber-river-cobalt-maple-orbit-lantern   # the first time: enrols, then stays connected
hive-hand dial          # every later start: keep it running (a launchd or systemd service)
```

What passes between the brain and the machine, from the join code to the
first verb of a job; the machine makes every call and the brain only answers:

```text
┌───────────┐                                           ┌──────────────┐
│ the brain │ ── a join code, carried by you ─────────→ │ the machine, │
│           │ ←── join: the code, a public key ──────── │ hive-hand    │
│           │ ── a device id, its dial executor ──────→ │              │
│           │ ←── nonce: asks for one ───────────────── │              │
│           │ ── a nonce, good for 2 minutes ─────────→ │              │
│           │ ←── connect: the nonce, signed ────────── │              │
│           │ ── a session token, 12 hours ───────────→ │              │
│           │ ←── poll: waits for a verb ────────────── │              │
│           │ ── one hands verb ──────────────────────→ │ runs it      │
│           │ ←── result: what the verb answered ────── │              │
└───────────┘                                           └──────────────┘
```

**The first line does not return by itself.** `join` enrols the machine and
then dials in. It prints
`joined URL as NAME (device dev-ID); jobs run under DIR`, then
`hive-hand: connected to URL as NAME (dev-ID)`, and stays in the foreground
holding the channel to the brain. It returns after enrolling only when
`HIVE_HAND_JOIN_ONLY=1` is set. `hive-hand dial` is for every later start:
it is what a launchd or systemd service keeps running.

**The address in that line.** `--brain` is the address the machine reaches
the brain on. The line the console shows carries the address your browser
used, so it usually reads `http://127.0.0.1:8765`: on another machine that
is the machine itself and not the brain, so change it to an address the
machine can reach. The console listens on 127.0.0.1 only, so another
machine reaches it only through something you set up on the brain's
machine: an SSH tunnel, or a proxy with TLS there. A private network alone
does not reach a port bound to 127.0.0.1. A tunnel reaches the whole
console, not only the hand's routes; a proxy under a name of your own
reaches the hand's routes only, until you list that name
([security.md](security.md#the-machines-you-pair)).

The console answers only to the host names `localhost`, an IP address or a
name listed in the setting `hub.hosts` (empty by default), and any other
`Host` header gets 403: a guard against DNS rebinding, not a login. The
routes a hand calls are exempt, so
`hive-hand join --brain https://hive.example.com` through a proxy works as
it is, while opening the console in a browser under that name needs the
name in `hub.hosts`.

An SSH tunnel is the short way. On the machine that joins, forward a port
of its own to the console and name that port. Keep the tunnel up for as
long as the machine should take work: while the console cannot be reached,
`dial` tries again and waits longer each time.

```
ssh -N -L 8765:127.0.0.1:8765 YOU@HOST &                # HOST is the brain's machine
hive-hand join --brain http://127.0.0.1:8765 CODE       # this machine's port 8765 now reaches the console
```

**What the machine needs to take work.** `hive-hand` enrols the machine and
carries the jobs. An agent itself runs on the harness, and the harness is
in `hive`: the only worker a hand starts is `hive harness run`. So the
machine also needs `hive` on its PATH and a model login of its own
(`hive harness login` there). The brain sends a hand the model's name and no
provider, so the worker runs on the provider that machine is set to, GitHub
Copilot unless a store of its own says otherwise. The installer puts both
programs in place: once as it is for `hive`, once with `--hand`
([release.md](release.md)).

`join [--brain URL] [--name NAME] [--labels a,b] [--workers N] [--workdir
DIR] CODE` makes an ed25519 device key in the hand's own directory
(`HIVE_HAND_HOME`, default `~/.hive-hand`, mode 0700; the private key
0600) and sends only its public half, with the code and the machine's facts
(os, arch, labels, workers, workdir), to `POST /api/v1/hands/join`. The
brain URL comes from `--brain`, else `HIVE_BRAIN`. The brain answers with
a device id and the executor the machine works as - a row of kind `dial`,
named after the machine unless `--name` says otherwise - and the hand keeps
both in `device.json` beside the key. After a join the hand's directory
holds `device.json`, `device_key`, `device_key.pub` and `work`, where jobs
run unless `--workdir` names another directory.

**Join codes.** `POST /api/v1/machines/join-codes` (a console write, held to
the same origin like every console write) issues one: six words, good for 10
minutes and one join. The brain keeps only its SHA-256 (`hand_join_codes`);
a lapsed, spent or wrong code answers 403 and enrols nothing
(`That join code has expired`, `That join code was already used`,
`That join code is not one this brain issued`). The answer
carries `code`, `expires_at`, `minutes`, `brain` (the URL the browser
reached the hub on) and `command`, the line above. The wizard watches
`GET /api/v1/compute`'s `devices` and says when the machine has joined.

**The dial channel.** `hive-hand dial` asks for a nonce
(`POST /api/v1/hands/nonce`, good for 2 minutes and one use), signs it with
the device key (`ssh-keygen -Y sign`, namespace `hive-hand`, so no other
signature of that key can connect) and trades the signature for a session
token (`/hands/connect`, 12 hours). A tampered signature, a used or lapsed
nonce and a revoked device are refused. The first connect puts the `dial`
executor into rotation (it is `drained` until then; a drain by the owner
stays). Since `join` dials in itself, a machine that has joined shows as
`active` as soon as it has connected, without a probe (`hive executor list`
says `probe never`), unlike a paired machine, which stays `drained` until
its probe passes. From then on it long-polls `/hands/poll` (25 seconds at
most per call) and posts each outcome to `/hands/result`. Every `hive-hands` verb
the brain would send over ssh is queued in that device's mailbox
(`hand_calls`) and run by the hand's own `hive-hand` exactly as sshd's
forced command would. The lease and report semantics, and the executor code,
are the ssh path's. The branch comes back as a git bundle (the `bundle JOB
BASE` verb) instead of `git-upload-pack`. A hand that has not polled for
90 seconds is not connected, so a verb for it is refused at once as
unreachable instead of waiting. HTTP goes through the system `curl`: the
hand carries no HTTP stack, and TLS is whatever fronts the hub.

**Revoke.** A dial machine's card on the Machines page shows its last call
and a **Revoke** button (`POST /api/v1/machines/devices/<device or
executor>/revoke`): the brain drops the key and its sessions and removes
the executor; the hand's next call is refused, and coming back takes a new
code. Events: `hand.join_code`, `hand.join`, `hand.connect` (`hand.connect_refused`
with the reason), `hand.revoke`.

`dial::end_to_end::a_hive_hand_joins_a_brain_and_completes_a_job_over_the_dial_channel`
(hive-runner) proves the path: a real `hive-hand` joins a test brain with
a code, dials, and one job runs on it and lands as a remote run. The join-code
and handshake tests sit beside it, and `hands::tests` in hive-hub cover the
routes.

## A brain without code: `brain.code none`

A brain that should never hold the projects' code - a brain you run on a
server of your own from the image in `deploy/` (`deploy/Dockerfile`), or
any machine whose review needs that -
sets `hive settings set brain.code none`. This machine's own slot then
leases only the `lead` and `reflect` jobs. Those run in an empty
workspace and plan from the board, the task text and what the hands
report. Every job that works in the code - tasks, improvements,
experiments and the rest - stays queued for a hand, which keeps its own
clone. A `review` job needs the code and the store both: this brain has no
code and a hand has no store, so no machine takes it yet. Check and watch
scripts are held. A run asked for by hand (`hive work --job ID`) does not
start a job that needs the code here either, `executor.transport
local-hand` included, since a local hand clones onto this disk too. The
source self-update stands down; `daemon.self_update release` installs
releases instead ([release.md](release.md)). What this promises, and the
test that holds it, is in [security.md](security.md).

A code-less brain is not finished. Landing merges in a worktree of the
registered checkout, so with no checkout the lander holds a finished branch
(`land.held`, reason `brain-code-none`: "brain.code is none") without
making a worktree, and the branch waits for a pull request merged through
the forge. A machine reaches such a brain as it reaches any other: paired
over SSH, or enrolled with a join code ([Enrolling a hand](#enrolling-a-hand)).
[security.md](security.md#what-is-not-built) lists what else is not built.

## Heartbeats and presence

Every agent has a heartbeat, and an agent that stops beating is dead. A beat
is any sign of life recorded under the agent's name: a `hive` command run as
the agent (`HIVE_AGENT`; read-only commands beat too), the harness post-tool
hook, taking or renewing a job lease, the runner's supervision of its run, or
`hive agent beat` by hand. Successfully started local workers and the SSH
hands supervisor record an execution heartbeat and refresh it every 60 seconds,
including quiet work and lead jobs; exit, cancellation and timeout stop that
observation. A stop is the execution's last word, not death: the agent stays
`online` while the runner verifies its run and writes its branch, and is `dead` once
`heartbeat.dead_after` has passed since the stop without a fresh beat. While
the runner runs the project's check on the finished worker's clone it beats for
the worker itself (source `verify`, every 60 seconds), so a check longer than
`heartbeat.dead_after` never shows a working run as dead.
Presence is `dead` once the latest beat is older than
`heartbeat.dead_after` seconds (300 by default, never under 30; `hive settings
set heartbeat.dead_after 600` for slow harnesses) and returns to `online` on
the next beat from any source. A worker or lead with no lease and no running
run is `idle`, not dead; an agent with no sign of life on record is `unknown`;
the owner is not monitored. `hive agent list`, `hive agents`, the roster API
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

A run this daemon supervises on a hand is watched another way, since a hand
does not report the run's progress back: while the run goes, the runner probes
the machine every five minutes over the connection the run already shares
(`probe`, five minutes to answer; any answer counts, a failing one too). A
busy machine is slow, not silent, so the bar is high: three probes in a row
without an answer - a quarter to half an hour of silence - end the run:
it is closed as `lost` with the summary *the machine NAME stopped answering:
3 probes in a row over N min went unanswered; the job returns to the queue*,
its job is queued again as above, and the machine turns `unreachable` with
the reason in its probe note (`executor.silent` on the bus), so nothing new
goes there until the runner's probe passes again
([`hand_watch.rs`](../crates/hive-runner/src/hand_watch.rs)).

## Moving a project

`hive move <slug> <path> [--force]` points a project at the checkout it now
lives in - the folder was moved, the disk it was on is gone, or the
repository was cloned again elsewhere - and keeps its slug, tasks, posts,
runs and records; the `project.moved` event remembers the old path. The
guards are registration's: the path must exist and be a git checkout (its
top level is taken, as `hive init` does), may not be a checkout the runner
made itself, and may not already be another project's. A checkout whose
`origin` is a different repository than the project's remote is refused -
a different remote usually means the wrong folder - unless `--force`, which
adopts the new remote as the project's. The console's **Change folder…**
(project page menu) and the **Point to its folder** button on a project
marked *Missing on disk* run the same function. Nothing on disk changes,
and jobs that failed with `cannot change to <old path>` run again as the
watchers queue them.

## Removing projects and executors

`hive remove project <slug>` and `hive remove executor <name>` show a preview
of what deactivates (nothing is deleted) and ask before applying; `--dry-run`
only previews, `--yes` skips the prompt. `hive remove project <slug> --forget`
is the hard delete for a registration that is already inactive and whose
checkout no longer exists: it previews the per-table row counts (posts, jobs,
runs, briefs, records, events, harness rows, ...), asks the same way, deletes
them all in one transaction with the project row, and writes a
`project.forgotten` event; it refuses while the project is active, while the
checkout is still on disk or lives on a volume that is not mounted, and while
a run, harness session or leased job of it is in flight. `hive remove
duplicate-findings [--project SLUG] [--apply] [--yes] [--json]` is dry-run by
default and reports
per-project duplicate groups and rows; with `--apply` it deletes only exact
duplicate system findings (`daemon` or project lead, same
project/area/thread/summary/agent, earliest kept), drops and recreates the post
immutability triggers from their stored definitions, verifies they are present,
and writes a `post.remove.duplicates` event. Human and worker findings are left
alone.

A removed executor keeps its row in the store, with the status `removed`.
`hive executor list` leaves such rows out. The `executors` line of
`hive status` counts them at its end instead of listing them
(`local active (0/5 busy), 2 removed`), and works out no note for them.
