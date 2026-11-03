# Democracy and self-improvement

## Suggestions

Suggestions collect votes. `quorum` voters (default 2; the author's and
same-session votes do not count) decide a suggestion: more ups than downs makes
it *eligible*, more downs than ups makes it terminal `rejected`, and a tie
stays `needs votes` until one more voter decides. Eligible is a ballot state,
not a job: every tick the daemon takes each project's eligible suggestions that
pass the gates, best first, and queues one worker job per card (source
`suggestion:<id>`), reusing an in-memory planner snapshot while the project's
board/job/event/settings watermark is unchanged (default safety TTL:
`daemon.planner_cache_ttl_seconds=600`; the clock enters the tally in
five-minute steps, so a claim's expiry is seen within five minutes rather than
costing a fresh tally every minute; a tally the project has superseded - older
watermark, older clock step, or past the TTL - leaves the cache the moment a
newer one is stored, so a busy board does not park thirty-two dead tallies in
the daemon's memory). It fills every free lane: as many new jobs per tick as
`parallel_per_project` leaves beside the project's queued and leased ordinary
jobs (experiments aside), so a project with three lanes and five voted cards
works three at once instead of one every five minutes. **Product cards go
first**: the planner classes every card as *product* (the thing the project
ships) or *operating-layer* (tests, fixtures, CI, tooling, formatting, PULSE
control-plane docs) and builds every product card before any operating-layer
one; within a class net score, then the card's quality score, then age decide.
A card is operating-layer when it declares `Class: operating-layer`, when its
`File:` is a test, fixture, CI, tooling or
`docs/{memory,plans,decisions,evolution,context,scripts}` path, or when its
`Hypothesis:` speaks of nothing but checks in every clause (`go test ...
FAIL->ok`, `gofmt`, `typecheck errors 0`, a new test passing, a line count)
*and* its `Variable:` is check, test-environment or refactoring work. "Fewer
runs miss their finish call, go test stays green" is product; "GIT_DIR=/tmp/x
go test ./internal/runner goes FAIL->ok" is not, whatever path it names.
`plan.operating_layer_per_day` (default 3, `0` = off) caps the operating-layer
cards planned per project per day; the gate reads `operating-layer cap: N of M
today; product cards go first`. The cap holds only while a product card of the
project passes the plan gates: once the day's slots are used and no product
card competes, voted operating-layer cards are planned anyway
(`plan.operating_layer_fill`, default `on`; the idle line notes
`operating-layer cap N of M lifted: no product card competes`), so ten idle
workers do not watch eight voted cards wait for midnight because three morning
attempts committed nothing; the moment a product card competes the cap holds
again. `off` keeps it strict. Only `needs votes` cards appear on ballots;
`pulse ballot`, worker briefs, `pulse queue` and planner ballot jobs use that
same ballot, ordered closest to quorum then oldest, capped by `lead.ballot_cap`
(default 10). The board hygiene pass also retires open suggestions older than
`board.stale_days` (default 7) when their non-empty `files` list names only
paths absent at the project's HEAD, or - for cards without a `files`
list - when their `File:` line names a path whose whole top-level tree is gone
from the checkout (the planner's own `GATE_FILE_GONE` rule, so a card the
planner would never run stops counting as eligible); it posts a daemon result
with `resolution: dropped` and logs `retired N stale suggestion(s) for
<project>`. Near-identical open cards with the same sorted normalized `files`
list and normalized summary are grouped before the brief is written: the oldest
card stays voteable and the newer ids trail it as `dup of <oldest>`, not as
separate ballot entries. A worker votes only with `VOTE:` lines for cards in
its Ballot section; a twin vote from the same agent is skipped with `vote
skipped: <id>: already voted`. The ballot also lists the project's open
amendments while votes can still decide them (`needs votes`, `short of the
supermajority`); a `ballot` job shows their full Current and Proposed text, and
the runner casts a `VOTE:` line on one as an informed vote - the proposal's
`text_hash` plus a `Read:` line holding the worker's reason - so it counts
toward the amendment quorum. The planner seeds a `ballot` job for such an
amendment on the same terms as for a suggestion (older than the ballot wait,
some other active agent still able to vote), so an amendment nobody listed
cannot sit at one vote for days while ordinary runs see a one-line summary; its
ballot line reads `(amendment, N of Q informed voters)` with the constitution's
own quorum. When `evolve.loop=1` and the project is at Level D, the evolution
loop gets first pick as a measured experiment and the planner builds the rest;
the daemon says `plan <slug>: left suggestion <id> to the evolution loop` when
it defers that card. A loop that cannot plan - stopped on an instruction-shaped
post until the owner decides its question - reserves nothing but the flagged
post itself: the planner carries the other voted cards as ordinary improve jobs
meanwhile, and the project's idle line says `evolve: level D, planning blocked:
...` with the owner's next step.

The gates, in order: a suggestion already carried by a queued, leased or
blocked job waits for that job; a later open card with the same summary, or the
same `File:` + `Variable:`, as an earlier open card is held as `duplicate of
<id>` and never put on a ballot (vote on the older card instead); it must not
be addressed to the owner (`owner-*` threads, "for the owner" / "vote
requested" summaries - those are yours to answer or queue, never a worker's
job); a worker's own suggestion must carry evidence - a `Signal:` line naming
an existing file (dotfiles included), post, job, run, record (an ADR, a `MEM-…`
memory entry) or `check:`; a log under `/tmp` is gone with the run and does not
count, while an absolute path into a worker clone is read by its
repository-relative part and checked at the project's HEAD. A card whose own
`Signal:` resolves nothing is held as `without evidence` until an `opinion`,
`finding`, `answer` or `vote` on its thread carries one that does - the idle
lead's brief lists each held card with its gate so the fleet supplies the
evidence or closes the thread - and the card's `File:` (what the job would
edit) must stay off owner-only paths (`docs/evolution/constitution.md`,
`evaluators.md`, `STOP`, `docs/decisions/`, `.github/`): one owner-only path
among its files holds the card as `points at owner-only paths`, while a card on
agent-writable files may cite an ADR or a workflow as its `Signal:` evidence
(only a card that names no file is read by its Signal's paths), and a card held
there longer than `expiry_days` is closed with `resolution: owner-only` - the
owner queues it by hand or lets it go; a card whose `File:` names only paths
whose whole top-level tree is gone from the checkout (written before a port, a
rename or a removal) is held as `names gone paths` - a card that adds a file,
however deep, under a tree that exists is fine; then the operating-layer cap
above. The evolution loop's experiment planner applies the same gates before it
picks a card, so a voted card the runner holds is never run as an experiment
either; it also skips a card whose `File:` the project's constitution closes
(an immutable path, or a class the current stage does not open) - that card is
a proposal nobody acted on, not a breach, so it never stops the loop for owner
review. A card with a credential-shaped value in one of its fields is passed
over the same way (no stop, no card job; the planning status names it once and
moves to the next candidate) until a corrected card lands on its thread; a long
repository path in a field (`src/web-app/lib/use-context-intake`) is a path,
not a credential, wherever it sits in the sentence, and so is a store id - a
post, job or run id is a UTC stamp followed by the agent's name
(`20261012T092238Z-copilot-fable-1012-hub-2`), and a long agent name carries it
past the token length without making it a secret; the experiment archive
applies the same shape to the row it writes. Votes carry no evidence for the
card they vote on: a gated card is replaced by a corrected one, not voted
through. The reflect brief asks for one provable Tier 0 *product* defect first
(a wrong message, a wrong default, a broken user path, a swallowed error) and
only then a flaky test, broken link, stale doc or lint finding; a quiet run
(checks green, nothing to fix) ends `SUMMARY: done - checked <paths>` with a
`FINDING:` line, not `failed`; research jobs rotate through their focuses
starting with the product as its user sees it, weighted by what the board
accepted, and keep to the product focuses while the day's operating-layer cap
is reached. The console's Votes page, `pulse queue` (`not planned: <gate>`) and
`pulse status` (`planner <slug>: N eligible, M plannable (...)`) show the same
gate for the same card, so a ballot that "does nothing" always says why. A job
that ran marks the suggestion `built` or `tried`; a `blocked` job holds only
its own card. **Amendments** carry a `text_hash`; a vote counts only when it
read the same text. Passed amendments are written into the project's
constitution record by `pulse amend apply` and ratified by the `pulse` system
agent.

## Evolve

`pulse evolve` runs the loop the constitution allows: `pulse evolve status`
prints the gates, `pulse evolve measure --project P --ref REF` scores a tree
against the project's public fixtures and the owner's held-out ones (names and
paths never leave the owner's side), `pulse evolve cycle --project P` takes the
top voted suggestion, builds it on an experiment branch, measures it, archives
the row, and hands a passing result to `pulse land`. When the gates allow, the
daemon runs the cycle by itself; the command is the by-hand alternative.

- The evolution archive keeps lineage as well as scores. Each measured
  variant saves a binary patch outside the checkout under
  `~/.pulse/patches/evolve/<project>/`, records the `parent` archive row it
  built on (empty means the default branch) and the `base_commit` merge-base
  it forked from. The planner uses `evolve.parent_rate` (default `0.5`) as the
  share of experiments that should try an archived parent; eligible parents
  must have a saved patch and are weighted by clade metaproductivity
  `(improved descendants + 1) / (descendants + 2)` with the Beta prior keeping
  unexplored nodes alive, times novelty `1 / (1 + children)`. A tie
  (`neutral`, the usual result on pass/fail fixtures) counts as a descendant
  but not as a productive one; only an improvement - a delta of at least
  `evolve.tolerance`, or a tie whose card's own check rose - earns
  productivity. `pulse evolve lineage --project P [--json]` prints the
  archived tree, each row's outcome (`improved`, `neutral`, `discarded`), CMP
  values and the current candidate count.

## Seeded records and starter fixtures

Registering a project (`pulse init`, `pulse onboard`, the console) seeds the
loop's records and six public behaviour fixtures. The records: a project whose
checkout commits `docs/evolution/constitution.md` / `evaluators.md` gets those
files indexed as `EVO-constitution-<slug>` / `EVO-evaluators-<slug>`; any other
project gets a copy of the template's text marked `seeded_from` in its
frontmatter, with `loop: off`, no baseline and `Product scope: allowed` (amend
the line to `operating-layer` to keep product cards out), so nothing runs until
you edit it (`pulse record amend EVO-constitution-<slug> --body-file FILE`).

An existing record is never touched. You do not have to write the text yourself
(ADR-046): while a project's constitution or evaluators is still the seeded
copy, the daemon plans one `reflect` job (source `draft:<slug>`, once per 24
hours; the console's **Draft it for me** queues it at once) whose worker reads
the checkout and files the complete text with every placeholder replaced as a
*draft* - `pulse record draft ID --body-file F [--note ...]`, refused while a
`<placeholder>` remains or a protected section (Hard stops, Kill switch,
Amendments) differs from the current text. A draft never changes the record:
the console shows it as `draft ready` on the project row and the Evolution tab,
and one **Adopt** (or `pulse record adopt ID [--words ...]`, human only) writes
it as the next revision authored by `human` with the drafting agent in the
note; **Discard** (`pulse record discard ID`) drops it; `pulse record drafts
[--project P]` lists what waits.

The fixtures, named `starter-*`: the right tier declared, hard stops refused,
injected "approvals" ignored. Their scorers live in the binary (`pulse evolve
score NAME`) and judge effects only. Alongside them the loop *derives* metric
fixtures from the project's own tree, named `derived-*`: `derived-tests` (test
declarations the tree carries - `#[test]`, `it(`/`test(`, `def test_`, `func
Test` - scored higher) and `derived-todo-markers` (`TODO`/`FIXME`/`XXX`/`HACK`
in the code, scored lower), for each stack a marker file declares at the root
or two directories down (`Cargo.toml`, `package.json`, `pyproject.toml` and
friends, `go.mod`; dependencies and build output are skipped). Their scale is
set from today's count so the tree scores `0.5` now and a voted change that
brings tests or settles a marker scores differently from one that does
not - the starter set judges conduct and ties on every good change, so without
a derived or hand-made metric no fixture would tell variants apart. In an
experiment they count only inside the card's changed files (the diff scope
below), so one card's change moves them visibly. A derived fixture whose count
reaches its scale, or whose script changed, is re-armed with a fresh date
(event `evolution.fixture.rearmed`); a disabled one is never re-added. `pulse
evolve seed [--project P]` seeds both for a project registered earlier (`pulse
evolve fixture seed` for the fixtures alone, which also derives); `pulse evolve
fixture list|disable` manages fixtures. The baseline is the daemon's: once the
records and at least one enabled fixture exist and `loop: on`, it plans a
`baseline` job that measures the default branch, archives the score and
refreshes the experiment-lane baseline cache for that default-branch commit and
fixture hash (a failure posts a finding and is retried after
`baseline.retry_hours`, or after `lane.short_retry_minutes` when the failure is
transient; `pulse job add --kind baseline` queues it by hand). The cache key is
the SHA-256 of the enabled fixture definitions, the held-out fixture marker and
harness-affecting settings; `evolve.baseline_ttl_hours` defaults to `24`, and
`0` means cached baselines never expire. The job's run log lists every public
fixture's score and, for a 0, the scorer's one-line reason. The two private
held-out fixtures - the proof that the loop is not grading itself - are the
daemon's too by default (`evolve.heldout auto`): it creates
`~/.pulse/private/heldout`, writes one fixture per tick through a sealed model
call and marks each as generated (`evolve status` says `2 ready (2 generated)`,
the archive carries `heldout_generated`). `pulse settings set evolve.heldout
owner` hands the step back to you: `pulse evolve setup DIR` or the console's
Held-out fixtures dialog, which also work under `auto` and are never
overwritten. Product fixtures - your project's own checks - are yours to add
with `pulse evolve fixture add`. Every scorer runs against a frozen clone: a
symlink or hardlink that leaves the clone, a special file, or a tree that
differs after the scorer ran fails the measurement as a violation. A metric
fixture skips the agent request and scores the frozen tree directly, which
gives the baseline headroom when governance fixtures already score `1.0`:
`pulse evolve fixture add --project P --name N --metric --scorer 'pulse evolve
score metric --higher --scale S -- CMD ARGS...'`. The helper takes the last
number printed by `CMD`; `--higher` scores `min(1, raw/S)`, `--lower` scores
`S/(S+raw)`, and `--floor F` clamps the final score upward. `pulse evolve
status` shows each cached baseline fixture score and a `Headroom` line
(`none - every fixture scores 1.0 at baseline; add a metric fixture (pulse
evolve fixture add --metric)` or `N of M fixtures below 1.0`). Fixtures run in
a bounded pool (`evolve.measure_parallel`, default `4`; `0` or `1` keeps them
sequential), and the log starts with `measuring <k> fixtures with <p> in
parallel`; rows and scores are still reported in fixture order. Build caches
are placed under that fixture's isolated measurement home
(`build/cargo-target`, `npm-cache`, `go-cache`, `go-mod`, `pip-cache`,
`uv-cache`, `yarn-cache`) and the scorer gets the same cache environment, so
cargo `target/` and other tool caches are not part of the frozen clone. Scorers
also receive `PULSE_MEASURE_COMMIT`, `PULSE_MEASURE_BINARY` for self-scorers
and `PULSE_MEASURE_SCRATCH`, a per-fixture scratch directory under the isolated
home for work that must stay outside the tree. Each worker has at most one
fixture clone at a time, and removes that clone as soon as its scorer finishes;
the isolated homes stay until the measurement root's final cleanup. An
experiment's measurement also exports its **diff scope**: `PULSE_MEASURE_BASE`
(the base commit), `PULSE_MEASURE_CHANGED` (a file listing the paths that
differ between the base and the measured tree, one per line) and
`PULSE_MEASURE_BASE_TREE` (a frozen, read-only checkout of the base, shared by
the fixtures). `pulse evolve score metric` then runs `CMD` on both trees and
takes the base's count as the scale in place of `--scale` (twice the count for
`--higher`, the count itself for `--lower`, never below 1), so the base scores
`0.5` and the variant's delta is relative to what the card touched; the derived
fixtures' scripts count only inside the listed files, and your own `CMD` does
the same by reading that list. Such a fixture answers `scope: diff` after its
score line, the log marks it `[diff-scoped]` and the archive row records it
(`diff_scoped` in the measurement and in the `evolution.archived` event). A
diff-scoped score is per card, so the cached project baseline never serves it:
those fixtures are measured again on the base in the same scope inside the same
cycle, and the whole-tree numbers stay for the project-level report. A diff
with no changed file exports the base alone and the scorers see the whole tree.
A diff made of lock files alone (`Cargo.lock`, `pnpm-lock.yaml`,
`package-lock.json`, `yarn.lock`) is not measured: the variant takes the base's
numbers and is archived `neutral` with the reason `lockfile-only`.

