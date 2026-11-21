#!/bin/sh
# Tests scripts/install.sh against a release made up on the spot. It needs no
# network.
#
#   sh scripts/tests/install-test.sh
#   HIVE_TEST_SHELL=/bin/dash sh scripts/tests/install-test.sh    the shell the installer runs under (default: sh)
#   HIVE_TEST_CUT_STRIDE=1 sh scripts/tests/install-test.sh       cut the installer at every byte, not every 97th
#
# It packs stub programs, which print a version, into archives named and
# laid out as .github/workflows/release.yml does, writes their SHA256SUMS
# the way `sha256sum` does, and points the installer at them through
# HIVE_RELEASE_BASE: a file:// address when curl reads one, else a python3
# http.server on a port between 40000 and 49999, stopped by its PID.
#
# Every run of the installer gets an emptied environment whose HOME is a
# directory of this test's own, so nothing can reach your home. Everything
# the test writes is under one temporary directory, removed at the end.
#
# It prints one line a case and a count at the end, and exits 1 when a case
# failed.
set -u

here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd) || exit 2
installer="$here/../install.sh"
workflow="$here/../../.github/workflows/release.yml"
crates="$here/../../crates"
if [ ! -f "$installer" ]; then
  echo "install-test: no installer at $installer" >&2
  exit 2
fi

has() {
  command -v "$1" > /dev/null 2>&1
}

