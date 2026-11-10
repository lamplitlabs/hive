# Delivery: every branch becomes a pull request

Workers commit in their own clones and never push; the runner copies each
job's commits to a `pulse/<agent>/<job>` branch in your checkout. From there
the daemon delivers: every finished branch is pushed to the project's upstream
and becomes a pull request against the upstream default branch, a `review` job
reads it and merges it or hands it to you ([The reviewer](review.md)), and the
daemon fast-forwards the local default branch from the upstream on every pass.
Nothing merges into the local default branch any more: the forge holds the
merge, the review and the pipeline, and the checkout follows it. (A project
with no upstream at all - no remote for its default branch and no `origin` -
has no forge to hold them, so its finished branches land locally as before;
the daemon says so in its delivery line and `pulse land --dry-run` says so
per project.)

Before this (ADR-024) the daemon merged each finished branch into the local
default branch itself - verify command first, fast-forward or a `Land …` merge
commit - and `landing.push` said whether the result went further. [What
changed and why](#what-changed-and-why) has the measured reasons; the rest of
this page describes the flow as it is now. The page keeps its file name so
links to it stay valid.

## What is delivered and when

A delivery pass runs inside the daemon's maintenance pass (its stage is named
`delivery` in the pass summary) - every tick, on its own thread, so a slow
`gh` call never holds dispatch, heartbeats or STOP - and by hand as `pulse
land`. It delivers four kinds of work:

| Kind | What | Branch | When |
| --- | --- | --- | --- |
| `task` | the owner's task: every job the lead cut for it, as one pull request | `pulse/task-<id>`, built in a scratch worktree (`.git/pulse-task-pr/<task>`) from the default branch plus every finished job branch of the task, each merged with its own merge commit in finished order | once no job of the task is queued, running or blocked and at least one finished job has a branch that no pull request carries yet; the branch is rebuilt and pushed again, and the same pull request updated, when a later job of the task finishes |
| `job` | one finished job that belongs to no task: suggestion work, a security update, a lead's or a schedule's job, a pipeline or setup fix, research, a handoff | `pulse/<agent>/<job>`, pushed as the runner wrote it | when the job is `done` with a branch; a branch that moves while its pull request is open is pushed again and the pull request updated |
| `experiment` | a kept evolve experiment | the experiment's branch | when the measurement says keep ([Evolve](self-improvement.md#evolve)); the measurement summary is the pull request body |
| `proposed` | a branch proposed by hand, or a conversation's branch | the branch itself; `pulse/chat-<id>` for a conversation | `pulse land --project X --branch B` at once; a conversation branch once it holds commits the default branch lacks and its last commit is older than ten minutes, so an agent still at work does not get a pull request of half its change |