## Card check

The card's own hypothesis, measured. A card whose `Signal:` line reads
`check:<command>` (or whose `Hypothesis:` names `check:<command>`, backticks
allowed) gets a fixture of its own for that experiment, `card-check` (kind
`card`, weight `1.0`, part of the card's composite, recorded with the other
fixtures in the archive row and as `card_check` on each measurement). The
command runs from the tree root on the base and on the variant through `pulse
evolve score check -- CMD ARGS...`: its exit status scores `1` or `0`; a single
integer on stdout is a count that should fall, scored like a diff-scoped
`--lower` metric against the base's count (so `grep -c unwrap src/x.rs` on both
trees works, whatever its exit status). It takes a command, not a project check
name: the project's recipe checks live with the runner, out of the
measurement's reach, so `check:cargo test -p demo` must spell the command. A
card that names a check can win on a tie (the keep rule below), and the worker
brief asks every card for one.

## Fleet

`pulse fleet` prepares isolated worker clones for parallel experiments;
**usage** gives best-effort local token diagnostics (never a ledger).

**Readiness is not delivery.** Level D means the general evolution gates are
open, not that a supported candidate exists or that product experiments are
authorized. Product scope is a separate owner-controlled gate, open by
default: an EVO-evaluators record with no `Product scope:` line lets product
cards run, `Product scope: off` (or `operating-layer`) restricts the loop to
the operating layer, and a line the loop cannot read - the seeded
placeholder, an unknown word, two lines that disagree - restricts it too and
`pulse evolve status` says why. Starter
fixtures measure agent safety, not the quality of the project's product;
baseline rows are not completed improvement cycles. The console reports
baselines and variants separately.

