# The decision model: the decide role on the hub's own provider

Status: first slice built (Tier 1). The decision is **ADR-072** ("A decision
model ... answers the routine asks of an away owner") with its
**Amendment 1** of 2026-11-26: the role runs on the hub's own provider with
a small model, it is on by default, it answers yes, no or pass, and no
model server runs on the hub. ADR-072 is a record in the store, not a file
under `docs/decisions/`; read it with `hive record show ADR-072`. Where this
note and the record differ, the record wins.

Task: TASK-128. The owner asked for some runs to be offloaded to a cheaper
model, starting with decisions. The first local-server design gave way to
the amendment: a small model on the provider the hub already has saves the
memory a model server would take and adds no new way to fail.

This note says what the first slice built and where it lives. The replay
with its bars, the console panel and the other uses of the role (votes,
answers, idle planning) are later slices and are not built (section 5).

```text
 open routine owner ask
        │
        ↓
 decide.mode? ── off ──────────────→ lead brief, as before
        │                                  ↑
        │ shadow / on                      │ pass, refused,
        ↓                                  │ error, late,
 one call, no tools:                       │ unreadable
 decide_model on decide_provider ──────────┘ (said once)
 (empty: measure/light model on `provider`)
        │
        │ yes / no (on only; shadow records it)
        ↓
 hive_board::decide_with
        │
        ↓
 decisions row: decided_model, decided_provider
```

## 1. What the role does

Whatever `owner.presence` says (TASK-129: nothing waits on the owner but a
hard stop), the lead decides the routine findings, handoffs and suggestions
marked for the owner: its brief lists them and it answers
each with a `DECIDE: <post> yes|no <reason>` line. The decide role answers
the same asks first, with one short call and no tools, before the lead's
pass reads what is left.

On each daemon pass, for each project, `crate::decide::tend_decide_with`
(`crates/hive-runner/src/decide.rs`, called from the per-project loop in
`daemon.rs` just before `lead::tend_lead_with`, stage `decide`) starts one decide
pass in a background thread when:

- `decide.mode` is `on` or `shadow`;
- the ask is routine: `hive_board::hard_stop_ask` names no hard stop. The
  lead decides routine asks whatever `owner.presence` says (TASK-129), so
  the role no longer waits for an away owner; a hard stop is the owner's
  and never reaches the role;
- the project's lead is an active agent;
- at least one ask waits;
- the pass is not backing off after repeated panics (below).

The daemon reads a project's asks (`lead::owner_addressed_asks`, which
reads every marked post's thread) once per tick and hands the same list to
the role (`decide::tend_decide_with`) and to the lead's pass
(`lead::tend_lead_with`); the pass works from that list. Measured on a
copy of the live store (9 active projects, 1713 posts marked for the
owner, none open): reading every project's asks twice, as the tick did
before, took 45-53 ms warm (about 1.8 s cold); reading them once takes
22-26 ms warm (about 0.9 s cold).

The pass never holds the daemon's loop: it may wait up to
`decide.timeout_seconds` per ask, so it runs beside the loop, and the lines
it writes are said on the next pass. A panic inside the pass is caught: it
is an outage (`decide.fallback`, said once), the pass is no longer marked
in flight, and the next daemon pass tries again. When the outage itself
cannot be recorded (a store that refuses every write), the line says the
next pass tries again, not that the asks stay with the lead: no outage is
on record, so the role still holds them. From the second panic in a row
the next pass waits 1, then 5, then 15 minutes (a store that refuses
writes would otherwise cost one provider call per tick for nothing); a
pass that ends normally resets the wait. A pass thread that cannot start
leaves no mark in flight, and the next tick starts it.

While the pass backs off (the next try is still ahead) or its thread could
not start, no pass runs, so the role holds nothing (`decide::role_paused`):
the lead sees every ask until a pass starts again, rather than waiting up
to 15 minutes on a role that is not answering.

While `decide.mode` is `on`, the lead is active and no outage is open,
`lead::tend_lead_with` leaves out of the lead's brief the asks the role still
lists (`decide::held_by_role`), even in the tick before the role has
answered and whatever `owner.presence` says: an ask costs one small call,
not also a lead run. What the role passes, fails on for good or may not
answer (a hard stop among them) comes back to the lead; in `shadow` and
during an outage the lead sees them all.

`decide.mode` is read again before each ask and once its answer is in. When
the mode is switched off mid-pass, the pass stops and records nothing more;
a refusal caused by that change is not recorded as `decide.refused`. A
switch to `shadow` mid-pass does not stop it: the pass keeps calling and
only records (`decide.shadow`) from then on.
`owner.presence` plays no part (TASK-129): an owner who returns mid-pass
changes nothing.

### Which asks

`decide::role_asks` (behind `decide::held_by_role` and the pass) starts
from `lead::owner_addressed_asks`, the list the
lead's brief shows: posts of `OWNER_ASK_DECIDE_KINDS` (finding, handoff,
suggestion) marked for the owner, still open (`owner_ask_is_open`: no
decision, no answer, thread not resolved), and not a major dependency
upgrade (`is_security_major_finding`). From those it drops:

