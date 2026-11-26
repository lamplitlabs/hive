#!/bin/sh
# A check: the public site shows what main holds.
#
# hivefleet.dev is built from web/site, docs/ and scripts/install.sh. The
# workflow .github/workflows/deploy-site.yml deploys it on a push to main
# only while the repository holds three Vercel secrets; without them the
# site stays at the commit a person last deployed. This check closes that
# gap on a machine whose owner is signed in to Vercel (`npx vercel login`)
# with access to the project: when the public address shows an older commit
# than main for those paths, the runner builds main and deploys it, the way
# docs/development.md says ("The same three commands deploy from a
# checkout"), then reads the public address back.
#
# What it reads is SITE/commit.txt: the commit the public site was built
# from, which web/site/scripts/prebuild.mjs writes into every build of a
# clean tree.
#
#   up to date    the commit there is on the upstream's default branch and
#                 nothing under the site's paths changed since it.
#   cannot tell   the address does not answer, or answers with something
#                 that is not this site. A note: no commit can fix that.
#   behind        the paths changed since the commit there; or the commit
#                 is not on the default branch (someone deployed a branch);
#                 or this site is there without a commit.txt (a build from
#                 before this check). Main is deployed.
#
# Who deploys, and what:
# - Only the runner, in the fresh worktree of the default branch it makes
#   for a check (it sets HIVE_SCHEDULE). Run anywhere else - a person's
#   checkout, an agent's clone - this script only says what a run would do.
# - Only this repository: a fork, a mirror or a copy has another upstream.
# - Only a tree with nothing changed and nothing untracked, whose HEAD is
#   on the upstream's default branch: nothing reaches the public address
#   that is not on the forge already.
# - Only to the project named here: the environment's own VERCEL_ORG_ID and
#   VERCEL_PROJECT_ID (and their older NOW_ names), which would choose
#   another, are dropped.
# - Only a machine signed in to Vercel whose login reaches the project. Any
#   other does nothing and its check stays green - unless it deployed
#   before ($HIVE_STATE_DIR/deployed): then a lost login is a fault.
#
# Red (exit 1) means main could not be put on the public address twice in
# a row: `npm run check` or the installer's test fails on main, the build
# or the upload failed, the address still shows another commit after the
# upload, or a machine that deployed before can no longer. One failed run
# is a note, since one run can fail on the network alone; the next one
# that fails is red, and the runner queues one job with the end of the
# output. An address that has not answered for six hours is red too on a
# machine that deployed before.
#
#   .hive/automations/site-shows-main.sh          the check (site-shows-main.yaml runs it)
#   .hive/automations/site-shows-main.sh --plan   say what a run would do, and do nothing
#   hive schedule run SCHED-NNN                   make the check due now, on the runner
#
# Environment, all optional: SITE (https://hivefleet.dev), SITE_REPOSITORY
# (github.com/lamplitlabs/hive), SITE_VERCEL_PROJECT (hive), SITE_VERCEL_SCOPE
# (lamplitlabs), SITE_REMOTE (origin), SITE_WAIT_SECONDS (120, how long the
# read-back waits for the address), SITE_RETRY_SECONDS (15, the pause
# before a second upload; the third waits twice as long),
# SITE_UNANSWERED_RUNS (12, the runs in a row without an answer that turn
# the check red). The runner sets HIVE_SCHEDULE and HIVE_STATE_DIR, a
# directory that lasts between runs.
set -u

plan=
asked_plan=
case "${1:-}" in
  '') ;;
  --plan)
    plan=1
    asked_plan=1
    ;;
  *)
    echo "site-shows-main: unknown option $1 (use --plan, or nothing)" >&2
    exit 2
    ;;
esac
# Only the runner deploys.
[ -n "${HIVE_SCHEDULE:-}" ] || plan=1

# The project is the one named below, and the pages are this tree's docs/:
# neither is the environment's to choose.
unset VERCEL_ORG_ID VERCEL_PROJECT_ID NOW_ORG_ID NOW_PROJECT_ID HIVE_DOCS_DIR

say() { echo "site-shows-main: $*"; }
short() { printf '%.12s' "$1"; }

