# Evolve settings

The evolve loop's gates are settings. Read the effective value with
`pulse settings get NAME`, change it with `pulse settings set NAME VALUE`
(only the owner changes them; agents and variant overlays cannot, since
`evolve.*` is in the immutable set), or edit them on the console's Settings
page. `pulse settings list --json` prints every default, rule and description.
A value outside the rule is refused with the reason, and the setting keeps its
old value.

This page covers the evolve settings that [Democracy and
self-improvement](self-improvement.md) does not explain. The loop itself, and
`evolve.loop`, `evolve.limits`, `evolve.tolerance`, `evolve.land_neutral`,
`evolve.experiments_in_flight`, `evolve.heldout`, `evolve.measure_parallel`,
`evolve.flat_cycles`, `evolve.heal`, `evolve.flat_wait_hours`,
`evolve.baseline_ttl_hours`, `evolve.parent_rate`, `evolve.outcome_hours`,
`evolve.mode` and `evolve.keep_days`, are covered there.

## Drafting and held-out fixtures

| Setting | Default | Takes | What it does |
| --- | --- | --- | --- |
| `evolve.draft_hours` | `24` | whole number, `0` or more | Hours between the `reflect` drafting jobs (source `draft:<slug>`) the planner queues for a project whose constitution or evaluators is still the seeded template text (ADR-046). `0` turns the planner's drafting off; the console's **Draft it for me** still queues one, and the planner says `the planner's drafting jobs are off (evolve.draft_hours 0)`. A value that does not parse counts as `24`. See [Seeded records](self-improvement.md#seeded-records-and-starter-fixtures). |
| `evolve.heldout_dir` | empty | a directory path, never a credential | The owner-private directory holding the private held-out fixtures (`H01.request`, `H01.score`, ...). Empty until something sets it: under `evolve.heldout auto` the daemon sets it to `~/.pulse/private/heldout` the first time it writes a fixture; `pulse evolve setup DIR` and the console's Held-out fixtures dialog set it to the directory you pick. The directory must be outside every registered project and the fleet area, mode `0700`. The path is never quoted to agents: it is masked in events, hook details and logs, and a request under it cannot be imported as a public fixture. |

## Agent-program variants (ADR-049)

These seven gates belong to the variant loop of `evolve.mode=variants`, which
is being built (see [Agent-program
variants](self-improvement.md#agent-program-variants-adr-049-being-built)).
The store validates and keeps them today, so you can set them ahead of time;
under the default `evolve.mode=fixtures` none of them changes anything.

| Setting | Default | Takes | What it does |
| --- | --- | --- | --- |
| `evolve.trigger_failures` | `5` | whole number, `1` or more | Jobs on one project that fail for the same cause within 7 days before the daemon queues a job that proposes a variant to fix that cause. |
| `evolve.replay_set` | `20` | whole number, `0` or more | Recorded past trials of the project a new variant replays in throwaway clones (verify only, never merged) before it may canary, scored against the incumbent's replay of the same set. `0` skips replay. |
| `evolve.canaries_per_island` | `3` | whole number, `0` or more | Live canary variants one project (an island) may carry at once. `0` lets none canary. |
| `evolve.canary_share` | `20` | percent, `0` to `100` | Share of a project's trials (task and improve jobs) that the router hands to a canary instead of the incumbent while the project has live canaries. `0` routes every job to the incumbent. |
| `evolve.min_trials` | `40` | whole number, `1` or more | Trials with a known kept status (see `evolve.keep_days`) a canary needs before the promotion test may promote or retire it. |
| `evolve.promote_confidence` | `0.95` | decimal, `0` to `1` | Posterior probability that the canary's kept landings per million tokens beat the incumbent's before it is promoted. `0.99` is stricter, `0.9` promotes sooner on less evidence. |
| `evolve.guard_margin` | `10` | percent, `0` to `100` | How far, relative to the incumbent, a variant may worsen a guard - failed-job rate, owner interventions per kept landing, verify flake rate - and still be promoted. A replay worse than the incumbent's by more than this retires the variant. `0` allows no worsening at all. |

## Tuning the gates

- **Slower, safer promotion:** raise `evolve.min_trials` and
  `evolve.promote_confidence`, lower `evolve.guard_margin`. A variant then
  needs more kept-or-reverted evidence before it replaces the incumbent.
- **Less real work exposed to candidates:** lower `evolve.canary_share` or
  `evolve.canaries_per_island`; `0` in either stops canarying without turning
  the mode off.
- **Fewer proposals:** raise `evolve.trigger_failures`, so only a cause that
  keeps failing gets a variant written for it.
- **Cheaper vetting:** lower `evolve.replay_set`; `0` sends new variants
  straight to canary, so pair it with a small `evolve.canary_share`.
- **Less drafting:** raise `evolve.draft_hours`, or set `0` and draft from the
  console when you want one.

Example:

```sh
pulse settings get evolve.promote_confidence     # 0.95
pulse settings set evolve.promote_confidence 0.99
pulse settings set evolve.canary_share 101       # refused: evolve.canary_share is a percent: it cannot exceed 100, not 101
```