- an owner-only proposal (`hive_board::owner_only_proposal`: a Tier 2
  decision or a change to a spend setting);
- a post with a `pair:` or `onboard:` marker;
- anything that waits on a compute request: compute is money and is never
  put to the role;
- an ask the role already answered with a pass, recorded in shadow, or that
  `hive_board::decide` refused (events `decide.pass`, `decide.shadow`,
  `decide.refused`): those are the lead's from then on;
- an ask whose call failed (event `decide.failed`, one per try) within the
  last 60 minutes, or 3 times in all: it is not sent again on every pass,
  and after the third try it is the lead's for good.

Right before the write, the ask is checked again as the lead's own path
checks it (`run::apply_result_decisions`): still open, and not a major
dependency upgrade. An ask answered or closed while the model thought is
left alone, with no event.

### The call

One `Provider::chat` call per ask, with no tools, on `decide_provider` with
`decide_model` (`Store::decide_provider`, `Store::decide_model`). Both are
empty by default, which means the hub's own `provider` and a small model
(section 3); no endpoint of its own is named. When every model setting is
empty, the pass asks the provider for its model list once and uses its
default chat model (else its first chat model), and records that name on
the decision: the call never sends an empty model. The resolved name is
kept for an hour per store and provider, so the list is not fetched on
every pass that has asks. A call that errors while that cached default is
in use drops it, so a default the provider withdraws is looked up again on
the next pass instead of failing every pass, and keeping the outage open,
for up to an hour. The prompt
(`decide::decide_prompt`) holds:

- what to answer and how: yes, no or pass, as the project's lead would;
- the yes/no rubric of the lead's brief, `lead::ROUTINE_ASK_RUBRIC` (the
  brief and the role share the one constant);
- the post: kind, id, area/thread, author, summary, what its `Yes:` and
  `No:` lines mean, and its body (clipped at 4,000 characters);
- the thread so far: the other posts of the post's thread, newest kept
  first within 8,000 characters, shown oldest first.

The answer must be one JSON object, optionally inside a code fence:

```json
{"answer": "yes" | "no" | "pass", "reason": "one sentence"}
```

A yes or a no without a reason cannot be read. The reason is folded to one
line and clipped at 300 bytes, cut at a character boundary, plus "..."
when cut: it becomes the decision's words, the
result post and the job brief. The first build does not
depend on token probabilities, which a provider's chat API does not always
return. The deadline is `decide.timeout_seconds`: the provider gets a
`CancelToken` with that deadline, and the call runs in a thread the pass
stops waiting for when it passes, so a provider that does not read the
deadline cannot hold the pass either. The Copilot provider takes the
deadline on the chat request itself as well as on the model-list and token
requests, so a stalled call is cut, not only left behind.

### What follows the answer

