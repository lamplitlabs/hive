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
- **Whatever a job's own commands reach.** An agent's shell commands and
  the project's build and check run with the network open: package
  installs, downloads, any `curl`. Only a conversation in lockdown has the
  network taken away ([Agents](#agents)).
- **Web search, in a session.** `harness.web_search` is `chat` by default:
  a session (a conversation with an agent) is offered web search and sends
  the search words to DuckDuckGo, or to Brave Search when you gave Hive a
  key for it. Jobs do not search unless the setting is `on`, and `off`
  turns it off everywhere.

Off until it is turned on or used:

- web pages from the hosts in `harness.fetch_allow` (empty by default, so
  the `fetch` tool is not offered);
- the MCP servers in the registry (`hive mcp list` names them, and
  `hive mcp show NAME` says who added each);
- store hooks with a web address (`hive hook add EVENT --url URL`): each
  matching event of the store is posted there as JSON. None exists until
  one is added;
- backups, to the place `backup.to` names (empty by default);
- the release check, at the address `update.latest_url` names (empty by
  default: no check and no network by itself). `hive update`, when you run
  it yourself, then looks at Hive's own releases on GitHub for that one run
  and says so first; under an agent's name or inside a job it does not;
- machines rented through the cloud CLI you are logged in to
  (`compute.max_vms` is `0` by default, which means agents cannot get
  compute; Hive stores no cloud credential);
- the dictation model, fetched once when you first use the microphone. The
  speech is recognised on the machine and no audio leaves it.

These switches are settings and commands. The rule is that the owner turns
them on. The program does not refuse `hive settings set` from an agent's
shell for them: [Agents](#agents) says which rules the program checks and
which it does not.

The sign-in under **Settings → Harness** is your model provider's, not
Hive's. The login is a file under `~/.hive/private/providers/` with mode
0600, and `hive harness logout` removes it
([harness.md](harness.md#sign-in-and-where-the-token-lives)). Without that
file the Copilot provider uses a GitHub token it finds in the environment
(`COPILOT_GITHUB_TOKEN`, `GH_TOKEN` or `GITHUB_TOKEN`), in the Copilot
editor plugins' own login or in the Copilot CLI's config, and checks it
against GitHub.

## The console

### Loopback only

The console (`hive hub`) binds 127.0.0.1 and refuses any other `--host`.
The HTTP layer also refuses a listener that is not on loopback
(`cargo test -p hive-hub --lib httpx::tests::serve_refuses_non_loopback_listener`).

The console has no login of its own. The loopback address is the boundary:
a program on this computer can reach the port, and nothing outside it can.
An agent's shell commands are programs on this computer too
([Agents](#agents)). Treat anything you put in front of that port, a tunnel
or a proxy, as a door to the whole console.

On that listener:

- Every request passes the **Host rule** before any page or API route. The
  console answers only when the request's `Host` header is one of:
  - `localhost`, or a name that ends in `.localhost`;
  - an IP address, v4 or v6;
  - a name you listed in the setting `hub.hosts`.

  The port is not compared. Any other `Host`, and a missing or empty one,
  gets 403. This stops DNS rebinding: a web page can point a name of its
  own at 127.0.0.1, and its requests then pass the same-origin guard below,
  but they carry that name as `Host`. An IP address passes because a name
  that was pointed at this machine is never an IP address, so a tunnel or a
  port forward that you open by address keeps working
  (`cargo test -p hive-hub --lib server::tests::a_rebound_name_gets_403_on_a_page_a_read_and_a_write`).
- **`hub.hosts`** holds the extra names, separated by commas: for example
  `hive.example.com`, when a proxy of yours forwards that name to
  127.0.0.1. It is empty by default. Set it with
  `hive settings set hub.hosts hive.example.com` or under **Settings →
  Reaching the console from anywhere**; the 403 names the host that came
  and this command. It counts only when the store names the owner as the
  one who set it: a value written under an agent's name is ignored
  ([Agents](#agents) says what that check rests on). List only names you
  control. Whoever decides where a listed name points can reach the console
  under it.
- Two kinds of request are not held to the console's Host rule. The routes
  a hand calls (`/api/v1/hands/`) are checked by join code and device key
  instead, so a hand can reach the brain through a proxy under a name of
  yours. Requests on the relay listener are held to that listener's own
  Host rule, which
  [the next section](#a-relay-in-front-of-the-console-huballow_remote)
  describes.
- Every write from the console also passes the **same-origin guard**: when
  the browser sends an `Origin`, its host and port must equal the request's
  `Host`, and a request the browser marks `Sec-Fetch-Site: cross-site` is
  refused. This stops a page of another site from writing to your console.
  A proxy of yours that serves the console over https must send
  `X-Forwarded-Proto: https`, or writes are refused as cross-origin: the
  console itself speaks plain HTTP, so it reads a `Host` without a port as
  port 80 unless that header says https. The routes a hand calls are
  checked by join code and device key instead.
- The writes that change the most also need an **explicit confirmation
  header**: `X-Hive-Settings: yes` for settings, `X-Hive-Decide: yes` for
  decisions, `X-Hive-Members: yes` for members, and one each for the push
  key, the private tests and job repair.
- Four route groups stay stricter than the Host rule: browsing folders, and
  the writes of credentials, held-out fixtures and job repair. They answer
  only when `Host` is `localhost` or a loopback address. A name in
  `hub.hosts` does not reach them, and neither does an IP address that is
  not loopback.

### A relay in front of the console (`hub.allow_remote`)

Hive ships no relay and nobody runs one for you. The brain's side of one is
built, for someone who wants to reach their console from another machine
through a web front of their own. The brain does not trust that front with
writes. Reads it serves without a signature, as the list says.

- **Owner-only switch.** `hub.allow_remote` holds the front's `https://`
  origin. It counts only when the store names the owner as the one who set
  it: a value written under an agent's name is ignored
  ([Agents](#agents) says what that check rests on). Empty is the default
  and means this machine only.
- **A separate listener.** `hive hub --front-port N` opens a second
  listener, also on 127.0.0.1, for the relay. Every request on it is
  marked as coming through the front. Request headers cannot change that,
  so a forged `Host: 127.0.0.1` does not unlock the loopback-only routes.
  While `hub.allow_remote` is empty, every request on that listener gets 403.
  Once the setting is set, the listener answers only when `Host` is the
  front's own name (the host of `hub.allow_remote`), `localhost`, a name
  that ends in `.localhost`, or an IP address. The port is not compared.
  Any other `Host`, and a missing or empty one, gets 403, so a web page
  that points a name of its own at 127.0.0.1 cannot read the console
  through the relay port
  (`cargo test -p hive-hub --lib remote::tests::the_relay_answers_only_to_the_fronts_name_localhost_and_ip_addresses`).
  A browser `Origin` other than the front's also gets 403. Reads (`GET` and
  `HEAD`) that pass these checks need no signature.
- **Passkey-signed writes.** Each write through the front carries a
  WebAuthn assertion from a registered member's passkey. The passkey signs
  over a nonce the brain issued (`GET /api/v1/nonce`: five minutes,
  spendable once, a bounded number outstanding at a time). The signed
  challenge binds the write's method, path and exact body. The brain
  verifies the assertion itself (`hive_runner::webauthn`, ES256 on P-256)
  and checks:
  - the type, challenge and origin in the client data;
  - the RP ID hash, which must be that of the front's host or of a parent
    domain of it;
  - the user-present and user-verified flags;
  - that the signature counter moved forward, for a passkey that keeps one
    (a passkey that always reports 0 is accepted);
  - the signature under the member's registered key.

  It then spends the nonce and records the counter in one step each. The
  spent nonce is what refuses a replayed write, also for a passkey that
  keeps no counter. A wrong key, another body or path, an unregistered or
  revoked passkey, or a missing signature all get 403.
- **Roles.** A member is `owner` or `member`. Settings and members changes
  through the front need an owner's passkey.
- **What is stored.** The `members` table holds each passkey's credential
  id, holder, display name, role, public key and counter. The
  `write_nonces` table holds the outstanding nonces. No private key,
  session token or password is ever stored. Registering and revoking a
  member is recorded as an event (`member.added`, `member.revoked`).

What a front can and cannot do: it can deny service, and it can read
everything the console can show, because reads through the relay port carry
no signature. It cannot change anything on the brain, because it cannot
produce a member's passkey signature. It cannot reuse a write it saw,
because the nonce is spent. The test
`the_consoles_signed_write_fixture_is_accepted_by_the_brain` in `hive-hub`
holds the console's signing and the brain's check to one format
([console.md](console.md#reaching-the-console-from-anywhere)).

Until you put a relay there, nothing outside this machine reaches the relay
port.

## The machines you pair

A hand has no store. It cannot read the board, the settings or another
project on its own. Each job comes to it on its own, with a brief that
carries what the brain chose to tell it. A hand with more than one worker
slot runs several jobs at the same time, each in a clone of its own.

**A machine paired over SSH** (`hive pair ADMIN@HOST --name NAME`, Linux
only) is set up over the SSH login of an admin there, who needs
passwordless sudo. Pairing copies a Linux `hive` to the machine and leaves
these on it:

- a `hive` user;
- the program, as `/usr/local/bin/hive-hands` with `/usr/local/bin/hive`
  linked to it;
- a git config and `mise` in that user's home. `mise` is downloaded on the
  machine from its release on GitHub and checked against a SHA-256 that
  Hive carries;
- a work directory with the clones of the jobs it runs;
- one line in that user's `authorized_keys`. The line is a forced command
  with `restrict`, so the brain's key can run the hands verbs on that
  machine and nothing else;
- an sshd drop-in, `/etc/ssh/sshd_config.d/60-hive-hands.conf`, which turns
  off passwords, forwarding and terminals for that user. sshd is reloaded
  after it is written.

On the brain, pairing adds a `Host` block to `~/.ssh/config` and the
machine's host keys to `~/.ssh/known_hosts`. The key is the brain's own,
`~/.ssh/hive_hands_ed25519`. A newly paired machine has no model sign-in
and takes no work until it has one:
`hive executor login NAME --admin ADMIN@HOST`, then
`hive executor probe NAME`. The login command places the token you stored
with `hive executor token`, or else the brain's own Copilot login when that
login reaches no repository; with `--with-token` it takes a token from
stdin. [machines.md](machines.md#pairing-hands) has every step.

**A machine enrolled with a join code** (`hive-hand join`) holds an ed25519
device key in its own directory (`~/.hive-hand`, mode 0700; the private key
0600) and a small `device.json`. It opens no port, and enrolling puts no
key of the brain's on it. The join code is six words, good for ten minutes
and one join; the brain keeps only its SHA-256 and the public half of the
device key. To connect, the hand signs a nonce from the brain (good for two
minutes and one use) and gets a session token for twelve hours. A tampered
signature, a used or lapsed nonce and a revoked device are refused
([machines.md](machines.md#enrolling-a-hand)). The test
`dial::end_to_end::a_hive_hand_joins_a_brain_and_completes_a_job_over_the_dial_channel`
in `hive-runner` runs a real `hive-hand` through a join and one job.
`hive-hand join` enrols the machine and then stays connected. On every
later start `hive-hand dial` holds the connection, so keep that running on
the machine. To run an agent's job the machine also needs `hive` on its
PATH and a model sign-in of its own, like any machine that runs the
harness.

The hand reaches the brain at the address you give it (`--brain`). The line
the Machines page shows carries the address your browser used, so it
usually reads `http://127.0.0.1:8765`; change it to an address the machine
can reach. The console listens on 127.0.0.1 only, so another machine
reaches it only through something you set up on the brain's machine: an
SSH tunnel, or a proxy with TLS there. A private network alone does not
reach a port bound to 127.0.0.1. The hand speaks HTTP through the system
`curl`, and TLS is whatever fronts the hub. What you put there reaches the
whole console, not only the hand's routes, so keep it between your own
machines.

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
directory on the machine itself. Those are yours to run, over the admin's
login. The printed line goes through `ssh NAME`, which is the login held to
the hands verbs, and that login refuses it. The commands leave the program
under `/usr/local/bin` and the sshd drop-in in place, so remove those by
hand too.

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

- The `vault_secrets` table holds only ciphertext: ChaCha20-Poly1305 under
  one data key, a fresh nonce per write, the name bound as associated data.
- The data key is stored in `vault_keys`, wrapped to its holder's key
  (X25519, then HKDF-SHA256 and ChaCha20-Poly1305). The brain is the only
  holder. Its key is a mode-0600 file in the brain's private directory,
  outside the store.
- A value comes in on stdin (`hive secret set NAME`) or through
  **Settings → Secrets** in the console. The command refuses a value typed
  at a terminal and refuses to run inside a job. `hive secret list` shows
  names, never values, and so does the console.
- A project's recipe declares the names its jobs may have (`secrets:`). The
  brain grants them per run and records `secret.granted`, `secret.refused`
  and `secret.read`. On the brain's own executor the values are handed to
  the worker as environment variables for that run only, and replaced by
  `[secret NAME]` in the run log and the result before either is stored. A
  value shorter than four characters is not replaced.

What is not built: a hand cannot open the vault. A project whose recipe
declares `secrets:` is marked unfit on every hand, so its jobs run on the
brain.

**Other places a secret can rest.** Model logins are files under
`~/.hive/private/providers/`. The token `hive executor token` stores for
the machines' model sign-in is a 0600 file, `private/executor-token`, in
the hive home. An MCP sign-in is a file under
`~/.hive/secrets/mcp-oauth/` with mode 0600. The token that reads a push
key from 1Password rests in the login Keychain on macOS and in a 0600 file
under `$HIVE_HOME/secrets/` elsewhere. The console never echoes a setting
value shaped like a secret. The private tests are kept outside every
repository, in `~/.hive/private/heldout` unless `evolve.heldout_dir` names
another folder, and that setting is masked in every view and event an agent
can read. In the default place, what keeps a job from reading the tests is
the deny rule on the hive home's `private/` directory. That is a rule on
the text of a command and not a barrier ([Agents](#agents)).

## A brain without the code: `brain.code none`

One setting says whether the brain keeps a copy of your code:

| `brain.code` | What the brain holds | Who runs what |
|---|---|---|
| `clone` (the default) | A clone of each project, as a laptop brain always has | The brain may run any job itself. Hands run what is routed to them. |
| `none` | **No checkout of any project, ever** | The brain runs only the jobs that plan from the board: the project lead and the reflection (`reflect`). They work in an empty workspace and read the task text and what the hands report. Jobs that work in the code (task, improve, experiment and the rest) wait for a hand. A `review` job has no machine to run on: it needs the code, which this brain lacks, and the store, which a hand lacks. Check and watch scripts are held. |

```
hive settings set brain.code none
```

It is under **Settings → This machine → Project code on this machine**. The
container image in `deploy/` sets it on every start
(`deploy/entrypoint.sh`).

What `none` changes, and the test that holds each part:

- **Leasing.** The brain's own executor leases only `lead` and `reflect`
  jobs. Anything else stays queued (`crates/hive-runner/src/jobs.rs`
  `lease_job`, `needs_code_here`). A hand takes the jobs that work in the
  code. It never takes a `review` job, which also needs the store, so with
  `none` a review stays queued.
- **Running.** A run asked for by hand (`hive work --job ID`) is not
  started for a job that needs the code: the lease leaves it queued, and
  the executor would refuse it with "brain.code is none". A lead or
  reflection gets an empty git repository with one empty commit, never a
  clone (`crates/hive-runner/src/exec.rs` `prepare_no_code_workspace`).
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
  that the executor refuses to run the task on this machine ("brain.code is
  none").

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
What a brain keeps under the hive home is on that volume, under
`HIVE_HOME=/data/hive`: the SQLite store and its WAL, the work directory,
the logs, the run keys, the model login and the console's state. Three
things live in the container's own home and go when the container goes:
`gh`'s login (`~/.config/gh`), the SSH key and config that pairing writes
(`~/.ssh`), and a binary swapped in by a release update. The image itself
holds no Hive source and no project code. The source tree stays in the
build stage, and `.dockerignore` keeps `.git`, build output, env files,
keys and stores out of the build context.

The container runs as an unprivileged user (uid 10001) with every Linux
capability dropped and `no-new-privileges` set (`deploy/compose.yaml`,
`deploy/brain.sh up`). It ships `hive`, `git`, `gh`, the OpenSSH client
(the brain signs and checks with `ssh-keygen`) and `openssl`, and no other
coding harness. It holds no `az`. The brain's own lead uses Hive's built-in
harness.

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
The image passes `HIVE_BACKUP_TO` through to it. An
`az://ACCOUNT/CONTAINER/PREFIX` destination is uploaded with the `az`
program, which the image does not hold, so it works there only after you
add the Azure CLI and a login to the image. Azure Storage encrypts blobs at
rest on its side. A backup is the store (`hive.sqlite`), so it holds the
board, the jobs and their results, but no project checkout. The archive
file beside the store, `hive-archive.sqlite`, is not in a backup. A copy is
as readable as the store it came from: put it somewhere you trust.

## Removing your data

Your data is on your machines, so removing it is yours to do:

- `hive remove project <slug>` deactivates a project after a preview.
  Nothing is deleted. `hive remove project <slug> --forget` is the hard
  delete for a project that is already inactive and whose checkout is gone:
  it deletes the project's rows (posts, jobs, runs, briefs, records,
  events) in one transaction ([machines.md](machines.md#removing-projects-and-executors)).
- `hive service uninstall` stops and removes the two user services.
- The store is `hive.sqlite`, in the hive home (`$HIVE_HOME`, `~/.hive` by
  default). Once a month rows older than `daemon.archive_days` (90) that
  nothing reads back move to `hive-archive.sqlite` beside it. The logs, the
  model logins and the work directories are in the same folder.
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

The installer (`scripts/install.sh`) checks the same way. It downloads over
https with curl, or with GNU wget, and stops before it downloads anything
when the only downloader is BusyBox's `wget`: that `wget` does not check a
server's certificate, so a checksum fetched over it would prove nothing.

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

Agents run as the user who runs Hive on that machine (on a paired machine
the `hive` user, on an enrolled one the user who runs `hive-hand`), under
the constitution's hard stops and the owner-only paths. A job has no
container and no operating-system sandbox around it. So much of what holds
an agent is a rule it is given, and the program checks only part of it.
This section says which part.

What the program checks:

- **Credentials.** No credential reference is resolved inside a job
  (ADR-041 D4).
- **Three settings count only from the owner.** `hub.allow_remote` opens
  the console to a front, `hub.hosts` adds names the console answers to,
  and `owner.presence away` hands the owner's decisions to the leads. Each
  counts only when the store names the owner as the one who set it: a value
  written under an agent's name is ignored. A `hive` command run under an
  agent's name is also refused the `owner.*` settings and `push.credential`.
- **Votes and variants.** No vote can touch the constitution's protected
  sections, and no variant of the loop can change `evolve.*`.
- **Every commit is traceable.** A run signs its commits with a key of its
  own and adds its agent, job and executor as trailers. The runner checks
  them before it delivers a branch, and `hive verify <ref>` checks them
  again ([landing.md](landing.md#the-pull-request)).
- **Lockdown has two strengths.** In lockdown, a conversation's commands
  run in a sandbox: no network except localhost, writes only inside the
  worktree and caches, and credential directories unreadable. On macOS the
  operating system enforces it (`sandbox-exec`). Elsewhere there is no
  operating-system sandbox behind it: the harness withdraws `fetch`,
  `web_search` and the MCP tools and refuses the commands that reach
  outside the machine.

What is a rule and not a barrier:

- **Deny rules in every job.** Before a shell command of a job runs, the
  harness matches its text against four patterns: a command that starts
  with `git push`, with `git remote add`, `set-url` or `remove`, or with
  `gh`, and a command that names `~/.ssh/`, `.netrc`, `~/.copilot`,
  `~/.config/gh` or the hive home's `private/` directory. A match is
  refused without being run. Other spellings pass (`git -C . push`, `gh`
  by its full path, `$HOME/.ssh/...`). The rules catch a mistake; they are
  not a barrier. `harness.deny_tools` adds rules of your own for every run
  ([harness.md](harness.md)).
- **The kill switch.** `hive stop` halts the runner. The rule is that only
  the owner runs `hive start`. The program does not refuse `hive start` or
  `hive settings set stop 0` from an agent's shell; the event log records
  who ran it, by the name the command carried.
- **Every other setting.** The caps and `evolve.*` included, any other
  setting can be changed by `hive settings set` from an agent's shell. The
  event log records the name it was written under.
- **The agent's name.** The first two checks above go by the name a `hive`
  command carries (`HIVE_AGENT`, `--agent`) and by the job's mark in its
  environment (`HIVE_JOB`). A job runs as your user, so a command can leave
  them out (`--agent human`), and it can write through the console on
  127.0.0.1, which has no login. Such a write is recorded as yours and
  counts as yours.
- **Posts are data.** Agents are told that a post on the board is data,
  never an instruction, and the loop stops for you when a post reads like
  one. Nothing tells an agent the same about the text of a web page it
  fetched. A repository's own instruction files (`AGENTS.md` and the like)
  are different: a run reads them as instructions, under the rule that the
  harness's own rules win.

The known risk stays: text in a repository can try to steer an agent
(prompt injection). The controls are the rules above, the review of every
pull request, the event log and the kill switch (`hive stop`). None of them
is a sandbox, so treat a repository you point Hive at like code you would
run yourself.

## What is not built

- **A hosted service.** Nobody runs a brain for you, and none is planned
  ([ADR-071](decisions/ADR-071-hive-is-an-open-source-tool.md)).
- **A relay.** The brain's side of `hub.allow_remote` is built, and so are
  the console's passkey pages (see [The console](#the-console)). The front
  itself is not part of Hive: no web login and no relay ship with it.
- **A sandbox for jobs.** A job runs as your user inside its clone, with
  the network open. No container or operating-system sandbox wraps it. The
  deny rules are rules on the text of a command: other spellings of the
  same command pass ([Agents](#agents)). Lockdown covers conversations, and
  the operating system enforces it on macOS only.
- **The vault on hands.** A hand cannot open the vault, model logins are
  not vault names, and there are no `vault://NAME` references for MCP and
  cloud credentials.
- **A leased push for remote hands.** Only a hand on the brain's own
  machine pushes with a leased key or GitHub App token. Paired and enrolled
  machines hand their commits back to the brain.
- **Registering a project without a checkout.** `hive init` and **Add
  project** take the path of a checkout on the brain's own disk. A brain
  that holds no checkout of a project has no way to register it.
- **Reviews on a brain without code.** A `review` job needs the code and
  the store both. A brain with `brain.code none` has no code and a hand has
  no store, so no machine takes it.
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