SITE=${SITE:-https://hivefleet.dev}
SITE=${SITE%/}
repository=$(printf '%s' "${SITE_REPOSITORY:-github.com/lamplitlabs/hive}" | tr '[:upper:]' '[:lower:]')
project=${SITE_VERCEL_PROJECT:-hive}
scope=${SITE_VERCEL_SCOPE:-lamplitlabs}
remote=${SITE_REMOTE:-origin}
wait_seconds=${SITE_WAIT_SECONDS:-120}
retry_seconds=${SITE_RETRY_SECONDS:-15}
unanswered_runs=${SITE_UNANSWERED_RUNS:-12}
state=${HIVE_STATE_DIR:-}
# What the site is built from, relative to the repository's root.
paths="web/site docs scripts/install.sh"

root=$(git rev-parse --show-toplevel 2> /dev/null) || {
  say "not inside a git checkout"
  exit 2
}
cd "$root" || exit 2
head=$(git rev-parse HEAD) || exit 2

# This check is for one repository. A fork, a mirror or a copy has another
# upstream, and its main is not what the site shows. The remote's address
# is read as host/owner/name, whichever way it is written: with a scheme
# or as host:path, with or without a user, a trailing slash or .git.
url=$(git remote get-url "$remote" 2> /dev/null | tr '[:upper:]' '[:lower:]')
url=${url%/}
url=${url%.git}
case "$url" in
  *://*) url=${url#*://} ;;
esac
case "$url" in
  *@*) url=${url##*@} ;;
esac
case "$url" in
  /* | */*:*) ;;
  *:*) url="${url%%:*}/${url#*:}" ;;
esac
if [ "$url" != "$repository" ]; then
  say "the remote $remote (${url:-none}) is not $repository, whose main the site shows. Nothing to do here."
  exit 0
fi

# When this machine last deployed, if it ever did.
deployed=
if [ -n "$state" ] && [ -s "$state/deployed" ]; then
  deployed=$(head -n 1 "$state/deployed")
fi

# Signed in to Vercel at all? Looked for on disk: asking the CLI would
# download it on every machine that tends this repository.
signed_in=
[ -n "${VERCEL_TOKEN:-}" ] && signed_in=1
for auth in \
  "${HOME:-}/Library/Application Support/com.vercel.cli/auth.json" \
  "${XDG_DATA_HOME:-${HOME:-}/.local/share}/com.vercel.cli/auth.json"; do
  [ -s "$auth" ] && signed_in=1
done
if [ -z "$signed_in" ] && [ -z "$deployed" ]; then
  say "this machine is not signed in to Vercel and never deployed the site. Nothing to do here."
  exit 0
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/site-shows-main.XXXXXX") || exit 2
trap 'rm -rf "$tmp"' EXIT
trap 'exit 130' INT TERM HUP

# keep NAME TEXT / forget NAME: a line that lasts between the runner's runs.
keep() {
  if [ -z "$plan" ] && [ -n "$state" ] && mkdir -p "$state" 2> /dev/null; then
    printf '%s\n' "$2" > "$state/$1"
  fi
}
forget() {
  if [ -z "$plan" ] && [ -n "$state" ]; then
    rm -f "$state/$1"
  fi
}

# cannot_tell WHY: the address gives no answer this check can read. A note,
# until it has lasted for SITE_UNANSWERED_RUNS runs on a machine that
# deployed before: then a person should look at the address.
cannot_tell() {
  n=0
  if [ -z "$plan" ] && [ -n "$state" ]; then
    n=$(cat "$state/unanswered" 2> /dev/null || echo 0)
    case "$n" in
      '' | *[!0-9]*) n=0 ;;
    esac
    n=$((n + 1))
    keep unanswered "$n"
  fi
  if [ -n "$deployed" ] && [ "$n" -ge "$unanswered_runs" ]; then
    say "$1, for $n runs in a row. This machine deployed the site before ($deployed), so someone should look at the address."
    exit 1
  fi
  say "$1, so this check cannot tell what the site shows. Nothing done."
  exit 0
}

# fail WHY: main could not be put on the address. One run can fail on the
# network alone, so the first is a note; another one within three hours is
# red. $state/failing holds when the first one was, in seconds.
fail() {
  say "$1"
  if [ -n "$state" ] && mkdir -p "$state" 2> /dev/null; then
    now=$(date +%s)
    before=$(cat "$state/failing" 2> /dev/null || echo 0)
    case "$before" in
      '' | *[!0-9]*) before=0 ;;
    esac
    if [ $((now - before)) -gt 10800 ]; then
      printf '%s\n' "$now" > "$state/failing"
      say "This is the first run to fail; the check is red if the next one fails too."
      exit 0
    fi
  fi
  exit 1
}

# installer_matches: the address serves this tree's scripts/install.sh,
# byte for byte. People pipe that file into a shell.
installer_matches() {
  [ -s "$root/scripts/install.sh" ] || return 0
  curl --fail --silent --location --max-time 30 --output "$tmp/install.sh" "$SITE/install.sh" 2> /dev/null &&
    cmp -s "$tmp/install.sh" "$root/scripts/install.sh"
}

# The commit the public address shows: forty hex digits, or nothing.
live=
status=$(curl --silent --location --max-time 20 --output "$tmp/commit" \
  --write-out '%{http_code}' "$SITE/commit.txt" 2> /dev/null) || status=000
