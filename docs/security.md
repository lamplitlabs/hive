# Security notes: where your code and your data live

This page is the promise ADR-070 rule 6 makes to a team, and what each part
of it rests on. It names what is built and tested today, and what still
waits on a later step of ADR-070. Nothing here is a promise until its test
exists. The tests are named so you can run them.

## Where code runs is your choice, written down

A team's **brain** is the PULSE daemon, its store and its console (the hub).
It plans the work, keeps the board and lands branches. A team's **hands** are
the machines its members enrol ([machines.md](machines.md#the-hand)). The
coding harness runs there, in a clone of the project, along with the build,
the tests and the push of the branch.

One setting says whether the brain keeps a copy of your code:

| `brain.code` | What the brain holds | Who runs what |
|---|---|---|
| `clone` (the default) | A clone of each project, as a laptop brain always has | The brain may run any job itself. Hands run what is routed to them. |
| `none` | **No checkout of any project, ever** | The brain runs only the jobs that plan from the board: the project lead and the reflection (`reflect`). They work in an empty workspace and read the task text and what the hands report. Every other job (task, improve, review, evolve, check scripts) waits for a hand. |

```
pulse settings set brain.code none
```

The hosted image sets `brain.code none` on every start
(`deploy/entrypoint.sh`). On a laptop it is under **Settings → This machine
→ Project code on this machine**.

What `none` changes, and the test that holds each part:

- **Leasing.** The brain's own executor leases only `lead` and `reflect`
  jobs. Anything else stays queued for a hand (`crates/pulse-runner/src/jobs.rs`
  `lease_job`, `needs_code_here`).
- **Running.** A run started by hand (`pulse run JOB`) for a job that needs
  the code is refused with "brain.code is none". A lead or reflection gets
  an empty git repository with one empty commit, never a clone
  (`crates/pulse-runner/src/exec.rs` `prepare_no_code_workspace`).
- **Schedules.** Check and watch scripts run in a worktree of the project,
  so on a brain without code they are held and the hold is said once
  (`crates/pulse-runner/src/schedule_script.rs`).
- **Self-update.** The source self-update builds from the brain's own
  checkout, so with `none` it stands down. Use `daemon.self_update release`
  instead (see "Updates" below).
- **The test.** `cargo test -p pulse-runner --lib
  daemon::tests::brain_code_none_leaves_no_project_checkout_after_a_pass`
  queues a task and a reflection on a brain with `brain.code none` and runs
  one full daemon pass. It checks three things: the reflection ran in a
  workspace with no project files, the task is still queued, and the
  brain's work directory holds no checkout of the project. It also checks
  that `pulse run` of the task is refused.

## One brain per team

A hosted brain is one container with one volume (`deploy/Dockerfile`,
`deploy/brain.sh`). Two teams never share a container, a volume, a store or
a key. Everything a brain keeps is on that volume, under
`PULSE_HOME=/data/pulse`. That covers the SQLite store and its WAL, the work
directory, the logs, the run keys and the console's state. The image
itself holds no PULSE source and no project code. The source tree stays in
the build stage, and `.dockerignore` keeps `.git`, build output, env files,
keys and stores out of the build context.

The container runs as an unprivileged user (uid 10001) with every Linux
capability dropped and `no-new-privileges` set (`deploy/compose.yaml`,
`deploy/brain.sh up`). It ships `git`, `gh` and `pulse`, and no coding
harness. The brain's own lead uses PULSE's built-in harness.

## Encryption at rest

The brain's volume is the one thing to encrypt, and the host encrypts it.
PULSE does not encrypt its store itself. Put the volume on a disk the
platform encrypts:

- on Azure, managed disks and Azure Files (encrypted with platform keys by
  default, or customer-managed keys in Key Vault);
- on AWS, encrypted EBS volumes;
- on a server you run yourself, LUKS or an encrypted ZFS dataset under
  Docker's volume directory.

Encryption covers the store, the work directory and every worktree the
brain makes (none with `brain.code none`).

## Backups

`backup.to` copies the store once a day ([runner.md](runner.md#backups-of-the-store)).
The image passes `PULSE_BACKUP_TO` through to it, for example
`az://ACCOUNT/CONTAINER/PREFIX`. Azure Storage encrypts blobs at rest on
its side. A backup is the store (`pulse.sqlite`), so it holds the board,
the jobs and their results, but no project checkout.

## Deleted on request

`deploy/brain.sh delete TEAM --yes` removes the team's container and its
volume: the store, the work directory and the keys. It prints where the
team's backups were going (`backup.to`), because copies there are the
operator's to remove. A deletion is finished only when those are gone too.

## Updates: releases, never a build on the brain

A hosted brain holds no PULSE source, so it cannot rebuild itself the way a
laptop does. `daemon.self_update release` makes the daemon read the release
document `update.latest_url` names once an hour. When a newer release
exists, it installs that release's binary in place of the running one, the
way `pulse update` does: download, SHA-256 check against the release's
`SHA256SUMS`, smoke run, swap, keep the old one as `pulse.prev`
([release.md](release.md)). The daemon and the hub then exit with code 3,
and the entrypoint starts both again on the new binary.

The test: `cargo test -p pulse-runner --lib
release_update::tests::self_update_release_mode_installs_the_release_and_never_runs_cargo`
serves a release folder, runs the release mode and checks that the newer
binary is installed while no cargo is ever run.

A binary swapped this way lasts as long as the container. A new image
(`deploy/brain.sh build VERSION`) is the durable update. Until the release
workflow publishes a Linux build, a hosted brain finds no archive for its
target, says so, and keeps the binary its image carries.

## The console

The hub binds to loopback only. Until `hub.allow_remote` exists (ADR-070
step 5), a hosted brain runs on the host's network
(`network_mode: host`). Its console is on the host's
`127.0.0.1:PULSE_HUB_PORT`, reached over an SSH tunnel:

```
ssh -L 8765:127.0.0.1:8765 brain-host    # then open http://127.0.0.1:8765
```

Writes keep the same-origin guard they have on a laptop.

## Not yet in place (later steps of ADR-070)

These are part of the ADR-070 promise but are not built yet. Do not rely on
them until their step lands and this page says so:

- **Hands that dial in to a hosted brain** (step 2: join codes, device keys,
  the dial-out channel). Today a brain reaches its hands over SSH.
- **GitHub App installation tokens** (step 3) instead of a member's own
  token on the brain.
- **The team secret vault** (step 4).
- **Passkey-signed writes and `hub.allow_remote`** (step 5).
- **Landing without a checkout.** The lander merges in a worktree of the
  project's registered checkout. A brain with `brain.code none` has none,
  so landing there waits on merging through the forge.
