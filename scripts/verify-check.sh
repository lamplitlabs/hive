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
# VERIFY_CHUNK_SECONDS is killed and names the tests it never finished. A
# chunk whose binary is gone, or passes but ran fewer tests than it listed
# (libtest filters out an --exact name the binary no longer has), fails as
# `changed`: another build rewrote the binary during the run (a target dir
# shared with another tree), which says nothing about this tree's code.
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
# How many chunks run at once: VERIFY_POOL when it is set (a whole number of
# at least 1, from the hand's environment), else half the CPUs, at least 2
# and at most 8. Half of a 2-CPU hand is 1, and one chunk at a time runs the
# tests for about 48 minutes, past the 40 the command limit gives the
# pre-commit verify; most chunks wait on a lock or a sleep rather than the
# CPU, so two at a time fit even there. `--pool` prints the pool this
# machine gets and exits; VERIFY_CPUS stands in for the CPU count there
# (scripts/tests/verify-check-test.sh).
#
# The pre-commit gate narrows the check to what the commit can break. The
# hook the runner installs exports HIVE_VERIFY_BASE, the commit the job
# started from: a cold full check takes 53-57 minutes on a 2-CPU hand, and
# the hook's `git commit` is killed at 40 (harness.commit_timeout), so a
# worker there could never commit. With HIVE_VERIFY_BASE set, the crates
# whose files changed since that commit are checked, with every workspace
# crate that depends on them (cargo tree -i, dev and build edges too); a
# crate whose changes are all test-only files (tests/, benches/, tests.rs,
# *_tests.rs, test_helpers*.rs: compiled only into its own tests) is
# checked alone. A change under web/hub (the console) counts as hive-hub
# code, which builds and embeds it, so it checks hive-hub and hive; one
# under web/hub/mock also checks hive-store, whose settings test reads the
# mock (scripts/tests/verify-check-hub-scope-test.sh). fmt stays whole;
# clippy, the test binaries and the doc tests take -p for those crates;
# the hand's release build and strings run only when hive-hand is among
# them. Any other change outside crates/ (the manifest, the lock file,
# scripts/, docs/, web/site), a
# crate this tree does not have, a base that is not a commit here, or a
# cargo tree that fails runs the full check, as does a scope that comes to
# every crate. A narrowed run writes `scoped` to HIVE_VERIFY_SCOPE_FILE when
# the hook names one, and the hook then writes no verified-tree memo: the
# run's own check and the landing, which never set HIVE_VERIFY_BASE, still
# run everything. `--scope` prints the decision and exits
# (scripts/tests/verify-check-test.sh).
#
# Usage: scripts/verify-check.sh            (the check)
#        scripts/verify-check.sh --pool     (print the pool size and exit)
#        scripts/verify-check.sh --scope    (print the pre-commit scope and exit)
#        scripts/verify-check.sh --chunk S C (internal: one chunk of the pool)
set -e

# Prints how many chunks run at once (see above).
pool_size() {
  case "${VERIFY_POOL:-}" in
    '') ;;
    *[!0-9]*)
      echo "verify-check: VERIFY_POOL=$VERIFY_POOL is not a whole number; using the default pool" >&2
      ;;
    *)
      # Leading zeros dropped: sh arithmetic reads 08 as a bad octal.
      size=${VERIFY_POOL#"${VERIFY_POOL%%[!0]*}"}
      if [ -n "$size" ]; then
        echo "$size"
        return
      fi
      echo "verify-check: VERIFY_POOL=$VERIFY_POOL is below 1; using the default pool" >&2
      ;;
  esac
  cpus=${VERIFY_CPUS:-$(getconf _NPROCESSORS_ONLN 2> /dev/null || echo 4)}
  case "$cpus" in
    '' | *[!0-9]*) cpus=4 ;;
  esac
  size=$((cpus / 2))
  [ "$size" -ge 2 ] || size=2
  [ "$size" -le 8 ] || size=8
  echo "$size"
}