test_shell=${HIVE_TEST_SHELL:-sh}
case "$test_shell" in
  /*) ;;
  *)
    if ! has "$test_shell"; then
      echo "install-test: no shell named $test_shell" >&2
      exit 2
    fi
    test_shell=$(command -v "$test_shell")
    ;;
esac
stride=${HIVE_TEST_CUT_STRIDE:-97}

work=$(mktemp -d "${TMPDIR:-/tmp}/hive-install-test.XXXXXX") || exit 2
# The path with its symlinks resolved: the installer runs with a clean
# environment (no PWD), so its shell names the working directory that way,
# and macOS's own temporary directory is behind a symlink (/var).
work=$(CDPATH='' cd -- "$work" && pwd -P) || exit 2
server_pid=""

clean_up() {
  if [ -n "$server_pid" ]; then
    kill "$server_pid" 2> /dev/null
    wait "$server_pid" 2> /dev/null
  fi
  rm -rf "$work"
}
trap 'clean_up' EXIT
trap 'exit 1' HUP INT TERM

rel="$work/www/releases"
out="$work/stdout"
err="$work/stderr"
mkdir -p "$rel/latest/download" "$work/tmp" "$work/cwd" "$work/build"
: > "$out"
: > "$err"

passed=0
failed=0
skipped=0
homes=0
status=0
case_name=""
why=""

# ---- the made-up release ---------------------------------------------------

sha256_files() {
  if has sha256sum; then
    sha256sum "$@"
  else
    shasum -a 256 "$@"
  fi
}

# make_archive TAG KIND PROGRAM TARGET BODY: the archive KIND-TAG-TARGET.tar.gz
# of release TAG, holding KIND-TAG-TARGET/PROGRAM, a script that runs BODY.
make_archive() {
  folder="$2-$1-$4"
  mkdir -p "$work/build/$folder" "$rel/download/$1"
  printf '#!/bin/sh\n%s\n' "$5" > "$work/build/$folder/$3"
  chmod 755 "$work/build/$folder/$3"
  tar -czf "$rel/download/$1/$folder.tar.gz" -C "$work/build" "$folder"
  rm -rf "$work/build/$folder"
}

# make_release TAG TARGET...: hive and hive-hand for each target. hive
# prints "hive TAG TARGET" and hive-hand prints "hive-hand TAG TARGET".
make_release() {
  release=$1
  shift
  for each in "$@"; do
    make_archive "$release" hive hive "$each" "echo \"hive $release $each\""
    make_archive "$release" hand hive-hand "$each" "echo \"hive-hand $release $each\""
  done
}

# write_sums TAG: the release's SHA256SUMS, as the workflow writes it.
write_sums() {
  (cd "$rel/download/$1" && sha256_files -- *.tar.gz > SHA256SUMS)
}

mac=aarch64-apple-darwin
linux=x86_64-unknown-linux-gnu

# v9.9.9 is the latest release and v9.8.0 an older one: two targets each.
make_release v9.9.9 "$mac" "$linux"
write_sums v9.9.9
cp "$rel/download/v9.9.9/SHA256SUMS" "$rel/latest/download/SHA256SUMS"
make_release v9.8.0 "$mac" "$linux"
write_sums v9.8.0
# v9.7.0: the archive is not the one its checksum was taken of.
make_release v9.7.0 "$mac"
write_sums v9.7.0
make_archive v9.7.0 hive hive "$mac" 'echo "hive changed on the way"'
# v9.6.0: a program that does not start.
make_archive v9.6.0 hive hive "$mac" 'echo "hive: this stub does not start" >&2; exit 1'
make_archive v9.6.0 hive hive "$linux" 'echo "hive: this stub does not start" >&2; exit 1'
write_sums v9.6.0
# v9.5.0: the checksums file lists an archive the release does not hold.
make_release v9.5.0 "$mac"
write_sums v9.5.0
rm -f "$rel/download/v9.5.0/hive-v9.5.0-$mac.tar.gz"
# v9.4.0: all four targets, for a run on this machine as it is.
make_release v9.4.0 "$mac" x86_64-apple-darwin "$linux" aarch64-unknown-linux-gnu
write_sums v9.4.0
# v9.3.0: the checksums as `sha256sum -b` run from another folder writes
# them, in capitals, after a line that is no checksum, and with no newline
# at the end of the file.
make_release v9.3.0 "$mac"
write_sums v9.3.0
odd=$(
  echo "not-a-digest  hive-v9.3.0-$mac.tar.gz"
  while read -r sum name; do
    printf '%s *./dist/%s\n' "$(echo "$sum" | tr 'abcdef' 'ABCDEF')" "$name"
  done < "$rel/download/v9.3.0/SHA256SUMS"
)
printf '%s' "$odd" > "$rel/download/v9.3.0/SHA256SUMS"
# v9.2.0: something answered in place of the checksums file.
make_release v9.2.0 "$mac"
echo '<html><body>Not Found</body></html>' > "$rel/download/v9.2.0/SHA256SUMS"

# ---- stand-ins for what a Linux says about its C library -------------------

# fake_libc DIR GLIBC_VERSION LONG_BIT LDD_BODY: a getconf and an ldd in DIR.
# With no GLIBC_VERSION, getconf does not know GNU_LIBC_VERSION, as on musl.
fake_libc() {
  mkdir -p "$1"
  glibc_answer=""
  if [ -n "$2" ]; then
    glibc_answer="GNU_LIBC_VERSION) echo \"glibc $2\" ;;"
  fi
  cat > "$1/getconf" << EOF
#!/bin/sh
case "\$1" in
$glibc_answer
LONG_BIT) echo $3 ;;
*) echo "getconf: \$1: unknown variable" >&2; exit 1 ;;
esac
EOF
  printf '#!/bin/sh\n%s\n' "$4" > "$1/ldd"
  chmod 755 "$1/getconf" "$1/ldd"
}

fake_libc "$work/bin-glibc" 2.39 64 'echo "ldd (GNU libc) 2.39"'
fake_libc "$work/bin-oldglibc" 2.31 64 'echo "ldd (GNU libc) 2.31"'
fake_libc "$work/bin-32" 2.39 32 'echo "ldd (GNU libc) 2.39"'
fake_libc "$work/bin-musl" "" 64 'echo "musl libc (x86_64)" >&2; echo "Version 1.2.5" >&2; exit 1'

# bare_bin DIR: a PATH that holds the tools the installer needs and nothing
# that downloads or hashes; cases add what they want with add_tool.
bare_bin() {
  mkdir -p "$1"
  for tool in tar gzip mkdir rmdir chmod cp mv rm tr mktemp uname; do
    at=$(command -v "$tool" 2> /dev/null) || continue
    case "$at" in
      /*) ln -s "$at" "$1/$tool" ;;
    esac
  done
}

# add_tool DIR TOOL: succeeds when this machine has TOOL.
add_tool() {
  at=$(command -v "$2" 2> /dev/null) || return 1
  case "$at" in
    /*) ln -s "$at" "$1/$2" ;;
    *) return 1 ;;
  esac
}

# ---- running the installer --------------------------------------------------

begin() {
  case_name=$1
  why=""
  homes=$((homes + 1))
  t_home="$work/home-$homes"
  mkdir -p "$t_home"
  t_path=$PATH
  t_os=Darwin
  t_arch=arm64
  t_base=$base
  t_version=""
  t_dir=""
  t_hand_dir=""
  t_umask=022
}

# The first thing that went wrong in a case is the one reported.
wrong() {
  if [ -z "$why" ]; then
    why=$1
  fi
}

finish() {
  left=$(ls -A "$work/tmp")
  if [ -n "$left" ]; then
    wrong "the installer left this in its temporary directory: $left"
    rm -rf "$work/tmp"
    mkdir -p "$work/tmp"
  fi
  if [ -z "$why" ]; then
    passed=$((passed + 1))
    printf 'ok    %s\n' "$case_name"
  else
    failed=$((failed + 1))
    printf 'FAIL  %s\n' "$case_name"
    {
      printf '      %s\n' "$why"
      printf '      the last run of the installer ended with status %s and printed:\n' "$status"
      sed 's/^/      out| /' "$out"
      sed 's/^/      err| /' "$err"
    } >&2
  fi
}

skip() {
  skipped=$((skipped + 1))
  printf 'skip  %s (%s)\n' "$case_name" "$1"
}

# shell_brings TOOL: whether the shell under test runs TOOL itself, whatever
# the PATH holds. BusyBox's shell does that for its own tools (sha256sum,
# wget, tar): a case that hides a tool by leaving it off the PATH, or that
# puts a stand-in for it on the PATH, cannot be set up under such a shell.
shell_brings() {
  env -i PATH=/nonexistent "$test_shell" -c "command -v $1" > /dev/null 2>&1
}

# clean_env COMMAND...: COMMAND in an emptied environment that holds only
# what the case set.
clean_env() {
  env -i PATH="$t_path" HOME="$t_home" TMPDIR="$work/tmp" HIVE_RELEASE_BASE="$t_base" \
    ${t_os:+"HIVE_INSTALL_OS=$t_os"} \
    ${t_arch:+"HIVE_INSTALL_ARCH=$t_arch"} \
    ${t_version:+"HIVE_VERSION=$t_version"} \
    ${t_dir:+"HIVE_INSTALL_DIR=$t_dir"} \
    ${t_hand_dir:+"HIVE_HAND_INSTALL_DIR=$t_hand_dir"} \
    "$@"
}

guard_home() {
  case "$t_home" in
    "$work"/*) ;;
    *)
      echo "install-test: refusing to run the installer with HOME=$t_home" >&2
      exit 2
      ;;
  esac
}

send() {
  cat "$1"
}

# run_installer ARGS...: as `curl ... | sh -s -- ARGS` runs it, the script
# arriving on a pipe. Leaves stdout in $out, stderr in $err, the exit
# status in $status.
run_installer() {
  guard_home
  (
    cd "$work/cwd" || exit 97
    umask "$t_umask"
    send "$installer" | clean_env "$test_shell" -s -- "$@"
  ) > "$out" 2> "$err"
  status=$?
}

# run_installer_file ARGS...: as `sh install.sh ARGS` runs it.
run_installer_file() {
  guard_home
  (
    cd "$work/cwd" || exit 97
    umask "$t_umask"
    clean_env "$test_shell" "$installer" "$@"
  ) > "$out" 2> "$err"
  status=$?
}

# ---- what a case asks for ----------------------------------------------------

want_status() {
  if [ "$status" -ne "$1" ]; then
    wrong "the installer ended with status $status, not $1"
  fi
}

want_out() {
  if ! grep -qF -- "$1" "$out"; then
    wrong "stdout does not say: $1"
  fi
}

want_no_out() {
  if grep -qF -- "$1" "$out"; then
    wrong "stdout says: $1"
  fi
}

want_err() {
  if ! grep -qF -- "$1" "$err"; then
    wrong "stderr does not say: $1"
  fi
}

want_no_err() {
  if grep -qF -- "$1" "$err"; then
    wrong "stderr says: $1"
  fi
}

want_quiet_err() {
  if [ -s "$err" ]; then
    wrong "stderr is not empty"
  fi
}

# want_runs FILE TEXT: FILE is a program with mode 755 that prints TEXT.
want_runs() {
  if [ ! -f "$1" ]; then
    wrong "there is no $1"
    return
  fi
  if [ -z "$(find "$1" -prune -type f -perm 755)" ]; then
    wrong "$1 is not a file with mode 755"
    return
  fi
  said=$("$1" version 2>&1)
  if [ "$said" != "$2" ]; then
    wrong "$1 prints '$said', not '$2'"
  fi
}

# want_only DIR FILE: FILE is all there is under DIR, directories aside.
want_only() {
  holds=$(cd "$1" && find . ! -type d | sort | tr '\n' ' ')
  if [ "$holds" != "./$2 " ]; then
    wrong "$1 holds [ $holds], not only ./$2"
  fi
}

want_empty() {
  holds=$(ls -A "$1")
  if [ -n "$holds" ]; then
    wrong "$1 is not empty: $holds"
  fi
}

# ---- where the release is read from -----------------------------------------

probe() {
  if has curl; then
    curl -fsS -o /dev/null "$1" 2> /dev/null
  else
    wget -q -O /dev/null "$1" 2> /dev/null
  fi
}

nap() {
  sleep 0.1 2> /dev/null || sleep 1
}

# Serves the release over http on 127.0.0.1 and sets http_base. Fails when
# there is no python3 or no free port.
http_base=""
start_server() {
  if [ -n "$http_base" ]; then
    return 0
  fi
  has python3 || return 1
  port=$((40000 + $$ % 10000))
  tries=0
  while [ "$tries" -lt 20 ]; do
    (cd "$work/www" && exec python3 -m http.server "$port" --bind 127.0.0.1 > "$work/server.log" 2>&1) &
    server_pid=$!
    waited=0
    while [ "$waited" -lt 50 ] && kill -0 "$server_pid" 2> /dev/null; do
      if probe "http://127.0.0.1:$port/releases/latest/download/SHA256SUMS"; then
        http_base="http://127.0.0.1:$port/releases"
        return 0
      fi
      nap
      waited=$((waited + 1))
    done
    kill "$server_pid" 2> /dev/null
    wait "$server_pid" 2> /dev/null
    server_pid=""
    port=$((port + 1))
    if [ "$port" -gt 49999 ]; then
      port=40000
    fi
    tries=$((tries + 1))
  done
  return 1
}

if has curl && curl -fsS -o /dev/null "file://$rel/latest/download/SHA256SUMS" 2> /dev/null; then
  base="file://$rel"
elif start_server; then
  base=$http_base
else
  echo "install-test: needs a curl that reads file:// addresses, or python3 to serve the release" >&2
  exit 2
fi

# ---- the cases ----------------------------------------------------------------

begin "a clean install puts hive in ~/.hive/bin, mode 755, and it runs"
t_umask=077
run_installer
want_status 0
want_runs "$t_home/.hive/bin/hive" "hive v9.9.9 $mac"
want_only "$t_home" .hive/bin/hive
want_out "Installed hive v9.9.9 at $t_home/.hive/bin/hive"
want_out "The archive matches its SHA-256 in SHA256SUMS"
want_quiet_err
finish

begin "it says the directory is not on the PATH, the line to add, and the next step"
run_installer
want_status 0
want_out "$t_home/.hive/bin is not on your PATH"
want_out "  export PATH=\"\$HOME/.hive/bin:\$PATH\""
want_out "  hive hub"
want_no_out "hive init"
want_out "http://127.0.0.1:8765"
finish

begin "it says so when the directory is on the PATH, and prints no line to add"
t_path="$t_home/.hive/bin:$PATH"
run_installer
want_status 0
want_out "$t_home/.hive/bin is on your PATH."
want_no_out "export PATH"
want_out "  hive hub"
finish

begin "--hand installs hive-hand in ~/.hive-hand/bin and names the join command"
run_installer --hand
want_status 0
want_runs "$t_home/.hive-hand/bin/hive-hand" "hive-hand v9.9.9 $mac"
want_only "$t_home" .hive-hand/bin/hive-hand
want_out "Installed hive-hand v9.9.9 at $t_home/.hive-hand/bin/hive-hand"
want_out "  export PATH=\"\$HOME/.hive-hand/bin:\$PATH\""
want_out "  hive-hand join --brain URL CODE"
want_no_out "hive hub"
finish

begin "Linux on x86_64 gets the linux-gnu archive; amd64 and aarch64 are understood"
t_path="$work/bin-glibc:$PATH"
t_os=Linux
t_arch=x86_64
run_installer
want_status 0
want_runs "$t_home/.hive/bin/hive" "hive v9.9.9 $linux"
t_os=linux
t_arch=amd64
run_installer --dir "$t_home/amd64"
want_status 0
want_runs "$t_home/amd64/hive" "hive v9.9.9 $linux"
t_os=darwin
t_arch=aarch64
run_installer --dir "$t_home/aarch64"
want_status 0
want_runs "$t_home/aarch64/hive" "hive v9.9.9 $mac"
finish

begin "--version picks that release, with or without the v, and beats HIVE_VERSION"
run_installer --version v9.8.0
want_status 0
want_runs "$t_home/.hive/bin/hive" "hive v9.8.0 $mac"
want_out "Installed hive v9.8.0"
run_installer --dir "$t_home/a" --version 9.8.0
want_runs "$t_home/a/hive" "hive v9.8.0 $mac"
run_installer --dir "$t_home/b" --version=v9.8.0
want_runs "$t_home/b/hive" "hive v9.8.0 $mac"
t_version=v9.8.0
run_installer --dir "$t_home/c"
want_runs "$t_home/c/hive" "hive v9.8.0 $mac"
run_installer --dir "$t_home/d" --version v9.9.9
want_runs "$t_home/d/hive" "hive v9.9.9 $mac"
run_installer --dir "$t_home/e" --hand
want_runs "$t_home/e/hive-hand" "hive-hand v9.8.0 $mac"
finish

begin "--dir picks that directory: absolute, relative, with a space, and for --hand"
run_installer --dir "$work/elsewhere/bin"
want_status 0
want_runs "$work/elsewhere/bin/hive" "hive v9.9.9 $mac"
want_out "Installed hive v9.9.9 at $work/elsewhere/bin/hive"
want_out "  export PATH=\"$work/elsewhere/bin:\$PATH\""
want_empty "$t_home"
run_installer --dir relative/bin
want_status 0
want_runs "$work/cwd/relative/bin/hive" "hive v9.9.9 $mac"
want_out "Installed hive v9.9.9 at $work/cwd/relative/bin/hive"
run_installer "--dir=$t_home/my tools"
want_status 0
want_runs "$t_home/my tools/hive" "hive v9.9.9 $mac"
run_installer --hand --dir "$t_home/hands"
want_status 0
want_runs "$t_home/hands/hive-hand" "hive-hand v9.9.9 $mac"
finish

begin "HIVE_INSTALL_DIR and HIVE_HAND_INSTALL_DIR each move their own program"
t_dir="$t_home/one"
t_hand_dir="$t_home/two"
run_installer
want_status 0
want_runs "$t_home/one/hive" "hive v9.9.9 $mac"
run_installer --hand
want_status 0
want_runs "$t_home/two/hive-hand" "hive-hand v9.9.9 $mac"
run_installer --dir "$t_home/three"
want_runs "$t_home/three/hive" "hive v9.9.9 $mac"
finish

begin "on this machine as it is, with no override, it installs or says why not"
t_os=""
t_arch=""
run_installer --version v9.4.0
if [ "$status" -eq 0 ]; then
  case "$("$t_home/.hive/bin/hive" version 2>&1)" in
    "hive v9.4.0 "*-apple-darwin | "hive v9.4.0 "*-unknown-linux-gnu) ;;
    *) wrong "the hive it installed is not one of the four" ;;
  esac
  finish
elif grep -qE "for this machine|musl|32-bit|Windows" "$err"; then
  want_status 1
  want_empty "$t_home"
  if [ -n "$why" ]; then
    finish
  else
    skip "no release fits this machine: $(sed -n 1p "$err")"
  fi
else
  wrong "it neither installed nor said that no release fits this machine"
  finish
fi

begin "running it again upgrades in place: the old file is replaced, not written over"
run_installer --version v9.8.0
want_status 0
want_runs "$t_home/.hive/bin/hive" "hive v9.8.0 $mac"
want_no_out "in place of the one that was there"
ln "$t_home/.hive/bin/hive" "$t_home/kept"
run_installer
want_status 0
want_runs "$t_home/.hive/bin/hive" "hive v9.9.9 $mac"
want_runs "$t_home/kept" "hive v9.8.0 $mac"
want_out "Installed hive v9.9.9 at $t_home/.hive/bin/hive, in place of the one that was there."
rm -f "$t_home/kept"
run_installer
want_status 0
want_runs "$t_home/.hive/bin/hive" "hive v9.9.9 $mac"
want_only "$t_home" .hive/bin/hive
finish

begin "run as a file (sh install.sh) it does the same"
run_installer_file --hand
want_status 0
want_runs "$t_home/.hive-hand/bin/hive-hand" "hive-hand v9.9.9 $mac"
want_only "$t_home" .hive-hand/bin/hive-hand
finish

begin "on a Mac, hive is signed the way hive update signs it, and hive-hand is not"
if [ "$(uname -s)" != Darwin ] || ! has codesign; then
  skip "this is not a Mac"
else
  run_installer
  want_status 0
  want_quiet_err
  run_installer --hand
  want_status 0
  signature=$(codesign -d -r- "$t_home/.hive/bin/hive" 2>&1 | tr -d '"')
  case "$signature" in
    *"designated => identifier hive"*) ;;
    *) wrong "hive does not carry the requirement every build meets: $signature" ;;
  esac
  signature=$(codesign -d -r- "$t_home/.hive-hand/bin/hive-hand" 2>&1 | tr -d '"')
  case "$signature" in
    *"designated => identifier hive"*) wrong "hive-hand was signed as hive" ;;
  esac
  want_runs "$t_home/.hive/bin/hive" "hive v9.9.9 $mac"
  finish
fi

begin "an archive that does not match its checksum is refused and nothing is installed"
run_installer --version v9.7.0
want_status 1
want_err "hive-v9.7.0-$mac.tar.gz does not match its SHA-256 in SHA256SUMS. Nothing was installed."
want_no_out "Installed"
want_empty "$t_home"
finish

begin "an archive that does not match its checksum leaves an older hive as it was"
run_installer --version v9.8.0
want_status 0
run_installer --version v9.7.0
want_status 1
want_err "does not match its SHA-256"
want_runs "$t_home/.hive/bin/hive" "hive v9.8.0 $mac"
want_only "$t_home" .hive/bin/hive
finish

begin "a program that does not start is refused and an older hive is left as it was"
run_installer --version v9.8.0
want_status 0
run_installer --version v9.6.0
want_status 1
want_err "hive v9.6.0 does not start on this machine"
want_err "hive: this stub does not start"
want_err "nothing was installed"
want_runs "$t_home/.hive/bin/hive" "hive v9.8.0 $mac"
want_only "$t_home" .hive/bin/hive
finish

begin "on Linux, a program that does not start is told the glibc of the machine"
t_path="$work/bin-oldglibc:$PATH"
t_os=Linux
t_arch=x86_64
run_installer --version v9.6.0
want_status 1
want_err "This machine has glibc 2.31. The Linux release is built to run on glibc 2.36 and newer."
want_err "cargo install --git https://github.com/lamplitlabs/hive --locked --root ~/.hive hive"
want_empty "$t_home"
finish

begin "checksums in sha256sum's other forms are read the way hive update reads them"
run_installer --version v9.3.0
want_status 0
want_runs "$t_home/.hive/bin/hive" "hive v9.3.0 $mac"
finish

begin "a checksums file that lists no checksum stops the install"
run_installer --version v9.2.0
want_status 1
want_err "lists no checksums, so no download could be checked. Nothing was installed."
want_empty "$t_home"
finish

begin "a target the latest release does not have fails and names the release and the target"
t_path="$work/bin-glibc:$PATH"
t_os=Linux
t_arch=aarch64
run_installer
want_status 1
want_err "the latest release, v9.9.9, has no hive for this machine (aarch64-unknown-linux-gnu)."
want_err "cargo install --git https://github.com/lamplitlabs/hive --locked --root ~/.hive hive"
want_empty "$t_home"
t_os=Darwin
t_arch=x86_64
run_installer --hand
want_status 1
want_err "the latest release, v9.9.9, has no hive-hand for this machine (x86_64-apple-darwin)."
want_err "cargo build --release -p hive-hand"
want_empty "$t_home"
finish

begin "a target the release named by --version does not have fails the same way"
t_arch=x86_64
run_installer --version v9.9.9
want_status 1
want_err "release v9.9.9 has no hive for this machine (x86_64-apple-darwin)"
want_err "its SHA256SUMS lists no hive-v9.9.9-x86_64-apple-darwin.tar.gz."
want_empty "$t_home"
finish

begin "an archive the release lists but does not hold fails and names the address"
run_installer --version v9.5.0
want_status 1
want_err "could not download $base/download/v9.5.0/hive-v9.5.0-$mac.tar.gz"
want_empty "$t_home"
finish

begin "a release that does not exist fails and asks whether it is one"
run_installer --version v1.2.3
want_status 1
want_err "could not download $base/download/v1.2.3/SHA256SUMS"
want_err "Is v1.2.3 a release of Hive?"
want_empty "$t_home"
t_base="file://$work/no-such-place"
run_installer
want_status 1
want_err "could not download file://$work/no-such-place/latest/download/SHA256SUMS"
want_empty "$t_home"
finish

begin "a tag that is not a tag is refused before anything is downloaded"
run_installer --version ../../etc
want_status 1
want_err "'../../etc' is not a release tag"
run_installer --version 'v1.0.0/../x'
want_status 1
want_err "is not a release tag"
want_empty "$t_home"
finish

begin "an unsupported system (freebsd) fails, names it, and says how to build from source"
t_os=freebsd
t_arch=amd64
run_installer
want_status 1
want_err "there is no release of hive for this machine (freebsd amd64)."
want_err "To build Hive from source instead (Rust 1.95+, Node.js 22+):"
want_err "  cargo install --git https://github.com/lamplitlabs/hive --locked --root ~/.hive hive"
want_no_out "Installed"
want_empty "$t_home"
run_installer --hand
want_status 1
want_err "there is no release of hive-hand for this machine (freebsd amd64)."
want_err "cargo build --release -p hive-hand"
want_empty "$t_home"
finish

begin "an unsupported processor fails the same way"
t_path="$work/bin-glibc:$PATH"
t_os=Linux
t_arch=riscv64
run_installer
want_status 1
want_err "there is no release of hive for this machine (Linux riscv64)."
want_err "cargo install --git https://github.com/lamplitlabs/hive"
want_empty "$t_home"
finish

begin "Windows is told that Hive does not run there yet, with no build line"
t_os=MINGW64_NT-10.0-19045
t_arch=x86_64
run_installer
want_status 1
want_err "Hive does not run on Windows yet"
want_no_err "cargo"
want_empty "$t_home"
finish

begin "a Linux with musl is told that the release is built for glibc"
t_path="$work/bin-musl:$PATH"
t_os=Linux
t_arch=x86_64
run_installer
want_status 1
want_err "this Linux uses musl as its C library, not glibc"
want_err "The release is built for glibc and does not run here."
want_err "cargo install --git https://github.com/lamplitlabs/hive --locked --root ~/.hive hive"
want_empty "$t_home"
finish

begin "a Linux that runs 32-bit programs is told that releases are 64-bit"
t_path="$work/bin-32:$PATH"
t_os=Linux
t_arch=aarch64
run_installer
want_status 1
want_err "this Linux runs 32-bit programs"
want_empty "$t_home"
finish

begin "an unknown option, a stray word and a flag without its value are errors"
run_installer --frobnicate
want_status 1
want_err "unknown option '--frobnicate'"
run_installer v9.8.0
want_status 1
want_err "unknown option 'v9.8.0'"
run_installer --version
want_status 1
want_err "--version needs a release tag"
run_installer --dir
want_status 1
want_err "--dir needs a directory"
run_installer --version=
want_status 1
want_err "--version needs a release tag"
run_installer --dir=
want_status 1
want_err "--dir needs a directory"
want_no_out "Installed"
want_empty "$t_home"
finish

begin "--help prints the options and installs nothing"
run_installer --help
want_status 0
want_out "--hand"
want_out "--version TAG"
want_out "--dir DIR"
want_out "HIVE_RELEASE_BASE"
want_quiet_err
want_empty "$t_home"
finish

begin "a directory where the program should go is refused and left alone"
mkdir -p "$t_home/.hive/bin/hive"
run_installer
want_status 1
want_err "$t_home/.hive/bin/hive is a directory"
want_empty "$t_home/.hive/bin/hive"
finish

begin "a directory it cannot write in fails without asking for more rights"
if [ "$(id -u)" -eq 0 ]; then
  skip "root can write anywhere"
else
  mkdir -p "$t_home/locked"
  chmod 555 "$t_home/locked"
  run_installer --dir "$t_home/locked"
  want_status 1
  want_err "could not write in $t_home/locked"
  want_no_err "sudo"
  want_empty "$t_home/locked"
  chmod 755 "$t_home/locked"
  finish
fi

begin "with neither sha256sum nor shasum it stops and installs nothing"
bare_bin "$work/bin-nohash"
if shell_brings sha256sum || shell_brings shasum; then
  skip "the shell under test brings its own checksum tool, which a PATH cannot hide"
elif add_tool "$work/bin-nohash" curl || add_tool "$work/bin-nohash" wget; then
  t_path="$work/bin-nohash"
  run_installer
  want_status 1
  want_err "found neither sha256sum nor shasum, so the download could not be checked. Nothing was installed."
  want_empty "$t_home"
  finish
else
  skip "no curl and no wget here"
fi

for hash_tool in sha256sum shasum; do
  begin "with $hash_tool alone to check the download it installs, and refuses a bad one"
  bare_bin "$work/bin-$hash_tool"
  if ! add_tool "$work/bin-$hash_tool" "$hash_tool"; then
    skip "no $hash_tool here"
  elif ! add_tool "$work/bin-$hash_tool" curl; then
    skip "no curl here"
  else
    t_path="$work/bin-$hash_tool"
    run_installer
    want_status 0
    want_runs "$t_home/.hive/bin/hive" "hive v9.9.9 $mac"
    run_installer --version v9.7.0
    want_status 1
    want_err "does not match its SHA-256"
    want_runs "$t_home/.hive/bin/hive" "hive v9.9.9 $mac"
    finish
  fi
done

begin "with neither curl nor wget it says that it needs one"
bare_bin "$work/bin-nofetch"
add_tool "$work/bin-nofetch" sha256sum || add_tool "$work/bin-nofetch" shasum
if shell_brings curl || shell_brings wget; then
  skip "the shell under test brings its own download tool, which a PATH cannot hide"
else
  t_path="$work/bin-nofetch"
  run_installer
  want_status 1
  want_err "this needs curl or wget to download"
  want_empty "$t_home"
  finish
fi

begin "with wget and no curl it installs over http, and still refuses a bad checksum"
bare_bin "$work/bin-wget"
add_tool "$work/bin-wget" sha256sum || add_tool "$work/bin-wget" shasum
if ! has wget; then
  skip "no wget here"
elif ! start_server; then
  skip "no python3 here to serve the release, or no free port"
else
  # A wget that notes how it was called, then is the real one.
  real_wget=$(command -v wget)
  cat > "$work/bin-wget/wget" << EOF
#!/bin/sh
echo "\$*" >> "$work/wget-calls"
exec "$real_wget" "\$@"
EOF
  chmod 755 "$work/bin-wget/wget"
  t_path="$work/bin-wget"
  t_base=$http_base
  run_installer
  want_status 0
  want_runs "$t_home/.hive/bin/hive" "hive v9.9.9 $mac"
  run_installer --hand --version v9.8.0
  want_status 0
  want_runs "$t_home/.hive-hand/bin/hive-hand" "hive-hand v9.8.0 $mac"
  run_installer --version v9.7.0
  want_status 1
  want_err "does not match its SHA-256"
  run_installer --version v1.2.3
  want_status 1
  want_err "could not download $http_base/download/v1.2.3/SHA256SUMS"
  want_runs "$t_home/.hive/bin/hive" "hive v9.9.9 $mac"
  # GNU wget would leave ~/.wget-hsts behind after an https download. A
  # shell that brings its own wget ran that one and not the stand-in, so
  # there is no note of how it was called.
  if shell_brings wget; then
    :
  elif "$real_wget" --help 2>&1 | grep -q -- --no-hsts; then
    if ! grep -q -- "-q --no-hsts -O " "$work/wget-calls"; then
      wrong "wget has --no-hsts and the installer did not use it"
    fi
  elif grep -q -- --no-hsts "$work/wget-calls"; then
    wrong "the installer gave --no-hsts to a wget that has no such option"
  fi
  finish
fi

# A stand-in for curl that says who runs it and then hangs, as a download
# that never ends would. The test stops the installer and that download by
# their PIDs, which is what a terminal does to both on Ctrl-C.
begin "stopped in the middle of a download, it removes its temporary directory and ends with 1"
mkdir -p "$work/bin-slow"
cat > "$work/bin-slow/curl" << EOF
#!/bin/sh
echo "\$PPID \$\$" > "$work/slow.pids"
exec sleep 30
EOF
chmod 755 "$work/bin-slow/curl"
t_path="$work/bin-slow:$PATH"
guard_home
clean_env "$test_shell" "$installer" > "$out" 2> "$err" &
runner=$!
waited=0
while [ ! -s "$work/slow.pids" ] && [ "$waited" -lt 100 ]; do
  nap
  waited=$((waited + 1))
done
if [ -s "$work/slow.pids" ]; then
  read -r installer_pid download_pid < "$work/slow.pids"
  if [ -z "$(ls -A "$work/tmp")" ]; then
    wrong "the installer had no temporary directory while it downloaded"
  fi
  kill -TERM "$installer_pid" "$download_pid"
else
  wrong "the installer never started its download"
  kill "$runner" 2> /dev/null
fi
wait "$runner"
status=$?
want_status 1
want_empty "$t_home"
finish

# A download can stop anywhere. Whatever part of the script arrives, the
# shell that reads it must run nothing: nothing is printed, no file appears
# in HOME and no temporary directory is made. The cuts: a quarter, half and
# three quarters of the way, every $stride-th byte, and every byte of the
# last 64, which hold the call of main and the brace that closes the script.
begin "cut off anywhere, the script installs nothing and runs nothing"
size=$(($(wc -c < "$installer")))
cuts=0
cut_at() {
  if [ -n "$why" ] || [ "$1" -lt 1 ] || [ "$1" -gt $((size - 2)) ]; then
    return
  fi
  dd if="$installer" of="$work/cut.sh" bs="$1" count=1 2> /dev/null
  guard_home
  send "$work/cut.sh" | clean_env "$test_shell" > "$out" 2> "$err"
  status=$?
  cuts=$((cuts + 1))
  if [ -s "$out" ]; then
    wrong "cut after $1 of $size bytes, the script printed something"
  elif [ -n "$(ls -A "$t_home")" ]; then
    wrong "cut after $1 of $size bytes, the script wrote into HOME"
  elif [ -n "$(ls -A "$work/tmp")" ]; then
    wrong "cut after $1 of $size bytes, the script made a temporary directory"
  fi
}
if [ "$(tail -n 2 "$installer")" != "main \"\$@\"
}" ]; then
  wrong "the installer does not end with the call of main and the brace that closes the script"
fi
# ksh93 never returns from a here-document that the end of its input leaves
# open, so the installer has none.
if grep -v '^ *#' "$installer" | grep -q '<<'; then
  wrong "the installer has a here-document, which hangs ksh93 when the download is cut inside it"
fi
cut_at $((size / 4))
cut_at $((size / 2))
cut_at $((size * 3 / 4))
at=$stride
while [ "$at" -lt $((size - 64)) ]; do
  cut_at "$at"
  at=$((at + stride))
done
at=$((size - 64))
while [ "$at" -le $((size - 2)) ]; do
  cut_at "$at"
  at=$((at + 1))
done
if [ "$cuts" -lt 20 ]; then
  wrong "only $cuts cuts were tried"
fi
# And the whole script, fed the same way, does install: the check above
# would pass for a script that never runs at all.
if [ -z "$why" ]; then
  run_installer
  want_status 0
  want_runs "$t_home/.hive/bin/hive" "hive v9.9.9 $mac"
fi
case_name="$case_name ($cuts cuts)"
finish

begin "the installer, the release workflow and hive name the same things"
if [ ! -f "$workflow" ]; then
  skip "no release.yml beside this copy"
else
  status=0
  : > "$out"
  : > "$err"
  floor=$(sed -n 's/^ *GLIBC_FLOOR="\([0-9.]*\)"$/\1/p' "$installer")
  ceiling=$(sed -n 's/^ *GLIBC_MAX: "\([0-9.]*\)".*$/\1/p' "$workflow")
  if [ -z "$floor" ] || [ "$floor" != "$ceiling" ]; then
    wrong "the installer says glibc '$floor' and the workflow checks '$ceiling'"
  fi
  for triple in aarch64-apple-darwin x86_64-apple-darwin x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
    if ! grep -q "target: $triple\$" "$workflow"; then
      wrong "the workflow builds no $triple"
    fi
    if ! grep -qF "hive-\$TAG-$triple.tar.gz" "$workflow" || ! grep -qF "hand-\$TAG-$triple.tar.gz" "$workflow"; then
      wrong "the workflow does not ask for the hive and hand archives of $triple"
    fi
  done
  if ! grep -qF 'CHECKSUMS="SHA256SUMS"' "$installer" || ! grep -qF '> SHA256SUMS' "$workflow"; then
    wrong "the installer and the workflow do not both name SHA256SUMS"
  fi
  # The name hive is signed under on macOS, and the file hive update reads.
  if [ -f "$crates/hive-cli/src/signed.rs" ] && [ -f "$crates/hive-cli/src/update.rs" ]; then
    ours=$(sed -n 's/^ *SIGN_AS="\([^"]*\)"$/\1/p' "$installer")
    theirs=$(sed -n 's/^pub const SIGNING_IDENTIFIER: &str = "\([^"]*\)";$/\1/p' "$crates/hive-cli/src/signed.rs")
    if [ -z "$ours" ] || [ "$ours" != "$theirs" ]; then
      wrong "the installer signs hive as '$ours' and hive signs itself as '$theirs'"
    fi
    if ! grep -qF 'pub const CHECKSUMS_ASSET: &str = "SHA256SUMS";' "$crates/hive-cli/src/update.rs"; then
      wrong "hive update no longer reads a file named SHA256SUMS"
    fi
  fi
  finish
fi

printf '%s of %s cases passed' "$passed" "$((passed + failed))"
if [ "$skipped" -gt 0 ]; then
  printf ', %s skipped' "$skipped"
fi
printf '\n'
# No `exit 0` after this: the last status is already 0, and shellcheck
# takes an EXIT trap behind a closing `exit` for one that never runs.
if [ "$failed" -gt 0 ]; then
  exit 1
fi
