#!/bin/sh
# The entrypoint of the brain's container (deploy/Dockerfile, docs/security.md).
#
# 1. The first start makes an empty store under HIVE_HOME (`hive init
#    --store-only`: no project is registered; the owner adds them from the
#    console once hands dial in).
# 2. Every start pins the two settings a brain in this image must hold:
#      brain.code         none     no project clone on the brain
#      daemon.self_update release  update from published releases, never build
#    and, when HIVE_BACKUP_TO is set, backup.to (az://ACCOUNT/CONTAINER/PREFIX;
#    an az:// backup needs the Azure CLI, which the image does not hold).
# 3. The hub and the daemon run side by side. Each exits 3 when the hive
#    binary changes (a release was installed); this script starts it again,
#    as the user service does on a laptop. Any other exit of either ends the
#    container, so the host's restart policy and its health check see it.
set -eu

HIVE_HOME="${HIVE_HOME:-/data/hive}"
HIVE_HUB_PORT="${HIVE_HUB_PORT:-8765}"
export HIVE_HOME

hive_bin() { command -v hive; }

if [ ! -f "$HIVE_HOME/hive.sqlite" ]; then
	mkdir -p "$HIVE_HOME"
	hive init --store-only
fi

hive settings set brain.code none >/dev/null
hive settings set daemon.self_update release >/dev/null
if [ -n "${HIVE_UPDATE_LATEST_URL:-}" ]; then
	hive settings set update.latest_url "$HIVE_UPDATE_LATEST_URL" >/dev/null
fi
if [ -n "${HIVE_BACKUP_TO:-}" ]; then
	hive settings set backup.to "$HIVE_BACKUP_TO" >/dev/null
fi

# keep NAME CMD...: run CMD again while it exits 3 (the binary changed);
# return its exit code otherwise.
keep() {
	name="$1"
	shift
	while :; do
		set +e
		"$(hive_bin)" "$@"
		code=$?
		set -e
		if [ "$code" -ne 3 ]; then
			echo "hive-brain: $name exited $code" >&2
			return "$code"
		fi
		echo "hive-brain: $name restarts on the new hive binary" >&2
		# A daemon also exits 3 when it refuses to start beside a live one;
		# a pause keeps that from spinning.
		sleep 2
	done
}

keep hub hub --port "$HIVE_HUB_PORT" --no-browser --restart-on-change &
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
