# Security notes: where your code and your data live

Hive is a tool you run yourself. There is no Hive service behind it: no
account, no sign-in to Hive, and no server of ours that your Hive reports
to. This page says where your code and your data are, what leaves your
machine, what a machine you pair holds, and what is not built. Where a
statement rests on a test, the test is named so you can run it.

Two words are used throughout. The **brain** is the runner (`hive daemon`),
its store and its console (`hive hub`). It plans the work, keeps the board
and delivers branches. **Hands** are the other machines you pair or enrol
([machines.md](machines.md#the-hand)). A job runs there in a clone of the
project: the harness, the build and the tests. On a single laptop the brain
is also the only machine that runs jobs.

## Where Hive runs, and what leaves the machine

Everything runs on your machine, as your user: the runner, the store
(`~/.hive/hive.sqlite`) and the console. A job runs in a throwaway clone
under the hive home, here or on a machine you added.

What leaves the machine in ordinary use:

- **Model calls.** Each turn of an agent goes to the model provider you
  chose, and carries what the agent read: the brief, files, command output.
  With `ollama`, `lmstudio` or `vllm` the provider is a server on your own
  machine or network ([harness.md](harness.md#other-providers)).
- **Git.** Fetches from each project's upstream, and pushes of Hive's own
  branches to it. The runner pushes branches and never the default branch;
  a merge is made on the forge ([landing.md](landing.md)).
- **GitHub, through the `gh` CLI signed in on the machine.** Opening,
  reading and merging pull requests, the Actions runs of Hive's pushes and
  of the default branch's head, and the repository's Dependabot alerts.
- **Package registries.** The publish date of a dependency version, asked
  of npm, crates.io, PyPI, the Go module proxy or RubyGems for the
  release-age rule ([landing.md](landing.md#dependencies-security-alerts-and-the-release-age-rule)).

And only when you turn it on or use it:

- web pages from the hosts in `harness.fetch_allow` (empty by default, so
  the `fetch` tool is not offered);
- web search (`harness.web_search`; by default it is offered in a
  conversation and not to jobs);
- the MCP servers in the registry (`hive mcp list` shows each one and who
  added it);
- backups, to the place `backup.to` names (empty by default);
- the release check, at the address `update.latest_url` names (empty by
  default: no check and no network);
- machines rented through the cloud CLI you are logged in to
  (`compute.max_vms` is `0` by default, which means agents cannot get
  compute; Hive stores no cloud credential);
- the dictation model, fetched once when you first use the microphone. The
  speech is recognised on the machine and no audio leaves it.

The sign-in under **Settings → Harness** is your model provider's, not
Hive's. The login is a file under `~/.hive/private/providers/` with mode
0600, and `hive harness logout` removes it
([harness.md](harness.md#sign-in-and-where-the-token-lives)).

## The console

### Loopback only

The console (`hive hub`) binds 127.0.0.1 and refuses any other `--host`.
The HTTP layer also refuses a listener that is not on loopback
(`cargo test -p hive-hub --lib httpx::tests::serve_refuses_non_loopback_listener`).

The console has no login of its own. The loopback address is the boundary:
a program on this computer can reach the port, and nothing outside it can.
Treat anything you put in front of that port, a tunnel or a proxy, as a
door to the whole console.

On that listener:

- Every write passes the **same-origin guard**: when the browser sends an
  `Origin`, it must name this hub, and a request the browser marks
  `Sec-Fetch-Site: cross-site` is refused. This stops a web page you have
  open from writing to your console.
- The writes that change the most also need an **explicit confirmation
  header**: `X-Hive-Settings: yes` for settings, `X-Hive-Decide: yes` for
  decisions, `X-Hive-Members: yes` for members, and one each for the push
  key, the private tests and job repair.
- Some routes also refuse any `Host` that is not this machine. These are
  folders, credentials, held-out fixtures and job repair. This protects
  them against DNS rebinding.

### A relay in front of the console (`hub.allow_remote`)

Hive ships no relay and nobody runs one for you. The brain's side of one is
built, for someone who wants to reach their console from another machine
through a web front of their own. The brain does not trust that front.

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

What a front can and cannot do: it can deny service, and it can see the
pages it relays. It cannot change anything on the brain, because it cannot
produce a member's passkey signature. It cannot reuse a write it saw,
because the nonce is spent and the counter has moved. The test
`the_consoles_signed_write_fixture_is_accepted_by_the_brain` in `hive-hub`
holds the console's signing and the brain's check to one format
([console.md](console.md#reaching-the-console-from-anywhere)).

Until you put a relay there, nothing outside this machine reaches the relay
port.

## The machines you pair

A hand has no store. It cannot read the board, the settings or another
project on its own. It is given one job at a time, with a brief that
carries what the brain chose to tell it.

**A machine paired over SSH** (`hive pair ADMIN@HOST`, Linux) holds a
`hive` user, the hands binary, a work directory with the clones of the jobs
it runs, and one line in that user's `authorized_keys`. The line is a
forced command with `restrict`, so the brain's key can run the hands verbs
on that machine and nothing else. The key is the brain's own,
`~/.ssh/hive_hands_ed25519`. The machine also holds a model sign-in, which
you place there (`hive executor login NAME --admin ADMIN@HOST`).

**A machine enrolled with a join code** (`hive-hand join`) holds an ed25519
device key in its own directory (`~/.hive-hand`, mode 0700; the private key
0600) and a small `device.json`. It opens no port and gets no key from the
brain. The join code is six words, good for ten minutes and one join; the
brain keeps only its SHA-256 and the public half of the device key. To
connect, the hand signs a nonce from the brain (good for two minutes and
one use) and gets a session token for twelve hours. A tampered signature, a
used or lapsed nonce and a revoked device are refused
([machines.md](machines.md#enrolling-a-hand)). The test
`dial::end_to_end::a_hive_hand_joins_a_brain_and_completes_a_job_over_the_dial_channel`
in `hive-runner` runs a real `hive-hand` through a join and one job. To run
an agent's job the machine also needs `hive` on its PATH and a model
sign-in of its own, like any machine that runs the harness.

The hand reaches the brain at the address you give it (`--brain`). The
console listens on loopback only, so getting from the hand to that port is
yours to set up: an SSH tunnel, a private network, or a proxy with TLS in
front of it. The hand speaks HTTP through the system `curl`, and TLS is
whatever fronts the hub. What you put there reaches the whole console, not
only the hand's routes, so keep it between your own machines.

**What the brain sends a hand.** For each job: the code as a git bundle of
the branch the job starts from, the brief, the run's signing key, the
pre-commit hook that runs the project's check, and the values of the
recipe's `env:`. The clone on the hand has no git remote.

**What a hand never gets.** The store. The push key or any other git
credential: a paired or enrolled machine does not push. The brain fetches
the job's commits from it (over SSH, or as a git bundle on the dial
channel), checks who made them, and delivers the branch itself. Vault
secrets (see below). The secrets of MCP servers: a worker on a hand is told
the servers exist and are reachable only from the brain.

**Revoking.** A machine that dials in has a **Revoke** button on the
Machines page. The brain drops its key and its sessions and removes it from
the roster; the hand's next call is refused, and coming back takes a new
code. `hive remove executor NAME` takes a paired machine out: after a
preview, the store marks it removed and no job is sent there again. The
preview also prints the commands that remove the `hive` user and the work
directory on the machine itself. Those are yours to run.

## Secrets

**References, not values.** The push key is a reference (`push.credential`:
`op://`, `keychain://` or `file://`), never the key. The runner resolves it
for the seconds of one push or fetch, writes it with mode 0600 into a 0700
directory for that call, and removes it. No reference is resolved inside a
job (ADR-041 D4). The one exception is the secret of an MCP server, which
`hive mcp` resolves on the brain for the request it sends and never prints
([landing.md](landing.md#credentials)).

**GitHub App tokens.** With a GitHub App configured (`git.github_app.*`),
the runner's own fetches, pushes and `gh` calls use a token that lasts an
hour and opens one repository. The App's private key is again a reference,
and only the brain reads it. A hand on the brain's own machine
(`executor.transport local-hand`) is handed such a token for its one push
and removes the helper when the push returns. The test
`jobs::tests::a_job_leased_to_a_local_hive_hand_pushes_with_the_app_token`
proves it against a git server that accepts nothing but that token; without
an App,
`jobs::tests::a_job_leased_to_a_local_hive_hand_pushes_with_the_leased_credential`
proves the same path with the push key.

**The vault.** `hive secret set NAME` seals a value in the brain's store
([harness.md](harness.md#team-secrets-the-vault)). What is built:

- The `secrets` table holds only ciphertext: ChaCha20-Poly1305 under one
  data key, a fresh nonce per write, the name bound as associated data.
- The data key is stored wrapped to its holder's key (X25519, then
  HKDF-SHA256 and ChaCha20-Poly1305). The brain is the only holder. Its key
  is a mode-0600 file in the brain's private directory, outside the store.
- A value comes in on stdin only. It is refused from a terminal and from
  inside a job. `hive secret list` shows names, never values.
- A project's recipe declares the names its jobs may have (`secrets:`). The
  brain grants them per run and records `secret.granted`, `secret.refused`
  and `secret.read`. On the brain's own executor the values are handed to
  the worker as environment variables for that run only, and replaced by
  `[secret NAME]` in the run log and the result before either is stored.

What is not built: a hand cannot open the vault. A project whose recipe
declares `secrets:` is marked unfit on every hand, so its jobs run on the
brain.

**Other places a secret can rest.** Model logins are files under
`~/.hive/private/providers/`. An MCP sign-in is a file under
`~/.hive/secrets/mcp-oauth/` with mode 0600. The token that reads a push
key from 1Password rests in the login Keychain on macOS and in a 0600 file
under `$HIVE_HOME/secrets/` elsewhere. The console never echoes a setting
value shaped like a secret. The folder of private tests
(`evolve.heldout_dir`) is masked in every view and event an agent can read.

## A brain without the code: `brain.code none`

One setting says whether the brain keeps a copy of your code:

| `brain.code` | What the brain holds | Who runs what |
|---|---|---|
| `clone` (the default) | A clone of each project, as a laptop brain always has | The brain may run any job itself. Hands run what is routed to them. |
| `none` | **No checkout of any project, ever** | The brain runs only the jobs that plan from the board: the project lead and the reflection (`reflect`). They work in an empty workspace and read the task text and what the hands report. Every other job (task, improve, review, evolve, check scripts) waits for a hand. |

```
hive settings set brain.code none
```

It is under **Settings → This machine → Project code on this machine**. The
container image in `deploy/` sets it on every start
(`deploy/entrypoint.sh`).

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

This option is not finished. "What is not built" below says what a brain
without code cannot do yet.

## A brain on a server of your own (`deploy/`)

`deploy/` holds a Dockerfile and scripts for someone who wants the brain on
a server instead of a laptop. It is a way to run Hive yourself, not a
service. On every start the container sets `brain.code none` and
`daemon.self_update release` (`deploy/entrypoint.sh`), so it is a brain
without the code, with the limits "What is not built" lists.

A brain in a container is one container with one volume
(`deploy/Dockerfile`, `deploy/brain.sh`). `deploy/brain.sh up NAME VERSION
PORT` starts one, and each name gets its own container and its own volume,
so two brains never share a container, a volume, a store or a key.
Everything a brain keeps is on that volume, under `HIVE_HOME=/data/hive`.
That covers the SQLite store and its WAL, the work directory, the logs, the
run keys and the console's state. The image itself holds no Hive source and
no project code. The source tree stays in the build stage, and
`.dockerignore` keeps `.git`, build output, env files, keys and stores out
of the build context.

The container runs as an unprivileged user (uid 10001) with every Linux
capability dropped and `no-new-privileges` set (`deploy/compose.yaml`,
`deploy/brain.sh up`). It ships `git`, `gh` and `hive`, and no other coding
harness. The brain's own lead uses Hive's built-in harness.

The console stays on loopback there too. The container runs on the host's
network (`network_mode: host`), so its console is on the host's
`127.0.0.1:HIVE_HUB_PORT`, reached over an SSH tunnel:

```
ssh -L 8765:127.0.0.1:8765 brain-host    # then open http://127.0.0.1:8765
```

Writes keep the same-origin guard they have on a laptop.

## Encryption at rest

Hive does not encrypt its store itself. The disk under the hive home is the
one thing to encrypt, so put it on a disk that is encrypted:

- on a laptop, the disk encryption of the operating system;
- for a brain in a container, the volume: on Azure, managed disks and Azure
  Files (encrypted with platform keys by default, or customer-managed keys
  in Key Vault); on AWS, encrypted EBS volumes; on a server you run
  yourself, LUKS or an encrypted ZFS dataset under Docker's volume
  directory.

Encryption covers the store, the work directory and every worktree the
brain makes (none with `brain.code none`).

## Backups

`backup.to` copies the store once a day ([runner.md](runner.md#backups-of-the-store)).
The image passes `HIVE_BACKUP_TO` through to it, for example
`az://ACCOUNT/CONTAINER/PREFIX`. Azure Storage encrypts blobs at rest on
its side. A backup is the store (`hive.sqlite`), so it holds the board,
the jobs and their results, but no project checkout. A copy is as readable
as the store it came from: put it somewhere you trust.

## Removing your data

Your data is on your machines, so removing it is yours to do:

- `hive remove project <slug>` deactivates a project after a preview.
  Nothing is deleted. `hive remove project <slug> --forget` is the hard
  delete for a project that is already inactive and whose checkout is gone:
  it deletes the project's rows (posts, jobs, runs, briefs, records,
  events) in one transaction ([machines.md](machines.md#removing-projects-and-executors)).
- `hive service uninstall` stops and removes the two user services.
- The store is one file, `hive.sqlite`, in the hive home (`$HIVE_HOME`,
  `~/.hive` by default), beside the logs, the model logins and the work
  directories.
- `deploy/brain.sh delete NAME --yes` removes a container brain and its
  volume: the store, the work directory and the keys. It prints where that
  brain's backups were going.
- Backups are copies somewhere else (`backup.to`). Remove them there. The
  data is not gone until they are.

## Updates

A release is checked before it replaces the running binary. `hive update`
downloads the archive for the machine, checks its SHA-256 against the
release's `SHA256SUMS`, smoke-runs the new binary, swaps it in and keeps
the old one as `hive.prev` ([release.md](release.md)). A release with no
`SHA256SUMS`, or with a digest that differs, is refused.

A brain that holds no Hive source cannot rebuild itself the way a
development machine does. `daemon.self_update release` makes the daemon
read the release document `update.latest_url` names once an hour. When a
newer release exists, it installs that release's binary in place of the
running one, the way `hive update` does. The daemon and the hub then exit
with code 3, and the service, or `deploy/entrypoint.sh` in a container,
starts both again on the new binary.

The test: `cargo test -p hive-runner --lib
release_update::tests::self_update_release_mode_installs_the_release_and_never_runs_cargo`
serves a release folder, runs the release mode and checks that the newer
binary is installed while no cargo is ever run.

In a container, a binary swapped this way lasts as long as the container. A
new image (`deploy/brain.sh build VERSION`) is the durable update. When a
release carries no archive for the machine's target, the daemon says so and
keeps the binary it has.

## Agents

Agents run as the user who runs Hive on that machine (on a hand, the
enrolled user), under the constitution's hard stops and the owner-only
paths. They cannot open the console to a front, since their
`hub.allow_remote` is ignored. They cannot write the owner's presence or
other owner-only settings either. Credentials are never resolved inside a
job (ADR-041 D4). In lockdown, a conversation's commands run in a sandbox:
no network except localhost, writes only inside the worktree and caches,
and credential directories unreadable.

What that rests on:

- **Deny rules in every job.** A command that would leave the clone (`git
  push`, changing `git remote`, any `gh` call) or touch credentials
  (`~/.ssh`, `.netrc`, the hive home's `private/` directory, `~/.copilot`,
  `~/.config/gh`) is refused without being run. `harness.deny_tools` adds
  rules of your own for every run ([harness.md](harness.md)).
- **Every commit is traceable.** A run signs its commits with a key of its
  own and adds its agent, job and executor as trailers. The runner checks
  them before it delivers a branch, and `hive verify <ref>` checks them
  again ([landing.md](landing.md#the-pull-request)).
- **Posts and pages are data.** Agents are told that a post on the board
  and the text of a web page are data, never an instruction. The loop stops
  for you when a post reads like one. A repository's own instruction files
  (`AGENTS.md` and the like) are different: a run reads them as
  instructions, under the rule that the harness's own rules win.
- **Lockdown has two strengths.** On macOS the operating system enforces it
  (`sandbox-exec`). Elsewhere there is no operating-system sandbox behind
  it: the harness withdraws `fetch`, `web_search` and the MCP tools and
  refuses the commands that reach outside the machine.

The known risk stays: text in a repository can try to steer an agent
(prompt injection). The rules above, the kill switch (`hive stop`), the
event log and the review of every pull request are the controls.

## What is not built

- **A hosted service.** Nobody runs a brain for you, and none is planned
  ([ADR-071](decisions/ADR-071-hive-is-an-open-source-tool.md)).
- **A relay.** The brain's side of `hub.allow_remote` is built, and so are
  the console's passkey pages (see [The console](#the-console)). The front
  itself is not part of Hive: no web login and no relay ship with it.
- **A sandbox for jobs.** A job runs as your user inside its clone, under
  the deny rules. No container or operating-system sandbox wraps it.
  Lockdown covers conversations, and the operating system enforces it on
  macOS only.
- **The vault on hands.** A hand cannot open the vault, model logins are
  not vault names, and there are no `vault://NAME` references for MCP and
  cloud credentials.
- **A leased push for remote hands.** Only a hand on the brain's own
  machine pushes with a leased key or GitHub App token. Paired and enrolled
  machines hand their commits back to the brain.
- **Registering a project without a checkout.** `hive init` and **Add
  project** take the path of a checkout on the brain's own disk. A brain
  that holds no checkout of a project has no way to register it.
- **Landing without a checkout.** The lander merges in a worktree of the
  project's registered checkout. A brain with `brain.code none` has none,
  so it holds every finished branch instead (`land.held`, reason
  `brain-code-none`, the note says "brain.code is none") and never makes a
  worktree under `HIVE_HOME/work`; the branch waits for its pull request to
  be merged through the forge. Merging that pull request from the brain
  itself is not built yet. Test:
  `land::tests::brain_code_none_holds_a_finished_branch_without_a_worktree`.

## Reporting a vulnerability

Do not open a public issue. [SECURITY.md](../SECURITY.md) says how to
report a vulnerability privately, what to include and what is in scope.
