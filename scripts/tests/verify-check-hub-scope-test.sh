#!/bin/sh
# Tests that the pre-commit gate (scripts/verify-check.sh --scope) reads a
# change under web/hub, the console, as hive-hub code: hive-hub's build.rs
# builds web/hub into the export it embeds, so a console-only commit checks
# hive-hub and what depends on it (hive), not the whole workspace, and fits
# under the 40-minute commit limit on a 2-CPU hand. --scope prints and exits
# before anything is built, so the test takes seconds; it needs cargo and
# git on PATH.
#
#   sh scripts/tests/verify-check-hub-scope-test.sh
#   HIVE_TEST_SHELL=/bin/dash sh scripts/tests/verify-check-hub-scope-test.sh
#
# It prints one line a case and a count, and exits 1 when a case failed.
set -u

here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd) || exit 2
script="$here/../verify-check.sh"
if [ ! -f "$script" ]; then
  echo "verify-check-hub-scope-test: no file at $script" >&2
  exit 2
fi

test_shell=${HIVE_TEST_SHELL:-sh}
case "$test_shell" in
  /*) ;;
  *)
    if ! command -v "$test_shell" > /dev/null 2>&1; then
      echo "verify-check-hub-scope-test: no shell named $test_shell" >&2
      exit 2
    fi
    test_shell=$(command -v "$test_shell")
    ;;
esac
if ! command -v cargo > /dev/null 2>&1 || ! command -v git > /dev/null 2>&1; then
  echo "verify-check-hub-scope-test: needs cargo and git on PATH" >&2
  exit 1
fi

passed=0
failed=0
scratch=$(mktemp -d "${TMPDIR:-/tmp}/verify-check-hub-scope-test.XXXXXX") || exit 2
trap 'rm -rf "$scratch"' EXIT

# A scratch workspace shaped like this one, path-only so cargo tree resolves
# it offline: core under everything, hive-hub on core, the hive binary on
# hive-hub, hive-store and d on core.
# crate WS DIR NAME DEPS [bin]: a package under WS/crates/DIR.
crate() {
  mkdir -p "$1/crates/$2/src"
  {
    printf '[package]\nname = "%s"\nversion = "0.1.0"\nedition = "2021"\n' "$3"
    printf '\n[dependencies]\n'
    for dep in $4; do printf '%s = { path = "../%s" }\n' "$dep" "$dep"; done
  } > "$1/crates/$2/Cargo.toml"
  if [ "${5:-}" = bin ]; then
    echo 'fn main() {}' > "$1/crates/$2/src/main.rs"
  else
    echo '' > "$1/crates/$2/src/lib.rs"
  fi
}

# workspace WS WITH_HUB: a committed workspace, with or without hive-hub.
workspace() {
  mkdir -p "$1/web/hub/src" "$1/web/hub/mock/api" "$1/web/site"
  printf '[workspace]\nresolver = "2"\nmembers = ["crates/*"]\n' > "$1/Cargo.toml"
  crate "$1" core core ""
  crate "$1" hive-store hive-store "core"
  crate "$1" d d "core"
  if [ "$2" = yes ]; then
    crate "$1" hive-hub hive-hub "core"
    crate "$1" hive hive "hive-hub" bin
  else
    crate "$1" hive hive "core" bin
  fi
  echo 'export {}' > "$1/web/hub/src/page.tsx"
  echo '{}' > "$1/web/hub/mock/api/settings.json"
  echo '{}' > "$1/web/hub/package.json"
  echo '{}' > "$1/web/site/package.json"
  (
    cd "$1" &&
      git init -q -b main &&
      git config user.email t@example.com &&
      git config user.name t &&
      git config commit.gpgsign false &&
      cargo tree --workspace --offline > /dev/null 2>&1 &&
      git add -A &&
      git commit -q -m base
  ) > /dev/null 2>&1 || echo "verify-check-hub-scope-test: could not make the scratch workspace $1" >&2
}

workspace "$scratch/ws" yes
workspace "$scratch/nohub" no

# scope WS WANT NAME EDIT...: with WS's working tree EDITed from its base
# commit (one file per word: appended to, or created), --scope prints WANT.
scope() {
  ws=$1
  want=$2
  name=$3
  shift 3
  base=$(git -C "$ws" rev-parse HEAD 2> /dev/null)
  git -C "$ws" checkout -q -f "$base" 2> /dev/null
  git -C "$ws" clean -qfd 2> /dev/null
  for f in "$@"; do
    case "$f" in
      */*) mkdir -p "$ws/${f%/*}" ;;
    esac
    echo '// changed' >> "$ws/$f"
  done
  said=$(cd "$ws" && env HIVE_VERIFY_BASE="$base" "$test_shell" "$script" --scope 2> /dev/null)
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

ws=$scratch/ws
scope "$ws" "crates: hive hive-hub" "a console source change checks hive-hub and the hive binary that embeds it" web/hub/src/page.tsx
scope "$ws" "crates: hive hive-hub" "a new console file checks hive-hub and hive" web/hub/src/components/new.tsx
scope "$ws" "crates: hive hive-hub" "a console package change checks hive-hub and hive" web/hub/package.json
scope "$ws" "crates: hive hive-hub hive-store" "a console mock change also checks hive-store, whose settings test reads the mock" web/hub/mock/api/settings.json
scope "$ws" "crates: d hive hive-hub" "a console change and a crate change check both" web/hub/src/page.tsx crates/d/src/lib.rs
scope "$ws" "full: web/site/package.json is outside crates/" "the marketing site is not hive-hub: it runs the full check" web/hub/src/page.tsx web/site/package.json
scope "$ws" "full: scripts/x.sh is outside crates/" "a console change beside a script change runs the full check" web/hub/src/page.tsx scripts/x.sh
scope "$scratch/nohub" "full: web/hub/src/page.tsx is outside crates/ and this tree has no crates/hive-hub" "a tree without hive-hub runs the full check on a console change" web/hub/src/page.tsx

printf '\n%s passed, %s failed (the script ran under %s)\n' "$passed" "$failed" "$test_shell"
if [ "$failed" -ne 0 ]; then
  exit 1
fi
