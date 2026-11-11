#!/bin/sh
# Scheduled check: the project's own verify command.
#
# The daemon runs this script in a throwaway worktree under HIVE_HOME/work
# that it removes when the run ends, and its environment carries no
# CARGO_TARGET_DIR. A target directory inside that worktree is therefore cold
# on every run: the whole workspace compiles from scratch (several GB written
# and then unlinked each day, on the bulk disk when one is attached). Build
# into a directory that outlives the worktree instead, one per schedule so it
# never overlaps the job clones' verify runs or the daemon's own
# `cargo build --release` (self-update): sharing a target with a concurrent
# build rewrites artifacts under both, doctests fail with E0460 "found
# possibly newer version of crate" and test binaries exit abnormally for
# reasons the default branch does not contain.
#
# A CARGO_TARGET_DIR set by the caller still wins; it gets its own
# subdirectory for the same reason.
#
# The checks are the verify command's: `cargo fmt --all --check`, `cargo
# clippy --workspace --all-targets -- -D warnings`, the hand's isolation
# from the brain (`scripts/check-hand-deps.sh`, ADR-070) and every test of
# `cargo test --workspace`. Only the way the tests are run differs. Plain
# `cargo test` runs one test binary after the other, and most tests of the
# big crates hold a process-wide lock for their whole body (hive-runner's
# `serial()`, hive-evolve's `e2_guard()`), so they run one at a time even
# inside a binary: about 25 minutes of mostly waiting on this machine, past
# the 20 minutes `schedule.script_timeout_minutes` gives the check. So the
# tests are built once, each binary's tests are listed and striped into
# chunks of about VERIFY_CHUNK_TESTS, and VERIFY_POOL chunks run at once,
# each in its own process (its own copy of those locks), one test at a time
# inside it, from the package directory with CARGO_MANIFEST_DIR set, as
# `cargo test` runs them. Every listed test must report a result: a chunk
# whose counts do not add up fails the check. A chunk still running after
# VERIFY_CHUNK_SECONDS is killed and names the tests it never finished.
#
# What each chunk process gets, and why:
# - One test thread. Several test binaries at once, each with a thread per
#   core, starve the timing-sensitive tests, and tests that share a hook
#   (hive-provider's retry jitter, the hub's handlers) race in ways a
#   binary's usual neighbours never trigger. Parallelism comes from the pool.
# - Its own short TMPDIR, under /tmp and with symlinks resolved, removed when
#   the check ends. Some tests clash on fixed names in the temp dir
#   (hive-runner's reclaim root) when two processes of one binary run, and
#   some fail when the temp path is long (a clipped command line, a socket
#   path), so a deep per-chunk directory would fail them for nothing.
# - No HIVE_JOB, HIVE_AGENT or HIVE_SESSION, as the daemon runs the check
#   (schedule_script.rs script_env drops them), so a run by hand inside a job
#   tests the same way. Some tests rely on an earlier test of their process
#   to clear them, and a chunk has a different earlier test.
#
# Usage: scripts/verify-check.sh            (the check)
#        scripts/verify-check.sh --chunk S C (internal: one chunk of the pool)
set -e

if [ "${1:-}" = "--chunk" ]; then
  # One chunk: suite S, chunk C, under the pool's work directory.
  w=$VERIFY_CHECK_WORK
  s=$2
  c=$3
  IFS='	' read -r count dir bin name < "$w/suite.$s"
  names="$w/chunk/$s.$c"
  log="$w/log/$s.$c"
  tmp="$w/tmp/$s.$c"
  mkdir -p "$tmp"
  expect=0
  set --
  if [ -s "$names" ]; then
    while IFS= read -r t; do
      set -- "$@" "$t"
      expect=$((expect + 1))
    done < "$names"
    set -- --exact "$@"
  fi
  if command -v perl > /dev/null 2>&1; then
    # SIGALRM survives exec: the test binary dies when its budget runs out.
    set -- perl -e 'alarm shift @ARGV; exec @ARGV or die "exec: $!\n"' \
      "${VERIFY_CHUNK_SECONDS:-600}" "$bin" "$@"
  else
    set -- "$bin" "$@"
  fi
  start=$(date +%s)
  rc=0
  (
    unset HIVE_JOB HIVE_AGENT HIVE_SESSION
    cd "$dir" && CARGO_MANIFEST_DIR="$dir" TMPDIR="$tmp" exec "$@" --test-threads=1
  ) < /dev/null > "$log" 2>&1 || rc=$?
  secs=$(($(date +%s) - start))
  # The harness's own summary is the last one: tests that re-run the binary
  # as a helper print theirs earlier.
  result=$(sed -n 's/^test result: [A-Za-z]*\. \([0-9]*\) passed; \([0-9]*\) failed; \([0-9]*\) ignored.*/\1 \2 \3/p' "$log" | tail -n 1)
  set -- $result
  passed=${1:-0}
  failed=${2:-0}
  ignored=${3:-0}
  if [ "$rc" -eq 0 ] && [ -n "$result" ] && [ "$failed" -eq 0 ] &&
    { [ "$expect" -eq 0 ] || [ $((passed + ignored)) -eq "$expect" ]; }; then
    verdict=ok
  elif [ "$rc" -eq 142 ]; then
    verdict=timeout
  else
    verdict=FAILED
  fi
  echo "$verdict $secs $passed $ignored $failed $rc $expect" > "$w/status/$s.$c"
  printf '%-7s %s chunk %s: %s passed, %s ignored, %s failed in %ss\n' \
    "$verdict" "$name" "$c" "$passed" "$ignored" "$failed" "$secs"
  exit 0
