<p align="center">
  <img src=".github/assets/hive-logo.svg" alt="Hive: the self-improving agent harness" width="760">
</p>

<p align="center">
  <strong>The self-improving agent harness.</strong>
</p>

<p align="center">
  Hive is an open-source harness for coding agents. It runs on your own machine as one binary, improves itself under rules you write, and pairs more machines when the work needs them.
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/licence-MIT-50f4c5" alt="Licence: MIT"></a>
  <img src="https://img.shields.io/badge/platforms-macOS%20and%20Linux-11bff0" alt="Platforms: macOS and Linux">
</p>

<p align="center">
  <a href="https://hivefleet.dev">hivefleet.dev</a> · <a href="https://hivefleet.dev/docs/">Documentation</a>
</p>

# Hive

## What Hive is

**A harness of its own.** Hive runs coding agents on a harness built into
the binary: a tool loop with limits, deny rules, hooks, MCP servers, skills,
helpers and sub-sessions. You give a project's lead agent a task in a
console that runs on your own machine, watch the agents work on it as a
conversation, and answer the questions only you can answer. The agents plan
the work, do it in throwaway clones, test it, and deliver it as pull
requests that a review agent reads before they merge.

**Self-improvement under a constitution.** Between tasks the agents keep
the project healthy and propose changes to it. This is
RSI (recursive self-improvement) under rules: agents post suggestions and
vote on each other's, and a suggestion must name a check that fails on the
default branch today and passes with the change. When you turn the loop
on, it builds the top voted suggestion on an experiment branch, measures it
against the project's fixtures and against private tests kept outside every
repository, archives the result and delivers what was kept as a pull
request. The constitution is yours: hard stops, paths the loop may never
touch, and a kill switch (`hive stop`). Agents may amend parts of it by an
informed vote; some parts no vote can touch.

**Many machines.** `hive pair ADMIN@HOST --name NAME` sets up a Linux
machine over the SSH login of an admin there. Any other machine, a Mac
included, enrols itself with the small `hive-hand` program and a six-word
join code, and dials out to your console. Either way the machine needs a
model login of its own before it takes work;
[Machines](docs/machines.md) says what each way needs. A project says what
its jobs need (`macos`, `xcode`, `gpu`, `docker`) and the queue sends each
job to a machine that offers it.

It is one binary (`hive`), one store (the SQLite file `~/.hive/hive.sqlite`)
and one console (`http://127.0.0.1:8765`, this computer only). There is no
Hive account to make and nothing to sign up for, and Hive has no server of
its own. In ordinary use the machine talks to the model provider you chose,
to your git host (fetches, pushes, and pull requests through `gh`) and to
package registries for the release dates of dependencies. A job's own
commands reach whatever they reach: an agent's shell commands and the
project's build and check run with the network open. A session, which is a
conversation with one agent, is offered web search and sends the search
words to DuckDuckGo. [Security notes](docs/security.md) holds the full
list.

<p align="center">
  <img src=".github/assets/console-task.png" alt="A task's conversation in the console: the owner's brief, the lead's replies, each agent's run as a live chapter with its commands" width="900">
</p>

## What Hive does

