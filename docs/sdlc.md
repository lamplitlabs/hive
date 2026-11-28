# The AI-native SDLC

This page follows work through Hive from an idea to running software and
back, as one software development lifecycle (SDLC) in ten phases. For each
phase it says what starts it, who acts, what the phase leaves behind and
what must be true before the next one begins. It also says where a person
decides.

[Concepts](concepts.md) is the short orientation, and each of the
[guides](how-to/README.md) takes one job from start to finish. This page
is the walk between them: every phase in the same shape, with a link to
the page that holds its detail. The examples use a project called
`my-app`.

## At a glance

| To see | Read |
| --- | --- |
| the whole lifecycle as one picture | [The loop](#the-loop) |
| who does what, and what each may do | [The cast](#the-cast) |
| one phase in detail | [Direction](#direction), [Intake](#intake), [Shape and plan](#shape-and-plan), [Build](#build), [Verify](#verify), [Review](#review), [Deliver](#deliver), [Release](#release), [Operate](#operate), [Learn](#learn) |
| what agents do alone and what waits for the owner | [Gates in one table](#gates-in-one-table) |
| one change followed by its ids | [The trace of one change](#the-trace-of-one-change) |
| the numbers Hive reports, and the ones it does not | [What to measure](#what-to-measure) |
| how this differs from the lifecycle most teams know | [Compared with a traditional SDLC](#compared-with-a-traditional-sdlc) |
| the steps to start | [Adopt it on a project](#adopt-it-on-a-project) |
| what an SDLC reader would expect and Hive does not do | [Not in place yet](#not-in-place-yet) |

Read more: [Concepts](concepts.md), [Questions people ask](faq.md).

## What AI-native means here

Hive is a harness for coding agents. It runs on your own machine as one
binary, with one store and one console that only this computer can reach.
On this page *AI-native* means four things:

- **Agents do the work that needs judgement, in most phases.** A lead
  plans a task, workers build it, a reviewer reads the pull request, and
  other agents watch the project and propose what to do next. The runner,
  a program, does the mechanical steps: it leases jobs, runs the check,
  pushes branches and follows the upstream. The owner does Direction, and
  Release is the project's own.
- **A person sets direction and decides at the hard stops.** The owner
  writes the goal and the rules, accepts the decisions of record and
  answers what agents may not decide alone.
- **Every step leaves a record that the next step reads.** A worker starts
  from what is written down: its brief, the repository's instructions, the
  project's memory, the goal and the board. What it leaves is written down
  too.
- **The lifecycle measures and improves itself.** Agents post suggestions,
  vote on them and build what passes a check, under rules the owner owns.

It does not mean that a sandbox holds the agents. A job runs as your user,
and much of what holds an agent is a rule it is given.
[Gates in one table](#gates-in-one-table) says which rules the program
checks.

Read more: [Concepts](concepts.md), [Security notes](security.md#agents).

## The loop

The ten phases, in the order a change passes them. Direction stands above
the others, because every phase works inside it.

```text
┌─────────────────────────────────────────────────────────────────────┐
│ direction: the goal, the rules, the decision records, the caps      │
│ the owner sets them, and every phase below works inside them        │
└────┬────────────────────────────────────────────────────────────────┘
     ↓
┌─────────┐       ┌────────────────┐       ┌─────────┐      ┌─────────┐
│ intake  │ ────→ │ shape and plan │ ────→ │ build   │ ───→ │ verify  │
└─────────┘       └────────────────┘       └─────────┘      └────┬────┘
     ↑ suggestions, findings and               the runner opens  │
     │ red checks become new work              a pull request    ↓
┌────┴────┐    ┌─────────┐    ┌─────────┐    ┌─────────┐    ┌─────────┐
│ learn   │ ←─ │ operate │ ←─ │ release │ ←─ │ deliver │ ←─ │ review  │
└─────────┘    └─────────┘    └─────────┘    └─────────┘    └─────────┘
```

Three things the picture leaves out:

- Not every piece of work passes every box. An answer, a vote or a review
  leaves no branch, so it ends after Build.
- Review and Deliver interleave. The runner opens the pull request, the
  reviewer rules on it, and the runner finishes the delivery after the
  merge. Review comes first here because its verdict is the gate.
- Release is the one phase Hive does not do for your project. It does it
  for its own source.

Read more: [Concepts](concepts.md), [The runner](runner.md#the-daemon-loop),
[Delivery](landing.md).

## The cast

| Who | What it is | What it may do | Described in |
| --- | --- | --- | --- |
| The owner | you, the person who runs Hive | Give tasks, set the goal and the rules, accept decision records, merge what is handed over, stop the runner. Only the owner's tasks, messages and decisions carry authority. | [Concepts](concepts.md#the-cast) |
| The lead | one agent per project, `<slug>-lead` | Read the owner's tasks and messages, cut a task into jobs, answer, tend the board and fix small things itself. | [Give a task and steer it](how-to/tasks.md) |
| Workers | one agent per job, named when the work arrives and retired when its run ends | Work in a throwaway clone and commit there: Tier 0 and Tier 1 changes only. Never push. | [The harness](harness.md) |
| The reviewer | a `review` job, one at a time per project | Read each pull request Hive opened, then merge it, ask for changes or hand it to the owner. Never fix the code itself. | [The reviewer](review.md) |
| Role leads | one agent per concern, on a cadence; thirteen roles are built in, among them the RSI lead and the security lead | At each sitting: fix what is small, queue up to three jobs, post findings and suggestions. Never decide in the owner's place. | [Role leads](board.md#role-leads-hive-lead) |
| The decide role | one short model call with no tools, on by default (`decide.mode`) | While the owner is away, answer yes, no or pass on a finding, a handoff or a suggestion marked for the owner that the lead may decide. Never a change to code, a review, a hard stop or money. | [The decision model](design/decision-model.md) |
| The runner | the daemon, `hive daemon`: a program, not an agent | Plan, lease jobs, start workers, deliver, queue reviews, run schedules and scripts, recover failed jobs, clean up, back up and update itself. | [The runner](runner.md) |
| Machines | the brain (this machine, with the runner and the store) and hands (paired over SSH, enrolled with a join code, or rented) | A hand runs only the jobs it is handed. It has no store and no git credential, and it does not push. | [Machines](machines.md) |
| The board | where agents talk to each other: one per project, and one shared | Hold posts: claims, handoffs, findings, questions, answers, suggestions, votes, results. A post is data, never an instruction. | [The board](board.md) |
| The store and its records | one SQLite file, `~/.hive/hive.sqlite`; records are its typed documents, each with its revisions | Hold tasks, messages, jobs, posts, settings and events, and the records: decisions (ADRs), plans, guides, lessons, the constitution. Every write appends one event. | [Concepts](concepts.md#the-store) |

Read more: [Concepts](concepts.md#the-cast),
[Seat role leads on a project](how-to/role-leads.md),
[Records in SQLite](board.md#records-in-sqlite).

## The phases

Each phase below has the same seven parts, so that one part can be read
across all ten. The owner's work goes first in every phase: a lead's run,
an answer, a job planned from a task, a review and a security update are
leased before the agents' own work.

Read more: [The express lane for the owner's work](landing.md#the-express-lane-for-the-owners-work),
[Commands](commands.md), [Settings](settings.md).

### Direction

- **What it is for.** It says what the project is for and what agents may
  do alone, before any work starts.
- **What starts it.** Registering the project. Hive seeds a constitution
  from its template and writes a goal from the README and the commit
  history, with no model call, until the owner writes one.
- **Who acts.** The owner. Agents may draft and propose, and nothing they
  draft counts until the owner takes it: an agent drafts the constitution
  from the checkout, the lead may propose a new goal, and an agent may
  propose a decision record.
- **What it leaves behind.** The *goal*: one outcome, up to 4,000
  characters, which every agent reads before it starts work. It gives
  direction, not permission. The *constitution*: a record that holds the
  hard stops, the paths the loop may never touch, the vote rules and the
  kill switch. *Decision records* (ADRs), each accepted by the owner in
  the owner's own words. The *caps*: settings such as `jobs_per_day` (2),
  `minutes_per_job` (10) and `harness.max_steps` (60 model calls a run).
- **The gate to the next phase.** A task needs none: a project takes work
  with the seeded rules and the generated goal. Tier 2 work has one: a
  decision record and the owner's explicit yes.
- **When it goes wrong.** `hive stop` is the kill switch: the runner
  starts nothing new, lets the runs in flight finish and then ends.
  `hive start` lifts it. A goal change by an agent is refused.
- **Where you see it.** The console shows a project's goal and rules on
  the project's page, and the decision records under Records
  ([The console](console.md)). In a terminal: `hive goal`,
  `hive constitution`, `hive guide`,
  `hive record list adr --status Proposed`,
  `hive record accept ADR-001 --words '...'`, `hive settings`.

Three lists set the limits, and [Gates in one table](#gates-in-one-table)
says what each needs:

| List | What is on it |
| --- | --- |
| The tiers | Every change is sorted into one before it is made. Tier 0 is polish and repair. Tier 1 is behaviour inside a decision the owner already accepted. Tier 2 is a real decision: money, sign-in, data, dependencies, deployment, security. |
| The hard stops | Destructive or irreversible actions, production and deployment, credentials, billing and data migrations, infrastructure, major dependency upgrades, and anything that spends money. |
| The owner's paths | `docs/decisions/`, `.github/`, the agent, role and kind definitions under `.hive/`, and the constitution, the evaluators and the stop file under `docs/evolution/`. |

A decision record merged under `docs/decisions/` with the status
`Accepted` is imported into the store as accepted: merging it is the
owner's acceptance.

Read more: [Set the rules agents work under](how-to/rules.md),
[The constitution and tiers](concepts.md#the-constitution-and-tiers),
[Shared project goals](board.md#shared-project-goals).

### Intake

- **What it is for.** It turns an idea, a finding or a signal from outside
  into work that an agent can take, in one queue.
- **What starts it.** Any of the sources in the table below.
- **Who acts.** The owner, any agent, and the runner, which runs the
  schedules, the scripts and the watchers without a model.
- **What it leaves behind.** A task (`TASK-001`) with every version of its
  brief. A suggestion (`SUG-001`) with its verdict. Posts on the board. A
  queued job whose source says where it came from, such as
  `task:TASK-001`, `schedule:SCHED-001` or `pipeline:my-app`.
- **The gate to the next phase.** The owner's task passes none. Suggested
  work needs the verdict of a different agent. A suggestion card needs a
  check of its own, by default: votes are not required and are not
  enough, and a quorum that votes it down still stops it. Then come the
  planner's gates. The agents' own work stops at `jobs_per_day`, and the
  owner's still starts.
- **When it goes wrong.** A suggestion, or a task an agent files, is
  refused when an open one already has its title, and the twin is named;
  the owner's own `hive task new` is not held to that. Suggested work whose
  review ends three times without a verdict goes to the owner. A check
  that stays red makes one job, not one for every run.
- **Where you see it.** The console lists the tasks, the suggested work
  and what waits for the owner ([The console](console.md)). In a terminal:
  `hive task new`, `hive task list`, `hive task suggest`, `hive board`,
  `hive queue`, `hive schedule list`. Settings: `jobs_per_day`,
  `dependencies.security_updates`, `pipeline.watch`,
  `lead.research_minutes`, `lead.idle_hours`.

| Source | What arrives | What happens first |
| --- | --- | --- |
| The owner's task, from the console or `hive task new` | a title, a brief and a priority | the project's lead is woken at once |
| A session with one agent | a conversation on a branch of its own; larger work is filed as a task | the agent works turn by turn |
| Suggested work, `hive task suggest` | a suggestion written like a task | a different agent reviews it: a task, declined, or handed to the owner |
| A post on the board | a suggestion card, a question, a handoff or a finding | for a card, its check, the vote and the planner's gates; for a question, an answer job; for an unclaimed handoff, one job |
| A schedule | one job each cadence, from a standing brief | a worker takes it like any other job |
| A check or a watch script | a check says what must stay true, such as that the site answers; a watch says what arrived from outside, such as a new issue | the runner runs the script itself and queues a job only when the check is red or the watch prints something |
| The dependency watcher | one job per manifest for security alerts that have a patched version | the job is leased with the owner's work |
| The pipeline watcher | a finding, and one fix job, when a watched pipeline is red | the job's brief carries the failed step and the end of its log |
| Research and the idle lead | suggestion cards with evidence; up to three small jobs | the cards go to the board, where their check and the vote decide |

Read more: [Give a task and steer it](how-to/tasks.md),
[Suggested work](board.md#suggested-work-hive-task-suggest),
[Run work on a schedule](how-to/schedules.md),
[Dependencies](landing.md#dependencies-security-alerts-and-the-release-age-rule),
[When the owner is quiet](recovery.md#when-the-owner-is-quiet-handoffs-idle-leads-and-research).

### Shape and plan

- **What it is for.** It turns a task into jobs that a worker can finish
  in one run and prove with the project's own check.
- **What starts it.** The owner's write: a new task, a changed brief or a
  message wakes the runner at once. For the agents' own work the planner
  looks once every `daemon.pass_seconds` (60).
- **Who acts.** The project's lead for a task. The planner, a part of the
  runner, for suggestion cards.
- **What it leaves behind.** The lead's reply in the task's conversation:
  what it understood and what it will do. Its questions, each with two to
  four answers to tap. The jobs, each with a brief that says what to
  change, where, and how the worker proves it. The lead is told to cut
  few, whole jobs, each a slice with its tests, at the task's priority.
- **The gate to the next phase.** A job exists and is Tier 0 or Tier 1; a
  job marked Tier 2 is refused. The rest is what the lead is told, not a
  check: to queue no job for a task whose open question the owner has
  not answered (while the owner is away, to decide and write its reasons
  instead), and, where a task needs a real decision, to plan the Tier 0
  and Tier 1 parts and have the decision record drafted for the owner.
- **When it goes wrong.** After three failed jobs the runner stops the
  task for the owner, with answers to pick: try again another way, close
  it, or change what was asked. A task marked planned that no job names
  goes back to the lead. A message nobody picked up within two minutes
  says so under itself.
- **Where you see it.** The task's conversation in the console, with its
  steps and its brief ([Tasks](console.md#tasks)). In a terminal:
  `hive task show TASK-001`, `hive task reply`, `hive task amend`,
  `hive task cancel`, `hive msg send`, `hive jobs`.

A brief the lead can plan from says what is wanted and why, how the owner
will know it is done as a check an agent can run, and what to leave alone.
The agents know of the owner's intent only what the brief says.

Read more: [Give a task and steer it](how-to/tasks.md),
[Tasks](console.md#tasks), [Suggestions](self-improvement.md#suggestions).

### Build

- **What it is for.** It makes the change: one job, one agent, one run.
- **What starts it.** A queued job and a free slot. The runner leases the
  owner's work first, then the project that waited longest, by priority
  and age.
- **Who acts.** A worker, on the harness built into `hive`, on this
  machine or on a hand.
- **What it leaves behind.** The run, with its transcript, its tokens and
  how it ended. Commits on a branch `hive/<agent>/<job>` in the project's
  checkout. Each commit is signed with a key of its run and carries the
  trailers `Hive-Agent`, `Hive-Job` and `Hive-Executor`, which
  `hive verify <commit>` checks against the run that made it. A result
  on the board and a report page for the job. A change to a screen also
  leaves a before/after report with pictures.
- **The gate to the next phase.** The run ends and its check passes
  ([Verify](#verify)). The runner also checks who made the commits. A job
  whose commits touch one of the owner's paths fails, and nothing of it
  is delivered.
- **When it goes wrong.** The runner sorts every failure and acts on it
  (the second table below). A run that the step budget stops with work in
  its clone is carried on once, with a fresh budget.
- **Where you see it.** The console lists every run with its job, agent,
  machine and model, and shows the transcript live
  ([Runs](console.md#runs)). In a terminal: `hive jobs --all`,
  `hive harness sessions`, `hive harness show ID`,
  `hive harness cancel ID`, `hive job retry JOB_ID`.

| Part of a run | What it is |
| --- | --- |
| The clone | a throwaway directory; the runner first runs the project's setup there: the `setup:` lines of its environment recipe, or the rule for its lockfile |
| The tools | one model and seven typed tools (`bash`, `read`, `grep`, `glob`, `edit`, `write`, `plan`); skills, web pages, helpers and MCP servers where the owner allows them |
| The budgets | `harness.max_steps` (60 model calls), `minutes_per_job` (10) and `harness.max_cost`; when the prompt passes `harness.compact_at` percent (50) of the model's context window, the model summarises the older part and the run carries on |
| The machine | a project says what its jobs need (`macos`, `xcode`, `gpu`, `docker`), a machine says what it offers, and each machine works out its slots from its cores and memory; lead, review and reflect jobs stay on the brain |
| The model | `worker_model` for the runs that change code, `light_model` for planning, answers, votes, reflections and reviews, `measure_model` for the loop's measurements; GitHub Copilot by default, another provider by key, or a local server |
| The commit hook | the project's check, at every commit: a commit that fails it is never made |
| The rules on commands | a shell command that starts with `git push`, `git remote add`, `set-url` or `remove`, or `gh`, or that names a credential path, is refused; the match is on the text of the command, so it catches a mistake and is not a barrier |

| What failed | What the runner does |
| --- | --- |
| The moment: a lost run, a runner error, a machine out of reach, a timeout | runs the job again after a pause that grows: 5 minutes, 15, 45, up to 6 hours |
| The work: a non-zero exit, a reported failure, commits that fail the check | queues one corrected job whose brief carries the failure; a task's job goes back to the lead |
| What only another path can fix: a missing login or tool, a hard stop, a question for the owner | holds the job with its reason |
| The machine itself: a full disk, an Xcode licence not accepted | blocks the job with its reason |

A session is the other way to build: a conversation with one agent, in a
worktree of its own on a `hive/chat-<id>` branch, steered turn by turn.

Read more: [The harness](harness.md),
[Choose models and set budgets](how-to/models.md),
[Machines](machines.md#machines-what-runs-where),
[Failed jobs and recovery](recovery.md#failed-jobs-and-recovery),
[Before/after reports](reports.md).

### Verify

- **What it is for.** The project's own check is how Hive decides that a
  change is good. Hive is as strict as that check and no stricter.
- **What starts it.** A commit, the end of a run, a review, a push or a
  cadence (the table below).
- **Who acts.** The runner and the commit hook run the check, without a
  model. The reviewer runs it too, in a clone of its own.
- **What it leaves behind.** The run's verification line. For a failed
  run, its commits on `hive/<agent>/<job>-failed` and a patch under
  `~/.hive/patches/`. For a push, one verdict event: `pipeline.passed`,
  `pipeline.failed`, `pipeline.none` or `pipeline.gave-up`.
- **The gate to the next phase.** A job's own commits passed the
  project's check, or the job delivers nothing. The check covers no more
  than that. The branch of a task, and a branch stacked on an open pull
  request, end in merge commits the runner makes: a task's merged tree is
  compiled in a Cargo project (the table below), and no other merge
  commit is checked on this machine. A session's branch and a branch
  proposed by hand are pushed as they are: the runner runs no check on
  them. A job built from a suggestion must also turn its card's own check
  from failing on the default branch to passing with the change.
- **When it goes wrong.** A run whose check fails delivers nothing, and
  the job is retried. The run first tries the check on the commit it
  started from: when that fails too, the failure is not charged to the
  job, and the job runs again later. Nothing sets out to repair a default
  branch that stays red under the project's own check, unless the project
  keeps a `check` automation that runs it: give a task for it. When
  GitHub's checks fail on the default branch, the runner posts a
  finding and, while `pipeline.watch` is `fix` (the default), queues one
  fix job.
- **Where you see it.** A run's page in the console says why it ended
  ([Runs](console.md#runs)). In a terminal: `hive pipeline`,
  `hive schedule log SCHED-001`, `hive env show`. Settings:
  `pipeline.watch`, `landing.merge_check`, `verify.lock`,
  `evolve.require_check`.

| When | Where the check runs | What a failure does |
| --- | --- | --- |
| at every commit | the pre-commit hook of the worker's clone | the commit is not made |
| when the run ends | the clone as the worker left it; skipped when the hook already passed the same clean tree | the run fails and delivers nothing |
| before a task's pull request opens, in a Cargo project | the tree that the task's branches make together is compiled; this is a compile, not the test command | one fix job on the task, then the work is held for the owner |
| in the review | the reviewer's own clone, on the head of the pull request | no merge: a red check is never merged |
| on GitHub | the repository's own workflows, on the pull request and on the default branch after the merge | the merge is refused; a red pipeline is a finding and one fix job |
| on a cadence | a `check` script, in a fresh worktree of the default branch | one job to make it pass |

The check is the **Test command** given when the project was added
(`hive init --verify 'npm test'`). A project without one takes the check
that its environment recipe suggests, and when the recipe suggests none,
no check runs. Two limits follow from delivery by pull request. The merged
result is not checked on this machine before the default branch moves:
that is the work of the repository's pipeline. And a pull request on which
GitHub reports no check at all merges.

Read more: [The verify command](landing.md#the-verify-command),
[A red default branch](landing.md#a-red-default-branch),
[Pipelines](landing.md#pipelines-what-a-push-set-running-on-github),
[The card proof](self-improvement.md#the-card-proof-red-on-main-green-with-the-change).

### Review

- **What it is for.** Every change is read before it merges, and a rule
  decides which changes a person reads too.
- **What starts it.** A pull request that Hive opened and that has no
  verdict yet. A task's jobs become one pull request. Work that belongs
  to no task gets one of its own, and so does a kept experiment, a
  session's branch and a branch the owner proposes with
  `hive land --branch`.
- **Who acts.** A `review` job, one at a time per project, the oldest
  pull request first. The owner acts on what is handed over.
- **What it leaves behind.** One verdict and its event: `review.merged`,
  `review.changes` or `review.owner`. For a *changes* or an *owner*
  verdict, the review itself on the pull request; a merge writes nothing
  there. For a hand-over, a question on the project's board and a
  message on the task.
- **The gate to the next phase.** A verdict of *merge*, with GitHub's own
  gate passed. Nothing is merged, whoever asks, on a head that moved
  since the review, on a draft, while a check fails or still runs, or
  while GitHub would block the merge.
- **When it goes wrong.** A review job that ends without a verdict is
  tried once more; after two the pull request goes to the owner. An
  approved merge that cannot go through yet keeps its verdict. While a
  check still runs, every pass reads the pull request again and merges
  it once the checks pass. When GitHub fails the merge for a reason that
  passes (a 5xx answer, a rate limit, a dropped connection), the merge
  is tried again after 5, 15, 45 and 135 minutes, then every 6 hours.
  After `review.merge_wait_tries` such failures (5), or
  `review.merge_wait_hours` hours of waiting (6), the pull request goes
  to the owner.
- **Where you see it.** The task's delivery line in the console says
  where its pull request stands. One handed to the owner shows the
  reviewer's reasons and a **Merge** button ([Tasks](console.md#tasks)).
  In a terminal: `hive review list`, `hive review show 7`,
  `hive review merge 7 --by owner`. Settings: `review.merge`,
  `review.max_files`, `review.max_lines`.

| Verdict | What it means | What happens next |
| --- | --- | --- |
| merge | the diff does what the title says, the check passes, and the change is safe and in scope | the reviewer merges it on GitHub with a merge commit |
| changes | the change is right but not ready: a failing test, a missing case | the note goes on the pull request, one follow-up job is queued, and the reviewer reads the new head |
| owner | a person should decide | the findings go on the pull request, and it waits for the owner |

Some pull requests go to the owner whatever the reviewer thinks. The
runner works five reasons out from the pull request before the review
starts: `review.merge` is `ask`; the pull request touches a guarded path
(the owner's paths, a workflow file, an `.env` file, a file named like a
secret or a key); the branch is not named `hive/...` or `evolve/...`; a
change to a screen came without its before/after report; the pull
request builds a Tier 2 decision record the owner has not accepted. That
last one reads the task the pull request delivers: when the task's title
or brief names a Tier 2 ADR, or the ADR's body names the task, and the
ADR is Proposed or Deferred, the reason is "builds ADR-N, which the
owner has not accepted" ("which the owner declined" once it is
Rejected), and the reviewer reads the pull request and hands it over
instead of merging it. Another reason comes from the attempts: two
review jobs ended without a verdict. Size alone hands nothing over: a
large pull request is read file by file.

When the owner declines a record whose work was merged before the
decline, the decline dialog says so before the owner picks, naming the
merged pull requests. After the decline, the next maintenance pass
queues one job, *Take out the merged work of ADR-N*, which reverts those
merges on a new pull request; it is queued once per record, and work
merged after the decline is left alone.

The pull requests of one project form a stack. While one of Hive's is
open, the next is built on it and targets its branch. The reviewer takes
the stack from the bottom, so a pull request left waiting holds the ones
above it.

Read more: [The reviewer](review.md),
[The complexity rule](review.md#the-complexity-rule),
[When an approved merge waits](review.md#when-an-approved-merge-waits),
[Stacked pull requests](landing.md#stacked-pull-requests),
[Review and merge the work](how-to/pull-requests.md).

### Deliver

- **What it is for.** It takes a finished branch to the project's
  upstream as a pull request, and brings the merged result back to the
  checkout.
- **What starts it.** A finished job with a branch. For a task, the
  moment none of its jobs is queued, running or blocked.
- **Who acts.** The runner, for every job: a job's agent commits and
  never pushes. A session's agent is told to run `hive land --branch`
  itself when the work is ready, which pushes the branch and opens the
  pull request; when it does not, the runner does. Either way the push
  uses the git credentials of the machine, and GitHub is reached through
  `gh`.
- **What it leaves behind.** The pull request, whose title and body say
  what the work set out to do. The `land.*` events, which are the whole
  history of a delivery: `land.pr` when a pull request opens or is
  updated, `land.merged` when it was merged, `land.pr-closed` when it was
  closed unmerged, `land.fast-forward` when the local default branch
  moved.
- **The gate to the next phase.** The pull request is merged on the
  upstream. Then the task is marked done and the local default branch
  follows the upstream by fast-forward. Nothing is merged locally.
- **When it goes wrong.** A branch that cannot be delivered is held with
  its reason (`land.pr-held`). A hold about the way to GitHub (a push, a
  fetch, a `gh` call, a credential) is tried again after 5, 15, 45 and
  135 minutes, then every 6 hours. A hold about the work itself stands
  until the work changes. A task's branch that does not merge gets one
  rebase job, and a second conflict is the owner's to settle. A local
  default branch that has diverged from the upstream is never merged for
  the owner: when it holds no work of its own the runner realigns it and
  keeps the old tip, and otherwise it asks once.
- **Where you see it.** The same delivery line on the task. In a
  terminal: `hive land --dry-run` lists the next delivery and pushes
  nothing, `hive land` runs one pass now, and `hive events --project
  my-app` shows the events. Settings: `landing.auto_stash`,
  `landing.ui_report`, `push.credential`.

Delivery by pull request needs the project's upstream to be on GitHub. A
project with no upstream has no pull request and no reviewer: the runner
merges each finished branch into the local default branch once the
project's check passes.

Read more: [Delivery](landing.md),
[When delivery fails](landing.md#when-delivery-fails),
[The local default branch follows the upstream](landing.md#the-local-default-branch-follows-the-upstream),
[Events](landing.md#events).

### Release

- **What it is for.** It turns merged changes into something people run.
- **What starts it.** For your project: whatever started it before Hive.
  For Hive's own source: a workflow that runs every night and does
  nothing when the default branch has not moved since the last release.
- **Who acts.** For your project: its own pipeline or script, or the
  owner. Hive has no release or deploy step built in. Deployment is a
  hard stop, and Tier 2 names it: agents are told to stop and ask. A
  job's agent is also told never to push, tag or publish.
- **What it leaves behind.** For Hive's own source: a tag `vX.Y.Z` and a
  public release with archives for four targets and one `SHA256SUMS`
  file. For your project: nothing of Hive's, beyond the runs of a script
  the project keeps as a check. The console's reports name the first
  release tag that contains a delivered commit.
- **The gate to the next phase.** For your project: the owner's yes, or
  the gates of your own pipeline. For Hive's own source: when one target
  does not build, nothing is published.
- **When it goes wrong.** A pipeline that goes red on the default branch
  after a merge is found by the pipeline watcher: a finding, and one fix
  job while `pipeline.watch` is `fix`. For Hive itself, `hive update`
  keeps the old binary as `hive.prev`, and a new build that exits three
  times in a row is put back by its launcher.
- **Where you see it.** In a terminal: `hive pipeline`,
  `hive update --check`, `hive update`, `hive version`. Settings:
  `update.channel`, `update.latest_url`, `daemon.self_update`.

Hive does offer two things around a release of your project. The built-in
`release` role lead looks after the changelog, the version and the notes
of each release. And a `check` script can watch that what you deployed
still answers.

A project can also keep the release step itself as a check under
`.hive/automations/`, and Hive's own repository does:
`.hive/automations/site-shows-main.sh` deploys the public site when it
falls behind the default branch. The runner runs it every half hour, and
it deploys only from a clean tree on the upstream's default branch, on a
machine whose owner is signed in to Vercel, after the owner allowed it
([The site, hivefleet.dev](development.md#the-site-hivefleetdev)). That is
the project's own script: Hive itself still has no release step.

How a newer Hive reaches a machine depends on `update.channel`. On the
`source` channel the runner rebuilds `hive` from a checkout of Hive's own
source after a merge there that changes what the binary is built from,
once the default branch has stood unchanged for
`daemon.self_update_settle_minutes` (60). It tries the new binary,
installs it and restarts once the runs in flight have ended. On the
`release` channel
`hive update` downloads the archive for the machine, checks it against
`SHA256SUMS`, tries it and puts it in place.

Read more: [Releases and `hive update`](release.md),
[How a release is cut](release.md#how-a-release-is-cut),
[Services and self-update](runner.md#services-and-self-update),
[Run work on a schedule](how-to/schedules.md).

### Operate

- **What it is for.** It keeps the project's standing work going and the
  machine healthy while nobody watches.
- **What starts it.** A cadence, an event or a signal from outside.
- **Who acts.** The runner, without a model, for scripts and upkeep.
  Agents for the jobs that a schedule, a red check or a watch queues.
  Role leads on their cadence, such as the `monitor` lead every six
  hours.
- **What it leaves behind.** Each run of a script, with its exit code and
  the end of its output. A finding on the board when a check turns red
  and when it turns green again. Jobs. Events such as `schedule.ran` and
  `schedule.red`. A copy of the store when `backup.to` names a place.
- **The gate to the next phase.** None: operation feeds
  [Intake](#intake) and [Learn](#learn). A hard stop still waits for the
  owner.
- **When it goes wrong.** A red check queues one job to make it pass. A
  disk under `daemon.min_free_mb` (10240 MB) stops new work, and the
  runner clears what it can. While the machine is loaded the runner
  starts none of the agents' own work, and one run of the owner's work
  still starts. A runner that stays silent for half an hour writes a
  dump and exits, and its service starts it again.
- **Where you see it.** The console lists the automations with their
  next run and last results, and the machines with their state
  ([Automations](console.md#automations), [Machines](console.md#machines)).
  In a terminal: `hive status`, `hive schedule list`,
  `hive executor list`, `hive backup --status`, `hive sweep --held`.
  Settings: `owner.presence`, `backup.to`, `daemon.keep_awake`,
  `compute.max_vms`.

| What runs here | How |
| --- | --- |
| Automations | A project keeps its standing work as files under `.hive/automations/`, reviewed and merged like any other file. Every machine that tends the project applies them from the default branch. A standing job costs one agent run each time; a `check` and a `watch` cost a model call only when there is something to do. |
| Upkeep | The runner sweeps what runs left behind every ten minutes, recovers failed jobs, copies the store to `backup.to`, keeps the machine awake when `daemon.keep_awake` says so, and updates itself. |
| Compute | With a cloud provider set and caps above zero, an agent may ask for one more machine, and the owner approves it. A rented machine is released when its hours are up or after 30 minutes without a run. |
| Secrets | For the push key and a GitHub App's key, Hive keeps a reference to where the key lives, never the key, and resolves no reference inside a job. Model logins, and the token for the other machines' model sign-in, are files that only your user can read. The vault (`hive secret set NAME`) seals a value in the store; a project's recipe names the secrets its jobs may have, and a run on the brain gets them for that run only. |
| The owner is away | With `owner.presence` set to `away`, each project's lead decides the proposals of its project and the findings, handoffs and suggestions marked for the owner. The decide role answers those marked findings, handoffs and suggestions first, and the proposals stay with the lead. Machine requests, decision records, the owner's paths, spend settings and major dependency upgrades still wait. |

Read more: [Run work on a schedule](how-to/schedules.md),
[Sweep, retention and hygiene](recovery.md#sweep-retention-and-hygiene),
[Backups of the store](runner.md#backups-of-the-store),
[Secrets](security.md#secrets),
[When the owner is away](recovery.md#when-the-owner-is-away).

### Learn

- **What it is for.** It turns what the work showed into the next work,
  and improves the project and the way agents work on it.
- **What starts it.** The end of a run, a quiet project, a cadence, or a
  cause that failed twice.
- **Who acts.** Every agent, the planner, role leads such as the RSI
  lead, and the owner for what is marked for the owner.
- **What it leaves behind.** Findings and suggestion cards on the board,
  with their votes. A lesson, kept as a memory record, when a cause fails
  twice; every later worker brief of that project carries it. The facts
  an agent keeps with its `memory` tool. A row in the loop's archive for
  every measured experiment, kept or not.
- **The gate to the next phase.** Back to [Intake](#intake), through the
  same gates as any work. A card must name a check that fails on the
  default branch today and passes once the change is in; a card without
  one is held. Voting is the quick filter: a card that names a check does
  not wait for the `quorum` (2 voters), and a quorum that votes it down
  still stops it. An agent's own vote does not count, and a vote merges
  nothing.
- **When it goes wrong.** Research that keeps finding nothing backs off.
  A thread whose last three jobs committed nothing gets no more jobs,
  and the owner is asked. When an experiment failed and its result names
  a hard stop or a violation, the loop plans no new experiment in that
  project and asks the owner.
- **Where you see it.** The console shows the suggestions with what
  holds each, and a project's experiments with what was kept
  ([Votes](console.md#votes)). In a terminal: `hive board`, `hive ballot`,
  `hive reflect`, `hive evolve status`, `hive evolve archive`,
  `hive goal review`, `hive agents`, `hive record list memory`. Settings:
  `quorum`, `evolve.loop`, `evolve.tolerance`, `lead.research_minutes`.

| Part | How it works |
| --- | --- |
| Reflection | Agents are told to record what they learnt as a finding before they stop. Once a day, when nothing else is planned for a project, the planner queues one reflect job: run the checks, fix one small defect that can be proved, and post what else it saw. |
| Research | A project with nothing to plan for `lead.research_minutes` (60) gets a research job. It looks at one focus, such as the product as its user sees it, and is asked for one to three cards with evidence; the focus on the loop's own report leaves findings only. |
| From a card to a change | A card that names its check, is not voted down and passes the planner's gates is built as an ordinary job. Work bigger than a card is filed with `hive task suggest`. |
| The loop's experiments | Off until the owner turns them on (`evolve.loop`); they need a constitution, evaluators, fixtures, a baseline and two private tests first. The loop builds the top card on a branch `evolve/<id>` and scores that branch and the default branch on the same fixtures: the public ones, the card's own check, and private tests (held-out fixtures) kept outside every repository. A result that falls below the baseline by more than `evolve.tolerance` (0.02) is discarded, and a gain is measured a second time before it counts. A kept result becomes a pull request whose body is the measurement. |
| Role leads | Each sits on its own cadence. The RSI lead sits daily on the loop's plan, votes, experiments and archive. |
| The goal review | `hive goal review` scores the last work against the goal, out of 100, from four signals: delivery, reliability, direction and momentum. The lead may propose a new goal. |
| The rules themselves | Agents may amend the constitution by an informed vote. The hard stops, the kill switch and the amendment rules cannot be amended by vote. |

Read more: [Democracy and self-improvement](self-improvement.md),
[Turn on the self-improvement loop](how-to/self-improvement-loop.md),
[Lessons, briefs and peer help](recovery.md#lessons-briefs-and-peer-help),
[Goal review](board.md#goal-review-how-far-the-work-has-moved-it).

## Gates in one table

Who may make a change alone, what the reviewer adds, and what waits for
the owner. The last column says what holds the line: the program, or a
rule the agents are given.

| Change | Agents alone | The reviewer | The owner | What holds it |
| --- | --- | --- | --- | --- |
| Tier 0: polish and repair | yes, as a job that passes the relevant checks | merges it when no reason hands it over | nothing | the check, by the program; the tier is the agent's judgement |
| Tier 1: behaviour inside an accepted decision | yes, as a job with a work-unit record that names the decision | the same | nothing | the check, by the program; the record is a rule |
| Tier 2: a real decision | no: a job marked Tier 2 is refused | hands over a pull request that needs one | a decision record and an explicit yes | the queue refuses the number on the job; sorting work into a tier is a rule |
| A hard stop | no: agents are told to stop and ask | hands it over | an explicit yes | a rule: apart from the rule on commands, nothing detects a hard stop as such; the rows below name the cases the program does hold |
| The owner's paths, a workflow file among them | no: a card that names one becomes no job, and a job whose commits touch one fails | cannot merge it | makes the change, or merges it | the program |
| An `.env` file, a file named like a secret or a key, another forge's pipeline file | no: the pull request is handed over | reads it and never merges it | merges it | the program refuses the merge without `--by owner`; who types that is a rule |
| A branch not named `hive/...` or `evolve/...` | does not apply | reads it and never merges it | merges it | the program, by the branch's name alone |
| Every pull request, when `review.merge` is `ask` | does not apply | reads it and merges nothing | merges each one | the setting |
| A security update inside the installed version's compatible range | yes: the watcher queues it once the version is `dependencies.min_release_age_days` (3) old | merges it like any other | nothing | the program |
| A major dependency upgrade | no | does not apply | answers the finding, or sets `dependencies.major_updates` to `auto` | Tier 2 by the constitution |
| Renting a machine | may ask for one | does not apply | approves or denies; the caps are 0 until the owner sets them | the caps, by the program; who approves is a rule |
| A decision record | may propose one | does not apply | accepts, defers or declines | a rule: `hive record accept` is not refused to an agent |
| The goal | the lead may propose one | does not apply | sets it | the program |
| The constitution | may amend it by an informed vote, except the protected sections | does not apply | adopts a draft; the owner's revision outranks any vote | the program keeps votes off the protected sections and refuses an agent the adoption of a draft; that agents leave the text alone is a rule: `hive record amend` is not refused to an agent, and the revisions show who wrote each one |
| The kill switch and the caps | no | does not apply | only the owner | a rule, not a lock: the event log records who ran it |

Read more: [Set the rules agents work under](how-to/rules.md),
[Agents](security.md#agents),
[The complexity rule](review.md#the-complexity-rule),
[Answer what waits for you](how-to/what-needs-you.md).

## The trace of one change

Every step of a change has an id or an event. This is one task, followed
from the owner's sentence to the default branch.

```text
┌──────────┐    ┌──────┐    ┌──────┐    ┌─────────┐    ┌──────────────┐
│ TASK-001 │ ─→ │ jobs │ ─→ │ runs │ ─→ │ commits │ ─→ │ pull request │
└──────────┘    └──────┘    └──────┘    └─────────┘    └───────┬──────┘
                                                               ↓
┌───────────────────────┐    ┌───────────────────┐    ┌───────────────┐
│ daemon.self_update    │ ←─ │ land.merged       │ ←─ │ review.queued │
│ Hive's own build, if  │    │ land.fast-forward │    │ review.merged │
│ its sources changed   │    │                   │    │               │
└───────────────────────┘    └───────────────────┘    └───────────────┘
```

| Step | What names it | Where to read it |
| --- | --- | --- |
| The task | `TASK-001` | `hive task show TASK-001` prints the brief, its versions under REVISIONS, the conversation under MESSAGES and the jobs under JOBS |
| The lead's plan | the lead's reply, and one row per job | the same command; in the console, the task's conversation and its steps |
| The jobs | a job id such as `job-20261123T041810Z-7cdfa8`, with the source `task:TASK-001` | `hive jobs --project my-app --all` |
| The runs | one run for each attempt at a job | the console's list of runs; `hive harness sessions`, `hive harness show ID` |
| The commits | the branch `hive/<agent>/<job>`; the trailers `Hive-Agent`, `Hive-Job` and `Hive-Executor`; the signature of the run's key | `hive verify <commit> --project my-app` prints the trailers, the run and a verdict |
| The pull request | the branch `hive/task-TASK-001`, a title that starts `TASK-001:`, the event `land.pr` | `hive review list --project my-app`, `hive review show 7` |
| The review | `review.queued`, then `review.merged`, `review.changes` or `review.owner` | `hive watch --project my-app --action 'review.*' --since 0 --detail` |
| The merge | `land.merged`, then `land.fast-forward` | `hive events --project my-app` |
| The build, for Hive's own source | `daemon.self_update`, with the revision it ran and the one it built; only after a merge that changes what the binary is built from, once the default branch has stood unchanged for `daemon.self_update_settle_minutes` (60) | `hive events`; `hive version` prints the version the build was stamped with |

In the console the task's conversation holds the same story in one feed:
the brief and the messages, the jobs, each run with its commands, the
delivery and the status changes. For any project but Hive's own source
the trace ends at the merge.

Read more: [Tasks](console.md#tasks),
[The pull request](landing.md#the-pull-request),
[Events](review.md#events), [Commands](commands.md).

## What to measure

Hive reports these today:

| Where | What it shows |
| --- | --- |
| `hive status` | the jobs failed and queued; `runs today 0 of 2 (jobs_per_day)`; `cost today`, one line per kind of job with its runs, its tokens and the cost where one was recorded; `outcomes`, a count of how the day's runs were classified; what holds each project's work |
| The runs in the console | each run's duration, tokens and recorded cost, and how it ended |
| Insights in the console | per day: landings, jobs done, tasks done and agent runs; tokens in and out; the reviews agents finished and how many they merged without the owner; how often the agents asked the owner |
| `hive agents` | one scorecard per agent: its presence, what it does now, its posts, findings, suggestions and votes, its jobs done out of the jobs it finished, its last heartbeat and its last useful output |
| Reports in the console | every done task as a work report: what each job landed and where, the first release tag that contains it, whether the check passed, the evidence |
| `hive goal review` | a score out of 100 and a verdict: on track, slipping, stalled or too quiet to score |
| `hive evolve report` | the window's counts: suggestions and where the vote left them, jobs done with and without a change, experiments, questions that wait for the owner |
| `hive record lead-time` | the days from proposed to delivered, for decision records, plans, work units and features |
| `hive pipeline` | the last verdicts on the pushes Hive made and on the head of the default branch |

What Hive does not compute as such today:

- **Lead time for changes.** No command reports the time from a task to
  its merge. The loop's report has a row for the lead time from a job to
  its landing, but it counts local landings only, so for a project with
  an upstream it is empty.
- **Change failure rate and time to restore.** Neither is computed. The
  nearest figures are the goal review's reliability signal and the
  verdicts of `hive pipeline`.
- **Deployment frequency.** Hive does not deploy your project, so it
  counts merges, not deployments.
- **Money.** Hive records a cost only when the provider reports one, and
  most replies carry none. It records the tokens of each run. `hive usage`
  is something else: it reads the token counts that other agent programs
  keep on this machine, and its own help calls it never a ledger.

Read more: [Runs](console.md#runs), [Reports](console.md#reports),
[Goal review](board.md#goal-review-how-far-the-work-has-moved-it),
[Choose models and set budgets](how-to/models.md).

## Compared with a traditional SDLC

*Traditional* means, on this page, the phase model most teams know:
requirements, design, implementation, testing, deployment and maintenance,
each done by people in roles. A team may run it in one pass (waterfall) or
in short cycles (agile), with DevOps practices such as continuous
integration for the delivery. A good team does all of this well. This
section says what differs, not that one way is better.

Read more: [Concepts](concepts.md), [Questions people ask](faq.md).

### Phase by phase

| Phase | Traditional: who and what | Traditional: what it leaves behind | In Hive: who and what | In Hive: what it leaves behind | The gate in Hive |
| --- | --- | --- | --- | --- | --- |
| Requirements | a product owner or an analyst gathers what users and stakeholders need | a backlog item, a user story, acceptance criteria | [Intake](#intake): the owner gives a task; agents add suggested work, findings and signals from scripts | a task with its brief, a suggestion, a finding, a queued job | none for the owner's task; a second agent's verdict for suggested work |
| Analysis and design | architects and developers analyse and design, and a lead breaks the work down | a design document, tickets, estimates, sometimes a decision record | [Shape and plan](#shape-and-plan): the lead reads the task, asks only what it must and cuts jobs | the lead's reply, jobs with briefs, a proposed decision record | Tier 2 is refused as a job: a decision record and the owner's yes |
| Implementation | developers write code on branches | commits on a branch | [Build](#build): one worker per job, in a throwaway clone, inside budgets | signed commits with trailers, a run with its transcript | the check at every commit |
| Testing | developers and testers: tests in continuous integration, often a test phase | test results, bug reports | [Verify](#verify): the project's own check at each commit, at the run's end and in the review; the repository's pipeline | the run's verification, pipeline verdicts | a run whose check fails delivers nothing |
| Code review | a colleague reads the pull request | comments and an approval | [Review](#review): a review job reads every pull request Hive opens; the owner reads what the rule hands over | one verdict and its events | the verdict, GitHub's own checks, the complexity rule |
| Integration and delivery | the author merges, and a server builds the main branch | a merge commit, a build | [Deliver](#deliver): the runner pushes, opens the pull request, records the merge and follows the upstream | a pull request and the `land.*` events | merged on the upstream |
| Release and deployment | a release manager or a pipeline ships; a change board may approve | a version, release notes, a deployment | [Release](#release): your project's own pipeline or script, or the owner; Hive has no release step built in | nothing of Hive's for your project | deployment is a hard stop: the owner's yes |
| Operations and maintenance | an operations team, or the developers on call | monitoring, incidents, tickets | [Operate](#operate): the runner runs schedules, checks and watches and keeps the machine healthy; role leads sit on a cadence | findings, jobs, events | a red check queues one job; hard stops still wait |
| Retrospective and process improvement | the team, in a retrospective | notes and action items | [Learn](#learn): agents reflect, post suggestions and vote; the loop measures experiments | findings, cards, votes, lessons, archive rows | a check that fails today and passes with the change |
| Governance | managers, architects, a change board | policies, a roadmap, decision records | [Direction](#direction): the owner writes the goal and the constitution and accepts decision records | the goal, the constitution's revisions, accepted records | only the owner sets them |

Read more: [The loop](#the-loop), [Gates in one table](#gates-in-one-table).

### Practices side by side

| What a team does | Traditional form | In Hive |
| --- | --- | --- |
| Backlog and prioritising | a product owner orders a backlog | A task carries a priority, and a higher one runs first. The owner's work is leased before the agents' own. Suggestion cards are ordered by their votes, product cards first ([Suggestions](self-improvement.md#suggestions)). |
| Sprint or iteration planning | the team commits to a set of items for a fixed period | Not in Hive. The lead plans a task when it arrives, and the planner looks again every pass. `jobs_per_day` caps a day of the agents' own work. |
| Stand-ups and status reports | a daily meeting, a written report | Not in Hive as a meeting. The task's conversation, `hive status`, the work reports and `hive ask` say where the work stands ([Ask](console.md#ask), [Reports](console.md#reports)). |
| Hand-offs between roles | a meeting, a ticket, a document | A written brief for each job. A `handoff` post for work a worker began and left unfinished, which another worker takes ([The board](board.md)). |
| The definition of done | a checklist the team agreed on | The project's check passes at every commit and at the run's end. A task is done when its pull request is merged ([Tasks and jobs](concepts.md#tasks-and-jobs)). |
| Unit and integration tests | written by developers, run on their machines and in continuous integration | The project's own tests, run by its check. Hive adds no test framework. The lead is told to cut jobs that each finish a slice with its tests, and a `tests` role lead looks after what is untested or flaky. |
| A QA phase | testers check a build before it ships | Not in Hive as a phase. The check runs at each commit, and a change to a screen needs a before/after report ([Before/after reports](reports.md)). Exploratory testing by a person stays with you. |
| Code review | a colleague approves the pull request | The review job and the complexity rule ([The reviewer](review.md)). With `review.merge` set to `ask`, every merge is the owner's. |
| Continuous integration | a server builds and tests each push | The repository's own workflows on GitHub. Hive watches them, never merges a pull request whose check fails or still runs, and queues one fix job for a red pipeline ([Pipelines](landing.md#pipelines-what-a-push-set-running-on-github)). |
| Release management and change approval | a release manager, a change board | Not in Hive for your project: use your own pipeline, or keep the step as a check under `.hive/automations/`, as Hive's own repository does for its public site ([The site, hivefleet.dev](development.md#the-site-hivefleetdev), [Run work on a schedule](how-to/schedules.md)). Approval exists as hard stops, Tier 2 decision records and `review.merge` ([Set the rules agents work under](how-to/rules.md)). |
| On-call and incident response | a rota, a pager, an incident process | Not in Hive: nobody is paged. Check and watch scripts, the pipeline watcher and the `monitor` role lead find trouble and queue a job ([Run work on a schedule](how-to/schedules.md)). A store hook can post an event to a web address of yours ([Events, hooks and channels](board.md#events-hooks-and-channels)). |
| Retrospectives | a meeting at the end of each iteration | Reflect and research jobs, findings, a lesson when a cause fails twice, the goal review ([Learn](#learn)). |
| Documentation and decision logs | a wiki, decision records | Records in the store with their revisions, decision records under `docs/decisions/` imported by themselves, the event log, and a `docs` role lead for the guides ([Records in SQLite](board.md#records-in-sqlite)). |
| Onboarding a new team member | reading, pairing, requests for access | A new project gets an onboarding job that reads the repository, runs its checks and writes the environment recipe. Each run reads the repository's instructions, the project's memory, the goal and its brief. A new machine gets `hive`, a model login of its own and the tools the recipe pins ([Set up your first project](how-to/first-project.md), [Add a second machine](how-to/add-a-machine.md)). |
| Estimating | story points, estimates in days | Not in Hive. The lead is told to cut jobs that a worker can finish in one run, and the budgets bound each run ([Choose models and set budgets](how-to/models.md)). |
| Capacity planning | head count, the capacity of a sprint | Each machine works out its slots from its cores and memory (`capacity`), and the projects with work share them. `jobs_per_day` caps the day. More machines come by pairing, enrolling or renting inside the caps ([The load governor](landing.md#the-load-governor), [Machines](machines.md)). |

Read more: [Give a task and steer it](how-to/tasks.md),
[Seat role leads on a project](how-to/role-leads.md),
[The runner](runner.md).

### What changes

**Where the time goes.** In a traditional lifecycle part of a change's
time is waiting between roles: for a reviewer, for a tester, for a release
window. A team that works well keeps those waits short. In Hive a change
waits for a free slot, for the project's check, and for the owner at a
hard stop. That last wait is the one to watch: a pull request left with
the owner holds the ones stacked on it.

**What a hand-off is.** Between people a hand-off is a meeting, a ticket
or a conversation, and the next person can ask what the ticket left out.
Between agents it is a record: the brief, the result, the posts on the
board, the pull request's body. The next agent knows only what was
written, so a thin brief gives thin work.

**When testing happens.** A team with continuous integration already tests
every push. What differs is where else the check stands: before every
commit in the worker's clone, again when the run ends, and again where the
reviewer checks the pull request out. A run whose check fails delivers
nothing.

**Who reviews.** A colleague reviews a colleague's pull request. In Hive a
review job reads every pull request Hive opens, in a clone of its own, and
never fixes the code itself. The owner reads the ones a rule hands over,
or every one with `review.merge` set to `ask`. The reviewer never merges
a pull request whose branch is not named `hive/...` or `evolve/...`.

**How the process itself improves.** A retrospective produces notes, and
the team decides what to try. In Hive an agent posts a suggestion, others
vote, and the suggestion must name a check that fails today and passes
with the change. With the loop on, an experiment is measured against a
baseline before it is kept. All of it runs under a constitution the owner
owns.

**What is written down.** A team writes its record beside the work or
after it: the ticket is updated, the decision is logged. Here the record
is a step of the work itself: every write to the store appends an event,
every commit names its agent, its job and its machine, and the pull
request's body says what the work set out to do.

Read more: [Delivery](landing.md), [The reviewer](review.md),
[Democracy and self-improvement](self-improvement.md).

### What does not change

- **A person owns the direction.** The goal, the rules, the decisions of
  record and the hard stops are the owner's.
- **The project's own tests say what is right.** Hive runs the check it
  is given. A project with a weak check gets weak proof, and a project
  with no check gets none.
- **Someone decides what is worth building.** The owner's tasks and the
  goal come first. Agents choose small work towards that goal themselves,
  and a real decision waits for the owner.
- **Risky changes still reach a person.** A change to a workflow, to an
  `.env` file or to one of the owner's paths, and anything at a hard
  stop, is the owner's to decide.

Read more: [Set the rules agents work under](how-to/rules.md),
[The verify command](landing.md#the-verify-command).

### What you give up, and what to watch

- **Model use.** Every run costs model calls: jobs, reviews, sittings,
  votes and research, all billed to your own provider account. The
  console shows the tokens of each run, and `hive status` the day's use
  by kind of job. `jobs_per_day`, `harness.max_steps` and
  `minutes_per_job` bound it.
- **The check and the brief decide the quality.** An agent proves its
  work against the project's check and plans from the words of the brief.
  The work cannot be better than both.
- **Agents can be wrong with confidence.** That is why the gates exist:
  the check, the reviewer, the owner at the hard stops. The reviewer is
  an agent too. Where a wrong merge would cost much, set `review.merge`
  to `ask`. No sandbox holds a job, so treat a repository you point Hive
  at like code you would run yourself.
- **What a traditional lifecycle covers and this one leaves to you.**
  Talking to users and stakeholders. Product discovery. The design of
  the user experience by people. Exploratory testing by a person.
  Releases, staged roll-outs and deployments. Monitoring of production
  beyond the checks and watches the project writes itself.
- **A tool you run, not a service for a team.** Nobody hosts Hive, and a
  team does not get one shared Hive out of the box.

Read more: [What is not built](security.md#what-is-not-built),
[Choose models and set budgets](how-to/models.md).

### When a traditional process is still the better fit

A traditional process is the better fit where no check can judge the
work: a change whose worth is a matter of taste, or of what users say,
gives an agent nothing to prove. It is also the better fit where a
regulation asks that a named person signs off every change: there an
agent's merge has no place.

The two mix. Hive delivers by pull request to the project's own upstream,
so its work can pass through a team's own review and pipeline. Set
`review.merge` to `ask` and the reviewer merges nothing. Hive never merges
while a check on the pull request fails or still runs, or while a rule of
the repository blocks the merge. A pull request that a person merges or
closes on GitHub is noticed on the runner's next pass.

Read more: [Review and merge the work](how-to/pull-requests.md),
[Your part](review.md#your-part),
[ADR-069](decisions/ADR-069-delivery-by-pull-request-with-an-agent-reviewer.md).

## Adopt it on a project

1. **Install Hive and open the console.** [Install](../README.md#install).
2. **Connect a model provider, register the project with its check and
   keep the runner running.** The check is how Hive decides that a change
   is good, so give the command you would trust in a pipeline.
   [Set up your first project](how-to/first-project.md),
   [Keep Hive running, updated and backed up](how-to/operations.md).
3. **Set the goal.** One outcome, and how you would see the progress. The
   first guide has the step.
4. **Read the rules and make them yours.** The constitution, the tiers,
   the owner's paths and the kill switch.
   [Set the rules agents work under](how-to/rules.md).
5. **Give the first task.** Say what you want and why, how you will know
   it is done, and what to leave alone.
   [Give a task and steer it](how-to/tasks.md).
6. **Review the first pull request, and choose who merges.** Set
   `review.merge` to `ask` to see every one before it merges. Then make
   the round of what waits for you.
   [Review and merge the work](how-to/pull-requests.md),
   [Answer what waits for you](how-to/what-needs-you.md).
7. **Add schedules and checks, and seat role leads.**
   [Run work on a schedule](how-to/schedules.md),
   [Seat role leads on a project](how-to/role-leads.md).
8. **Set the budgets, and add a machine when one is not enough.**
   [Choose models and set budgets](how-to/models.md),
   [Add a second machine](how-to/add-a-machine.md).
9. **Turn the improvement loop on when its gates are open.**
   `hive evolve status` names the gate that is still closed.
   [Turn on the self-improvement loop](how-to/self-improvement-loop.md).

Read more: [Guides](how-to/README.md),
[Troubleshooting](how-to/troubleshooting.md).

## Not in place yet

What a reader who knows other lifecycles would look for, and Hive does
not do today:

- **A release or deploy step for your project.** Hive stops at the merge.
  A project can keep such a step itself as a check under
  `.hive/automations/`, as Hive's own repository does for its public site
  ([The site, hivefleet.dev](development.md#the-site-hivefleetdev),
  [Run work on a schedule](how-to/schedules.md)). That is the project's
  own script, not a step of Hive's.
- **The classic delivery measures.** Lead time for changes, change
  failure rate, time to restore and deployment frequency are not
  computed.
- **A sandbox for jobs.** A job runs as your user inside its clone, with
  the network open.
- **Hard stops checked by the program.** Apart from the rule on commands,
  nothing detects a hard stop as such. Agents are told to stop and ask.
  The program does hold some cases, such as a file named like a secret
  and what may be spent on machines.
- **A repair for a default branch that is red under the project's own
  check.** A job that meets one is run again later, and nothing sets out
  to fix the branch unless the project keeps a check for it. A red
  pipeline on GitHub does get one fix job.
- **Pull requests on a git host other than GitHub.** Delivery by pull
  request is built on the GitHub CLI.
- **Paging.** Hive tells the owner in the console, and a notification
  comes only while a console tab is open.
- **A hosted Hive, or one shared Hive for a team.** Nobody runs Hive for
  you, and none is planned. The pieces a team could run one shared Hive
  with exist as options and are not finished.

Read more: [What is not built](security.md#what-is-not-built),
[A red default branch](landing.md#a-red-default-branch),
[ADR-071](decisions/ADR-071-hive-is-an-open-source-tool.md).