case "$status" in
  200)
    live=$(tr -d '[:space:]' < "$tmp/commit" | cut -c 1-41)
    case "$live" in
      *[!0-9a-f]* | '') live= ;;
    esac
    [ "${#live}" -eq 40 ] || cannot_tell "$SITE/commit.txt answers with something that is not a commit"
    ;;
  404)
    # No commit.txt. This site from before the check serves the installer;
    # an address that points elsewhere does not.
    ours=
    if [ -s scripts/install.sh ] &&
      curl --fail --silent --location --max-time 30 --output "$tmp/installer" "$SITE/install.sh" 2> /dev/null &&
      [ "$(sed -n 2p "$tmp/installer")" = "$(sed -n 2p scripts/install.sh)" ]; then
      ours=1
    fi
    [ -n "$ours" ] || cannot_tell "$SITE has no commit.txt and does not serve this repository's installer"
    ;;
  *) cannot_tell "$SITE/commit.txt does not answer (status $status)" ;;
esac

branch=$(git symbolic-ref --quiet --short "refs/remotes/$remote/HEAD" 2> /dev/null)
branch=${branch#"$remote"/}
[ -n "$branch" ] || branch=main
upstream="refs/remotes/$remote/$branch"
known() { git cat-file -e "$1^{commit}" 2> /dev/null; }
on_upstream() { known "$1" && git merge-base --is-ancestor "$1" "$upstream" 2> /dev/null; }

# A commit that is not on the default branch as this checkout last fetched
# it is a newer main, or a deploy from a branch: fetch, and look again.
if [ -n "$live" ] && ! on_upstream "$live"; then
  if [ -n "$plan" ]; then
    say "$SITE shows $(short "$live"), which is not on $remote/$branch as last fetched: a run fetches first, then deploys main if the commit is still not on it."
    exit 0
  fi
  ssh=${GIT_SSH_COMMAND:-$(git config core.sshCommand 2> /dev/null)}
  if ! GIT_TERMINAL_PROMPT=0 GIT_SSH_COMMAND="${ssh:-ssh} -o BatchMode=yes -o ConnectTimeout=15" \
    git fetch --quiet "$remote" "$branch" 2> /dev/null; then
    cannot_tell "$SITE shows $(short "$live"), which is not on $remote/$branch as last fetched, and the fetch failed"
  fi
fi
forget unanswered

if [ -z "$live" ]; then
  why="$SITE has no commit.txt (a build from before this check)"
elif ! on_upstream "$live"; then
  why="$SITE shows $(short "$live"), which is not on $remote/$branch"
else
  # shellcheck disable=SC2086 # the paths are words on purpose
  if [ "$live" = "$head" ] || git diff --quiet "$live" "$head" -- $paths; then
    # On the machine that deploys, the installer is looked at every run.
    if [ -z "$plan" ] && [ -n "$deployed" ] && ! installer_matches; then
      fail "$SITE names $(short "$live") in commit.txt, but its install.sh is not the scripts/install.sh of that commit."
    fi
    say "$SITE shows $(short "$live"); nothing under $paths changed since. Up to date."
    forget failing
    exit 0
  fi
  if git merge-base --is-ancestor "$head" "$live"; then
    say "$SITE shows $(short "$live"), which is newer than this checkout ($(short "$head")). Nothing done."
    exit 0
  fi
  why="$SITE shows $(short "$live"), and $paths changed since"
fi

# Only what the upstream's default branch holds goes to the public address.
if ! git merge-base --is-ancestor "$head" "$upstream" 2> /dev/null; then
  say "$why, but HEAD $(short "$head") is not on $remote/$branch as last fetched: only the upstream's default branch is deployed. Nothing done."
  exit 0
fi
if [ -n "$(git status --porcelain --untracked-files=normal)" ]; then
  say "$why, but this tree has changed or untracked files: only a clean tree is deployed. Nothing done."
  exit 0
fi

if [ -n "$plan" ]; then
  if [ -z "$signed_in" ]; then
    say "$why: a run would deploy $(short "$head") from a machine signed in to Vercel; this one is not."
  elif [ -z "$asked_plan" ]; then
    say "$why: the runner would deploy $(short "$head"). Only the runner deploys (\`hive schedule run\` makes the check due now); nothing done here."
  else
    say "$why: would deploy $(short "$head")."
  fi
  exit 0
fi

# This machine cannot deploy: a note where it never did, a failure where it did.
cannot() {
  if [ -n "$deployed" ]; then
    fail "$why, and $1. This machine deployed the site before ($deployed), so the site no longer follows main: sign in again with \`npx vercel login\`."
  fi
  say "$why, but $1. Nothing is deployed from this machine."
  exit 0
}

# The Vercel CLI the deploy workflow pins, so the two deploy with one version.
cli=$(sed -n 's/^[[:space:]]*VERCEL_CLI:[[:space:]]*//p' .github/workflows/deploy-site.yml 2> /dev/null |
  head -n 1 | sed -e 's/[[:space:]]*#.*$//' -e 's/[[:space:]]*$//' -e "s/^[\"']//" -e "s/[\"']$//")
case "$cli" in
  vercel@*[!0-9.]* | vercel@ | vercel@.*) cli= ;;
  vercel@[0-9]*) ;;
  *) cli= ;;
