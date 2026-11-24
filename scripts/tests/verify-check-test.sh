#!/bin/sh
# Tests how many chunks scripts/verify-check.sh runs at once, through its
# `--pool` option, and which crates the pre-commit gate checks, through its
# `--scope` option: both print and exit before anything is built, so the
# test takes seconds (the scope cases need cargo and git on PATH).
#
#   sh scripts/tests/verify-check-test.sh
#   HIVE_TEST_SHELL=/bin/dash sh scripts/tests/verify-check-test.sh    the shell the script runs under (default: sh)
#
# VERIFY_CPUS stands in for the machine's CPU count. It prints one line a
# case and a count, and exits 1 when a case failed.
set -u

here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd) || exit 2
script="$here/../verify-check.sh"
if [ ! -f "$script" ]; then
  echo "verify-check-test: no file at $script" >&2
  exit 2
fi

test_shell=${HIVE_TEST_SHELL:-sh}
case "$test_shell" in
  /*) ;;
  *)
    if ! command -v "$test_shell" > /dev/null 2>&1; then
      echo "verify-check-test: no shell named $test_shell" >&2
      exit 2
    fi
    test_shell=$(command -v "$test_shell")
    ;;
esac

passed=0
failed=0

# pool WANT NAME [VAR=VALUE...]: the pool with the given environment (and no
# VERIFY_POOL or VERIFY_CPUS but those) is WANT.
pool() {
  want=$1
  name=$2
  shift 2
  said=$(env -u VERIFY_POOL -u VERIFY_CPUS "$@" "$test_shell" "$script" --pool 2> /dev/null)
  status=$?
  if [ "$status" -eq 0 ] && [ "$said" = "$want" ]; then
    passed=$((passed + 1))
    printf 'ok    %s\n' "$name"
  else
    failed=$((failed + 1))
    printf 'FAIL  %s\n        exit %s, pool %s, want %s\n' "$name" "$status" "$said" "$want"
  fi
}

pool 2 "a 2-CPU hand runs two chunks at once, not one" VERIFY_CPUS=2
pool 2 "a 1-CPU hand runs two chunks at once" VERIFY_CPUS=1
pool 2 "4 CPUs run two" VERIFY_CPUS=4
pool 6 "12 CPUs run half of them" VERIFY_CPUS=12
pool 8 "64 CPUs run at most 8" VERIFY_CPUS=64
pool 2 "a CPU count that is not a number counts as 4" VERIFY_CPUS=many
pool 3 "VERIFY_POOL=3 is honoured on a 2-CPU hand" VERIFY_CPUS=2 VERIFY_POOL=3
pool 1 "VERIFY_POOL=1 is honoured below the floor" VERIFY_CPUS=2 VERIFY_POOL=1
pool 12 "VERIFY_POOL=12 is honoured above the cap" VERIFY_CPUS=64 VERIFY_POOL=12
pool 8 "VERIFY_POOL=08 is read as 8, not as octal" VERIFY_CPUS=2 VERIFY_POOL=08
pool 2 "VERIFY_POOL=0 falls back to the default pool" VERIFY_CPUS=2 VERIFY_POOL=0
pool 6 "VERIFY_POOL=lots falls back to the default pool" VERIFY_CPUS=12 VERIFY_POOL=lots
pool 2 "an empty VERIFY_POOL is the default pool" VERIFY_CPUS=2 VERIFY_POOL=

# A bad override is named on stderr, not swallowed.
said=$(env -u VERIFY_CPUS VERIFY_POOL=lots "$test_shell" "$script" --pool 2>&1 > /dev/null)
case "$said" in
  *"VERIFY_POOL=lots is not a whole number"*)
    passed=$((passed + 1))
    printf 'ok    %s\n' "a VERIFY_POOL that is not a number is named"
    ;;
  *)
    failed=$((failed + 1))
    printf 'FAIL  %s\n        stderr: %s\n' "a VERIFY_POOL that is not a number is named" "$said"
    ;;
esac

# What the pre-commit gate checks (`--scope`): a scratch workspace of
# path-only crates, so cargo tree resolves it offline. core is what every
# crate depends on; b depends on a, c on a for its tests only (a dev edge);
# d stands alone; hand is a binary.
if ! command -v cargo > /dev/null 2>&1 || ! command -v git > /dev/null 2>&1; then
  failed=$((failed + 1))
  printf 'FAIL  %s\n' "the scope cases need cargo and git on PATH"
else
  scratch=$(mktemp -d "${TMPDIR:-/tmp}/verify-check-test.XXXXXX")
  trap 'rm -rf "$scratch"' EXIT
  ws="$scratch/ws"
  # crate DIR NAME DEPS DEV_DEPS [bin]: a package under crates/DIR.
  crate() {
    mkdir -p "$ws/crates/$1/src"
    {
      printf '[package]\nname = "%s"\nversion = "0.1.0"\nedition = "2021"\n' "$2"
      printf '\n[dependencies]\n'
      for dep in $3; do printf '%s = { path = "../%s" }\n' "$dep" "$dep"; done
      printf '\n[dev-dependencies]\n'
      for dep in $4; do printf '%s = { path = "../%s" }\n' "$dep" "$dep"; done
    } > "$ws/crates/$1/Cargo.toml"
    if [ "${5:-}" = bin ]; then
      echo 'fn main() {}' > "$ws/crates/$1/src/main.rs"
    else
      echo '' > "$ws/crates/$1/src/lib.rs"
    fi
  }
  mkdir -p "$ws/scripts"
  printf '[workspace]\nresolver = "2"\nmembers = ["crates/*"]\n' > "$ws/Cargo.toml"
  crate core core "" ""
  crate a a "core" ""
  crate b b "a core" ""
  crate c c "core" "a"
  crate d d "core" ""
  crate hand hive-hand "core" "" bin
  echo 'echo hi' > "$ws/scripts/x.sh"
  (
    cd "$ws" &&
      git init -q -b main &&
      git config user.email t@example.com &&
      git config user.name t &&
      git config commit.gpgsign false &&
      cargo tree --workspace --offline > /dev/null 2>&1 &&
      git add -A &&
      git commit -q -m base
  ) > /dev/null 2>&1 || echo "verify-check-test: could not make the scratch workspace" >&2
  base=$(git -C "$ws" rev-parse HEAD 2> /dev/null)

  # scope WANT NAME EDIT...: with the working tree EDITed from base (one
  # file per word: appended to, or created), --scope prints WANT.
  scope() {
    want=$1
    name=$2
    shift 2
    git -C "$ws" checkout -q -f "$base" 2> /dev/null
    git -C "$ws" clean -qfd 2> /dev/null
    for f in "$@"; do
      case "$f" in
        */*) mkdir -p "$ws/${f%/*}" ;;
      esac
      echo '// changed' >> "$ws/$f"
    done
    said=$(cd "$ws" && env -u HIVE_VERIFY_BASE ${scope_env:-HIVE_VERIFY_BASE=$base} "$test_shell" "$script" --scope 2> /dev/null)
    status=$?
    case "$said" in
      $want)
        if [ "$status" -eq 0 ]; then
          passed=$((passed + 1))
          printf 'ok    %s\n' "$name"
          return
        fi
        ;;
    esac
    failed=$((failed + 1))
    printf 'FAIL  %s\n        exit %s, said %s, want %s\n' "$name" "$status" "$said" "$want"
  }

  scope "crates: d" "a change to one leaf crate checks that crate alone" crates/d/src/lib.rs
  scope "crates: a b c" "a library change checks its dependents, dev edges too" crates/a/src/lib.rs
  scope "crates: a" "a change to a crate's unit-test module checks that crate alone" crates/a/src/tests.rs
  scope "crates: a" "a change to a crate's test helpers checks that crate alone" crates/a/src/test_helpers_x.rs
  scope "crates: a" "a new integration test checks that crate alone" crates/a/tests/new.rs
  scope "crates: a d" "a test-only change and a leaf change check both crates" crates/a/tests/new.rs crates/d/src/lib.rs
  scope "crates: hive-hand" "a crate is named by its package, not its folder" crates/hand/src/main.rs
  scope "full: every crate depends on what changed" "a change everything depends on runs the full check" crates/core/src/lib.rs
  scope "full: Cargo.toml is outside crates/" "a workspace manifest change runs the full check" Cargo.toml
  scope "full: scripts/x.sh is outside crates/" "a script change runs the full check" crates/d/src/lib.rs scripts/x.sh
  scope "full: crates/new has no Cargo.toml" "a file in a folder that is no crate runs the full check" crates/new/src/lib.rs
  scope "full: nothing changed since *" "an unchanged tree runs the full check"
  scope_env="HIVE_VERIFY_UNSET=1"
  scope "full: no HIVE_VERIFY_BASE" "without HIVE_VERIFY_BASE (the landing, the run's check) it is the full check" crates/d/src/lib.rs
  scope_env="HIVE_VERIFY_BASE=0000000000000000000000000000000000000000"
  scope "full: HIVE_VERIFY_BASE=* is not a commit of this tree" "a base this tree lacks runs the full check" crates/d/src/lib.rs
  scope_env=
  # Staged and committed changes count as well as unstaged ones.
  git -C "$ws" checkout -q -f "$base" 2> /dev/null
  echo '// staged' >> "$ws/crates/d/src/lib.rs"
  git -C "$ws" add -A
  git -C "$ws" commit -q -m one 2> /dev/null
  echo '// later' >> "$ws/crates/b/src/lib.rs"
  git -C "$ws" add -A
  said=$(cd "$ws" && HIVE_VERIFY_BASE=$base "$test_shell" "$script" --scope 2> /dev/null)
  if [ "$said" = "crates: b d" ]; then
    passed=$((passed + 1))
    printf 'ok    %s\n' "committed and staged changes since the base both count"
  else
    failed=$((failed + 1))
    printf 'FAIL  %s\n        said %s, want %s\n' "committed and staged changes since the base both count" "$said" "crates: b d"
  fi
fi

printf '\n%s passed, %s failed (the script ran under %s)\n' "$passed" "$failed" "$test_shell"
if [ "$failed" -ne 0 ]; then
  exit 1
fi
