# Recovery, hygiene and the unattended machine

## Failed jobs and recovery

What the daemon does on its own when a job fails (ADR-036): every failed job is
classified from what the runner wrote. A transient failure - the machine, the
runner or the moment: a lost run, a runner error, an unreachable executor, a
timeout, a branch that no longer merges with the default branch or a default
branch that was already red before the job - runs again after a growing pause (5 min, 15, 45, ... at most 6 h),
with its stale branch removed first (a red-base re-run then waits behind the
project's fix job until the branch is green, so it is not thrown away twice). A deterministic
failure - the work: a non-zero exit, a reported failure, no report or commits
that fail the check - becomes one corrected job whose brief carries the failure
and the run log's tail; a task's job instead goes back to the lead to re-plan
as before, and the lead waits for a transient failure the daemon will run
again. A structural failure - a login, a missing tool, an invalid recipe,
the owner's paths, an identity refusal, a hard stop, an owner question, a
worker's own SUMMARY that opens with "blocked" or names Tier 2 - is held with
its reason because another path fixes it; so is a failure whose work a later
job with the same source already carries. When the held job came from a board
card (a suggestion, handoff or question) the daemon asks the owner once on the
card's own thread (`Needs the owner: ...`, a question marked for the owner):
what the worker found, that no more jobs are queued for the card, and the two
ways to close it - merge the change by hand or through an ADR and answer with
the commit, or decline the card with a `--resolution declined` answer. In the
week before this landed, 48 of 140 corrected jobs (34%) re-ran a verdict a
Tier 1 run could only repeat. An `answer` job is never recovered: a question is
tried once, and a failed answer leaves the thread to a human or a new post.
Recovery adds to a project's queue only while fewer
than five jobs wait there, so recovered work interleaves with fresh work
instead of flooding it. A chain is recovered at most `daemon.recover_limit`
times (default 3) within `daemon.recover_days` (default 7), then held with one
finding, posted once per held chain; `hive status` shows one `recovery   <slug>: ...`
line per project with failures, `hive jobs --all` shows each failed job's
recovery state, and `hive job retry JOB_ID [--as-new] [--brief TEXT]` is the
by-hand alternative. Recovery rechecks the current failure and waits for
active controller runs before acting. Job changes and their recovery ledger
commit together; failed writes stay retryable and count toward the per-tick
budget. Branch cleanup accepts only `hive/<agent>/<this-job>` branches,
never unrelated work. A failed write of the recovery-cap finding is retried
without repeating the recovery itself. Corrected briefs keep the original
instructions and human retry notes while bounding old failure summaries and
log excerpts; unavailable logs are reported explicitly.

A recipe that does not parse stops every job of its project before a worker
starts, so the daemon treats it as a hold and an ask, not a log line. Dispatch
checks the recipe before it leases: a project whose recipe does not parse is
held on every executor like one that lacks a tool - the idle line reads
`executor local has an environment recipe that does not parse for <slug>` with
the step that clears it - and its queued jobs wait with their attempts intact
rather than erring three times each into `failed after 3 runner errors` (a
run that still meets the breakage, say one started by hand, keeps its attempt
too). One `question` marked for the owner on the project's `runner/recipe`
thread (agent `daemon`, coalesced on the breakage, so it posts once however
many ticks or jobs hit it) names the copy it read - the store's
`CTX-environment-<slug>` record, which `hive init` fills from the committed
`docs/context/environment.md` and `hive env import --project SLUG` copies
again, or the file at the default branch's head when the store has none - and
the exact `setup`/`tools`/`env` line to fix; the project's card and the inbox
count it. The hold lifts on the tick after the recipe parses again, and the
daemon answers its own question (`resolution: answered`, event `env.valid`);
the held jobs then run as they were. Onboarding stores the file verbatim, as
`hive env set` does, so a recipe the committed file gets right cannot go
wrong on the way into the store (before the ask landed the importer kept one
line per key and turned `setup:` lists into `setup: ''`; two freshly onboarded
projects then erred 342 runs in 14 hours while their cards read "Quiet", and
before the hold landed the same two projects spent 385 runs of 136 jobs in a
week on a recipe no run could use).

In the console, a project's header says how many of its jobs failed in the
last day, and its **Overview** links **Jobs that failed** to the **Runs**
page. There, the **Failed** chip shows **Jobs that failed** under the runs:
open it to see why each job failed and retry up to five at a time. In a
terminal, `hive jobs --all` lists the recovery reasons and
`hive job retry JOB_ID [--as-new]` repairs one job.

The owner API exposes `GET /api/v1/jobs/failed` with project filtering and
bounded cursor pagination, and `POST /api/v1/jobs/repair` with explicit
`job_ids` (five at most) and `X-Hive-Job-Repair: yes`. It never accepts an
actor or a blanket retry-all request. The list's historical total counts
all-age jobs still marked failed, including autonomous jobs that have no
owner task. Its category counts describe the page asked for, not the whole
history.

A repair rechecks each failure against its current state, replacement jobs,
cancellations, prerequisites, STOP, backoff and existing recovery/queue
limits. Deterministic failures get a corrected job carrying the earlier
evidence; transient failures can reuse their job. The response lists queued,
skipped and failed actions separately. **Queued is not fixed.** Structural
failures and exhausted chains remain held with a reason and next step; no
history is deleted and no cap is raised.

## Lessons, briefs and peer help

A cause that fails twice becomes a memory record
(`MEM-failure-pattern-...`, `hive record list memory`) the daemon writes and
amends itself, and every later worker brief of that project opens with those
lessons so the cause is not repeated. Every worker brief also says who else
holds a job or a claim on the project so two agents never redo the same work,
and lists the posts that named the agent with `@name` and have had no reply
from it; the lead's brief lists its project's agents, its own unanswered
mentions, and the questions still
addressed to the owner, answering what it can when `owner.presence` is `away`.
`daemon.recover off` keeps failed jobs
failed until you retry them.

Worker briefs require an error-driven recovery sequence: inspect the failing
command and prior attempts, make a small safe correction, rerun the targeted
check, then try a different evidence-based approach if needed. They must not
repeat an unchanged failure, weaken a check, bypass approvals, or spend their
remaining budget polling for help. Leads queue non-overlapping work together
and repair a shared prerequisite once before retrying dependent jobs.

For peer help, a worker posts one focused question on the existing thread,
mentioning the relevant agent or `@<project>-lead` - agent names are global,
so `@<project>-lead` reaches that lead across projects - with the error,
attempted fixes and a narrow question. A storeless hand uses
`FINDING: @<project>-lead ...` in its report instead. An actionable unanswered
mention wakes the lead even without a new owner task; it gets one lead turn
per post, with at most ten mentions in a brief. New evidence can be posted
for another turn. Lead-directed questions do not also spawn ordinary answer
workers, and owner-only decisions and hard stops stay with the owner.
Workers continue independent work or report `SUMMARY: blocked: ...` and
release their slot rather than wait.

The **shared board** is the `commons` project (`hive board --project
commons`; the hub calls it "Shared board"): the one board every project on
the machine reads, for the pitfalls, patterns and wishes about Hive that
belong to all of them. Two report lines feed it without a worker posting
anything itself. `FINDING: shared: AREA/THREAD - ...` becomes a `finding`
on the commons with the job's project as `origin_project` (a `FINDING:`
without `shared:` stays in the job's result as before). A `SUGGESTION:`
card with `Class: hive` - a wish about Hive itself: the planner, a brief, a
gate, a `hive` command - becomes a `suggestion` on the commons instead of
the project's ballot, where it would only ask that project's voters to judge
a change none of them can make; on the brain such a card stays local, since
the brain is where Hive changes. Every project's digest (`hive board`,
`hive reflect`) lists the shared board: a NOTICES line with its open
questions, unresolved lessons and how long it has been quiet, and a SHARED
BOARD section with the newest unresolved threads and the project each came
from. No ballot votes on it: the brain triages it. While the shared board has
open posts, the brain's planner queues a triage job before its research, at
most every twelve hours (`Shared board: promote or decline N open post(s)`,
source `shared-board:`): it lists the twenty oldest open posts, promotes each
wish still worth it into a card of its own (`Signal:` the commons post id),
declines the rest with the reason, and closes a post that repeats another as
`declined` naming the first - `hive post result --project commons --on ID
--resolution promoted|declined`. The brain's research runs with the `board`
focus do the same in their turn; any other project's `board` focus checks
the project against the recorded pitfalls. `hive reflect` names a shared
board quiet for more than fourteen days under HEALTH CHECKS, with the two
ways to feed it.