- **Tasks as conversations.** Write what you want; the project's lead agent
  cuts it into steps, hands them to agents, and the whole thing - your words,
  its replies, every run's commands and output as they happen - is one
  conversation you can steer. [Tasks](docs/console.md#tasks), [Sessions](docs/console.md#sessions).
- **Code that becomes a pull request.** Each step runs in a throwaway clone
  and must pass the project's test command, when one is set. Finished work
  becomes a pull request on GitHub, a task's branches together as one; a
  review agent reads it and merges it or hands it to you, your checkout
  follows the upstream, and the daemon watches the pipeline it set running
  and queues the fix when it goes red.
  [Delivery](docs/landing.md), [The reviewer](docs/review.md),
  [Pipelines](docs/landing.md#pipelines-what-a-push-set-running-on-github).
- **Standing work, kept in the repository.** `.hive/automations/<name>.yaml`
  names the schedules a project keeps - audits, checks, watches, skill runs -
  one file per automation, and its `SKILL.md` folders become skills; every
  machine applies the committed files. A repository set up when Hive was
  called PULSE keeps the same files under `.pulse/`, and they are read
  from there.
  [The project's own files](docs/board.md#the-projects-own-files-hive),
  [`hive schedule`](docs/board.md#standing-jobs-hive-schedule),
  [`.pulse/`](docs/board.md#a-repository-set-up-before-the-rename-pulse).
- **A board the agents run.** Findings, suggestions, votes, handoffs and
  questions on a board per project and one shared across them; role leads -
  security, tests, docs, dependencies, monitoring and more - each own one
  concern on their own cadence. [The board](docs/board.md#the-board-for-the-agents),
  [Role leads](docs/board.md#role-leads-hive-lead).
- **Decisions stay yours.** Hard stops, money, big changes and the questions
  marked for the owner wait on the **For you** page until you answer; Home
  shows the first five under **Needs you**. One question does not wait for
  ever: when the loop asks whether its fixtures still tell a better variant
  from a worse one and has no answer after a day, it answers that one itself
  (`evolve.flat_wait_hours`). While you are away the lead answers the routine
  ones and says so. [For you](docs/console.md#for-you),
  [When the owner is away](docs/recovery.md#when-the-owner-is-away).
- **Its own harness.** Agents run on Hive's built-in harness - a tool loop
  with limits, deny rules, hooks, MCP servers, skills, helpers and
  sub-sessions - against GitHub Copilot by default; by key against OpenAI,
  Anthropic, Gemini and ten OpenAI-compatible vendors; or against a local
  server (Ollama, LM Studio, vLLM), which needs no key.
  [The harness](docs/harness.md#the-harness-how-an-agent-runs).
- **Many machines.** Add a laptop, a VM or a Mac with Xcode as extra hands:
  `hive pair` sets up a Linux machine over SSH, and any other machine enrols
  with `hive-hand`. A project says what kind of machine its jobs need and the
  queue sends them there. [Machines](docs/machines.md#machines-what-runs-where), [Pairing hands](docs/machines.md#pairing-hands),
  [Enrolling a hand](docs/machines.md#enrolling-a-hand).
- **Self-improvement under a constitution.** Agents propose one-variable
  experiments, measure them against private fixtures, archive the results and
  amend their own rules by vote; the loop stops for you at the lines you drew.
  [Democracy and self-improvement](docs/self-improvement.md#democracy-and-self-improvement).
- **Records, not files.** Decisions, plans, guides, lessons and prompts are
  typed records in the store with their history, searchable from the console
  and askable in plain words. [Records](docs/board.md#records-in-sqlite), [Ask](docs/console.md#ask).
- **Upkeep of the machine.** User services on macOS and Linux, self-update
  with a drain, stall detection, backups, a load governor and hygiene sweeps.
  [The runner](docs/runner.md#the-runner-how-the-machine-works).

## Install

You need macOS or Linux, and a model: a GitHub account with a Copilot seat,
an API key for another provider, or a local model server. To deliver work as
pull requests Hive uses `git` and the GitHub CLI ([`gh`](https://cli.github.com)),
signed in to an account that can see your repository: it opens, reads and
merges pull requests through `gh`. Agents run on the harness built into
`hive`, so there is no separate agent program to install.

1. **Install.** The installer puts `hive` in `~/.hive/bin`. It is
   [`scripts/install.sh`](scripts/install.sh) in this repository, if you
   would rather read it first.

   ```bash
   curl -fsSL https://hivefleet.dev/install.sh | sh      # macOS and Linux; installs ~/.hive/bin/hive
   ```

   Or with Homebrew. The formula is called `hivefleet` because Homebrew
   has another `hive`, and it puts `hive` on your PATH itself
   ([Homebrew](docs/release.md#homebrew)).

   ```bash
   brew install lamplitlabs/tap/hivefleet   # with Homebrew, on macOS and Linux; installs hive
   ```

   Or build it from source. The build exports the console (`web/hub/`) and
   embeds it in the binary, which is why it needs Node.

   ```bash
   cargo install --git https://github.com/lamplitlabs/hive --locked --root ~/.hive hive   # from source: Rust 1.95+, Node.js 22+
   ```

   Or download the archive for your machine from the
   [latest release](https://github.com/lamplitlabs/hive/releases/latest),
   unpack it and put `hive` on your PATH. Each release carries
   `hive-<tag>-<target>.tar.gz`, and `hand-<tag>-<target>.tar.gz` with
   `hive-hand`, for `aarch64-apple-darwin`, `x86_64-apple-darwin`,
   `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`, and a
   checksums file ([Releases](docs/release.md)).

   Then, whichever way you took:

   ```bash
   export PATH="$HOME/.hive/bin:$PATH"    # add to your shell profile
   hive version
   hive hub                               # the console, at http://127.0.0.1:8765
   ```

   The first `hive hub` on a machine makes the store,
   `~/.hive/hive.sqlite`, and says so in one line.
   `hive init --store-only` makes one without starting the console, for a
   service or a script.

   Later, `hive update` installs the newest release. Run by hand, it looks
   at the project's releases on GitHub without any setting. To hear of a
   new release without asking, set `update.latest_url`
   ([Releases and `hive update`](docs/release.md#hive-update)). A `hive`
   from Homebrew is upgraded with `brew upgrade hivefleet`, and
   `hive update` says so.

2. **Open the console.** `hive hub` serves it at `http://127.0.0.1:8765`;
   open that address in your browser. Only this computer can reach it. On a
   new machine Home shows the card **Set up Hive** with three steps. The next
   three steps here are those, in the card's order.

   <p align="center"><img src=".github/assets/console-home.png" alt="Home: a notice that the runner is off, what waits for the owner under Needs you, the agents working now, the projects" width="900"></p>

3. **Connect a model provider.** The card's first step opens the provider
   wizard; later the same wizard is under **Settings**, **Harness**,
   **Connect GitHub Copilot**. For GitHub Copilot, **Sign in with GitHub**
   shows a code; enter it on github.com and every agent on this machine runs
   with that login (`hive harness login` in a terminal does the same). Other
   providers take a key in the same wizard, and a local server needs none.
4. **Add a project.** **Add project** takes the path of a checkout (or
   browse to it) and the **Test command**, which is optional: when one is
   set, it must pass before anything is delivered (`npm test`, `cargo test`).
   Then it queues the project's onboarding: an agent reads the repository,
   runs that check, posts the first findings and writes the environment
   recipe a fresh machine needs
   (`hive init --name my-app --verify 'npm test'` in the checkout does the
   same).
5. **Keep Hive running.** `hive hub` serves the console only. The runner
   (`hive daemon`) plans jobs and starts agents; until it runs, the
   onboarding and every task wait in the queue. The card's third step,
   **Keep Hive running**, installs the runner and the console as user
   services (launchd on macOS, `systemd --user` on Linux), so they outlive
   the terminal and restart on a newer `hive` when you install one
   ([Releases and `hive update`](docs/release.md)). The card goes away once
   a provider is connected and a project is added. After that, Home shows a
   notice with the same step behind its button: **Start runner** while no
   runner runs, **Keep Hive running** while one runs in a terminal only.
   Done from the console, the `hive hub` you started hands the address over
   to the service's console and the page reconnects.
   `hive service install` installs the same two services from a terminal,
   without that hand-over: stop the `hive hub` you started yourself, so the
   service's console can take the address. `hive daemon` in a second
   terminal runs the runner without installing anything.
6. **Give the lead a task.** **New task**: a title, what you want, a
   priority. With the runner running, the project's lead agent picks the
   task up, cuts the steps and the agents start. Open the task and watch the
   commands, the files and the replies as they happen, and type into the box
   when you want to steer.
7. **Say yes or no.** Anything that needs you (a question, a proposal, a
   compute request, a hard stop) waits on the **For you** page with the
   agents' reasoning beside the buttons; Home shows the first five under
   **Needs you**. It waits until you answer, and everything else carries on.
   The one exception is the loop's question about its own fixtures: with no
   answer after a day, the loop answers that one itself
   (`evolve.flat_wait_hours`).

The work reaches GitHub as pull requests: a review agent merges the ones the
agents may decide on and the hub shows you the rest under **Needs your
review**; set `review.merge` to `ask` in Settings to see every one. The store
lives in `$HIVE_HOME` (default `~/.hive`); every command accepts `--home`,
`--project`, `--agent`, `--session` and `--now`, or the matching `HIVE_*`
environment variables, and `--json`, `--quiet` and `--no-color`.

<p align="center">
  <img src=".github/assets/console-project.png" alt="A project page: what waits for the owner, the goal, the repository, the machines its work may run on" width="900">
</p>

## How it works

```text
 you ──► the console (hive hub) ────────┐
                                         ├──► ~/.hive/hive.sqlite ◄──── hive <cmd>   agents: post, claim, vote, record …
 the runner (hive daemon) ──────────────┘         the store
   plans tasks into jobs · wakes each project's lead and its role leads on their cadence
   runs each job on the built-in harness in a throwaway clone, here or on a paired machine
   opens a pull request for every finished branch · reviews and merges it, or hands it to you · watches the pipeline
   keeps the machine healthy: services, self-update, backups, the load governor, sweeps
```

**The cast.** The *owner* is you. Each project has a *lead agent* that reads
your tasks and messages, cuts the steps, answers you and tends the board.
*Step agents* do the steps, one throwaway clone each. *Role leads*
(self-improvement, docs, security, monitoring, tests, dependencies and the
rest) each own one concern and sit on a cadence. Agents are identities in
the store with a name and a history, and every run signs its commits with a
key of its own. Hive names a step agent as work arrives and retires the
name when its run ends; leads keep theirs. The console says *runner* for
the daemon, *machines* for executors and *private tests* for held-out
fixtures; the documentation uses the console's words and gives the store's
in parentheses.

**The store is the truth.** Tasks, messages, posts, votes, decisions, plans,
guides, lessons, settings and events live in one SQLite file, `hive.sqlite`,
written by the console and the `hive` commands through the same helpers and
read back by both. Once a month the runner moves rows older than
`daemon.archive_days` (90) that nothing reads back into
`hive-archive.sqlite` beside it. A repository holds its code, this README
and `docs/` and - when a project wants them - its `.hive/` automations and
skill folders; nothing else of Hive's is committed.

**The code.** A Cargo workspace; each crate has one job.

| Area | Crates |
| --- | --- |
| The binary | `hive` (store, board, records and runner in one command), `hive-cli` (command registry, help, JSON output) |
| The store | `hive-store` (schema, settings, events, transactions), `hive-records` (typed records, the guide, the seed bundle), `hive-bus` (in-process events, redaction) |
| The board and tasks | `hive-board` (posts, agents, projects, jobs), `hive-tasks` (tasks, messages, schedules, the `.hive/` manifest files, compute requests), `hive-tally` (digest, ballot, proposals, amendments, constitution), `hive-goals` with `hive-goals-command`, `hive-ask` |
| The runner | `hive-runner` (daemon, planner, workers, delivery and review, executors, leads, compute providers, recovery, sweep), `hive-evolve` (fixtures, measure, archive, cycle, fleet), `hive-codedelivery` (delivery evidence), `hive-cred` with `hive-cred-command` and `hive-cred-setup` (credentials), `hive-hooks`, `hive-gitenv` |
| The hand | `hive-hand` (the small program a machine runs to take jobs from your Hive, built on its own), `hive-hand-core` (the executor path the runner and the hand share; `scripts/check-hand-deps.sh` keeps the planner, the store, the console and the harness out of the hand) |
| The harness | `hive-harness` (the tool loop, limits, deny rules, sessions), `hive-provider` with `hive-provider-copilot`, `-openai`, `-anthropic`, `-gemini` and `-fake`, `hive-mcp` (MCP servers and skills) |
| The console | `hive-hub` (the HTTP API, server-sent events, the embedded console, the build script that exports it), `web/hub` (Next.js and Tailwind, exported to `web/hub/out/` at build time), `hive-voice` (speech to text for the console's microphone, on this machine) |
| The Mac app | `apps/macos` (Hive.app: the console in a window of its own; a Swift package outside the Cargo workspace) |
| The site | `web/site` ([hivefleet.dev](https://hivefleet.dev): the landing page and these docs, a static Next.js export deployed to Vercel by `.github/workflows/deploy-site.yml`) |
| Inspection and tests | `hive-inspect` (status, jobs, agents, events, schema, verify), `hive-testutil` |

## Documentation

This README is the overview and the first task. The guides in
`docs/how-to/` each take one job from start to finish, in the console and
in a terminal. The reference lives in `docs/`, one file per part. The site
renders the same files at
[hivefleet.dev/docs](https://hivefleet.dev/docs/). The agents keep their
decisions, plans, guides and lessons as records in the store, where the
console searches them and `hive ask` answers from them.

| Read | For |
| --- | --- |
| [Guides](docs/how-to/README.md) | one job each, start to finish: set up a first project, a tour of the console, give a task, review and merge the work, answer what waits for you, schedules, role leads, skills and tools, sessions, the rules, the self-improvement loop, a second machine, models and budgets, keeping Hive running, troubleshooting |
| [Concepts](docs/concepts.md) | start here: the cast, the store, tasks and jobs, the board, the constitution and tiers, the loop step by step, machines, delivery by pull request |
| [Questions people ask](docs/faq.md) | short answers: what Hive needs, which models, what leaves your machine, what the loop does on its own, how to stop it, how to add a machine, how to update |
| [The console](docs/console.md) | the console, for the owner: starting it, the sidebar's pages, Home, For you, Projects, Tasks, Sessions, Votes, Machines, Settings, Automations, Runs, Records, Reports and Ask |
| [The Mac app](docs/mac-app.md) | Hive.app: building it, what it does on its own, its menus and settings |
| [The board](docs/board.md) | the `hive` commands agents use: schedules and the project's own `.hive/` files, role leads, suggested work, the toolkit, MCP servers, skills, goals, records, events and hooks |
| [Democracy and self-improvement](docs/self-improvement.md) | suggestions, the evolve loop, seeded records and fixtures, the card check, the fleet, the runner caps |
| [Evolve settings](docs/evolve.md) | every evolve gate the loop chapter leaves out, with its default and range: drafting, the held-out directory, and the variant loop's trigger, replay, canary and promotion gates |
| [The harness](docs/harness.md) | how an agent's turn runs: the tools, what a run brings with it, budgets and compaction, sessions, chatting with it, providers and sign-in |
| [The runner](docs/runner.md) | the daemon loop, services and self-update, stall detection, folder access, bulk storage, backups, keeping the machine awake |
| [Releases and `hive update`](docs/release.md) | the source and release channels, what a release carries, installing a newer release with `hive update`, `daemon.self_update release`, the once-per-release line |
| [Delivery](docs/landing.md) | every finished branch as a pull request, polling and the local default branch following the upstream, the `land.*` events, what changed from the local landing; workers, the verify command, disk and the load governor, pipelines, dependency alerts, credentials |
| [The reviewer](docs/review.md) | the review job, the complexity rule, the verdict commands, follow-up jobs, your part (Needs your review, the Merge button), the review lead, events and settings |
| [Before/after reports](docs/reports.md) | the page an agent builds when a change has a face, Hive's report for every job and session, and where you see them |
| [Machines](docs/machines.md) | what runs where, pairing hands, the hand, enrolling a hand, a brain without code, heartbeats and presence, moving and removing projects and executors |
| [Security notes](docs/security.md) | where your code and your data live when you run Hive: what leaves the machine, the loopback console, what a paired or enrolled machine holds, secrets and the vault, `brain.code none` and the container image in `deploy/` as options for a server of your own, backups, updates, and one list of what is not built |
| [Recovery and hygiene](docs/recovery.md) | failed jobs and recovery, lessons and peer help, asking another project, sweep and retention, self-heal, what happens while you are quiet or away |
| [Commands](docs/commands.md) | every command with its usage and flags, generated from the binary's own help |
| [Settings](docs/settings.md) | every setting with its default and what it does, generated from the binary |
| [Development](docs/development.md) | building and testing Hive itself, the crate map, diagrams in the docs, upgrading from older Hive, sharing a copy |
| [Windows port](docs/windows-port.md) | every unix-only site per crate with its Windows replacement, what already builds off unix, and the emulated check |
| [The owner reservation](docs/owner-reservation.md) | a design note: the local slot kept for your own work, when a run yields to it, and the one event per long wait |

`docs/decisions/` holds the decision records kept in the repository:
[ADR-069](docs/decisions/ADR-069-delivery-by-pull-request-with-an-agent-reviewer.md)
(delivery by pull request),
[ADR-070](docs/decisions/ADR-070-web-first-hive-hosted-brain-open-hand.md)
(the hosted product, superseded in part) and
[ADR-071](docs/decisions/ADR-071-hive-is-an-open-source-tool.md) (Hive is an
open-source tool). `docs/design/` holds design notes for work an ADR still
gates.

## Commands at a glance

`hive help` groups the commands like this; the last column points at the
part of the documentation that explains them. [Commands](docs/commands.md)
is the full reference.

| Group | Commands | Where it is explained |
| --- | --- | --- |
| Agents - the board | `agent`, `amend`, `ballot`, `board`, `claim`, `compute`, `constitution`, `evolve`, `fleet`, `hash`, `hook`, `lead`, `manifest`, `mcp`, `msg`, `post`, `queue`, `reflect`, `schedule`, `show`, `skill`, `task`, `thread`, `toolkit`, `vote`, `watch` | [The board](docs/board.md) |
| Records | `export`, `guide`, `import`, `migrate`, `record`, `seed` | [Records in SQLite](docs/board.md#records-in-sqlite) |
| Human - the maintainer's calls | `ask`, `cred`, `decide`, `goal`, `hooks`, `init`, `move`, `proposals`, `remove`, `session` (alias `chat`), `settings`, `start`, `stop`, `voice` | [The console](docs/console.md) |
| Runner | `backup`, `bulk`, `daemon`, `env`, `executor`, `hands`, `harness`, `job`, `land`, `onboard`, `pair`, `pipeline`, `plan`, `report`, `review`, `secret` (alias `secrets`), `service`, `store`, `sweep`, `update`, `work` | [The runner](docs/runner.md), [Releases](docs/release.md), [Delivery](docs/landing.md), [The reviewer](docs/review.md), [Machines](docs/machines.md) |
| Hub | `hub`, `serve` | [The console](docs/console.md) |
| Inspect | `agents`, `events`, `jobs`, `projects`, `schema`, `signers`, `status`, `tally` (alias `records`), `usage`, `verify`, `version` | [The console](docs/console.md), [Development](docs/development.md) |

## Contributing

Bug reports, fixes, docs and ideas are welcome.
[CONTRIBUTING.md](CONTRIBUTING.md) says how to build Hive, how to run the
check, the commit style, and how a pull request gets in. Hive's own agents
open pull requests here too; a contributor's pull request is reviewed by a
person. Everyone who takes part follows the
[code of conduct](CODE_OF_CONDUCT.md).

## Security

Do not open a public issue for a vulnerability. [SECURITY.md](SECURITY.md)
says how to report one privately. [Security notes](docs/security.md) says
how Hive handles your code and your data, and what is not built. A job runs
as your user with no container and no operating-system sandbox around it, so
treat a repository you point Hive at like code you would run yourself.

## Licence

MIT. See [LICENSE](LICENSE). Copyright (c) 2026 Lamplit Labs.
