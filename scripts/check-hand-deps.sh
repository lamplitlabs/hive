#!/bin/sh
# The hand stays free of the brain (ADR-070 rule 1).
#
# `pulse-hand` is the one binary a customer installs; the code that plans,
# leads, scores and briefs (the brain) stays hosted and must never ship to a
# customer's machine. This check holds that line twice:
#
# 1. deps: `cargo tree -p pulse-hand -e normal` must not list a brain crate
#    (pulse-runner with its lead/plan/evolve/tally/research/idle modules,
#    pulse-evolve, pulse-tally, pulse-hub, pulse-goals, pulse-goals-command,
#    pulse-ask, and the brain's store and board). A crate that pulls one in
#    fails here, before it reaches a release.
# 2. strings BINARY: a built `pulse-hand` must not carry the brain's prompt
#    text. Three sentinel phrases from the worker brief
#    (crates/pulse-runner/src/brief.rs) must each be found 0 times by
#    `strings BINARY | grep -c`.
#
# Usage: scripts/check-hand-deps.sh [deps] [--manifest-path Cargo.toml] [--package pulse-hand]
#        scripts/check-hand-deps.sh strings target/release/pulse-hand
#        scripts/check-hand-deps.sh self-test   (the check fails on a bad manifest)
# Exit 0: clean; 1: the brain leaked in (named on stderr); 2: usage or tool error.
set -eu

# The brain's crates. A line of `cargo tree --prefix none` starts with the
# crate name and a space ("pulse-hub v0.1.0 (/path)").
FORBIDDEN="pulse-runner pulse-evolve pulse-tally pulse-hub pulse-goals
pulse-goals-command pulse-ask pulse-store pulse-board pulse-harness"

# Phrases only the brain's briefs hold (crates/pulse-runner/src/brief.rs).
SENTINELS="Show your work when it has a face
Clean up after yourself before you finish
Never edit"

deps() {
  manifest=""
  package=pulse-hand
  while [ $# -gt 0 ]; do
    case $1 in
    --manifest-path) manifest=$2; shift 2 ;;
    --package) package=$2; shift 2 ;;
    *) echo "check-hand-deps: unknown argument $1" >&2; exit 2 ;;
    esac
  done
  if [ -n "$manifest" ]; then
    set -- --manifest-path "$manifest"
  else
    set --
  fi
  if ! tree=$(cargo tree "$@" -p "$package" -e normal --prefix none --offline 2>&1); then
    echo "check-hand-deps: cargo tree -p $package failed:" >&2
    echo "$tree" >&2
    exit 2
  fi
  found=""
  for crate in $FORBIDDEN; do
    if printf '%s\n' "$tree" | grep -q "^$crate v"; then
      found="$found $crate"
    fi
  done
  if [ -n "$found" ]; then
    echo "check-hand-deps: $package links the brain:$found" >&2
    echo "  the hand must link the executor path alone (ADR-070 rule 1);" >&2
    echo "  move what it needs into pulse-hand-core instead" >&2
    exit 1
  fi
  echo "check-hand-deps: $package links none of the brain ($(printf '%s\n' "$tree" | sort -u | wc -l | tr -d ' ') crates)"
}

strings_check() {
  bin=$1
  if [ ! -f "$bin" ]; then
    echo "check-hand-deps: no binary at $bin" >&2
    exit 2
  fi
  if ! command -v strings > /dev/null 2>&1; then
    echo "check-hand-deps: strings is not installed" >&2
    exit 2
  fi
  bad=0
  text=$(strings "$bin")
  while IFS= read -r phrase; do
    n=$(printf '%s\n' "$text" | grep -c -- "$phrase" || true)
    if [ "$n" -ne 0 ]; then
      echo "check-hand-deps: $bin carries brief text \"$phrase\" ($n times)" >&2
      bad=1
    fi
  done <<EOF
$SENTINELS
EOF
  if [ "$bad" -ne 0 ]; then
    exit 1
  fi
  echo "check-hand-deps: $bin carries none of the brief's sentinel phrases"
}

# The check must fail on a manifest that pulls the brain in: a scratch
# workspace whose `hand` crate depends on a crate named pulse-hub.
self_test() {
  scratch=$(mktemp -d "${TMPDIR:-/tmp}/check-hand-deps.XXXXXX")
  trap 'rm -rf "$scratch"' EXIT INT TERM
  mkdir -p "$scratch/hand/src" "$scratch/pulse-hub/src"
  cat > "$scratch/Cargo.toml" <<'EOF'
[workspace]
members = ["hand", "pulse-hub"]
resolver = "2"
EOF
  cat > "$scratch/pulse-hub/Cargo.toml" <<'EOF'
[package]
name = "pulse-hub"
version = "0.1.0"
edition = "2021"
EOF
  : > "$scratch/pulse-hub/src/lib.rs"
  cat > "$scratch/hand/Cargo.toml" <<'EOF'
[package]
name = "pulse-hand"
version = "0.1.0"
edition = "2021"

[dependencies]
pulse-hub = { path = "../pulse-hub" }
EOF
  echo 'fn main() {}' > "$scratch/hand/src/main.rs"
  rc=0
  (deps --manifest-path "$scratch/Cargo.toml") > /dev/null 2>&1 || rc=$?
  if [ "$rc" -ne 1 ]; then
    echo "check-hand-deps: self-test: a hand that links pulse-hub passed (exit $rc)" >&2
    exit 1
  fi
  # And a hand without it passes.
  sed -i.bak '/pulse-hub = /d' "$scratch/hand/Cargo.toml"
  rc=0
  (deps --manifest-path "$scratch/Cargo.toml") > /dev/null 2>&1 || rc=$?
  if [ "$rc" -ne 0 ]; then
    echo "check-hand-deps: self-test: a clean hand failed (exit $rc)" >&2
    exit 1
  fi
  echo "check-hand-deps: self-test: a bad manifest fails, a clean one passes"
}

case ${1:-deps} in
deps) [ $# -gt 0 ] && shift; deps "$@" ;;
--manifest-path | --package) deps "$@" ;;
strings) [ $# -eq 2 ] || { echo "usage: $0 strings BINARY" >&2; exit 2; }; strings_check "$2" ;;
self-test) self_test ;;
*) echo "usage: $0 [deps [--manifest-path M] [--package P] | strings BINARY | self-test]" >&2; exit 2 ;;
esac