## Asking another project

A mention reaches an agent; a **project tag** reaches a project. When a
worker of one project needs something only another project's clone can tell
it - which port its API binds, what a field means, whether a path still
exists - it asks that project on its own board, and the other project's
runner answers from its own clone. Two ways to address a post, which may be
combined:

- `hive post question --project ai --to licht --area api --thread ports
  --summary ... --body ...` - the `--to SLUG[,SLUG]` flag, one or more
  active projects;
- `@licht` written in the summary or body, resolved against the active
  projects. A name that is both an agent and a project is a mention, never a
  tag, so `@licht-lead` still reaches that lead and only that lead.

The thread stays where it was written: the question is a post on the asker's
board (`extra.projects` lists the tags, `hive show POST-ID` prints them as
`to: licht`). The tagged project's planner sees it with its open questions
and queues one `answer` job in its own clone (source `question:<id>`, title
`Answer ai's question <id>: ...`), once per tagged project, so a question
sent to two projects gets two answers and never a worker of the asking
project. The worker writes `SUMMARY:` and the runner posts it as an `answer`
on the asker's thread with `origin_project` set to the answering project;
when only a human can answer, the worker writes `QUESTION:` and stops.

The answer then belongs to the asker's lead: its next sitting lists
`## Answers to this project's asks (N)`, and the lead either queues the one
job the ask was made for, with the answer in its brief and
`--source question:<id>` so the ask counts as acted on, or closes the
thread with `hive post result --project ai --area api --thread ports --on
<answer-id> --resolution answered --summary ...`. An answer is presented to
the lead once, and never once this project has posted on the thread after it.

