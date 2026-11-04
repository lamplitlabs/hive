---
id: ADR-069
title: Delivery by pull request with an agent reviewer
status: Proposed
date: 2026-11-04
areas:
  - runner
  - hub
  - docs
tags:
  - adr
  - delivery
  - review
supersedes:
  - ADR-024 (the local landing: verify before land, the landing lock, `landing: agents|human`)
  - ADR-037 (`landing.push`: pushing the default branch, the `pulse/landed` pull request)
delivery: not started
delivered: null
---

# ADR-069: Delivery by pull request with an agent reviewer

## Context

Since ADR-024 the daemon merged every finished branch into the project's local
default branch itself: the verify command on the merged result in a throwaway
worktree, then a fast-forward or a `Land …` merge commit, under one landing
lock per project. ADR-037 added `landing.push` to carry the result further -
`main` pushed the default branch, `pr` pushed it to a `pulse/landed` branch
and kept one pull request open from it. PR #53 moved one kind of work, the
owner's tasks, to a pull request per task.

Measured on one machine over the seven days before this decision: 260
landings, of which 22 (8%) were task work. The other 238 - suggestion jobs
120, security updates 39, lead idle work 21, role leads 17, schedules 11,
red-base fixes 7, pipeline fixes 7, research 6, handoffs 3, others 7 - went
through the local landing, serially, with nobody reading the diff. The landing
stalled for reasons unrelated to the work: a lock left by the restart of a
self-update froze a project's queue for 30 minutes at a time, four times in a
day; a verify of 7-10 minutes per branch on the brain serialised every branch
behind it; the owner saw tasks *waiting to land* for hours. The brain project
(PULSE itself) self-updates from its local default branch, so whatever
replaces the landing must keep that branch moving.

## Options Considered

### Option 1: keep the local landing and make it faster

Batch landing (`landing.batch`) and per-project landing workers
(`landing.parallel`) were this option, already built.

- Pros: no forge needed; the merged result is verified on this machine.
- Cons: the verify stays on the owner's machine and serial per project; the
  lock stays; every branch still merges unread; PR #53 leaves two delivery
  paths to keep working.

### Option 2: a pull request for every finished branch, merged by the owner

- Pros: one path; the forge runs the pipeline on each pull request at once;
  the diff is read before it merges.
- Cons: 260 pull requests a week for one person; the fleet's own work waits
  on the owner, which is what the fleet exists to avoid.

### Option 3: a pull request for every finished branch, reviewed by an agent, with a rule for what reaches the owner

- Pros: everything of option 2, and the owner reads only the pull requests
  the rule hands over; the review is a job like any other and is measured
  like one.
- Cons: an agent merges code; the rule must be conservative and the owner
  must be able to turn merging off.

## Decision

Option 3. For a project with an upstream the daemon never merges into the
local default branch. Every finished branch becomes a pull request there: a task's
jobs as one `pulse/task-<id>` pull request, every other finished job's
`pulse/<agent>/<job>` branch as its own, a kept evolve experiment with its
measurement as the body, and a branch proposed by hand or a conversation's
branch ten minutes after its last commit. The pass polls the open pull
requests, records `land.merged` or `land.pr-closed`, and fast-forwards the
local default branch from the upstream on every pass; a dirty checkout is
reported (`land.sync-skipped`) and left alone. `gh` logins are tried in turn
and the one that works is remembered per repository; no token is logged. A
project without an upstream keeps the local landing of ADR-024, since there
is no forge to deliver to.

A `review` job reads each pull request PULSE opened (`review:<slug>:<number>`,
owner priority, one at a time per project; a pull request opened by hand on
the forge is not queued) and ends with one verdict: `pulse review
merge`, `pulse review changes --note` (a follow-up job: a `task` job for a
task's pull request, an `improve` job with source `pr-fix:<slug>:<n>` pushed
onto the head otherwise) or `pulse review owner --note`. The complexity rule
is computed before the job runs, and any reason hands the pull request to
the owner with the reviewer's findings: `review.merge=ask`; more than
`review.max_files` (20) files or `review.max_lines` (600) lines excluding
generated paths; a guarded path (constitution-closed files,
`.github/workflows/**`, `.env*`, secret-looking files); more than twice as
much removed as added with over 200 deletions; a head not under `pulse/` (a
proposed branch, or an experiment's `evolve/<id>`); the reviewer failed twice
on the same head. The hub shows *Needs your review* with the reasons and a
**Merge** button, which records `review.ask` so that the daemon merges the
pull request on GitHub as the owner on its next pass (the same as `pulse
review merge <n> --by owner`); the owner may equally merge or close on
GitHub. `pulse land --branch B --local` remains as the by-hand local merge
and warns that the local default branch will diverge from the upstream.

## Consequences

### Positive

- No landing lock, no verify on the owner's machine, no serial path: a branch
  is delivered in the seconds a push and a `gh pr create` take.
- Every change is read before it merges, and GitHub's history says who merged
  what.
- One delivery path for every kind of work; the per-task flow of PR #53 is a
  kind of it.
- The local default branch follows the upstream by fetch and fast-forward,
  which is what the brain's self-update needs.

### Negative

- The daemon depends on the forge and on a `gh` login that can see each
  repository; without them branches wait as `land.pr-held`.
- An agent merges code when no reason fires. `review.merge=ask` turns that
  off; the guarded paths and the human-pull-request reason cannot be turned
  off.
- The end-to-end acceptance job and the *is it what you wanted?* question of
  the local flow are not asked on a merge; the pull request is where the
  owner looks.
- `landing.parallel` is removed. `landing.push` is read by nothing;
  `landing.batch`, `landing.verify_memo*`, `landing.base_red_retry_minutes`
  and `landing.setup_repair` matter only to the local landing that remains
  for `pulse land --local` and for projects without an upstream; `landing`
  (`agents|human`) still gates that landing and the experiment lane;
  `landing.auto_stash` now decides whether the fast-forward stashes around a
  dirty checkout or skips it.
- The evolve loop's landing counts, the code-delivery report, the revert
  scan and the landing outcomes still read `land.landed`; under pull
  requests they see only the no-upstream projects until they read
  `land.merged` as well.

## Follow-Up

- [ ] Confirm this number against `pulse record list --kind adr` before import
      (`pulse migrate --records --repo . --ref HEAD`, or `pulse record new adr
      --body-file docs/decisions/ADR-069-delivery-by-pull-request-with-an-agent-reviewer.md`)
- [ ] Retire the landing parts of ADR-024 and the `landing.push` part of ADR-037
      (`pulse record deliver ADR-024 retired`, `pulse record deliver ADR-037
      retired`) once this is accepted
- [ ] Amend `ARCH-overview` and the `WF-pulse` / `INSTR-*` records that say
      `pulse land` merges locally (`pulse record amend`)
- [ ] Update `delivery:` and `delivered:` when the work closes (`pulse record deliver`)

## Delivery

`docs/landing.md` (delivery), `docs/review.md` (the reviewer),
`docs/console.md` (the hub's states), `docs/self-improvement.md` (settings).
Built by: the `fleet/pr-delivery`, `fleet/pr-reviewer`, `fleet/pr-hub` and
`fleet/pr-docs` branches.
