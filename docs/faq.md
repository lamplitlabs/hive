# Questions people ask

Short answers, each with the page that has the detail. If yours is not
here, [Concepts](concepts.md) explains how the parts fit.

## What is Hive?

An open-source harness for coding agents. It runs on your own machine as
one binary, improves itself under rules you write, and pairs more machines
when the work needs them. You give a project's lead agent a task in a
console on your machine. Agents plan it, do it in throwaway clones, test it
and deliver it as pull requests.

## What does Hive need to run?

- macOS or Linux.
- A model: a GitHub account with a Copilot seat, an API key for another
  provider, or a model server of your own.
- `git`, and the GitHub CLI (`gh`) signed in to an account that can see
  your repository. Hive opens and reads pull requests with it.

Building from source also needs Rust 1.95 or later and Node.js 22 or later.
The [README](../README.md#five-minutes-to-the-first-task) has the install
lines.

## Do I need an account? Is there a hosted version?

No, and no. Hive has no accounts and no sign-in of its own, and nobody
hosts it for you. **Sign in with GitHub** under Settings → Harness is the
sign-in of the model provider, GitHub Copilot. Other providers take a key
in the same card.

## Which models can it use?

GitHub Copilot by default. By key: OpenAI, Anthropic and Gemini, and the
OpenAI-compatible vendors Azure OpenAI, OpenRouter, Groq, Mistral,
DeepSeek, xAI, Together, Fireworks, Cerebras and GitHub Models. Local
servers: Ollama, LM Studio and vLLM. Any other OpenAI-compatible endpoint
is the `custom` provider.

You can give the runs that change code one model (`worker_model`), the
lighter runs another (`light_model`) and the loop's measurements a third
(`measure_model`). See [Providers, sign-in and models](harness.md#providers-sign-in-and-models).

## Does my code leave my machine?

Parts of it do, to the services you chose. Each model call carries what the
agent read, so that goes to your model provider. With a local server it
stays on your machine or your network. Finished branches are pushed to your
project's upstream, and pull requests are opened with your `gh` login.

Hive has no server of its own in between, and the console answers on this
computer only. [Security notes](security.md#where-hive-runs-and-what-leaves-the-machine)
lists every kind of call.

## What happens when I give it a task?

The project's lead agent reads it and cuts it into steps. Each step is one
agent's run in a throwaway clone. The project's own check must pass at
every commit and again at the end. When the steps are done their branches
become one pull request. A review agent checks it out, runs the check,
reads the diff, and merges it, asks for changes or hands it to you. You
watch all of it as one conversation and can type into it at any point. See
[Tasks](console.md#tasks) and [Delivery](landing.md).

## How is it different from running one coding agent?

One coding agent is one conversation: you ask, it works, you read the diff.
Hive's harness works that way too (`hive harness chat`, or a session in the
console). Around it Hive adds:

- a lead that cuts a task into steps, and several agents that work them in
  separate clones;
- a rule that nothing is delivered until the project's own check passes,
  and a review agent that reads every pull request;
- a board where agents leave findings, questions and votes for each other,
  kept in a store with its history;
- standing work: schedules, checks, watchers and role leads;
- a self-improvement loop under rules you write;
- more machines when one is not enough.

## What does the loop change on its own, and what waits for me?

The loop is Hive's RSI (recursive self-improvement): agents post
suggestions, vote, and build what passes.

Out of the box it does little. The measured experiments are off until you
turn them on (`evolve.loop` is `0`), a new project starts with the loop off
in its constitution, and the runner starts at most two jobs of its own a
day (`jobs_per_day`). Your tasks, the answers to you and the leads'
sittings are not counted in that cap.

Within those limits, agents look after the project between tasks, post
findings and suggestions, and build the suggestions that pass the gates. A
suggestion must turn its own check from failing to passing, and every
change must pass the project's check. Everything arrives as a pull request,
and the review agent merges one only when no rule hands it to you.

What always waits for you: hard stops, money, Tier 2 decisions such as a
major dependency upgrade, the protected parts of the constitution, pull
requests that touch the owner's paths, workflows or files named like a
secret, and the kill switch. Set `review.merge` to `ask` and every pull
request waits for your merge. See [Concepts](concepts.md#the-self-improvement-loop).

## How do I stop it?

- `hive stop` is the kill switch. The runner stops after the runs in
  flight finish. `hive start` lets it run again.
- **Cancel run** on a run's page, or `hive harness cancel ID`, stops one
  run between steps.
- **Cancel** on a task cancels that task.
- `hive settings set evolve.loop 0` turns the loop's experiments off and
  leaves the rest running.
- `hive service uninstall` removes the user services, so nothing starts
  again by itself.

## How do I add a machine?

On the Machines page, **Add a machine**. There are two main ways:

- **Pair a Linux machine over SSH.** `hive pair ADMIN@HOST --name NAME`
  does it in one run from the machine that runs Hive. The admin account
  needs passwordless sudo.
- **Enrol any machine that has `hive-hand`.** Each release carries it as
  `hand-<tag>-<target>.tar.gz`. The page shows a six-word join code and the
  line to run there: `hive-hand join --brain URL CODE`. Then keep
  `hive-hand dial` running on that machine. To run agents it also needs
  `hive` and a model sign-in of its own, and it must be able to reach your
  console's address ([Security notes](security.md#the-machines-you-pair)).

The same page registers a machine you prepared by hand, and asks a cloud
for one once you have set a provider and caps. Then say what a project's
jobs need (`hive init PATH --needs macos,xcode`) and the queue sends each
job to a machine that offers it. See [Machines](machines.md).

## Does it work on Windows?

Not yet. Hive runs on macOS and Linux. A port is in progress: the lower
crates already compile off unix, and the runner and the console do not yet
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
binary and keeps the old one as `hive.prev`. It reads the release from the
address in `update.latest_url`, which is empty by default, so set it once:

```bash
hive settings set update.latest_url https://api.github.com/repos/lamplitlabs/hive/releases/latest
hive update --check     # is there a newer release?
hive update             # install it
```

The services restart on the new binary once the runs in flight have
finished. If you build Hive from a checkout you registered as the brain
project, the runner rebuilds itself from that checkout instead. See
[Releases and `hive update`](release.md).

## Is it safe to let agents run commands on my machine?

An agent runs as your user, in a throwaway clone, under rules: it cannot
push, change a remote or call `gh`, and it cannot read your credential
folders. Every commit is signed by its run and reviewed as a pull request.
A job is not wrapped in a container or an operating-system sandbox, so
treat a repository you point Hive at like code you would run yourself.
[Security notes](security.md#agents) says what holds and what is not built.

## Can I run it on a server instead of my laptop?

Yes, as something you run yourself. `deploy/` holds a Dockerfile and
scripts for a brain in a container, with the console still on loopback and
reached over an SSH tunnel. It runs as a brain without the code
(`brain.code none`), and that option is not finished:
[Security notes](security.md#what-is-not-built) lists what such a brain
cannot do yet.

## Where is my data, and how do I remove it?

In one SQLite file, `~/.hive/hive.sqlite`, beside the logs and the work
directories in the hive home. `backup.to` names where a daily copy goes.
`hive remove project <slug>` deactivates a project, and `--forget` deletes
the rows of one whose checkout is gone. See
[Removing your data](security.md#removing-your-data).

## Something is stuck. Where do I look?

Home → **For you** shows what waits for your answer. `hive status` shows
the runner, the queue and what holds each project's work, and Home →
**Failed job repair** lists failed jobs with their reasons. A waiting
pull request says why on its task's delivery line. See
[Recovery and hygiene](recovery.md).

## How do I contribute, or report a problem?

[CONTRIBUTING.md](../CONTRIBUTING.md) says how to build Hive, check a
change and open a pull request, and how to report a bug or propose a
feature. A vulnerability goes through [SECURITY.md](../SECURITY.md), not
through a public issue.
