#!/bin/sh
# The Hive installer, for macOS and Linux.
#
#   curl -fsSL https://hivefleet.dev/install.sh | sh
#   curl -fsSL https://hivefleet.dev/install.sh | sh -s -- --hand
#
# It takes one archive and the checksums file of a release from GitHub,
# checks the archive's SHA-256 against that file, runs the program once to
# see that it starts on this machine, and puts it in a directory you own:
# hive in ~/.hive/bin, or with --hand hive-hand in ~/.hive-hand/bin. It
# never asks for sudo and never edits a shell profile. Run it again to
# upgrade. `sh install.sh --help` lists the options.
#
# docs/release.md says what a release carries. The names used here (the
# archives, SHA256SUMS, the four targets, the glibc the Linux programs are
# built for) are the ones .github/workflows/release.yml produces, and
# scripts/tests/install-test.sh checks the two files against each other.
#
# Every command sits in a function, and the one call that starts them,
# `main`, is the last thing in the file. The braces around the whole script
# make the shell read all of it before it runs any of it, so a download that
# is cut off at any byte runs nothing: not `main`, and not a stray word
# either, such as the `find` a cut inside `find_target` would leave.
{

info() {
  printf '%s\n' "$*"
}

warn() {
  printf '%s\n' "$*" >&2
}

fail() {
  warn "install.sh: $*"
  exit 1
}

has() {
  command -v "$1" > /dev/null 2>&1
}

# Printed line by line and not from a here-document: ksh93 never returns
# from one that a cut-off download leaves open.
usage() {
  info "Install Hive on macOS or Linux."
  info ""
  info "  curl -fsSL https://hivefleet.dev/install.sh | sh"
  info "  curl -fsSL https://hivefleet.dev/install.sh | sh -s -- [options]"
  info "  sh install.sh [options]"
  info ""
  info "Options:"
  info "  --hand          install hive-hand, the small program a machine runs to"
  info "                  take work from your Hive, and not hive"
  info "  --version TAG   install the release TAG (v0.2.0) and not the latest one"
  info "  --dir DIR       install into DIR"
  info "  --help          print this"
  info ""
  info "Where the program goes when you give no --dir:"
  info "  hive        ~/.hive/bin        (or \$HIVE_INSTALL_DIR)"
  info "  hive-hand   ~/.hive-hand/bin   (or \$HIVE_HAND_INSTALL_DIR)"
  info ""
  info "Other variables:"
  info "  HIVE_VERSION        the same as --version"
  info "  HIVE_RELEASE_BASE   where the releases are, in place of"
  info "                      https://github.com/lamplitlabs/hive/releases"
  info "  HIVE_INSTALL_OS, HIVE_INSTALL_ARCH"
  info "                      the system and the processor to install for, in place"
  info "                      of what uname says (the tests use them)"
  info ""
  info "The download is checked against the release's SHA256SUMS before anything is"
  info "installed. Nothing runs with sudo and no shell profile is changed."
}

# Removes what a run leaves behind: the download directory and a program
# written beside its destination but not moved into place. A run that
# failed also removes the directories it made, and ends with status 1.
on_exit() {
  status=$?
  if [ -n "$staged" ]; then
    rm -f "$staged"
  fi
  if [ -n "$tmp" ]; then
    rm -rf "$tmp"
  fi
  if [ "$status" -ne 0 ]; then
    # Deepest first. rmdir leaves a directory that holds anything.
    printf '%s' "$made_dirs" | while IFS= read -r made; do
      rmdir "$made" 2> /dev/null || true
    done
    exit 1
  fi
}

parse_args() {
  while [ $# -gt 0 ]; do
    case "$1" in
      --hand)
        program=hive-hand
        ;;
      --version)
        if [ $# -lt 2 ] || [ -z "$2" ]; then
          fail "--version needs a release tag, such as v0.2.0"
        fi
        version=$2
        shift
        ;;
      --version=*)
        version=${1#--version=}
        if [ -z "$version" ]; then
          fail "--version needs a release tag, such as v0.2.0"
        fi
        ;;
      --dir)
        if [ $# -lt 2 ] || [ -z "$2" ]; then
          fail "--dir needs a directory"
        fi
        dir=$2
        shift
        ;;
      --dir=*)
        dir=${1#--dir=}
        if [ -z "$dir" ]; then
          fail "--dir needs a directory"
        fi
        ;;
      -h | --help)
        usage
        exit 0
        ;;
      *)
        fail "unknown option '$1'. Run it with --help to see the options."
        ;;
    esac
    shift
  done
}

