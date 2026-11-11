# Security notes: where your code and your data live

This page is the promise ADR-070 rule 6 makes to a team, and what each part
of it rests on. It names what is built and tested today, and what still
waits on a later step of ADR-070. Nothing here is a promise until its test
exists. The tests are named so you can run them.

## Where code runs is your choice, written down

A team's **brain** is the Hive daemon, its store and its console (the hub).
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
hive settings set brain.code none
```

The hosted image sets `brain.code none` on every start
(`deploy/entrypoint.sh`). On a laptop it is under **Settings → This machine
→ Project code on this machine**.

What `none` changes, and the test that holds each part:

- **Leasing.** The brain's own executor leases only `lead` and `reflect`
  jobs. Anything else stays queued for a hand (`crates/hive-runner/src/jobs.rs`
  `lease_job`, `needs_code_here`).
- **Running.** A run started by hand (`hive run JOB`) for a job that needs
  the code is refused with "brain.code is none". A lead or reflection gets
  an empty git repository with one empty commit, never a clone
  (`crates/hive-runner/src/exec.rs` `prepare_no_code_workspace`).
- **Schedules.** Check and watch scripts run in a worktree of the project,
  so on a brain without code they are held and the hold is said once
  (`crates/hive-runner/src/schedule_script.rs`).
- **Self-update.** The source self-update builds from the brain's own
  checkout, so with `none` it stands down. Use `daemon.self_update release`
  instead (see "Updates" below).
- **The test.** `cargo test -p hive-runner --lib
  daemon::tests::brain_code_none_leaves_no_project_checkout_after_a_pass`
  queues a task and a reflection on a brain with `brain.code none` and runs
  one full daemon pass. It checks three things: the reflection ran in a
  workspace with no project files, the task is still queued, and the
  brain's work directory holds no checkout of the project. It also checks
  that `hive run` of the task is refused.

## One brain per team

A hosted brain is one container with one volume (`deploy/Dockerfile`,
`deploy/brain.sh`). Two teams never share a container, a volume, a store or
a key. Everything a brain keeps is on that volume, under
`HIVE_HOME=/data/hive`. That covers the SQLite store and its WAL, the work
directory, the logs, the run keys and the console's state. The image
itself holds no Hive source and no project code. The source tree stays in
the build stage, and `.dockerignore` keeps `.git`, build output, env files,
keys and stores out of the build context.

The container runs as an unprivileged user (uid 10001) with every Linux
capability dropped and `no-new-privileges` set (`deploy/compose.yaml`,
`deploy/brain.sh up`). It ships `git`, `gh` and `hive`, and no coding
harness. The brain's own lead uses Hive's built-in harness.

## Encryption at rest

The brain's volume is the one thing to encrypt, and the host encrypts it.
Hive does not encrypt its store itself. Put the volume on a disk the
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
The image passes `HIVE_BACKUP_TO` through to it, for example
`az://ACCOUNT/CONTAINER/PREFIX`. Azure Storage encrypts blobs at rest on
its side. A backup is the store (`hive.sqlite`), so it holds the board,
the jobs and their results, but no project checkout.

## Deleted on request

`deploy/brain.sh delete TEAM --yes` removes the team's container and its
volume: the store, the work directory and the keys. It prints where the
team's backups were going (`backup.to`), because copies there are the
operator's to remove. A deletion is finished only when those are gone too.

## Updates: releases, never a build on the brain

A hosted brain holds no Hive source, so it cannot rebuild itself the way a
laptop does. `daemon.self_update release` makes the daemon read the release
document `update.latest_url` names once an hour. When a newer release
exists, it installs that release's binary in place of the running one, the
way `hive update` does: download, SHA-256 check against the release's
`SHA256SUMS`, smoke run, swap, keep the old one as `hive.prev`
([release.md](release.md)). The daemon and the hub then exit with code 3,
and the entrypoint starts both again on the new binary.

The test: `cargo test -p hive-runner --lib
release_update::tests::self_update_release_mode_installs_the_release_and_never_runs_cargo`
serves a release folder, runs the release mode and checks that the newer
binary is installed while no cargo is ever run.

A binary swapped this way lasts as long as the container. A new image
(`deploy/brain.sh build VERSION`) is the durable update. Until the release
workflow publishes a Linux build, a hosted brain finds no archive for its
target, says so, and keeps the binary its image carries.

## The console

### Loopback only

The console (`hive hub`) binds 127.0.0.1 and refuses any other `--host`.
The HTTP layer also refuses a listener that is not on loopback.

On that listener, every write must pass two checks:

- The **same-origin guard**: the browser's `Origin` must name this hub, and
  a request the browser marks `Sec-Fetch-Site: cross-site` is refused.
