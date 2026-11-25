# Troubleshooting

Find what you see, read why it happens, and fix it. Four places answer most
questions, so look there first.

## Look here first

`hive status` is one screen about the whole machine:

```text
store      ~/.hive/hive.sqlite (schema 3)
projects   1 active
  my-app             ~/code/my-app
jobs       1 failed, 1 queued
runs today 0 of 2 (jobs_per_day)
runner     running allowed
harness    native · GitHub Copilot · not signed in - Sign in with GitHub (Settings -> Harness) ...
daemon     never started
executors  local active (0/5 busy)
```

| Line | What it tells you |
| --- | --- |
| `daemon` | the heartbeat of the runner (`hive daemon`, the process that plans jobs and starts agents): `alive`, `silent` (no beat for a while), `exited` or `never started` |
| `runner` | `running allowed`, or that the kill switch (`hive stop`) is on |
| `harness` | the model provider, and whether it is signed in |
| `runs today` | the jobs started today, against the cap `jobs_per_day` |
| `executors` | this machine (`local`) and every other machine that runs jobs, each with its state (`active`, `drained`, `unreachable`) and how many of its slots are busy. After a `;` come the projects a machine cannot run now, each with the reason (`lacks node@22 for my-app`). Removed machines are counted at the end, not listed: `2 removed` |
| `capacity`, `waiting` | there only while jobs wait: how many are queued, and what holds them back |
| `planner`, `recovery` | with a project's name: why nothing is planned for a project that has nothing queued, and what became of its failed jobs |

- **The console.** **Home** shows a notice with its button when the runner is
  off, paused or not responding, and the card **Set up Hive** until a provider
  is connected and a project added. The foot of the sidebar shows the runner's
  state on every page. **Runs** lists every run: open one for its transcript.
- **`hive jobs --all` and `hive events --limit 20`.** The first lists the
  queue and the finished jobs, a failed one with its result. The second prints
  the event log, newest first; `--project my-app` narrows both.
- **The logs.** As services, the runner writes `~/.hive/logs/daemon.log` and
  the console `~/.hive/logs/hub.log`. When the runner has nothing to start, it
  says why in a line that begins `idle:`. A `hive daemon` or `hive hub` that
  you started in a terminal prints there.

## Starting

- **The shell cannot find `hive`.** The installer puts it in `~/.hive/bin` and
  does not edit your shell profile. Add `export PATH="$HOME/.hive/bin:$PATH"`
  to the profile and open a new terminal.
- **`a hive daemon is already running on this store`.** A runner already
  works on this store, and one is enough. `hive status` shows its pid.

### `The hub could not listen`

```text
The hub could not listen
  why: listen tcp4 127.0.0.1:8765: bind: address already in use
  fix: hive hub --port 0
```

Another program holds port 8765. Often it is Hive itself: if
`http://127.0.0.1:8765` answers, a console already runs (the service's, or one
in another terminal). Otherwise start it with `hive hub --port 8766`, or with
`--port 0` for a free port, which the line `Serving on` names. The service's
console moves with `hive service install --hub-port 8766`. No setting holds
the port.

### `No store at ...`

```text
No store at ~/.hive/hive.sqlite
  why: this machine has no Hive store yet
  fix: hive init --store-only (or hive init <project-path> to register a project as well)
```

The store is `hive.sqlite`, the one file that holds everything Hive knows,
and none exists in the hive home the command looked in. Run the fix. A
`hive hub` started in a terminal makes the store itself (it says
`No store yet: made one at` with the path); one started as a service does not.
If you do have a store, the path in the message is not the one you expect:
`HIVE_HOME` or `--home` points the command at another folder.

### `This console does not answer to the name ...`

You opened the console under a host name. It answers to `localhost`, to IP
addresses and to the names in the setting `hub.hosts`, and refuses every other
name. Open `http://127.0.0.1:8765`, or allow the name with
`hive settings set hub.hosts my-mac.local`.

### The console opens, but nothing runs

`hive hub` serves the console only. The runner is a second process, and until
it runs, every task and every onboarding waits in the queue. Home says
**Runner is off** with the button **Start runner**, or shows the step
**Keep Hive running** on its card **Set up Hive**, and the foot of the sidebar
says **Runner off**. Press the button and confirm in its dialog, or run
`hive service install`, or run `hive daemon` in a second terminal
([Keep Hive running, updated and backed up](operations.md)).

## Nothing happens to my task

A task stays **Open** until the project's lead (`my-app-lead`, the agent that
owns the project) plans it into jobs. When it stays open, or its jobs stay
queued, read `hive status` from the top:

