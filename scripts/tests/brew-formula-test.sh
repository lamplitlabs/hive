#!/bin/sh
# Tests scripts/brew-formula.sh against a SHA256SUMS made up on the spot. It
# needs no network and no Homebrew.
#
#   sh scripts/tests/brew-formula-test.sh
#   HIVE_TEST_SHELL=/bin/dash sh scripts/tests/brew-formula-test.sh    the shell the script runs under (default: sh)
#
# It checks what the script writes against the three things that must agree
# with it: the archive names .github/workflows/release.yml builds, the marker
# crates/hive-hand-core/src/managed.rs reads, and Homebrew's rules for a
# formula that can be checked without Homebrew (`brew style` and `brew audit`
# are run by hand, docs/release.md says how).
#
# Everything the test writes is under one temporary directory, removed at the
# end. It prints one line a case and a count, and exits 1 when a case failed.
set -u

here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd) || exit 2
script="$here/../brew-formula.sh"
workflow="$here/../../.github/workflows/release.yml"
managed="$here/../../crates/hive-hand-core/src/managed.rs"
for file in "$script" "$workflow" "$managed"; do
  if [ ! -f "$file" ]; then
    echo "brew-formula-test: no file at $file" >&2
    exit 2
  fi
done

has() {
  command -v "$1" > /dev/null 2>&1
}

