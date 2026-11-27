# The reviewer: a job reads every pull request

Every finished branch becomes a pull request on the project's upstream
([Delivery](landing.md)); this page is about who merges it. A `review` job
reads each pull request Hive opens - the diff, the project's check on the
branch, the conventions the README and the ADRs state - and merges it when
the change is one the agents may decide on, asks for changes when it is not
ready, and hands it to you when the complexity rule says a person should
look. You see the ones handed to you in the hub under **Needs your review**,
with the reviewer's reasons and a **Merge** button (on the task, or on the
session, that the pull request belongs to), on the project's board as a
question marked for you, or on GitHub like any other pull request.

A pull request's way through the review, with the event each step records:

```text
                ┌──────────────────────────┐
┌─────────────→ │ pull request Hive opened │
│               └────────────┬─────────────┘
│                            │ review.queued
│                            ↓
│               ┌──────────────────────────┐
│               │ a review job             │
│               │ reads the diff           │
│               └────────────┬─────────────┘
│         ┌──────────────────┼──────────────────────────────┐
│         │ changes          │ merge                        │ owner
│         ↓                  ↓                              ↓
│ ┌────────────────┐ ┌───────────────┐   Merge    ┌───────────────────┐
│ │ follow-up job  │ │ merged        │ ←───────── │ Needs your review │
│ │ review.changes │ │ review.merged │ review.ask │ review.owner      │
│ └───────┬────────┘ └───────┬───────┘            └───────────────────┘
│         │ land.pr          │ next delivery pass
└─────────┘                  ↓
                  land.merged, and the local
                  default branch follows
```