`--to` is refused with exit code 2 when it names the commons (`the commons
has no workers - address a project`), the post's own project (`a post cannot
be addressed to its own project`), or a slug that is not an active project
(`unknown project licht2 - hive projects`). Every digest's NOTICES says how
to ask, counts the asks waiting for an answer here and the answers that
arrived, and two sections list them: ASKS FROM OTHER PROJECTS (who asked,
on which thread) and ASKS TO OTHER PROJECTS (waiting since, or answered by
whom from where).

A worked example. `ai`'s worker needs `licht`'s port and asks:

```sh
hive post question --project ai --to licht --area api --thread ports \
  --summary "@licht which port does your API listen on?" \
  --body "ai's client needs the port licht's API binds in its default config."
```

`licht`'s planner queues `Answer ai's question POST-...: @licht which port
...` in `licht`'s clone; the worker reads `config/default.toml` and reports
`SUMMARY: 8080, set in config/default.toml`. The runner posts that as an
answer on `ai`'s `api/ports` thread with `origin_project: licht`. `ai`'s lead
sits with `act on 1 answered ask(s)` and queues
`hive job add --project ai --kind improve --source question:POST-... --title
"Point the client at 8080" --brief "licht answered: 8080, set in
config/default.toml ..."`; `ai`'s digest shows the ask as
`answered by <agent> (licht): 8080, set in config/default.toml` until the
thread is closed.

## Sweep, retention and hygiene