# The last lines of every "no release fits this machine" message, on stderr.
from_source() {
  warn ""
  if [ "$program" = hive ]; then
    warn "To build Hive from source instead (Rust 1.95+, Node.js 22+):"
    warn ""
    warn "  cargo install --git $REPO --locked --root ~/.hive hive"
  else
    warn "To build hive-hand from source instead (Rust 1.95+), in a clone of"
    warn "$REPO:"
    warn ""
    warn "  cargo build --release -p hive-hand    # gives target/release/hive-hand"
  fi
}

no_release() {
  warn "install.sh: there is no release of $program for this machine ($machine)."
  warn "Releases are built for macOS and Linux, on arm64 and x86_64."
  from_source
  exit 1
}

# Sets target, the Rust target triple the release archive is named for,
# and machine, what this system calls itself.
find_target() {
  os=${HIVE_INSTALL_OS:-}
  if [ -z "$os" ]; then
    os=$(uname -s) || fail "uname did not say which system this is"
  fi
  arch=${HIVE_INSTALL_ARCH:-}
  if [ -z "$arch" ]; then
    arch=$(uname -m) || fail "uname did not say which processor this is"
    # A shell that runs under Rosetta on Apple silicon is told x86_64.
    if [ "$os" = Darwin ] && [ "$arch" = x86_64 ] &&
      [ "$(sysctl -n hw.optional.arm64 2> /dev/null || true)" = 1 ]; then
      arch=arm64
    fi
  fi
  machine="$os $arch"

  case "$os" in
    Darwin | darwin | macos | macOS)
      os_part=apple-darwin
      ;;
    Linux | linux)
      os_part=unknown-linux-gnu
      ;;
    MINGW* | MSYS* | CYGWIN* | Windows* | windows*)
      fail "Hive does not run on Windows yet ($machine). A port is in progress."
      ;;
    *)
      no_release
      ;;
  esac
  case "$arch" in
    arm64 | aarch64)
      cpu=aarch64
      ;;
    x86_64 | amd64)
      cpu=x86_64
      ;;
    *)
      no_release
      ;;
  esac
  target="$cpu-$os_part"
}

# Prints the C library of this Linux: "glibc 2.36", "glibc", "musl", or
# nothing when it cannot tell.
linux_libc() {
  if has getconf; then
    says=$(getconf GNU_LIBC_VERSION 2> /dev/null || true)
    case "$says" in
      glibc\ *)
        info "$says"
        return 0
        ;;
    esac
  fi
  if has ldd; then
    says=$(ldd --version 2>&1 || true)
    case "$says" in
      *musl*)
        info musl
        return 0
        ;;
      *GLIBC* | *"GNU libc"* | *"GNU C Library"*)
        info glibc
        return 0
        ;;
    esac
  fi
  for loader in /lib/ld-musl-*.so.1; do
    if [ -e "$loader" ]; then
      info musl
      return 0
    fi
  done
  return 0
}

# The Linux archives hold 64-bit programs linked against glibc.
check_linux() {
  if has getconf && [ "$(getconf LONG_BIT 2> /dev/null || true)" = 32 ]; then
    warn "install.sh: this Linux runs 32-bit programs (getconf LONG_BIT says 32; $machine)."
    warn "Releases are built for 64-bit systems only."
    from_source
    exit 1
  fi
  libc=$(linux_libc)
  if [ "$libc" = musl ]; then
    warn "install.sh: this Linux uses musl as its C library, not glibc (Alpine does, for one)."
    warn "The release is built for glibc and does not run here."
    from_source
    exit 1
  fi
}

# fetch URL FILE: succeeds when FILE holds what URL serves.
fetch() {
  case "$fetcher" in
    curl)
      case "$1" in
        https://*)
          curl -fsSL --proto '=https' --tlsv1.2 --retry 3 -o "$2" "$1"
          ;;
        *)
          curl -fsSL --retry 3 -o "$2" "$1"
          ;;
      esac
      ;;
    wget)
      # GNU wget notes the sites it met in a file in HOME; where it has a
      # switch for that, the installer leaves no such file behind.
      wget -q ${wget_no_hsts:+"$wget_no_hsts"} -O "$2" "$1"
      ;;
  esac
}

