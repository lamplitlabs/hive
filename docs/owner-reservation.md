# The owner reservation

Design note for TASK-067. The surrounding behaviour, the express lane and the
yield, is described in [landing.md](landing.md), section *The express lane for
the owner's work*. This note covers the three rules added on top of it and
the reasons for them.

## The problem

The daemon log showed 177 load holds in a day, most with the load between 24
and 32 on a 12-CPU machine. The fleet's own builds and checks caused that
load. While it lasted, the owner's queued tasks waited in three ways:

- **The slots.** Under the run cap the fleet took every free local slot as
  soon as it opened. A queued owner job raced the fleet for each one and often
  lost.
- **The load line.** Above twice `daemon.max_load`, the express lane closed,
  so nothing started, the owner's work included. The fleet's runs kept
  the load there, so the owner's job could wait for hours behind the work
  that caused the hold.
- **The yield.** A fleet run in the owner's lane was asked to hold (a
  `job.yield` event) even when the governor would then refuse the owner's job
  anyway: past the hard load line, under the memory floor, or behind a console
  drain. The fleet run threw away its step, the freed slot stayed empty, and
  the same run was leased straight back.

## (a) One local slot kept for the owner

Whenever owner-driven work is queued that this machine could run, one local
slot is reserved for it (`daemon_load::reserve_for_owner`). Owner-driven work
is `jobs.rs` `OWNER_DRIVEN_SQL`: lead sittings, answers, `task:` and
`security:` jobs. The job must belong to an active project, must not be unfit
here, and the local executor must offer what it needs (`jobs.rs`
`owner_work_queued_here`).

- **Under the cap**, the reserved slot is one of the governor's free slots. The
  first local slot offered on the tick takes owner-driven work only, and the
  fleet gets what is left (`room - 1`). If the owner job cannot be leased this
  tick (its lane is still held, for example), the slot stays empty. The idle
  line then reports a hold, not starvation. The owner's work leases past its
  project's share, but the last free local slot of a tick is kept back from a
  `task:` or `security:` job past that share (`LeaseOpts::keep_slot`), so a
  lead's sitting, an answer, a review or another project's owner task still
  finds a slot.
- **Over the cap, over `daemon.max_load` at any height, or under
  `daemon.min_free_mb`**, the reservation becomes the owner's one run over the
  line: one restricted slot that only the owner's work can take. Only one
  such run is allowed at a time. While an owner-driven run is in flight here,
  the next one waits for the machine like everyone else. The fleet stays
  queued throughout.
- **Critical memory pressure from the kernel** (the machine is swapping) keeps
  the slot shut. Starting anything then only makes the machine slower for
  every run, the owner's included.
- **A lead job queued while its own lead is sitting** reserves nothing. The
  lead takes the job afterwards, and a slot kept for it would sit empty.

The line `one express slot stays open for the owner's work` is the same as
before, so the hub and the idle-hold kinds read it unchanged.

## (b) A yield only where the owner job could start

The yield pass (`yield_lane.rs`) now receives an `OwnerAdmission`, which the
daemon builds from the same readings the governor uses on that tick. A fleet
run is asked to hold only if the owner job it makes way for could start once
the lane is free:

- **The lane.** The lanes in the owner job's way are held, some of them by
  the fleet: the project's narrowed lanes (fewer than its share), the owner's
  bound under `verify.lock` (share plus one), or every slot of the fleet. The
  fleet is full when no slot would be offered to the owner's job this tick:
  no free slot on a paired machine and no room the load governor gives here
  (`daemon_load::owner_room`). This is not the fleet's nominal total, which
  stays the same when the kernel's memory warning halves the cap. The
  plain share is not in its way, since the owner's work leases past it; a
  fleet run is not asked to hold while a free slot would start the owner's job.
- **Everywhere.** No console drain is running (a drain starts nothing), and
  neither the bulk volume nor the disk floor would refuse the start.
