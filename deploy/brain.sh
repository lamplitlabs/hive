#!/bin/sh
# Build, start and delete brains in containers on a server of your own, one
# per team (docs/security.md).
#
#   deploy/brain.sh build VERSION          build hive-brain:VERSION from this tree
#   deploy/brain.sh up TEAM VERSION PORT   start team TEAM's brain, its hub on the host's 127.0.0.1:PORT
#   deploy/brain.sh delete TEAM --yes      stop TEAM's brain and delete its volume
#
# Each team gets its own container (hive-brain-TEAM) and its own volume
# (hive-brain-TEAM-data): a store, worktrees and keys never shared with
# another team. `delete` is "deleted on request": the container and the
# volume go; backups under backup.to are the operator's to remove (the
# script prints where they were going).
#
# The hub binds loopback only, and a container's loopback is its own, so a
# brain runs on the host's network: its hub is on the host's 127.0.0.1:PORT,
# reached over an SSH tunnel (ssh -L PORT:127.0.0.1:PORT host). Give each
# team its own PORT.
#
# Environment for `up`: HIVE_BACKUP_TO (az://ACCOUNT/CONTAINER/PREFIX; an
# az:// backup runs the Azure CLI, which the image does not hold: add it and
# a login to the image first),
# HIVE_UPDATE_LATEST_URL (the release document daemon.self_update release
# reads). DOCKER overrides the docker command (podman works).
set -eu

DOCKER="${DOCKER:-docker}"
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(dirname -- "$here")

die() {
	echo "brain.sh: $*" >&2
	exit 2
}

team_ok() {
	case "$1" in
	'' | *[!abcdefghijklmnopqrstuvwxyz0123456789-]* | -*) die "team names are lowercase letters, digits and dashes: '$1'" ;;
	esac
}

version_ok() {
	case "$1" in
	v[0-9]*.[0-9]*.[0-9]*) ;;
	*) die "VERSION is a release tag like v0.2.0: '$1'" ;;
	esac
}

cmd="${1:-}"
[ $# -gt 0 ] && shift
case "$cmd" in
build)
	[ $# -eq 1 ] || die "usage: brain.sh build VERSION"
	version_ok "$1"
	exec "$DOCKER" build -f "$here/Dockerfile" --build-arg "HIVE_VERSION=$1" -t "hive-brain:$1" "$root"
	;;
up)
	[ $# -eq 3 ] || die "usage: brain.sh up TEAM VERSION PORT"
	team_ok "$1"
	version_ok "$2"
	case "$3" in '' | *[!0-9]*) die "PORT is a number: '$3'" ;; esac
	exec "$DOCKER" run -d \
		--name "hive-brain-$1" \
		--restart unless-stopped \
		--cap-drop ALL --security-opt no-new-privileges:true \
		-v "hive-brain-$1-data:/data" \
		--network host -e "HIVE_HUB_PORT=$3" \
		-e "HIVE_BACKUP_TO=${HIVE_BACKUP_TO:-}" \
		-e "HIVE_UPDATE_LATEST_URL=${HIVE_UPDATE_LATEST_URL:-}" \
		"hive-brain:$2"
	;;
delete)
	[ $# -eq 2 ] && [ "$2" = "--yes" ] || die "usage: brain.sh delete TEAM --yes (deletes the team's store for good)"
	team_ok "$1"
	backup=$("$DOCKER" exec "hive-brain-$1" hive settings get backup.to 2>/dev/null || true)
	"$DOCKER" rm -f "hive-brain-$1" >/dev/null 2>&1 || true
	"$DOCKER" volume rm "hive-brain-$1-data"
	echo "deleted team $1: container hive-brain-$1 and volume hive-brain-$1-data"
	if [ -n "$backup" ]; then
		echo "backups went to $backup (hive-*.sqlite.gz); remove them there to finish the deletion"
	fi
	;;
*)
	die "usage: brain.sh build VERSION | up TEAM VERSION PORT | delete TEAM --yes"
	;;
esac
