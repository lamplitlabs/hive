# A decision model on a local provider: where it plugs in

Status: design note (Tier 1). Nothing here is built or enabled. Whether a
model on a local provider may answer the owner's routine asks, and under
which guards, is up to **ADR-072** ("A decision model on a local provider
answers the routine asks of an away owner"). ADR-072 is a record in the
store, not a file under `docs/decisions/`; read it with
`hive record show ADR-072`. The owner accepted it on 2026-11-26, and its
delivery is "not started". The owner's acceptance note changes two of its
parts: "this decision can also be used for other things as well, not only
the owner decision ... on by default". So the role is on by default, and
the later candidates (votes, answers, idle planning) are in scope. The
never-list still stands: runs that change code, a reviewer's verdict on a
pull request, hard stops and money. On by default still changes nothing
while `decide_model` or `decide_provider` is empty, or while no local
server answers, because every such ask falls back to the main provider.
This note shows where each part of that record fits into the
code as it stands, so the work that delivers it is plumbing, not new
design. Where this note and the ADR differ, the ADR wins.

Task: TASK-128. The owner asked for some runs to move to a local model,
starting with decisions.

```text
 open owner ask (owner away)
        │
        ↓
 decide.mode? ── off ─────────────────────→ lead brief, as today
        │                                       ↑
        │ shadow / on                           │ pass, refused, late,
        ↓                                       │ unreadable (said once)
 one call: decide_model on decide_provider ─────┘
        │
        │ yes / no  (on only; shadow records it and stops)
        ↓
 hive_board::decide ──→ decisions row with decided_model,
                        decided_provider
```

## 1. Settings

The settings are new rows in the catalogue of
`crates/hive-store/src/settings.rs`, beside `light_model` (line 103) and
`provider` (line 123):

| setting | kind | default | says |
|---|---|---|---|
| `decide.mode` | choice off, shadow, on | `on` (owner's acceptance note; the record said `off`) | whether the decide role answers; it answers only once a replay of its model and provider meets the bar, and shadows until then |
| `decide_model` | text | empty | the model of the decide role; empty means no role |
| `decide_provider` | choice, the `provider` list | empty | the provider of the decide role, read only by the decide call |
| `decide.base_url` | text | empty | an endpoint of its own, when `decide_provider` is the same vendor as `provider` (two `custom` servers) |
| `decide.timeout_seconds` | number | 30 | the request deadline; later than this is a fallback |
| `decide.memory_mb` | number | 6144 | what the local server takes; the governor reserves slots for it |
| `decide.bar_agreement` | number | empty | the owner's bar: the lowest agreement a replay must show |
| `decide.bar_false_yes` | number | empty | the owner's bar: the highest false-yes rate |
| `decide.bar_min_noes` | number | empty | how many recorded noes a replay needs before its rate counts |

For llama-server on this machine, set `decide_provider = custom`,
`provider.custom.base_url = http://127.0.0.1:8080/v1` and
`provider.custom.no_key = on` (lines 141-142). Ollama, LM Studio and vLLM
already have local vendor entries (`crates/hive-provider-openai/src/lib.rs`,
lines 272-300).

## 2. The call

The provider is opened with `hive_provider::open(decide_provider, ...)`
(`crates/hive-provider/src/lib.rs:962`), the same way the sealed
held-out call opens it in `crates/hive-runner/src/heldout.rs:26-35`, with
`CancelToken::with_timeout(decide.timeout_seconds)`. No other call site
reads `decide_provider`, so the worker and reviewer paths
(`crates/hive-runner/src/exec.rs` `worker_model`, line 1424, and the
harness's `--provider`) cannot reach it.

The asks are the same ones the lead may decide today:
`lead::OWNER_ASK_DECIDE_KINDS` (`crates/hive-runner/src/lead.rs:67`),
open by `lead::owner_ask_is_open` (lead.rs:1042), and not
`hive_board::owner_only_proposal` (`crates/hive-board/src/jobs.rs:781`)
or `lead::is_security_major_finding` (lead.rs:1027). The checks that
`apply_result_decisions` makes (`crates/hive-runner/src/run.rs:3335`)
are shared, not copied.

The model must answer with
`{"answer": "yes" | "no" | "pass", "reason": "..."}`. Anything else is
unreadable.

## 3. The record of who decided

`decisions` (`crates/hive-store/src/ddl.rs:146-154`) gets
`decided_model TEXT NOT NULL DEFAULT ''` and
`decided_provider TEXT NOT NULL DEFAULT ''`. They are added through
`additive_columns` (`crates/hive-store/src/store.rs:1090`). These are the
writers:

- `hive_board::decide` (jobs.rs:858). The decide role passes its own pair.
  The lead's `DECIDE:` lines pass `light_model` and `provider`. The human
  passes empty.
- compute proposals (`crates/hive-tasks/src/compute.rs:333`). They stay
  empty, because money is never the model's.

## 4. The replay

The replay is a brain job, started from the console, that scores one
model and provider against the table:

```sql
SELECT d.post_id, d.answer, d.decided_by, d.decided_at, p.summary, p.body
FROM decisions d JOIN posts p ON p.id = d.post_id
WHERE d.answer IN ('yes', 'no')
```

Each ask is shown as it stood before `decided_at`: the thread's posts up
to then, without the result post. Owner-only asks are left out, because
the role never sees them. The report gives n, the agreement, the
false-yes rate (model yes where the record says no, out of the record's
noes), the false-no rate, the pass rate, and p50/p95 latency. Each figure
is shown for the owner's rows and the leads' rows separately. The report
is kept against the pair (model, provider), and `decide.mode = on` reads
the latest one.

On 2026-11-26 this hub's table held 159 rows. The owner decided 113
(106 yes, 7 no) and the leads decided 46 (10 yes, 36 no).

## 5. The governor

When the decide endpoint is a loopback address and answers its health
probe, the runner passes
`ceil(decide.memory_mb / capacity::MB_PER_RUN)` as `reserved` to
`daemon_load::decide` (`crates/hive-runner/src/daemon_load.rs:219`;
`MB_PER_RUN` is in `crates/hive-runner/src/capacity.rs:16`). On this
16 GB hub that is 2 of its 5 slots. A replay counts as a measuring run.
For an endpoint on a paired machine, nothing is reserved here.

## 6. Tests the implementation will need

- A fake provider that refuses, stalls past the timeout, and answers in
  prose. Each case leaves the ask in the lead's brief and writes one
  `decide.fallback` event per outage.
- `decide.mode = off` and an empty `decide_model` reach the provider zero
  times.
- An owner-only proposal, a major upgrade and a compute proposal are never
  sent to the model.
- A replay on a seeded store with known answers gives the expected
  agreement and false-yes rate, and never shows the model the result post.
- `daemon_load::decide` with a reserved local model holds the slots.
