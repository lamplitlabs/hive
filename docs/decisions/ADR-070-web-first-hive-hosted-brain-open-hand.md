---
id: ADR-070
title: Web-first Hive - a hosted brain per team, an open hand enrolled once per machine, one web login that every machine inherits
status: Proposed
date: 2026-11-09
areas:
  - runner
  - hub
  - harness
  - machines
  - security
  - docs
tags:
  - adr
  - web-first
  - onboarding
  - hands
  - secrets
  - teams
  - product
amends:
  - ADR-019 (one SQLite store per machine, agents work on the owner's machine)
  - ADR-020 (hands join the brain over inbound SSH; credentials stay on the brain)
  - ADR-021 (one command pairs a machine, hub-initiated; the console is a loopback window; "never federated")
  - ADR-041 / ADR-042 (the push credential is a reference resolved on the brain from 1Password, the keychain or a file)
delivery: not started
delivered: null
---

# ADR-070: Web-first Hive - a hosted brain per team, an open hand enrolled once per machine, one web login that every machine inherits

> Superseded in part by [ADR-071](ADR-071-hive-is-an-open-source-tool.md) (proposed): the hosted product is withdrawn, and what this ADR built stays as options for someone who runs Hive themselves.

## Context

Hive today is a thing one person installs on one Mac. The README's first
step is a private git clone and `cargo install --path crates/hive --root
~/.hive --locked` (README.md:83-88); the hub binds `127.0.0.1` only and the
reserved `hub.allow_remote` is refused with "not supported yet"
(crates/hive-hub/src/register.rs:191-230). A second machine joins by `hive
pair ADMIN@HOST` *from the brain*, over inbound SSH, Linux only, writing an
`authorized_keys` line, an sshd drop-in and `/usr/local/bin/hive-hands`
(docs/machines.md:116-152; crates/hive-runner/src/pair.rs:13-14); its
harness login is a GitHub token the brain pushes over that SSH session
(`hive executor login NAME --admin ADMIN@HOST --with-token`,
crates/hive-runner/src/executor.rs:1179-1182) and keeps on disk there.
Secrets are references - `op://`, `keychain://`, `file://`
(crates/hive-cred/src/reference.rs:19) - resolved only on the brain and
refused inside any job (`HIVE_JOB`, crates/hive-cred/src/resolve.rs:113,
ADR-041 D4), so a hand can hold no secret at all. The brain self-updates by
rebuilding from the git clone on its own disk
(crates/hive-runner/src/selfupdate.rs:748), so a brain needs the source.

The owner now wants Hive to be a product for individual developers and
small firms, with three properties the current shape cannot give:

1. **Web-first onboarding.** Sign in on the web; log in to git there once;
   every machine of the account inherits it. A machine - the owner's Mac, a
   colleague's laptop, a rented box - is touched once and then "just works"
   as a remote harness. No PATs pasted per machine, no inbound SSH.
2. **Secrets shared well.** A team's API keys and tokens reach the job that
   needs them, on whichever machine runs it, without living on that machine.
3. **The code of Hive itself protected.** The lead, the planner, evolve and
   tally, the prompts and briefs, the hub: these are the product. A Rust
   binary on a customer's machine gives them away (`strings` on the binary
   lists every prompt), and the self-update path would put the whole source
   there. Nothing of the brain may be installed at a customer.

And one goal above the three: agents of one team must talk to each other
so they do not conflict, and people must be able to join forces on one
thing. Hive already has the mechanism - one board, advisory claims,
global `@name` mentions, one landing per project, every branch a reviewed
pull request (ADR-069) - but only for the agents of *one machine*.

Three shapes were weighed on the owner's three criteria - cheapest, most
innovative, best for scaling - and then on code protection, which decided
it.

## Options Considered

### Option 1: keep the brain on the owner's machine; add a hosted login and relay

The store, daemon and hub stay where they are; a thin hosted service gives a
web login, a relay to reach the hub from any browser, and device enrolment.

- Pros: smallest change; the store never leaves the owner.
- Cons: git auth and secrets stay per machine, the exact pain named above;
  the brain must stay on; and the whole product - prompts, planner, hub -
  is installed on every customer's machine. Dead end for a product.

### Option 2: a multi-tenant hosted control plane, rebuilt for the web

One hosted database for all customers, a web app, and agents that pull work.

- Pros: works with every customer machine off; nothing of the product ships.
- Cons: a rewrite of the store, board and runner for multi-tenancy; a
  noisy-neighbour and data-isolation problem designed from scratch; the
  fleet would stall for months while it is built. Most expensive and
  slowest.