# Sets scope_crates to the packages the check is narrowed to, space
# separated, or to nothing for the full check, and scope_why to the reason
# (see above).
scope() {
  scope_crates=
  scope_why=
  scope_base=${HIVE_VERIFY_BASE:-}
  if [ -z "$scope_base" ]; then
    scope_why="no HIVE_VERIFY_BASE"
    return 0
  fi
  if ! git rev-parse -q --verify "$scope_base^{commit}" > /dev/null 2>&1; then
    scope_why="HIVE_VERIFY_BASE=$scope_base is not a commit of this tree"
    return 0
  fi
  # The working tree against the base, committed, staged and unstaged alike,
  # and the files git does not track yet (cargo builds a new tests/x.rs all
  # the same): what the check is about to build.
  if ! scope_changed=$(git diff --name-only --no-renames "$scope_base" -- 2> /dev/null) ||
    ! scope_new=$(git ls-files --others --exclude-standard 2> /dev/null); then
    scope_why="git could not list the changes since $scope_base"
    return 0
  fi
  scope_changed=$(printf '%s\n%s\n' "$scope_changed" "$scope_new" | grep . || true)
  if [ -z "$scope_changed" ]; then
    scope_why="nothing changed since $scope_base"
    return 0
  fi
  scope_code=
  scope_tests=
  while IFS= read -r scope_path; do
    case "$scope_path" in
      crates/*/*) ;;
      web/hub/*)
        # The console is hive-hub's code: its build.rs builds web/hub into
        # the export server.rs embeds, and its tests read files under
        # web/hub/src. The mock is read by hive-store's settings test too.
        if [ ! -f crates/hive-hub/Cargo.toml ]; then
          scope_why="$scope_path is outside crates/ and this tree has no crates/hive-hub"
          return 0
        fi
        scope_code="$scope_code hive-hub"
        case "$scope_path" in
          web/hub/mock/*)
            if [ -f crates/hive-store/Cargo.toml ]; then
              scope_tests="$scope_tests hive-store"
            fi
            ;;
        esac
        continue
        ;;
      *)
        scope_why="$scope_path is outside crates/"
        return 0
        ;;
    esac
    scope_dir=${scope_path#crates/}
    scope_dir=${scope_dir%%/*}
    if [ ! -f "crates/$scope_dir/Cargo.toml" ]; then
      scope_why="crates/$scope_dir has no Cargo.toml"
      return 0
    fi
    scope_name=$(sed -n 's/^name *= *"\(.*\)" *$/\1/p' "crates/$scope_dir/Cargo.toml" | head -n 1)
    if [ -z "$scope_name" ]; then
      scope_why="crates/$scope_dir/Cargo.toml names no package"
      return 0
    fi
    case "${scope_path#crates/"$scope_dir"/}" in
      tests/* | benches/* | src/tests.rs | src/*/tests.rs | src/*_tests.rs | src/test_helpers.rs | src/test_helpers_*.rs)
        scope_tests="$scope_tests $scope_name"
        ;;
      *)
        scope_code="$scope_code $scope_name"
        ;;
    esac
  done << EOF
$scope_changed
EOF
  scope_all=$(cargo tree --workspace --offline --depth 0 -e normal --prefix none 2> /dev/null |
    sed -n 's/^\([^ ]*\) v[^ ]* (\/.*/\1/p' | sort -u)
  if [ -z "$scope_all" ]; then
    scope_why="cargo tree could not list the workspace"
    return 0
  fi
  scope_set=
  if [ -n "$scope_code" ]; then
    set --
    for scope_name in $scope_code; do
      set -- "$@" -i "$scope_name"
    done
    if ! scope_set=$(cargo tree --workspace --offline -e normal,build,dev --prefix none "$@" 2> /dev/null); then
      scope_why="cargo tree could not list what depends on$scope_code"
      return 0
    fi
    scope_set=$(printf '%s\n' "$scope_set" | sed -n 's/^\([^ ]*\) v[^ ]* (\/.*/\1/p')
  fi
  scope_set=$(printf '%s\n' $scope_set $scope_tests | sort -u)
  if [ -z "$scope_set" ]; then
    scope_why="no workspace crate is affected"
    return 0
  fi
  if [ "$(printf '%s\n' "$scope_set" | grep -c .)" -ge "$(printf '%s\n' "$scope_all" | grep -c .)" ]; then
    scope_why="every crate depends on what changed"
    return 0
  fi
  scope_crates=$(printf '%s\n' "$scope_set" | tr '\n' ' ')
  scope_crates=${scope_crates% }
  scope_why="$(printf '%s\n' "$scope_set" | grep -c .) of $(printf '%s\n' "$scope_all" | grep -c .) crates are affected by the changes since $scope_base"
}

if [ "${1:-}" = "--pool" ]; then
  pool_size
  exit 0
fi

if [ "${1:-}" = "--scope" ]; then
  scope
  if [ -n "$scope_crates" ]; then
    echo "crates: $scope_crates"
  else
    echo "full: $scope_why"
  fi
  exit 0
