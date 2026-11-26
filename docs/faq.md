# Questions people ask

Short answers, each with the page that has the detail. If yours is not
here, [Concepts](concepts.md) explains how the parts fit. To get one thing
done, step by step, open the [guides](how-to/README.md).

## What is Hive?

An open-source harness for coding agents. It runs on your own machine as
one binary. It does the tasks you give it, keeps a project's standing work
going with automations, and improves every project it works on, its own
source included, under rules you write. It pairs more machines when the
work needs them. You give a project's lead agent a task in a
console on your machine. Agents plan it, do it in throwaway clones, test it
and deliver it as pull requests.

## What does Hive need to run?

- macOS or Linux. On Windows, WSL
  ([Does it work on Windows?](#does-it-work-on-windows)).
- A model: a GitHub account with a Copilot seat, an API key for another
  provider, or a local model server.
- To deliver work as pull requests: `git`, and the GitHub CLI (`gh`) signed
  in to an account that can see your repository. Hive opens, reads and
  merges pull requests through `gh`.

Building from source also needs Rust 1.95 or later and Node.js 22 or later.
The [README](../README.md#install) has the install lines.

## Do I need an account? Is there a hosted version?

No, and no. Hive has no accounts and no sign-in of its own, and nobody
hosts it for you. **Sign in with GitHub**, which you reach through
**Settings**, **Harness**, **Connect GitHub Copilot**, is the sign-in of the
model provider, GitHub Copilot. Other providers take a key in the same
wizard, and a local server needs none.

## Which models can it use?

GitHub Copilot by default. By key: OpenAI, Anthropic and Gemini, and ten
OpenAI-compatible vendors: Azure OpenAI, OpenRouter, Groq, Mistral,
DeepSeek, xAI, Together, Fireworks, Cerebras and GitHub Models. Local
servers (Ollama, LM Studio and vLLM) need no key. Any other
OpenAI-compatible endpoint is the `custom` provider.

You can give the runs that change code one model (`worker_model`), the
lighter runs another (`light_model`) and the loop's measurements a third
(`measure_model`). See [Providers, sign-in and models](harness.md#providers-sign-in-and-models).
Step by step: [Choose models and set budgets](how-to/models.md).

## Does my code leave my machine?

Parts of it do, to the services you chose. Each model call carries what the
agent read, so that goes to your model provider. With a local server it
stays on your machine or your network. Finished branches are pushed to your
project's upstream, and pull requests are opened, read and merged with your
`gh` login. Package registries are asked for the release dates of
dependencies. A job's own commands reach whatever they reach: an agent's
shell commands and the project's build and check run with the network open.

Web search is on for sessions by default: a session (a conversation with
one agent) is offered web search and sends the search words to DuckDuckGo.
Jobs do not search unless you set `harness.web_search` to `on`, and `off`
turns it off everywhere.

Off until you turn them on: backups (`backup.to`), the daily check for new
releases (`update.latest_url`), fetching web pages (`harness.fetch_allow`),
rented machines, and store hooks with a URL
(`hive hook add EVENT --url URL`).

Hive has no server of its own in between, and the console answers on this
computer only. [Security notes](security.md#where-hive-runs-and-what-leaves-the-machine)
holds the full list.

## What happens when I give it a task?

A task needs the runner (`hive daemon`): `hive hub` serves the console only,
and until the runner runs a task waits in the queue. Then the project's lead
agent reads the task and cuts it into steps. Each step is one agent's run in
a throwaway clone. The project's test command, when one is set, must pass at
every commit and again at the end. When the steps are done their branches
are merged into one pull request. A review agent checks it out, runs the
check, reads the diff, and merges it, asks for changes or hands it to you.
You watch all of it as one conversation and can type into it at any point.
See [Tasks](console.md#tasks) and [Delivery](landing.md). Step by step:
[Give a task and steer it](how-to/tasks.md) and
[Review and merge the work](how-to/pull-requests.md).

## How is it different from running one coding agent?

One coding agent is one conversation: you ask, it works, you read the diff.
Hive's harness works that way too (`hive harness chat`, or a session in the
console). Around it Hive adds:

- a lead that cuts a task into steps, and several agents that work them in
  separate clones;
- a rule that nothing is delivered until the project's test command passes,
  when one is set, and a review agent that reads every pull request Hive
  opens;
- a board where agents leave findings, questions and votes for each other,
  kept in a store with its history;
- standing work: schedules, checks, watchers and role leads;
- a self-improvement loop under rules you write;
- more machines when one is not enough.

## What does the loop change on its own, and what waits for me?

The loop is Hive's RSI (recursive self-improvement). It improves each
project Hive works on, and Hive runs the same loop on its own source:
agents post suggestions, vote, and build what passes.

Out of the box it does little. The measured experiments are off until you
turn them on (`evolve.loop` is `0`). That setting is the switch: the program
does not read the `loop: off` line in a new project's seeded constitution.
And once two jobs have run in a UTC day (`jobs_per_day`), the runner starts
no more work of its own until midnight. Every job that ran counts, yours
included. Your tasks, the answers to you, the project lead's sittings,
reviews and security updates still start past the cap.

Within those limits, and with the loop on or off, agents look after the
project between tasks, post findings and suggestions, and build the
suggestions that pass the gates. A suggestion must turn its own check from
failing to passing, and every change must pass the project's test command,
when one is set. Everything arrives as a pull request, and the review agent
merges one only when no rule hands it to you.

What waits for you: hard stops, money, Tier 2 decisions such as a major
dependency upgrade, the protected parts of the constitution, and pull
requests that touch the owner's paths, workflows or files named like a
secret. Set `review.merge` to `ask` and every pull request waits for your
merge. One question does not wait for ever: when the loop's fixtures stop
telling a better variant from a worse one it pauses and asks, and with no
answer after `evolve.flat_wait_hours` (24) it answers that one question
itself and runs `evolve.flat_cycles` (6) more cycles. Set the hours to `0`
to make it wait for you.

The kill switch and the caps are yours by rule, not by lock: the program
does not refuse `hive start` or `hive settings set` from an agent's shell
(see [Is it safe to let agents run commands on my machine?](#is-it-safe-to-let-agents-run-commands-on-my-machine)).
See also [Concepts](concepts.md#the-self-improvement-loop). Step by step:
[Turn on the self-improvement loop](how-to/self-improvement-loop.md).

## How do I stop it?

- `hive stop` is the kill switch. The runner starts nothing new and
  leaves the runs in flight to finish; after `minutes_per_job` plus five
  minutes it ends the ones still going. A session you have open in the
  console keeps working. `hive start` lets the runner run again. The rule is that only you
  run `hive start`; the program does not refuse it from an agent's shell,
  and the event log records who ran it.
- **Cancel run** on a run's page, or `hive harness cancel ID`, stops one
  run between steps.
- **Cancel** on a task cancels that task.
- `hive settings set evolve.loop 0` turns the loop's experiments off and
  leaves the rest running.
- `hive service uninstall` removes the user services, so nothing starts
  again by itself.

Step by step: [Set the rules agents work under](how-to/rules.md).

## How do I add a machine?

On the Machines page, **Add a machine**. There are two main ways:

- **Set up a Linux machine over SSH.** `hive pair ADMIN@HOST --name NAME`
  does it from the machine that runs Hive, over the SSH login of an admin
  there, who needs passwordless sudo. It is more than one command on a new
  setup:
  - It copies a Linux `hive` to the machine. A Linux brain with the same
    processor sends its own program. Otherwise it needs one from you: set
    `HIVE_HANDS_BINARY` to a Linux `hive` for that machine's processor (the
    `hive` inside the release's Linux archive), or put that file beside your
    `hive` as `hive-linux-amd64` or `hive-linux-arm64`. From a checkout of
    Hive's source it can cross-build one. With none of these it stops and
    says so.
  - On the machine it creates a `hive` user, installs the program, a work
    directory, one line in that user's `authorized_keys` and an sshd
    drop-in, and reloads sshd. On the brain it adds a `Host` block to
    `~/.ssh/config` and the machine's host keys to `~/.ssh/known_hosts`.
    [Pairing hands](machines.md#pairing-hands) lists every change.
  - A newly paired machine has no model login, so `hive pair` ends with
    `paired NAME: drained (the probe did not pass)`, names the cause
    (`harness login missing`) and prints the next step. The machine takes
    no work until it has a login:
    `hive executor login NAME --admin ADMIN@HOST`, then
    `hive executor probe NAME`.

  `hive pair` refuses anything but Linux.
- **Enrol any other machine, a Mac included, with `hive-hand`.** Each
  release carries it as `hand-<tag>-<target>.tar.gz`. **Run one command on
  it**, then **Get a join code**, shows a six-word code, good for ten
  minutes and one join, and the line to run there:
  `hive-hand join --brain URL CODE`. The URL in that line is the address
  your browser used, so it usually reads `http://127.0.0.1:8765`. The
  console listens on 127.0.0.1 only, so another machine reaches it only
  through something you set up on the brain's machine: an SSH tunnel, or a
  proxy with TLS there. Change the URL to the address that machine can
  reach. `hive-hand join` enrols the machine and then stays connected: it
  does not return by itself. A wrong, used or lapsed code is refused with
  the reason. On every later start run `hive-hand dial`, which is what a
  launchd or systemd service keeps running. To take work the machine also
  needs `hive` on its PATH and a model login of its own
  ([Security notes](security.md#the-machines-you-pair)).

The same page registers a machine you prepared by hand, and asks a cloud
for one once you have set a provider and caps. Then say what a project's
jobs need (`hive init PATH --needs macos,xcode`) and the queue sends each
job to a machine that offers it. See [Machines](machines.md). Step by
step: [Add a second machine](how-to/add-a-machine.md).

## Does it work on Windows?

Yes, inside WSL, the Linux that Windows runs. The install line is the same
there, the console opens in your Windows browser, and the services run
under the Linux's `systemd` ([Run Hive on Windows](how-to/windows.md)).

Not on Windows itself yet. That port compiles, and the `hive.exe` it gives
starts and serves the console, but it does not run work
([Windows port](windows-port.md)).

## Which git hosts does it work with?

Delivery by pull request is built on the GitHub CLI, so it needs the
project's upstream to be on GitHub. A project with no upstream at all still
works: its finished branches land in the local default branch instead.

## What does it cost to run?

Hive itself costs nothing. It is open source under the MIT licence and
there is nothing to subscribe to. What you pay for is what it uses in your
name: the model calls are billed to your own provider account, and a
machine is rented only after you set a cloud provider and caps and approve
the request.

You bound the use with settings: `harness.max_steps` (model calls per run),
`minutes_per_job`, `harness.max_cost` and `jobs_per_day`. The Runs page
shows the tokens each run used, and its cost when the provider reports one.

## How do I update?

`hive update` installs the newest release: it downloads the archive for
your machine, checks it against the release's `SHA256SUMS`, tries the new
binary and keeps the old one as `hive.prev`.

```bash
hive update --check     # is there a newer release?
hive update             # install it
```

Run by hand, both work on a fresh install: while the setting
`update.latest_url` is empty they look at the project's own releases on
GitHub for that one run, and say so. Hive hears of a new release without
being asked only once that setting names where releases are. The daily
check, the console's **Update now** button and `daemon.self_update release`
all wait for it. Set it once:

```bash
hive settings set update.latest_url https://api.github.com/repos/lamplitlabs/hive/releases/latest
```

The services notice the new binary and restart on it by themselves: the
runner once its runs in flight have ended or been put on hold
(`daemon.restart_drain_minutes`, 30), the console once no conversation is at
work. A `hive hub` you started by hand keeps running the old program until
you start it again. If you build Hive from a checkout you registered as the
brain project, the runner rebuilds itself from that checkout instead. See
[Releases and `hive update`](release.md). Step by step:
[Keep Hive running, updated and backed up](how-to/operations.md).

If Homebrew installed your `hive`, upgrade it with
`brew upgrade hivefleet`. Homebrew owns that file, so `hive update` names
the command and installs nothing, and the console shows the command where
the **Update now** button would be. The services restart on the new
version by themselves all the same
([Homebrew](release.md#homebrew)).

## I set Hive up when it was called PULSE. Does it still work?

Yes. Hive reads what that setup left, where it is.

- **The home.** Hive uses `~/.pulse` while the install still lives there:
  when there is no `~/.hive`, and when `~/.hive` holds only the program and
  the store is in `~/.pulse`. `PULSE_HOME` stands in for `HIVE_HOME`. The
  store file `pulse.sqlite` is renamed to `hive.sqlite` the first time it
  is opened, and `hive service install` replaces the `pulse.daemon` and
  `pulse.hub` services.
- **A project's folder.** `.pulse/automations`, `.pulse/pulse.yaml`,
  `.pulse/skills`, `.pulse/agents`, `.pulse/rules` and `.pulse/hooks.json`
  are read like their `.hive/` twins. One repository may hold both folders
  ([the rules](board.md#a-repository-set-up-before-the-rename-pulse)).
- **Scripts and hooks.** The variables a script or a hook is given are
  there again under their `PULSE_...` names.
- **Open pull requests.** One that Hive opened then has its branch under
  `pulse/`. It is still Hive's own: the reviewer reads it like the rest,
  and the owner's paths stay closed to it.

To move the home, stop the services, run `mv ~/.pulse ~/.hive`, then
`hive service install`. Other programs may still name the old path, so a
link from the old name is worth keeping: `ln -s ~/.hive ~/.pulse`.

## Is it safe to let agents run commands on my machine?

An agent runs as your user, in a throwaway clone, with no container and no
operating-system sandbox around it. So treat a repository you point Hive at
like code you would run yourself. What holds an agent is mostly a set of
rules, and the program checks only part of them:

- **Deny rules.** Before a shell command of a job runs, the harness
  matches its text against four patterns: a command that starts with
  `git push`, with `git remote add`, `set-url` or `remove`, or with `gh`,
  and a command that names `~/.ssh/`, `.netrc`, `~/.copilot`,
  `~/.config/gh` or the hive home's `private/` directory. A match is
  refused without being run. Other spellings pass (`git -C . push`, `gh`
  by its full path, `$HOME/.ssh/...`). The rules catch a mistake; they are
  not a barrier.
- **The kill switch and the settings.** The rule is that only you run
  `hive start` and only you change the caps. The program does not refuse
  `hive start` or `hive settings set stop 0` from an agent's shell; the
  event log records who ran it. Of the settings, a `hive` command run
  under an agent's name is refused only the `owner.*` ones and
  `push.credential`, and a
  value an agent wrote for `hub.allow_remote`, `hub.hosts` or `owner.presence` is
  ignored.
- **Votes and variants.** These are held to the rule in code: no vote can
  touch the constitution's protected sections, and no variant can change
  `evolve.*`.
- **Commits and review.** A run signs its commits with a key of its own.
  On a project whose upstream is on GitHub, its work is also reviewed as a
  pull request before it merges.

[Security notes](security.md#agents) says what holds and what is not built.

## Can I run it on a server instead of my laptop?

Yes, as something you run yourself. `deploy/` holds a Dockerfile and
scripts for a brain in a container, with the console still on loopback and
reached over an SSH tunnel. It runs as a brain without the code
(`brain.code none`), and that option is not finished:
[Security notes](security.md#what-is-not-built) lists what such a brain
cannot do yet.

## Where is my data, and how do I remove it?

In the hive home, `~/.hive` by default. The store is one SQLite file there,
`hive.sqlite`; the first `hive hub` on a machine makes it. Once a month
rows older than `daemon.archive_days` (90) that nothing reads back move to
`hive-archive.sqlite` beside it. The logs, the model logins and the work
directories are in the same folder. `backup.to` names where a daily copy
of the store goes. `hive remove project <slug>` deactivates a project, and
`--forget` deletes the rows of one that is already inactive and whose
checkout is gone. See [Removing your data](security.md#removing-your-data).
[Keep Hive running, updated and backed up](how-to/operations.md) goes
through removing Hive in order.

## Something is stuck. Where do I look?

**Sessions**, where the console opens, shows the decisions only you can
make, and each project's **For you** tab shows everything that waits for
your answer there. Sessions also shows a notice while no runner
runs, with the button **Start runner**: until the runner runs, a task waits
in the queue. `hive status` shows the runner, the queue and what holds each
project's work. For failed jobs, a project's Overview links **Jobs that
failed** to the Runs page, where they can be retried, and
`hive jobs --all` lists them in a terminal
with what each one ended on. A waiting pull request says why on its task's
delivery line. See [Recovery and hygiene](recovery.md), and
[Troubleshooting](how-to/troubleshooting.md) for each symptom and its fix.

## How do I contribute, or report a problem?

[CONTRIBUTING.md](../CONTRIBUTING.md) says how to build Hive, check a
change and open a pull request, and how to report a bug or propose a
feature. A vulnerability goes through [SECURITY.md](../SECURITY.md), not
through a public issue.
