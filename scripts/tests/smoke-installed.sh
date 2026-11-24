#!/bin/sh
# Tries an installed hive on this machine: the first things a new machine
# must be able to do with it.
#
#   sh scripts/tests/smoke-installed.sh [HIVE [PORT]]
#
# HIVE is the program to try: a path, or a name on PATH. Without it the
# script takes ~/.hive/bin/hive, where the installer puts it. PORT is the
# port its console listens on for the test (43117 when not given).
#
# The program gets a home of its own, in a temporary directory that is
# removed at the end. So the Hive of whoever runs this is not read and not
# changed: no store, no setting, no service. It calls no model and asks
# nothing of the network; the one address it reads is 127.0.0.1.
#
# What it checks:
#   1. the program starts and names a version (`hive version`);
#   2. it makes a store (`hive init --store-only`) and reads it
#      (`hive status`);
#   3. its console answers on 127.0.0.1 with its own page (`hive hub`).
set -eu

hive=${1:-"$HOME/.hive/bin/hive"}
port=${2:-43117}

fail() {
  printf 'smoke-installed.sh: %s\n' "$*" >&2
  exit 1
}

case $hive in
  */*)
    if [ ! -x "$hive" ]; then
      fail "there is no program at $hive"
    fi
    ;;
  *)
    found=$(command -v "$hive" 2> /dev/null) || fail "there is no $hive on PATH"
    hive=$found
    ;;
esac
if ! command -v curl > /dev/null 2>&1; then
  fail "curl is needed, to ask the console for its page"
fi

work=$(mktemp -d "${TMPDIR:-/tmp}/hive-smoke.XXXXXX") || fail "could not make a temporary directory"
hub=""
cleanup() {
  if [ -n "$hub" ]; then
    kill "$hub" 2> /dev/null || true
    wait "$hub" 2> /dev/null || true
  fi
  rm -rf "$work"
}
trap cleanup EXIT
trap 'exit 1' INT TERM HUP

HIVE_HOME="$work/home"
export HIVE_HOME

version=$("$hive" version) || fail "\`$hive version\` did not run"
case $version in
  v[0-9]*) ;;
  *) fail "\`$hive version\` printed \"$version\", which is not a version" ;;
esac
printf 'ok    it starts: hive %s\n' "$version"

if ! "$hive" init --store-only > "$work/init.out" 2>&1; then
  cat "$work/init.out" >&2
  fail "\`hive init --store-only\` failed"
fi
if [ ! -s "$HIVE_HOME/hive.sqlite" ]; then
  fail "\`hive init --store-only\` made no store at $HIVE_HOME/hive.sqlite"
fi
if ! "$hive" status > "$work/status.out" 2>&1; then
  cat "$work/status.out" >&2
  fail "\`hive status\` failed"
fi
printf 'ok    it makes a store and reads it\n'

"$hive" hub --no-browser --port "$port" > "$work/hub.out" 2>&1 &
hub=$!
answered=0
tries=0
while [ "$tries" -lt 60 ]; do
  if curl -fsS --max-time 3 -o "$work/page.html" "http://127.0.0.1:$port/" 2> /dev/null; then
    answered=1
    break
  fi
  # A console that has stopped will not answer later either.
  if ! kill -0 "$hub" 2> /dev/null; then
    break
  fi
  tries=$((tries + 1))
  sleep 1
done
if [ "$answered" != 1 ]; then
  cat "$work/hub.out" >&2
  fail "the console did not answer on http://127.0.0.1:$port/"
fi
if ! grep -q '<title>Hive</title>' "$work/page.html"; then
  fail "http://127.0.0.1:$port/ answered, but not with the console's page"
fi
printf 'ok    its console answers on 127.0.0.1:%s\n' "$port"
