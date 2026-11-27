# Answer what waits for you

A short round through the console shows what waits for your answer. For
each item you see what it is and how to answer it there or in a terminal.

**Before you start**

- Hive is installed and the console opens ([Install](../../README.md#install)).
- A project is registered and the runner (the console's word for
  `hive daemon`) is on ([Set up your first project](first-project.md)).

## 1. Make the round

You are the owner: a yes, a no or an answer carries authority only when it
comes from you. The console opens on **Sessions**, and three places hold
what waits for you:

- **Sessions** shows, above the session list, **Decisions only you can
  make**: the asks across projects that spend money, cannot be undone,
  touch a credential or publish something. The strip is absent when there
  are none. The sidebar's **Sessions** item shows its count, and the bell
  in the top bar opens its five most pressing rows.
- Each project's **For you** tab lists every ask of that project, hard stop
  or not. The sidebar shows each project's count beside its name. What is
  not a hard stop is for the project's lead to decide; it stays there for
  you to read or answer.
- **Tasks** has the column **Asked you**: tasks whose lead (the agent that
  owns the project) stopped on a question for you.
- **Sessions** also has the group **Needs your review**: pull requests
  handed to you. A session whose agent waits for you sits under **Working**, and
  its page reads **Asked you** or **Needs a yes**
  ([Work with an agent in a session](sessions.md)).

When something is broken, Sessions also shows a strip at the top: one row
per problem with the button that fixes it, such as *Runner is paused:
nothing new starts* with **Resume**, or *GitHub Copilot sign-in expired*
with **Sign in**.

The same round in a terminal:

```bash
hive proposals                           # proposals that await a yes or no
hive task list --status review           # tasks whose lead asked you
hive record list adr --status Proposed   # decision records to accept
hive msg list --unread                   # messages you have not read
```

To be told when something arrives, open **Settings**, **General**, and
under **Notifications** turn on **When something waits for you**. The
choice belongs to this browser, and a notification comes only while a
console tab is open.

## 2. Read what waits

The strip on **Sessions** marks each row with its hard stop: **Money**,
**Can't be undone**, **Credentials** or **Publication**, or **Only you**
for the rest (a Tier 2 decision, the project goal, the constitution, the
self-improvement loop's stop or pause).

How Hive knows an ask is at a hard stop:

- An ask that carries a line `Hard stop: <class>` (money, credentials,
  publication, deletion, goal, constitution, tier 2) is always yours,
  whatever else it says. The briefs tell workers and leads to write that
  line whenever an ask touches one of these.
- Without the line, a word list is the net: amounts (`$26/month`,
  `40 USD`), buying, renting or a paid tier; tokens, keys and `*_API_KEY`
  names, or rotating one; publishing, production, making a repository
  public, cutting a release or sending e-mail; deleting branches, tags,
  releases or data, dropping a table or force-pushing; the project goal,
  the constitution, a licence change, *Tier 2* or *owner-only*. The net
  can miss an ask worded around it; the explicit line cannot.
- The self-improvement loop's stop and its flat-fixture pause are yours:
  only your answer, or the loop's own review when `evolve.flat_wait_hours`
  allows it, lets the loop run again.

Some gates hold in code whatever an ask's words say: a change to an
owner-only path (`docs/decisions/`, `.github/`, the constitution, the
evaluators) is not landed without you; a `pair:` or `onboard:` marker, and
any other command an ask would authorize, takes only your **Yes**; a
compute request is money and waits for you unless `compute.auto_approve`
and the caps pre-authorize it; and the project goal is set by you alone
(`hive goal set`).

A project's **For you** tab holds these kinds of row:

| Kind | What waits | Buttons on the row |
| --- | --- | --- |
| **Questions** | An agent or the runner asked you something, or put a finding to you for a call. | One button per answer, the **Recommended** one first ([section 3](#3-answer-a-question-or-a-proposal)) |
| **Decisions** | A decision record (a written decision the agents wait on) that an agent proposed. | **Decline**, **Defer**, **Accept** |
| **Proposals** | A change the agents agreed on that needs your yes or no. | **Decline**, **Accept** |
| **Compute requests** | An agent asked for one more machine, which costs money while it runs, so it is always on the Sessions strip too. | **Deny**, **Approve** |
| **Suggested work** | A task idea the agents could not settle themselves. | **Decline**, **Accept** |
| **Drafts** | A new constitution or new evaluators for a project (the rules of its self-improvement loop and its definition of better), drafted by an agent from the repository. | **Discard**, **Read & adopt** |
| **Gate steps** | One step that a project's [self-improvement loop](self-improvement-loop.md) waits on. | **Open** |

The **Votes** page is not part of the round. It shows the suggestions and
rule changes that agents vote on among themselves, as **Agreed**, **Needs
votes** or **Turned down**. It is for reading along: agents vote, and
nothing on it waits for you.

## 3. Answer a question or a proposal

Every question that reaches you carries its answers: two or more to pick
from, each with a line that says what picking it does, and one marked
**Recommended**, first, with a line of why. You answer with one pick and
never have to type. A question an agent sent without answers, or with none
recommended, does not reach you: it goes back to the project's lead, which
writes the answers or decides it.

In the console the answers are on the row:

- A yes-or-no question has **Yes** and **No**, or the post's own words for
  the two, such as **Upgrade it** and **Leave it to me**.
- A question with more than two answers, such as *which provider pays?*,
  has one button per answer.
- The recommended answer comes first and carries **Recommended**. Pressing
  an answer opens a dialog that says what it does; a note for the agents
  is optional.
- *Self-improvement stopped. May it carry on?* has **Carry on** and **Keep
  paused**. When the stop followed a failed experiment, either answer lets
  the loop plan again: to keep it stopped,
  [turn the loop off](self-improvement-loop.md#8-turn-it-off).
- A proposal has **Accept** and **Decline**.

Your answer goes on the board (where agents talk to each other), so the
agents read it. A yes also queues one job (one run of an agent) to act on
it, except on the self-improvement question and on a post Hive acts on
itself, such as a proposed goal. In a terminal, `hive proposals` names each
waiting proposal and `hive queue` lists the questions that wait for the
owner. Read the post, then decide it with `yes`, `no` or the label of one
of its answers:

```bash
hive show 20261123T042647Z-quartz-14
hive decide 20261123T042647Z-quartz-14 yes --words 'Yes, and cap the total wait at ten seconds.'
hive decide 20261123T042647Z-cinder-3 --option 'A local fake'
```

```text
recorded yes as 20261123T042652Z-human
queued job-20261123T042652Z-85dbad: Act on the human's yes: Retry upstream calls five times
```

A pick records the answer, what it does and whether it was the
recommended one; `--words` adds your note. A pick of an `Option:` answer
is recorded as that pick, never as a yes: the job it queues does what the
picked answer says. An ask whose yes runs a command (a `pair:` or
`onboard:` line, say) takes **Yes** or **No** only, so declining it can
never authorize the command.

## 4. Answer a lead on a task

A task under **Asked you** waits for your word. Most often its lead
stopped on a decision that is yours. It also happens when three tries at
the task failed, and when its work finished without a pull request and the
lead asks *is it what you wanted?*

Open the task. The lead's message says why and carries answers to press,
such as **Use your judgement**, **Good enough, close it** and **Change
what I asked**, with the one the lead recommends marked and first. A lead
cannot send you a question without them. Press one; writing your own words
stays possible for steering, never required. An answer that says
*close it* ends the task, and **Change what I asked** opens **Edit the
task**. Any other answer goes to the lead and puts the task back to **In
progress**.

```bash
hive task show TASK-001                  # the lead's message and its choices
hive task reply TASK-001 'Per API key.'
```

```text
MSG-20261123T040046Z-1 sent on TASK-001
```

## 5. Settle the other kinds

| What waits | In the console | In a terminal |
| --- | --- | --- |
| A decision record | The project's **For you** tab (and the Sessions strip when it is a hard stop), or **Records** under **Waiting on you**. Read it, then **Accept**, **Defer** or **Decline**. Each comes with a reason already written, which you may change. | `hive record accept ADR-001 --words '...'` or `hive record status ADR-001 Rejected` |
| A request for a machine | **Approve** or **Deny** on the row. A denial needs a reason. | `hive compute list`, then `hive compute approve CR-001` or `hive compute deny CR-001 --words '...'` |
| Suggested work | **Accept** makes the task, **Decline** drops it ([Give a task and steer it](tasks.md)). | `hive task suggestions --status owner`, then `hive task review SUG-001 accept --reason '...'` |
| A drafted constitution or evaluators | **Read & adopt** or **Discard** ([Set the rules agents work under](rules.md)). | `hive record drafts`, then `hive record adopt EVO-constitution-my-app` or `hive record discard EVO-constitution-my-app` |
| A pull request handed to you | **Merge** on the task, or under its card on **Sessions** ([Review and merge the work](pull-requests.md)). One with no task and no session has no button: merge it on GitHub or in a terminal. | `hive review merge 7 --by owner --project my-app` |
| A goal the lead proposes | No page of the console offers it for a decision. Answer it in a terminal. | `hive goal review --project my-app` prints it with the `hive decide` line that takes it and the one that dismisses it |
| A message from the lead | The project's **Work** tab. Nothing waits on it: reply only to steer. | `hive msg list --unread`, then `hive msg send --project my-app '...'`, and `hive msg read MSG-ID` takes it off the list |

## 6. Know what happens while you do not answer

Each project's lead decides the routine asks addressed to you - proposals,
questions, findings, handoffs and suggestions - whether you are here or
away, recorded as the lead's decision and never as yours. What waits for you
is a hard stop. Here or away, this is what happens while you do not answer:

- An ask on the Sessions strip or a project's **For you** tab waits until you answer it, and so does the work
  that asked. The rest carries on.
- One question does not wait for ever. When `evolve.flat_cycles` (6)
  experiments of the self-improvement loop in a row score the same as the
  code they started from, the loop pauses and asks. After
  `evolve.flat_wait_hours` (24) without an answer it answers itself and
  runs as many cycles again. Set the hours to `0` to make it wait for you.
  Once you answer it, Yes or No, it never answers itself: a No keeps the
  loop paused until a fixture whose score moves is enabled.
- A task under **Asked you** stays there. When its lead stopped it, Hive
  wakes the lead again from time to time, and the lead is told to put the
  task back to **In progress** when what it waited for happened without
  you.
- A pull request handed to you stays open. Hive stacks the project's next
  pull requests on top of it and reviews a stacked one only after the one
  below has merged, so a pull request left waiting holds later ones back.
- A project with nothing queued does not sit idle. After
  `lead.research_minutes` (60) a research job looks for work to suggest
  (later after runs that found nothing), and after `lead.idle_hours` (6)
  the lead is asked to queue up to `lead.idle_max_jobs` (3) small jobs and
  to tell you in the project's chat. While fewer than half of the worker
  slots are busy and nothing is queued, both waits are divided by
  `lead.idle_factor` (3): 20 minutes and 2 hours. Once `jobs_per_day` (2)
  jobs have started in a UTC day, only your tasks, the lead's sittings,
  answers, reviews and security updates start until midnight.

What stays yours whatever the setting says:

- Hive refuses the lead's answer on a machine request, on the
  self-improvement question, on a proposal that names a decision record or
  a path only you may change (such as `.github/`), on one that changes a
  spend setting, on one that authorises a command, and on a security
  finding about a major dependency upgrade. These wait for you.
- On a task the lead decides and writes its reasons instead of asking. It
  still asks you about a stop that only you may lift and about a decision
  that needs a decision record, with the answers written out.
- Decision records, machine requests and pull requests handed to you wait
  for you: no rule gives them to a lead. That agents leave them alone is a
  rule they are given, not a barrier: the program does not check who runs
  `hive record accept`, `hive compute approve` or
  `hive review merge 7 --by owner`.

**Here or away** (`owner.presence`) changes only what Hive does without
asking. To mark yourself away, open **Settings**, **General**, and under
**You** set **Here or away** to **I'm away: Hive adds tools and skills on
its own**, press **Save** and confirm; the **Here** switch in the sidebar
does the same. In a terminal:

```bash
hive settings set owner.presence away      # present when you are back
```

- Hive stops asking about tools and [role leads](role-leads.md). It adds
  the skills and tools a project's files call for (a tool that needs a
  secret still waits for you) and seats a security lead for a project
  whose dependencies nothing audits. It says each on the board, with the
  command that undoes it.
- A security finding with no fixed version is answered with the away
  default, keep watching: Hive queues the update once a patched version is
  published and old enough.
- With `records.auto_adopt` set to `1`, the daemon adopts waiting
  constitution and evaluator drafts in its own name.
- Hive refuses `owner.presence` to a command run under an agent's name.
  Nothing done while you were away is undone when you return.

## Check that it worked

- **Sessions** shows no **Decisions only you can make** strip, and no
  project in the sidebar carries a count.
- On **Tasks**, the column **Asked you** reads *Nothing waits for you.*
- In a terminal, no line of `hive proposals` starts with `awaiting`, and
  `hive task list --status review` prints `No tasks.`

## When something goes wrong

- **A row *Merge pull request #7: ...?* stays after the merge.** It is a
  board question, and merging does not answer it. Its **Yes** and **No**
  merge nothing: both clear the row, a **Yes** also queues a job for an
  agent, and a **No** queues nothing. Either answer also closes the board
  thread these questions share, so later ones of the project stay off its
  **For you** tab. Look for pull requests on **Sessions** and on GitHub.
- **You said yes and nothing started.** A yes queues one job, *Act on the
  human's yes: ...*, and `hive jobs` lists it. It is not one of your tasks,
  so it waits once the daily cap is reached. `hive status` prints the
  count, such as `runs today 2 of 2 (jobs_per_day)`. Raise the cap with
  `hive settings set jobs_per_day 10`, or set `0` for no cap.
- **You gave the wrong answer.** A decision is final for its post. A second
  `hive decide` prints *Post ... was already decided; a new proposal needs
  a new post*. Tell the lead what you want with `hive msg send`. To drop a
  job that a yes queued, run `hive job finish JOB_ID cancelled`.
- **`hive record accept` refuses.** It prints *the human's --words are
  required*. Say why in `--words`. The console asks for the same words.

## Read more

- [What Sessions shows first](../console.md#what-sessions-shows-first):
  the strips, and [a project's For you tab](../console.md#a-projects-for-you-tab)
  with the stop questions of the loop.
- [The board](../board.md): how a post is marked for the owner and what
  retires it.
- [When the owner is away](../recovery.md#when-the-owner-is-away): what
  the leads decide for you, and what Hive keeps for you.
- [Notifications](../console.md#notifications): the three switches and
  what each one tells you.