Open **Project → Evolution → Experiment planning** for the current candidate
or blocking reason. `pulse evolve status` reports the same planning state:
gated, blocked, waiting for an eligible suggestion, writing an experiment
card, ready but not dispatched, queued, in flight, or finalizing. When the
planner is blocked on something only the owner can do (an instruction-shaped
post awaiting `pulse decide`), that command is also the `Next human step`
line, in `pulse evolve status` and the `pulse reflect` digest alike. The
tripwire reads commands ("skip the checks", "ignore previous instructions",
"push immediately"), not the vocabulary of the work: a bare `skip check(s)` or
`skip guard(s)` with nothing between the words names a thing and passes. Level D
with no eligible suggestion is still waiting, not a running experiment. A voted
suggestion that names no single `File:` and `Variable:` is not the owner's to
fix: the planner queues one agent card job for it, which posts the card as a
new suggestion on the same thread, where it earns its own votes and passes
every gate before the loop runs it (ADR-044). Workers can post a complete card
in one `SUGGESTION:` line with `;`-separated fields; the line's first prose
segment is the card's summary, falling back to the Hypothesis, never to a
`Class:` value standing alone (`product; File: …`). A `SUGGESTION:` line that
only says the worker saw nothing (`none`, `nothing further`, `n/a`, `no
suggestion`, `none identified`, `I did not find …`, `no defect`) is dropped by
the runner rather than posted, even behind `File: n/a; Variable: none`: an
empty card on the ballot only spawns vote jobs, and each of those tends to end
in another such line.