**Merge** is the button on the task or the session the pull request belongs
to, and [Your part](#your-part) says how you merge one that has no button.

An agent reviews because the owner cannot read 260 pull requests a week: of
the branches one machine landed in seven days, 22 (8%) were the owner's
tasks and 238 were the fleet's own work - suggestion jobs, security updates,
lead and schedule work, fixes, research, handoffs - and the old flow merged
every one of them into the local default branch with nobody reading the
diff. The reviewer reads every diff, and the rule below decides which of
them you read too.

## The review job

The maintenance pass has a stage for it, `pull request reviews`, after the
delivery stage. For each project it honours the Merge buttons you
pressed since the last pass ([Your part](#your-part)), records review jobs
that ended without a verdict ([When the review
fails](#when-the-review-fails)), and then, when no review job of the project
is queued or running, queues one for the oldest pull request that waits for
a verdict: one Hive opened (its newest record is a `land.pr`, so a pull
request pushed anew after a verdict is read afresh) and that GitHub still
shows open. At most one review job of a project is queued or running at a
time, because a merge moves the base the next pull request is read against;
reviews of one project go one after another, oldest pull request first.

The job is kind `review`, source `review:<slug>:<number>`, title `Review PR
#<n>: <title>`, recorded as `review.queued` with the pull request, the head
commit (`sha`) and the complexity reasons already computed from `gh pr view`
(the files and the head branch; size is not one). It leases with the owner's own work - the
express lane, ahead of the fleet's improve and research jobs - because a
pull request that waits is work the owner is waiting on. The reasons are
computed again by every verdict command, so the brief and the command
agree.

The job runs like any other ([Runner](runner.md)): a clone of the project at
its default branch, the worker's harness, the project's recipe, the run's
time budget (`minutes_per_job`; the brief gives the reading twenty minutes
and says a job that ends without a verdict counts as a failed review). Its
brief names the pull request, what it carries (the work of a task, the
branch of a job, an experiment's branch, a proposed branch), the project's
check command, and the procedure in order: `hive review show <n>`
for the title, body, files and reasons; `hive review checkout <n>`, which
puts the clone on `review/pr-<n>` at the pull request's head - fetched from
the project's local repository, which holds every branch Hive pushed, or
from the upstream's `refs/pull/<n>/head` when the local branch is not at the
head GitHub has (a branch moved on the forge); run the project's check, since
a red check is never merged; `hive review diff <n>` and read the whole diff
as a reviewer - does it do what the title and body say, is it correct, is it
tested, does it touch anything the body does not explain, does it break a
convention the README or an ADR states; exactly one verdict command; and
`hive job finish $HIVE_JOB done --result 'SUMMARY: reviewed #<n>:
<verdict> - <one line>'` with no `--branch`, because a review leaves no
branch. The run that carries a review lands nothing either: once the clone
stands on the pull request's head, the commits past the run's base are the
author's, signed by the author's run key, and the runner counts them as none
of this run's rather than refusing them at the identity check (eighteen
review runs failed that way on 2026-11-04, after reviews that had merged
their pull request). The reviewer never fixes the code itself (a fix is a *changes* note,
and the follow-up job makes it), never calls `gh` or pushes - `hive review`
talks to GitHub for it - and never loosens a check to make it pass. A review
job leaves no branch, so the delivery has nothing to push for it.

Where no reason holds, the verdict is one of three:

- **Merge** - `hive review merge <n>`: the diff does what the title says,
  the check passes on the branch, the change is safe and in scope. The
  command reads the pull request on GitHub first (`gh pr view`) and refuses
  to merge - without a verdict, so the job ends with the reason - a head
  that moved since the review was queued (*the pull request moved since it
  was reviewed: reviewed X, now Y; it is reviewed again*), a draft, a
  failing check (*has failing checks*) or one still running (*has checks
  pending*), and a merge GitHub would block (`mergeStateStatus` `DIRTY`,
  `BLOCKED` or `BEHIND`, named). A pull request with no check reported at
  all merges; when the checkout has workflows under `.github/workflows`
  and `pipeline.watch` is on, `review.merged` says so (`no_checks`). The
  same gate stands in front of the hub's Merge button. Then it merges on
  the forge (`gh pr merge --merge --delete-branch`, as the `gh` account
  that works for this repository; without `--delete-branch` while another
  pull request still open on the forge is based on its head - whatever its
  own review said, a "changes" verdict or a hand-off to you included -
  since GitHub would close that one, and delivery deletes the branch once
  it has moved it) and records
  `review.merged` with `by: <the agent>`, the job, `head_sha`, `checks`
  and `branch_kept_for` (the pull requests the branch was kept for); the delivery pass
  sees the merge on its next poll, records `land.merged`, closes the task
  and fast-forwards the local default branch.
- **Changes** - `hive review changes <n> --note "…"`: the change is right
  but not ready - a failing test, a missing case, a convention broken. The
  note goes on the pull request as a review that requests changes (as a
  comment when GitHub refuses, which it does when the pull request was
  opened by the same account: `Changes requested by the Hive reviewer:`),
  a follow-up job is queued ([below](#the-follow-up-after-changes)), and a
  task's conversation gets the note.
- **Owner** - `hive review owner <n> --note "…"`: the reviewer read it and
  does not want to decide it alone, even though no rule fired - money, auth,
  data, dependencies, deploy, a surface no ADR covers, a conflict with the
  base, anything it is unsure of. The findings go on the pull request as a
  comment (`The Hive reviewer handed this pull request to the owner.` with
  the reasons and the review) and the pull request goes to you.

With `review.merge` at `ask` the first verdict is not available: every pull
request goes to you with the reviewer's findings, and the reviewer merges
nothing. A verdict is refused when the pull request is no longer open on the
forge (you merged or closed it while the review ran) or already has a
verdict since its last push: a review gives exactly one.

That rule binds the reviewer, not you. After a hand-over (`review.owner`,
or `review.failed` when the reviewer failed twice) your verdict stays open:
`hive review merge <n> --by owner` merges it, `hive review changes <n>
--by owner --note "…"` asks for the changes and queues the follow-up job
(source `pr-fix:<project>:<n>`, [below](#the-follow-up-after-changes)) just
as a reviewer's *changes* does, and closing it on GitHub ends it when the
landing poll records `land.pr-closed`. A reviewer that tries a second verdict
after the hand-over is still refused.

A review that cannot check the pull request out (`hive review checkout`
fails: the head is not in the project's repository and the upstream refuses
or cannot be reached) is not the owner's problem. Run inside its own review
job, the checkout records that review's verdict itself, `review.again` with
`checkout: true` in its detail, and a fresh review is queued; it does not
count as one of the reviewer's failed attempts, and the reviewer gives no
other verdict for that run.

## The complexity rule

Before the review job runs, the daemon reads the pull request's paths and
lists the reasons a person should decide it. Any one reason means the brief
says *You may NOT merge this pull request*: the reviewer still reads the
diff and leaves its findings, but ends with `hive review owner` rather
than a merge, and `hive review merge` refuses it. The reasons are in the
brief, in `review.queued`, in `hive review list` and `show`, and in the
hub's **Needs your review** row, which a pull request enters the moment its
review is queued with a reason - you need not wait for the reading to merge
it, though the reading is what you get for waiting.

The rule as a picture, with the four reasons the daemon checks before it queues
the review (the fifth in the table, `the reviewer failed twice`, comes from the
attempts, not from the pull request):

```text
                          ┌──────────────────┐
                          │ the daemon reads │
                          │ the pull request │
                          └────────┬─────────┘
        ┌─────────────────┬────────┴────────┬─────────────────┐
        ↓                 ↓                 ↓                 ↓
┌───────────────┐ ┌───────────────┐ ┌───────────────┐ ┌───────────────┐
│ review.merge  │ │ touches a     │ │ UI files and  │ │ head branch   │
│ is ask        │ │ guarded path  │ │ no report     │ │ not Hive's    │
└───────┬───────┘ └───────┬───────┘ └───────┬───────┘ └───────┬───────┘
        └─────────────────┴────────┬────────┴─────────────────┘
                 ┌─────────────────┴─────────────────┐
                 │ no reason holds                   │ any one holds
                 ↓                                   ↓
┌─────────────────────────────────┐ ┌─────────────────────────────────┐
│ the reviewer's verdict:         │ │ Needs your review:              │
│ merge, changes or owner         │ │ only you merge it               │
└─────────────────────────────────┘ └─────────────────────────────────┘
```

| Reason (as the reviewer and the hub word it) | Why |
| --- | --- |
| `the owner asked to merge every pull request` | `review.merge` is `ask` |
| `touches <path>` | a guarded path: the files the landing refused to agents (`docs/evolution/constitution.md`, `docs/evolution/evaluators.md`, `docs/evolution/STOP`, `.github/workflows/`) and, for a branch under `hive/` (or `pulse/`, the prefix from before the rename), the owner's paths too (`docs/decisions/`, `.github/`, `.hive/agents/`, `.hive/roles/`, `.hive/kinds/`, and the same three folders under `.pulse/`); `.env*` files other than `.example` and `.sample`; anything whose name contains `secret`, `credential`, `password`, `.pem`, `id_rsa` or `id_ed25519`; the other forges' CI files (`.gitlab-ci.yml`, `.travis.yml`, `azure-pipelines.yml`, `Jenkinsfile`, `bitbucket-pipelines.yml`, `.circleci/`). An agent's merge must never move these on its own |
| `opened by a human/other tool (branch <head>)` | the head branch is neither under `hive/` nor an experiment's `evolve/<id>`: a branch you proposed with `hive land --branch`, or one a person or another tool pushed. The reviewer reviews it - findings, a drafted review - and never merges it. An experiment's branch keeps its `evolve/<id>` name on the forge and counts as Hive's own. So does a head under `pulse/`: a pull request Hive opened before the rename is still Hive's |
| `changes N UI files with no before/after report (...; landing.ui_report)` | the change touches a screen (a page, a view, a stylesheet) and the folder of the job, the task's jobs or the session that made it holds no `report.html` showing a change before and after, while `landing.ui_report` is `require` for the project ([Before/after reports](reports.md)). It names work the agent left undone rather than the shape of the change: a review queued once the report shows the change (a later commit on the pull request) no longer has it |
| `the reviewer failed twice` | two review jobs of the same head ended without a verdict ([When the review fails](#when-the-review-fails)); this one is not read from the diff but from the attempts |

A pull request whose only reason is `review.merge=ask` is the common case
when you want to approve everything yourself; the others are the shape of a
change, not its content, so the reviewer's findings beside the diff are what
you read.

Size is not a reason. A pull request over `review.max_files` files or
`review.max_lines` lines (added and deleted together, generated paths left
out: anything under `web/hub/out/`, lockfiles - `*.lock`, `Cargo.lock`,
`package-lock.json` - minified files with `.min.` in the name, and any path
with a `dist`, `out` or `target` segment), or one that removes more than it
adds (over 200 lines deleted and more than twice as many as added), gets a
*Read it with extra care* line in its brief and in `hive review show`
instead: `<F> files / <L> lines changed (the usual reading is 20 files /
600 lines)`, `removes more than it adds (<D> lines removed, <A> added)`.
The reviewer goes file by file and merges when every file is what the title
and body say and the check is green; a file the body does not account for,
or anything it cannot judge, goes to you through `hive review owner` with
what it found, as for any other pull request. Until 2026-11-04 size was a
reason, and the two pull requests it handed over that day - a blog post
with its data, 81 dictionary entries - carried the reviewer's own "fine to
merge" and waited a day for a click; a large change the reviewer has read
and can account for is the reviewer's to merge.

## The verdict commands

The `hive review` commands are the reviewer's and yours alike; an agent's
run uses them inside its clone, and you can use them from a terminal with
`--project S` (a job clone has `HIVE_PROJECT`; without either, the checkout
the command runs in names the project). Every one takes the pull request
number (`7` or `#7`); `--json` gives the same as data.

| Command | What it does |
| --- | --- |
| `hive review list [--project S]` | the pull requests Hive opened that wait for a verdict, every project's when no project is given: `<project> #<n>  <title>  [task T\|job J\|experiment\|proposed]  waiting\|queued\|running\|failed (<job>)  for the owner: <reasons>` and the url on the next line; `No open Hive pull requests.` when there are none |
| `hive review show <n>` | one pull request from the forge: title and url, `<head> -> <base>  by <author>  <state>  +<added> -<deleted>`, `Kind: Hive task T` (or job, experiment, proposed; `not a Hive pull request` for one Hive did not open), either `May be merged by the reviewer when correct, safe, in scope and green.` (followed by `Read with extra care: …` for a large or mostly-deleting change) or `For the owner (do not merge):` with the reasons, the files with their counts, and the body |
| `hive review diff <n>` | the diff against the base, from the forge |
| `hive review checkout <n>` | inside a job clone: fetches the pull request's head onto `review/pr-<n>` from the project's local repository (its branch, or its remote-tracking ref `refs/remotes/<remote>/<head>` when that holds the head GitHub has, as after a refresh), or from the upstream's `refs/pull/<n>/head` when neither is at the head GitHub has, checks it out and prints `review/pr-<n> at <sha> (<head>)`; refused in the project's own checkout |
| `hive review merge <n> [--by owner]` | merges on the forge (`gh pr merge --merge --delete-branch`; the branch is kept while another open pull request is based on it) as the account that works for the repository and prints `Merged #<n> (<url>) as <by>`; refuses, whoever asks, a head that moved since the review was queued, a draft, a failing or pending check and a merge GitHub blocks (`mergeStateStatus` `DIRTY`, `BLOCKED`, `BEHIND`), with the reason and no `review.failed`; refuses when reasons exist unless `--by owner` (`--by` takes no other value), which is what the hub's **Merge** button comes to. `--by owner` is the answer to a hand-off, so it is accepted after `review.owner` and after `review.changes` as well as before any verdict; a merged pull request takes no second one. Records `review.merged`, or `review.failed` when `gh` could not merge; the reviewer's merge (not `--by owner`) that meets only running checks, or a 5xx, internal error, rate limit or dropped connection from GitHub (on the merge or on the read before it), keeps the verdict as `review.merge-wait` and the daemon merges it later ([When an approved merge waits](#when-an-approved-merge-waits)) |
| `hive review changes <n> --note "…" [--by owner]` | asks for the changes on GitHub, queues the follow-up job and records `review.changes`; `--note` is required, since it becomes the follow-up's brief; `--by owner` (`--by` takes no other value) is your answer to a hand-over, accepted after `review.owner` and `review.failed` as well as before any verdict; run from a terminal outside any job (no `HIVE_JOB`) it is yours without the flag |
| `hive review owner <n> --note "…"` | comments the findings on the pull request and hands it to you: `review.owner`, a question on the project's board, a message on the task; `--note` is required |

The forge calls go through `gh` with the account that works for the
repository ([Polling and bookkeeping](landing.md#polling-and-bookkeeping)),
so a machine with several logins needs nothing set; no token is written
anywhere.

## The follow-up after changes

A *changes* verdict does not leave the pull request to wait. `hive review
changes` queues the work that answers the note, titled `Address the review
of PR #<n>: <title>`:

- A **task's** pull request gets a `task` job with source `task:<id>`, like
  any job of the task, so its branch joins the task's pull request the way
  every finished job's branch does - the task's branch is built from the
  default branch plus every job branch and pushed again ([What is delivered
  and when](landing.md#what-is-delivered-and-when)).
- Any **other** pull request gets an `improve` job with source
  `pr-fix:<slug>:<n>` (one at a time per pull request) whose branch, when it
  finishes, is pushed onto the pull request's head - the same pull request,
  more commits, a `land.pr` with `fix` naming the job - rather than becoming
  a pull request of its own. The branch must descend from the pull request's
  head; one that does not is held with the head to rebase onto
  ([When delivery fails](landing.md#when-delivery-fails)).

Both briefs start with `hive review checkout <n>`, so the worker begins from
the branch as reviewed, not from the default branch, then carry the pull
request, the reviewer's note and the rule: make exactly these fixes on top
of the head, run the project's check, commit, do not rewrite the pull
request's history and do not widen its scope - the reviewer reads the new
commits next. The push of the follow-up is a new `land.pr`, so the pull
request is read afresh and a new review job queued for it. The hub shows
*Changes requested on PR #n: <note>. Follow-up <job> is queued.* on the
task's delivery line until then, and the task's conversation gets the note
with the follow-up's id; the verdict is `review.changes` with the job id in
`follow_up`. A follow-up that fails is retried as any job is
([Recovery](recovery.md)).

## Your part

A pull request handed to you is **Needs your review** in the hub: on the
task's delivery line (*Needs your review: PR #n — <reasons>. <the review>*),
and at the top of the Sessions page's list in a group of its own
([Sessions](console.md#sessions)), under the card of the task or the session
it belongs to, with a link to the pull request and a **Merge** button. A pull
request that belongs to neither (a job's own, or a branch proposed with
`hive land --branch`) has no button in the hub: merge it with
`hive review merge <n> --by owner` or on GitHub. The button asks once
(*Merge PR #n into <base>? The daemon merges it on GitHub as you.*) and
records `review.ask {action: merge}`; the hub merges nothing itself. The
next maintenance pass merges the pull request as the owner, whatever the
rule says, and records `review.merged` with `by: owner`; the line reads
*Merging PR #n…* until then and *Merged* after the delivery poll. The
forge's own gate is not bypassed: a failing or still running check, a
draft, a merge GitHub blocks or a head that moved since the hub showed it
is refused (the daemon log says *could not be merged for the owner:
<reason>*), and the ask stands while the head stays the same - each pass
tries again until a running check finishes or the pull request is settled
on GitHub. The ask is for the head the hub showed when you pressed it, and
it stores that commit (`review.ask`'s `sha`): once Hive holds another head
- a push, a fix job's commits, the base merged in to cure a conflict, a
head found moved while a merge waited, a review queued at it - the ask
lapses, even when the new head was recorded a moment before the press
reached the daemon. So a pull request that needs a cure first (a conflict,
a red check fixed by a new commit) is never merged by the press that came
before the cure: the new head is reviewed on its own, and handed to you
again if it is still yours; press Merge again to take it. You are told
once, where you pressed - a message on the task's conversation, or a note
in the chat's transcript for a chat's pull request: *Pull request #n was
not merged: your Merge was for commit <sha>, and <why it moved> - it is at
<sha> now.* The daemon log says *not merged for the owner: Merge was
pressed at <sha>, the pull request is at <sha> now (<why>)* once. The same
hand-over is a question on the project's board, marked for you, on the
`review` thread - *Merge pull request #n: <title>?* with the reasons, the
review and the ways to merge it - and a message on the task's conversation
when it is a task's pull request.

You can equally merge or close the pull request on GitHub, or run `hive
review merge <n> --by owner --project <slug>` yourself: the delivery pass
notices a merge or a close on its next poll and finishes the bookkeeping the
same way ([Polling and bookkeeping](landing.md#polling-and-bookkeeping)). A
pull request you close unmerged sends a task back to its lead with the pull
request's words; a job's branch stays on the remote. A Merge pressed on a
pull request somebody merged or closed meanwhile asks for nothing.

To see every pull request before it merges, run `hive settings set
review.merge ask`: the reviewer still reads each one and leaves its findings, so what you
get is a reviewed pull request and a button, not a bare diff.
`review.max_files` and `review.max_lines` set where the brief starts asking
for the file-by-file reading ([The complexity rule](#the-complexity-rule));
they do not hand anything to you. The guarded paths and the human-branch
reason cannot be turned off. The hub's Settings page has the three under
**Reviewing pull requests**.

The reviewer reads the pull requests Hive opened and no others. A branch of
yours gets the same reading when you propose it (`hive land --project S
--branch B`, or a conversation's **Open PR for review**): the pull request is
reviewed, and since its head is not under `hive/` the verdict is yours. A
pull request opened straight on GitHub is not queued; `hive review show <n>
--project S` and `hive review diff <n> --project S` read it all the same,
and `hive review merge <n> --by owner` merges it.

## The review lead

The built-in `review` role ([Role leads](board.md#role-leads-hive-lead)) is
the hourly look over everything open, where the review job is the reading of
one pull request: `hive lead add --project S --role review` seats it. Each
sitting starts from `hive review list --project S` - every pull request
Hive opened that has no verdict yet, with the reasons the rule found and the
review job each one has or waits for - and covers what slipped: a review that
failed twice, a pull request whose job was retired, a pull request from
outside (`for the owner: opened by a human/other tool`). For each, oldest
first, it reads the pull request the way the review job does (`show`,
`checkout` in its clone, the project's check, `diff`) and gives exactly one
verdict with the same commands, never `gh pr merge`, `gh pr review` or `gh
pr checkout` by hand, never a push to a pull request's branch, never a
force; it says in one message what it merged, what it sent back, what it
handed to you and why. The daemon fast-forwards the local default branch
after each merge and marks the task done, so the lead need not. Hive never
suggests this seat on its own (the review job is what every pull request
gets); the lead is for a project with enough pull requests from outside, or
enough failed reviews, to want a reader of the whole list every hour.

## When the review fails

A review job that ends without a verdict - it failed, was cancelled, or
finished without running a verdict command - is recorded by the next pass as
`review.failed` (`review job <id> ended <status> without a verdict`, with the
job's result when it left one), and one more review job is queued for the
same head. When two review jobs at the same head have ended without a
verdict, the pull request is handed to you with the reason `the reviewer
failed twice` and the note *Two review jobs of pull request #n ended without
a verdict; the pull request waits for you.*; no further review job is queued
for that head until the pull request is pushed anew. The hub reads *The
review of PR #n failed: <error>* with the Merge button after the first
failure and *In review* again while the second job runs.

A `gh` call that fails inside a verdict command - the merge, the review that
requests changes, the comment that hands over - is `review.failed` with the
error, and the command exits with it; the pull request stays open for the
next attempt or for you on GitHub. A pull request nobody can read (`gh pr
view` fails for every account) is not queued; the daemon log says `review
<slug>: #n not queued: <why>` each pass, and a push that cannot be made at
all is the delivery's own `land.pr-held` ([When delivery
fails](landing.md#when-delivery-fails)).

## When an approved merge waits

The reviewer's `hive review merge <n>` is a verdict, and a forge that cannot
take it yet for a reason that passes does not undo it. Two cases keep the
verdict and record `review.merge-wait` instead of `review.failed`; the
command exits 0 with *Merge of #n waits: …* and the job finishes:

- **A check still runs.** Nothing failed yet (`statusCheckRollup` has only
  queued or running checks besides the green ones). Every pass reads the
  pull request again; it is merged as soon as the checks pass.
- **GitHub failed the merge for a passing reason.** `gh pr merge` - or the
  `gh pr view` that reads the pull request before it - answered 5xx, an
  internal error (*Something went wrong*), a rate limit (429, or a 403 that
  says *rate limit*), *Base branch was modified*, lost its connection
  (*error connecting*, *connection refused*, *connection reset*, *network
  is unreachable*, *no route to host*, *broken pipe*, `EOF`) or did not
  finish in time. The merge is tried again on the recovery
  back-off - 5, 15, 45 and 135 minutes after each failure, then every 6
  hours - with the same gate as the first time.

Neither queues another review job or asks you. Every retry reads the pull
request again and judges it afresh: your rule first (`review.merge` set to
`ask` while it waited, a guarded path, a missing report hands it to you, as
at the verdict), then the forge's gate. A head that moved on the forge is
never merged: `review.again` puts the pull request back at *open* and the
new head is reviewed. A pull request merged or closed on GitHub meanwhile
ends the wait when the landing poll records it. A failed read waits only
when it passes; a 401, 403 or 404 hands the pull request to you. Anything
else at the retry - a check that failed, a conflict (`DIRTY`), a rule of the
repository (`BLOCKED`, `BEHIND`), a draft, or a merge error that does not
pass (*not mergeable*, 4xx) - hands the pull request to you as
`review.owner`, its reason the refusal and its note *<reviewer> approved pull
request #n and its merge waited; it cannot go through now: …*. At the verdict
itself those are refused or failed exactly as before.

**The wait is bounded.** After `review.merge_wait_tries` failures of the
forge (default 5), or `review.merge_wait_hours` hours since the verdict
(default 6) - for running checks as for the forge - the wait ends: the pull
request is handed to you (`review.owner`, its reason the bound and the
forge's last answer) and the project's lead is told once, by a finding on
the `runner/review` thread that mentions it and quotes that answer. A forge
wait that is already past its hours when a pass reaches it (the machine
slept, the daemon was down) but has tries left makes one last try first,
as a checks wait at its bound merges when its checks passed: merged when
the forge takes it, handed to you as above when it fails again. That try is
the merge's: when reading the pull request fails for a passing reason on
that pass, the failure is counted and the last try waits for the next step
of the back-off (`review.merge-wait` with `last: true`) instead of ending
the wait with no merge tried - unless that failure used up the tries. A pull
request stacked on the one that waited still waits below it, and is
reviewed when the one below merges.

Your own Merge (`hive review merge <n> --by owner`, the hub's button) never
waits: it is refused or fails with the reason. The hub offers it while a
reviewer's merge waits (the task's sheet and the pull request's row show
**Merge**), and the daemon makes it as yours on its next pass, ahead of the
retry.

What is waited for is one line: the Sessions page shows it on the pull
request's row (*Approved, merge waits · The merge waits for the checks still
running (ci/build)* or *· GitHub refused the merge (HTTP 502: Bad Gateway);
retried at <time> (attempt 2)*), a task's sheet reads *Approved: PR #n — …*,
and the daemon log prints `review <slug>: #n approved; …` once when the wait
starts or changes and `review <slug>: #n merged after waiting (…)` when it
ends.

## Events

Every event's subject is the project slug; `hive watch --project X --action
'review.*'` streams them (`--since 0 --detail` replays them), and the hub
lists them at `GET /api/v1/events?project=X&action=review.*`.

| Event | When | Detail |
| --- | --- | --- |
| `review.queued` | a review job is queued for a pull request | `project`, `number`, `url`, `job`, `kind` (`task`, `job`, `experiment`, `proposed`), `reasons`, `sha` (the head reviewed), `task` for a task's |
| `review.merged` | the pull request was merged by `hive review merge` or for a Merge you pressed | `project`, `number`, `url`, `by` (the reviewing agent, or `owner`), `job` when a job ran the command, `head_sha` (the head merged), `checks` (how many reported), `no_checks` with a `note` when none did although the project has workflows |
| `review.changes` | the reviewer asked for changes | `project`, `number`, `url`, `note`, `follow_up` (the job queued) |
| `review.owner` | the pull request was handed to you | `project`, `number`, `url`, `reasons`, `note` |
| `review.failed` | a review job ended without a verdict, or a `gh` call of a verdict failed for a reason that does not pass | `project`, `number`, `url`, `error`, `job` when it was the job's ending |
| `review.ask` | you pressed **Merge** in the hub | `project`, `number`, `action: merge`, `task` when pressed on a task |
| `review.merge-wait` | the reviewer said merge and the merge waits ([When an approved merge waits](#when-an-approved-merge-waits)); recorded again at each failed retry | `project`, `number`, `url`, `by`, `reviewer`, `head_sha`, `waiting` (`checks` or `forge`), `error` (the forge's last answer), `attempt` (failures of the forge so far), `due` (the next try; empty while checks run), `since` (when the wait began), `note` (the one line shown) |
| `review.again` | a waiting merge found the head moved on the forge: nothing merged, the pull request is open for review again | `project`, `number`, `url`, `head_sha` (the new head), `was` (the head approved), `note` |

The delivery's own events - `land.pr`, `land.merged`, `land.pr-closed`,
`land.pr-held` - are listed with [Delivery](landing.md#events); a merge
reaches the store twice, as the reviewer's `review.merged` and the poll's
`land.merged`, because a merge made on GitHub by hand has only the second.
The hub reads both sets to place a pull request: `land.pr` puts it at *open*,
`review.queued` at *in review* (or *needs your review* when it carries
reasons), `review.owner` at *needs your review*, `review.ask` at *merging*,
`review.changes` at *changes requested*, `review.failed` at *review failed*,
`review.merge-wait` at *approved, merge waits* (with its note and a
**Merge** for you), `review.again` at *open*, and `review.merged`, `land.merged` or `land.pr-closed` settle it
([Sessions](console.md#sessions)).

## Settings

| Setting | Default | Meaning |
| --- | --- | --- |
| `review.merge` | `auto` | `auto`: the reviewer merges a pull request no reason holds back; `ask`: every pull request is handed to you with the reviewer's findings, and the reviewer merges nothing (the hub's label: *Who merges a reviewed pull request*) |
| `review.max_files` | `20` | a pull request changing more files than this is read file by file; the reviewer still merges one whose every file the title and body explain (size alone hands nothing to you); generated paths are not counted |
| `review.max_lines` | `600` | the same for a pull request adding and deleting more lines than this together; generated paths are not counted |
| `review.merge_wait_tries` | `5` | how many times the forge may fail a reviewer's approved merge (refuse it, or not be read, for a passing reason) before the wait ends and the pull request goes to you ([When an approved merge waits](#when-an-approved-merge-waits)) |
| `review.merge_wait_hours` | `6` | how many hours an approved merge may wait, for its checks or for the forge, before the wait ends the same way |

`hive settings set review.merge ask` sets one; the rest of the settings are in
[self-improvement.md](self-improvement.md#evolution-runner-caps). An evolve
variant cannot set any of them: they are off the variant allowlist, so
the loop changes how an agent reviews, never what it may merge.
