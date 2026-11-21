---
id: ADR-071
title: Hive is an open-source tool, local-first and MIT-licensed, with no hosted brain
status: Proposed
date: 2026-11-21
areas:
  - runner
  - hub
  - machines
  - security
  - release
  - docs
tags:
  - adr
  - open-source
  - local-first
  - licence
  - product
amends:
  - ADR-070 (a hosted brain per team, a closed brain and an open hand, one web login that every machine inherits)
delivery: not started
delivered: null
---

# ADR-071: Hive is an open-source tool, local-first and MIT-licensed, with no hosted brain

## Context

ADR-070 (9 November 2026) set out to make Hive a product for individual
developers and small firms. It chose a hosted brain per team: the `hive`
binary, closed, run by the owner as one container per team. It chose an
open hand, `hive-hand`, as the only thing a customer installs. And it chose
a hosted front for sign-in, enrolment, git tokens, a secret vault and the
relay to each team's brain. Protecting the code of Hive itself decided that
choice: nothing of the brain was to be installed at a customer.

What ADR-070 built is on the default branch today:

- **The hand.** `crates/hive-hand` and `crates/hive-hand-core`, the check
  `scripts/check-hand-deps.sh` in the verify command, and
  `executor.transport local-hand` (docs/machines.md, "The hand").
- **Enrolment and the dial channel.** A six-word join code, a device key
  per machine, `hive-hand join` and `hive-hand dial` (docs/machines.md,
  "Enrolling a hand").
- **GitHub App tokens.** A token for one repository that lasts one hour,
  minted by the brain for its own git and `gh` calls and leased to a hand
  on the same machine for its one push (docs/landing.md, "One git login
  for every machine").
- **The vault.** `hive secret` and the recipe's `secrets:` list, sealed in
  the brain's store. Only the brain opens it; a hand cannot yet
  (docs/harness.md, "Team secrets").
- **A brain without code.** `brain.code none`, `hive init --store-only` and
  `daemon.self_update release` (docs/security.md, docs/release.md).
- **A container image.** `deploy/Dockerfile`, `deploy/entrypoint.sh`,
  `deploy/compose.yaml` and `deploy/brain.sh`.
- **The brain's side of remote access.** `hub.allow_remote`, the relay
  listener of `hive hub --front-port`, passkey-signed writes and the
  `members` table (docs/console.md).

What it did not build on the default branch is the product around those
pieces: the hosted front and the brains run for teams. That work is on the
branch `feat/web-first`, 43 commits ahead of the default branch (head
`3c6f97db`, 20 November 2026): a sign-in service (`hive-front`), tooling to
run many teams' brains on one host (`hive-fleet`), a Members page for the
console, registering a repository by its URL, and the site's pages for
teams. It was never merged.

On 21 November 2026 the owner decided not to build a hosted business. In
his words, Hive is "a harness tool with RSI which can pair multiple
machines": a tool that runs on your own machines, with a harness of its
own, RSI (recursive self-improvement) under a constitution, and more
machines paired when the work needs them. And it is open source.

This record writes that decision down and says what follows from it. It is
Proposed until the owner accepts it.

## Options Considered

### Option 1: carry on with ADR-070's hosted product

The brain stays closed and is run for each team as a hosted container. The
hand is the only thing a team installs. The front signs people in and
relays them to their brain.

- Pros: the path is partly built, on the default branch and on
  `feat/web-first`; a team installs one small program per machine and
  nothing else; one brain per team gives every machine of a team the same
  board; the code of the brain is never downloaded.
- Cons: the owner becomes an operator, every day: containers, a sign-in
  service, backups, and other people's code and data (ADR-070 names this
  itself: "New operational surface for the owner"); a team's code passes
  through a brain the team does not run and has to be held to a written
  promise; the front is a new trust point that must be built and kept
  unable to act as the owner; a closed brain cannot be read, audited or
  fixed by the people whose agents it runs; delivery steps 5 to 7 are still
  to finish, and then to run.

### Option 2: keep Hive closed and local

Hive stays what it was before ADR-070: one binary on its owner's machine,
the source private.

- Pros: nothing to operate; nothing changes in how Hive runs; the code
  stays the owner's alone.
- Cons: nobody else can use it, read it or fix it; a tool that runs
  commands with a person's credentials asks for a trust that a closed
  binary cannot earn; the install stays a private clone and a build; none
  of what ADR-070 built for more machines reaches anyone but the owner.

### Option 3: open source, local-first (chosen)

The whole repository is published under the MIT licence. Hive runs on the
machines of whoever installs it. Nobody hosts anything for anybody.

- Pros: the owner operates nothing and holds nobody's code or data; anyone
  can read what the agents are allowed to do, which is the first thing
  asked of a tool that runs commands on your machine; the install becomes
  one line, from a public release; what ADR-070 built keeps its use, as
  options for people who run a brain on a server of their own; the
  founding concepts need no change; issues and pull requests from outside
  become possible.
- Cons: the prompts, the briefs, the planner and the loop are public, so
  ADR-070's third goal, protecting the code of Hive itself, is given up;
  there is no hosted product to sell; each person operates their own Hive:
  updates, backups, credentials, machines; a team does not get one shared
  brain out of the box; a public repository publishes its whole history,
  so the history must be clean of secrets before it opens; outside issues
  and pull requests take the owner's time.

## Decision

Option 3, with these rules.

1. **The whole repository is open source under the MIT licence.** Brain
   and hand alike: every crate, the console, the site, the Mac app, the
   docs and the deploy scripts. `LICENSE` holds the text (Copyright (c)
   2026 Lamplit Labs); `Cargo.toml` already said `license = "MIT"`.

