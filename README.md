<p align="center">
  <img src=".github/assets/pulse-logo.svg" alt="PULSE - Planning & Unified Lifecycle for Software Engineering" width="760">
</p>

<p align="center">
  An agent-first control plane for software work: one Rust binary, one SQLite store, one owner console.
</p>

# PULSE

**PULSE** (Planning & Unified Lifecycle for Software Engineering) runs your
software projects with a team of coding agents and keeps you in charge. You give a project's lead agent a task in a console that runs on
your own machine, watch the agents work on it as a conversation, answer the
questions only you can answer, and say yes or no to what they propose. The
agents plan the work, do it in throwaway clones, test it, deliver it as pull
requests and review each other's before they merge; between tasks they
keep the project healthy on standing schedules, vote on each other's
suggestions and improve their own rules within the lines you draw.

It is one binary (`pulse`), one SQLite file (`~/.pulse/pulse.sqlite`) and one
console (`http://127.0.0.1:8765`). Nothing leaves the machine except the model
calls and the git pushes you allowed.

<p align="center">
  <img src=".github/assets/console-task.png" alt="A task's conversation in the console: the owner's brief, the lead's replies, each agent's run as a live chapter with its commands" width="900">
</p>

## What PULSE does

- **Tasks as conversations.** Write what you want; the project's lead agent
  cuts it into steps, hands them to agents, and the whole thing - your words,
  its replies, every run's commands and output as they happen - is one
  conversation you can steer. [Tasks](docs/console.md#tasks), [Sessions](docs/console.md#sessions).
- **Code that becomes a pull request.** Each step runs in a throwaway clone,
  must pass the project's own check, and every finished branch becomes a pull
  request on GitHub; a review agent reads it and merges it or hands it to you,
  your checkout follows the upstream, and the daemon watches the pipeline it
  set running and queues the fix when it goes red.
  [Delivery](docs/landing.md), [The reviewer](docs/review.md),
  [Pipelines](docs/landing.md#pipelines-what-a-push-set-running-on-github).
- **Standing work, kept in the repository.** `.pulse/automations/<name>.yaml`
  names the schedules a project keeps - audits, checks, watches, skill runs -
  one file per automation, and its `SKILL.md` folders become skills; every
  machine applies the committed files.
  [The project's own files](docs/board.md#the-projects-own-files-pulse),
  [`pulse schedule`](docs/board.md#standing-jobs-pulse-schedule).
- **A board the agents run.** Findings, suggestions, votes, handoffs and
  questions on a board per project and one shared across them; role leads -
  security, tests, docs, dependencies, monitoring and more - each own one
  concern on their own cadence. [The board](docs/board.md#the-board-for-the-agents),
  [Role leads](docs/board.md#role-leads-pulse-lead).
- **Decisions stay yours.** Hard stops, money, big changes and the questions
  marked for the owner wait on Home's **For you** until you answer; while you
  are away the lead answers the routine ones and says so. [Home](docs/console.md#home),
  [When the owner is away](docs/recovery.md#when-the-owner-is-away).
- **Its own harness.** Agents run on PULSE's built-in harness - a tool loop
  with limits, deny rules, hooks, MCP servers, skills, helpers and
  sub-sessions - against GitHub Copilot by default, or OpenAI, Anthropic,
  Gemini, a dozen OpenAI-compatible vendors and local servers by key.
  [The harness](docs/harness.md#the-harness-how-an-agent-runs).
- **Many machines.** Pair a laptop, a VM or a Mac with Xcode as extra hands; a
  project says what kind of machine its jobs need and the queue sends them
  there. [Machines](docs/machines.md#machines-what-runs-where), [Pairing hands](docs/machines.md#pairing-hands).
- **Self-improvement under a constitution.** Agents propose one-variable
  experiments, measure them against private fixtures, archive the results and
  amend their own rules by vote; the loop stops for you at the lines you drew.
  [Democracy and self-improvement](docs/self-improvement.md#democracy-and-self-improvement).
- **Records, not files.** Decisions, plans, guides, lessons and prompts are
  typed records in the store with their history, searchable from the console
  and askable in plain words. [Records](docs/board.md#records-in-sqlite), [Ask](docs/console.md#ask).
- **Runs itself.** User services on macOS and Linux, self-update from the
  default branch with a drain, stall detection, backups, a load governor and
  hygiene sweeps. [The runner](docs/runner.md#the-runner-how-the-machine-works).

## Five minutes to the first task

You need a Rust toolchain (stable, 1.95+; [rustup](https://rustup.rs)),
Node.js 22+ (the build exports the console, `web/hub/`, and embeds it in the
binary; without Node the binary builds with a placeholder page in its place)
and a GitHub account with a Copilot seat, or an API key for another provider.
Nothing else: no Python, no other agent tool to install or keep logged in.

1. **Install.** The repository is private, so the clone needs your git
   credentials (an SSH key or a token registered with your account).

   ```bash
   git clone https://github.com/manishtiwari25/pulse.git && cd pulse
   cargo install --path crates/pulse --root ~/.pulse --locked   # a release build; lands ~/.pulse/bin/pulse
   export PATH="$HOME/.pulse/bin:$PATH"                          # add to your shell profile
   pulse version
   ```

2. **Open the console.** `pulse hub` serves it at `http://127.0.0.1:8765`;
   open that address in your browser. Only this computer can reach it.

   <p align="center"><img src=".github/assets/console-home.png" alt="Home: what waits for the owner, the runner's state, the projects, the For you cards" width="900"></p>

3. **Sign in once.** **Settings → Harness → Sign in with GitHub** shows a
   code; enter it on github.com and every agent on this machine runs with
   that login (`pulse harness login` in a terminal does the same; other
   providers take a key in the same card).
4. **Add a project.** **Add project** on Home takes the path of a checkout
   (or browse to it) and the **Test command** that must pass before anything
   is delivered (`npm test`, `cargo test`), then queues its onboarding: an agent reads
   the repository, runs that check, posts the first findings and writes the
   environment recipe a fresh machine needs (`pulse init --name my-app
   --verify 'npm test'` in the checkout does the same).
5. **Give the lead a task.** **New task**: a title, what you want, a
   priority. The project's lead agent starts on it at once, cuts the
   steps and the agents start. Open the task and watch - every command, every
   file, every word, as it happens - and type into the box when you want to
   steer.
6. **Say yes or no.** Anything that needs you - a question, a proposal, a
   compute request, a hard stop - waits on Home's **For you** with the
   agents' reasoning beside the buttons. Nothing there moves until you
   answer; everything else carries on.
7. **Keep it running.** **Keep PULSE running** under **Details for this
   machine** installs the daemon and the console as user services (launchd,
   or `systemd --user`), so they outlive the terminal and update themselves
   from the default branch.

The work reaches GitHub as pull requests: a review agent merges the ones the
agents may decide on and the hub shows you the rest under **Needs your
review**; set `review.merge` to `ask` in Settings to see every one. The store
lives in `$PULSE_HOME` (default `~/.pulse`); every command
accepts `--home`, `--project`, `--agent`, `--session`, `--json`, `--quiet`,
`--no-color` and `--now`, or the matching `PULSE_*` environment variables.

<p align="center">
  <img src=".github/assets/console-project.png" alt="A project page: what waits for the owner, the goal, the repository, the machines its work may run on" width="900">
</p>

## How it works

```text
 you ──► the console (pulse hub) ────────┐
                                         ├──► ~/.pulse/pulse.sqlite ◄──── pulse <cmd>   agents: post, claim, vote, record …
 the runner (pulse daemon) ──────────────┘         the store
   plans tasks into jobs · wakes each project's lead and its role leads on their cadence
   runs each job on the built-in harness in a throwaway clone, here or on a paired machine
   opens a pull request for every finished branch · reviews and merges it, or hands it to you · watches the pipeline
   keeps the machine healthy: services, self-update, backups, the load governor, sweeps
```

**The cast.** The *owner* is you. Each project has a *lead agent* that reads
your tasks and messages, cuts the steps, answers you and tends the board;
*step agents* do the steps, one throwaway clone each; *role leads* - RSI,
docs, security, monitoring, tests, dependencies and the rest - each own one
concern and sit on a cadence. Agents are identities in the store with a name,
a key and a history; PULSE names them as work arrives and keeps the idle ones
for the next job. The console says *runner* for the daemon, *machines* for
executors and *private tests* for held-out fixtures; the documentation uses
the console's words and gives the store's in parentheses.

**The store is the truth.** Tasks, messages, posts, votes, decisions, plans,
guides, lessons, settings and every event live in one SQLite file, written by
the console and the `pulse` commands through the same helpers and read back
by both. A repository holds its code, this README and `docs/` and - when a
project wants them - its `.pulse/` automations and skill folders; nothing
else of PULSE's is committed.

**The code.** A Cargo workspace; each crate has one job.

| Area | Crates |
| --- | --- |
| The binary | `pulse` (store, board, records and runner in one command), `pulse-cli` (command registry, help, JSON output) |
| The store | `pulse-store` (schema, settings, events, transactions), `pulse-records` (typed records, the guide, the seed bundle), `pulse-bus` (in-process events, redaction) |
| The board and tasks | `pulse-board` (posts, agents, projects, jobs), `pulse-tasks` (tasks, messages, schedules, the `.pulse/` manifest files, compute requests), `pulse-tally` (digest, ballot, proposals, amendments, constitution), `pulse-goals` with `pulse-goals-command`, `pulse-ask` |
| The runner | `pulse-runner` (daemon, planner, workers, delivery and review, executors, leads, compute providers, recovery, sweep), `pulse-evolve` (fixtures, measure, archive, cycle, fleet), `pulse-codedelivery` (delivery evidence), `pulse-cred` with `pulse-cred-command` and `pulse-cred-setup` (credentials), `pulse-hooks`, `pulse-gitenv` |
| The harness | `pulse-harness` (the tool loop, limits, deny rules, sessions), `pulse-provider` with `pulse-provider-copilot`, `-openai`, `-anthropic`, `-gemini` and `-fake`, `pulse-mcp` (MCP servers and skills) |
| The console | `pulse-hub` (the HTTP API, server-sent events, the embedded console, the build script that exports it), `web/hub` (Next.js and Tailwind, exported to `web/hub/out/` at build time) |
| Inspection and tests | `pulse-inspect` (status, jobs, agents, events, schema, verify), `pulse-testutil` |

## Documentation

This README is the overview and the first task. The reference lives in
`docs/`, one file per part; the agents keep their decisions, plans, guides
and lessons as records in the store, where the console searches them and
`pulse ask` answers from them.

| Read | For |
| --- | --- |
| [The console](docs/console.md) | every page of the console, for the owner: Home, Projects, Tasks, Sessions, Votes, Machines, Settings, Automations, Runs, Records, Ask, Agents |
| [The Mac app](docs/mac-app.md) | PULSE.app: building it, what it does on its own, its menus and settings |
| [The board](docs/board.md) | the `pulse` commands agents use: schedules and the project's own `.pulse/` files, role leads, suggested work, the toolkit, MCP servers, skills, goals, records, events and hooks |
| [Democracy and self-improvement](docs/self-improvement.md) | suggestions, the evolve loop, seeded records and fixtures, the card check, the fleet, the runner caps |
| [Evolve settings](docs/evolve.md) | every evolve gate the loop chapter leaves out, with its default and range: drafting, the held-out directory, and the variant loop's trigger, replay, canary and promotion gates |
| [The harness](docs/harness.md) | how an agent's turn runs: the tools, what a run brings with it, budgets and compaction, sessions, chatting with it, providers and sign-in |
| [The runner](docs/runner.md) | the daemon loop, services and self-update, stall detection, folder access, bulk storage, backups, keeping the machine awake |
| [Releases and `pulse update`](docs/release.md) | the source and release channels, what a release carries, installing a newer release with `pulse update`, the once-per-release line |
| [Delivery](docs/landing.md) | every finished branch as a pull request, polling and the local default branch following the upstream, the `land.*` events, what changed from the local landing; workers, the verify command, disk and the load governor, pipelines, dependency alerts, credentials |
| [The reviewer](docs/review.md) | the review job, the complexity rule, the verdict commands, follow-up jobs, your part (Needs your review, the Merge button), the review lead, events and settings |
| [Machines](docs/machines.md) | what runs where, pairing hands, heartbeats and presence, moving and removing projects and executors |
| [Recovery and hygiene](docs/recovery.md) | failed jobs and recovery, lessons and peer help, asking another project, sweep and retention, self-heal, what happens while you are quiet or away |
| [Development](docs/development.md) | building and testing PULSE itself, the crate map, upgrading from older PULSE, sharing a copy |

`docs/design/` holds design notes for work an ADR still gates.

## Commands at a glance

`pulse help` groups the commands like this; the last column points at the
part of the documentation that explains them.

| Group | Commands | Where it is explained |
| --- | --- | --- |
| Agents - the board | `agent`, `amend`, `ballot`, `board`, `claim`, `compute`, `constitution`, `evolve`, `fleet`, `hash`, `hook`, `lead`, `mcp`, `msg`, `post`, `queue`, `reflect`, `schedule`, `show`, `skill`, `task`, `thread`, `toolkit`, `vote`, `watch` | [The board](docs/board.md) |
| Records | `export`, `guide`, `import`, `migrate`, `record`, `seed` | [Records in SQLite](docs/board.md#records-in-sqlite) |
| Human - the maintainer's calls | `ask`, `cred`, `decide`, `goal`, `hooks`, `init`, `proposals`, `remove`, `session` (alias `chat`), `settings`, `start`, `stop`, `voice` | [The console](docs/console.md) |
| Runner | `bulk`, `daemon`, `env`, `executor`, `hands`, `harness`, `job`, `land`, `onboard`, `pair`, `plan`, `review`, `service`, `sweep`, `update`, `work` | [The runner](docs/runner.md), [Releases](docs/release.md), [Delivery](docs/landing.md), [The reviewer](docs/review.md), [Machines](docs/machines.md) |
| Hub | `hub`, `serve` | [The console](docs/console.md) |
| Inspect | `agents`, `events`, `jobs`, `projects`, `schema`, `signers`, `status`, `tally`, `usage`, `verify`, `version` | [The console](docs/console.md), [Development](docs/development.md) |