fi

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
  elif [ ! -x "$bin" ] ||
    { [ "$rc" -eq 0 ] && [ -n "$result" ] && [ "$failed" -eq 0 ] &&
      [ "$expect" -gt 0 ] && [ $((passed + ignored)) -lt "$expect" ]; }; then
    # The binary no longer holds tests it listed when the pool was planned:
    # libtest filters out an --exact name it lacks and still passes. Another
    # build rewrote it (a target dir shared with another tree), so this is
    # no verdict on the tree's code.
    verdict=changed
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
# The pre-commit gate's scope (see above): -p for each affected crate, or
# the whole workspace.
scope
if [ -n "$scope_crates" ]; then
  echo "verify-check: scoped to $scope_crates ($scope_why); the landing runs the full check"
  if [ -n "${HIVE_VERIFY_SCOPE_FILE:-}" ]; then
    echo scoped > "$HIVE_VERIFY_SCOPE_FILE" || true
  fi
  set --
  doc=
  for scope_name in $scope_crates; do
    set -- "$@" -p "$scope_name"
    # `cargo test --doc` refuses a package without a library (hive-hand).
    if [ -f "crates/$scope_name/src/lib.rs" ]; then
      doc="$doc -p $scope_name"
    fi
  done
else
  if [ -n "${HIVE_VERIFY_BASE:-}" ]; then
    echo "verify-check: full check: $scope_why"
  fi
  set -- --workspace
  doc=--workspace
fi
cargo fmt --all --check
cargo clippy "$@" --all-targets -- -D warnings
# The hand links none of the brain and carries none of its brief text
# (ADR-070 rule 1): its dependency graph, the check's own self-test on a bad
# manifest, and the release binary a customer installs.
sh scripts/check-hand-deps.sh deps
sh scripts/check-hand-deps.sh self-test
if [ -z "$scope_crates" ] || printf ' %s ' "$scope_crates" | grep -q ' hive-hand '; then
  cargo build -q -p hive-hand --release
  sh scripts/check-hand-deps.sh strings "$CARGO_TARGET_DIR/release/hive-hand"
fi

# A check killed outright (the daemon's timeout) cannot run its trap; its
# directory is swept by a later run once it is a day old.
find /tmp -maxdepth 1 -name 'hive-vc.*' -type d -mmin +1440 -exec rm -rf {} + 2> /dev/null || true
work=$(mktemp -d /tmp/hive-vc.XXXXXX)
trap 'rm -rf "$work"' EXIT
work=$(cd "$work" && pwd -P)
trap 'exit 130' INT TERM HUP
mkdir -p "$work/chunk" "$work/log" "$work/tmp" "$work/status"
export VERIFY_CHECK_WORK="$work"

cargo test "$@" --no-run --message-format=json-render-diagnostics > "$work/build.json"
# One line per test binary: its package's manifest and the executable.
grep '"profile":{[^}]*"test":true}' "$work/build.json" | grep '"executable":"' |
  sed -n 's/.*"manifest_path":"\([^"]*\)".*"executable":"\([^"]*\)".*/\1	\2/p' > "$work/binaries" || true
if [ ! -s "$work/binaries" ]; then
  echo "verify-check: cargo built no test binary; nothing would be tested" >&2
  exit 1
fi
built=$(date +%s)

# Doc tests are compiled by rustdoc, not listed: cargo runs them as before.
if [ -n "$doc" ]; then
  # shellcheck disable=SC2086 # one word per flag
  cargo test $doc --doc
fi

per=${VERIFY_CHUNK_TESTS:-20}
pool=$(pool_size)

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
changed=0
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
  if [ "$verdict" = changed ]; then
    echo "the test binary changed during the run (shared target dir?): $bin"
    echo "tests it listed before the run and no longer ran:"
    while IFS= read -r t; do
      grep -qF "test $t ... " "$work/log/$id" || echo "  $t"
    done < "$work/chunk/$id" | head -n 20
    changed=$((changed + 1))
  fi
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
      if [ "$verdict" = changed ]; then
        echo "  $name chunk ${id#*.}: $verdict (exit $rc, $((passed + ignored)) of $expect tests ran) - the test binary changed during the run (shared target dir?)"
      else
        echo "  $name chunk ${id#*.}: $verdict (exit $rc, $failed failed)"
      fi
    done
    if [ -s "$work/failed" ]; then
      echo "verify-check: failing tests:"
      sort -u "$work/failed" | head -n 40
    fi
    if [ "$changed" -ne 0 ]; then
      echo "verify-check: $changed chunk(s) ran a test binary another build rewrote during the run (shared target dir?); rerun the check with a CARGO_TARGET_DIR no other tree builds into before reading this as a code failure"
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
