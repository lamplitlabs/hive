#!/bin/sh
# The hosted brain's entrypoint (deploy/Dockerfile, docs/security.md).
#
# 1. The first start makes an empty store under PULSE_HOME (`pulse init
#    --store-only`: no project is registered; the owner adds them from the
#    console once hands dial in).
# 2. Every start pins the two settings a hosted brain must hold:
#      brain.code         none     no project clone on the brain
#      daemon.self_update release  update from published releases, never build
#    and, when PULSE_BACKUP_TO is set, backup.to (az://ACCOUNT/CONTAINER/PREFIX).
# 3. The hub and the daemon run side by side. Each exits 3 when the pulse
#    binary changes (a release was installed); this script starts it again,
#    as the user service does on a laptop. Any other exit of either ends the
#    container, so the host's restart policy and its health check see it.
set -eu

PULSE_HOME="${PULSE_HOME:-/data/pulse}"
PULSE_HUB_PORT="${PULSE_HUB_PORT:-8765}"
export PULSE_HOME

pulse_bin() { command -v pulse; }

if [ ! -f "$PULSE_HOME/pulse.sqlite" ]; then
	mkdir -p "$PULSE_HOME"
	pulse init --store-only
fi

pulse settings set brain.code none >/dev/null
pulse settings set daemon.self_update release >/dev/null
if [ -n "${PULSE_UPDATE_LATEST_URL:-}" ]; then
	pulse settings set update.latest_url "$PULSE_UPDATE_LATEST_URL" >/dev/null
fi
if [ -n "${PULSE_BACKUP_TO:-}" ]; then
	pulse settings set backup.to "$PULSE_BACKUP_TO" >/dev/null
fi

# keep NAME CMD...: run CMD again while it exits 3 (the binary changed);
# return its exit code otherwise.
keep() {
	name="$1"
	shift
	while :; do
		set +e
		"$(pulse_bin)" "$@"
		code=$?
		set -e
		if [ "$code" -ne 3 ]; then
			echo "pulse-brain: $name exited $code" >&2
			return "$code"
		fi
		echo "pulse-brain: $name restarts on the new pulse binary" >&2
		# A daemon also exits 3 when it refuses to start beside a live one;
		# a pause keeps that from spinning.
		sleep 2
	done
}

keep hub hub --port "$PULSE_HUB_PORT" --no-browser --restart-on-change &
hub=$!
keep daemon daemon &
daemon=$!

stop() {
	kill -TERM "$hub" "$daemon" 2>/dev/null || true
	wait "$hub" "$daemon" 2>/dev/null || true
	exit 0
}
trap stop TERM INT

# Whichever loop ends first ends the brain.
while kill -0 "$hub" 2>/dev/null && kill -0 "$daemon" 2>/dev/null; do
	sleep 5
done
kill -TERM "$hub" "$daemon" 2>/dev/null || true
wait "$hub" 2>/dev/null || true
wait "$daemon" 2>/dev/null || true
exit 1