What is not delivered: a job's branch while another job of its task is still
queued or running (the task is one pull request, so the reviewer reads the
whole change once) - and a job counts as running while its run is, whatever
its status says, because the worker marks the job done itself when its
session ends and names the branch the runner creates only once the run's
verification has passed, up to forty minutes later (nine holds in one day
were a task "branch" that did not exist yet) - a job that ended without a commit (there is nothing to
deliver; the job is simply done), a failed run (its commits were never
verified; the job is retried as before, see [Recovery](recovery.md)), and a
`review` job, which changes nothing in the project and has no branch of its
own - and, while `landing.ui_report` is `require`, a conversation branch
whose change has a face and whose session left no before/after report: the
session is woken once with what to do, and its pull request opens when the
report shows the change, or an hour after the ask for the owner to merge
([Before/after reports](reports.md)). The first push of a branch is a plain `--force` (the branch is PULSE's
own); a push onto an open pull request is `--force-with-lease` against the
commit the last push left there, so a branch someone moved on the forge is
not overwritten. A pass opens at most ten pull requests per project
(GitHub's secondary rate limit is on creations; the rest wait a tick), and
reads every open one.

The measured share: of the 260 landings on one machine in the seven days
before this flow, 22 (8%) were task work and 238 were everything else -
suggestion jobs 120, security updates 39, lead idle work 21, role leads 17,
schedules 11, red-base fixes 7, pipeline fixes 7, research 6, handoffs 3,
others 7. PR #53 (the per-task pull request) covered the 8%; this flow covers
every finished branch.

## The pull request

A pull request is PULSE's words about the work before its diff, so the
reviewer and you read what was meant first. A task's is titled `<task id>:
<task title>`; its body names the task, lists the jobs with their branches
and says that the reviewer reviews it, that merging it closes the task and
that closing it unmerged sends the task back to its lead. A job's is titled
with the job's title; its body is the worker's result summary (cut at 4,000
characters) followed by the job's id, kind and source (`suggestion:…`,
`security:…`, `pipeline:…`) and its branch, so the reviewer knows what the
change set out to do. An experiment's carries the measurement: the experiment
and archive ids, the summary with the baseline and the variant scores, the
fixtures that moved and the verdict, and the job and branch; merging it
finalizes the experiment. A proposed branch's title is its last commit's
subject, and its body says who proposed it (a conversation by id, or
`pulse land --branch` by hand) and lists the commits it adds over the
upstream default branch.

The commits are the runner's: each carries its `Pulse-Agent`, `Pulse-Job` and
`Pulse-Executor` trailers and the run key's signature, and `pulse verify
<ref>` checks that they match the run that made it (ADR-022; `pulse signers`
lists the trusted keys). The `land.pr` event names the pull request (`number`,
`url`, `title`), the branches (`head`, `base`, `remote`), the commits (`from`,
the merge base with the default branch; `to`, the head pushed; `commits`, how
many between them), the `jobs` it delivers, whether the pass `opened` the pull
request or updated an open one, and the ids of the work: `task`, `job`,
`experiment` with `archive` and `run_id`, or `conversation`. A follow-up
pushed onto an open pull request after the reviewer asked for changes
([The follow-up after changes](review.md#the-follow-up-after-changes)) is a
`land.pr` with `opened` false and `fix` naming the follow-up job.

## Stacked pull requests

A project's pull requests form one stack. While a pull request of PULSE's
is open, the next one is built on it rather than beside it: a task's branch
is rebuilt from the head of the open pull request instead of the default
branch, a job's, an experiment's or a hand-proposed branch has that head
merged in (a merge commit on PULSE's own branch, in a scratch worktree),
and the pull request is opened with `--base` the open pull request's branch.
Two open pull requests therefore never hold changes to the same lines that
meet for the first time at merge time. The one a new pull request goes on is
the **top**: the stacking pull request opened last; its `land.pr` records
the branch as `base` and the pull request as `stack_on`. A conversation's
pull request (`pulse/chat-<id>`) is never stacked and never stacked on: the
hub keeps its branch checked out in a worktree of its own, so nothing is
merged into it and nothing is built on it. It targets the default branch
whatever is open, and the next pull request of another kind goes on the
task or job pull request below it. An experiment's pull request
(`evolve/<id>`, kind `experiment`, and any `evolve/` branch proposed by hand)
stays out of the stack the same way: the experiment was measured on the
default branch, so its branch goes out based on the default branch, never
on another open pull request, and nothing is built on it - a pull request
merged into a pull request strands its change when the one below is closed
or squashed ([Experiment pull requests](#experiment-pull-requests)). A
`pulse review`'s fix goes onto the pull request it fixes, wherever that
stands.

A worker may build on the stack itself: a follow-up task whose brief names
the sibling's branch and commit (TASK-078 on TASK-077's, 2026-11-05) puts
its clone on that commit and commits on top. The landing's identity check
(ADR-022) then answers only for the commits past the default branch as it
stands now and past PULSE's own `pulse/*` branches - never a conversation's
`pulse/chat-*`, which nothing is built on, and never the branch under check
itself. The sibling's commits passed that check when their own branch
landed, and carry their own run's signature for `pulse verify`; the run log
says `N of the M commit(s) are already on the project's own branches` and
the delivery stacks the pull request on the sibling's as it would any other.
Until 2026-11-05 the check judged every commit past the clone's base as this
run's, and refused the whole branch with `authored by quartz-14, not
cinder-15` after 24 minutes of finished work - also when the sibling's pull
request had merged while the run was on, since the base stays where the
clone started.

The stack is plain git branches and explicit bases, nothing a forge keeps
for PULSE. When the pull request below merges, the pass moves every pull
request that targeted its head onto its own base (`gh pr edit <n> --base`;
the default branch when the merged one was the bottom) and writes a fresh
`land.pr` for each with the new `base`, `stack_on` for the new one below or
absent, `opened` false and `retargeted_from` naming the old base; a forge
failure leaves no event and the next pass tries again. GitHub moves a pull
request itself when the branch below is deleted; PULSE does not count on it.
A pull request below that is closed unmerged leaves its commits in the one
above, and moving that one down would carry the rejected work into the
base through its merge; so the pass rebuilds it instead: the branch as the
forge has it is fetched, its own commits (what it has past the point it
left the closed branch; the merge that took the stack in goes with them)
are replayed onto the new base in a scratch worktree, force-pushed to the
same branch (`--force-with-lease` on the head fetched), and only then is
the pull request retargeted. A fresh `land.pr` records the new `base`,
`from`, `to` and `commits` with `rebuilt` true, `land.pr-rebuilt` names
what was dropped, and the origin is told once - a task in its conversation,
a conversation in its transcript. The project's check runs on the new head
and the reviewer reads it again. Each pull request stacked on a rebuilt one
is rebuilt the same way on its new tip, so the whole stack above loses the
commits. A replay that conflicts changes nothing: the pull request keeps
its base and head, a `land.pr-held` hold names the closed pull request and
the files, the origin is told what to do by hand, and the pull requests
above it are left alone. A merge conflict with the pull request
below - rather than with the default branch - goes where a conflict went
before: a task's branch to a rebase job whose brief names the pull request
it must go onto, a job's branch to a `land.pr-held` hold that names the
files and the pull request.

Merging a stack needs merge commits, which is what PULSE's reviewer and the
hub's Merge button use (`gh pr merge --merge`). A squash would land the
lower pull request as one new commit the upper one does not have, and the
upper would show the lower's changes again as its own. The reviewer takes
the stack bottom up: only a pull request whose base is the default branch
is queued for review (the log says `#8 stacked on #7: reviewed after #7
merges` for the rest); `pulse review merge 8`, and the Merge button, refuse
a stacked pull request and name the one that must merge first; `pulse
review list` shows `stacked on #7` on its line. Nothing you merge yourself
by hand needs a different order: merge the bottom and the pass moves the
rest down within a pass.

## Experiment pull requests

Each pass looks after an experiment's open pull request
(`crates/pulse-runner/src/delivery/experiments.rs`) on top of what any open
pull request gets:

- **Its change is on the default branch already** - landed by a squash, a
  cherry-pick, another pull request, or a merge the forge did not record as
  this one's: merging the head would change nothing (`git merge-tree` of the
  default branch and the head gives the default branch's own tree; after
  the default branch was squashed to a new root, from the base the pull
  request was delivered on). The pass closes it with a comment naming the
  commit that holds its change (`gh pr close <n> --comment`; the commit with
  the same patch when there is one, else the newest that touched its files),
  records `land.pr-closed` with `reason` `superseded` and `superseded_by`,
  moves any pull request built on it onto its base, and deletes its branch
  on the remote (`--force-with-lease` on the head it judged, so a push made
  since keeps it) and in the checkout.
- **Open three days with no push** (`STALE_DAYS`, from its newest
  `land.pr`): one based on the default branch that lacks the default
  branch's tip takes it in and is pushed (`land.pr-refreshed`, as for a
  conflict), which restarts the clock; one that does not merge cleanly, or
  that stands on another pull request, is closed with a comment saying why
  (`land.pr-closed`, `reason` `stale`). Its branch stays on the remote, and
  the experiment can be planned again on the default branch.

A forge or push failure leaves no event and is tried again next pass.

## Polling and bookkeeping

Each pass asks GitHub about every pull request PULSE opened and has no verdict
for yet (one `gh pr list --state all` per repository, `gh pr view <n>` for a
number the list missed; both read `state`, `mergedAt`, `url`, `headRefName`,
`headRefOid`, `mergeable`, `mergeStateStatus` and `statusCheckRollup`):

- **Merged** - `land.merged`, with `merged_at` from GitHub. The jobs' branches
  are cleared (nothing of the work waits any more; the hub's row leaves *In
  review*) and the local copies deleted - unless a worktree has one checked
  out, which the line says - a task is marked done (merging its pull request
  is what closes it, as the body says, and its delivery line reads *Merged*,
  [Tasks](console.md#tasks)), a kept experiment is finalized the way a local
  landing finalized it ([Evolve](self-improvement.md#evolve)), and the local
  default branch is fast-forwarded from the upstream at once rather than when
  the fetch memo runs out. Whether the reviewer merged it or you did, the
  event is the same; the reviewer's own record is `review.merged`
  ([The reviewer](review.md)). The pull requests stacked on it move onto its
  base ([Stacked pull requests](#stacked-pull-requests)). A conversation's
  pull request (`pulse/chat-<id>`) finishes the conversation as well: the
  pass says so on the bus (`chat.finish`, subject the conversation, with the
  text *pull request #N merged into main; this session is finished*) and the
  hub's wake loop ends the session there - after a turn in flight, never
  through it - and writes the text into the transcript; a hub that was down
  picks the event up from its cursor when it starts ([Sessions](console.md#sessions)).
  A session with a sub-session still at work - working, waiting for you, or
  stopped by a restart - stays open instead, with a note naming each one
  (*PULSE kept this session open: ... is not done*): the sub-session's report
  comes back to it, its agent's join may add commits to the branch, which go
  out as a new pull request, and its agent or you finish it.
- **Closed without merging** - `land.pr-closed`. The branches are cleared
  too; a task goes back to *in progress* with a message in its conversation
  that says the pull request was closed and that its lead reads why on the
  pull request and plans what comes next. A conversation gets a note in its
  transcript (*closed without merging; nothing is marked done; the branch
  stays on the remote; write here to carry on*) and stays as it was, its
  agent gets no turn; anything else is only the log line. The branch stays
  on the remote, so nothing is lost and nothing delivers it again; a new job
  of the task makes a new pull request. The pull requests stacked on it are
  rebuilt without its commits and moved onto its base
  ([Stacked pull requests](#stacked-pull-requests)).
- **Open** - where the review stands is the reviewer's record
  (`review.queued`, `review.owner`, `review.changes`), and the hub reads both
  ([Where the work stands](#where-the-work-stands-in-the-hub)). What the
  reviewer cannot fix - a head the base moved away from, checks that fail -
  is looked after here, once per head ([After the pull request
  opens](#after-the-pull-request-opens)).

The pass remembers what it delivered in the event log alone: the newest of
`land.pr`, `land.merged` and `land.pr-closed` for a task, job or branch says
whether a pull request is open, so a daemon restart loses nothing and `pulse
events --action 'land.*'` is the whole history.

`gh` is the GitHub CLI logged in on this machine; its login never reaches git.
A machine often has more than one login (`gh auth status` lists them), and `gh`
uses one of them for everything, so a fine-grained token that cannot see a
repository answered 404 to every call while the account beside it could see
them all. The daemon therefore tries every logged-in account when a call
comes back 401, 403 or 404 and remembers, per repository, the one that
answered, for as long as it runs; the list of logins itself is read from `gh
auth status` once every ten minutes. An error names the account and the
command, and no token is ever written to a log, an event or an error. The
same selection serves the pull requests, the reviewer's `pulse review`
commands and the pipeline watcher below; git's own push and fetch use the
machine's credentials or `push.credential`, never `gh`
([Credentials](#credentials)).

## After the pull request opens

An open pull request is not left to wait. The same poll reads what stands
between each one and its merge, and the pass acts on it once per head sha:
every follow-up event names the `head_sha` it was about, so a head is never
acted on twice, and a head GitHub has not judged yet (`mergeable` `UNKNOWN`,
or no sha from an older `gh`) waits for the next pass. Four outcomes:

- **Conflicting, and the base merges in cleanly** (`mergeable` `CONFLICTING`
  or `mergeStateStatus` `DIRTY`: the base - the default branch, or the pull
  request below when the pull request is stacked - moved after the head was
  pushed) - the pass fetches the head and the base, merges the base into the
  head in a scratch worktree of the checkout (`.git/pulse-pr-refresh/<head>`,
  removed whatever happens) and pushes the merge onto the pull request with
  `--force-with-lease` on the tip GitHub named. `land.pr-refreshed` records
  it (`head_sha` the old tip, `new_sha` the merge), and a `land.pr` naming
  the new tip follows, as a follow-up push writes one, so the next push onto
  the branch leases on what the pull request holds and the reviewer reads the
  new head. Nobody is told: there was nothing to decide.
- **Conflicting, and the merge conflicts** - nothing is pushed; `land.pr-conflict`
  records the paths left unmerged (`files`) and the origin is told: a task
  through a message in its conversation (its lead reads it and plans the fix;
  the message names the pull request, the sha, the paths and what to do), a
  conversation through a `chat.wake` on the bus with the same text (its agent
  gets a turn on it, as on any wake), a job or an experiment only through the
  daemon log line. No job is queued: the work itself is at odds with what
  landed, which is the agent's to resolve. A head that moved since GitHub
  was read, or a fetch or push that fails, leaves no record and is read
  again next pass.
- **Checks failing** (a `statusCheckRollup` entry concluded `FAILURE`,
  `CANCELLED`, `TIMED_OUT`, `ACTION_REQUIRED` or `STARTUP_FAILURE`, or a
  status in state `ERROR` or `FAILURE`) - `land.pr-checks` records the names
  (`checks`) and the origin is told the same way, once for this head: *checks
  are failing on pull request #N at <sha>: names; a fix on the branch is the
  next job on this work*. A new head is judged anew and told anew; a run
  still in progress, or green, is no news.
- **Anything else open** - nothing; where the review stands is the
  reviewer's ([The reviewer](review.md)).

A conflict GitHub reports before the pull request opens is the per-task
delivery's: a job branch that will not rebase onto the default branch gets a
rebase job (`land.rebase`, [When delivery fails](#when-delivery-fails)); the
follow-up here is for the base moving away after the pull request opened. A
GitHub call that fails leaves everything as it was - no event, no push, no
message - and the next pass reads again.

## The local default branch follows the upstream

Every pass fetches each active project's upstream (`origin/<default>`, or
whatever the local default branch tracks; a fetch is trusted for two minutes,
and a merge the pass just settled fetches at once) and fast-forwards the local
default branch to it. A clean checkout on the default branch is fast-forwarded
in place; a checkout on another branch, or detached, has its default branch
moved by ref (`git update-ref`, with a reflog message) and its working tree
left alone, so your branch, your edits and your stash stay as you left them;
the move is `land.fast-forward` in the log (`behind`, `from`, `to`, `how`:
`checkout` or `ref`) and a `landing fast-forwarded <slug>: main -> origin/main
(N commits)` line in the daemon log. The next worker clone starts from the
commit the forge has.

A dirty checkout on the default branch is stashed around the fast-forward
when `landing.auto_stash` is on (the default), as the old landing stashed
around a merge; with it off the branch is left alone, the daemon records
`land.sync-skipped` with what holds it back (at most once an hour per
project, so the log does not fill with the same line) and tries again on the
next pass - commit or stash your changes and the branch follows. A local
default branch that has diverged from the upstream - commits of its own that
the forge lacks, which only a `pulse land --branch B --local` or a merge by
hand makes now - is not merged for you: on the default branch the
fast-forward fails and `land.fast-forward` says so (`ok` false, `why`); on
another branch the ref is not moved. Push or reset the local commits and the
branch follows again.

Following the upstream is what keeps PULSE updating itself: the brain project
(PULSE on the machine that runs it) rebuilds and restarts the daemon when its
local default branch moves ([Services and self-update](runner.md#services-and-self-update)), and
with the merge on the forge, the fast-forward is how a merged change reaches
the running binary. The pass fast-forwards whether or not anything of PULSE's
merged, so a change you merged on GitHub by hand reaches the checkout within a
pass too.

## Events

Every event's subject is the project slug; `pulse watch --project X --action
'land.*'` streams them (`--since 0 --detail` replays them), and the hub lists
them at `GET /api/v1/events?project=X&action=land.*`.

| Event | When | Detail |
| --- | --- | --- |
| `land.pr` | a pull request opened, updated or moved onto another base | `project`, `kind` (`task`, `job`, `experiment`, `proposed`), `number`, `url`, `title`, `head`, `base`, `remote`, `from`, `to`, `commits`, `jobs`, `opened`; the ids: `task`, `job`, `experiment` with `archive` and `run_id`, `conversation`; `stack_on` (the pull request `base` is the head of) when it is stacked; `fix` (the follow-up job) when a follow-up was pushed onto it; `refreshed` true when the pass merged the base into the head; `retargeted_from` (the old base) when the pull request below merged or closed |
| `land.pr-refreshed` | an open pull request conflicted with its base and the base was merged into its head and pushed | `project`, `kind`, `number`, `head`, `head_sha` (the tip before), `new_sha` (the merge), `base` |
| `land.pr-conflict` | an open pull request conflicted with its base and the merge did not go cleanly; the origin was told | `project`, `kind`, `number`, `head`, `head_sha`, `base`, `files` (the paths left unmerged) |
| `land.pr-checks` | checks failed on an open pull request's head; the origin was told | `project`, `kind`, `number`, `head`, `head_sha`, `base`, `checks` (the failing names) |
| `land.merged` | the poll found the pull request merged | `project`, `kind`, `number`, `url`, `head`, `base`, `jobs`, `merged_at`, the same ids |
| `land.evidence` | the pass after a merge read which files the pull request changed: the merge commit on the default branch against its first parent, or the pull request's base against its head when the history no longer holds that merge; the Code delivered report counts source changes from it | `project`, `number`, `merged_seq` (the `land.merged` it completes), `sha`, `code_delivery`, `work` |
| `land.pr-closed` | the poll found it closed unmerged, or the pass closed an experiment's pull request | the same, without `merged_at`; when the pass closed it, `reason` (`superseded`: its change is on the default branch, in `superseded_by`; `stale`: no push for three days and it could not be brought up to date) and `head_sha` |
| `land.pr-rebuilt` | a pull request stacked on one closed unmerged was rebuilt on the new base without the closed one's commits, force-pushed and retargeted | `project`, `kind`, `number`, `url`, `head`, `old_base`, `base` (the new one), `from` (the head before), `to` (the new head), `commits` (its own), `dropped` (`number` of the closed pull request, `commits` it took with it); the matching `land.pr` carries `rebuilt` true and `retargeted_from`. A rebuild that conflicts is a `land.pr-held` with `closed`, `base` and `files` instead |
| `land.pr-held` | a branch could not be delivered this pass | `project`, `task`, `job` or `head` (with `conversation`, `experiment`, `archive` when they apply), `work` (the branch and commit it tried; a task's branches each with its commit, and the pull request they would stack on), `note` (why) |
| `land.rebase` | a task's job branch conflicted with the task's base and a job was queued to bring it over | `project`, `task`, `job`, `branch`, `sha`, `files` (the paths left unmerged), `follow_up` (the rebase job), `base` (the default branch, or the branch of the pull request the task's stacks on) |
| `land.fast-forward` | the local default branch moved to the upstream, or could not | `project`, `branch`, `upstream`, `behind`, `ok`; `from`, `to`, `how` (`checkout`, `ref`) when it moved; `why` when it could not |
| `land.sync-skipped` | the local default branch was left behind the upstream because the checkout is dirty and `landing.auto_stash` is off | `project`, `branch`, `upstream`, `behind`, `why`; at most once an hour per project |

The reviewer's events (`review.queued`, `review.merged`, `review.changes`,
`review.owner`, `review.failed`, `review.ask`) are listed with [the
reviewer](review.md#events). Older stores hold the events of the local flow -
`land.landed`, `land.batch`, `land.pushed`, `land.synced`, `land.held`,
`land.failed-verify`, `land.deferred`, `land.conflict` - as history; only a
project without an upstream writes `land.landed` now. The goal review counts
`land.landed` and `land.merged` alike ([Goal review](board.md#goal-review-how-far-the-work-has-moved-it)), and so does the
code-delivery report (the hub's Code delivered), which reads a pull
request's changed files from its `land.evidence`; a merge whose
`land.evidence` is not written yet shows as still being read, never as
zero. The loop report's landing counts still read `land.landed` only, so for
a project with an upstream they count nothing under this flow until they
are taught `land.merged`.

## When delivery fails

A branch the pass cannot deliver is held with `land.pr-held` and the reason in
`note`, shown on the task's conversation and on the hub's row, and tried again
only when the work changes (a new job of the task finishes, a branch of it
appears or moves to a new commit) or when you propose the branch by hand with `pulse land
--branch` - a hold is not retried blind every tick, because the reasons below
do not clear by themselves:

- **A task's branch does not merge onto the default branch**: the branch was
  cut before something else landed on the same lines, or two jobs of one task
  changed them. The first time, the pass does not hold the task: it queues a
  `task` job on the same task - "Rebase `<branch>` onto `<default>`: `<task
  title>`", with the conflicting files and the job's own account of its work
  in the brief - records `land.rebase`, tells the task's conversation, and the
  conflicting job keeps its result but loses the branch, so the task's pull
  request opens from the rebased one once that job finishes. A second
  conflict on the same task is held with the note naming the branch, the
  files and the rebase job that already ran: the work itself is at odds with
  what landed, and that is yours to settle.
- **The push was refused**: a credential that cannot be resolved
  ([Credentials](#credentials)), a branch protection, a network that is down.
  The note carries git's words with secrets removed.
- **`gh` could not open or read the pull request**: no account that can see
  the repository, a rate limit, GitHub down. The note names the account and
  the command that failed; the pass tries again next time.
- **The branch is gone**: a job whose branch no longer exists in the checkout
  is held once and forgets the branch, so it is not asked about again.
- **A follow-up is not built on the pull request**: a `pr-fix` job's branch
  that does not descend from the pull request's head is held with the head
  to rebase onto ([The follow-up after changes](review.md#the-follow-up-after-changes)).

A pull request closed on GitHub without a merge is not a failure of the
delivery: `land.pr-closed` records it, the task goes back to its lead with the
pull request's words, and the branch stays on the remote (above). A review
that could not be made - the review job ended twice without a verdict on the
same head - is the reviewer's `review.failed`, and the pull request waits for
you ([The reviewer](review.md#when-the-review-fails)). A checkout that cannot
follow the upstream is `land.fast-forward` with `ok` false or
`land.sync-skipped` (above).

## Where the work stands in the hub

A task's conversation shows one delivery line under its title, read from the
same events: *PR #n open, waiting for review*, then *PR #n is being reviewed*
while the review job runs, then one of *Needs your review: PR #n — <the
reviewer's reasons>* with a **Merge** button (the button records `review.ask`,
and the daemon merges as you on its next pass - the same as `pulse review
merge <n> --by owner`), *Changes requested on PR #n: <note>. Follow-up <job>
is queued.*, *Merging PR #n…*, *The review of PR #n failed* (with the button
too) and *Merged*. A finished step whose branch still waits for the task's
pull request reads *Waiting for its pull request*. The Chat page's list
groups the rows the same way: **Needs your review** at the top, then
**Working**, then **In review** for work that has a pull request and nobody
waiting on you, then the rest ([Sessions](console.md#sessions)). The old *Waiting to
land* and *Reviewing* groups are gone with the local landing they described.

## By hand

- `pulse land [--project X | --all]` runs one delivery pass now: it pushes the
  ready branches, opens or updates their pull requests, polls the open ones
  and fast-forwards the local default branch. It prints one line per pull
  request (`task pr <slug> <task>: #n opened|updated <url>`, `job pr <slug>
  job <id>: …`, `task pr <slug> <task>: held: <why>`, `task pr <slug>: #n
  merged; task done`, `… closed unmerged; the branch stays on the remote`)
  and `<slug>: nothing waiting` when there is nothing to do; a project
  without an upstream prints `<slug>: no upstream, landing locally` and the
  local landing's lines. `--limit` is accepted for older scripts and does
  nothing: a pass opens at most ten pull requests per project.
- `pulse land --dry-run` lists what the next pass would deliver, per project,
  without pushing anything: where the pull requests go (`pull requests go to
  origin/main <url>`, or `no upstream, finished branches land locally on
  main`), then each task with its jobs and branches, each job's branch and
  whether it becomes a pull request or a push onto one, and each conversation
  branch with how far ahead it is and how old its last commit is.
- `pulse land --project X --branch B` proposes `B` as a pull request: the
  branch is pushed and a pull request opened from it into the upstream default
  branch, delivered and reviewed like any other; it prints `<slug> proposed: B
  #n <url>`, or says the branch is delivered already (`--propose` is accepted
  as an alias; a conversation's **Open PR for review** button runs the same).
  A branch not under `pulse/` counts as a person's to the reviewer, so it is
  handed to you rather than merged ([The complexity
  rule](review.md#the-complexity-rule)).
- `pulse land --project X --branch B --local` is the escape hatch: it merges
  `B` into the local default branch the way the old landing did (the verify
  command first unless `--no-verify`, then a fast-forward or a `Land …` merge
  commit) and warns that the local default branch will be ahead of its
  upstream until that is pushed or reset, and that the daemon only ever
  fast-forwards it. Use it when the forge is unreachable and a change cannot
  wait; the rest of the time, propose the branch.
- `pulse review …` reads and decides pull requests ([The
  reviewer](review.md#the-verdict-commands)); `pulse pipeline` shows what the
  merges set running ([Pipelines](#pipelines-what-a-push-set-running-on-github)).

## Moving from the local landing

Nothing needs doing: on its first pass under this flow the daemon delivers
every branch that was waiting to land - each finished job's branch of a task
as that task's pull request, each other finished branch as its own - and the
hub's *Waiting to land* rows become *In review*. A branch the old flow had
held (an owner-only path, a lockfile too young, a verify that failed on the
merged result) is a pull request now too, and the reviewer's complexity rule
hands the ones that need a person to you ([The complexity
rule](review.md#the-complexity-rule)). A project with no upstream keeps the
local landing, verify command and all, until you give its default branch one
(`git remote add origin …` or `git branch --set-upstream-to`).

Settings: `review.merge`, `review.max_files` and `review.max_lines` are new
(review.md has them). `landing.parallel` is gone with the per-project landing
workers. `landing.batch` is removed too: the local landing of a project
without an upstream verifies and lands one branch at a time, and a stored
value is ignored. `landing.push` is kept so an older store's
value still parses, but no code reads it: the daemon pushes branches and
never the default branch, and the merge is the forge's. `landing.auto_stash`
keeps its meaning for the fast-forward of a dirty checkout
([above](#the-local-default-branch-follows-the-upstream)) and for `pulse land
--local`; `landing.verify_memo` and `landing.verify_memo_hours` govern the
local landings and the run's own check ([The verify
command](#the-verify-command)). `landing.base_red_retry_minutes` and
`landing.setup_repair` keep their meaning for the runs and local landings
that find the default branch red or its setup broken ([A red default
branch](#a-red-default-branch)). `push.credential` is used for the push and
the fetch alike ([Credentials](#credentials)). Patches the old flow saved
under `~/.pulse/patches/<slug>/` stay where they are; only a local landing
writes there now.

What the local flow had, in one paragraph each, so the older events and
settings read right:

- **Verify before landing.** Each branch was merged in a throwaway worktree
  and the project's verify command run on the merged result before the
  default branch moved; a failure was charged to the branch (patch saved, job
  requeued) only when the default branch passed the same command. The check
  now runs in the worker's clone (the pre-commit hook and the run-end check,
  [The verify command](#the-verify-command)) and once more where the reviewer
  checks the pull request out.
- **Batch landing** (`landing.batch`). Two or more waiting branches were
  merged into one chain and verified once, because one verify took 1,210 s on
  average on the brain and the branches queued behind each other. A pull
  request per branch has no shared verify to save, and the local landing of a
  project without an upstream has too few branches to need one: the setting
  and the chain code are removed, and a stored value is ignored. Older stores
  still hold `land.batch` events as history.
- **The landing lock and parallel landings** (`landing.parallel`). One lock
  per project (`.git/pulse-land.lock`) kept landings serial; a lock left by a
  daemon killed mid-landing froze the project's queue until it expired, 30
  minutes at a time. There is no lock: the pass pushes and reads, and nothing
  it does needs the checkout to hold still.
- **The red-base fix and the setup repair at landing.** A landing that found
  the default branch itself failing the check held the project's landings and
  queued a fix job; one whose worktree setup failed queued a repair. Both are
  still planned from the runs that find the same thing ([A red default
  branch](#a-red-default-branch)); a landing no longer finds it first.
- **Resync.** When someone had moved the upstream, the landing merged the
  remote commits in as a landing of their own and re-applied the local
  commits the upstream lacked onto its tip, with a `task` job per commit that
  could not be re-applied. The local default branch has no commits of its own
  any more, so there is nothing to re-apply: it fast-forwards.
- **Landing hold notes.** `pulse status` and the hub carried a `landing` line
  (how many branches waited and why: a red base, a refused push, a young
  lockfile, an owner-only path). A branch that cannot be delivered is
  `land.pr-held` on its own task or job, and a pull request that waits for a
  person is *Needs your review* on the hub.

## What changed and why

The local landing was the slowest and most fragile part of the day, and it
served the smallest share of the work. Of the 260 landings on one machine in
the seven days before this flow, 22 (8%) were task work; the rest - suggestion
jobs, security updates, lead and schedule work, fixes, research, handoffs -
went through the same serial path. That path stalled in ways that had nothing
to do with the change being landed: a landing lock left by the restart of a
self-update froze a project's queue for 30 minutes, four times in one day;
each branch's verify ran 7-10 minutes on the brain and serialised every branch
behind it; and the owner saw task after task *waiting to land* for hours while
nothing was wrong with the work.

Delivering by pull request removes the serial step. The daemon pushes a branch
and opens a pull request in seconds, the forge runs the project's pipeline on
every pull request at once, the reviewer reads each one as a job like any
other, and the local default branch follows the upstream by a fetch and a
fast-forward that touches nothing of yours. The owner reads pull requests, not
hold notes: a diff with the reviewer's findings beside it, a **Merge** button
in the hub for the ones the rule hands over, and GitHub's own history of who
merged what. The 8% that PR #53 had already moved to pull requests showed the
shape worked; this is the same shape for the other 92%.

## Workers and the load governor

The rest of this page is what else happens on the machine around a run: the
workers and paired hands, the disk floor and the headroom per run, the load
governor and the express lane, the verify command and how its runs take turns,
a red default branch, the pipelines a merge sets running, the dependency
watcher and the credentials the daemon pushes and fetches with.

### The verify command

The verify command is the project's `verify_cmd` (`pulse init PATH --verify
CMD` sets or replaces it; re-registering keeps everything else); a project
registered without one adopts its recipe's `suggested_check` the first time the
daemon or a run reads a recipe that parses - once, with a `finding` on the
project's `runner/verify` thread and a `verify.adopted` event - so a project is
never left delivering unchecked work because nobody passed `--verify` (`pulse
env status` shows both when they differ, and an owner's own command is never
replaced; before this, seven of eleven projects on one machine had no verify
command and landed every job unchecked, three of them with a `make test`
committed in their recipe that the daemon never ran).

The command runs where the work is made: in the worker clone's pre-commit hook
at every commit, so a commit that fails the check is never made, and once more
when the run ends, on the clone as the worker left it - unless the clone is
clean and its committed tree is the one the hook just passed: the hook leaves
a memo of the tree and the command under `~/.pulse/verified/<slug>/`, and a
run that finds a fresh one (`landing.verify_memo`, `landing.verify_memo_hours`)
says `verify skipped: tree <tree> already passed this check on this store` and
records `verified earlier by hook at <when>` as its verification rather than
run the identical command on the identical tree again; a check that does run
and passes on a clean clone leaves a memo of its own (`by: run`), so a landing
or a [base check](#a-red-default-branch) on that tree can skip in turn. The
measured reason:
on the brain, 42 run checks in three days ran 720 minutes and waited 462 for
the project's verify lock while the hook had written 46 memos for those
trees in the same days - twenty hours, a third of the project's lock time,
spent passing the same trees twice. A run whose worker left changes
uncommitted, or on a remote executor (whose hook writes its memo on the
remote store), runs the check as before. A run whose check fails
is a failed run - no branch is written, the job is retried as [Recovery](recovery.md)
describes - so every branch the daemon delivers passed the project's own check
on the commit it carries. The forge then runs the project's pipeline on the
pull request ([Pipelines](#pipelines-what-a-push-set-running-on-github)), and
the reviewer runs the command again where it checks the pull request out
([The reviewer](review.md)). The merged result is not verified on this machine
before the default branch moves, because the default branch does not move
here: a merge the forge makes is a merge the forge's pipeline checked.

The clone starts bare, so the runner first runs the project's setup there -
the recipe's `setup:` lines from the default branch, else the lockfile rules
(`corepack pnpm install`, `uv sync`, `npm ci`, …) - within ten minutes
(`SETUP_MINUTES`, the budget every setup gets, whatever `minutes_per_job`
says: a hung `pnpm install` once held a slot for the two hours a job may run).
A setup that fails that way - not a timeout, not a dead network or a full
disk, judged on the output's last lines where the error is, so that npm's
`could not resolve dependency` peer note above a lockfile error never passes
for a DNS failure - is a fault of the default branch itself that no retry and
no other job can get past, since every clone of the project runs the same
setup first. Two such failures with the same signature mark the project unfit
for that executor ([Workers and paired hands](#workers-and-paired-hands)), and
the daemon plans the fix (`landing.setup_repair`, default `on`): one Tier 1
`task` job with source `setup:SLUG` (a `setup.repair` event records the
command, the exit code and the last lines), briefed with the base commit, the
recipe file, the command and the output, and told the three ways such a setup
breaks and how to prove each fixed: the recipe names paths or tools the tree no
longer has (`pulse env set --project SLUG --file …`, and `pulse init PATH
--verify` for a verify command that moved with them), a lockfile out of step
with its manifest (regenerated only at the versions the manifest already pins,
since the release-age rule reads every new version it adds), or the tree
itself. That job is the one job of the project the setup is not run for - the
runner says so in its log and the brief's rule 3 tells the worker to install
only what the setup would have - and it is leased even while the project is
marked unfit for the machine, which holds everything else of the project back.
No repair is planned while the checkout is behind its upstream, because the
recipe may already describe the upstream's tree, set by hand after the owner
renamed a solution, and a worker sent to fix it at the stale head would write
the old paths back; the fast-forward brings the checkout up on the next pass,
and a setup that still fails there plans one.

Three repairs a day per project at most; a failing repair stays open and is
briefed afresh at the next failure rather than queued again; and the mark
clears when the executor's next probe gets through the setup (an `executor.fit`
event; the daemon re-probes a marked machine on `daemon.unfit_reprobe_minutes`
and the mark expires after 24 hours in any case), so the held work follows the
repair out. Before this, a project whose `server/package-lock.json` had drifted
from its manifest sat frozen on the hub for a day behind an `npm ci` that no
job was allowed to fix, and a pnpm workspace whose lockfile no longer matched
its `overrides` held every run the same way.

### Workers and paired hands

Each worker is one `pulse harness run` process (the daemon's own binary here;
on a paired hand the shipped `pulse-hands` binary, which `/usr/local/bin/pulse`
links to) given the brief, the clone, the limits and the model from Settings,
one minute less than `minutes_per_job` so it can stop with a report of its own,
and the run id as its session; its stdout is the report the runner reads. A
paired hand needs the same sign-in once: the native harness reuses the brain's
stored provider login during heal/provision (a machine the daemon buys is
paired first and logged in right after, before the probe that makes it
active), and `pulse executor login NAME
--admin admin@host` copies that login to the hand's `pulse` user
(`--with-token` still reads a token from stdin; `--remove` forgets it). The
hand's probe reports `harness=yes|no` and the `pulse` build it runs. The probe
also refuses a hand whose workdir filesystem has under `daemon.min_free_mb`
(default 10240, ten gigabytes: a build and the clones beside it, since a run
that hits ENOSPC mid-build is lost work) megabytes free and names that path in
its note; 0 turns the check off. A hand that was drained but never active
(added with `pulse executor add`, registered in the console, or paired while
the host was still booting) becomes active on its first passing probe,
including the daemon's own; a hand drained by the owner, by errors or by the
login gate still needs a human probe.

### Disk: the floor and the reclaim ladder

The daemon checks the same floor before it leases new work or starts an
experiment measurement, so a full work filesystem pauses dispatch with `disk
under floor: <free> MB free on <path> (daemon.min_free_mb <floor>)`; after
fifteen minutes it writes one runner finding that says to free space or adjust
the floor. It does not only wait for you: while the disk stays under the floor,
each maintenance pass (at most every thirty minutes) climbs a reclaim ladder
(`crates/pulse-runner/src/reclaim.rs`) and stops at the first rung that puts
the free space back over the floor. Rung one removes what the runner's own
tools left in the OS temp directory (`TMPDIR`, and `/tmp` when that is a
different place): Rust `tempfile` directories (`.tmpXXXXXX`) and
`pulse-*`/`check-site-*`/`distill-ui-*` folders that nothing has written to for
two hours and no live process has its working directory or executable
under - on 2026-10-12 one laptop carried 105,774 of them, 19.7 GB, the test
homes of `cargo test` runs the step deadline had killed before their
destructors ran. Rung two runs the sweep in pressure mode: finished job work
goes at once instead of after its hour of grace, work the store does not know
and run logs after a day instead of the configured keep, build-cache lanes idle
for an hour instead of `daemon.build_cache_keep_days`, and the store gives its
free pages back past 64 MB instead of `daemon.vacuum_free_mb`; the things the
sweep never touches stay untouched under pressure too. A floor still not clear
after both rungs is reported with the five largest entries under the temp roots
and `~/.pulse`, so the owner knows what to look at. Each pass that removed or
listed something writes one `daemon.reclaim` event, and the disk-floor finding
carries the last pass: what it freed, over which rungs, and what is left.
`pulse sweep --reclaim [--dry-run]` climbs every rung by hand, floor or not.

The leak itself is closed at the source: a local run's worker, its pre-commit
hook and its run-end check get `TMPDIR` set to `<clone>.tmp` beside the clone
(a recipe that sets `TMPDIR` wins), and a by-hand `pulse land --local` gets
`<worktree>.tmp` beside its worktree, so what a killed test leaves behind goes
with the clone or the worktree instead of piling up in the system temp
directory.

### Disk headroom per run

The floor alone cannot see that the ten setups one tick is about to start each
need a gigabyte (on 2026-10-02 a restart with a ten-job backlog did exactly
that on 1.1 GB of headroom: every clone and `npm install` ended in ENOSPC, the
failures counted as transient, and the same jobs failed the same way after each
five-minute pause, fifty-four times in an hour), so the daemon also measures
the work directory of every local run as it ends (`runs.work_mb`) and, each
tick, lets a local slot take only a project whose footprint the free space
above the floor still pays for - the largest of the project's last ten measured
local runs, or `daemon.setup_mb` (default 1024) until a project's runs have
measured it. Each start claims its project's footprint, a run still in setup
(no brief written yet) holds its footprint until its brief exists, and a
project the rest no longer pays for waits while a smaller one starts (the first
hour of the admission held every slot behind the fleet's largest footprint:
1455 MB of room and nothing started, while four of five projects took under 900
MB). One run is always admitted while none is in flight so a small disk still
measures itself, and a slot held back this way leaves the queue as short of
hands as a busy one (below).

A held project shows in the idle reason as `N queued wait: N for <project>
(local needs M MB of disk)`, and a slot no project fits says `disk headroom: R
MB above daemon.min_free_mb F with K run(s) setting up; waits: <project> (M
MB), ...`; `daemon.setup_mb` 0 turns the admission off. Hands keep their own
disks.

### The load governor

The machine itself is governed the same way
(`crates/pulse-runner/src/daemon_load.rs`, sized by `capacity.rs`): a 12-core
16 GB laptop once told to run twelve local slots leased seven jobs within a few
seconds and ran their `npm ci`, test suites and `cargo test` builds at once, at a
load average of forty with 2.3 GB of its 3 GB swap in use. There is no slot
setting any more: every machine works out its own slots from its hardware - the
smaller of its physical cores and one run per 3 GB of RAM, never under one (a
12-core 16 GB laptop: five; a 32-core 64 GB box: twenty-one) - and each paired
machine does the same from the cores and memory its hands report on every
probe. The fleet's total is the sum over the active machines, and a project may
have `ceil(total / projects with work)` jobs in flight, so a lone busy project
uses every slot and several share them fairly; `pulse status` shows it as
`slots 18 = this machine 10 + vm-1 8, per project 6`. The one optional override is
`capacity` (auto by default; a number caps this machine, 0 keeps it idle), and
`pulse daemon --max-jobs N` overrides both for one daemon; `pulse stop` pauses.
The startup line `load governor: up to N local run(s) at once
(auto|capacity|--max-jobs; C cores, P CPUs, M MB), nothing new over load L or
under F MB of memory free; children run at nice 10, builds at -jJ, the daemon
itself at nice 5` names what is in force.

Before leasing it also reads the one-minute load average and the memory a new
run could use (free, inactive, speculative and purgeable pages on macOS - the
caches the kernel drops on request, a gigabyte or two on a 16 GB laptop whose
`free` reads a few hundred megabytes - `MemAvailable` on Linux) and leases
nothing new while the load is over the load line (twice the number
of CPUs - the load average counts runnable threads, and a laptop with an
emulator or a browser build sits at its CPU count for hours while its CPUs are
not busy; a line at the CPU count held the fleet all day) or the memory is
under 1024 MB (both internal constants, not settings), or while macOS itself
reports critical memory pressure; the runs in flight finish, the idle line says
`machine saturated: load 40.1 on 12 CPUs is over the line 24` or
`machine saturated: 512 MB of memory free, under the floor 1024`
once per change rather than per tick. A queue behind held slots is as short of
hands as one behind busy slots: once an improve or task job has waited half an
hour that way, the daemon rents a machine itself when `compute.auto_approve` is
on and `compute.max_vms` and `compute.max_hours_per_day` allow (one open
request at a time, four hours, `AUTO_COMPUTE_HOURS`), and otherwise posts
`hands wanted` to the owner once, saying why no machine was rented. A job
whose needs only a rented machine meets does not wait for the slots to fill:
it rents after ten minutes on the same terms (`docs/machines.md`, "When no
machine fits").

The kernel's memory warning (the level under critical, which a 16 GB laptop
reports for hours with a browser open) is not a hold: while it stands the
daemon starts half as many local runs at once (the machine's slots halved, never
under one), and the experiment lane's measurements in progress count as runs
under the cap, since their builds are load like any other.

### The express lane for the owner's work

A load hold alone (not memory, not a restart window) keeps one local slot open
as the express lane: it leases only the owner's work - lead runs, answers,
`task:` jobs - while the load stays under twice the load line and the run
cap has room; the idle line adds `one express slot stays open for the owner's
work`. The fleet's own builds and checks are what push the load over the
line, and without the lane they held the owner's tasks behind themselves. The
cap itself is the other way they did it: with two local slots and both
slots on fleet verifies, the lead sitting your *Try again another way* queued
waited forty minutes for a slot. So when every slot under the cap is busy with
the fleet's work, none of it is the owner's, no slot is reserved for a restart
and the machine is not hard-pressed (memory, the floor, twice the load line),
the daemon lends one slot over the cap to the owner's work alone - one at a
time, never while an owner run is already in flight - with the line `every slot of this machine is busy
with the fleet's work; one express slot stays open for the owner's work`.

On top of that, while owner-driven work this machine could run is queued,
one local slot is held for it. Under the cap the fleet's work cannot take
that slot. Over the cap, the load line at any height or the memory floor, the
owner's one run starts there anyway, and only the kernel's critical memory
pressure keeps it shut. A fleet run is asked to yield only when the owner's job
could then start. [owner-reservation.md](owner-reservation.md) explains how
this works and why.

A free slot is not a free lane, though: a project's own work lanes
(the fleet's per-project share, narrowed to what fits the commit cap while its checks
take turns) count the fleet's runs and the owner's alike, so an owner's task
queued behind one long fleet `improve` job in its project waited four hours
while the daemon leased fifty runs in other projects around it. Now the owner's
work goes first in its own project: while an owner-driven work job (`task:`,
`security:`) is queued in a project, the fleet's work jobs of that project stay
in the queue, and when every lane the owner's job needs is held and some by the
fleet's runs, the daemon asks the youngest of those runs to hold at its current
step - the same hold the restart uses: the clone and its commits stay, the job
goes back to the queue costing it no attempt, and the same agent carries it on
from its harness session once the owner's run has a lane. The daemon says
`<project>: <job> (improve) holds after its current step so the owner's <job>
can start; it carries on later`, writes a `job.yield` event on the fleet job
(`project`, `run`, `to`), and the held run's summary reads `held so the owner's
work could start first after N steps`. The owner's own runs are never held, and
a project whose default branch is red is left alone: the owner's task waits
behind the fix job there and holding the fix would only lengthen the wait.

The narrowed lanes are the fleet's, not the owner's. On the brain the check
takes twenty to twenty-seven minutes against a forty-minute commit cap, so one
lane of five is open, and the jobs the lead planned from the owner's tasks -
one and a half to two and a half hours each - serialised behind each other
through that one lane: four of them sat queued ten to thirteen hours with no
attempt, and the yield could not help, since the run in the lane was the
owner's own. So the owner's work (a lead's sitting, an answer, the jobs planned
from the owner's tasks, `task:…`, the security updates) takes a lane beyond the
narrowed count at once, bounded by the plain `parallel_per_project` alone, and
its run's commit cap is widened to cover the checks it shares the lock with -
`n × the verify median + harness.step_timeout` for `n` work runs in the
project, never below the configured cap; with the default five-minute step the
run log says `commit cap widened to 51 min: 2 checks share the verify lock on
amistio-template (verify ~23 min each)`. The fleet's work keeps the narrowed
lanes, and a fleet run in the owner's way is still asked to hold, so the lanes
return to their count and its own commit is not killed waiting on the owner's
check. The owner's work fits every restart window too (the window names the
kinds expected to end before the runs in flight, a task on the brain never was
one, and the same tasks waited through seven windows in a day; see
[runner.md](runner.md)).

The lead's own sitting is outside its project's count altogether. A sitting
reads and posts for a minute, plans the owner's work and directs the fleet's;
with a per-project share of 2 and two long fleet `improve` runs in the project,
the sitting an owner's *continue* queued once waited 54 minutes for a lane the
express slot had offered it every tick. Now a project lead's sitting leases
whatever the project's lanes hold (at most one short run over the limit: one
lead leases one job at a time); a role lead's standing sitting (`lead-role:`)
is the fleet's work and keeps the count.

### Nice, build jobs and probes

Remote hands carry their own load and are not capped by this machine's
governor. Every run the daemon starts here - the worker, its `npm ci`, its test
suite, the clone's removal - runs at nice 10, so the owner's own work and the
daemon's loop stay responsive while the fleet works. Nice keeps the machine
responsive, not cool: four runs each compiling on all twelve cores still ran
the laptop hot for most of a day, so the daemon also exports
`CARGO_BUILD_JOBS=<daemon.build_jobs>` (default 0: half the physical cores) to
everything it spawns - a worker's `cargo test`, the pre-commit verification
that inherits the worker's environment, the run-end check, the
self-update build's `-j`, the experiment lane's measurements (each fixture's
share of the cores, `evolve.measure_parallel`, never over this cap) - whenever
the environment does not set it already, so an owner's own export or a recipe's
`env:` wins and a `-j` on a command line still overrides it. Other toolchains
read their own variables (`GOMAXPROCS`, `MAKEFLAGS`); a recipe sets those where
a project needs them.

The daemon itself lowers its own priority to `daemon.nice` (default 5) as it
starts, so its planning, sweeps and delivery passes yield to the owner's
foreground work while the loop still comes before the fleet's builds; 0 leaves
the level the service started it at, and a `renice` by hand does not survive
the restart a self-update brings, which is why the daemon does it itself.

`pulse executor probe local` probes this machine the same way (git, free space
under `~/.pulse/work`), and a passing probe run by the owner clears the
projects a failed setup marked unfit here, instead of waiting a day for the
marks to expire. While queued work waits behind an unfit mark, the maintenance
pass also re-probes after `daemon.unfit_reprobe_minutes` (default 60; 0 off),
then doubles the wait for that executor/project/tool up to one day; a passing
daemon probe clears only that project's mark. A mark about a tool (`lacks
node@22`, `lacks bundle`, `lacks mise`) is cleared only once the tool
answers - the probe runs `mise where PIN` or `TOOL --version` on the machine
first (on remote hands through their `tool` verb; older hands are cleared as
before), since a probe that looked only at git and disk sent the jobs back into
the same failing setup every hour; while the tool is still missing the mark
stays, the re-probe wait doubles as after a failed probe, and the mark expires
a day after the setup failed, so the setup gets one real try a day. A pin the
recipe no longer lists is cleared whether or not it is installed.

Remote hands take a recipe's `tools:` and `env:` the way the hub does - the
worker runs under `mise exec PIN… --` with the recipe's env set, after the
`setup:` lines ran there - once their probe answers `recipe=1` (hands shipped
since this landed do; `pulse pair` refreshes older ones, then `pulse executor
probe NAME`); until then such a project runs on the hub only, as before, which
kept five of eight projects off the hands.

A setup that fails twice the same way marks the project unfit for that executor
and plans the setup repair job described under [the verify
command](#the-verify-command), which that mark lets through; a failure the machine did not cause - a dead network, a full disk (`no
space left on device`, `disk quota exceeded`) - never does, and the job simply
goes back to the queue. While a mark holds queued jobs, the daemon's idle line
says so (`no queued job; 3 queued wait: 3 for blog (local cannot set up
Gemfile.lock); …`) with the probe command, instead of a bare `no queued job`
over a queue nobody can see. The login check the daemon runs on an executor is
a two-step harness run asked for the one line `SUMMARY: ok`; it passes when the
provider answered, which includes a run that spent its two steps on a command
and stopped at the budget after one or more model calls - the login held, the
model merely wandered. A run the provider never answered fails the check.

### A red default branch

A red default branch is also a plan, not just a hold: the first run that
finds the base itself failing the verify command - a run whose own check
failed asks whether the base was already red before charging the job - records it (`base.red`
event, one `finding` on the project's `runner/verify` thread with the check's
tail), queues one `improve` job to make the check pass - source
`redbase:<slug>`, priority above the planner's improve jobs, a brief that
carries the command, the exit code and the failing lines and forbids loosening
or skipping the check; a corrected job keeps the source if it fails - and holds
the project's other work on the branch (improve, task, experiment and clean
jobs) with its attempts intact until the default branch moves; research, ballots,
answers and leads keep running, and the idle line counts the held jobs (`2 jobs
of blog wait for a green default branch (3f9c… is red; fix job … leased)`).

A check that did not run to a verdict - the disk under `daemon.min_free_mb`
(exit 71), the verify lock given up (exit 75), a timeout, a worktree that could
not be made - says nothing about either tree: the run waits as before, but the base is not recorded red and the branch is not charged, and the
base tree is not checked again when the job's own check could not run.

The base check reads and leaves the same verified-tree memos as the run's own
check ([The verify command](#the-verify-command)): a base tree this user
passed within `landing.verify_memo_hours` - as a run's committed tree, a local
landing's, or an earlier base check's - is green without taking the project's
verify lock (`base verify skipped: tree <tree> already passed this check on
this store`), and a base the check passes here is memoed for the next run that
fails on it. The measured reason: the three brain runs that failed their check
on 2026-11-04 each re-checked the same main tree, fifteen to twenty-five
minutes of the lock apiece, to learn the same thing.

The hold lifts on the tick after the default branch moves (`base.moved`),
whoever moved it, and when the fix job ends with no commit and the daemon's own
check of the default branch as it stands passes (`base.green`, a finding on the
same thread; the red was never the tree's own, or was fixed without moving the
branch); if half an hour passes with no fix job open and the base still red,
the daemon asks the owner once (`default branch main is still red at …`, a
question marked for the owner) with the two ways out - merge a fix by hand,
or give the project a check it can pass with `pulse init PATH --verify CMD` - and
answers its own question when the branch moves or the hold lifts. In the week
before this landed, 42 runs across 40 jobs on one machine each spent a worker
to learn the same thing and threw their commits away.

A branch that changes `docs/context/environment.md` alone was yours to merge
under the local landing (ADR-026: the daemon held the branch and asked once
on the project's `runner/environment` thread). It is delivered as a pull
request like any other now, and the reviewer's rule does not single the path
out ([The complexity rule](review.md#the-complexity-rule)): the reviewer
merges it unless another reason holds it, or every pull request when
`review.merge=ask`.

### Verifications take turns

The same verification command runs more than once per change - in the worker
clone's pre-commit hook at every commit, once more when the run ends, and
again where the reviewer checks the pull request out - and on one machine
those runs overlap across clones.
When the clones share a build cache (a `CARGO_TARGET_DIR`, a Gradle or Bazel
cache: the usual way to keep a large build fast and the disk small) two
overlapping runs rewrite each other's artifacts mid-run and both fail for
reasons neither branch contains, so a good branch is charged or the default
branch is judged red. `pulse settings verify.lock project` makes the
verifications of one project take turns on the machine (`machine`: every
verification does); the command runs under an atomic lock in
`$PULSE_HOME/locks/`, a later arrival prints that it is waiting and for whom, a
lock whose holder has died is removed by the next arrival, and a wait longer
than `minutes_per_job` gives up with exit 75 without running the command.
Taking turns does not make a shared cache tell one checkout from another: a
build system that decides freshness by file times still needs what the
checkout owns refreshed before it builds. For a Cargo project whose check
builds into a target dir outside the clone - the recipe's `CARGO_TARGET_DIR`
or the one the verify command names - the daemon does this itself: every
check it runs (the pre-commit hook's, the run's own, the base tree's, a
landing's) touches the tree's tracked `*.rs`, `Cargo.toml` and `build.rs`
files *inside the lock, after the wait*, so the workspace crates rebuild from
the tree being judged while the dependency cache stays. The place matters:
Cargo names a workspace crate by its path relative to the workspace root and
calls it fresh when its sources are older than the artifact, so a clone whose
hook waited an hour behind another clone's check inherited that clone's test
binaries - on 2026-11-04 two brain runs passed their hook that way and failed
the run-end check, one with a compile error, after a day's work each.

Taking turns also sets how many work jobs a project can usefully run at once:
with a per-project share of 4 and a check that takes eight minutes, a clone's
commit may wait for three other checks before its own and the harness kills the
command at `harness.commit_timeout` (25 min by default; plain commands stop at
`harness.step_timeout`) - the run retries, and the lanes burn their budget
waiting on each other. So the daemon measures the check's own run time
(`verify_seconds`, the lock's wait excluded; the local flow recorded it on
every `land.landed` event, and those measurements are what an older store
still reads) and leases `improve` and `task` jobs for the project only up to the lanes that
fit that commit cap (`commit_timeout / median of the last five`, never below
one; the cap is `harness.commit_timeout`, never below `harness.step_timeout`,
the same deadline the harness gives the gated commit); research, ballots and
answers keep the plain limit. The project's idle line says so: `2 of 4 work
lanes open: the check takes ~10 min and 2 in a row fit the 25 min commit cap
(harness.commit_timeout)`. A quicker check, a longer commit cap, or
`verify.lock off` opens the lanes again. The lanes are the fleet's: the owner's
work takes one beyond the count at once, with its commit cap widened to cover
the checks it then shares the lock with (see the express lane above).

When the deadline still lands on a worker's `git commit` - the change finished,
the pre-commit check did not - the runner does not discard the tree: if the
worker exited normally, left no commit, and the run's transcript shows a `git
commit` step that timed out, the runner commits what the worker left (tracked
changes and whatever it had staged, under the worker's own `-m` message when it
gave one, signed with the run key and carrying the provenance trailers; only
the pre-commit check is skipped) and the run's own verification judges it like
any other commit. The run log says `the runner committed its work for
verification`; a worker that stopped for a reason of its own keeps its verdict.

## Pipelines: what a push set running on GitHub

A push is not the end of a delivery: the repository's GitHub Actions
workflows run on it, and a red one is a regression PULSE made. So the daemon
watches what its own pushes set off - and, since a project's default branch
is PULSE's to keep green whoever pushed to it, the head of that branch on
the forge as well. Each tick, for every active project
whose remote is on GitHub, it takes the pushes of the last 48 hours that
still have no verdict - every `land.pr` push, the head a pull request was
opened or updated from - and asks GitHub for the runs on that commit (`gh
run list --commit`, the GitHub CLI logged in as the user the daemon runs
as, with read access to the repository's Actions). It counts the runs the
push itself caused (`push`, `pull_request`, `pull_request_target`,
`merge_group`, and the `dynamic` runs of the repository's own workflows,
not GitHub's **Dependabot Updates** and **Dependency Graph**) and records
one verdict per push as an event with the runs in its detail:
`pipeline.passed` when every counted run finished green (`skipped` and
`neutral` count as green); `pipeline.failed` when one failed, timed out,
did not start or wants your approval; `pipeline.none` when nothing ran for
the push ten minutes after it, or every run was cancelled; `pipeline.gave-up`
when a run is still going `pipeline.wait_hours` (default `3`) after the push.
While runs are still going the daemon looks again every three minutes - a
failure beside a run that is still going is reported once the push is half
an hour old, so one slow deployment does not hide a failed test suite.

A `failed` or `gave-up` verdict is a `finding` for you on the project's
`runner/pipeline` thread: which workflow failed, the step that failed in
each failed job, the last lines of its log (secrets redacted), what the push
landed, and the command that re-runs the failed jobs (`gh run rerun <id>
--failed`); `none` is only recorded, since a push that ran nothing broke
nothing. With `pipeline.watch` at `fix`, the default, a red pipeline also
queues one fix job for the project (`task`, tier 1, priority 60, source
`pipeline:<slug>`) whose brief carries the same evidence and the project's
verify command: a worker takes it like any other job, and its branch is
delivered as a pull request and watched in turn. A worker has no `gh` of its own, so the brief is
what it knows of the failure; when GitHub no longer has the failed log
(HTTP 410 or 404: the repository's log retention passed, or someone deleted
it) the daemon does not plan a job around a brief with no error in it. It
reruns the failed jobs instead (`gh run rerun <id> --failed`, once per
commit, recorded as `pipeline.rerun` with the reason), polls the rerun like
a push of its own and judges that: green means there was nothing to fix and
the verdict says so; red again gives a fresh log, and the brief carries it
with a line that says the first attempt's log had expired. A rerun GitHub
refuses, and a log the daemon could not read for another reason (a timeout,
say), plan the job at once with the error named in the brief and the
finding, so a worker knows the log is missing rather than empty. One such
job is open per project at a time -
a second red push while it waits refreshes the queued job's brief instead
of queueing a second one - and after three fix jobs in 24 hours the daemon
plans no more and says so in the finding, so a pipeline that is red for a
reason outside the code (a secret that expired, a runner that is off) does
not turn into a day of workers. When several pushes of one project are
waiting for a verdict, only the newest gets the finding and the job; the
older ones are recorded as superseded. `watch` keeps the findings and skips
the jobs; `off` reads nothing. A repository the active gh login cannot read
(HTTP 403 or 404) is read as another account `gh auth status` lists when
one can see it, and that account is remembered for the project; when none
can, it is reported once as `pipeline.unreadable` with the accounts tried,
left alone for an hour and tried again after it ([the
runner](runner.md#the-daemon-loop) has the detail).

The branch head is read every ten minutes (`gh api
repos/<owner>/<repo>/branches/<branch>`, the branch the local default branch
tracks, `origin/<default>` when it tracks none). A head that is one of
PULSE's own pushes, or a commit already judged, is left alone; any other
is judged by the same rules, its verdict recorded with `head: true` and the
login or name that pushed it (`by`), and a red one is a finding and a fix job
like a push's. The merge of a pull request is such a head: the merge commit
is the forge's, not a push of PULSE's, so a merge that turns the default
branch red is found here, whoever merged it. One difference from a push's
fix job: a worker starts from the project checkout, so the job is planned
only once the checkout's default branch holds the commit. The delivery pass
brings it there - it fetches each project's upstream and fast-forwards the
local default branch every pass ([above](#the-local-default-branch-follows-the-upstream)),
because a tracking ref nobody fetches never says the forge moved: before
this, a merge made on GitHub never reached the checkout, and the red head
waited its two hours for nothing. Until the checkout holds the commit the
daemon looks again every three minutes and the log says it is waiting; when
the checkout still lacks the commit two hours after the daemon first saw the
failure - a dirty checkout on the default branch is the usual reason - the
finding goes out without a job and names what holds the checkout behind (the
`land.sync-skipped` reason, when there is one), so you can see what to clear. The
brief of a head's fix job says that the push was not PULSE's and that PULSE
looks after the branch all the same.

`pulse pipeline` shows, per project, the branch head last read (who pushed
it, and since when a red one waits for the checkout), the pushes still being
watched and the last verdicts (`(pushed by X)` on a head's; a red one names
its fix job and how that job stands - `(queued)`, `(leased)`, `(done)`, or
`(failed: blocked: ...)` with the worker's first words, so a red pipeline
whose fix came back blocked reads as yours, not as looked after), with `--project
<slug>` for one, `--all` for every active project, `--limit N` for more
history and `--json` for the records (`head` holds the sha, `by`, `read`,
`next` and `waiting_since`); `pulse pipeline --check` reads GitHub now
instead of waiting for the daemon's pass. A task's conversation in the hub
shows the verdict on its own push on the delivery line
(`delivery.pipeline` and `delivery.pipeline_job` in `GET /api/v1/tasks/<id>`).

## Dependencies: security alerts and the release-age rule

One rule governs every dependency change in every project PULSE manages: **a
version is adopted only once it is `dependencies.min_release_age_days` old**
(default `3` days, a store-wide setting, `0` turns the wait off). A package
that is hijacked or shipped broken is usually pulled within days; waiting
that long costs little against a known vulnerability and nothing against a
routine bump. The publish dates come from the package's own registry - npm,
crates.io, PyPI, the Go module proxy, RubyGems - and are cached in the store
once known (`release_age:<ecosystem>:<name>:<version>` in `meta`), since a
publish date never changes. The rule is applied by the watcher before it
queues an update and by the local landing before it merges a branch; a pull
request on the forge does not apply it (below).

**The watcher.** Once per `dependencies.alert_check_minutes` (default `60`)
the daemon reads each active project's open Dependabot alerts through the
`gh` CLI it already uses (`gh api repos/<owner>/<repo>/dependabot/alerts`,
GitHub remotes only), groups them by manifest and asks the registry, for
every vulnerable package, for the newest patched version that stays within
the installed version's compatible range - same major, same minor below
`1.0` - and is at least the rule's age. Those become **one Tier 1 `task` job
per manifest** (source `security:<slug>:<manifest>:<alert numbers>`, priority
`90`, `security.queued` in the log) whose brief is the exact versions and the
package-manager commands that adopt them - `npm install next@16.4.2`, `cargo
update -p x --precise 1.2.3`, `uv lock --upgrade-package`, `go get`, `bundle
update --conservative` - and nothing else: change no other package, never a
blanket update, rebuild a committed bundle when the repository has one, run
the project's check, one commit. Because the owner is waiting on it, a
security job leases with the owner's own work, ahead of the fleet's and
alone through the load governor's express lane; its worker brief names the
lockfile change as the job instead of forbidding it (every other job still
reads "never change a lockfile - dependencies are Tier 2"). A patched version
younger than the rule is not queued yet: the daemon logs the date it becomes
eligible (`security.waiting`) and queues it on the first pass after that.
Alerts a job already covers - queued, running, or finished within the last
24 hours, while its pull request may still be open - are not queued again.
What the watcher cannot take it hands to you, once each, as a `finding` marked
for the owner on the project's `security/dependabot` thread, where it waits in
the console's For you inbox with two buttons: a fix that
needs a **major upgrade** (`security.major`; a Tier 2 change by the
constitution, so the finding asks "Yes: Upgrade it / No: Leave it to me" and
your yes queues the upgrade as a security job - or set
`dependencies.major_updates auto` and the watcher queues major upgrades like
any other fix, to the newest eligible version of the patched line, and
answers the finding that asked you itself (`security.auto_major`, naming the
job) so the question leaves your inbox the moment the setting decides it), an alert
whose advisory names **no patched version** yet (`security.unfixable`, "Yes:
Look for a way around it / No: Keep watching"; the update is queued the
moment one is published and old enough), an **ecosystem the watcher does
not read** (`security.unsupported`: it reads npm, Cargo, pip, Go and
RubyGems), and alerts **switched off** on GitHub or unreadable to the
signed-in `gh` account (`security.disabled`, with the setting to turn on).
When an alert behind a finding is closed on GitHub - fixed by hand, dismissed,
or gone with the dependency - the watcher answers the finding itself
(`security.closed`) so nothing stale waits for you.
`dependencies.security_updates off` stops the watcher; the watcher's memory
per project is the `meta` row `dependabot:<slug>`.

**The pull request.** The local landing read the rule itself: it diffed the
pins between the default branch and the branch (`Cargo.lock`,
`package-lock.json`, `pnpm-lock.yaml` in every shape pnpm has written,
`yarn.lock` classic and berry, `uv.lock`, `Gemfile.lock`), asked the registry
how old every new version was, recorded a `land.release-age` event naming the
versions adopted, and held a branch with a version younger than the rule
(reason `release-age`, with the date it could land from), a jump past the
compatible range (`next 16.3.8 -> 17.0.0`), a lockfile it did not read
(`poetry.lock`, `go.sum`, `bun.lock`) or a registry it could not ask - with
one exception, a branch the **security watcher** queued (job source
`security:…`) past the compatible range when `dependencies.major_updates` is
`auto`, which carried `authorized: dependencies.major_updates` in its event.
That reading still runs where a branch lands locally: a project without an
upstream and `pulse land --branch B --local` ([Moving from the local
landing](#moving-from-the-local-landing)). A pull request carries no
release-age reading today: the watcher already waits for a patched version to
be the rule's age before it queues the update, and a pull request that bumps
a lockfile is read by the reviewer like any other - its rule looks at
guarded paths and who pushed the branch, not versions ([The complexity
rule](review.md#the-complexity-rule)). A lockfile change you want to see
before it merges is one to set `review.merge=ask` for, until the pull request
body carries the versions and their ages.

## Credentials

"With the git credentials this machine already has" means the daemon is only
as unattended as your credentials are. An SSH key that lives in an agent
which asks you before each use (1Password's SSH agent, a hardware key) makes
every push wait for a click that never comes at 3 a.m.: `push failed:
sign_and_send_pubkey: signing failed ... communication with agent failed`
lines in the daemon log, branches piling up undelivered. No vault whitelists a
process for good - 1Password's agent remembers an approval only until the app
locks - so the honest answer is a key of the daemon's own (ADR-041):

- Make the daemon a key GitHub knows and that can do nothing but push this
  repository: a **deploy key with write access** on the repository (or a
  machine user's key). In 1Password, keep it as an SSH Key item in a vault of
  its own, say `PULSE`, and create a **Service Account** (Developer > Service
  Accounts) with **read-only** access to that vault only - service accounts
  cannot see your Private vault and never need the app to be unlocked.
- `pulse cred token` - paste the service-account token once. It goes to the
  login Keychain on macOS (service `pulse`, account
  `OP_SERVICE_ACCOUNT_TOKEN`), to `$PULSE_HOME/secrets/` as a 0600 file
  elsewhere, never on a command line, never in the store.
- `pulse cred set "op://PULSE/pulse push key/private key?ssh-format=openssh"`
  - the setting `push.credential` holds a **reference**, never the key. `set`
  reads the key, prints its fingerprint and public key, probes every
  project's upstream with a dry-run push through that key alone, and saves
  the reference only when the remote answers. A key the remote refuses names
  the public key to add as a deploy key and saves nothing.
- `pulse cred status` (also `--json`) says what is set and where the token was
  found; `pulse cred clear [--token]` returns the daemon to your credentials.

The same steps live in the hub (ADR-042): Settings > **Push key** > *Choose a
key…* opens a picker that lists the providers this machine has (1Password,
the macOS Keychain, a key file), takes the service-account token once, lets
you browse the vaults and SSH Key items the token can see and pick one, then
**Test and save** runs the same read-and-probe as `pulse cred set`. A refused
key shows its public key to add as a deploy key and saves nothing; *Remove*
is `pulse cred clear`. The hub route (`/api/v1/cred`) answers only on
loopback, only with the `X-Pulse-Cred` header, and records the change as
`human:dashboard`, so the browser gets no more than the CLI does - the token
and the key never pass through the settings store or the page.

Without 1Password, `keychain://SERVICE/ACCOUNT` names a macOS Keychain item
holding the key, and `file:///ABSOLUTE/PATH` a 0600 file (a symlink or a
looser mode is refused). Whatever the reference, the daemon resolves it when
it pushes a branch and when it fetches the upstream to fast-forward the local
default branch, writes the key 0600 into a 0700 directory under
`$PULSE_HOME/tmp` for the seconds of the call, runs `ssh -oBatchMode=yes
-oIdentitiesOnly=yes -oIdentityAgent=none -i <that file>` for exactly that
push or fetch, and removes it. A key that cannot be resolved holds the
delivery (`land.pr-held` with `push credential: <why>` in its note, shown on
the task's row in the hub and by `pulse land`); the daemon never falls back to your
credentials, and no reference is resolved inside a worker - the one exception
is an MCP server's secret, which `pulse mcp` resolves on the local executor
for the request it sends and never prints (see [Connecting the outside
world](board.md#connecting-the-outside-world-pulse-mcp)). Only the owner runs `pulse
cred token|set|clear` or sets `push.credential`; agents may read `pulse cred
status`. Rolling back is `pulse cred clear` (ADR-037's behaviour
returns at once), then revoke the service account and the deploy key.

Where this runs: the daemon is a launchd agent on macOS and a `systemd
--user` service on Linux, and the credential behaves the same on both - the
one difference is where the service-account token rests (the login Keychain
on macOS, a 0600 file under `$PULSE_HOME/secrets/` on Linux, or the
`OP_SERVICE_ACCOUNT_TOKEN` environment variable on either) and that
`keychain://` is refused on Linux with a pointer to `op://` or `file://`.
1Password's Linux agent asks the same questions as the macOS one, and a
`systemd --user` service does not even see its socket, so the reasons for a
key of the daemon's own hold there too. Windows is not a host for the daemon
today - the module does not build there (process groups, signals and disk
statistics are POSIX-only), so nothing in this section applies until that is
a decision of its own.
