#!/bin/sh
# Emulated "does it build off unix" check for the Windows port
# (docs/windows-port.md), for machines without the x86_64-pc-windows-msvc
# standard library installed.
#
# It copies the tracked workspace to a scratch directory, and in the named
# crates turns `cfg(unix)` off, renames `std::os::unix`, hides
# `signal_hook::iterator` (signal-hook has none on Windows) and drops the
# `nix` / `libc` dependencies, then runs `cargo check` on them. What fails
# there will fail on Windows; what passes is very likely to pass, though a
# real `cargo check --target x86_64-pc-windows-msvc` stays the proof
# (Windows-only std APIs and third-party crates are not exercised here).
#
# Usage: sh scripts/check-not-unix.sh [--all-targets] CRATE...
#        sh scripts/check-not-unix.sh --leaves [--all-targets]
# Exit status is cargo's; the scratch copy is removed afterwards.
set -eu

LEAVES="hive-gitenv hive-cred hive-store hive-bus hive-cli
hive-cred-setup hive-cred-command hive-provider hive-provider-anthropic
hive-provider-copilot hive-provider-gemini hive-provider-openai
hive-provider-fake hive-board hive-tasks hive-goals hive-goals-command
hive-tally hive-records hive-codedelivery hive-mcp hive-voice hive-hooks
hive-testutil"

targets=""
crates=""
for a in "$@"; do
  case $a in
  --all-targets) targets=--all-targets ;;
  --leaves) crates="$crates $LEAVES" ;;
  *) crates="$crates $a" ;;
  esac
done
if [ -z "$crates" ]; then
  echo "usage: $0 [--all-targets] (--leaves | CRATE...)" >&2
  exit 2
fi

root=$(git rev-parse --show-toplevel)
ws=$(mktemp -d "${TMPDIR:-/tmp}/hive-not-unix.XXXXXX")
trap 'rm -rf "$ws"' EXIT
(cd "$root" && git ls-files -z Cargo.toml Cargo.lock crates | xargs -0 tar -cf - | tar -xf - -C "$ws")

pkgs=""
for c in $crates; do
  d="$ws/crates/$c"
  if [ ! -d "$d" ]; then
    echo "no crate $c" >&2
    exit 2
  fi
  find "$d/src" -name '*.rs' -exec perl -pi -e '
    s/cfg\(unix\)/cfg(hive_unix_off)/g;
    s/cfg\(not\(unix\)\)/cfg(not(hive_unix_off))/g;
    s/cfg!\(unix\)/cfg!(hive_unix_off)/g;
    s/cfg_attr\(not\(unix\),/cfg_attr(not(hive_unix_off),/g;
    s/cfg_attr\(unix,/cfg_attr(hive_unix_off,/g;
    s/cfg\(all\(unix,/cfg(all(hive_unix_off,/g;
    s/cfg\(all\(test, unix\)\)/cfg(all(test, hive_unix_off))/g;
    s/target_os = "(macos|linux)"/target_os = "hive_off"/g;
    s/std::os::unix/std::os::hive_unix_off/g;
    s/signal_hook::iterator/signal_hook::hive_iterator_off/g;
  ' {} +
  perl -ni -e 'print unless /^(nix|libc)\.workspace/' "$d/Cargo.toml"
  pkgs="$pkgs -p $c"
done

cd "$ws"
# Build into the caller's target dir when set, so a second run is warm; the
# rewritten sources fingerprint differently, so they never serve a real build.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ws/target}/not-unix"
# shellcheck disable=SC2086
RUSTFLAGS="-A unexpected_cfgs" cargo check --offline --keep-going $pkgs $targets