esac
if [ -z "$cli" ]; then
  cli=vercel@65.14.0
  say "the VERCEL_CLI line of .github/workflows/deploy-site.yml could not be read; using $cli."
fi
vercel() { npx --yes "$cli" "$@"; }

[ -n "$signed_in" ] || cannot "this machine is not signed in to Vercel"
command -v npx > /dev/null 2>&1 || cannot "this machine has no npx (Node.js)"
if ! vercel whoami > "$tmp/step.log" 2>&1; then
  tail -n 10 "$tmp/step.log"
  cannot "this machine's Vercel login was not accepted (the lines above are the CLI's)"
fi

# step NAME COMMAND...: runs it, and on a failure shows the end of what it
# printed and ends the run.
step() {
  name=$1
  shift
  say "$name"
  if ! "$@" > "$tmp/step.log" 2>&1; then
    tail -n 40 "$tmp/step.log"
    fail "$name failed. Nothing was uploaded."
  fi
}

say "$why: deploying $(short "$head")."
cd web/site || exit 2
if ! vercel link --yes --project "$project" --scope "$scope" > "$tmp/step.log" 2>&1; then
  tail -n 10 "$tmp/step.log"
  cannot "this machine's Vercel login cannot reach the project $project of $scope"
fi
# The link leaves the development environment's variables in .env.local;
# the build reads the production ones `vercel pull` fetches. It can also
# add a line of its own to .gitignore, a tracked file: put back, because
# the build is to be of what the commit holds.
rm -f .env.local
git checkout --quiet -- .gitignore
# People pipe /install.sh into a shell: its own test passes before a
# changed installer goes out, as in the workflow.
if [ -f "$root/scripts/tests/install-test.sh" ]; then
  if [ -z "$live" ] || ! known "$live" ||
    ! git diff --quiet "$live" "$head" -- "$root/scripts/install.sh" "$root/scripts/tests/install-test.sh"; then
    step "the installer's test" sh "$root/scripts/tests/install-test.sh"
  fi
fi
step "npm ci" npm ci --no-audit --no-fund
step "npm run check" npm run check
step "vercel pull" vercel pull --yes --environment=production
touched=$(git status --porcelain --untracked-files=normal)
if [ -n "$touched" ]; then
  echo "$touched"
  fail "the steps above changed or added files git sees, so a build here would not be what $(short "$head") holds. Nothing was uploaded."
fi
step "vercel build" vercel build --prod
# What goes up must say which commit it is, or the next run could not tell
# and would deploy again.
stamp=$(find .vercel/output -name commit.txt -type f 2> /dev/null | head -n 1)
if [ -z "$stamp" ] || [ "$(tr -d '[:space:]' < "$stamp")" != "$head" ]; then
  fail "the build holds no commit.txt for $(short "$head"). Nothing was uploaded."
fi

attempt=1
while :; do
  say "vercel deploy (attempt $attempt of 3)"
  if vercel deploy --prebuilt --prod > "$tmp/step.log" 2>&1; then
    break
  fi
  tail -n 20 "$tmp/step.log"
  if [ "$attempt" -ge 3 ]; then
    fail "the upload failed three times."
  fi
  sleep $((attempt * retry_seconds))
  attempt=$((attempt + 1))
done
# From here on this machine is one that deploys the site.
keep deployed "$head at $(date -u +%Y-%m-%dT%H:%M:%SZ)"
deployed="$head just now"

# The public address, not the deployment's own, is what a reader gets.
say "reading $SITE back"
deadline=$(($(date +%s) + wait_seconds))
while :; do
  got=$(curl --silent --location --max-time 20 "$SITE/commit.txt" 2> /dev/null | tr -d '[:space:]' | cut -c 1-41)
  [ "$got" = "$head" ] && break
  if [ "$(date +%s)" -ge "$deadline" ]; then
    fail "the upload went through, but $SITE/commit.txt still shows '${got:-nothing}' after ${wait_seconds}s, not $(short "$head"): the address may not point at the project $project."
  fi
  sleep 5
done
if ! installer_matches; then
  fail "$SITE/install.sh is not the scripts/install.sh of $(short "$head"), though commit.txt names it."
fi

forget failing
say "$SITE shows $(short "$head") now, read back with its installer."
exit 0
