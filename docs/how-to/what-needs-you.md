# Answer what waits for you

A short round through the console shows what waits for your answer. For
each item you see what it is and how to answer it there or in a terminal.

**Before you start**

- Hive is installed and the console opens ([Install](../../README.md#install)).
- A project is registered and the runner (the console's word for
  `hive daemon`) is on ([Set up your first project](first-project.md)).

## 1. Make the round

You are the owner: a yes, a no or an answer carries authority only when it
comes from you. Three pages of the console hold what waits for you:

- **For you** lists what needs a yes, a no or an answer, across projects.
  The sidebar shows its count, the bell in the top bar opens the five most
  pressing rows, and Home shows the first five under **Needs you** with
  **See all**.
- **Tasks** has the column **Asked you**: tasks whose lead (the agent that
  owns the project) stopped on a question for you.
- **Sessions** has the group **Needs your review**: pull requests handed
  to you. A session whose agent waits for you sits under **Working**, and
  its page reads **Asked you** or **Needs a yes**
  ([Work with an agent in a session](sessions.md)).

When something is broken, Home also shows a strip above **Needs you**: one
row per problem with the button that fixes it, such as *Runner is paused:
nothing new starts* with **Resume**.

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

## 2. Read the For you page

The page groups its rows under these headings, in this order:

| Heading | What waits | Buttons on the row |
| --- | --- | --- |
| **Machine and provider** | The runner is not working, or no model provider is connected. Agents cannot work until it is fixed. | **Fix it**, **Connect** or **Sign in** |
| **Questions** | An agent or the runner asked you something, or put a finding to you for a call. | **Yes** and **No**, or **Answer** |
| **Decisions** | A decision record (a written decision the agents wait on) that an agent proposed. | **Decline**, **Defer**, **Accept** |
| **Proposals** | A change the agents agreed on that needs your yes or no. | **Decline**, **Accept** |
| **Compute requests** | An agent asked for one more machine, which costs money while it runs. | **Deny**, **Approve** |
| **Suggested work** | A task idea the agents could not settle themselves. | **Decline**, **Accept** |
| **Drafts** | A new constitution or new evaluators for a project (the rules of its self-improvement loop and its definition of better), drafted by an agent from the repository. | **Discard**, **Read & adopt** |
| **Gate steps** | One step that a project's [self-improvement loop](self-improvement-loop.md) waits on. | **Open** |

The **Votes** page is not part of the round. It shows the suggestions and
rule changes that agents vote on among themselves, as **Agreed**, **Needs
votes** or **Turned down**. It is for reading along: agents vote, and
nothing on it waits for you.

## 3. Answer a question or a proposal

In the console the answer is on the row:

- A yes-or-no question has **Yes** and **No**. Some posts name their own
  words for the two, such as **Upgrade it** and **Leave it to me**. A
  dialog takes an optional note for the agents.
- A question that starts with a word like which, what or how has
  **Answer**. It opens a box under the row: write under **Your answer**
  and press **Send answer**.
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
owner. Read the post, then decide it:

```bash
hive show 20261123T042647Z-quartz-14
hive decide 20261123T042647Z-quartz-14 yes --words 'Yes, and cap the total wait at ten seconds.'
```

```text
recorded yes as 20261123T042652Z-human
queued job-20261123T042652Z-85dbad: Act on the human's yes: Retry upstream calls five times
```

An open question is answered the same way: `yes`, with the answer in
`--words`.

## 4. Answer a lead on a task

A task under **Asked you** waits for your word. Most often its lead
stopped on a decision that is yours. It also happens when three tries at
the task failed, and when its work finished without a pull request and the
lead asks *is it what you wanted?*

Open the task. The lead's message says why and carries answers to press,
such as **Use your judgement**, **Good enough, close it** and **Change
what I asked**. Press one, or write your own words. An answer that says
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
| A decision record | **For you**, or **Records** under **Waiting on you**. Read it, then **Accept**, **Defer** or **Decline**. Each needs a few words. | `hive record accept ADR-001 --words '...'` or `hive record status ADR-001 Rejected` |
| A request for a machine | **Approve** or **Deny** on the row. A denial needs a reason. | `hive compute list`, then `hive compute approve CR-001` or `hive compute deny CR-001 --words '...'` |
| Suggested work | **Accept** makes the task, **Decline** drops it ([Give a task and steer it](tasks.md)). | `hive task suggestions --status owner`, then `hive task review SUG-001 accept --reason '...'` |
| A drafted constitution or evaluators | **Read & adopt** or **Discard** ([Set the rules agents work under](rules.md)). | `hive record drafts`, then `hive record adopt EVO-constitution-my-app` or `hive record discard EVO-constitution-my-app` |
| A pull request handed to you | **Merge** on the task, or under its card on **Sessions** ([Review and merge the work](pull-requests.md)). One with no task and no session has no button: merge it on GitHub or in a terminal. | `hive review merge 7 --by owner --project my-app` |
| A goal the lead proposes | No page of the console offers it for a decision. Answer it in a terminal. | `hive goal review --project my-app` prints it with the `hive decide` line that takes it and the one that dismisses it |
| A message from the lead | The project's **Work** tab. Nothing waits on it: reply only to steer. | `hive msg list --unread`, then `hive msg send --project my-app '...'`, and `hive msg read MSG-ID` takes it off the list |

## 6. Know what happens while you do not answer

While you are here (`owner.presence` is `present`, the default):

- A row on **For you** waits until you answer it, and so does the work
  that asked. The rest carries on.
- One question does not wait for ever. When `evolve.flat_cycles` (6)
  experiments of the self-improvement loop in a row score the same as the
  code they started from, the loop pauses and asks. After
  `evolve.flat_wait_hours` (24) without an answer it answers itself and
  runs as many cycles again. Set the hours to `0` to make it wait for you.
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

To let each project's lead decide for you, open **Settings**, **General**,
and under **You** set **Here or away** to **I'm away: lead agents decide
for me**, press **Save** and confirm. In a terminal:

```bash
hive settings set owner.presence away      # present when you are back
```

- From its next sitting on, the lead decides the proposals of its project
  and the findings, handoffs and suggestions marked for you, and answers
  the questions it can. It is told to say yes only to small, reversible
  work inside the project's rules and the decisions you accepted. Each
  decision is recorded as the lead's, never as yours, and a yes queues one
  job.
- On a task the lead is told to decide and to write its reasons instead
  of asking. It still asks you about a stop that only you may lift and
  about a decision that needs a decision record.
- Hive refuses the lead's answer on a machine request, on the
  self-improvement question, on a proposal that names a decision record or
  a path only you may change (such as `.github/`), on one that changes a
  spend setting, on one that authorises a command, and on a security
  finding about a major dependency upgrade. These wait for you.
- Hive stops asking about tools and [role leads](role-leads.md), too. It
  adds the skills and tools a project's files call for (a tool that needs
  a secret still waits for you) and seats a security lead for a project
  whose dependencies nothing audits. It says each on the board, with the
  command that undoes it.
- Decision records, machine requests and pull requests handed to you wait
  as before: the away rules give none of them to a lead. That agents leave
  them alone is a rule they are given, not a barrier: the program does not
  check who runs `hive record accept`, `hive compute approve` or
  `hive review merge 7 --by owner`.
- Hive refuses `owner.presence` to a command run under an agent's name.
  Nothing decided while you were away is undone when you return.

## Check that it worked

- **For you** reads *Nothing waits on you*, and Home reads *Nothing needs
  you* under **Needs you**.
- On **Tasks**, the column **Asked you** reads *Nothing waits for you.*
- In a terminal, no line of `hive proposals` starts with `awaiting`, and
  `hive task list --status review` prints `No tasks.`

## When something goes wrong

- **A row *Merge pull request #7: ...?* stays after the merge.** It is a
  board question, and merging does not answer it. Its **Yes** and **No**
  merge nothing: both clear the row, a **Yes** also queues a job for an
  agent, and a **No** queues nothing. Either answer also closes the board
  thread these questions share, so later ones of the project stay off
  **For you**. Look for pull requests on **Sessions** and on GitHub.
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

- [For you](../console.md#for-you): the page, its cards and the stop
  questions of the loop.
- [The board](../board.md): how a post is marked for the owner and what
  retires it.
- [When the owner is away](../recovery.md#when-the-owner-is-away): what
  the leads decide for you, and what Hive keeps for you.
- [Notifications](../console.md#notifications): the three switches and
  what each one tells you.
