# Seat role leads on a project

Give one concern of a project, such as its docs or its security, an agent
of its own that looks after it on a cadence. Pause or end it when you want.

**Before you start**

- Hive is installed and the console opens ([Install](../../README.md#install)).
- The project `my-app` is registered and the runner is running
  ([Set up your first project](first-project.md)). The runner is the
  console's word for the daemon (`hive daemon`). It queues each lead's work.

## 1. Pick a role

Every project has one **lead**, here `my-app-lead`: the agent that reads
your tasks and messages, plans the work and tends the board, where the
project's agents post what they find. A **role lead** is one more agent on
the same project. It owns one concern and **sits** on a cadence: each
sitting is one **job**, which is one run of an agent. Only the role lead
runs its sittings, and it takes none of the project lead's work.

Thirteen roles are built in:

| Role | Sits | Looks after |
| --- | --- | --- |
| `rsi` | daily | the self-improvement loop: its plan, votes, experiments and archive |
| `docs` | daily | the guides and the README, kept true to what the code does |
| `security` | weekly | dependencies, secrets, permissions and the attack surface |
| `monitor` | every 6 hours | the checks and watchers that tell when the product is broken |
| `tests` | daily | what is untested, flaky or slow, and whether the check proves what a change claims |
| `deps` | weekly | the lockfile, major versions, deprecations and the toolchain pin |
| `release` | weekly | the changelog, the version and the notes of each release |
| `perf` | weekly | startup, the hot paths, the size of what ships, and the benchmarks |
| `a11y` | weekly | the product as every user meets it: keyboard, screen reader, zoom, a phone |
| `triage` | daily | the repository's issues and pull requests |
| `tidy` | weekly | lint findings, dead code, duplicates and files that outgrew their name |
| `dx` | weekly | the path from clone to green: setup, build and test time, tooling |
| `review` | hourly | the pull requests Hive opened that have no verdict yet |

`hive lead roles --project my-app` prints the same list.

## 2. Ask which roles the project is missing

In the console, open **my-app** under **Projects** in the sidebar and go to
its **Team & tools** tab. The **Leads** panel lists the seated leads and,
under **Hive suggests**, each seat the project shows a need for, with the
reason. In a terminal:

```bash
hive lead suggest --project my-app
```

```text
Tidy-up lead (tidy): the check `npm test` runs no linter or formatter
  hive lead add --project my-app --role tidy
```

Hive never suggests `triage` or `review`: seat those when you want them.
The runner also asks by itself, one seat at a time, as a question on the
project's **For you** tab, such as `Seat a tidy-up lead for my-app?`. **Yes** seats
the lead. It asks about a role again only after `lead.suggest_days` (30)
days, and `0` turns the questions off.

## 3. Seat a lead

In the **Leads** panel choose **Seat a lead**. A gallery opens with a card
for each role nobody sits in. Pick **Guide docs lead**, then choose **Seat
the guide docs lead**. The console answers *Guide docs lead seated. Its
first sitting is queued.* A suggested seat has a button of its own, **Seat
this lead**. In a terminal:

```bash
hive lead add --project my-app --role docs
```

Hive answers that `LEAD-001` is seated as the agent `my-app-lead-docs` and
sits every `1d`. To sit at another cadence, add `--every` with `6h`, `1d`
or `1w`: one hour is the shortest. A project has one lead per role.

## 4. Seat a role of your own

A word of your own makes a new role. It needs a title and a standing brief:
the brief is what the lead reads at every sitting, so say what it owns and
what counts as done. In the gallery, pick **A word of your own** and fill
in **Role word**, **Title**, **Sits** and **Standing brief**. In a terminal:

```bash
hive lead add --project my-app --role content --title "Content lead" --every 1w \
  --brief "Write one post a week in docs/blog, in the voice of the README."
```

The role word is 2 to 24 lowercase letters, digits and hyphens, starting
with a letter. The agent is named after it: `my-app-lead-content`. Hive
refuses a word with `lead` as one of its parts, such as `tech-lead`.

## 5. Follow what the lead does

The runner queues the first sitting on its next pass, then one each cadence.
A sitting's brief is the role's standing brief plus what the project shows
now. It tells the lead to:

- Fix what is small in its own clone and commit. The commits are delivered
  like any other job's ([Review and merge the work](pull-requests.md)).
- Queue jobs for the larger work, three at most (`lead.idle_max_jobs`).
- Post findings and suggestions on the board, in an area named after the role.
- Send you one short message, and only when something changed or needs you.

These are rules the lead is given, not locks. The same brief tells it never
to decide in your place and never to push.

The lead's row in the **Leads** panel says how often it sits, when it sits
next and how many sittings it has had. Its runs are on the **Runs** page and
its posts on the project's **Board** tab. In a terminal:

```bash
hive lead list --project my-app            # each seat: status, cadence, next sitting, sittings
hive lead show LEAD-001                    # one seat, its brief and its sittings
hive board --project my-app --area docs    # the board's digest for the lead's area
hive msg list --project my-app             # the messages between you and the leads
```

`hive lead show` ends with the lead's sittings and the jobs it queued, one
row each:

```text
SITTINGS
JOB                          STATUS  CREATED               FINISHED  BRANCH
job-20261123T042154Z-561bc8  queued  2026-11-23T04:21:54Z
```

## 6. Pause, resume, run now or end

Each row in the **Leads** panel has **Sit now**, **Pause** (then **Resume**)
and **End**. In a terminal:

```bash
hive lead pause LEAD-001 --note "docs freeze before the release"   # no sittings until resumed
hive lead resume LEAD-001
hive lead run LEAD-001        # a sitting now, without waiting for the cadence
hive lead end LEAD-002 --note "the blog moved to another repository"
```

```text
LEAD-001 paused
LEAD-001 active
LEAD-001 sits at the daemon's next pass (within its interval)
LEAD-002 ended
```

The program's messages say *daemon* where the console says runner. Ending
is final. The seat keeps its row and its history, its sittings still in the
queue are cancelled, and a sitting that is running finishes. To have the
role again, seat it again.

## 7. Count what it costs

A role lead costs model calls, because every sitting is a job:

- A sitting runs within the budgets of any job: `minutes_per_job` (10) and
  `harness.max_steps` (60 model calls).
- A lead has one sitting open at a time. While one is still open, the next
  cadence queues nothing. From the cadence alone, a daily lead has one
  sitting a day and the hourly `review` lead 24.
- Sittings count towards `jobs_per_day` (2 by default), the jobs Hive may
  start in one UTC day on all machines together. Once it is reached, a
  sitting waits until midnight UTC, while jobs for your own tasks still
  start. The jobs a lead queues count in the same way.
- Sittings run on the light model (`light_model`), except the `rsi` lead's,
  which run on `worker_model` ([Choose models and set budgets](models.md)).

```bash
hive status                          # "runs today 0 of 2 (jobs_per_day)" is today's count
hive settings set jobs_per_day 10    # 0 lifts the cap
```

## Check that it worked

In the console, the **Leads** panel shows the lead with the badge **Active**,
its agent `my-app-lead-docs` and when it sits next. Once the runner has
queued the first sitting, the row counts it (`1 sitting so far`). In a
terminal, `hive lead list --project my-app` shows the seat as `active`, and
`hive lead show LEAD-001` lists that sitting.

## When something goes wrong

- **`my-app already has a docs lead: LEAD-001`.** A project has one lead per
  role, paused ones included. Look at it with `hive lead show LEAD-001`, or
  end it first.
- **The lead never sits.** The runner queues and runs the sittings, so
  nothing happens while it is off or paused: Sessions then says **Runner is
  off** or **Runner is paused: nothing new starts**. A sitting also waits
  while `jobs_per_day` is reached (the `runs today` line of `hive status`)
  and while the lead is paused.
- **`A content lead needs a title`.** A role that is not built in needs
  `--title` and `--brief`.
- **`30m is shorter than the 1h floor`.** A lead sits once an hour at most.
- **You ended a lead that had work in progress.** Its sittings in the queue
  are cancelled: `hive jobs --all` shows them as `cancelled` with the note
  `lead LEAD-002 ended`. A sitting that was running finishes, and the jobs
  the lead had queued for other agents stay in the queue.
- **A security lead sits that you did not seat.** While `owner.presence` is
  `away`, Hive asks about no seat. It seats the `security` role itself,
  once, when the project has dependencies that nothing audits, and says so
  on the board. `hive lead end` undoes it.

## Read more

- [Role leads](../board.md#role-leads-hive-lead): what each role's brief holds, when Hive suggests a seat, role records.
- [The review lead](../review.md#the-review-lead): what the `review` role adds to the review of each pull request.
- [When the owner is away](../recovery.md#when-the-owner-is-away): what the leads decide while you are away.
- [Lead settings](../settings.md#lead): `lead.suggest_days`, `lead.idle_max_jobs` and the rest.
- [Answer what waits for you](what-needs-you.md): where Hive's question about a seat arrives.
- [Run work on a schedule](schedules.md): the checks and watchers a `monitor` lead keeps.
- [Troubleshooting](troubleshooting.md): when the runner is off or a job fails.