fi

if [ -n "${CARGO_TARGET_DIR:-}" ]; then
  CARGO_TARGET_DIR="$CARGO_TARGET_DIR/verify-check"
elif [ -n "${HIVE_HOME:-}" ]; then
  CARGO_TARGET_DIR="$HIVE_HOME/cache/cargo-target-check/${HIVE_PROJECT:-amistio-template}/${HIVE_SCHEDULE:-verify-check}"
else
  CARGO_TARGET_DIR="target/verify-check"
fi
export CARGO_TARGET_DIR
# Incremental compilation keeps per-crate session caches that are rewritten on
# every change; a once-a-day check gains little from them and writes far less
# without. A caller's explicit choice still wins.
: "${CARGO_INCREMENTAL:=0}"
export CARGO_INCREMENTAL

began=$(date +%s)
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
# The hand links none of the brain and carries none of its brief text
# (ADR-070 rule 1): its dependency graph, the check's own self-test on a bad
# manifest, and the release binary a customer installs.
sh scripts/check-hand-deps.sh deps
sh scripts/check-hand-deps.sh self-test
cargo build -q -p hive-hand --release
sh scripts/check-hand-deps.sh strings "$CARGO_TARGET_DIR/release/hive-hand"

# A check killed outright (the daemon's timeout) cannot run its trap; its
# directory is swept by a later run once it is a day old.
find /tmp -maxdepth 1 -name 'hive-vc.*' -type d -mmin +1440 -exec rm -rf {} + 2> /dev/null || true
work=$(mktemp -d /tmp/hive-vc.XXXXXX)
trap 'rm -rf "$work"' EXIT
work=$(cd "$work" && pwd -P)
trap 'exit 130' INT TERM HUP
mkdir -p "$work/chunk" "$work/log" "$work/tmp" "$work/status"
export VERIFY_CHECK_WORK="$work"

cargo test --workspace --no-run --message-format=json-render-diagnostics > "$work/build.json"
# One line per test binary: its package's manifest and the executable.
grep '"profile":{[^}]*"test":true}' "$work/build.json" | grep '"executable":"' |
  sed -n 's/.*"manifest_path":"\([^"]*\)".*"executable":"\([^"]*\)".*/\1	\2/p' > "$work/binaries" || true
if [ ! -s "$work/binaries" ]; then
  echo "verify-check: cargo built no test binary; nothing would be tested" >&2
  exit 1
fi
built=$(date +%s)

# Doc tests are compiled by rustdoc, not listed: cargo runs them as before.
cargo test --workspace --doc

per=${VERIFY_CHUNK_TESTS:-20}
pool=${VERIFY_POOL:-}
if [ -z "$pool" ]; then
  cpus=$(getconf _NPROCESSORS_ONLN 2> /dev/null || echo 4)
  pool=$((cpus / 2))
  [ "$pool" -le 8 ] || pool=8
fi
[ "$pool" -ge 1 ] || pool=1