- An **explicit confirmation header**, for example `X-Hive-Settings: yes`,
  `X-Hive-Decide: yes` or `X-Hive-Members: yes`.

Some routes also refuse any `Host` that is not this machine. These are
folders, credentials, held-out fixtures and job repair. This protects them
against DNS rebinding.

### Reaching the console through the front (`hub.allow_remote`)

Other machines reach the console only through the team's hosted front. The
front signs members in on the web and relays bytes to the brain. The brain
does not trust it.

- **Owner-only switch.** `hub.allow_remote` holds the front's `https://`
  origin. It counts only when the owner set it. A value an agent wrote is
  ignored. Empty is the default and means this machine only.
- **A separate listener.** `hive hub --front-port N` opens a second
  listener, also on 127.0.0.1, for the relay. Every request on it is
  marked as coming through the front. Request headers cannot change that,
  so a forged `Host: 127.0.0.1` does not unlock the loopback-only routes.
  While `hub.allow_remote` is empty, every request on that listener gets 403.
  A browser `Origin` other than the front's also gets 403.
- **Passkey-signed writes.** Each write through the front carries a
  WebAuthn assertion from a registered member's passkey. The passkey signs
  over a nonce the brain issued (`GET /api/v1/nonce`: five minutes,
  spendable once, a bounded number outstanding at a time). The signed
  challenge binds the write's method, path and exact body. The brain
  verifies the assertion itself (`hive_runner::webauthn`, ES256 on P-256)
  and checks:
  - the type, challenge and origin in the client data;
  - the RP ID hash of the front's host;
  - the user-present and user-verified flags;
  - that the signature counter moved forward;
  - the signature under the member's registered key.

  It then spends the nonce and records the counter in one step each, so a
  replayed write is refused. A wrong key, another body or path, an
  unregistered or revoked passkey, or a missing signature all get 403.
- **Roles.** A member is `owner` or `member`. Settings and members changes
  through the front need an owner's passkey.
- **What is stored.** The `members` table holds each passkey's credential
  id, holder, display name, role, public key and counter. The
  `write_nonces` table holds the outstanding nonces. No private key,
  session token or password is ever stored. Registering and revoking a
  member is recorded as an event (`member.added`, `member.revoked`).

What the front can and cannot do: it can deny service, and it can see the
pages it relays. It cannot change anything on the brain, because it cannot
produce a member's passkey signature. It cannot reuse a write it saw,
because the nonce is spent and the counter has moved.

### A hosted brain before the front exists

Until the hosted front is built, a hosted brain runs on the host's network
(`network_mode: host`). Its console is on the host's
`127.0.0.1:HIVE_HUB_PORT`, reached over an SSH tunnel:

```
ssh -L 8765:127.0.0.1:8765 brain-host    # then open http://127.0.0.1:8765
```

Writes keep the same-origin guard they have on a laptop.

### Agents

Agents run as the enrolled user, under the constitution's hard stops and
rule 8's owner-only paths. They cannot open the console to the front, since
their `hub.allow_remote` is ignored. They cannot write the owner's
presence or other owner-only settings either. Credentials are never resolved
inside a job (ADR-041 D4). In lockdown, a conversation's commands run in a
sandbox: no network except localhost, writes only inside the worktree and
caches, and credential directories unreadable.

## Not yet in place (later steps of ADR-070)

These are part of the ADR-070 promise but are not built yet. Do not rely on
them until their step lands and this page says so:

- **Hands that dial in to a hosted brain** (step 2: join codes, device keys,
  the dial-out channel). Today a brain reaches its hands over SSH.
- **GitHub App installation tokens** (step 3) instead of a member's own
  token on the brain.
- **The team secret vault** (step 4).
- **The hosted front** (the rest of step 5). The brain side - nonces,
  passkey-signed writes the brain verifies, the `members` table and
  `hub.allow_remote` - is built (see [The console](#the-console)). Still to
  build, and waiting on choices only the owner can make (the domain, the
  hosting and the Clerk account): the front itself, with its web login
  (Clerk) and team = organization; syncing members into the `members`
  table from the organization; passkey registration in the browser; the
  device enrolment relay; serving the console at `https://<team>.<domain>`.
  Until then, nothing outside this machine reaches the relay port. See
  [Reaching the console from anywhere](console.md#reaching-the-console-from-anywhere).
- **Landing without a checkout.** The lander merges in a worktree of the
  project's registered checkout. A brain with `brain.code none` has none,
  so it holds every finished branch instead (`land.held`, reason
  `brain-code-none`, the note says "brain.code is none") and never makes a
  worktree under `HIVE_HOME/work`; the branch waits for its pull request to
  be merged through the forge. Merging that pull request from the brain
  itself is not built yet. Test:
  `land::tests::brain_code_none_holds_a_finished_branch_without_a_worktree`.