- **`daemon     never started`, or `exited`.** No runner runs. Start it
  ([above](#the-console-opens-but-nothing-runs)).
- **`runner     the runner is stopped in the store (hive start to resume)`.**
  The kill switch is on: somebody ran `hive stop` or pressed
  **Pause all work**. Home says **Runner is paused: nothing new starts**.
  Press **Resume** there, or run `hive start`.
- **`harness    ... not signed in`.** No model provider is connected, and the
  runner starts no agent on this machine without one. Home shows the card
  **Set up Hive**: press **Connect** on its first step
  ([Choose models and set budgets](models.md)).
- **`daemon     silent`.** The runner has not beaten for a while, and Home
  says **Runner is not responding**. On macOS, look for a dialog about a
  folder ([below](#macos-asks-about-folder-access)). A runner that stays
  silent for half an hour writes `~/.hive/logs/daemon-stall-<time>.log` and
  exits, and its service starts it again.
- **The machine is busy.** The runner starts none of the agents' own work
  while the load of the last minute is over twice the number of CPUs, or
  while less than 1024 MB of memory is free. The log says, for example,
  `machine saturated: load 40.1 on 12 CPUs is over the line 24`. One run of
  your own work still starts through such a hold. Neither line is a setting:
  wait, or close what loads the machine.
- **The disk is nearly full.** With less than `daemon.min_free_mb` (10240 MB)
  free where `~/.hive/work` lives, the runner starts nothing, and the log
  says `disk under floor:` with the free space and the path. The runner
  clears what it can by itself, and `hive sweep --reclaim` does the same now.
- **`runs today 2 of 2 (jobs_per_day)`.** The day's cap is reached. Until
  midnight UTC the runner starts no more work of its own: onboarding,
  suggestions and research wait. Your tasks, the lead's replies, reviews and
  security updates still start. Raise the cap under **Settings**, **Limits**,
  **How much work runs**, or with `hive settings set jobs_per_day 10`.
- **No machine offers what the project needs.** On the project's page, under
  **Team & tools**, the panel **Runs on** says **Nothing can take this
  project's jobs right now.** Press **Change** to edit what the project needs,
  or add a machine that offers it ([Add a second machine](add-a-machine.md)).
- **Every slot is taken.** Once the next step of a task has waited ten
  minutes, the task's page says why, for example that all agent slots were
  taken, and `hive status` adds a `waiting` line.

## A job failed

In the console, open **Runs**. The chips **Failed** and **Stopped early**
narrow the list, and a run opens with its transcript and the reason it ended.
Under **Failed**, **Jobs that failed** lists the jobs behind such runs. In a
terminal, `hive jobs --all` shows each failed job with its result:

```text
failed    my-app         task     tier 1  job-20261123T041810Z-7cdfa8        Add a rate limit
          npm test failed: 2 tests red in api/limits.test.js
```

The runner sorts every failure and acts on it by itself:

| What failed | What the runner does |
| --- | --- |
| The moment: a lost run, a runner error, a machine out of reach, a timeout | runs the job again after a pause that grows: 5 minutes, 15, 45, up to 6 hours |
| The work: a non-zero exit, a reported failure, commits that fail the check | queues one corrected job whose brief carries the failure; a task's job goes back to the lead |
| What only another path can fix: a missing login or tool, a hard stop, a question for you | holds the job with its reason |

It recovers a chain at most `daemon.recover_limit` times (3) within
`daemon.recover_days` (7), then holds it. In `hive jobs --all` the state
follows the word `recovery:`: `ran again`, `re-planned as` with the new job,
`held:` with the reason, or `retried by hand`.

To retry a failed or cancelled job by hand:

```bash
hive job retry job-20261123T041810Z-7cdfa8      # queue the same job again
hive job retry job-20261123T041810Z-7cdfa8 --as-new \
  --brief 'Count per key, not per address.'     # one corrected copy, with your note
```

In the console, open **Jobs that failed**, tick **Retry this job** on up to
five jobs, press the button, which with two ticked reads **Retry 2 jobs**,
and confirm with **Retry**. A job that cannot be retried says why, for
example **Needs a fix first** or **Waiting before the next retry**. Three
ways a run ends are worth knowing:

- **Hit the step limit**, **Hit the time limit**. A run may make
  `harness.max_steps` model calls (60) and take `minutes_per_job` minutes
  (10). A run that the step limit stops with work in its clone is carried on
  once with a fresh budget. To give task runs more, set
  **Steps per task run** under **Settings**, **Harness**, **Limits per run**.
- **Model provider error**, **Not signed in**. The provider answered with an
  error, or Hive is not signed in to it. Check the provider under
  **Settings**, **Harness** ([Choose models and set budgets](models.md)).
- **Lost track of it**. The run was cut off before it ended: the runner
  stopped under it, its machine stopped answering, or its agent gave no sign
  of life for `heartbeat.dead_after` seconds (300). The job is queued again.

## No pull request

Finished work becomes a pull request on the project's upstream, opened with
`gh`, the GitHub CLI. Open the task: the line under its title says where the
work stands.

- **The project has no upstream.** Then there is nothing to open a pull
  request on, and finished branches are merged into the local default branch
  instead. `hive land --project my-app --dry-run` says which it is:
  `my-app: no upstream, finished branches land locally on main`. For pull
  requests, give the checkout a remote named `origin` on GitHub.
- **The line reads `Held:`.** Delivery could not push the branch or open the
  pull request, and the reason follows. When it says that `gh` is not
  installed where the daemon can run it, install `gh`; if it is installed,
  the services lack it on their `PATH`, so run `hive service install` again
  from a shell that finds it. When it says that `gh` is not logged in, run
  `gh auth login`. When the push was refused, the note carries git's words: a
  key that asks before each use stops every push, so give the runner a key of
  its own ([Credentials](../landing.md#credentials)).
- **The default branch is red.** When a run's check fails, the runner tries
  the check on the commit the run started from. If that fails too, the job's
  result says the base `was already red before this job`, and the job is run
  again later ([above](#a-job-failed)). Merge a fix yourself, or give the
  project a check it can pass: `hive init ~/code/my-app --verify 'npm test'`.
- **The pull request waits for you.** The task says **Needs your review**,
  with the reviewer's reasons and a **Merge** button
  ([Review and merge the work](pull-requests.md)).
- **The run delivered nothing.** A run whose check fails delivers nothing. Its
  commits are kept on a branch `hive/<agent>/<job>-failed` in your checkout,
  and the job is retried ([above](#a-job-failed)).
- **A step of the task still runs.** A task is one pull request, opened once
  none of its jobs is queued, running or blocked. Until then a finished step
  reads **Waiting for its pull request**.

## The loop does nothing

The self-improvement loop builds and measures the changes that agents vote
for. Its experiments are off until you turn them on, and that is not its only
gate. Ask which one is closed:

```bash
hive evolve status --project my-app
```

```text
Loop                            false
Baseline                        none
```

The last line of the output, `Next human step`, names the command that opens
the next gate: here `hive settings set evolve.loop 1`
([Turn on the self-improvement loop](self-improvement-loop.md)). The `planner`
line of `hive status` says what holds the project's suggestions back.

## A second machine

The **Machines** page and `hive executor list` show the state of each machine:

- **Can't reach it** (`unreachable`). The machine stopped answering. The
  runner probes it again after a minute, then less often, up to every thirty
  minutes, and the first probe that passes puts it back.
  `hive executor probe linux-box` probes it now.
- **Not taking work** (`drained`). You drained it, or it has not passed a
  probe yet. A newly paired machine has no model login and stays drained
  until it has one ([Add a second machine](add-a-machine.md)).
- A machine that joined with `hive-hand` takes work only while that program
  is connected. Start it there with `hive-hand dial`.

## macOS asks about folder access

A checkout under Desktop, Documents or Downloads makes macOS ask once whether
`hive` may access that folder, the first time the runner touches it. Answer
Allow. Until you do, the runner waits on the dialog and `hive status` shows it
as `silent`. `hive service install` signs `hive` so that macOS keeps your
answer across updates. A `hive` from Homebrew is not signed again, so macOS
may ask again after `brew upgrade`. Checkouts elsewhere never ask
([macOS folder access](../runner.md#macos-folder-access)).

## After an update

- **Which version runs?** `hive version` prints it, with a second line when a
  newer release is known; the console's banner says the same. **Settings**,
  **This machine** shows the version of **This console** and of the
  **Runner**, and a row **Update waiting** while a new program waits for runs
  to end. You can restart from that row.
- **The console still shows the old version.** A `hive hub` that you started
  by hand keeps the old program until you start it again.
- **The new version misbehaves.** Go back with
  `mv ~/.hive/bin/hive.prev ~/.hive/bin/hive`; the services restart on it. A
  runner that exits three times in a row within a minute of starting is put
  back on `hive.prev` by its launcher.

## Still stuck

Open an issue with the *Bug report* form
([CONTRIBUTING.md](../../CONTRIBUTING.md)). It asks what you did, what
happened and what you expected; what `hive version` prints, your operating
system and how you installed Hive; and what the command printed, what
`hive status` shows, or the end of the two logs. Take tokens, keys and private
paths out of what you paste. A vulnerability does not go in a public issue:
[SECURITY.md](../../SECURITY.md) says how to report it privately.

## Read more

- [Recovery and hygiene](../recovery.md): failed jobs, retries and clean-up.
- [When delivery fails](../landing.md#when-delivery-fails): why a branch is held.
- [The load governor](../landing.md#the-load-governor): when new work is held.
- [Questions people ask](../faq.md): short answers to the first questions.