- **Local.** The login gate is open, and `daemon_load::owner_refusal` returns
  nothing. That function is `decide` with the yielding run removed, plus the
  reservation. Because it is the governor's own answer for the next tick, the
  yield and the dispatch cannot disagree.

When the machine would refuse the owner job, no `job.yield` event is written
and the fleet run keeps its step. The daemon prints the reason once per change
instead of once per tick: `no fleet run is asked to hold: the owner's <job>
waits for a lane, but <reason>`. A typical reason is `load 60.0 on 12 CPUs is
over the line 24, and the owner's one run over the line is already in
flight`.

A restart window does not block the yield, because the owner's work fits
every restart window: its run anchors the window rather than outlasting it
(see [runner.md](runner.md)).

## (c) One event per long wait, naming the holds

What the owner could not see on 2026-11-03 was *why* the task waited. On each
tick the daemon notes, for every owner-driven job still queued, the holds that
tick's dispatch saw, in the owner's words: `lanes full`, `load 27 > 24`,
`memory 300 MB < 1024 MB`, `restart drain`, `disk floor`, `bulk disk away`,
`daily cap reached`. Once a job has waited ten minutes
(`hive_tasks::OWNER_WAIT_MINUTES`) the daemon writes one `job.owner-wait`
event on it with every reason seen so far and logs `owner job <id>
(<project>) waiting since 04:40 UTC: lanes full, load 27 > 24`
(`crates/hive-runner/src/owner_wait.rs`). `hive status` lists the open
waits, and the task's wait line on the board and the hub names the reasons
in place of "waiting for a free agent". A job queued again later is a new
wait with its own event. The reading side is described in
[runner.md](runner.md).

With (a) in place the holds such an event names are the ones that still
refuse the owner: the owner's one run over the line already in flight, the
kernel's critical memory pressure, a console drain, the disk, the daily cap.

## Proof

- `daemon_load` unit tests:
  - `the_owner_reservation_keeps_one_slot_and_runs_over_the_load_line`
  - `the_owner_is_refused_only_where_the_governor_would_refuse_it_after_a_yield`
- `yield_lane` tests:
  - `no_fleet_run_is_asked_to_hold_while_the_freed_lane_could_not_start_the_owners_job`
  - `a_fleet_run_is_asked_to_hold_once_the_machine_would_admit_the_owners_job`
- Daemon tests with a live daemon:
  - `daemon_starts_the_owners_job_over_the_load_line_while_the_fleets_stays_queued`:
    the fleet fills both slots and the load is pinned at 60 (line 24). The
    owner's job starts on the reserved slot while the fleet's stays queued,
    and the owner's next step waits while the first run is in flight.
  - `daemon_asks_no_fleet_run_to_yield_while_the_load_would_refuse_the_owners_job`:
    no `job.yield` is written while the load would refuse the owner. Exactly
    one is written once the load would admit the owner.
  - `daemon_names_the_holds_of_an_owner_job_queued_past_ten_minutes`: with
    the load pinned at 60 (line 24), the owner's first job starts on the
    reservation and the owner's next step, queued twenty minutes earlier,
    gets one `job.owner-wait` event whose reasons read `load 60 > 24`; the
    `hive status` line says the same.
- `owner_wait` unit tests cover the short reasons, the ten-minute line, one
  event per wait and the status lines; `hive_tasks::owner_wait` the wait
  line the board and the hub show.
- The existing `yield_lane` tests and
  `daemon_lends_the_owner_one_slot_over_a_cap_full_of_fleet_work` still pass.
- The `idle_hold` load and memory question tests (`a_load_hold_asks_…`,
  `a_memory_hold_asks_…`) used to rely on the old rule that a load or memory
  hold alone shut the owner out. Under the new rule the owner's run starts, so
  these tests now also pin the kernel's critical memory pressure, the only
  hold that still refuses it. The questions they check are asked as before.