### Option 3: the brain hosted as it is, one per team; a small open hand on every machine (chosen)

The `hive daemon` runs unchanged as a hosted container per team with its own
SQLite store - the same binary this repository builds today, deployed by
the owner rather than installed by the customer. A new, small, open-source
**hand** is the only thing a customer installs: it enrols once, dials out,
pulls jobs, runs the coding harness in a worktree, pushes a branch, reports.
A thin hosted **front** does login, device enrolment, GitHub App tokens,
the secret vault and the relay to each team's brain.

- Pros: cheapest path that protects the code (the brain is never
  downloaded); the hand is small enough to publish and audit, which is what
  developers and firms ask first; one brain per team gives the shared
  board that stops agents conflicting; each brain is its own shard, so
  scaling is "run another container", not a multi-tenant design; the
  owner's Mac becomes the first hand, nothing special. Option 2 becomes a
  later deployment mode (more brains), not a rewrite.
- Cons: customer code passes through the brain (the lead plans from the
  repository, evolve measures it) and must be held to a written promise;
  the hosted front is a new trust point and must be unable to act as the
  owner; the hand is a new crate to carve out and keep small.

## Decision

Option 3, with these rules.

1. **Two artefacts.** The **brain** is the `hive` binary as built today,
   closed, run only by the owner as a hosted container per team (one
   SQLite store, one daemon, one hub each). The **hand** is a new crate,
   `hive-hand`, built and released on its own, open source, with no lead,
   planner, evolve, tally, prompt or hub code in its dependency tree - a
   check in the pipeline fails the build if it links any of those crates.
   The brain's self-update pulls a release binary, not a rebuild from
   source (`selfupdate.rs` keeps the rebuild path for the owner's own
   development machine, chosen by setting).

