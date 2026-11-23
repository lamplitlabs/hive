#!/bin/sh
# Tests how many chunks scripts/verify-check.sh runs at once, through its
# `--pool` option: it prints the pool and exits before it builds anything,
# so the test needs no cargo and takes no time.
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

printf '\n%s passed, %s failed (the script ran under %s)\n' "$passed" "$failed" "$test_shell"
if [ "$failed" -ne 0 ]; then
  exit 1
fi
