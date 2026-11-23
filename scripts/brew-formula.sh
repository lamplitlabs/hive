#!/bin/sh
# Writes the Homebrew formulae of a Hive release.
#
#   sh scripts/brew-formula.sh --tag v0.2.0 --sums SHA256SUMS --out Formula
#
# A formula is the file Homebrew installs a program from. This script writes
# two of them, from the archive names and the digests in a release's
# SHA256SUMS:
#
#   hivefleet.rb        installs hive        brew install lamplitlabs/tap/hivefleet
#   hivefleet-hand.rb   installs hive-hand   brew install lamplitlabs/tap/hivefleet-hand
#
# The formula is not named hive: Homebrew's own list has a `hive` already
# (Apache Hive), and a second one of that name could not be told from it.
#
# Each formula names the four archives of the release (macOS and Linux, on
# arm64 and x86_64) with their SHA-256, and Homebrew checks the download
# against that digest. It names no version: Homebrew reads 0.2.0 out of an
# archive's name, and `brew audit` calls a second mention redundant. It
# builds nothing. It puts the program in the
# formula's libexec with a marker beside it, `<program>.managed`, and a link
# in bin. The marker is what tells hive that Homebrew owns the file
# (crates/hive-hand-core/src/managed.rs): `hive update` then names
# `brew upgrade` and writes nothing, and Hive's services start the program
# by the path that stays the same from one version to the next.
#
# .github/workflows/release.yml runs this after it publishes a release and
# pushes the two files to the tap repository (docs/release.md, "Homebrew").
# scripts/tests/brew-formula-test.sh is its test, and needs no network.
set -eu

BASE=https://github.com/lamplitlabs/hive/releases/download

usage() {
  cat << 'EOF'
Write the Homebrew formulae of a Hive release.

  sh scripts/brew-formula.sh --tag TAG --sums FILE --out DIR [--base URL]

Options:
  --tag TAG     the release (v0.2.0)
  --sums FILE   that release's SHA256SUMS
  --out DIR     where hivefleet.rb and hivefleet-hand.rb are written; made
                when it is not there
  --base URL    where the releases are downloaded from, in place of
                https://github.com/lamplitlabs/hive/releases/download
                (an archive is at URL/TAG/NAME)
  --help        print this

Both files are written, or neither: a release that lacks an archive or a
digest stops the script before it writes anything.
EOF
}

fail() {
  printf 'brew-formula.sh: %s\n' "$*" >&2
  exit 1
}

bad_usage() {
  printf 'brew-formula.sh: %s\n' "$*" >&2
  printf 'Try: sh scripts/brew-formula.sh --help\n' >&2
  exit 2
}

tag=""
sums=""
out=""
base=$BASE
while [ $# -gt 0 ]; do
  case $1 in
    --tag | --sums | --out | --base)
      if [ $# -lt 2 ]; then
        bad_usage "$1 needs a value"
      fi
      case $1 in
        --tag) tag=$2 ;;
        --sums) sums=$2 ;;
        --out) out=$2 ;;
        --base) base=$2 ;;
      esac
      shift 2
      ;;
    --help | -h)
      usage
      exit 0
      ;;
    *)
      bad_usage "unknown option $1"
      ;;
  esac
done

if [ -z "$tag" ] || [ -z "$sums" ] || [ -z "$out" ]; then
  bad_usage "--tag, --sums and --out are all needed"
fi
if ! printf '%s\n' "$tag" | grep -Eqx 'v[0-9]+\.[0-9]+\.[0-9]+'; then
  fail "$tag is not a release tag (vX.Y.Z)"
fi
if [ ! -f "$sums" ]; then
  fail "no checksums file at $sums"
fi
# The address goes into a Ruby string: nothing that could end it.
case $base in
  "" | *\"* | *\\* | *\#* | *" "*) fail "--base cannot be empty or hold a quote, a backslash, a # or a space" ;;
esac
base=${base%/}

# The digest SHA256SUMS lists for the archive NAME: one line, 64 hex digits.
# `sha256sum` writes "DIGEST  NAME", and "DIGEST *NAME" in binary mode.
digest_of() {
  found=$(awk -v name="$1" '{ file = $2; sub(/^\*/, "", file); if (file == name) print $1 }' "$sums")
  if [ -z "$found" ]; then
    fail "$sums lists no $1: the release is not whole, so no formula was written"
  fi
  if [ "$(printf '%s\n' "$found" | wc -l | tr -d ' ')" -ne 1 ]; then
    fail "$sums lists $1 more than once"
  fi
  if ! printf '%s\n' "$found" | grep -Eqx '[0-9a-f]{64}'; then
    fail "$sums gives $1 the digest $found, which is not a SHA-256"
  fi
  printf '%s\n' "$found"
}

