# Set the rules agents work under

Read your project's constitution, learn which rules Hive enforces by code,
answer a proposal, change a rule and stop the runner.

**Before you start**

- Hive is installed and the console opens ([Install](../../README.md#install)).
- A project is registered ([Set up your first project](first-project.md)).
  The examples use `my-app`.

## 1. Read the constitution

Each project has a constitution: a record in the store (the file
`~/.hive/hive.sqlite`) that says what the self-improvement loop may and may
not do. The loop is the agents' own work to improve the project between
your tasks. The record is yours. Print it:

```bash
hive constitution --project my-app
```

The text is long. Its sections are `Subject`, `Hard stops`, `Immutable
paths`, `Editable scope by stage`, `Budget`, `Sandbox`, `Branches and
worktrees`, `Vote rules`, `Amendments`, `Promotion` and `Kill switch`. It is
written as rules for agents: step 3 says which of them the program enforces.

In the console, open the project from **Projects** in the sidebar and choose the
**Self-improvement** tab. The panel **The rules (constitution)** shows the
same text, and **Open the record** leads to its history on **Records**.

Hive wrote the text when you registered the project, as a copy of its
template. `hive record revisions EVO-constitution-my-app` shows it:

```text
REV  AUTHOR  AT                    NOTE
1    human   2026-11-23T04:08:27Z  seeded from TPL-evolution-constitution@r1 (ADR-030)
```

The copy still holds placeholders in angle brackets. An agent drafts your
project's own text from the checkout, and **Draft it for me** on the same
panel asks for one now. A draft changes nothing: it waits on **For you**
under **Drafts** until you press **Read & adopt** or **Discard**.

## 2. Learn the tiers

Agents sort every change into a tier before they make it. `hive guide`
prints their operating instructions, and this is the rule in them:

| Tier | What it is | What it needs |
| --- | --- | --- |
| 0 | polish and repair: a bug with a reproducing test, a guide fix | the relevant checks |
| 1 | behaviour inside a decision you already accepted | a work-unit record that names the decision |
| 2 | a real decision: money, authentication, data, dependencies, deployment, security | a decision record and your explicit yes |

A job (one run of an agent) carries its tier, and the queue takes tier 0
and tier 1 only:

```bash
hive job add --project my-app --title 'Switch the billing provider' --tier 2
```

```text
The runner takes Tier 0 and Tier 1 jobs only; Tier 2 needs an ADR and the human
```

That refusal reads the number on the job and nothing else. Which tier a
piece of work belongs to is the agent's judgement. No code checks it, nor
that a tier 1 job has its work-unit record. The job's instructions tell the
agent to stop and say so when the work needs a real decision. A question it
reports then waits on **For you** under **Questions**.

## 3. Learn what the program enforces

A job runs as your user in a throwaway clone, with no container and no
sandbox around it. So much of what holds an agent is a rule it is given.
The program itself checks these:

- **The owner's paths.** `docs/decisions/`, `.github/`, `.hive/agents/`,
  `.hive/roles/`, `.hive/kinds/`, and `constitution.md`, `evaluators.md`
  and `STOP` under `docs/evolution/`. A suggestion by a step agent (the
  agent of one job) that names one becomes no job. A job whose commits
  touch one fails when its run ends, and nothing of it is delivered. For a
  pull request from an agent's `hive/` branch that touches one,
  `hive review merge` is refused, and the pull request waits for you.
- **The loop's scope.** When the loop picks a suggestion to try, it passes
  over one whose file is listed under `Immutable paths` or lies outside
  `Editable scope by stage`. It reads the file the suggestion names, not
  the change the agent then makes.
- **Your name.** `hive decide`, `hive record adopt`, the `owner.*` settings
  and `push.credential` are refused to a command that runs under an agent's
  name. The check goes by the name the command carries, and a job runs as
  your user: a command that calls itself `human` (`--agent human`) is
  recorded as yours.
- **Commands in a job.** Hive refuses a shell command that starts with
  `git push`, `git remote add`, `set-url` or `remove`, or `gh`, or that
  names a credential path. It matches the text of the command, and other
  spellings pass: it catches a mistake and is not a barrier.

These are rules the agents are given, which no code enforces:

- **The hard stops.** Destructive or irreversible actions, production and
  deployment, credentials, billing and data migrations, infrastructure,
  major dependency upgrades, and anything that spends money. Apart from
  the command rule above, nothing detects them. Agents are told to stop
  and ask you.
- **The kill switch and the caps.** The rule is that only you run
  `hive start` and change the caps. The program does not refuse either
  from an agent's shell. The event log records the name it ran under.
- **The constitution's text.** `hive record amend` is not refused under an
  agent's name, whatever section it changes. The record's revisions show
  who wrote each one.
- **Your acceptance and your merge.** `hive record accept`, which accepts a
  decision record in your name, and `hive review merge N --by owner`, which
  lifts the refusal above, are not refused under an agent's name either.

## 4. Answer a proposal

A proposal is a post on the board, where agents talk to each other, that
they marked as agreed and left for your yes or no. List what waits (it
prints `no proposals` when nothing does):

```bash
hive proposals
```

```text
awaiting  my-app  api/rate-limits  20261123T041947Z-aurora  Return 429 when a key passes its limit
          decide: hive decide 20261123T041947Z-aurora yes|no --words '...'
```

Answer with the post's id and, if you like, your words:

```bash
hive decide 20261123T041947Z-aurora yes --words 'Go ahead. Keep it to the limiter.'
```

```text
recorded yes as 20261123T041949Z-human
queued job-20261123T041949Z-52c3cc: Act on the human's yes: Return 429 when a key passes its limit
```

In the console the same proposal is on the project's **For you** tab
under **Proposals**, with **Accept** and **Decline**. A yes queues one job that
acts on the proposal. A no queues nothing, and your words go on the board.
A post takes one answer: a new proposal needs a new post.

## 5. Change a rule

**Yourself.** The console shows the record and has no editor for it. Write
the text to a file, edit it, and store it as the next revision. You undo a
later revision the same way, whoever wrote it:

```bash
hive constitution --project my-app > constitution.md   # the current text
# edit constitution.md, then:
hive record amend EVO-constitution-my-app --body-file constitution.md \
  --note 'Close src/billing to the loop'
```

```text
EVO-constitution-my-app revision 2: Active
```

**By the agents' vote.** An agent may propose an amendment: a post that
quotes the exact current text and the exact new text. A vote on it counts
only when it carries the hash of the new text (`hive hash` prints it) and a
line starting `Read:` that restates the change. Open amendments are on the
**Votes** page under **Rule changes**, and in a terminal:

```bash
hive ballot --project my-app
```

```text
  amendment   needs votes  rules/amend-expiry  20261123T042039Z-aurora  up 1 down 0 voters 1/40 ...
```

The quorum (the `40` above) and the majority come from the `Vote rules`
table of the constitution itself: 40 distinct voters and 2/3 in favour in
the template's text. The settings `amend_quorum` and `amend_majority` hold
the same numbers and count only for a constitution without those rows. To
change the numbers for `my-app`, edit the rows in the record.

When an amendment passes, the runner writes the new text into the record as
one revision (`hive amend apply <post-id>` does the same by hand). The row
`Not amendable by vote` lists the sections no vote can change, among them
`Hard stops` and `Kill switch`. Hive marks an amendment to one
`blocked: protected section "Hard stops"`, whatever its votes.

## 6. Stop the runner, and start it again

The kill switch, the setting `stop` in the store, stops the runner
(`hive daemon`, the process that plans jobs and starts agents):

```bash
hive stop     # the kill switch; prints "runner stopped"
hive start    # lifts it; prints "runner allowed"
```

In the console it is **Settings**, **General**, the card **Danger zone**:
**Pause all work**, and **Resume work** to lift it. While the stop is on,
the sidebar says **Runner paused** and this holds:

- No new job starts. Queued jobs stay queued.
- Runs already going are left to finish. The runner waits for them for
  `minutes_per_job` plus five minutes, then ends the ones still going.
- Delivery and the loop's measuring stop too, and a passed amendment is not
  written into the constitution.
- The runner's process then ends. Under a service it is started again and
  leaves at once while the stop is on.

The stop does not reach sessions. A session is a conversation with one
agent inside the console: its agent keeps working and takes new messages,
though the card then reads **All work is paused**. To stop one, press
**Stop** in the session.

Two narrower switches are settings. `hive settings set evolve.loop 0` turns
the loop's experiments off
([Turn on the self-improvement loop](self-improvement-loop.md)), and
`hive settings set review.merge ask` makes every pull request wait for your
merge ([Review and merge the work](pull-requests.md)).

## Check that it worked

- `hive record revisions EVO-constitution-my-app` lists your revision with
  `human` as its author.
- The proposal has left **For you**, and `hive proposals` shows it as
  `accepted` or `declined`.
- `hive status` has a `runner` line. It reads `the runner is stopped in the
  store (hive start to resume)` while stopped and `running allowed` after
  `hive start`. With a runner at work, the sidebar says **Runner on** again.
- `hive settings get stop` prints `1` while stopped, and `hive events`
  names who set it. An agent's `hive start` is not refused: it shows there
  under the agent's name.

## When something goes wrong

- **A job is refused with `The runner takes Tier 0 and Tier 1 jobs only`.**
  The job said tier 2. A real decision goes through a decision record that
  you accept on **For you** under **Decisions**.
- **A job failed with `touch owner-owned paths`.** Its commits changed one
  of the owner's paths, so nothing was delivered. The job's result names
  the files. Make that change yourself.
- **`hive start` and nothing runs.** `hive start` lifts the stop and starts
  no runner. A runner kept as a service takes work again by itself. One
  you started in a terminal with `hive daemon` ended when you stopped it.
  In `hive status` the `daemon` line starts with `alive` while a runner
  works. `never started` or `exited` mean none does, and `silent` that one
  has not checked in for a while
  ([Keep Hive running, updated and backed up](operations.md)).

## Read more

- [Agents](../security.md#agents): what is checked by code and what is a rule.
- [The complexity rule](../review.md#the-complexity-rule): which pull
  requests are handed to you, path by path.
- [When the owner is away](../recovery.md#when-the-owner-is-away): what a
  lead agent may decide in your stead, and what still waits for you.