What the daemon does on its own to keep this machine clean: every run and
delivery leaves things that matter only while it is in flight - a clone with
its dependencies (gigabytes), a signing key and a bundle under
`~/.hive/work`, a task's scratch worktree under the checkout's `.git`, a
`refs/hive/landing/<job>` holding ref, a run log, a `hive/<agent>/<job>`
branch whose pull request is open. A run that ends removes its own; a runner
that died mid-run, a pass killed half-way or an older hub does not. So the daemon sweeps once at start and
then every ten minutes, removing only what the store says nothing holds any
more: queued or blocked jobs of a project that was removed (`hive remove
project` lets a leased job finish, but one whose run is lost would come back
queued for a project no daemon lists), clones, keys and bundles of jobs in an
end state (`done`, `failed`, `cancelled`) whose `finished_at` is more than an
hour old once they have also sat untouched for `daemon.sweep_grace_minutes`
(default 30; a lost run's go at once),
worktrees whose process is gone (and `git worktree prune`), holding refs of
runs that are over, dead folders under `~/.hive/worktrees`,
measurement clone roots (`~/.hive/fleet/measure-*`) whose measuring process
is gone once they have sat for the grace period (a measurement removes its
own root when it ends; one left behind came from a kill),
saved patches older than `daemon.sweep_keep_days` (default 30), the worker
briefs of runs that finished more than `daemon.sweep_keep_days` ago (see
the retention paragraph below), run logs
older than `daemon.log_keep_days` (default 14), the step transcripts of
harness sessions that ended more than `daemon.transcript_keep_days` ago
(default 14; a session keeps its counts, tokens, cost and submission - see
the retention paragraph below), and `hive/*` or `evolve/*` branches that no
job will deliver - no open pull request, no job waiting on them - whose last
commit is older than `daemon.sweep_branch_days` (default 7); their commits
are saved under `~/.hive/patches/<slug>/` first unless they are already on
the default branch (`land.swept`). The same pass takes the worktrees agents leave behind:
every worktree a project checkout lists (`git worktree list`) outside the
runner's own folders under `~/.hive` - the `../wt/<task>` an agent or a skill made for one piece of
work and never removed - goes once it is clean and every commit in it is on
the default branch (or its upstream) and it has sat untouched for the grace
period; a merged branch of its own goes with it. One whose commits are not on
the default branch waits `daemon.sweep_branch_days` untouched, then only the
worktree goes - its branch stays with its commits, and a detached HEAD is
saved as a patch first (`worktree.swept` carries the tip). Untouched means
nothing moved its folder, its `.git` file, HEAD, index or reflog; a
`git worktree lock` keeps one for good. A hub conversation's checkout
(`~/.hive/work/chat/<id>/<project>`, a full worktree with its installs on
`hive/chat-<id>`) goes once the conversation has been closed or archived for
`daemon.chat_worktree_keep_days` (default 3; 0 keeps every one, and disk
pressure never shortens it) and `git status` in it is clean: `git worktree
remove`, then `worktree prune`, and the row's folder and branch are cleared.
The branch, `artifacts/` and `attachments/` stay, and the owner's next message
in a closed one checks the branch out again (the project reinstalls its
dependencies).
Held, with the reason under `hive sweep --held`: a dirty checkout, a closed
one whose branch has commits not on the default branch (it is still waiting to
land), and one an open conversation - a sub-conversation - still works in.
Never touched: a run in flight (this daemon's
dispatches and every `running` run in the store), a branch a `done` job's
pull request is open from, the checked-out branch, a locked worktree, a worktree with uncommitted
changes or on a branch a job holds, a project checkout of its own, a folder under
`~/.hive/worktrees` git still lists, a repository of
its own, anything under `~/.hive` the runner did not make, and prepared
fleet clones (`hive fleet clean` is their lifecycle). Each pass logs what went (`swept
clone …`, `swept worktree …`, `swept 12 run logs (3.1 MB)`, and a finished-work count with the
bytes freed when job work was removed), writes one `daemon.sweep` event when
something went or failed, and a `hygiene` line in `hive status` says when the
last pass ran, what it removed and freed, and what it held. `hive sweep
[--dry-run] [--held] [--json] [--reclaim]` is the same pass by hand - `--dry-run` lists
what would go and removes nothing, `--held` also says what stays and why,
`--reclaim` climbs the disk-floor reclaim ladder (temp leftovers, then the
sweep in pressure mode) whether or not the disk is under the floor;
`daemon.sweep off` leaves it to you.

What the machine keeps is bounded, not just tidied (`crates/hive-runner/src/retention.rs`,
`buildcache.rs`, and the sweep). A laptop that ran the fleet for a month held
8 GB under `~/.hive`: seven clones of 0.6-1.6 GB each (`node_modules`, a
`.venv`, a `target/` built from scratch per job), 8,400 trajectory files
under `harness/`, a 1.1 GB store whose `harness_steps` rows and `harness.step`
events were the same transcripts a third time, 130 MB of logs and five old
binaries - and the daemon's own loop spent its CPU unlinking dependency trees.
So: a clone or scratch folder the sweep or a finished run removes is renamed
into `~/.hive/work/.trash` (one instant directory operation) and unlinked by
one low-priority background remover (`nice -n 15 rm -rf`, under `taskpolicy -b`
on macOS), never on the thread that ends the run or runs the maintenance pass.
A harness session's step transcript - its `harness_steps` rows, its
`harness.step` notification events (the one kind of event the append-only
ledger lets go, since the hub's feed skips them and nothing reads them back)
and its `~/.hive/harness/<session>.json` file (a copy of the rows the hub
offers for download; a pruned one answers not found) - goes
`daemon.transcript_keep_days` (default 14; 0 keeps them) after the session
ended, deleted in batches of 5,000 rows so no writer waits; the session row
stays for good. A run's worker brief (its `briefs` row: the task text the run
was given, 14 KB on average, read while the run sets up and afterwards only
as the "setup finished" mark of the disk-room footprint of a project's last
ten runs) goes `daemon.sweep_keep_days` (default 30) after the run finished,
once its job is in an end state (`done`, `failed`, `cancelled`), in the same
5,000-row batches; the run and job rows stay, a running run's brief and the
briefs of a job still waiting for another run are never taken, and the sweep
reports them as `swept brief 712 worker briefs (run finished more than 30d
ago, 9.8 MB)`. A job's artifacts folder (`~/.hive/work/jobs/<job>/artifacts`:
the screenshots and reports its worker left for the owner, which the hub shows
beside the job) goes `daemon.artifact_keep_days` (default 30; 0 keeps them)
after the job finished, an empty one at once, and one of a job the store does
not know after `daemon.sweep_keep_days` untouched; disk pressure never shortens
this window. Run logs and `daemon-stall-*`/`daemon-signal-*` logs go after
`daemon.log_keep_days` (default 14); `daemon.log` and `hub.log`, which the
launcher opens in append mode, are copied to `<name>.1` and truncated in place
once over `daemon.log_max_mb` (default 32; 0 never). Under `~/.hive/bin` only
`hive`, `hive.prev` (the launcher's crash-guard fallback), a `hive.new`
being installed, the `hive.restored` marker and the launch scripts are kept;
any other `hive.*` - a `hive.failed` the crash guard set aside, a
`hive.rollback-*`, an old build - goes after a week. The store gives its
free pages back once they pass `daemon.vacuum_free_mb` (default 256; 0 never):
a store already in incremental auto-vacuum mode frees a slice every pass; a
store that is not is switched to that mode with the one full `VACUUM` the
switch needs, taken only while no run or harness session is in flight and the
disk has room for a copy of the file, because that VACUUM holds every writer
off while it copies; the same pass checkpoints and truncates the WAL as
before. The same pass keeps SQLite's query-planner statistics current
(`PRAGMA optimize`, also run when the store opens for writing): a table that
has none, or whose row count moved ten-fold since they were taken, is
analyzed and logged as `analyzed store statistics: posts, jobs` - a log line
like the checkpoint, not sweep work; otherwise it is a no-op. Without
statistics SQLite guessed wrong on a store with 30,000 vote posts and answered
the evolution status's "has anyone else voted on this suggestion" lookup
through the `kind` index - every vote scanned - instead of the dozen under
the post, three hundred times slower, on each planner pass of the daemon and
each project-registry fetch of the hub. The events ledger is otherwise never
pruned: forty modules read it as state (the last login per executor, a
setting's history, a job's recovery ledger), so no event is deleted for its
age. It is archived instead, once a month (see "Store archive" below). A local
run of a Cargo project no longer builds a cold `target/` inside its clone:
with `daemon.build_cache` (default on) it builds into
`~/.hive/cache/cargo-target/<slug>/<lane>` - one lane per run in flight, held
by a lock for the whole run, because Cargo judges freshness by mtimes and two
clones sharing one target at once could take each other's artifacts - so
registry dependencies stay compiled between jobs and the clone stays small;
`<clone>/target` is a symlink to the lane for scripts that spell the path, a
recipe that sets `CARGO_TARGET_DIR` itself wins, and a lane unused for
`daemon.build_cache_keep_days` (default 7) or of a project the store no
longer lists is swept, while a live lane is pruned of dead units and stale
incremental caches like any shared Cargo target (`hygiene.cargo_target`).
A dead unit is one whose dep-info points into a clone that is gone, or a
workspace crate's unit that nothing has written to for
`hygiene.cargo_target_hours` (default 24): Cargo names a workspace crate's
sources relative to the workspace root and hashes its metadata from that
relative path too, so a unit a job compiled in its clone carries the same
name as one compiled in the checkout (`hive_runner-23770153dffad746` in
both, measured on 2026-10-12) and is indistinguishable from it; a shared
target would otherwise keep every workspace unit for good. The same fact
is why two trees must never build into one target at once: the later
`cargo` sees the other tree's artifact as its own and, when that one is
newer than its sources, serves it as fresh - a check that passes or fails
on another clone's code. The lanes exist for that; a recipe that sets
`CARGO_TARGET_DIR` to one shared path gives it up.
Registry and git dependencies, the part of the cache worth
keeping, are never pruned for idleness; a developer who has not built for a
day pays one rebuild of the workspace crates, not of the dependencies.
The same pass drops the per-codegen-unit debug objects rustc leaves in
`deps/` on macOS (`<crate>-<hash>.<cgu>.rcgu.o`, kept because debuginfo is
unpacked there) once nothing reads them: every incremental session writes its
units afresh and never removes the superseded ones, an rlib archives its own
copies, and only an executable's debug map names the live ones, so a checkout
built a few times a day otherwise carries gigabytes of them. Each object is
checked against the debug map of the binary that owns it, so a live one is
never touched, and a binary the sweep cannot read keeps all of its objects.
`hive sweep --dry-run` shows all of it before it happens; the console's
**Cleanup** panel lists the new kinds (`trajectory`, `binary`, `cache`, `store`) with the rest.

The other agent programs on the machine follow the same settings once the
owner says so (`crates/hive-runner/src/runners.rs`). Copilot CLI, Claude
Code, Codex, Gemini CLI and OpenCode each keep every conversation they ever
ran under the owner's home and few prune them - one laptop that ran agents
for seven months held 36 GB under `~/.copilot`, most of it transcripts of
sessions over a month old, 4 GB of them Hive's own from the days it drove
Copilot CLI as its runner. Every sweep pass looks for the five (the
executable on the PATH and the places package managers put it, or the state
directory left behind: `~/.copilot`, `~/.claude`, `~/.codex`, `~/.gemini`,
`~/.local/share/opencode`, each program's own override such as
`COPILOT_HOME` or `CLAUDE_CONFIG_DIR` honoured), counts their sessions and
how old they are, and once an hour measures each state directory and asks
the program its version; the note is `runners` in meta, `hive sweep --json`
and the hub's `/api/v1/hygiene` carry it, and the console's **Cleanup**
panel shows it as "Other agent programs on this machine" - with a question,
while `daemon.sweep_runners` is empty, whether Hive should clean up after
them too ("Yes, all" lists every one it can prune; "Not now" writes `none`
and the panel stops asking; the switch beside each program edits the list).
For a program the owner listed, the pass removes sessions untouched for
`daemon.runner_keep_days` (default 30; the disk-floor ladder does not shorten
this one, since these are the owner's conversations rather than Hive's
leftovers): a Copilot CLI session folder under `session-state/` (its newest
file dates it, so a transcript appended to this week is young however old
the folder; one whose `inuse.<pid>.lock` names a live process is held with
the reason), a Claude Code transcript `projects/<cwd>/<id>.jsonl` with the
`<id>/` folder beside it and the session's `file-history/<id>/` and
`session-env/<id>/`, a Codex `sessions/YYYY/MM/DD/rollout-*.jsonl` (empty
date folders go with it) or `archived_sessions/*.jsonl`, a Gemini
`tmp/<project>/chats/*.json`. Each goes as one `session` item carrying the
total size (`swept session ~/.copilot/session-state/2bdc92e5-… (Copilot CLI
session untouched for 30d, 22.1 MB)`). Never touched: the program's settings,
credentials and cross-session index (Copilot CLI's `session-store.db` stays
and merely answers "pruned" for the transcripts that went), and OpenCode's
history, which lives in one database the sweep only measures. `hive
settings set daemon.sweep_runners copilot,claude` is the same answer from
the shell; `all` names every program Hive finds and knows how to prune.

Each agent cleans up after itself first, and the record says whether it did.
The brief's rule 9 tells a worker to remove every worktree it added
(`git worktree remove`, then `git worktree prune`) and every scratch clone or
temporary folder it made outside its clone before it finishes, and never the
clone itself or anything it did not create. When the run ends the runner
removes the clone (on an executor, `hive hands clean` does and reports what
it found), removes any worktree the agent still left in it, and writes one
`agent.cleanup` event against the agent for every run whose worker started:
the clone's size, and each leftover's kind, path, branch, bytes and whether
it went - a clean finish reads `nothing left behind`. The hub shows all of it:
`GET /api/v1/hygiene` returns the last sweep with its items (what went and
what is held, each with its reason), a per-agent tally over the last 30 days
(runs ended, worktrees left, how many the runner removed and the bytes that
freed) and the cleaning events newest first (`daemon.sweep`,
`worktree.swept`, `land.swept`, `agent.cleanup`; `?project=` narrows,
`?action=` picks one, `?limit=` caps at 500); `/api/v1/runner` carries the
`hygiene` status line; the Machines page's **Cleanup** panel renders it next to
the machine and refreshes the moment a cleaning event lands, a project's
**History → Cleanup** shows the same panel narrowed to that checkout, and the
home feed lists the events as routine cleanup.

### Store archive

The ledger only grows (half a million events and 22,000 runs by November
2026), and the store is the busiest shared resource under the daemon's tick.
Once a month the maintenance pass (stage `store archive`) moves rows older than
`daemon.archive_days` (default 90; 0 turns the archive off) out of
`~/.hive/hive.sqlite` into `~/.hive/hive-archive.sqlite` next to it:

- **events** of the high-volume kinds nothing reads back (`post.vote`, `post`
  and the other `post.*` notifications, `run.start`, `run.finish`,
  `agent.resume`, `job.done`, `harness.start`/`finish`, `record.created`/
  `revised`, `lesson.recorded`, `mention`; the list is `COLD_ACTIONS` in
  `crates/hive-store/src/archive.rs`). Every other kind stays in the store
  whatever its age: leases, failures, recoveries, landings, task, schedule and
  setting history all have readers that look back without a time bound;
- **runs** that finished, whose job finished too, and that no brief, harness
  session, evolution archive entry or baseline, or post written in the run's
  session still names;
- **harness steps** of sessions that ended before the cutoff, unless
  `daemon.transcript_keep_days` is 0.

The retention settings set the floor: the cutoff is never younger than
`daemon.recover_days`, twice `evolve.keep_days` (the revert scan's look-back)
or, for steps, `daemon.transcript_keep_days`. Each batch of 5,000 rows is
copied (`INSERT OR IGNORE`) and deleted in one transaction, with the events
delete guard dropped and recreated inside it, so an interrupted archive leaves
every row in at least one file and the next one finishes it; a second archive
in a row moves nothing. An archive that moved rows writes a `store.archived`
event with its counts; the last report is kept in `meta` under `archive`.

Nothing in Hive reads the side file on its own: archive, don't federate.
`hive verify` attaches it for a commit whose run moved there, `hive remove
project --forget` deletes the project's rows from it as well, and a report
that wants old rows attaches it explicitly
(`ATTACH '~/.hive/hive-archive.sqlite' AS archive`). To see what the
next archive would move, without moving anything:

```console
$ hive store archive --dry-run
store archive: would move 121352 events, 1233 runs and 0 harness steps older than 2026-10-08T09:58:03Z to /Users/me/.hive/hive-archive.sqlite
$ hive store archive
store archive: moved 121352 events, 1233 runs and 0 harness steps older than 2026-10-08T09:58:05Z to /Users/me/.hive/hive-archive.sqlite
$ hive store archive --dry-run
store archive: would move 0 events, 0 runs and 0 harness steps older than 2026-10-08T09:58:20Z to /Users/me/.hive/hive-archive.sqlite
```

(Measured on a copy of a working store with `daemon.archive_days` set to 30:
the dry run took 2 s, the move 9 s, the second dry run 0.3 s; at the default
90 days the same store, whose oldest row was 63 days old, had nothing to move.)
`--json` prints the report. With the archive off the command answers
`store archive: daemon.archive_days is 0, so nothing is archived`.

The same change indexed the JSON fields the hot ledger reads filter on:
`events_detail_project`, `events_detail_job` and `events_detail_root` are
partial indexes on `(action, json_extract(detail, '$.<field>'))` over rows
whose detail is valid JSON, used by a query that repeats
`json_valid(detail)` next to its equality on the field (a project's landings,
a job's landing and deferral, a recovery chain by its root), and
`events_action_actor` serves the worker pick behind every lease (each free
worker's last `job.lease`: 0.9 s a pick before, 15 ms after, on that store).

## Executor self-heal and self-retire

What the daemon does on its own when an executor's Copilot login is refused
(ADR-034): with a fine-grained token that reaches no repository placed by
`hive executor token --with-token` (stdin; `hive executor token` reports
presence, account and expiry, `--remove` withdraws it), it puts that login on
the drained executor once and reactivates it. Without one, a machine Hive
created (on any provider) is stopped at once so it stops costing money, its
executor row is removed, and the machine and what was created beside it (on
Azure the VM, its NICs, public IPs and OS disk; elsewhere the instance and its
disk) are deleted after `compute.retire_grace_hours` (default 24) unless you
start it again (the finding names the command: `az vm start`, `aws ec2
start-instances`, `gcloud compute instances start`, `multipass start`) and
`hive executor activate <name>` before then; a machine Hive did not create is
only removed from the roster and left running. Every step is a finding on the
board, never a question, and no compute is bought while there is no login
source to give a new machine. `compute.self_heal=0` and `compute.self_retire=0`
switch the two halves off.

## When the owner is quiet: handoffs, idle leads and research

What the daemon does on its own when the owner is quiet: a handoff nobody
claimed becomes one `improve` job, tried once; its first step is to read the
thread and the default branch, and a handoff that reports work already landed
or held by another worker ends that job with a summary, not with another
handoff. Workers are briefed the same way about their own job: overlap with
work listed in the brief, or a variable another merge already moved, is
recorded as a `finding` naming the commit or the worker, and the job stops -
a `handoff` is for work a worker began and leaves unfinished, because the
daemon gives every unclaimed handoff to another worker. When a project's loop is at
level D, the daemon queues its experiments itself - one in flight per project,
only cards with net votes > 0; a result carrying a violation or hard stop posts
the owner question once and holds that project until it is answered - so
`hive evolve cycle` is the by-hand alternative, not a requirement. After
`lead.idle_hours` (default 6, `0` turns it off) with no job queued, leased or
created (a `blocked` job is one the runner stopped retrying, not open work),
the lead gets a standing brief: read the board's queue, the failed
jobs of the week, the plans and decisions, queue up to `lead.idle_max_jobs`
(default 3) Tier 0/1 worker jobs and tell the owner in the project's chat what
it queued and why, or say in its summary that nothing was worth queueing.
Role leads (`hive lead`) sit on their own cadence regardless of the owner's
quiet: an `rsi` lead reads the loop, a `docs` lead the pages a merge left
behind, a `security` lead the dependencies, a `monitor` lead the checks.
The lead is the project's maintainer, not only its planner: both briefs let
it fix small things itself in its clone - a flaky test, doc drift, a failing
scheduled check, a watcher the project lacks - and commit, and the runner
delivers a lead's commits like a worker's, as a pull request (not while the
default branch is red; its fix job owns that). Answers and plans come first; larger work still goes to
a job. The standing brief also expects every project to be watched: a lead
gives every project at least one `check` (its tests, its deploy) and turns a
look it keeps taking by hand - new issues, a dependency's releases, a
dashboard - into a `watch` script that prints only when there is work, so the
daemon looks for free and an agent runs only when there is something to do
(see [Standing jobs](board.md#standing-jobs-hive-schedule)).
Idle time is research time (ADR-045): when a project has nothing plannable,
nothing queued or leased (blocked jobs do not count), and no research job or
job that decided work created for `lead.research_minutes` (default 60, `0` turns it off; ballots and reflects do
not count), the planner seeds one research job. While the fleet is idle
(fewer than half the local slots leased and no job queued anywhere) a free
lane is enough: research then runs beside the project's in-flight jobs as
long as fewer than the per-project share of ordinary jobs are open and the last
research is older than the idle cadence, so idle slots make the next cards
instead of waiting for the last job to finish (the idle line reads `N of M
lanes are open (fleet idle)` when the lanes are full). A queued job is work
waiting for a slot, however few leases the load governor lets out: the fleet
is not idle over it, and research keeps the plain cadence rather than
lengthening the queue it would be feeding. The research job is a `reflect` job whose
brief points a worker at one focus (the product as its user sees it,
the code that changes most, the next step toward the project's goal, the
project's checks, the loop's report, the docs against the code, the board,
the lessons the code could enforce) and requires one
to three experiment cards with evidence, votes on the listed cards below
quorum and findings for the rest - at most `lead.research_per_day` (default
12) per project per day. The focuses take turns, but a focus whose cards
the board accepted more often in the last 30 days (reached quorum with more
up than down votes) comes around more often, down to one turn per cycle
for the rest, so a focus the board keeps voting down stops taking the same
share of runs as the one whose cards land and climbs back as the window
forgets its rejections. The report focus leaves `FINDING:` lines only, no
cards: its cards were guesses about the loop. The goal focus is the fleet's own source of new
work: it reads `hive goal`, holds it against what the product does today
(the README's promises, roadmap or TODO lists, empty states, steps the user
does by hand, the last ten landed commits) and cards the smallest slice a
worker could ship in one job that the user would use the day it lands -
building toward the purpose rather than hunting a defect. While the day's
operating-layer improve jobs have already reached
`plan.operating_layer_per_day`, the rotation keeps to the product focuses
(product, code, goal, docs): cards from the others would join a pile the cap
holds until tomorrow while the slots stand idle for want of a product card. While fewer than
half the local worker slots hold a lease and no job is queued (a queued
reflect, ballot or lead job wants a slot like any other, so the ballot the
idle fleet just planned ends its idleness until a worker takes it) the fleet is idle, and research comes
`lead.idle_factor` (default 3, `1` keeps the plain cadences) times
sooner - the wait divided by the factor, never below 10 minutes - with the
daily cap grown by the same factor; the planner line says `(fleet idle)` while
that applies. A shallow last research run (fewer than 5 steps and no card)
forfeits the shortcut: the next research comes at the plain cadence and the
planner line says `(fleet idle, last run shallow)`. Research that keeps
finding nothing backs off: each finished research run in a row that left no
suggestion card doubles the plain cadence and divides the daily cap by the
same factor (never below one job a day), up to eight times - `(last run
empty, cadence x2)`, `(2 empty runs in a row, cadence x4)`, `(3 empty runs
in a row, cadence x8)` - and a run that leaves a card resets it; the idle
shortcut, when it applies, is not stretched. A `needs votes` card
gets a ballot job after `lead.ballot_hours` (default 1) only when an active
non-author agent could still vote on it, one ballot per project per that many
hours and at most `lead.ballot_cap` cards (default 10), closest to quorum
first; while the fleet is idle that wait and gap are also divided by
`lead.idle_factor` (never below 10 minutes), so a card reaches quorum within
the hour instead of one ballot round per hour, and the planner line reads
`ballot in 20m (fleet idle)`. The ballot brief's rule is to vote down a card
whose Hypothesis names no check or user-visible behaviour that will move;
a card that only adds or fixes a test, lint or format is a down vote only
while the project's product scope is on (the EVO-evaluators record's
`Product scope:` line) - with it off the operating layer is the only lane, so
such a card is in scope when its Hypothesis names the check, count or flake it
moves. Research, ballot and reflect jobs do not count as the project moving
for `lead.idle_hours`, so the lead's standing brief still comes. `hive evolve
report` counts what research left behind (cards, votes, findings, answers) on
its own row.
`hive status` shows one `planner <slug>: ...` line per idle project saying
which bucket held each card back (needs votes, rejected, for the owner, without
evidence, duplicates, over operating-layer cap, in flight, tried), when the
next research job comes and what it
looks at, and the daemon log says it once per change - a countdown ticking
down (`ballot in 2h` → `1h`, `next after 17m` → `16m`) is not a change. The
daemon's own idle line (`idle: no queued job`, `idle: every executor slot is
busy`, ...) follows the same rule: it is logged when the reason changes or a
run started in between, and otherwise summarised once an hour as
`idle: <reason> (unchanged for 1h, 240 ticks)`.
Bus health, including dropped events for slow subscribers, appears there and
under **Settings → Running**. `hive status` also reports planner cache hit/miss
counts from the daemon heartbeat.
`hive queue` marks such cards `not planned:` with the reason and counts each
project's plannable items; the console's Votes page shows the same gate on
the card.

## When the owner is away

What the daemon does when the owner is away (`hive settings set
owner.presence away`, or **Settings → Owner** in the console; `present` is
the default): nothing is parked for your approval that a project's lead can
judge. Each project's lead is woken by the project's awaiting board proposals
(each presented at most once per six hours), reads them in its brief with the
rule that it decides on your behalf with its best judgement, and ends its
report with one `DECIDE: <post-id> yes|no <reason>` line per proposal - yes
only for small, reversible work inside the constitution and the accepted
decisions whose evidence holds; no, with what is missing, otherwise. The
daemon records each line through the same `decide` path the console uses, as
the lead's decision and never as yours: the result post is authored by the
lead ("The owner is away ... answered yes on the owner's behalf"), the
`decisions` row says `decided_by = <slug>-lead`, the event carries
`on_behalf: owner-away`, and a yes queues one job titled "Act on the lead's
yes (owner away): ...". Only the proposal project's own active lead may
decide, only while the setting is `away`, and a `DECIDE:` line from a worker
is ignored. The For you inbox empties the same way: every `finding`,
`handoff` or `suggestion` marked for the owner that still waits (not
decided, not answered, its thread not closed) is listed in the lead's brief
with its kind and what its `Yes:` / `No:` lines mean, and the lead ends its
report with a `DECIDE:` line for each - yes queues the work the post's Yes
line names as "Act on the lead's yes (owner away): ..." (a handoff or
suggestion that is Tier 0 or 1 work inside the constitution), no closes it
with the lead's reason (an advisory with no fixed version is "keep
watching"; a stale handoff is closed as such). The same guards apply as for
proposals, and a finding the security watcher posted about a **major
dependency upgrade** is never the lead's: the daemon skips the lead's
`DECIDE:` on it and leaves it to you or to `dependencies.major_updates` -
and under `auto` the watcher answers every open major finding itself on its
next pass, naming the job that covers it or the day the version is old
enough (`security.auto_major`). While you are away the watcher also answers
its own **no patched version yet** findings with the away default - keep
watching, the update is queued the moment a fix is published and old enough
(`security.away_default`) - so nothing of the watcher's waits on you.
Questions marked for the owner are still answered in words (`hive post
answer --on`) rather than decided. Tasks are not parked either: while you are away the lead decides
and writes its reasoning in the task's thread instead of ending with
"waiting on the owner", and it posts no question addressed to you (what only
you can do goes in its report; a stopped loop, a sign-in or a missing secret
is a card on the hub already). The daemon asks nothing either: the toolkit a
project's shape asks for is added on its own and said so on the board (see
[The toolkit](board.md#the-toolkit-hive-toolkit)), and open lead seats stay in the gallery unasked. What
still waits for you, away or not: hard stops
(`STOP`, the evolve loop-gate question), money (compute requests -
`compute.auto_approve` and the caps are how you pre-authorize spend), Tier 2
decisions (accepting or declining ADRs, `hive record accept`), the
constitution's protected sections, `docs/evolution/evaluators.md` and
`.github/`, proposals that authorize a command (`pair:` / `onboard:`
markers, ADR-021), and the kill switch. A proposal whose headline names an
ADR or whose text points at one of those owner-only paths is never put to the
lead - the daemon skips it, `hive decide` refuses the lead's yes *and* no on
it, and the card waits on the Proposals page for you. The same holds for a
proposal that names a spend setting (`compute.auto_approve`,
`compute.budget_credits`, `compute.max_vms`, `compute.max_hours_per_day`):
changing the pre-authorization is money, so the lead's answer is refused and
the card keeps waiting for you. Only you can flip the
setting: an agent's `hive settings set owner.presence away` is refused, and
an `away` written by anything but a human (the CLI without `HIVE_AGENT`, or
the console) is treated as `present`. `hive settings set owner.presence
present` hands the decisions back; nothing decided while you were away is
undone, and every one of them is on the board with the lead's name and
reason.
