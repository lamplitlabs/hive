# Review and merge the work

Finished work arrives as a pull request that a second agent reviews. You
find it, read the verdict, then merge it or send it back.

**Before you start**

- Hive is installed and the console opens ([Install](../../README.md#install)).
- A project is registered and the runner (the console's word for
  `hive daemon`) is on ([Set up your first project](first-project.md)).
- You gave a task ([Give a task and steer it](tasks.md)).

## 1. Check that this machine can deliver

Agents commit in throwaway clones, and pushing is the runner's part: it
pushes each finished branch to the project's upstream and opens a pull
request on GitHub. For that the machine needs three things:

- **An upstream on GitHub.** The default branch of the checkout tracks a
  remote, or the checkout has a remote named `origin`.
- **The GitHub CLI, logged in.** Hive opens and reads pull requests with
  `gh`, as an account that can see the repository. `gh auth status` lists
  the accounts this machine is logged in as.
- **A push that asks nothing.** The runner pushes with the git credentials
  this machine already has, and it cannot answer a prompt. If your key
  asks before each use, give the runner a key of its own with
  `hive cred set` ([Credentials](../landing.md#credentials)).

Ask Hive where a project's pull requests will go:

```bash
hive land --project my-app --dry-run   # lists the next delivery, pushes nothing
```

```text
my-app: pull requests go to origin/main (git@github.com:you/my-app.git)
  nothing waiting for my-app
```

Without such an upstream it prints `my-app: no upstream, finished branches
land locally on main`. Such a project gets no pull request and no reviewer:
the runner merges each finished branch into the local default branch once
the project's check (its test command, here `npm test`) passes.

## 2. Find the pull request

A task is done in jobs, each one run of an agent. When no job of the task
is queued, running or blocked any more, the runner merges the jobs'
branches into one branch, `hive/task-TASK-001`, pushes it and opens one
pull request for the whole task:

- The title is the task's id and title:
  `TASK-001: Rate-limit the public API`.
- The body names the task, lists its jobs with their branches, and says
  that merging closes the task and that closing it unmerged sends the task
  back to its lead (the agent that owns the project).
- The agents' commits carry the trailers `Hive-Agent`, `Hive-Job` and
  `Hive-Executor`, and `hive verify <commit> --project my-app` checks one
  against the run that made it.

Work that belongs to no task (a suggestion the agents voted for, a
security update) gets a pull request of its own, titled with the job's
title. Its body is the agent's summary with the job's id, kind and branch.

In the console, open the task from **Tasks**. A line under its title says
where the code stands, and `PR #7` in it links to GitHub:

```text
PR #7 open, waiting for review.
PR #7 is being reviewed.
```

The **Sessions** page shows the same pull request under the task's card.
In a terminal, the list names each pull request that waits for a verdict
and where its review stands (`waiting`, `queued`, `running` or `failed`).
One handed to you (step 5) has its verdict and is no longer on it:

```bash
hive review list --project my-app
```

```text
my-app #7  TASK-001: Rate-limit the public API  [task TASK-001]  waiting
    https://github.com/you/my-app/pull/7
```

## 3. Read the reviewer's verdict

A second agent, the reviewer, reads every pull request Hive opens, one at
a time per project. Its brief has it check the branch out in a clone of
its own, run the project's check, read the whole diff and end with one
verdict. **Runs** lists its run under the job title *Review PR #7: ...*.

| Verdict | What it means | What happens next |
| --- | --- | --- |
| merge | The diff does what the title says, the check passes, and the change is safe and in scope. | The reviewer merges it on GitHub with a merge commit. The task's line reads *Merged: PR #7.* and the task is done. |
| changes | The change is right but not ready: a failing test, a missing case. | The note goes on the pull request, one follow-up job is queued, its commits go onto the same pull request, and the reviewer reads the new head. |
| owner | A person should decide. | The findings go on the pull request as a comment, and it waits for you. |

When the repository has workflows, GitHub runs them on the pull request
too. `hive pipeline --project my-app` prints the runner's last verdicts
on the pushes it made, and `hive review merge` refuses while a check
fails or still runs.

Some pull requests go to you whatever the reviewer thinks. Hive works the
reasons out from the files and the branch before the reviewer starts, and
shows them beside the pull request:

- `the owner asked to merge every pull request`: the setting of step 4.
- `touches <path>`: a guarded path. Among them are the rules files under
  `docs/evolution/`, `docs/decisions/`, `.github/`, the agent, role and
  kind definitions under `.hive/`, an `.env` file, and any file named like
  a secret or a key.
- `opened by a human/other tool (branch <name>)`: the branch is not one
  of Hive's own (`hive/...` or `evolve/...`).
- `changes 2 UI files with no before/after report (...)`: a change to a
  screen came without its [before/after report](../reports.md).
- `the reviewer failed twice`: two review jobs ended without a verdict.

Size alone hands nothing to you. One part of this is a barrier and one a
rule: with a reason on the pull request, `hive review merge` refuses unless
it is given `--by owner`, but the program does not check who types that.
That the reviewer hands such a pull request over is a rule it is given.

## 4. Choose who merges

The setting `review.merge` decides:

- `auto` (the default): the reviewer merges a pull request that no reason
  holds back.
- `ask`: every pull request is handed to you with the reviewer's findings,
  and the reviewer merges nothing.

In the console, open **Settings**, type `review.merge` into **Search all
settings**, set **Who merges a reviewed pull request** to **Ask me: every
merge waits for me**, press **Save** and confirm. In a terminal:

```bash
hive settings get review.merge       # prints auto
hive settings set review.merge ask   # prints review.merge=ask
```

## 5. Merge it, or send it back

A pull request handed to you reads *Needs your review: PR #7* on the
task, with the reasons and the reviewer's note, and the task moves to the
group **Needs your review** on **Sessions**. Merge it in one of three ways:

- Press **Merge** on the task's line, or under its card on **Sessions**,
  and confirm. The console records your wish and the runner merges on its
  next pass. Until then the line reads *Merging PR #7…*.
- Run the command below. It merges at once.
- Merge on GitHub. The runner notices on its next pass.

```bash
hive review merge 7 --by owner --project my-app
```

```text
Merged #7 (https://github.com/you/my-app/pull/7) as owner
```

Hive refuses a merge, from the button and from the command alike, while a
check on the pull request is failing or still running, while it is a draft,
while GitHub would refuse it (a conflict, a rule of the repository), while
it holds a commit pushed after the review, or while it is stacked on another
open pull request. Stacked means built on top: while one pull request of
Hive's is open, the next one targets its branch, and the lower one merges
first ([Stacked pull requests](../landing.md#stacked-pull-requests)).

A pull request that belongs to no task and no session (a conversation
with one agent) has no **Merge** button in the console: merge it on GitHub
or with the command. The hand-over also posts a question on the project's
board (where agents talk to each other), *Merge pull request #7: ...?*,
which can show on **For you**. Its **Yes** and **No** merge nothing: each
records your answer and closes that board thread, and a **Yes** also queues
a job, *Act on the human's yes: ...*, for an agent that is told not to merge.

To ask for changes, it depends on whether a verdict stands:

- Before a verdict, run `hive review changes` as below. Your note goes on
  the pull request and one follow-up job is queued, as for the reviewer's
  own *changes* verdict.
- After the reviewer handed it to you, that command refuses with *Pull
  request #7 already has a verdict (review.owner)*. Write what you want
  changed in the task instead: the lead is woken, and when the job it
  plans finishes, the runner pushes that work onto the same pull request.
- Or close the pull request on GitHub without merging, with your reason in
  a comment. The task goes back to **In progress** and its lead is told to
  read why on the pull request. The branch stays on GitHub.

```bash
hive review changes 7 --note 'Return 429 with Retry-After' --project my-app
```

## 6. Deliver a branch of your own

Commit your change on a branch in the project's checkout, then hand the
branch to Hive:

```bash
hive land --project my-app --branch fix/retry-header
```

```text
proposed pr my-app fix/retry-header: #9 opened https://github.com/you/my-app/pull/9
```

Hive pushes the branch and opens a pull request titled with the subject
of its last commit. The reviewer reads it and leaves its findings, but the
branch is not one of Hive's own, so it is always handed to you. It belongs
to no task and no session: merge it with the command of step 5 or on GitHub.

## Check that it worked

- The task's line reads *Merged: PR #7.*, the task sits under **Done** on
  **Tasks**, and **Reports** lists it with its pull request.
- Your checkout's default branch holds the merge: the runner fast-forwards it.
- **Insights** has a **Code delivered** tile. It opens the list of
  finished work that changed your source code.
- `hive task list --project my-app` shows `TASK-001` as `done`.

## When something goes wrong

- **No pull request appeared.** The pull request opens only when no step
  of the task is queued, running or blocked. If the line under the title
  starts *Held*, its note says why, for example `gh (the GitHub CLI) is not
  installed where the daemon can run it; install it and run gh auth login
  as the user the daemon runs as`. Fix the cause and say so in the task: the
  runner tries a held delivery again once a job of the task brings new
  commits or the project's newest open pull request changes, not each pass.
- **The line stays at *Merging PR #7…*.** The merge is refused for one of
  the reasons in step 5, and the runner tries again on each pass. Run
  `hive review merge 7 --by owner --project my-app` to read the reason,
  such as *pull request #7 has failing checks: test; a red check is never
  merged*.
- **The pull request has conflicts.** The default branch moved after the
  branch was pushed. When it merges in cleanly, the runner merges it into
  the pull request's branch and pushes. When it does not, the task gets a
  message that names the files, which its lead reads. A pull request with
  no task and no session gets no message: resolve it on GitHub or close it.
- **The reviewer keeps asking for changes.** Each *changes* verdict queues
  one follow-up job and a new review. To end it yourself, merge with
  `--by owner` (accepted after a *changes* verdict too) or close it.
- **The default branch is red.** When GitHub's checks fail on its head,
  the runner posts a finding for you on the board and, while
  `pipeline.watch` is `fix` (the default), queues one fix job. When the
  project's own check fails on it, a job ends with *the base ... was
  already red before this job* and delivers nothing. Give a task that makes
  the check pass, or merge a fix yourself.

## Read more

- [Delivery](../landing.md): what is delivered and when, why a branch is
  held, the `hive land` commands, and how the runner watches the
  [checks GitHub runs](../landing.md#pipelines-what-a-push-set-running-on-github).
- [The reviewer](../review.md): the review job, the reasons that hand a
  pull request to you, and every `hive review` command.
- [Answer what waits for you](what-needs-you.md): the rest of your round.