2. **Hive is local-first.** It runs on the machines of the person who
   installs it: one binary, one SQLite store, one console on loopback. No
   brain is hosted by anyone for anyone. Hive has no accounts and no
   sign-in of its own; the only sign-ins are to services a person already
   uses, such as a model provider and GitHub.

3. **What ADR-070 built stays, as options for someone who runs Hive
   themselves.** None of it is a service, and the docs describe each as
   what it is, with what is not finished (docs/security.md):
   - `hive-hand`, `hive-hand-core` and `executor.transport local-hand`;
   - enrolling a machine with a join code and the dial channel
     (`hive-hand join`, `hive-hand dial`), beside pairing over SSH
     (`hive pair`), which is not retired;
   - GitHub App installation tokens (`git.github_app.*`);
   - the secret vault (`hive secret`, the recipe's `secrets:`);
   - `brain.code none`, `hive init --store-only` and
     `daemon.self_update release`;
   - the container image and its scripts in `deploy/`;
   - the relay listener and passkey-signed writes (`hub.allow_remote`,
     `hive hub --front-port`, the `members` table). No relay ships with
     Hive.

4. **What ADR-070 planned and is withdrawn.**
   - The hosted front: the web login, the relay run by the owner, a team
     as an organization (rule 5's front, rule 7, delivery step 5).
   - Hosted brains for teams: a brain "run only by the owner as a hosted
     container per team", and the promise of a service around it, per
     team, encrypted at rest and deleted on request (rules 1 and 6,
     delivery step 6).
   - The rule that nothing of the brain may be installed at a user (rule
     1 and the first consequence). The brain is what a user installs.
   - Sign-in on the site.

5. **The branch `feat/web-first` is not merged.** The work for the hosted
   product stays there as an archive.

6. **`scripts/check-hand-deps.sh` stays, and its reason changes.** It was
   written to keep closed code off a customer's machine. From now on it
   keeps the hand small and easy to audit: the program that answers a
   brain's calls on someone's machine links none of the planner, the
   lead, the loop, the store, the console or the harness. The check and
   the hand's own test (`crates/hive-hand/tests/no_brain.rs`) stay in the
   verify command as they are.

7. **The founding concepts are unchanged.** The harness, the board, votes,
   the constitution, the self-improvement loop and records in the store
   are what Hive is. Delivery by pull request with an agent reviewer
   (ADR-069) stands as written.

## Consequences

- The repository becomes public. That is the owner's step, and it comes
  after a scan of the history for secrets: making a repository public
  publishes every commit, not only the tree.
- Releases become public downloads. The install is one line (the
  installer, `scripts/install.sh`, served by the site), with archives for
  macOS and Linux and a checksums file on each release. The steps a
  private repository needed (a signed-in `gh`, `gh release download`) are
  no longer the way in.
- The site is rebuilt for the tool: what Hive is, how to install it, the
  docs. Its sign-in goes.
- ADR-070's remaining steps are closed. Step 5 (the front), step 6 as a
  hosted offer and step 7 (retiring SSH pairing and the per-machine login
  paths) will not be done. `hive pair`, `hive executor add --ssh` and
  `hive executor login` stay.
- The docs change their voice. `docs/security.md` stops being the promise
  of a service and becomes the security notes of a tool you run yourself,
  with one list of what is not built.
- Each person operates their own Hive. Nothing is operated for them, and
  nothing of theirs is held by anyone else.
- A team that wants one shared brain is not served by a product. The
  pieces exist as options and are unfinished in the ways
  `docs/security.md` lists.
- Contributions from outside arrive as issues and pull requests. A person
  reviews a contributor's pull request; the reviewer's rule (ADR-069)
  already keeps an agent from merging a branch Hive did not push.

## Follow-ups

- [ ] The owner accepts or declines this record (`hive record accept
      ADR-071 --words '...'`, or **Accept** on its card under For you).
- [ ] Scan the history for secrets, then make the repository public (the
      owner).
- [ ] Turn on GitHub's private vulnerability reporting for the
      repository; `SECURITY.md` points at it (the owner).
- [ ] The release workflow publishes archives for macOS and Linux and the
      installer downloads them (`scripts/install.sh`, `docs/release.md`).
- [ ] The site is rebuilt without a sign-in, renders `docs/` and serves
      the installer (`web/site`).
- [ ] Close ADR-070: mark what was delivered, close steps 5 to 7
      (`hive record deliver`), and amend the records it pointed at a
      hosted brain (ADR-019, ADR-020, ADR-021, ADR-041, ADR-042).
- [ ] Reword what still speaks of a hosted product and is outside this
      change: the comments in `deploy/` and `scripts/check-hand-deps.sh`,
      the setting descriptions and the hand's usage text in `crates/`, and
      the pages `docs/console.md`, `docs/harness.md`, `docs/landing.md`,
      `docs/mac-app.md` and `docs/self-improvement.md`.
- [ ] Decide, one by one, what becomes of the unfinished options that
      `docs/security.md` lists (the vault on other machines, landing
      without a checkout, a relay).
- [ ] Push or tag `feat/web-first` if the archive should outlive one
      machine; today the branch exists only in the owner's repository.

## Delivery

`LICENSE`, `README.md`, `CONTRIBUTING.md`, `SECURITY.md`,
`CODE_OF_CONDUCT.md` and the issue and pull request templates;
`docs/security.md` (reframed), `docs/concepts.md` and `docs/faq.md` (new);
the installer and the release workflow; the site. Built on the branch
`feat/open-tool`.
