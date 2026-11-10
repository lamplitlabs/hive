# Releases and `pulse update`

How a machine gets a newer PULSE: by rebuilding from the brain project's
checkout (the **source** channel, [self-update](runner.md#services-and-self-update)),
or by installing a published release (the **release** channel, this page).

PULSE is released as a tagged version `vX.Y.Z` with a prebuilt `pulse`
executable for macOS attached to a GitHub Release. The repository is
private, so reading or downloading a release needs a GitHub account that can
see it (`gh` signed in).

## How a release is cut

Two workflows in `.github/workflows/`:

- **cut-release.yml** decides the version, writes it into `Cargo.toml`
  (`[workspace.package] version`), `Cargo.lock` and `web/hub/package.json`,
  commits `release: vX.Y.Z` on `main`, tags it and pushes. It runs every night
  at 03:00 UTC and does nothing when `main` has not moved since the last
  release tag. Run it by hand from the Actions tab (**Run workflow**) for a
  release now, picking `patch`, `minor` or `major`. The nightly run bumps the
  patch unless a pull request merged since the last tag carries the label
  `release: minor` or `release: major`.
- **release.yml** runs when a `vX.Y.Z` tag is pushed (cut-release starts it
  explicitly, since a push made with the workflow token starts no workflow on
  its own). It builds `pulse` with the console embedded on
  `aarch64-apple-darwin` (Apple silicon) and `x86_64-apple-darwin` (Intel),
  checks that the binary reports the tag in `pulse version`, packs each as
  `pulse-vX.Y.Z-<target>.tar.gz`, and publishes a GitHub Release with the
  archives, a `SHA256SUMS` file and notes listing the commits since the
  previous tag.

By hand, the same thing is:

```sh
git checkout main && git pull
git tag -a v0.2.0 -m "PULSE v0.2.0" && git push origin v0.2.0
gh workflow run release.yml -f tag=v0.2.0
```

The version a binary prints comes from `crates/pulse/build.rs`: an exact
release tag at HEAD gives `vX.Y.Z`; a later commit on `main` gives
`vX.Y.(Z+1)-0.<stamp>-<rev>`, which the update check and `pulse version`
treat as newer than `vX.Y.Z` and older than the next release. Release tags
must stay reachable from `main`: squashing `main` to a new root orphans them
and every build falls back to `v0.0.0-...`.

## Installing a release on a Mac by hand

```sh
mkdir -p ~/.pulse/bin && cd "$(mktemp -d)"
gh release download --repo lamplitlabs/pulse --pattern "*-$(uname -m | sed s/arm64/aarch64/)-apple-darwin.tar.gz"
tar -xzf pulse-*.tar.gz && install -m 755 pulse-*/pulse ~/.pulse/bin/pulse
pulse version
```

`gh release download` without a tag takes the latest release; add `vX.Y.Z`
for a particular one. Verify the archive against `SHA256SUMS` with
`shasum -a 256 -c SHA256SUMS --ignore-missing`. Once installed, `pulse
update` (below) takes over. The binaries are not notarized; Gatekeeper does
not check a binary `gh` downloads and `install` copies, but Finder may
refuse one dragged in from a browser (`xattr -d com.apple.quarantine pulse`
clears that).

## Windows

Not built yet. The runner and daemon use unix-only process, signal and file
APIs (`std::os::unix` in ~140 places; services via launchd/systemd), so a
Windows release means porting those first (TASK-091), not just adding a
build target.

## The two channels: `update.channel`

| Value | What happens |
| --- | --- |
| `auto` (default) | `source` when a brain project is registered and `cargo` is on PATH, `release` otherwise |
| `source` | the daemon rebuilds pulse from the brain's checkout after each landing (`daemon.self_update`); `pulse update` only reports (`--check`) |
| `release` | the daemon never builds from source; `pulse update` installs the newest published release, and every command says once per release that one is out |

Set it with `pulse settings set update.channel release` or under
**Settings → Updates** in the console.

## Where releases are found: `update.latest_url`

Both the daily check and `pulse update` read the JSON document
`update.latest_url` names: a GitHub `releases/latest` answer
(`https://api.github.com/repos/OWNER/REPO/releases/latest`), a GitHub
releases list (drafts and pre-releases skipped), or a plain document:

```json
{
  "version": "v1.2.3",
  "url": "https://example.test/releases/v1.2.3",
  "assets": [
    {"name": "pulse-v1.2.3-aarch64-apple-darwin.tar.gz", "url": "https://example.test/pulse-v1.2.3-aarch64-apple-darwin.tar.gz"},
    {"name": "SHA256SUMS", "url": "https://example.test/SHA256SUMS"}
  ]
}
```

GitHub's answer lists the same `assets` (`name`, `browser_download_url`,
and the API `url`). The setting is empty by default: no check, no network.

**A private repository.** GitHub answers 404 to an anonymous request for a
private repository's releases. When an `api.github.com` address fails over
HTTP and `gh` is on PATH, PULSE reads it again with `gh api` as the account
gh is signed in with, and downloads the assets through their API addresses
the same way. PULSE never stores a token: the setting refuses a URL that
carries a credential, and gh keeps its own login.

## What a release must carry

For each target it ships, one archive named
`pulse-vX.Y.Z-<target>.tar.gz` holding the `pulse` binary at its top (or one
folder down), plus one `SHA256SUMS` file listing every archive in
`sha256sum` form (`<64 hex digits>  <file name>`). The targets PULSE asks
for are `aarch64-apple-darwin` and `x86_64-apple-darwin` (and
`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` on Linux). The
binary inside must print the release's version from `pulse version --json`;
a build at the exact tag does (`crates/pulse/build.rs`), and
`PULSE_VERSION=vX.Y.Z cargo build --release -p pulse --locked` forces it
elsewhere. `release.yml` above produces exactly this layout.

## `pulse update`

```
pulse update [--check] [--json]
```

- `--check` says whether a newer release exists. It uses what the daily
  check stored when that is under a day old and reads the document
  otherwise; it downloads nothing.
- Without `--check` (channel `release` only) it reads the document, picks
  the archive for the running target (the exact name first, else the one
  `pulse-*<target>*.tar.gz`), downloads it and `SHA256SUMS`, and refuses to
  go on when the release has no `SHA256SUMS`, lists no digest for the
  archive, or the digest differs. It then unpacks the archive, writes the
  binary beside the running one as `pulse.new`, signs it on macOS (the same
  ad-hoc requirement the self-update gives every build), smoke-tests it -
  `pulse.new version --json` must print the release's version, `pulse.new
  help` must exit 0 - copies the running binary to `pulse.prev` and renames
  `pulse.new` into its place. Any failure before the rename leaves the
  running binary as it was and removes `pulse.new`.
- The daemon sees its binary change on disk and restarts on it after its
  runs in flight (the restart drain, `daemon.restart_drain_minutes`); the
  hub restarts within a minute. Nothing in flight is stopped.
- Roll back with `mv ~/.pulse/bin/pulse.prev ~/.pulse/bin/pulse`.
- An install writes the event `pulse.update` (from, to, binary, asset,
  sha256).

On the `source` channel `pulse update` refuses to install and says how to
switch; `pulse update --check` still reports.

## `daemon.self_update release`: a machine without source

A hosted brain ([security.md](security.md)) holds no PULSE source, so the
source self-update has nothing to build. `pulse settings set
daemon.self_update release` makes the daemon do what `pulse update` does,
on its own:

- once an hour it reads the document `update.latest_url` names (an empty
  setting says so once and does nothing);
- when a newer release exists, it installs it exactly as above: the archive
  for the running target, its `SHA256SUMS` line, signed, smoke-tested,
  swapped in with the old binary kept as `pulse.prev`;
- it restarts after its runs in flight, the same way the source
  self-update does (exit 3; the service or `deploy/entrypoint.sh` starts it
  again). A failed install leaves the running binary alone, says why in the
  daemon's log and is tried again after `daemon.self_update_retry_minutes`.

It never runs `cargo` and never reads the brain's checkout, and it works
whatever `update.channel` says. With `brain.code none` the source mode
(`on`) stands down and points here. The test is
`release_update::tests::self_update_release_mode_installs_the_release_and_never_runs_cargo`
in `crates/pulse-runner`.

**A Linux brain needs a Linux archive.** The release workflow attaches
macOS archives today. Until it also builds `x86_64-unknown-linux-gnu`
(and `aarch64-unknown-linux-gnu`), a hosted brain finds no archive for its
target, says so, and keeps the binary its image was built with. Rebuild
the image (`deploy/brain.sh build vX.Y.Z`) to update it in the meantime.

## The once-per-release line

On the `release` channel, when the stored check knows a newer release, the
owner's next command prints one line on stderr and then stays quiet until
the next release:

```
A newer PULSE v1.2.3 is available - run: pulse update
```

It reads the store only (no network). Commands run as an agent or inside a
job never print it, nor do read-only commands; the release it last named is
kept in the store's `meta` row `update.prompted`. `pulse version` keeps
printing its own notice line every time.

## The console

The banner above every page (except chat) shows the stored notice with the
release link. An **Update now** button that asks the daemon to install the
release (`POST /api/v1/update/apply`) is proposed, not built: `web/hub` is
owner-owned.

Code: crates/pulse-cli/src/update.rs (the pure parts: assets, checksums,
channel, the prompt), crates/pulse-runner/src/release_update.rs (download,
verify, install, `pulse update`), crates/pulse-runner/src/update_check.rs
(the daily check).