2. **Enrol once, dial out only.** The hub's Machines page shows a six-word
   join code (or a `hive.sh/join/<code>` link) valid for ten minutes. On
   the machine: `curl -fsSL https://hive.sh | sh` then `hive-hand join
   <code>` (or the macOS app's first-launch screen, docs/mac-app.md). The
   hand creates a device key pair, sends the public key with the code, and
   receives a device identity bound to the team. From then on the hand
   opens one outbound TLS connection to its brain through the front, leases
   work, streams results and self-updates. No inbound port, no sshd
   drop-in, no `authorized_keys`, no SSH key from the brain. `hive pair`
   and `executor add --ssh` stay for machines the owner prepares by hand
   and are no longer the default (ADR-020/021 amended).

3. **One git login, inherited.** Git access is a **GitHub App** (and later a
   GitLab equivalent) installed by the team on the web, once. A job that
   needs the repository gets an installation token minted by the brain for
   that repository only, valid for one hour, passed to the hand in the job
   lease and placed in the worktree's git credential helper for the run.
   No PAT reaches a machine; `hive executor login --with-token` and the
   per-hand token file are retired. The brain's own pushes (ADR-037/041/042)
   use the same App token; the `op://` / `keychain://` / `file://` push-key
   references stay as the owner's choice for a brain they run themselves.

4. **Secrets live in a vault, reach memory only.** The front holds a per-team
   vault of ciphertext; the team's data key is wrapped to each enrolled
   device's key and to each member's passkey, so the front cannot read a
   value. A job declares the secret names it needs (the recipe of ADR-026
   is the place); the brain grants them per lease; the hand fetches, decrypts
   and exports them to the harness process for that run and never writes
   them to disk. Every grant and every read is an audit-log event
   (`secret.granted`, `secret.read`) with job, machine and member. The
   `secret_ref` settings of today become names in this vault; the three
   reference schemes stay for a self-run brain.

5. **The front cannot act as the owner.** Every write that changes a brain -
   decide, settings, add a machine, kill switch, task, message - is signed
   in the browser with the member's **passkey** (WebAuthn) over the request
   body and a nonce the brain issued, and the brain verifies the signature
   itself before writing. The front relays bytes; it holds no key that can
   forge a decision. Reads go through the relay under TLS; the board carries
   secret *names* and references, never values, so this is acceptable.
   `hub.allow_remote` is implemented as "through the front only", with the
   same-origin write guard kept.

6. **Where code runs is the team's choice, written down.** Default: the
   brain reads the repository to plan and review (as the lead does today),
   and every job's harness, build and tests run on the team's own hands.
   A `brain.code: none` setting keeps the repository off the brain entirely
   - the lead then plans from the board, the task text and diffs the hands
   report - for teams whose review needs it. Hosted brains keep their store
   and worktrees encrypted at rest, per team, deleted on request. This
   promise is the first page of the product's security notes and is tested
   (a hand with `brain.code: none` set must find no clone on the brain).

7. **A team is one brain.** Members sign in on the web (Clerk, the fleet's
   existing skill), enrol their own machines, and share one hub and one
   board. Agents of the team therefore see one another's claims, mentions,
   landings and reviews - the conflict-avoidance Hive has today, now
   across every member's machine. An individual is a team of one. Joining
   forces across teams is a later decision; the board's append-only,
   immutable posts make it a log-sync problem, not a merge.

8. **Sandbox is a tier, not a day-one requirement.** The hand runs the
   harness as the enrolled user today; `hand.sandbox: container` runs each
   job in a container on the machine (what ADR-008 wanted) when the team
   turns it on. Prompt injection from repository content remains the known
   risk; ADR-041 D4 (no credential resolution inside a job), the kill
   switch, the audit log and the pull-request reviewer of ADR-069 are the
   controls it inherits.

## Consequences

- Nothing of the brain ships to a customer; the only installable artefact is
  small and public, and its smallness is enforced by the pipeline.
- The owner's Mac stops being special: it is the first enrolled hand, and
  the brain it ran becomes a hosted container of the owner's team. Local
  development keeps the current single-machine mode behind a setting.
- The current per-machine credential paths (`executor login --with-token`,
  the `hive-hands` token file, hub-pushed Copilot tokens for rented
  machines) are retired once the App token flow lands; ADR-034's "a
  provisioned machine with no login drains" becomes moot because a hand
  never needs a stored login for git, and harness-provider logins (Copilot,
  Anthropic, Gemini, OpenAI) follow the vault rule 4 as named secrets the
  brain grants per lease.
- ADR-021's "never federated" holds in the strong sense - one brain per
  team, never two stores for one team - and is loosened only in that the
  brain may run off the owner's machine.
- New operational surface for the owner: the hosted front and the brain
  containers (one image, one container per team, SQLite on a persistent
  volume with `backup.to` already pointing at object storage).

## Delivery plan

In order; each step is a task on the board and lands by pull request.

1. **Carve out the hand.** `crates/hive-hand`: the executor path of
   `hive-runner` (lease, worktree, harness run, report) behind a
   dial-out client; a `cargo tree` check that it links none of
   `hive-runner`'s lead/plan/evolve/tally/hub code or `hive-hub`. Builds
   on macOS and Linux (Windows follows TASK-091/092).
2. **Enrolment.** Join code on the Machines page, `hive-hand join`, device
   keys, the brain-side device registry (an `executors` row with
   `transport: dial`), and the outbound lease channel on the hub's port.
3. **GitHub App tokens.** App registration, installation on the web, the
   brain mints per-repo one-hour tokens into the lease, the hand's credential
   helper; the brain's own push uses the same token.
4. **The vault.** Ciphertext store on the front, device- and
   passkey-wrapped team key, `secret.granted` / `secret.read` events, the
   recipe's secret names, the hand's in-memory export.
5. **The front.** Web login (Clerk), passkey-signed writes verified by the
   brain, the relay, `hub.allow_remote` through the front only.
6. **Hosted brains.** The container image, one per team, release-binary
   self-update, encryption at rest, `brain.code: none`, the security notes.
7. **Retire** the SSH pairing default, `--with-token`, the per-hand token
   file; amend ADR-019/020/021/041/042 records.

## Follow-ups

- [ ] `hive record add` this ADR when it merges; `hive record amend`
      ADR-019, ADR-020, ADR-021, ADR-041, ADR-042 to point here
- [ ] Amend `ARCH-overview` (one store per machine -> one brain per team,
      hands dial out) and the `WF-hive` / `INSTR-*` records that say a
      machine is paired over SSH
- [ ] `docs/machines.md` gains "Enrolling a hand" and marks SSH pairing as
      the by-hand path; `docs/console.md` Machines gains the join code;
      `docs/harness.md` the vault-granted provider logins; a new
      `docs/security.md` carries the promise of rule 6
- [ ] Update `delivery:` and `delivered:` as steps close (`hive record deliver`)

## Delivery

`docs/machines.md` (hands, enrolment), `docs/console.md` (Machines, the
front), `docs/harness.md` (provider logins as vault secrets),
`docs/self-improvement.md` (settings: `hand.sandbox`, `brain.code`,
`hub.allow_remote`), a new `docs/security.md`. Built by the tasks the
Delivery plan names.
