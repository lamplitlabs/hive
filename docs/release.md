# Releases and `pulse update`

How a machine gets a newer PULSE: by rebuilding from the brain project's
checkout (the **source** channel, [self-update](runner.md#services-and-self-update)),
or by installing a published release (the **release** channel, this page).

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
binary inside must print the release's version from `pulse version --json`:
build it with `PULSE_VERSION=vX.Y.Z cargo build --release -p pulse --locked`.

The workflow that builds these lives under `.github/`, which the owner
decides (CODEOWNERS); agents propose changes to it rather than edit it.

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
