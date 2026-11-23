# Turn on the self-improvement loop

Open each gate of the self-improvement loop, turn it on, read what it tried
and kept, and turn it off again.

**Before you start**

- Hive is installed and the console opens ([Install](../../README.md#install)).
- A project (`my-app` here) is registered, a model provider is connected
  and the runner runs ([Set up your first project](first-project.md)).
- You know the project's constitution and the kill switch
  ([Set the rules agents work under](rules.md)).

## 1. See where the loop stands

Between your tasks, agents post suggestions for the project and vote on
them. With the loop on, the runner (`hive daemon`, the process that plans
jobs and starts agents) takes the top suggestion and has an agent build it
on a branch `evolve/<id>`. It scores that branch and the default branch on
the same test cases: the public ones, the suggestion's own check, and
private tests that agents are not shown. A change that scores better
arrives as a pull request. Any other is dropped. Each measured try is archived.

The loop is off until you turn it on. For a newly registered project,
before the runner has done anything, this is where it stands:

```bash
hive evolve status --project my-app
```

```text
SELF-IMPROVEMENT LOOP - my-app
STOP                 false
Loop                 false
Limits               none
Tolerance            0.0200
Baseline             none
Public fixtures      8
Held-out fixtures    0 ready; 0 pending
Editable scope       2 of 5 class(es): product polish (Stage 4), project docs (Stage 2); ...
Level allowed        B - no baseline measured yet; the daemon records it once the loop is on
Next human step      hive settings set evolve.loop 1  (then the daemon plans the baseline job ...
```

Nine more rows are left out here. The output uses the program's words: a
*fixture* is a test case, a *held-out fixture* a private test, the
*baseline* the starting score, the *daemon* the runner. Experiments start
only when `Level allowed` says `D`. `Next human step` names your step, or
starts with `none` when the next thing is the runner's. The console shows
the same on the project's **Self-improvement** tab, under **Show details**.

## 2. Check the records and the test cases

Registering the project opened the first gates. It wrote the constitution
and the evaluators (the console says **The rules (constitution)** and
**Scoring rules**), and it added public test cases:

```bash
hive evolve fixture list --project my-app
```

```text
PROJECT  NAME                          KIND    MODE     WEIGHT  ENABLED
my-app   derived-tests                 public  metric   1       true
my-app   derived-todo-markers          public  metric   1       true
```

Six `starter-*` lines follow. Those cases judge an agent's conduct: the
right tier, a hard stop refused, a planted instruction ignored. The
`derived-*` cases, added for a Rust, Node, Python or Go project, count
things in your tree, such as test declarations and to-do markers. If
`evolve status` says a record or the test cases are missing,
`hive evolve seed --project my-app` adds what is missing and keeps the rest.

## 3. Name the files the loop may change

The `Editable scope` line says which groups of files a suggestion may be
about. It comes from the table `Editable scope by stage` in the
constitution. The program reads a row's paths and stage: stage 2 opens a
group, product code needs stage 4, and a group whose paths are not in your
checkout is closed. Its `Autonomy` cell changes nothing. The loop passes
over a suggestion that names a file outside the open groups. What the agent
then changes is not checked against the table: that is a rule it is given.
The template's text names example paths, so make the text your own first.

The runner has an agent draft the constitution and the scoring rules from
your checkout. On the project's **Self-improvement** tab, **Draft it for
me** asks for a draft now. **Read & adopt**, then **Adopt**, makes it the
next revision in your name (`hive record adopt EVO-constitution-my-app` in
a terminal). `evolve status` then names the groups that opened. Or write
the text yourself ([Set the rules agents work under](rules.md)).

The scoring rules also carry the line `Product scope: allowed`. Changed to
`operating-layer`, it keeps the loop to suggestions about tests, tooling
and docs ([the reference](../self-improvement.md#fleet)).

## 4. Choose who writes the private tests

Private tests are test cases kept outside every repository, in a folder
that only your user can read, so that a change is scored on tests it was
not tuned to pass. What keeps a job from reading them is a rule on its
commands: Hive refuses one that names the `private/` folder under
`~/.hive`. The rule works on the text of the command and is not a sandbox:
a job runs as your user ([Agents](../security.md#agents)).

The setting `evolve.heldout` says who writes them, one set for all projects:

- `auto`, the default: the runner writes them in `~/.hive/private/heldout`,
  with the loop on or off. Each is one model call that is given nothing
  from your repository. Two are enough for experiments to start. It goes
  on to six and writes a test anew after 30 days.
- `owner`: you write them. Run `hive settings set evolve.heldout owner`.
  Once the starting score is measured, the project's **Self-improvement**
  tab shows the step **Set up two private tests**, and **Set up private
  tests** opens a dialog: choose a folder, then write each test as a request
  and a way to score it. If the runner already wrote two, no step shows: use
  **Add or rewrite private tests** under **Show details**. From a terminal,
  `hive evolve setup DIR` makes the folder with two template pairs to fill
  in. The command rule covers folders under `~/.hive/private` only:
  elsewhere Hive only keeps the path out of what it prints.

## 5. Set the limits

Look at what bounds the loop before you turn it on. Each is a setting,
changed with `hive settings set NAME VALUE` or on the **Settings** page:

| Setting | Default | What it bounds |
| --- | --- | --- |
| `jobs_per_day` | `2` | Jobs that may run in a UTC day, the loop's among them. Past it only your own work starts: tasks, answers to you, the lead's sittings, reviews, security updates. `0` means no cap. |
| `evolve.experiments_in_flight` | `2` | Experiments one project may have going at once. |
| `minutes_per_job` | `10` | Minutes one run may take, and each scoring run. |
| `evolve.limits` | `none` | A budget such as `cycles=20,hours=48` (a cycle is one measured try), after which the loop stops by itself. |
| `review.merge` | `auto` | `auto`: the review agent merges a kept experiment when no rule hands it to you. `ask`: every pull request waits for your merge. |

Scoring is model work too: each test case that asks an agent something, the
starter ones and the private ones, runs three times per measurement. The
program does not read the constitution's `Budget` table: it bounds nothing.
The limits and the switch below are yours by rule, not by lock: an agent's
`hive settings set` is not refused ([the rules guide](rules.md)).

## 6. Turn the loop on

```bash
hive settings set evolve.loop 1
```

In the console it is **Settings**, **Self-improvement**, the switch **Run
experiments continuously**. There is one switch for all your projects: it
turns the loop on for every project whose gates are open.

The runner does the rest, and `hive evolve status` follows it:

1. It queues a job titled `Record baseline` and scores the default branch.
   Until then `Next human step` reads `none - the daemon plans the baseline
   job on its next tick (hive daemon)`.
2. Two private tests must be ready: `Held-out fixtures` reads
   `2 ready (2 generated); 0 pending` or more when the runner wrote them.
3. `Level allowed` turns to `D - the loop may run until a stop rule fires`.
   The loop takes the top suggestion that passed the gates, and the next
   one whenever a place is free. It looks in each planning pass
   (`daemon.pass_seconds`, 60): there is no timetable.

A try is kept when its total score is at least `evolve.tolerance` (0.02)
above the default branch's, even if one test case fell. A gain is measured
a second time before it counts. A try that falls more than the tolerance
below is discarded. In between is a tie. While `evolve.require_check` is
on (the default), a tie is kept only when the suggestion's own check scores
higher on the branch (say, from failing to passing). A change to lock files
alone is delivered unmeasured.

A kept try arrives as a pull request whose body is the measurement. These
wait for you:

- A pull request that touches a guarded path, such as a workflow file or
  a file named like a secret, and every pull request when `review.merge`
  is `ask` ([Review and merge the work](pull-requests.md)).
- A stop. When the project's newest experiment failed and its result names a
  hard stop or a violation, or a post in an open thread reads like an
  instruction to agents, the loop plans no new experiment in that project
  and asks `Evolution stopped; owner review needed`. The console words it
  **Self-improvement stopped. May it carry on?** A yes lets it plan again.
  So does a no after a failed experiment, and a stop over a post ends when
  any agent closes its thread. To keep the loop stopped, turn it off.
- A pause after `evolve.flat_cycles` (6) ties in a row. With `evolve.heal`
  on (the default) the loop first adds `derived-*` cases read from your
  tree, and it turns off a test case whose score on one commit flipped
  twice. With no answer after `evolve.flat_wait_hours` (24) it answers that
  one question itself. Set the hours to `0` to make it wait for you.

## 7. Read what it did

```bash
hive evolve archive --project my-app      # one row per measured try
hive evolve report --project my-app       # the last 7 days in counts
hive evolve lineage --project my-app      # which try was built on which
```

The archive's `DECISION` column says `baseline`, `improved`, `neutral` or
`discarded`. The report's `Experiments` line counts the tries planned, kept
and discarded, and `Owner inbox` the questions that wait for you. Its
`Landings` line counts local landings only, which a project with no upstream
has, not merged pull requests: read `Experiments` and `Lineage`.

In the console, the **Self-improvement** tab lists every try under
**Experiments so far**, with **Kept** or **Dropped**, and shows under
**From suggestion to change** where the suggestions stand. A kept try's
pull request starts with `Experiment <job> (archive <id>), measured and kept.`

## 8. Turn it off

```bash
hive settings set evolve.loop 0
```

No new experiment is planned. One already queued or built is still finished,
measured and, when kept, delivered. Agents still post suggestions and vote,
and a suggestion that passes the gates is still built as an ordinary job,
which must turn its own check from failing to passing. To stop the runner
altogether, use the kill switch ([the rules guide](rules.md)).

## Check that it worked

- `hive evolve status --project my-app` shows `Loop` as `true`, and
  `hive jobs --project my-app` lists `Record baseline` until it has run.
  Then `Baseline` shows an archive id and a score, and `Level allowed`
  starts with `D` once two private tests are ready.
- On the project's **Self-improvement** tab, **Show details** says
  **Switched on**, and **Experiments so far** fills as tries are measured.

## When something goes wrong

- **A gate stays closed.** `Next human step` names it. When it starts with
  `none`, the step is the runner's: `hive status` shows whether a runner
  works (the `daemon` line) and whether the day's cap is reached
  (`runs today 2 of 2 (jobs_per_day)`).
- **The starting score is never measured.** A failed measurement is tried
  again after `baseline.retry_hours` (6), and
  `hive job add --project my-app --kind baseline --title 'Record baseline'`
  queues one now. `hive evolve measure` only prints scores.
- **The loop is on and nothing happens.** The `Experiment planning` line
  says why. `waiting - no eligible, untried, in-scope suggestion with net
  votes > 0; vote with hive ballot` means no suggestion on the board is one
  the loop may try. The **Votes** page shows each suggestion and what holds
  it. No experiment is measured while the setting `landing` is `human`.
- **Results stay flat.** The loop asks `Evolution paused: the fixtures do
  not tell variants apart`. Add a test case that counts something in your
  code (below, lower is better). Or answer the question: yes or no, either
  lets the loop run six more tries as it is. To keep it paused, turn the
  loop off.

```bash
hive evolve fixture add --project my-app --name stray-logs --metric --scorer \
  "hive evolve score metric --lower --scale 20 -- sh -c 'grep -rn console.log src | wc -l'"
```

## Read more

- [Seeded records and starter fixtures](../self-improvement.md#seeded-records-and-starter-fixtures):
  the records, the test cases and how each is scored.
- [Card check](../self-improvement.md#card-check): the check a suggestion names.