| Answer | `decide.mode = on` | `decide.mode = shadow` |
| --- | --- | --- |
| yes or no | `hive_board::decide_with` under the lead's name (`<slug>-lead`), so every gate of a lead's decision applies; the reason is the decision's words | event `decide.shadow` with the answer; nothing decided |
| pass | event `decide.pass`; the ask stays in the lead's brief | event `decide.shadow` |
| error, late | fallback (below); the ask stays in the lead's brief | the same |
| unreadable | one failed try of that ask; no outage | the same |

A yes or no that `hive_board::decide` refuses (a gate the role's list does
not know) writes `decide.refused` with the refusal, and the lead keeps the
ask.

### Falling back, said once

A provider error, no answer within the timeout, or an answer that cannot be
read records one `decide.failed` try on the ask (above). Only a provider
error or a timeout opens the store-wide outage: the provider is out, each
further call would only wait again, the rest of that pass is left to the
lead, and every project's asks go to its lead until an answer comes back.
An unreadable answer is that ask's alone: the pass goes on to the next ask
and no outage opens. Since the role runs on the hub's own
provider, an outage adds no new failure: no run works without the provider
either.

The first fallback of an outage writes one `decide.fallback` event (actor
`daemon`, subject the ask, detail: the reason, the model, the provider).
Later fallbacks are only counted (meta `decide.outage_fallbacks`). An
outage opened while the events table refused the insert has its meta and
no event; the next fallback writes the event then. The next
answer the role can read writes one `decide.recovered` event with the count
of fallbacks the outage had, and clears the outage (meta `decide.outage`).

### What never runs on the decision model

ADR-072's list stands: any run that changes code (task, improve,
experiment, fix); the reviewer's verdict on a pull request; anything at a
hard stop (Tier 2, `OWNER_PATHS`, a major dependency upgrade); anything
about money (compute proposals, `SPEND_SETTINGS`). Only the decide call
reads `decide_provider` and `decide_model`, so the worker and reviewer
paths cannot reach them.

## 2. Who decided, on every row

The `decisions` table has two columns, added through the store's additive
columns (`crates/hive-store/src/ddl.rs`): `decided_model` and
`decided_provider`, both `TEXT NOT NULL DEFAULT ''`. Every writer fills
them:

| Writer | `decided_model` | `decided_provider` |
| --- | --- | --- |
| the decide role (`hive_board::decide_with` with `DecidedBy { role: true, .. }`) | `decide_model` as resolved | `decide_provider` as resolved |
| a lead's `DECIDE:` line (`run::apply_result_decisions_ran`) | the model the lead's run was started with (`run::started_model`: an agent file's pin, else `run_model("lead")`); with none set, the provider's default its session resolved and recorded (`run::resolved_run_model`) | `provider` |
| compute (`crates/hive-tasks/src/compute.rs`), an agent's answer | the agent's recorded model, else `light_model` | `provider` |
| the human, anywhere | empty | empty |

Rows written before the columns read as empty: unknown. The `decide` event
carries `decided_model` and `decided_provider` too; a decision of the role
adds `"via": "decide-role"`, and its result post ends with "Answered by the
decide role (ADR-072): <model> on <provider>."

## 3. Settings

| Setting | Default | Meaning |
| --- | --- | --- |
| `decide_model` | empty | the role's model; empty means `measure_model`, then `light_model` (which itself falls back to `worker_model`); all empty means the provider's default model |
| `decide_provider` | empty | one of the `provider` choices; empty means `provider`, the hub's own |
| `decide.mode` | `on` | `off`: no call, the lead decides as before; `shadow`: answers recorded as `decide.shadow` events, nothing decided; `on`: a yes or a no is decided under the lead's name |
| `decide.timeout_seconds` | `30` | seconds the role waits for an answer before the ask is left to the lead |

On by default means a hub has the role with no owner step and no setup;
setting `decide.mode` back to `off` undoes it, and the lead's `DECIDE:`
lines stay the path for every ask. `decide_provider` stays so that one
setting can later point the role at a paired machine or a local server.

`docs/settings.md` is generated from the program and lists them with the
same descriptions. `decide.memory_mb`, `decide.base_url`, the governor
reservation and a health probe matter only when `decide_provider` names a
server on this machine; Amendment 1 leaves them out of the first build.

## 4. How it is proved

`cargo test -p hive-runner --lib decide::tests` runs the role against a
fake provider (`decidefake`, registered by the tests) that answers yes, no,
pass, prose, an error and a stall. Each test walks the ask to its end: the
decision row with its `decided_model` and `decided_provider` and the
`decide` event, or the ask left in the lead's list with its `decide.pass`,
`decide.shadow` or `decide.fallback` event. Further tests cover the
once-per-outage `decide.recovered`, `off` making no call, an owner at hand
still leaving a routine ask to the role, owner-only, hard-stop and
major-upgrade asks never reaching the role, and the daemon hook running the
pass in the background. The review follow-ups (TASK-137) each have a test
that fails without the fix: an ask closed or marked major during the call
is not written; the lead does not sit for an ask the role holds in the same
tick, with the owner away or at hand, and gets it back after a pass or an
outage; a panic is an outage and the next pass runs; a failing ask waits
while the asks behind it are decided, and is the lead's after three tries;
an empty model resolves to the provider's default and is recorded; the mode
switched off during a pass stops it with nothing refused, and the owner
back during a pass changes nothing; the reason is one clipped line; and a
lead's own decision records the model its run used. The second round
(TASK-148) adds, each failing without its fix: an unreadable answer is that
ask's try and opens no outage; the resolved default model is kept between
passes; a switch to shadow mid-pass keeps calling and only records; a call
error is one try of that ask; a refused ask goes to the lead; a passed,
failed or outage ask reaches the lead with the owner present; repeated
panics back off and a normal pass resets the wait; an outage opened
without its event gets it later; every write refused and a thread that
cannot start both say the next pass tries again; the role and the lead
work from the asks the tick handed them. The third round (TASK-154) adds,
each failing without its fix: a call error drops the cached default model
(`a_call_error_drops_the_cached_default_model`); the back-off and a thread
that cannot start hold no asks from the lead
(`the_back_off_holds_no_asks_from_the_lead`,
`a_thread_that_cannot_start_holds_no_asks_from_the_lead`); one daemon tick
reads the asks once for the role and the lead
(`one_daemon_tick_reads_the_asks_once_for_the_role_and_the_lead`, through
`once_daemon`); and `run_job` itself records on a lead's decisions the
default model its session resolved and an agent file's model
(`run_job_records_the_sessions_default_model_on_the_leads_decisions`,
`run_job_records_the_agent_files_model_on_the_leads_decisions` in
`run.rs`, driving a fake harness). In `run.rs`, a run without a
model records the default its session resolved, and a run as an agent
file is started on the file's model; in `hive-provider-copilot`,
`chat_stops_at_the_callers_deadline_when_the_api_stalls`.
`decide_settings_fall_back_to_measure_light_and_the_hub_provider`
(`hive-store`) covers the settings and their fallbacks.

The runner's test store sets `decide.mode = off` so that no other test
with a routine ask calls a real provider; the role's own tests turn it on.

## 5. Not built yet

- **The replay.** A brain job, started from the console, that scores one
  model and provider against the `decisions` table (`answer IN ('yes',
  'no')`), each ask shown as it stood before `decided_at` and without its
  result post, owner-only asks left out. It reports n, agreement, the
  false-yes and false-no rates, the pass rate and p50/p95 latency, for the
  owner's rows and the leads' rows apart, kept against the pair (model,
  provider). Under Amendment 1 the bars have defaults the owner may change
  and need not set; a replay under the bar moves the role to `shadow`,
  says so once, and the lead decides as before. On 2026-11-26 this hub's
  table held 159 rows: the owner decided 113 (106 yes, 7 no), the leads 46
  (10 yes, 36 no).
- **The console's "Decision model" panel**: the setting, the last replay
  against the bar, and each decision's "decided by <lead> with <model> on
  <provider>".
- **Wider asks.** Later uses (votes,
  answers that need no code reading, the lead's idle planning pass) each
  come by a lead's amendment of ADR-072 with its own replay.
