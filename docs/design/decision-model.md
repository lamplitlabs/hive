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
 open owner ask (owner away)
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

While `owner.presence` is `away`, the lead decides the findings, handoffs
and suggestions marked for the owner: its brief lists them and it answers
each with a `DECIDE: <post> yes|no <reason>` line. The decide role answers
the same asks first, with one short call and no tools, before the lead's
pass reads what is left.

On each daemon pass, for each project, `crate::decide::tend_decide`
(`crates/hive-runner/src/decide.rs`, called from the per-project loop in
`daemon.rs` just before `lead::tend_lead`, stage `decide`) starts one decide
pass in a background thread when:

- `decide.mode` is `on` or `shadow`;
- the owner is away (`Store::owner_away`), the condition under which the
  lead decides the owner's routine asks today (TASK-129 widens that
  condition; this slice does not);
- the project's lead is an active agent;
- at least one ask waits.

The pass never holds the daemon's loop: it may wait up to
`decide.timeout_seconds` per ask, so it runs beside the loop, and the lines
it writes are said on the next pass.

### Which asks

`decide::decide_asks` starts from `lead::owner_addressed_asks`, the list the
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
  `decide.refused`): those are the lead's from then on.

### The call

One `Provider::chat` call per ask, with no tools, on `decide_provider` with
`decide_model` (`Store::decide_provider`, `Store::decide_model`). Both are
empty by default, which means the hub's own `provider` and a small model
(section 3); no endpoint of its own is named. The prompt
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

A yes or a no without a reason cannot be read. The first build does not
depend on token probabilities, which a provider's chat API does not always
return. The deadline is `decide.timeout_seconds`: the provider gets a
`CancelToken` with that deadline, and the call runs in a thread the pass
stops waiting for when it passes, so a provider that does not read the
deadline cannot hold the pass either.

### What follows the answer

| Answer | `decide.mode = on` | `decide.mode = shadow` |
| --- | --- | --- |
| yes or no | `hive_board::decide_with` under the lead's name (`<slug>-lead`), so every gate of a lead's decision applies; the reason is the decision's words | event `decide.shadow` with the answer; nothing decided |
| pass | event `decide.pass`; the ask stays in the lead's brief | event `decide.shadow` |
| error, late, unreadable | fallback (below); the ask stays in the lead's brief | the same |

A yes or no that `hive_board::decide` refuses (a gate the role's list does
not know) writes `decide.refused` with the refusal, and the lead keeps the
ask.

### Falling back, said once

A provider error, no answer within the timeout, or an answer that cannot be
read leaves the ask to the lead, who decides it as before the role. The
rest of that pass is left to the lead too: the provider is out, and each
further call would only wait again. Since the role runs on the hub's own
provider, an outage adds no new failure: no run works without the provider
either.

The first fallback of an outage writes one `decide.fallback` event (actor
`daemon`, subject the ask, detail: the reason, the model, the provider).
Later fallbacks are only counted (meta `decide.outage_fallbacks`). The next
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
| a lead's `DECIDE:` line (`hive_board::decide` with a lead's name) | the lead's run model (`run_model("lead")`) | `provider` |
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
once-per-outage `decide.recovered`, `off` and an owner at hand making no
call, owner-only and major-upgrade asks never reaching the role, and the
daemon hook running the pass in the background.
`decide_settings_fall_back_to_measure_light_and_the_hub_provider`
(`hive-store`) covers the settings and their fallbacks.

The runner's test store sets `decide.mode = off` so that no other test of
an away owner calls a real provider; the role's own tests turn it on.

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
- **Wider asks.** TASK-129 has the leads decide routine asks whatever
  `owner.presence` says; the role answers those first. Later uses (votes,
  answers that need no code reading, the lead's idle planning pass) each
  come by a lead's amendment of ADR-072 with its own replay.