: > "$work/suites"
s=0
total=0
while IFS='	' read -r manifest bin; do
  s=$((s + 1))
  dir=${manifest%/Cargo.toml}
  name=${bin##*/}
  name=${name%-*}
  if ! (cd "$dir" && CARGO_MANIFEST_DIR="$dir" "$bin" --list --format terse) < /dev/null > "$work/list.$s" 2>&1; then
    echo "verify-check: could not list the tests of $bin:" >&2
    tail -n 20 "$work/list.$s" >&2
    exit 1
  fi
  sed -n 's/: test$//p' "$work/list.$s" > "$work/names.$s"
  count=$(grep -c . "$work/names.$s" || true)
  total=$((total + count))
  printf '%s\t%s\t%s\t%s\n' "$count" "$dir" "$bin" "$name" > "$work/suite.$s"
  chunks=$(((count + per - 1) / per))
  [ "$chunks" -ge 1 ] || chunks=1
  # Striped, not cut in runs: a module's slow tests spread over the chunks.
  awk -v n="$chunks" -v base="$work/chunk/$s." '{ f = base ((NR - 1) % n); print >> f; close(f) }' "$work/names.$s"
  c=0
  while [ "$c" -lt "$chunks" ]; do
    : >> "$work/chunk/$s.$c"
    printf '%s %s %s\n' "$count" "$s" "$c" >> "$work/suites"
    c=$((c + 1))
  done
done < "$work/binaries"

echo "verify-check: $total tests in $s binaries, $(grep -c . "$work/suites") chunks, $pool at a time"
# The biggest binaries first, so their chunks do not trail at the end. A
# chunk always exits 0; what it found is in its status file, and a chunk
# that wrote none is counted below.
sort -k1,1nr -k2,2n -k3,3n "$work/suites" | while read -r count s c; do echo "$s $c"; done |
  xargs -n 2 -P "$pool" sh "$0" --chunk || true
ran=$(date +%s)

bad=0
seen=0
: > "$work/failed"
for f in "$work"/status/*; do
  [ -f "$f" ] || continue
  read -r verdict secs passed ignored failed rc expect < "$f"
  seen=$((seen + passed + ignored))
  [ "$verdict" = ok ] && continue
  bad=$((bad + 1))
  id=${f##*/}
  IFS='	' read -r count dir bin name < "$work/suite.${id%.*}"
  echo
  echo "=== $verdict: $name chunk ${id#*.} (exit $rc, $passed passed, $failed failed, $ignored ignored of $expect)"
  if [ "$verdict" = timeout ]; then
    echo "killed after ${VERIFY_CHUNK_SECONDS:-600}s; tests that never finished:"
    # libtest prints "test NAME ... " when a test starts and its verdict
    # when it ends; a test without a verdict was still running.
    while IFS= read -r t; do
      grep -F "test $t ... " "$work/log/$id" | grep -qE '\.\.\. (ok|FAILED|ignored)' ||
        echo "  $t" | tee -a "$work/failed"
    done < "$work/chunk/$id"
  fi
  sed -n 's/^test \(.*\) \.\.\. FAILED$/  \1/p' "$work/log/$id" >> "$work/failed"
  if grep -q '^failures:' "$work/log/$id"; then
    sed -n '/^failures:/,$p' "$work/log/$id" | tail -n 60
  else
    tail -n 30 "$work/log/$id"
  fi
done
expected=$(grep -c . "$work/suites")
reported=$(ls "$work/status" | grep -c . || true)
if [ "$reported" -ne "$expected" ]; then
  echo "verify-check: only $reported of $expected chunks reported a result" >&2
  bad=$((bad + 1))
fi
echo
echo "verify-check: slowest chunks:"
for f in "$work"/status/*; do
  [ -f "$f" ] || continue
  read -r verdict secs rest < "$f"
  id=${f##*/}
  IFS='	' read -r count dir bin name < "$work/suite.${id%.*}"
  echo "$secs $name chunk ${id#*.}"
done | sort -k1,1nr | head -n 5 | sed 's/^\([0-9]*\) \(.*\)/  \2: \1s/'
echo "verify-check: build $((built - began))s, tests $((ran - built))s, total $((ran - began))s; $seen of $total tests reported"
if [ "$bad" -ne 0 ]; then
  if [ -s "$work/failed" ]; then
    echo "verify-check: failing tests (each binary runs from its package directory):"
    sort -u "$work/failed"
  fi
  # The daemon keeps only the tail of stdout followed by stderr (schedule
  # OUTPUT_TAIL), and cargo's Compiling/Doc-tests lines fill stderr, so the
  # report above can fall out of a red run's log. Repeat its gist on stderr,
  # just before the last line: which chunks failed and which tests.
  {
    echo "verify-check: failed chunks:"
    for f in "$work"/status/*; do
      [ -f "$f" ] || continue
      read -r verdict secs passed ignored failed rc expect < "$f"
      [ "$verdict" = ok ] && continue
      id=${f##*/}
      IFS='	' read -r count dir bin name < "$work/suite.${id%.*}"
      echo "  $name chunk ${id#*.}: $verdict (exit $rc, $failed failed)"
    done
    if [ -s "$work/failed" ]; then
      echo "verify-check: failing tests:"
      sort -u "$work/failed" | head -n 40
    fi
  } >&2
  echo "verify-check: $bad chunk(s) failed" >&2
  exit 1
fi
if [ "$seen" -ne "$total" ]; then
  echo "verify-check: $seen of $total listed tests reported a result" >&2
  exit 1
fi
echo "verify-check: ok"