test_shell=${HIVE_TEST_SHELL:-sh}
case "$test_shell" in
  /*) ;;
  *)
    if ! has "$test_shell"; then
      echo "brew-formula-test: no shell named $test_shell" >&2
      exit 2
    fi
    test_shell=$(command -v "$test_shell")
    ;;
esac

work=$(mktemp -d "${TMPDIR:-/tmp}/hive-brew-formula-test.XXXXXX") || exit 2
trap 'rm -rf "$work"' EXIT
trap 'exit 1' HUP INT TERM

passed=0
failed=0
skipped=0

pass() {
  passed=$((passed + 1))
  printf 'ok    %s\n' "$1"
}

fail() {
  failed=$((failed + 1))
  printf 'FAIL  %s\n' "$1"
  if [ -n "${2:-}" ]; then
    printf '%s\n' "$2" | sed 's/^/        /'
  fi
}

skip() {
  skipped=$((skipped + 1))
  printf 'skip  %s (%s)\n' "$1" "$2"
}

# Runs the script with the given options; its output goes to $work/said and
# its exit status to $status.
status=0
run() {
  "$test_shell" "$script" "$@" > "$work/said" 2>&1
  status=$?
}

tag=v0.2.0

# A digest for the Nth archive: 64 hex digits, a different one each.
digest() {
  printf '%064x' "$((0xabc000 + $1))"
}

# Writes a whole SHA256SUMS for $tag into $1, in the order sha256sum gives
# (sorted by name), and the same pairs as "NAME DIGEST" into $1.pairs.
whole_sums() {
  : > "$1"
  : > "$1.pairs"
  n=0
  for kind in hand hive; do
    for target in aarch64-apple-darwin aarch64-unknown-linux-gnu x86_64-apple-darwin x86_64-unknown-linux-gnu; do
      n=$((n + 1))
      printf '%s  %s-%s-%s.tar.gz\n' "$(digest "$n")" "$kind" "$tag" "$target" >> "$1"
      printf '%s-%s-%s.tar.gz %s\n' "$kind" "$tag" "$target" "$(digest "$n")" >> "$1.pairs"
    done
  done
}

# The "NAME DIGEST" pairs a formula holds, one a line: each url with the
# sha256 on the line after it.
pairs_in() {
  awk '
    /^ *url "/ { url = $2; gsub(/"/, "", url); n = split(url, part, "/"); name = part[n]; next }
    /^ *sha256 "/ { sum = $2; gsub(/"/, "", sum); print name, sum; name = "" }
  ' "$1"
}

sums="$work/SHA256SUMS"
whole_sums "$sums"
out="$work/out"
hive="$out/hivefleet.rb"
hand="$out/hivefleet-hand.rb"

run --tag "$tag" --sums "$sums" --out "$out"
if [ "$status" -eq 0 ] && [ -s "$hive" ] && [ -s "$hand" ]; then
  pass "a whole release gives both formulae"
else
  fail "a whole release gives both formulae" "exit $status: $(cat "$work/said")"
  echo "brew-formula-test: nothing to check further" >&2
  exit 1
fi

if [ "$(find "$out" -type f | wc -l | tr -d ' ')" -eq 2 ]; then
  pass "and nothing else beside them"
else
  fail "and nothing else beside them" "$(find "$out" -type f)"
fi

for kind in hive hand; do
  if [ "$kind" = hive ]; then
    file=$hive
  else
    file=$hand
  fi
  want=$(grep "^$kind-" "$sums.pairs" | sort)
  got=$(pairs_in "$file" | sort)
  if [ -n "$want" ] && [ "$got" = "$want" ]; then
    pass "$(basename "$file") names its four archives, each with its own digest"
  else
    fail "$(basename "$file") names its four archives, each with its own digest" "want:
$want
got:
$got"
  fi
done

# Every archive is under the release's address, in an order Homebrew's
# on_macos/on_linux and on_arm/on_intel blocks expect.
order=$(awk '/^ *(on_macos|on_linux|on_arm|on_intel) do/ { printf "%s ", $1 } /^ *url "/ { n = split($2, part, "/"); gsub(/"/, "", part[n]); printf "%s ", part[n] }' "$hive")
want_order="on_macos on_arm hive-$tag-aarch64-apple-darwin.tar.gz on_intel hive-$tag-x86_64-apple-darwin.tar.gz on_linux on_arm hive-$tag-aarch64-unknown-linux-gnu.tar.gz on_intel hive-$tag-x86_64-unknown-linux-gnu.tar.gz "
if [ "$order" = "$want_order" ]; then
  pass "each archive sits in the block of its own system and processor"
else
  fail "each archive sits in the block of its own system and processor" "$order"
fi

if grep -q "url \"https://github.com/lamplitlabs/hive/releases/download/$tag/hive-$tag-aarch64-apple-darwin.tar.gz\"" "$hive"; then
  pass "the downloads are the release's own, on GitHub"
else
  fail "the downloads are the release's own, on GitHub" "$(grep 'url "' "$hive")"
fi

# The names the release workflow checks before it publishes are the names
# the formulae download: a renamed archive must fail here.
# shellcheck disable=SC2016 # $TAG is the workflow's own text, looked for as it is written there
built=$(grep -oE '"(hive|hand)-\$TAG-[a-z0-9_-]+\.tar\.gz"' "$workflow" | tr -d '"' | sed "s/\$TAG/$tag/" | sort -u)
named=$({ pairs_in "$hive"; pairs_in "$hand"; } | awk '{ print $1 }' | sort -u)
if [ "$(printf '%s\n' "$built" | wc -l | tr -d ' ')" -eq 8 ] && [ "$built" = "$named" ]; then
  pass "the archives are the eight release.yml publishes"
else
  fail "the archives are the eight release.yml publishes" "release.yml:
$built
formulae:
$named"
fi
for target in aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu; do
  if ! printf '%s\n' "$built" | grep -q -- "-$target.tar.gz"; then
    fail "release.yml builds $target"
  fi
done

# The marker: its name and its three keys are the ones hive reads.
marker_ok=1
grep -q 'pub const SUFFIX: &str = ".managed";' "$managed" || marker_ok=0
for key in by upgrade path; do
  grep -q "\"$key\"" "$managed" || marker_ok=0
  grep -q "^      $key=" "$hive" || marker_ok=0
  grep -q "^      $key=" "$hand" || marker_ok=0
done
grep -q '(libexec/"hive.managed").write' "$hive" || marker_ok=0
grep -q '(libexec/"hive-hand.managed").write' "$hand" || marker_ok=0
if [ "$marker_ok" -eq 1 ]; then
  pass "the marker is the one managed.rs reads: <program>.managed with by, upgrade and path"
else
  fail "the marker is the one managed.rs reads: <program>.managed with by, upgrade and path" "$(grep -n 'managed\|^      [a-z]*=' "$hive" "$hand")"
fi

if grep -q '^      upgrade=brew upgrade hivefleet$' "$hive" && grep -q '^      upgrade=brew upgrade hivefleet-hand$' "$hand" &&
  grep -q '^class Hivefleet < Formula$' "$hive" && grep -q '^class HivefleetHand < Formula$' "$hand"; then
  pass "each marker's upgrade command names its own formula"
else
  fail "each marker's upgrade command names its own formula" "$(grep -n 'upgrade=\|^class' "$hive" "$hand")"
fi

# opt_bin is the one path Homebrew keeps the same from version to version.
if grep -q '^      path=#{opt_bin}/hive$' "$hive" && grep -q '^      path=#{opt_bin}/hive-hand$' "$hand"; then
  pass "the path a marker gives is the one that outlives an upgrade"
else
  fail "the path a marker gives is the one that outlives an upgrade" "$(grep -n 'path=' "$hive" "$hand")"
fi

if grep -q '^    libexec.install "hive"$' "$hive" && grep -q '^    bin.install_symlink libexec/"hive"$' "$hive" &&
  grep -q '^    libexec.install "hive-hand"$' "$hand" && grep -q '^    bin.install_symlink libexec/"hive-hand"$' "$hand"; then
  pass "the program is in libexec, beside its marker, with a link in bin"
else
  fail "the program is in libexec, beside its marker, with a link in bin" "$(grep -n 'install' "$hive" "$hand")"
fi

if grep -q '^  conflicts_with "hive", because: ' "$hive" && ! grep -q 'conflicts_with' "$hand"; then
  pass "hivefleet says it cannot sit beside Homebrew's own hive"
else
  fail "hivefleet says it cannot sit beside Homebrew's own hive" "$(grep -n 'conflicts_with' "$hive" "$hand")"
fi

# What `brew audit` asks of a description: at most 80 characters, no full
# stop at the end, no article at the start, and not the formula's own name.
desc_ok=1
for file in "$hive" "$hand"; do
  desc=$(sed -n 's/^  desc "\(.*\)"$/\1/p' "$file")
  name=$(basename "$file" .rb)
  if [ -z "$desc" ] || [ "${#desc}" -gt 80 ]; then
    desc_ok=0
  fi
  case $desc in
    *. | "A "* | "An "* | "The "* | *"$name"*) desc_ok=0 ;;
  esac
done
if [ "$desc_ok" -eq 1 ]; then
  pass "the descriptions keep to Homebrew's rules"
else
  fail "the descriptions keep to Homebrew's rules" "$(grep -n '^  desc' "$hive" "$hand")"
fi

# Homebrew reads the version out of the archive's name; a version line as
# well is what `brew audit` calls redundant.
if ! grep -q '^  version ' "$hive" "$hand" && grep -q '^  license "MIT"$' "$hive" && grep -q '^  homepage "https://hivefleet.dev"$' "$hand"; then
  pass "no version line, and the licence and the site are named"
else
  fail "no version line, and the licence and the site are named" "$(grep -n '^  version\|^  license\|^  homepage' "$hive" "$hand")"
fi

if grep -q 'assert_match "v#{version}", shell_output("#{bin}/hive version")' "$hive" &&
  grep -q 'assert_match version.to_s, shell_output("#{bin}/hive-hand --version")' "$hand"; then
  pass "each formula's test runs the program and looks for the release's version"
else
  fail "each formula's test runs the program and looks for the release's version" "$(grep -n 'assert' "$hive" "$hand")"
fi

if has ruby; then
  if ruby -c "$hive" > "$work/ruby" 2>&1 && ruby -c "$hand" >> "$work/ruby" 2>&1; then
    pass "ruby reads both files"
  else
    fail "ruby reads both files" "$(cat "$work/ruby")"
  fi
else
  skip "ruby reads both files" "no ruby on this machine"
fi

run --tag "$tag" --sums "$sums" --out "$work/again"
if [ "$status" -eq 0 ] && cmp -s "$hive" "$work/again/hivefleet.rb" && cmp -s "$hand" "$work/again/hivefleet-hand.rb"; then
  pass "the same release gives the same files"
else
  fail "the same release gives the same files" "exit $status"
fi

run --tag "$tag" --sums "$sums" --out "$work/mirror" --base "file://$work/releases/download/"
if [ "$status" -eq 0 ] && [ "$(grep -c "url \"file://$work/releases/download/$tag/hive-$tag-" "$work/mirror/hivefleet.rb")" -eq 4 ] &&
  [ "$(grep -c "url \"file://$work/releases/download/$tag/hand-$tag-" "$work/mirror/hivefleet-hand.rb")" -eq 4 ]; then
  pass "--base moves all eight downloads, and a slash at its end is dropped"
else
  fail "--base moves all eight downloads, and a slash at its end is dropped" "exit $status: $(grep 'url "' "$work/mirror/hivefleet.rb" 2> /dev/null)"
fi

# sha256sum in binary mode puts a * before the name.
sed 's/  \(h[a-z]*-\)/ *\1/' "$sums" > "$work/binary-sums"
run --tag "$tag" --sums "$work/binary-sums" --out "$work/binary"
if [ "$status" -eq 0 ] && cmp -s "$hive" "$work/binary/hivefleet.rb" && grep -q ' \*hive-' "$work/binary-sums"; then
  pass "a SHA256SUMS written in binary mode reads the same"
else
  fail "a SHA256SUMS written in binary mode reads the same" "exit $status: $(cat "$work/said")"
fi

# A run that must fail: NAME, the words its message must hold, the exit
# status, then the script's options. It may leave no formula behind.
refused() {
  name=$1
  words=$2
  want_status=$3
  shift 3
  rm -rf "$work/refused"
  run "$@" --out "$work/refused"
  left=$(find "$work/refused" -type f 2> /dev/null | wc -l | tr -d ' ')
  if [ "$status" -eq "$want_status" ] && grep -q -- "$words" "$work/said" && [ "$left" -eq 0 ]; then
    pass "$name"
  else
    fail "$name" "exit $status (want $want_status), $left file(s) left, said: $(cat "$work/said")"
  fi
}

grep -v "hand-$tag-x86_64-unknown-linux-gnu" "$sums" > "$work/short-sums"
refused "a release that lacks an archive gives no formula at all" "lists no hand-$tag-x86_64-unknown-linux-gnu.tar.gz" 1 --tag "$tag" --sums "$work/short-sums"

sed "s/^[0-9a-f]*  hive-$tag-aarch64-apple-darwin/abc123  hive-$tag-aarch64-apple-darwin/" "$sums" > "$work/bad-sums"
refused "a digest that is not a SHA-256 is refused" "is not a SHA-256" 1 --tag "$tag" --sums "$work/bad-sums"

{
  cat "$sums"
  printf '%s  hive-%s-x86_64-apple-darwin.tar.gz\n' "$(digest 99)" "$tag"
} > "$work/twice-sums"
refused "an archive listed twice is refused" "more than once" 1 --tag "$tag" --sums "$work/twice-sums"

refused "the checksums of another release are refused" "lists no hive-v0.3.0-" 1 --tag v0.3.0 --sums "$sums"
refused "a tag that is not vX.Y.Z is refused" "is not a release tag" 1 --tag 0.2.0 --sums "$sums"
refused "a tag with more behind it is refused" "is not a release tag" 1 --tag "v0.2.0\"; system 'id'" --sums "$sums"
refused "a checksums file that is not there is refused" "no checksums file" 1 --tag "$tag" --sums "$work/nothing"
refused "an address that could end the formula's string is refused" "cannot be empty or hold" 1 --tag "$tag" --sums "$sums" --base 'https://x.test/"; system("id"); "'
refused "an unknown option is refused" "unknown option --force" 2 --tag "$tag" --sums "$sums" --force

rm -rf "$work/refused"
run --sums "$sums" --out "$work/refused" --tag
if [ "$status" -eq 2 ] && grep -q -- "--tag needs a value" "$work/said" && [ ! -e "$work/refused" ]; then
  pass "an option without its value is refused"
else
  fail "an option without its value is refused" "exit $status: $(cat "$work/said")"
fi

run --sums "$sums" --out "$work/refused"
if [ "$status" -eq 2 ] && grep -q -- "--tag, --sums and --out are all needed" "$work/said"; then
  pass "a missing option is named"
else
  fail "a missing option is named" "exit $status: $(cat "$work/said")"
fi

# A failed run for a later release leaves the formulae of this one alone.
cp "$hive" "$work/before-hive"
cp "$hand" "$work/before-hand"
run --tag v0.3.0 --sums "$sums" --out "$out"
if [ "$status" -ne 0 ] && cmp -s "$hive" "$work/before-hive" && cmp -s "$hand" "$work/before-hand" &&
  [ "$(find "$out" -type f | wc -l | tr -d ' ')" -eq 2 ]; then
  pass "a run that fails leaves the formulae that were there as they were"
else
  fail "a run that fails leaves the formulae that were there as they were" "exit $status: $(find "$out" -type f)"
fi

run --help
if [ "$status" -eq 0 ] && grep -q -- "--sums FILE" "$work/said" && grep -q -- "--base URL" "$work/said"; then
  pass "--help prints the options"
else
  fail "--help prints the options" "exit $status"
fi

printf '\n%s passed, %s failed, %s skipped (the script ran under %s)\n' "$passed" "$failed" "$skipped" "$test_shell"
if [ "$failed" -ne 0 ]; then
  exit 1
fi