# Prints the SHA-256 of FILE in lowercase hex.
sha256_of() {
  case "$hasher" in
    sha256sum)
      digest=$(sha256sum < "$1") || return 1
      ;;
    shasum)
      digest=$(shasum -a 256 < "$1") || return 1
      ;;
  esac
  info "${digest%% *}" | tr 'ABCDEF' 'abcdef'
}

# Reads a checksums file as sha256sum and shasum write it (64 hex digits,
# blanks, an optional *, the file name) and prints "digest name" for every
# line of that shape, the way `hive update` reads it: a directory in front
# of the name is dropped and any other line is skipped.
read_checksums() {
  cr=$(printf '\r')
  while read -r sum name || [ -n "$sum" ]; do
    name=${name%"$cr"}
    name=${name#\*}
    name=${name##*/}
    if [ -z "$name" ] || [ "${#sum}" -ne 64 ]; then
      continue
    fi
    case "$sum" in
      *[!0123456789abcdefABCDEF]*)
        continue
        ;;
    esac
    info "$sum $name"
  done < "$1"
}

# Prints the digest the release lists for the file NAME, in lowercase, or
# nothing when it lists none.
listed_digest() {
  while read -r sum name; do
    if [ "$name" = "$1" ]; then
      info "$sum" | tr 'ABCDEF' 'abcdef'
      return 0
    fi
  done < "$tmp/sums"
  return 0
}

# Prints the archives the release lists for KIND (hive or hand) on TARGET,
# one name a line.
listed_archives() {
  while read -r _ name; do
    case "$name" in
      "$1"-v[0-9]*-"$2".tar.gz)
        info "$name"
        ;;
    esac
  done < "$tmp/sums"
  return 0
}

# A tag goes into an address, so it may hold nothing but what a version does.
check_tag() {
  case "$1" in
    v[0-9]*) ;;
    *)
      fail "'$1' is not a release tag. A tag looks like v0.2.0."
      ;;
  esac
  case "$1" in
    *[!0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ._+-]*)
      fail "'$1' is not a release tag. A tag looks like v0.2.0."
      ;;
  esac
}

# Downloads the release's checksums file and reads it into $tmp/sums.
get_checksums() {
  if [ -n "$tag" ]; then
    sums_url="$base/download/$tag/$CHECKSUMS"
  else
    sums_url="$base/latest/download/$CHECKSUMS"
  fi
  if ! fetch "$sums_url" "$tmp/sums.raw"; then
    warn "install.sh: could not download $sums_url"
    if [ -n "$tag" ]; then
      warn "Is $tag a release of Hive? The releases are listed at $base"
    else
      warn "Is this machine online? The releases of Hive are listed at $base"
    fi
    exit 1
  fi
  read_checksums "$tmp/sums.raw" > "$tmp/sums" || fail "could not read $sums_url"
  if [ ! -s "$tmp/sums" ]; then
    fail "$sums_url lists no checksums, so no download could be checked. Nothing was installed."
  fi
}

# Sets tag from the latest release's checksums file: the release names
# itself in the archive it lists for this target.
find_latest_tag() {
  names=$(listed_archives "$kind" "$target")
  if [ -z "$names" ]; then
    # Name the release when one of its other archives does.
    latest="the latest release"
    for other in aarch64-apple-darwin x86_64-apple-darwin x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
      names=$(listed_archives hive "$other")
      case "$names" in
        "" | *"$NEWLINE"*) ;;
        *)
          latest=${names#hive-}
          latest="the latest release, ${latest%-"$other".tar.gz},"
          break
          ;;
      esac
    done
    warn "install.sh: $latest has no $program for this machine ($target)."
    from_source
    exit 1
  fi
  case "$names" in
    *"$NEWLINE"*)
      fail "the latest release lists more than one $program for $target. Say which release you want with --version TAG."
      ;;
  esac
  tag=${names#"$kind"-}
  tag=${tag%-"$target".tar.gz}
  check_tag "$tag"
}