# The four lines of one archive in a formula: KIND TARGET INDENT.
archive() {
  name="$1-$tag-$2.tar.gz"
  digest=$(digest_of "$name") || exit 1
  printf '%s  url "%s/%s/%s"\n' "$3" "$base" "$tag" "$name"
  printf '%s  sha256 "%s"\n' "$3" "$digest"
}

# The url and sha256 of every target: KIND.
archives() {
  printf '  on_macos do\n'
  printf '    on_arm do\n'
  archive "$1" aarch64-apple-darwin "    "
  printf '    end\n'
  printf '    on_intel do\n'
  archive "$1" x86_64-apple-darwin "    "
  printf '    end\n'
  printf '  end\n'
  printf '\n'
  printf '  on_linux do\n'
  printf '    on_arm do\n'
  archive "$1" aarch64-unknown-linux-gnu "    "
  printf '    end\n'
  printf '    on_intel do\n'
  archive "$1" x86_64-unknown-linux-gnu "    "
  printf '    end\n'
  printf '  end\n'
}

head_of() {
  printf '# Written by scripts/brew-formula.sh in https://github.com/lamplitlabs/hive\n'
  printf '# from the SHA256SUMS of the release %s. The next release writes this file\n' "$tag"
  printf '# again, so a change made here by hand is lost: change the script.\n'
}

# How each formula installs: PROGRAM FORMULA. The marker's three lines are
# the ones crates/hive-hand-core/src/managed.rs reads.
install_block() {
  printf '  def install\n'
  printf '    # The program goes in libexec with a marker beside it, and bin gets a\n'
  printf '    # link. The marker tells the program that Homebrew owns the file, and\n'
  printf '    # gives the path that names it from one version to the next.\n'
  printf '    libexec.install "%s"\n' "$1"
  printf '    (libexec/"%s.managed").write <<~EOS\n' "$1"
  printf '      by=Homebrew\n'
  printf '      upgrade=brew upgrade %s\n' "$2"
  printf '      path=#{opt_bin}/%s\n' "$1"
  printf '    EOS\n'
  printf '    bin.install_symlink libexec/"%s"\n' "$1"
  printf '  end\n'
}

hive_formula() {
  head_of
  cat << 'EOF'
class Hivefleet < Formula
  desc "Self-improving harness for coding agents that runs on your own machines"
  homepage "https://hivefleet.dev"
  license "MIT"

EOF
  archives hive
  cat << 'EOF'

  conflicts_with "hive", because: "both install a `hive` program"

EOF
  install_block hive hivefleet
  cat << 'EOF'

  def caveats
    <<~EOS
      Start the console:
        hive hub

      Upgrade with `brew upgrade hivefleet`. `hive update` does not replace
      a hive that Homebrew installed.
    EOS
  end

  test do
    assert_match "v#{version}", shell_output("#{bin}/hive version")
    assert_path_exists libexec/"hive.managed"
  end
end
EOF
}

hand_formula() {
  head_of
  cat << 'EOF'
class HivefleetHand < Formula
  desc "Small program a machine runs to take work from a Hive on another machine"
  homepage "https://hivefleet.dev"
  license "MIT"

EOF
  archives hand
  printf '\n'
  install_block hive-hand hivefleet-hand
  cat << 'EOF'

  def caveats
    <<~EOS
      Join your Hive with the line its console shows under Machines,
      Add a machine:
        hive-hand join --brain URL CODE

      Upgrade with `brew upgrade hivefleet-hand`.
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/hive-hand --version")
    assert_path_exists libexec/"hive-hand.managed"
  end
end
EOF
}

# Every archive is looked up before a file is written, so a release that
# lacks one leaves no formula behind, not even one of the two.
for target in aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu; do
  for kind in hive hand; do
    digest_of "$kind-$tag-$target.tar.gz" > /dev/null
  done
done

mkdir -p "$out" || fail "could not make $out"
staged_hive="$out/.hivefleet.rb.$$"
staged_hand="$out/.hivefleet-hand.rb.$$"
trap 'rm -f "$staged_hive" "$staged_hand"' EXIT
hive_formula > "$staged_hive" || fail "could not write in $out"
hand_formula > "$staged_hand" || fail "could not write in $out"
mv -f "$staged_hive" "$out/hivefleet.rb"
mv -f "$staged_hand" "$out/hivefleet-hand.rb"
printf 'brew-formula.sh: wrote %s and %s for %s\n' "$out/hivefleet.rb" "$out/hivefleet-hand.rb" "$tag"
