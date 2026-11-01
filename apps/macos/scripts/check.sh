#!/bin/sh
# Checks the Mac app end to end: the PulseKit tests, a release build, and a run
# of the built app against the console's mock (web/hub/mock/server.mjs) on a
# free port. The run goes to Records through the Go menu and to Tasks with a
# real mouse click on the sidebar (through AppKit's hit testing, so nothing
# native may sit over the page), right-clicks the sidebar's Tasks link and a
# bare patch of page (an app's menu must show, not a browser's), loses the
# console while it reloads, and has to come back by itself; the app's own
# log and two snapshots are the evidence. The outage follows the app's own progress (the Tasks snapshot on
# disk, the failed reload in its log), not a clock, so a slow first launch
# cannot shift it onto the wrong step. The click asks to bring the app's
# window to the front for a moment, and lands whether or not macOS agrees.
# It never touches the live console on 8765, and a test run remembers
# nothing (-EphemeralRun). Needs Xcode or the command line tools, and Node
# for the mock (without Node only the tests and the build run).
#
#   apps/macos/scripts/check.sh            the evidence lands in a temporary folder it names
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)
repo=$(cd "$here/../.." && pwd)
app="$here/build/PULSE.app"
scratch=$(mktemp -d "${TMPDIR:-/tmp}/pulse-mac-check.XXXXXX")
mock=""
pid=""

say() { printf '%s\n' "$*" >&2; }
note() { printf '%s %s\n' "$(date '+%H:%M:%S')" "$*" >> "$scratch/check.log"; }
fail() { say "check.sh: $*"; note "FAIL: $*"; say "evidence: $scratch"; exit 1; }
cleanup() {
  [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
  [ -n "$mock" ] && kill "$mock" 2>/dev/null || true
}
trap cleanup EXIT

say "== tests"
swift test --package-path "$here" >&2

say "== build"
"$here/scripts/build.sh" >/dev/null

if ! command -v node >/dev/null 2>&1; then
  say "== run: skipped (no node for the mock console)"
  exit 0
fi

port=$(node -e 'const s=require("net").createServer();s.listen(0,"127.0.0.1",()=>{console.log(s.address().port);s.close()})')
console="http://127.0.0.1:$port/"
subsystem='subsystem == "io.github.manishtiwari25.pulse"'
start_mock() {
  (cd "$repo/web/hub" && exec env MOCK_STATIC=out PULSE_HUB_PORT="$port" node mock/server.mjs) >> "$scratch/mock.log" 2>&1 &
  mock=$!
  i=0
  until curl -fs -o /dev/null "${console}api/v1/health"; do
    i=$((i + 1)); [ $i -lt 80 ] || fail "the mock console did not start (see mock.log)"; sleep 0.25
  done
  note "mock up (pid $mock)"
}
stop_mock() {
  kill "$mock"; wait "$mock" 2>/dev/null || true; mock=""
  i=0
  while curl -s -o /dev/null "${console}api/v1/health"; do
    i=$((i + 1)); [ $i -lt 40 ] || fail "the mock console still answers after being stopped"; sleep 0.25
  done
  note "mock down"
}
# Waits up to $2 seconds for the app to have logged $1 (the unified log lags
# a little behind the app).
wait_logged() {
  i=0
  until /usr/bin/log show --start "$since" --predicate "$subsystem" --style compact 2>/dev/null | grep -qF "$1"; do
    i=$((i + 1)); [ $i -lt "$2" ] || return 1; sleep 1
  done
}

say "== run against the mock on $console"
start_mock
since=$(date '+%Y-%m-%d %H:%M:%S')
# The commands' clock starts when the first page has loaded.
"$app/Contents/MacOS/PULSE" -EphemeralRun YES -HubURL "$console" -QuitAfter 60 \
  -Commands "1:go /records/;2:clicklink /tasks/;3:snapshot $scratch/tasks.png;4:rightclicklink /tasks/;6:rightclick 700 500;8:reload;16:snapshot $scratch/back.png;17:quit" >/dev/null 2>&1 &
pid=$!
note "app started (pid $pid)"
i=0
until [ -s "$scratch/tasks.png" ]; do
  kill -0 "$pid" 2>/dev/null || fail "the app ended before it reached Tasks"
  i=$((i + 1)); [ $i -lt 120 ] || fail "the app did not reach Tasks in 30 s"; sleep 0.25
done
note "tasks.png written"
stop_mock                                                 # the console goes away before the reload...
wait_logged "load failed" 20 || fail "the reload did not fail while the console was down"
note "the reload failed"
start_mock                                                # ...and comes back
i=0
while kill -0 "$pid" 2>/dev/null; do
  i=$((i + 1)); [ $i -lt 150 ] || fail "the app did not quit"; sleep 0.5
done
wait "$pid" && code=0 || code=$?
pid=""
note "app ended with $code"
[ "$code" = 0 ] || fail "the app exited with $code"
wait_logged "test hook: quit" 10 || true
/usr/bin/log show --start "$since" --predicate "$subsystem" --style compact \
  > "$scratch/app.log" 2>/dev/null || true

# Each message must appear, in this order: the whole message (expect) or its
# beginning (expect_prefix); a message is what follows the app's category.
match() {
  awk -v want="$2" -v how="$1" '
    found { print; next }
    {
      message = $0
      sub(/^.*\] \[io\.github\.manishtiwari25\.pulse:[a-z]+\] /, "", message)
      if ((how == "whole" && message == want) || (how == "prefix" && index(message, want) == 1)) found = 1
    }
    END { exit found ? 0 : 1 }' "$scratch/remaining.log" > "$scratch/rest.log" || fail "never logged: $2"
  mv "$scratch/rest.log" "$scratch/remaining.log"
  say "  ok: $2"
}
expect() { match whole "$1"; }
expect_prefix() { match prefix "$1"; }
cp "$scratch/app.log" "$scratch/remaining.log"
expect "state: ready($console)"
expect "showing $console"
expect "page /records"
expect "test hook: click lands on ConsoleWebView"
expect "page /tasks"
expect "snapshot written to $scratch/tasks.png"
expect "test hook: right-click lands on ConsoleWebView"
expect_prefix "context menu: Open in New Tab | Copy Link"
expect "test hook: right-click lands on ConsoleWebView"
expect "context menu: (none)"
expect_prefix "load failed"
expect_prefix "state: reconnecting($console"
expect "state: ready($console)"
expect "showing ${console}tasks/"
expect "snapshot written to $scratch/back.png"

for shot in tasks back; do
  size=$(wc -c < "$scratch/$shot.png" 2>/dev/null || echo 0)
  [ "$size" -gt 20000 ] || fail "$shot.png is missing or blank ($size bytes)"
done
say "PULSE.app check passed (snapshots in $scratch)"
