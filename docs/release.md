# Releases

PULSE is released as a tagged version `vX.Y.Z` with a prebuilt `pulse`
executable for macOS attached to a GitHub Release. Nothing is published
outside the repository yet; the repository is private, so downloading a
release needs a GitHub account that can read it.

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

## Installing a release on a Mac

```sh
mkdir -p ~/.pulse/bin && cd "$(mktemp -d)"
gh release download --repo lamplitlabs/pulse --pattern "*-$(uname -m | sed s/arm64/aarch64/)-apple-darwin.tar.gz"
tar -xzf pulse-*.tar.gz && install -m 755 pulse-*/pulse ~/.pulse/bin/pulse
pulse version
```

`gh release download` without a tag takes the latest release; add `vX.Y.Z`
for a particular one. Verify the archive against `SHA256SUMS` with
`shasum -a 256 -c SHA256SUMS --ignore-missing`. The binaries are not notarized;
macOS Gatekeeper does not check a binary that `gh` or `curl` downloads and
`install` copies, but Finder may refuse one dragged in from a browser
(`xattr -d com.apple.quarantine pulse` clears that).

## Being told about a new release

The daemon reads the setting `update.latest_url` at most once a day and
remembers the newest version it names; `pulse version` and the console banner
then say when a newer PULSE is out. For a private repository the GitHub API
needs a token and the setting refuses URLs carrying credentials, so point it
at a document the machine can read without one: a GitHub releases list on a
machine where `gh` is signed in is not enough. Until a public release
document exists, the check stays off (the setting's default is empty) and the
daemon's own `daemon.self_update` keeps a brain machine current by rebuilding
from `main`. See the board thread of TASK-087 for the plan to publish a
public `latest` document and a `pulse update` command.

## Windows

Not built yet. The runner and daemon use unix-only process, signal and file
APIs (`std::os::unix` in ~140 places; services via launchd/systemd), so a
Windows release means porting those first, not just adding a build target.
