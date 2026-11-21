# Releases and `hive update`

How a machine gets Hive, and a newer Hive after that: by installing a
published release (the **release** channel, this page), or by rebuilding
from the brain project's checkout (the **source** channel,
[self-update](runner.md#services-and-self-update)).

Hive is released as a tagged version `vX.Y.Z`. Each release is a public
download at <https://github.com/lamplitlabs/hive/releases>, with prebuilt
programs for macOS and Linux: `hive`, and `hive-hand`, the small program
another machine runs to take work from your Hive
([machines.md](machines.md#the-hand)).

## Installing

```bash
curl -fsSL https://hivefleet.dev/install.sh | sh      # macOS and Linux; installs ~/.hive/bin/hive
```

then

```bash
export PATH="$HOME/.hive/bin:$PATH"    # add to your shell profile
hive version
hive hub                               # the console, at http://127.0.0.1:8765
```

The first `hive hub` on a machine makes the store, `~/.hive/hive.sqlite`,
and says so in one line. `hive init --store-only` makes one without
starting the console, for a service or a script.

The script is `scripts/install.sh` in the repository, and the site serves
the same file. To read it before you run it, end the line with `| less`
in place of `| sh`. It never runs `sudo` and never edits a shell profile.
The same file can be taken straight from the repository, for when the site
cannot be reached:

```bash
curl -fsSL https://raw.githubusercontent.com/lamplitlabs/hive/main/scripts/install.sh | sh
```

What it does, in order:

1. **Finds the machine.** macOS or Linux, on arm64 or x86_64
   ([the targets](#the-targets)). On anything else it names the machine,
   prints the line that builds Hive from source and stops. A Linux whose
   C library is musl (Alpine, for one) is told that the release is built
   for glibc.
2. **Finds the release.** The latest one, or the one you name. It reads
   the release's `SHA256SUMS` first, and that file names the release's
   archives. So it makes no call to the GitHub API, and GitHub's hourly
   limit on API calls from one address does not stop an install.
3. **Downloads and checks.** It downloads the archive for the machine with
   `curl` (or `wget` where there is no `curl`) into a temporary directory,
   and compares the archive's SHA-256 with its line in `SHA256SUMS`, using
   `sha256sum` or `shasum`. A digest that differs stops it. So does a
   machine with neither tool: nothing is installed unchecked.
4. **Tries the program.** It writes the program beside the place it will
   go and runs it once (`hive version`). A program that does not start on
   this machine is removed again, and the script says why: on Linux the
   usual reason is an older glibc than the release is built for. On macOS
   it first signs `hive` on your machine the way `hive update` does, with
   no certificate, so macOS does not ask about your folders again after an
   upgrade ([runner.md](runner.md#macos-folder-access)).
5. **Puts it in place.** `~/.hive/bin/hive`, mode 755, moved there in one
   step, so an older `hive` stays whole until the new one takes its place.
   The temporary directory is removed when the script ends, and a run that
   fails leaves nothing behind.
6. **Says what is next.** Where the program is, whether that directory is
   on your `PATH` (with the line to add when it is not), and the command
   to run: `hive hub`.

Options go after `sh -s --` when the script is piped, or after the file
name (`sh install.sh --hand`):

| Option | Variable | What it does |
| --- | --- | --- |
| `--hand` | | installs `hive-hand` in `~/.hive-hand/bin`, and not `hive` |
| `--version TAG` | `HIVE_VERSION` | installs the release `TAG` (`v0.2.0`), and not the latest one |
| `--dir DIR` | `HIVE_INSTALL_DIR` for `hive`, `HIVE_HAND_INSTALL_DIR` for `hive-hand` | installs into `DIR` |
| `--help` | | prints the options |
| | `HIVE_RELEASE_BASE` | reads releases from another address than `https://github.com/lamplitlabs/hive/releases`: a mirror, or with `curl` a folder (`file:///...`), laid out as below |

```sh
curl -fsSL https://hivefleet.dev/install.sh | sh -s -- --version v0.2.0
curl -fsSL https://hivefleet.dev/install.sh | sh -s -- --hand
```

Give `HIVE_VERSION` to that one command (`... | HIVE_VERSION=v0.2.0 sh`)
and do not export it: a build from source reads the same variable to stamp
the binary. `HIVE_INSTALL_OS` and `HIVE_INSTALL_ARCH` stand in for what
`uname` says; the installer's own test uses them.

Under `HIVE_RELEASE_BASE` the script reads three addresses:
`latest/download/SHA256SUMS` for the latest release, and for a release
`TAG`, `download/TAG/SHA256SUMS` and `download/TAG/<archive>`.

**What the check proves.** The archive you got is the one the release
lists: a download that was damaged, cut short or swapped on the way is
refused. It does not prove who built the release, because the checksums
come from the same place as the archive, and a release carries no
signature of its maker.

**Upgrading.** Run the same line again, or use [`hive update`](#hive-update).
Hive's services (`hive service install`) notice the new program on disk
and restart on it by themselves: the runner once its runs in flight have
ended or been put on hold, the console once no conversation is at work
([`hive update`](#hive-update) has the details, and
[runner.md](runner.md#services-and-self-update) the services). A `hive hub`
you started by hand keeps running the old program until you start it
again. To go back to an older release, run the line with `--version` and
its tag.

**Hearing about new releases.** A fresh install does not look for
releases by itself: `update.latest_url` is empty until you set it
([below](#where-releases-are-found-updatelatest_url)). `hive update` and
`hive update --check` do not wait for the setting when you run them by
hand: while it is empty they look at the project's own releases,
`https://api.github.com/repos/lamplitlabs/hive/releases/latest`, for that
one run, and say so. To hear of a release without asking, set it once:

```sh
hive settings set update.latest_url https://api.github.com/repos/lamplitlabs/hive/releases/latest
```

From then on Hive looks once a day and says when a release is out. The
console's **Update now** button needs the setting too, and so does
[`daemon.self_update release`](#daemonself_update-release-a-machine-without-source),
which has the runner install each release by itself. `hive update`
installs a release on a machine that follows the release channel
([the two channels](#the-two-channels-updatechannel)).

The test of the installer is `scripts/tests/install-test.sh`. It makes up
a release of stub programs in a temporary directory, runs the installer
against it with a home of its own, and needs no network.

### The targets

| Machine | Target |
| --- | --- |
| macOS on Apple silicon | `aarch64-apple-darwin` |
| macOS on Intel | `x86_64-apple-darwin` |
| Linux on x86_64 | `x86_64-unknown-linux-gnu` |
| Linux on arm64 | `aarch64-unknown-linux-gnu` |

The Linux programs are built on Debian 12, and the release workflow fails
when one of them asks for a glibc newer than 2.36. So they run on Debian
12, on Ubuntu 24.04 and on anything with a glibc that new or newer. That
number is what a release promises, and a build may ask for less: the
workflow prints what each binary asks for, and the installer does not read
the version, it tries the program, so a somewhat older Linux where the
program starts is not turned away. On a Linux where it does not start, on
a Linux with another C library, and on any other processor, build from
source:

```bash
cargo install --git https://github.com/lamplitlabs/hive --locked --root ~/.hive hive   # from source: Rust 1.95+, Node.js 22+
```

The macOS programs are not notarized. Gatekeeper does not check a program
that `curl` downloads and the installer copies, but Finder may refuse one
that a browser downloaded (`xattr -d com.apple.quarantine hive` clears
that).

### `hive-hand`

```sh
curl -fsSL https://hivefleet.dev/install.sh | sh -s -- --hand
```

puts `hive-hand` in `~/.hive-hand/bin`, checked the same way. The machine
then joins your Hive with the line the console shows under **Machines**,
**Add a machine**, **Run one command on it**, **Get a join code**
([machines.md](machines.md#enrolling-a-hand)):

```sh
hive-hand join --brain URL CODE
```

That line enrols the machine and then stays connected, so it does not
return by itself. `hive-hand dial` is for every later start.

### Checking a download by hand

Every release has one `SHA256SUMS` file beside its archives. To do by
hand what the installer does:

```sh
tag=v0.2.0
target=aarch64-apple-darwin
files=https://github.com/lamplitlabs/hive/releases/download/$tag
curl -fsSLO "$files/hive-$tag-$target.tar.gz"
curl -fsSLO "$files/SHA256SUMS"
shasum -a 256 -c SHA256SUMS --ignore-missing    # on Linux: sha256sum -c SHA256SUMS --ignore-missing
tar -xzf "hive-$tag-$target.tar.gz"
mkdir -p ~/.hive/bin && install -m 755 "hive-$tag-$target/hive" ~/.hive/bin/hive
hive version
```

The check prints `hive-v0.2.0-aarch64-apple-darwin.tar.gz: OK` and fails
when the digest differs. `--ignore-missing` passes over the lines of the
archives you did not download. For `hive-hand` the archive is
`hand-$tag-$target.tar.gz` and the program inside it `hive-hand`. Once
`hive` is installed, `hive update` (below) takes over.

## How a release is cut

Two workflows in `.github/workflows/`:

- **cut-release.yml** decides the version, writes it into `Cargo.toml`
  (`[workspace.package] version`), `Cargo.lock` and `web/hub/package.json`,
  commits `release: vX.Y.Z` on `main`, tags it and pushes. It runs every night
  at 03:00 UTC and does nothing when `main` has not moved since the last
  release tag. Run it by hand from the Actions tab (**Run workflow**) for a
  release now, picking `patch`, `minor` or `major`. The nightly run bumps the
  patch unless a pull request merged since the last tag carries the label
  `release: minor` or `release: major`. It looks only at merge commits, so
  the label counts on a pull request merged with a merge commit and not on
  one that was squashed or rebased.
- **release.yml** runs when a `vX.Y.Z` tag is pushed (cut-release starts it
  explicitly, since a push made with the workflow token starts no workflow on
  its own). For each of the four targets it builds `hive` with the console
  embedded, checks that the binary reports the tag in `hive version` and
  that the console is in it, builds `hive-hand`, checks that it carries
  none of the brain's brief text (`scripts/check-hand-deps.sh`), and packs
  each program in an archive of its own. It then makes sure all eight
  archives are there, writes one `SHA256SUMS`, installs both programs from
  those files with `scripts/install.sh`, and publishes a GitHub Release
  with the archives, `SHA256SUMS` and notes that say how to install and
  list the commits since the previous tag.

What a release carries:

| Asset | Holds | Targets |
| --- | --- | --- |
| `hive-<tag>-<target>.tar.gz` | `hive-<tag>-<target>/hive` | [the four](#the-targets) |
| `hand-<tag>-<target>.tar.gz` | `hand-<tag>-<target>/hive-hand` | the same four |
| `SHA256SUMS` | one `sha256sum` line for each archive above | |

- **Only hive's archives start with `hive`.** When `hive update` finds no
  archive under the exact name, it takes the one asset that starts with
  `hive` and names its target, so no other asset may.
- **Linux is built inside Debian 12**, natively on each architecture, on
  GitHub's `ubuntu-24.04` and `ubuntu-24.04-arm` runners in the
  `rust:1.95-bookworm` image. A binary linked against a newer C library
  than the one it finds does not start, and Debian 12 is also what the
  image in `deploy/` runs. A step fails the build when a binary asks for
  more than glibc 2.36; the installer names the same number when a
  program does not start on a Linux.
- **macOS** is built on the `macos-15` (Apple silicon) and
  `macos-15-intel` runners.
- **A release goes out whole.** When one target does not build, nothing
  is published: no release lacks an archive the installer would ask for.
- **Rights.** The workflow uses no secret but its own token, and only the
  job that publishes may write to the repository.
- Releases cut before this workflow built Linux carry `hive` for macOS
  only. The installer says so when you ask one of them for another
  target.

By hand, a release is a tag:

```sh
git checkout main && git pull
git tag -a v0.2.0 -m "Hive v0.2.0" && git push origin v0.2.0
```

A tag you push yourself starts release.yml. To build a tag again, after a
run that failed for example, start it from the Actions tab or with
`gh workflow run release.yml -f tag=v0.2.0`. Two runs for one tag take
turns.

The version a binary prints comes from `crates/hive/build.rs`: an exact
release tag at HEAD gives `vX.Y.Z`; a later commit on `main` gives
`vX.Y.(Z+1)-0.<stamp>-<rev>`, which the update check and `hive version`
treat as newer than `vX.Y.Z` and older than the next release. Release tags
must stay reachable from `main`: squashing `main` to a new root orphans them
and every build falls back to `v0.0.0-...`.

## Windows

Not built yet. The runner and daemon use unix-only process, signal and file
APIs (`std::os::unix` on about 160 lines in 84 files; services via
launchd/systemd), so a Windows release means porting those first (TASK-091,
[windows-port.md](windows-port.md)), not just adding a build target. The
installer says so on Windows and stops.

## The two channels: `update.channel`

| Value | What happens |
| --- | --- |
| `auto` (default) | `source` when a brain project is registered and `cargo` is on PATH, `release` otherwise |
| `source` | the daemon rebuilds hive from the brain's checkout after each landing (`daemon.self_update`); `hive update` only reports (`--check`) |
| `release` | the daemon never builds from source; `hive update` installs the newest published release, and every command says once per release that one is out |

Set it with `hive settings set update.channel release`, or in the console
under **Settings → Other**.

## Where releases are found: `update.latest_url`

Both the daily check and `hive update` read the JSON document
`update.latest_url` names: a GitHub `releases/latest` answer
(`https://api.github.com/repos/OWNER/REPO/releases/latest`; for Hive's own
releases, `https://api.github.com/repos/lamplitlabs/hive/releases/latest`),
a GitHub releases list (drafts and pre-releases skipped), or a plain
document:

```json
{
  "version": "v1.2.3",
  "url": "https://example.test/releases/v1.2.3",
  "assets": [
    {"name": "hive-v1.2.3-aarch64-apple-darwin.tar.gz", "url": "https://example.test/hive-v1.2.3-aarch64-apple-darwin.tar.gz"},
    {"name": "SHA256SUMS", "url": "https://example.test/SHA256SUMS"}
  ]
}
```

GitHub's answer lists the same `assets` (`name`, `browser_download_url`,
and the API `url`). The setting is empty by default: Hive then makes no
check of its own and no network request for one. `hive update` and
`hive update --check`, run by hand while it is empty, read Hive's own
releases (`https://api.github.com/repos/lamplitlabs/hive/releases/latest`)
for that one run and say so. The setting stays empty, so the daily check,
`daemon.self_update release` and the console's **Update now** button
still wait for you to set it.

**When GitHub turns the request away.** GitHub limits how many API calls
one address may make in an hour without an account, and it answers 404
for the releases of a repository that is not public, such as a fork you
keep to yourself. When an `api.github.com` address fails over HTTP and
`gh` is on PATH, Hive reads it again with `gh api` as the account gh is
signed in with, and downloads the assets through their API addresses the
same way. Without `gh` the read fails: `hive update` says why, and the
daily check tries again a day later. Hive never stores a token: the
setting refuses a URL that carries a credential, and gh keeps its own
login.

## What a release must carry

For each target it ships, one archive named
`hive-vX.Y.Z-<target>.tar.gz` holding the `hive` binary at its top (or one
folder down), plus one `SHA256SUMS` file listing every archive in
`sha256sum` form (`<64 hex digits>  <file name>`). The targets Hive asks
for are `aarch64-apple-darwin` and `x86_64-apple-darwin` on macOS, and
`x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu` on Linux. The
binary inside must print the release's version from `hive version --json`;
a build at the exact tag does (`crates/hive/build.rs`), and
`HIVE_VERSION=vX.Y.Z cargo build --release -p hive --locked` forces it
elsewhere. `release.yml` above produces exactly this layout. `hive update`
does not read the `hand-` archives, and none of their names starts with
`hive`. The installer reads the same names and the same `SHA256SUMS`, so
a release that `hive update` can install is one the installer can
install.

## `hive update`

```
hive update [--check] [--json]
```

- `--check` says whether a newer release exists. It uses what the daily
  check stored when that is under a day old and reads the document
  otherwise; it downloads nothing.
- Without `--check` (channel `release` only) it reads the document, picks
  the archive for the running target (the exact name first, else the one
  `hive-*<target>*.tar.gz`), downloads it and `SHA256SUMS`, and refuses to
  go on when the release has no `SHA256SUMS`, lists no digest for the
  archive, or the digest differs. It then unpacks the archive, writes the
  binary beside the running one as `hive.new`, signs it on macOS (the same
  ad-hoc requirement the self-update gives every build) and smoke-tests
  it: `hive.new version --json` must print the release's version and
  `hive.new help` must exit 0. Then it copies the running binary to
  `hive.prev` and renames `hive.new` into its place. Any failure before
  the rename leaves the running binary as it was and removes `hive.new`.
- While `update.latest_url` is empty, both forms look at Hive's own
  releases, `https://api.github.com/repos/lamplitlabs/hive/releases/latest`,
  for that one run, and say so. Nothing is written to the setting.
- The daemon sees its binary change on disk and restarts on it: as soon as
  nothing is in flight, and otherwise when the restart drain ends
  (`daemon.restart_drain_minutes`, 30 by default). A run whose harness is
  still at work then is asked to hold (`daemon.restart_hold`, `on` by
  default): the harness saves its conversation, and the next daemon
  carries the job on in the same clone. A run that is past its harness, in
  its check for one, runs to its end first.
- The hub looks at its binary every 30 seconds and restarts when no
  conversation is at work. Otherwise it waits for the turns in flight, for
  up to 30 minutes, and then stops them and restarts.
- The two services (`hive service install`) do this by themselves. A
  daemon you started by hand exits with code 3 and is yours to start
  again. A `hive hub` you started by hand keeps running the old program
  until you start it again, unless you gave it `--restart-on-change`.
- Roll back with `mv ~/.hive/bin/hive.prev ~/.hive/bin/hive`.
- An install writes the event `hive.update` (from, to, binary, asset,
  sha256).

On the `source` channel `hive update` refuses to install and says how to
switch; `hive update --check` still reports.

## `daemon.self_update release`: a machine without source

A machine can run Hive without holding its source: one that got Hive from
the installer need not have a checkout of it, and the container image in
`deploy/` is built to hold none ([security.md](security.md)). The source
self-update has nothing to build there. `hive settings set
daemon.self_update release` makes the daemon do what `hive update` does,
on its own:

- once an hour it reads the document `update.latest_url` names (an empty
  setting says so once and does nothing);
- when a newer release exists, it installs it exactly as above: the archive
  for the running target, its `SHA256SUMS` line, signed, smoke-tested,
  swapped in with the old binary kept as `hive.prev`;
- it then restarts the way `hive update` above describes, as the source
  self-update does (exit 3; the service or `deploy/entrypoint.sh` starts it
  again). A failed install leaves the running binary alone, says why in the
  daemon's log and is tried again after `daemon.self_update_retry_minutes`.

It never runs `cargo` and never reads the brain's checkout, and it works
whatever `update.channel` says. With `brain.code none` the source mode
(`on`) stands down and points here. The test is
`release_update::tests::self_update_release_mode_installs_the_release_and_never_runs_cargo`
in `crates/hive-runner`.

**Linux needs a Linux archive.** A release cut by the workflow above
carries `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`, built
against the C library of Debian 12, which the image in `deploy/` runs. A
container started by `deploy/brain.sh` therefore finds an archive for its
target in such a release. A release cut before the workflow built Linux
carries none: the daemon says so and keeps the binary it has. Rebuild the
image (`deploy/brain.sh build vX.Y.Z`) to update it then.

## The once-per-release line

On the `release` channel, when the stored check knows a newer release, the
owner's next command prints one line on stderr and then stays quiet until
the next release:

```
A newer Hive v1.2.3 is available - run: hive update
```

It reads the store only (no network). Commands run as an agent or inside a
job never print it, nor do read-only commands; the release it last named is
kept in the store's `meta` row `update.prompted`. `hive version` keeps
printing its own notice line every time.

## The console

The banner above every page (except chat) shows the stored notice with the
release link. On the `release` channel it also has an **Update now**
button, which asks the hub to install the release the way `hive update`
does (`POST /api/v1/update/apply`) and then shows how the install went.
The hub refuses, and says why, on the `source` channel, while
`update.latest_url` is empty and while an install is already running.

Code: scripts/install.sh (the installer), scripts/tests/install-test.sh
(its test), .github/workflows/release.yml (what a release carries),
crates/hive-cli/src/update.rs (the pure parts: assets, checksums, channel,
the prompt), crates/hive-runner/src/release_update.rs (download, verify,
install, `hive update`), crates/hive-runner/src/update_check.rs (the daily
check).