# Downloads the archive, checks it against the release's checksums and
# unpacks it; sets found, the program inside it. Nothing is written outside
# the download directory.
get_program() {
  archive="$kind-$tag-$target.tar.gz"
  want=$(listed_digest "$archive")
  if [ -z "$want" ]; then
    warn "install.sh: release $tag has no $program for this machine ($target):"
    warn "its $CHECKSUMS lists no $archive."
    from_source
    exit 1
  fi

  info "Downloading $program $tag for $target"
  if ! fetch "$base/download/$tag/$archive" "$tmp/archive.tar.gz"; then
    fail "could not download $base/download/$tag/$archive"
  fi

  got=$(sha256_of "$tmp/archive.tar.gz") || fail "could not work out the SHA-256 of the download with $hasher. Nothing was installed."
  if [ "$got" != "$want" ]; then
    warn "install.sh: $archive does not match its SHA-256 in $CHECKSUMS. Nothing was installed."
    warn "  expected $want"
    warn "  got      $got"
    warn "The download may be incomplete, or it was changed on the way. Run this again."
    exit 1
  fi
  info "The archive matches its SHA-256 in $CHECKSUMS"

  mkdir "$tmp/unpacked" || fail "could not write in $tmp"
  tar -xzf "$tmp/archive.tar.gz" -C "$tmp/unpacked" || fail "could not unpack $archive"
  # The program is at the top of the archive or one folder down.
  found="$tmp/unpacked/$program"
  if [ ! -f "$found" ]; then
    found=""
    for inside in "$tmp/unpacked"/*/"$program"; do
      if [ -f "$inside" ]; then
        found=$inside
        break
      fi
    done
  fi
  if [ -z "$found" ]; then
    fail "$archive holds no file named $program. Nothing was installed."
  fi
}

# On macOS, signs hive the way `hive update` and `hive service install` do
# (crates/hive-runner/src/sign.rs). macOS ties its answer to "may hive
# reach this folder?" to the requirement in the program's signature, and
# this requirement is one every build meets, so the question is not asked
# again after an upgrade. No certificate and no password are involved.
sign_for_macos() {
  if [ "$program" != hive ] || [ "$os_part" != apple-darwin ] || ! has codesign; then
    return 0
  fi
  if ! codesign --force --sign - --identifier "$SIGN_AS" \
    --requirements "=designated => identifier \"$SIGN_AS\"" "$staged" > /dev/null 2>&1; then
    warn "install.sh: could not sign hive, so macOS may ask about your folders again. hive service install signs it too."
  fi
}

# Writes the program beside its destination, runs it once, and only then
# moves it into place: an older program stays whole until the new one is
# known to start, and is replaced in one step.
install_program() {
  # The directories that are not there yet, deepest first, for on_exit.
  missing=$dir
  while [ -n "$missing" ] && [ ! -d "$missing" ]; do
    made_dirs="$made_dirs$missing$NEWLINE"
    case "$missing" in
      */*)
        missing=${missing%/*}
        ;;
      *)
        break
        ;;
    esac
  done
  if ! mkdir -p "$dir" 2> /dev/null; then
    fail "could not make the directory $dir. Choose one you own with --dir DIR."
  fi
  dir=$(CDPATH='' cd -- "$dir" && pwd) || fail "could not open the directory $dir"
  dest="$dir/$program"
  if [ -d "$dest" ]; then
    fail "$dest is a directory. Move it away, or choose another place with --dir DIR."
  fi
  replaced=no
  if [ -e "$dest" ]; then
    replaced=yes
  fi

  staged=$(mktemp "$dir/.$program.XXXXXX" 2> /dev/null) || {
    staged=""
    fail "could not write in $dir. Choose a directory you own with --dir DIR."
  }
  cp "$found" "$staged" || fail "could not write $staged"
  chmod 755 "$staged" || fail "could not make $staged a program"
  sign_for_macos

  mkdir "$tmp/home" || fail "could not write in $tmp"
  if ! HIVE_HOME="$tmp/home" "$staged" "$try" < /dev/null > /dev/null 2> "$tmp/try.err"; then
    warn "install.sh: $program $tag does not start on this machine ($machine), so nothing was installed."
    if [ -s "$tmp/try.err" ]; then
      warn "It said:"
      shown=0
      while read -r line && [ "$shown" -lt 3 ]; do
        warn "  $line"
        shown=$((shown + 1))
      done < "$tmp/try.err"
    fi
    case "$libc" in
      glibc\ *)
        warn "This machine has $libc. The Linux release is built to run on glibc $GLIBC_FLOOR and newer."
        ;;
    esac
    from_source
    exit 1
  fi

  mv -f "$staged" "$dest" || fail "could not put $program at $dest"
  staged=""
}

report() {
  if [ "$replaced" = yes ]; then
    info "Installed $program $tag at $dest, in place of the one that was there."
    if [ "$program" = hive ]; then
      info "Hive's services (hive service install) restart on it by themselves,"
      info "once their runs in flight have ended."
    fi
  else
    info "Installed $program $tag at $dest"
  fi
  info ""

  case ":${PATH:-}:" in
    *":$dir:"* | *":$dir/:"*)
      info "$dir is on your PATH."
      next="Next,"
      ;;
    *)
      # Shown with $HOME when it is under it, so the line fits a profile.
      profile_dir=$dir
      if [ -n "${HOME:-}" ]; then
        case "$dir" in
          "$HOME"/*)
            profile_dir="\$HOME${dir#"$HOME"}"
            ;;
        esac
      fi
      info "$dir is not on your PATH. Add this line to your shell profile,"
      info "then open a new terminal:"
      info ""
      info "  export PATH=\"$profile_dir:\$PATH\""
      next="Then"
      ;;
  esac
  info ""

  if [ "$program" = hive ]; then
    # hive hub needs a store and does not make one; hive init does.
    if [ -f "${HIVE_HOME:-${HOME:-}/.hive}/hive.sqlite" ]; then
      info "$next start the console:"
      info ""
      info "  hive hub"
    else
      info "$next make the store, once, and start the console:"
      info ""
      info "  hive init --store-only"
      info "  hive hub"
    fi
    info ""
    info "and open http://127.0.0.1:8765 in your browser."
  else
    info "$next join this machine to your Hive. In the console, open Machines,"
    info "choose Add a machine, then Run one command on it, then Get a join code."
    info "It shows the line to run here, of this shape:"
    info ""
    info "  hive-hand join --brain URL CODE"
    info ""
    info "To run agents this machine also needs hive itself and a model login:"
    info "run this installer once more without --hand, then hive harness login."
  fi
}

main() {
  set -eu
  LC_ALL=C
  export LC_ALL

  REPO="https://github.com/lamplitlabs/hive"
  CHECKSUMS="SHA256SUMS"
  # The newest glibc a Linux program in a release may ask for; release.yml
  # checks the same number (GLIBC_MAX).
  GLIBC_FLOOR="2.36"
  # The name hive is signed under on macOS; crates/hive-cli/src/signed.rs
  # has the same one (SIGNING_IDENTIFIER).
  SIGN_AS="hive"
  NEWLINE='
'

  tmp=""
  staged=""
  made_dirs=""
  libc=""
  trap on_exit EXIT
  trap 'exit 1' HUP INT TERM

  program=hive
  version=${HIVE_VERSION:-}
  dir=""
  parse_args "$@"

  if [ "$program" = hive ]; then
    kind=hive
    try=version
    if [ -z "$dir" ]; then
      dir=${HIVE_INSTALL_DIR:-}
    fi
  else
    kind=hand
    try=--version
    if [ -z "$dir" ]; then
      dir=${HIVE_HAND_INSTALL_DIR:-}
    fi
  fi

  case "$version" in
    "" | latest)
      tag=""
      ;;
    [0123456789]*)
      tag="v$version"
      ;;
    *)
      tag=$version
      ;;
  esac
  if [ -n "$tag" ]; then
    check_tag "$tag"
  fi

  find_target
  if [ "$os_part" = unknown-linux-gnu ]; then
    check_linux
  fi

  if has curl; then
    fetcher=curl
  elif has wget; then
    fetcher=wget
    wget_no_hsts=""
    case "$(wget --help 2>&1 || true)" in
      *--no-hsts*)
        wget_no_hsts=--no-hsts
        ;;
    esac
  else
    fail "this needs curl or wget to download. Install one of them and run it again."
  fi
  if has sha256sum; then
    hasher=sha256sum
  elif has shasum; then
    hasher=shasum
  else
    fail "found neither sha256sum nor shasum, so the download could not be checked. Nothing was installed. Install one of them and run this again."
  fi
  for tool in tar mktemp; do
    has "$tool" || fail "this needs $tool. Install it and run this again."
  done

  if [ -z "$dir" ]; then
    if [ -z "${HOME:-}" ]; then
      fail "HOME is not set, so there is no place to install to. Say where with --dir DIR."
    fi
    if [ "$program" = hive ]; then
      dir="$HOME/.hive/bin"
    else
      dir="$HOME/.hive-hand/bin"
    fi
  fi

  base=${HIVE_RELEASE_BASE:-$REPO/releases}
  base=${base%/}

  tmp=$(mktemp -d "${TMPDIR:-/tmp}/hive-install.XXXXXX") || {
    tmp=""
    fail "could not make a temporary directory in ${TMPDIR:-/tmp}"
  }

  get_checksums
  if [ -z "$tag" ]; then
    find_latest_tag
  fi
  get_program
  install_program
  report
}

main "$@"
}