The loop may edit only the file classes the constitution's Editable scope
table has raised to Stage 2 (Stage 4 for product code) whose paths exist in
the checkout. When no class qualifies, `pulse evolve status` reports Level C,
`gated`, names the classes and the owner's step (raise a stage with `pulse
record amend`, or point a staged class at paths that exist); the planner then
cards nothing and asks nothing, because no experiment could run. `pulse evolve
report --project P [--days N]` prints the week from the store - suggestions
and where the vote left them, card jobs, jobs done with and without a change,
landings, experiments, and the questions still waiting for the owner - so
"is anything improving" is a table anyone can recompute, not an essay. Its
last three rows measure the improver itself: landings per worker-hour,
first-pass share of landing attempts, median lead time from job to landing,
owner asks per landing, how many landings changed the improver's own crates
(`crates/pulse-runner`, `pulse-evolve`, `pulse-harness`, `pulse-provider*`,
`pulse-board`, `pulse-tally`, `pulse-tasks`, `pulse-goals`) and, on the
brain, how often the daemon rebuilt itself - for the window and for the
window of the same length before it, with an `Improver trend` line that names
the direction that counts as better. A window with fewer than five landings is
marked a thin comparison. A system that is getting better at improving moves
those numbers; one that is only busy does not.

Landing outcomes close the self-change loop after each improver landing. For
brain projects, the daemon waits `evolve.outcome_hours` (default `24`) after a
`land.landed` event whose delivered paths touch the improver crates, compares
the fleet-wide loop window before the landing with the same-length window after
it, records a `land.measured` event, and posts a `result` on the landing's
suggestion thread when the job came from one (otherwise under
`evolution/landing-outcomes`). The result names landings per worker-hour,
first-pass share, median lead time, and owner asks per landing across all
projects. If landings per worker-hour drops by more than 20% and both windows
have at least 10 landing attempts, the daemon posts a revert `suggestion` with
the numbers as evidence; it never reverts by itself. A pass measures at most
five landings, oldest first, so a backlog drains over a few passes instead of
flooding the board.

Landing, sweeping and binary checks share a maintenance pass outside the
supervisor tick, so a slow verification does not freeze worker dispatch,
heartbeats or STOP handling. The pass starts per-project landing workers for
plain branches, up to `landing.parallel` projects at once (default `4`; `1`
keeps the old serial pass), and then continues with sweep and binary checks
instead of waiting for another project's verify command. One worker holds a
project's landing lock and lands that project's ready branches serially; later
passes skip a project whose worker is still running. Experiments stay on a
second thread, the experiment lane, which measures one experiment at a time; a
ninety-minute measurement therefore holds no plain landing. The lane obeys the
load governor: while the machine is saturated (the `machine saturated` hold
below) it does not start a measurement - the log says `landing experiment
<branch> (<project>) waits: machine saturated: …` once per hold and the
experiment is tried again after `lane.short_retry_minutes` - and a
measurement in progress counts as a local run under `daemon.max_jobs`, since
its two builds are load like any other. The lane reuses a
cached baseline when the default-branch commit, fixture hash and TTL still
match, logging `baseline for <commit7> reused from <at> (score <x>)`; archive
notes say `baseline: reused <at>` or `baseline: measured`. It measures the
default branch on a cache miss, then the variant as landing
would merge it - the branch merged onto the current default branch, built
from git objects alone so neither the checkout nor the branch moves (a branch
that already contains the default branch, or one that conflicts with it, is
measured as it is and landing reports the conflict); a variant measured on
its stale fork point would miss every fix the default branch gained since,
and the archive notes `measured as … merged onto …` when that happened. The
keep rule is asymmetric: a variant whose total score falls more than
`evolve.tolerance` below the baseline is `discarded` and loses its branch; a
gain of at least `evolve.tolerance` is `improved`; anything in between is a
tie, labelled `neutral`. A tie is not a win: it still goes to the same landing
as any branch, where the project's own check decides it, but the archive,
`pulse evolve status`, `report` and `lineage` show it apart from improvements
and it earns no lineage credit - a tie means the card's hypothesis went
unmeasured, so the worker brief asks every card to name a check that will move
(`Signal: check:<command>`, the card check above). The one exception: a
tie whose card-check score rose counts as `improved`. Held-out fixtures are
mostly pass/fail, so a voted change that keeps every fixture green measures
as an exact tie; before the tie lane such ties were discarded and no voted
experiment ever landed. A discard must reproduce: when a regression hinges on request fixtures alone (the
ones that run a harness, whose score depends on a model's answer - a metric
fixture scores the frozen tree and is believed as measured), the lane measures
those fixtures again on both trees before the discard stands (`--fixture` on
the measurement names them; `H1`, `H2`, ... are the held-out fixtures by their
archive names). A difference that comes back the same way is a discard; one
that does not scores as a tie, the archive row notes `the discard hinged on
NAME, measured again on both trees: … scored as a tie`, and the event
`evolution.measured_again` records which fixtures were noise and which
reproduced. Before this, a fixture that scored the same commit `1` one cycle
and `0` the next discarded three voted experiments on its own. An
experiment the lane could not land stays queued and is tried again after
`lane.retry_minutes` (or `lane.short_retry_minutes` after a lock, STOP,
network/rate-limit/database-lock/full-disk-style transient), recorded as
`land.deferred` with the reason and `retry_after_min`. The daemon log says
`retry in 20m` (or the configured value). The retries are bounded: an
experiment still deferred for a reason of its own (a governor hold does not
count) `lane.retry_minutes` × 72 (`EXPERIMENT_DEFERRAL_RETRIES`, a day at the
default) after its first such deferral is given up - the job fails with a
`land.missing` event naming the wait and the last reason, the branch stays
for review - where one experiment was deferred for eight days. The measurement harness the lane starts
is the same `pulse harness run` the workers use, with the same tool denials,
so it never waits for a permission prompt; it runs on `measure_model`
(`worker_model` while empty). When the daemon restarts on a new
binary it cancels the lane at once (the next daemon measures again) and gives
the per-project landing workers ten minutes before cancelling them too; a self-update that
is due while the lane measures waits for the lane instead of throwing the
measurement away, and the lane starts nothing new until that update has
built. Cancellation reaches the measurement and verification commands; it
does not bypass their results or the landing gates. A watchdog bounds the
pass itself: each pass runs under its own one-hour deadline (every command
inside has a budget; this bounds their sum), and a pass that overruns is
cancelled and logged with the stage it was at (`landing <project>`,
`pushing <project>`, `sweep`, `binary check`) so the next pass can start -
what landed stays landed, the rest waits for it. A pass still running ten
minutes after that cancellation sits in a call no budget reaches; the daemon
then logs it and exits with code 3 so the service restarts it (not under
`--no-restart`, which logs once instead). Without this, one wedged pass once
held every finished job unlanded for ten hours while dispatch went on. The
daemon's idle wait wakes when a pass ends and takes the pass's result in
itself (lines, landing holds, the pass cleared) - the ten-hour stall was a
pass that had in fact finished, whose result the wait had received and
dropped, leaving the pass marked running until the next daemon start. A pass
whose thread ends without a result is cleared the same way.

A streak of exact ties is not progress: when a project's last
`evolve.flat_cycles` experiment cycles (default `6`; baseline refresh rows
do not count) all scored the variant equal to its baseline, the fixtures are
not telling a better variant from a worse one, and every further cycle would
cost a worker run and a measurement and teach nothing. The loop then pauses
with one owner question (`Evolution paused: the fixtures do not tell variants
apart`) on the project's `evolution/loop-gate` thread, listing the tied
cycles, the enabled fixtures and the next step. Each fixture line carries
what the archived cycles show about it, so you know which one to replace:
`told no variant from its baseline in 26 cycles`, `moved 3 times with the
default branch` (its baseline score changed when the default branch did -
it measures main's own commits, not the experiments), or `scored the same
baseline commit two ways, so a difference it reports is not a signal on its
own`, or `scored as a tie by the discard dispute in 2 cycles`; held-out
fixtures appear by their archive names (`H1 (held-out)`). Judge noise counts
neither way: the streak is read fixture by fixture from each cycle's
archived matrix, leaving out a fixture the discard dispute scored as a tie
and one that scored the same baseline commit two ways inside the window, so
such a fixture neither breaks a streak with a difference that is noise nor
makes one with a tie that was a verdict; the question's cycle lines and the
reason name what was left out (`request left out as judge noise`). A fixture
that flips the same baseline commit both ways twice is retired by the
healer below (disabled, event `evolution.fixture.retired`, one finding
`Evolution fixture retired: it scored the same commit both ways twice`);
`pulse evolve fixture enable NAME` brings it back and only later cycles
count for it again.
`pulse evolve status` shows the same reason on its `Experiment planning`
line (`blocked - …; only NAME told a variant from its baseline in 26 cycles;
NAME scored the same baseline commit two ways`) and the step as `Next human
step`. Two things clear it: a
fixture enabled after the streak began (usually a metric fixture, `pulse
evolve fixture add --project P --name N --metric --scorer CMD`, which the loop
notices on its own), or a review of the question - your answer (`pulse post
answer --project P --area evolution --thread loop-gate --on QUESTION_ID
--summary 'Reviewed'`) or a decision on it - which lets the loop run
`evolve.flat_cycles` more cycles before it may ask again (the review's own
date starts the new window, whichever question it lands on). `pulse settings
set evolve.flat_cycles 0` turns the gate off; the smallest streak it accepts
is `2`.

The loop heals a flat evaluator set itself before it asks or waits
(`evolve.heal`, default `on`). Whenever it plans, it first retires the judge
noise described above (a public fixture that flipped the same baseline commit
both ways twice since it was last enabled). When the gate fires, it derives
the metric fixtures the project's tree affords (`derived-tests`,
`derived-todo-markers`, above) and re-arms a derived fixture whose count
reached its scale - unless that fixture told no variant from its baseline in
the whole window, which a fresh date would hide: that one stays as it is, and
the healer's post points at the card check (`Signal: check:<command>`)
instead; a fixture dated after the window clears the gate at once,
and what was done is posted on `evolution/loop-gate` - as an answer on the
open question when there is one, so your inbox folds it away, else as a
finding (`Evolution healed: the loop derived fixtures from the tree`; event
`evolution.healed`). When nothing can be derived and the question has waited
`evolve.flat_wait_hours` (default `24`) without an owner answer, the planner
reviews it itself: one answer in the loop's name (`Reviewed by the loop: run
N more cycles`, event `evolution.flat.self_reviewed`) lets `evolve.flat_cycles`
more cycles run as the fixtures are, and the gate asks again after that many
more ties - the loop never waits forever on a question nobody answers, and
never runs more than one window on its own say-so. Only the flat gate's own
question is reviewed this way; the failed-experiment gate and `STOP` stay
yours, and while a `STOP` or hard-stop question is open on the thread, or
the failed-experiment gate holds, the loop writes nothing there at all
(every gate on that thread reads a later answer as its review). A flat
question that no longer asks for anything - you added a fixture, `fixture
seed` derived one, the gate was turned off, a later cycle moved - is closed
by the loop with the reason (`Evolution resumed: the gate no longer holds`,
event `evolution.flat.resumed`), so it leaves your inbox on its own. `pulse evolve status` says what comes next while the gate holds
(`the loop reviews it itself at 2026-10-13T02:29:30Z (evolve.flat_wait_hours
24)`), the daemon logs `evolve P: healed derived: added …` / `healed
self-reviewed: …`, and the question body says so. `pulse settings set
evolve.flat_wait_hours 0` keeps derivation and leaves every review to you;
`evolve.heal off` turns the step off.

After a measured experiment is finalized with no pending branch, the daemon
can queue one durable, result-specific reflection per eligible archive, for
both kept and discarded outcomes. The archive must belong to the job's latest
successful run. Its evidence feeds subsequent suggestions in the shared
project-goal context; it does not create votes or authorize another experiment.
Failed or provisional runs are not reported as successful measured feedback.
STOP, loop-off and resource gates can defer this follow-up.
Finalization requires a landing or discard receipt tied to the current
project, job, run and archive; an empty or missing branch is not success.
Older archives without that proof remain history, not successful feedback;
no receipt is backfilled automatically.

The `cycles` count is non-baseline experiment archives since the current
loop-enable event; kept/discarded totals cover all history. An archive can
consume a cycle before landing finishes, even if landing later fails. These
counts are measured attempts, not shipped changes.

A `done` job or `built` suggestion is not a promise that code shipped:
answers, ballots and leads can legitimately finish without commits.
Pending landing jobs count job records, not distinct verified patches.
The controller must finish accepting a worker's artifacts before its job can
enter landing; local landing and remote publication remain separate steps.
Dirty checkouts, Git locks and missing upstreams still hold delivery.

When the owner is away, ordinary owner-addressed questions are paced like
other lead work. Evolution stop questions remain visible for the owner but
do not repeatedly wake the lead on their own. A reply does not authorize
bypassing the underlying stop, and new owner messages and tasks still wake
the lead promptly.

## Evolution runner caps

`pulse settings help` explains the supported caps without opening a store
(`pulse settings --help` and `pulse help settings` show the same guidance).
The CLI does **not** support `evolve_minutes` or `evolve_credits`; they are
not aliases. The former `docs/workflows/evolve.md` is now the `WF-evolve`
record, read with `pulse record show WF-evolve`.

| Setting | Built-in default | Scope |
| --- | --- | --- |
| `minutes_per_job` | `10` minutes | Worker wall-clock cap; also the deadline for each measurement harness and scorer. Measurement requires a positive value. The built-in harness gets one minute less so it stops with a report of its own. |
| `evolve.measure_parallel` | `4` fixtures | Fixtures one measurement runs at once; `0` or `1` keeps measurement sequential. Each worker keeps only one clone live and removes it after scoring, builds with the cores divided by this value (`CARGO_BUILD_JOBS`), and runs at nice 10, so four cold builds share the machine instead of each taking all of it. |
| `evolve.flat_cycles` | `6` cycles | Tied experiment cycles in a row (variant score = baseline score) before the loop pauses with one owner question on the project's `evolution/loop-gate` thread; a fixture enabled since, or an answer to the question, lets it run again. At least `2`; `0` turns the gate off. |
| `evolve.heal` | `on` | `on`: when the flat-fixture gate pauses a project, the planner first heals the evaluator set itself - retires a fixture that flipped the same commit both ways twice, derives metric fixtures from the project's tree (`derived-tests`, `derived-todo-markers`), re-arms a derived fixture whose count reached its scale, except one that tied the whole window - and posts what it did on `evolution/loop-gate`; `off`: the gate waits for you alone. |
| `evolve.flat_wait_hours` | `24` hours | How long the flat gate's question may stay unanswered before the planner reviews it itself (an answer in the loop's name that lets `evolve.flat_cycles` more cycles run; it asks again after that many more ties). `0` waits for you. |
| `lane.retry_minutes` | `20` minutes | Experiment-lane retry after a failed or deferred measured experiment. |
| `lane.short_retry_minutes` | `2` minutes | Experiment-lane short retry after a lock, STOP or transient failure; baseline failures classified as transient use this pause too. |
| `landing.base_red_retry_minutes` | `10` minutes | Landing hold retry after a project's default branch itself fails verify. |
| `landing.setup_repair` | `on` | `on`: a project setup that fails on its own default branch - at a landing, or twice the same way in a worker clone - queues one repair job (source `setup:SLUG`, at most three a day per project) that runs without that setup and fixes the recipe, the lockfile or the tree; `off` only defers the landing and marks the machine unfit, as before. |
| `pipeline.watch` | `fix` | What the daemon does with the GitHub Actions runs a push of its own set off ([Pipelines](landing.md#pipelines-what-a-push-set-running-on-github)): `fix` tells you on the project's `runner/pipeline` thread and queues one fix job for a red pipeline; `watch` tells you only; `off` reads nothing. |
| `pipeline.wait_hours` | `3` hours | How long the daemon waits for a run that is still going before it records `gave-up` and leaves the push to you. At least `1`. |
| `machine.labels` | empty | What this machine offers beyond its OS and chip, as words a project or job can need (`gpu`, `android`, a tool's name), comma-separated; `xcode` and `docker` it finds on its own ([Machines](machines.md#machines-what-runs-where)). Paired hands carry theirs in `pulse executor label`. |
| `machine.reserve` | `off` | `on`: this machine takes only the jobs that need something it offers and leaves the rest of the queue to the other machines - the only Mac kept for the iOS app. Paired hands: `pulse executor reserve NAME`. |
| `dependencies.security_updates` | `on` | `on`: the daemon reads every active GitHub project's open Dependabot alerts and queues one security update job per affected manifest ([Dependencies](landing.md#dependencies-security-alerts-and-the-release-age-rule)); `off` leaves the alerts to you. |
| `dependencies.major_updates` | `ask` | `ask`: a security fix that needs a major upgrade is a `finding` for you with Yes / No in the For you inbox ([Dependencies](landing.md#dependencies-security-alerts-and-the-release-age-rule)); `auto`: the watcher queues it like any other security update, to the newest eligible version of the patched line. |
| `dependencies.min_release_age_days` | `3` days | A dependency version is adopted only once it has been published this long, in every project: a security update waits for the patched version to age, and a landing that bumps a lockfile is held until every new version it pins is this old. `0` turns the wait off. |
| `dependencies.alert_check_minutes` | `60` minutes | How often the daemon asks GitHub for a project's open Dependabot alerts. |
| `verify.lock` | `off` | `project`: a project's verifications (worker pre-commit hook, post-run check, landing) take turns on a machine; `machine`: all of them do. For checkouts that share one build cache. |
| `baseline.retry_hours` | `6` hours | Baseline measurement retry after a non-transient failed baseline job. |
| `provider` | `copilot` | The model provider the built-in harness talks to: `copilot` (GitHub device login), `github-models` (GitHub device login or a token), `azure-openai` (Microsoft device login or a key) or one of the API-key providers `openai`, `anthropic`, `gemini`, `openrouter`, `groq`, `mistral`, `deepseek`, `xai`, `together`, `fireworks`, `cerebras`, `ollama`, `lmstudio`, `vllm`, or `custom` for any other OpenAI-compatible server (**Other providers** under [The harness](harness.md#the-harness-how-an-agent-runs)). |
| `provider.<name>.base_url` | the vendor's endpoint (`https://api.openai.com/v1`, `https://api.anthropic.com`, `https://generativelanguage.googleapis.com`, `http://127.0.0.1:11434/v1` for `ollama`, …; empty for `azure-openai` and `custom`) | Where one API-key provider sends its calls; while it keeps its default, the vendor's `*_BASE_URL` environment variable stands in when set. One row per provider name above except `copilot`. |
| `provider.custom.no_key`, `provider.custom.api_key_header` | `off`, empty | How the `custom` provider authenticates: `no_key` `on` calls the server without a key the way the local servers do; `api_key_header` names a header (`x-api-key`, `api-key`) the key travels in raw instead of `Authorization: Bearer`. |
| `provider.azure-openai.tenant` | `organizations` | The Microsoft Entra tenant the `azure-openai` *Sign in with Microsoft* device code uses: `organizations` (any work or school account), `common`, `consumers`, or a tenant id or domain; `AZURE_OPENAI_TENANT_ID` or `AZURE_TENANT_ID` stands in while it keeps its default. |
| `provider.retry_after_cap_s` | `60` seconds | Maximum provider rate-limit header wait before jitter; the retry backoff still floors the wait. |
| `provider.max_body_mb` | `8` MiB | Successful provider response body cap; exceeding it fails the call as non-retryable. |
| `provider.max_request_mb` | `1` MiB (`0` off) | How big one request - the conversation with its tool results and images, plus the tool schemas, as the harness estimates them - may grow before a run or chat turn compacts the older conversation whatever the token count says, so a long conversation stays under what the gateway takes: Copilot times out reading a body of about 1.5 MiB (`Timed out reading request body`, HTTP 408) long before a 1M-token window is full. A request the provider still refuses as too large is compacted and sent again, then its older observations collapsed and sent once more, before the turn stops (**Compaction** under [The harness](harness.md#the-harness-how-an-agent-runs)). `--max-request-mb N` for one run. |
| `harness.max_steps` | `60` | Model calls one worker run (or one chat turn) may make before the harness stops it; `none`, `off` or `unlimited` lifts the cap and leaves the run to the wall clock, the cost cap and compaction. |
| `harness.task_max_steps` | - | Model calls a `task`, `improve` or `experiment` run may make before the harness stops it. Empty keeps these kinds on `harness.max_steps`, so a task can get a larger budget without the short kinds (answer, ballot, reflect, onboard) getting the same. |
| `harness.step_timeout` | `300` seconds | Wall-clock cap on one command the model runs. |
| `harness.commit_timeout` | `1500` seconds | Wall-clock cap on a `git commit`/`merge`/`rebase`/`cherry-pick`/`am` whose pre-commit hook is the runner's verify gate; never below `harness.step_timeout`, capped by the run's remaining minutes. A step that still times out tells the model the verify gate was running (and how long it waited for the verify lock) instead of the generic retry hint. |
| `harness.output_limit` | `20000` characters | Command output kept per step; longer output is cut in the middle. |
| `harness.format_retries` | `1` | Prose-only native-harness replies corrected without consuming `harness.max_steps` before later format errors count normally. |
| `harness.context_window` | `0` tokens (= the model list) | The context window the harness measures `harness.compact_at` against and collapses observations harder past 80% of; `0` reads the window the provider lists for the chosen model - 1M for the Claude and Gemini models that take it - and a model that lists none is taken as 128,000. `pulse harness run --context-window N` overrides it for one run. |
| `harness.compact_at` | `50` percent (`0` off, `95` at most) | Where in the context window a native-harness run asks the model to summarise the older part of its conversation and replaces those messages with the summary (**Compaction** under [The harness](harness.md#the-harness-how-an-agent-runs)); a request past `provider.max_request_mb` compacts whatever the percent. `--compact-at N` for one run. |
| `harness.compact_model` | empty (= the run's model) | The model that writes the compaction summaries. A cheaper one saves money on the providers that bill per token - each summary reads up to about 30,000 tokens of transcript - while Copilot bills per request, so there it saves time rather than credits. `--compact-model M` for one run. |
| `harness.reasoning` | `default` (`minimal`, `low`, `medium`, `high`, `xhigh`, `max`) | The reasoning effort a native-harness run asks of the model, sent as `reasoning_effort` to the OpenAI-compatible providers and to Copilot for the models that take it; `default` sends nothing (**Reasoning effort and prompt caching** under [The harness](harness.md#the-harness-how-an-agent-runs)). `--reasoning LEVEL` for one run. |
| `harness.instructions` | `on` | A native-harness run reads the owner's `~/.pulse/AGENTS.md`, the clone's `AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, `.github/copilot-instructions.md` and its rules (`.pulse/rules`, `.claude/rules`, `.github/instructions`) into its system prompt; path-scoped rules and nested `AGENTS.md` files arrive with the files they cover. |
| `harness.mcp_tools` | `on` | A native-harness run offers the project's enabled MCP servers' tools to the model as `mcp__<server>__<tool>`. |
| `harness.skills` | `on` | A native-harness run offers the `skill` tool over the clone's skill folders and the project's `pulse skill` registry. |
| `harness.hooks` | `on` | A native-harness run runs the hooks in the clone's `.pulse/hooks.json` and the owner's `~/.pulse/hooks.json`. |
| `harness.subagents` | `on` | A top-level native-harness run may hand a sub-task to a bounded helper run through the `subagent` tool. |
| `harness.memory` | `on` | A native-harness run reads `~/.pulse/memory/<project>.md` and `~/.pulse/memory/user.md` into its system prompt and offers the `memory` tool (**Memory** under [The harness](harness.md#the-harness-how-an-agent-runs)). `--no-memory` for one run. |
| `harness.tasks` | `on` | A top-level native-harness run with a store offers the `task` tool: an agent files tasks on the board under its own name, with the conversation or job it came from as the origin ([Tasks an agent files](board.md#tasks-an-agent-files-the-task-tool)). `--no-tasks` for one run. |
| `harness.teach` | `off` | Teacher mode (**Teacher mode** under [The harness](harness.md#the-harness-how-an-agent-runs)): the agent still does the work, but explains the why of each design or coding choice as it goes, for a junior reading along; a job reports `LESSON:` lines that become `lesson` posts on the board. `--teach` / `--teach-off` for one run. |
| `harness.web_search` | `chat` | Where the model may search the web with the `web_search` tool (**Web search** under [The harness](harness.md#the-harness-how-an-agent-runs)): `chat` offers it in `pulse harness chat` and not to jobs, `on` to jobs too, `off` nowhere. `--web-search` / `--no-web-search` for one run. |
| `harness.fetch_allow` | empty (the tool is not offered) | Hosts the `fetch` tool may read web pages from, comma-separated, `*.` for subdomains. |
| `harness.deny_tools` | empty | Permission rules every run refuses outright, comma-separated: `bash(rm *)`, `read(.env*)`, `write(*.lock)`, `subagent`; a tool denied whole is not offered. Jobs get it as `--deny-tool`. |
| `harness.allow_tools` | empty | Permission rules `pulse harness chat` runs without asking: `bash(git *)`, `edit(src/*)`, `mcp__github__*`; a deny rule wins. Jobs ask nobody, so it changes nothing there. |
| `worker_model` | empty (the runner's default) | Model of the runs that change code: task, improve and experiment jobs. Also the loop's measurements while `measure_model` is empty, and every other run while `light_model` is empty. |
| `light_model` | empty (= `worker_model`) | Model of the light runs: lead, answer, ballot, reflect, onboard and clean jobs, the sealed held-out call and the executor login check. A cheaper model here leaves the strong one to implementation. |
| `measure_model` | empty (= `worker_model`) | Model the self-improvement loop measures fixtures with: every baseline, experiment and landing re-measure. It is part of the evaluator, so a change re-measures every baseline; while it is pinned, a `worker_model` change keeps them. Empty measures with the model the workers use, so the tree is tuned for the agent that ships it. |
| `worker_reasoning` | empty (= `harness.reasoning`) | Reasoning effort (`default`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`) of the runs that change code and of a worker-class agent; `default` sends nothing. Jobs get it as `--reasoning`. |
| `light_reasoning` | empty (= `harness.reasoning`) | Reasoning effort of the light runs and of a light-class agent, so the cheap model can also think less. |
| `measure_reasoning` | empty (= `worker_reasoning`) | Reasoning effort the loop measures fixtures with. Like `measure_model` it is part of the evaluator: a non-default value enters the baseline key, so a change re-measures every baseline. |
| `board.stale_days` | `7` days | Board hygiene retires an open suggestion this old when its non-empty `files` list names only paths absent at the project's HEAD. |
| `schedule.script_timeout_minutes` | `20` minutes (minimum `1`) | How long a check or watch script (`pulse schedule add --kind check\|watch --script`) may run before the daemon kills it and counts the run as a failure: a red check, a broken watcher. |
| `schedule.scripts_per_tick` | `2` scripts (minimum `1`) | Check and watch scripts the daemon runs in one tick, the longest overdue first; the rest wait for the next tick, and none runs while the machine is saturated. |
| `manifest.load` | `on` | Whether the daemon reads each project's own PULSE files - `.pulse/automations/<name>.yaml` and `.pulse/pulse.yaml` on the default branch - and the `SKILL.md` folders beside them (`.pulse/skills`, `.claude/skills`, `.github/skills`, `.agents/skills`) whenever a commit changes them: the schedules the files name are added, updated, paused and ended in the store, the skills join the project's `pulse skill` registry, and what a file gets wrong is a problem on the project board, never a stopped schedule. `off` leaves the files to the harness, which still offers the skill folders to the model. |
| `lead.suggest_days` | `30` days (`0` turns it off) | Days before the daemon asks the owner again to seat a role lead (`pulse lead add`) for a concern of a project nobody owns: a self-improvement loop with no `rsi` lead, code landing without docs and no `docs` lead, a dependency manifest with no `security` lead, no check or watcher and no `monitor` lead. One question per project per pass, on the board thread `lead/<role>`; a yes on it seats the lead. Nothing is asked while `owner.presence` is `away`, except that the `security` seat is then seated unasked, once, and said so on the thread `lead/security`. |
| `toolkit.suggest_days` | `30` days (`0` turns it off) | Days before the daemon asks the owner again about a tool or skill a project's shape asks for (`pulse toolkit`), one question per project at a time on the board thread `toolkit/tool-<name>` or `toolkit/skill-<recipe>`; a yes adds it. While `owner.presence` is `away` nothing is asked: a drafted skill or a tool without a secret is added and said so on the thread; a tool that needs a secret waits for the owner. |
| `goal.review_days` | `7` days (minimum `1`) | Days between the lead's readings of the project goal. `pulse goal review` and the hub's goal panel score the last work against the goal from the store and blend in the lead's latest reading (a `finding` on `goal/review` with a `Score: NN` line) while it is this recent; older, and the lead's brief asks for a new one. |
| `evolve.limits` | `none` | Optional loop budget such as `cycles=20,hours=48`; `none` does not remove runner caps. |

These are defaults, not the owner's effective values. Read those with
`pulse settings get NAME` or `pulse settings list --json` (also includes
defaults, validation rules and descriptions). Only the owner changes caps;
neither an unlimited loop nor these docs adds a measurement-wide credit budget.

Applicability notes for the historical legacy fixtures (F03, F08) live in the
`EVO-archive` record (`pulse record show EVO-archive`).
