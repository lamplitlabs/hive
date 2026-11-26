#!/bin/sh
# Tests .hive/automations/site-shows-main.sh in a scratch repository, with
# stand-ins for curl, npx and npm first on PATH: what the check decides (up
# to date, cannot tell, behind), what it refuses to deploy and where, the
# order of its steps, and when a machine that cannot deploy is green or
# red. Nothing leaves the machine and nothing is installed; it needs git
# and takes some seconds.
#
#   sh scripts/tests/site-shows-main-test.sh
#   HIVE_TEST_SHELL=/bin/dash sh scripts/tests/site-shows-main-test.sh    the shell the script runs under (default: sh)
#
# It prints one line a case and a count, and exits 1 when a case failed.
set -u

here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd) || exit 2
script="$here/../../.hive/automations/site-shows-main.sh"
if [ ! -f "$script" ]; then
  echo "site-shows-main-test: no file at $script" >&2
  exit 2
fi
test_shell=${HIVE_TEST_SHELL:-sh}

# Git must see the scratch repository alone: not the repository this test
# runs in (a commit hook exports GIT_DIR and GIT_INDEX_FILE), and not this
# machine's own git settings (a global hooks path would run its hooks here).
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES \
  GIT_COMMON_DIR GIT_PREFIX GIT_NAMESPACE
GIT_CONFIG_GLOBAL=/dev/null
GIT_CONFIG_SYSTEM=/dev/null
GIT_CONFIG_NOSYSTEM=1
export GIT_CONFIG_GLOBAL GIT_CONFIG_SYSTEM GIT_CONFIG_NOSYSTEM

work=$(mktemp -d "${TMPDIR:-/tmp}/site-shows-main-test.XXXXXX") || exit 2
work=$(CDPATH='' cd -- "$work" && pwd -P) || exit 2
trap 'rm -rf "$work"' EXIT
trap 'exit 130' INT TERM HUP

stub=$work/stub
bin=$work/bin
home=$work/home
state=$work/state
repo=$work/repo
# The upstream, at a path that ends as the real one does.
origin=$work/lamplitlabs/hive.git
mkdir -p "$stub" "$bin" "$home" "$work/lamplitlabs"

# curl: the public address. $stub/status is the HTTP status of /commit.txt
# (000: no answer), $stub/live what it serves, $stub/install.sh the installer.
cat > "$bin/curl" << 'EOF'
#!/bin/sh
out= ; code= ; url=
while [ $# -gt 0 ]; do
  case "$1" in
    --output) out=$2; shift ;;
    --write-out) code=1; shift ;;
    --max-time) shift ;;
    -*) ;;
    *) url=$1 ;;
  esac
  shift
done
echo "curl $url" >> "$STUB/calls"
emit() { if [ -n "$out" ]; then cat "$1" > "$out"; else cat "$1"; fi; }
case "$url" in
  */commit.txt)
    status=$(cat "$STUB/status" 2> /dev/null || echo 200)
    if [ "$status" = 000 ]; then
      [ -n "$code" ] && printf 000
      exit 6
    fi
    if [ "$status" = 200 ]; then emit "$STUB/live"; else : > "${out:-/dev/null}"; fi
    [ -n "$code" ] && printf '%s' "$status"
    exit 0
    ;;
  */install.sh)
    [ -f "$STUB/install.sh" ] || exit 22
    emit "$STUB/install.sh"
    exit 0
    ;;
esac
exit 22
EOF

# npx: the Vercel CLI. A file $stub/fail-<subcommand> makes that one fail;
# $stub/deploy-fails holds how many uploads fail before one goes through.
# As the real one does, link leaves a .env.local and can add a line to the
# tracked .gitignore. pull, build and deploy note the variables that would
# choose another project. build stamps its output with HEAD when the tree
# is clean, as prebuild.mjs does, unless $stub/no-stamp. A deploy puts HEAD
# and the installer on the "public address", unless $stub/unreached says
# the address points elsewhere. $stub/pull-edits makes pull change a
# tracked file, which no real step does today.
cat > "$bin/npx" << 'EOF'
#!/bin/sh
echo "npx $*" >> "$STUB/calls"
[ "$1" = --yes ] && shift
shift
sub=$1
[ -f "$STUB/fail-$sub" ] && { echo "vercel $sub: failed on purpose"; exit 1; }
case "$sub" in
  pull | build | deploy)
    env | grep -E '^(VERCEL_ORG_ID|VERCEL_PROJECT_ID|NOW_ORG_ID|NOW_PROJECT_ID|HIVE_DOCS_DIR)=' >> "$STUB/env-seen"
    ;;
esac
if [ "$sub" = link ]; then
  echo .vercel >> .gitignore
  echo SECRET=dev > .env.local
fi
if [ "$sub" = pull ] && [ -f "$STUB/pull-edits" ]; then
  echo '{"edited":true}' > package.json
fi
if [ "$sub" = build ]; then
  { git status --porcelain; ls .env.local 2> /dev/null; } > "$STUB/tree-at-build"
  rm -rf .vercel/output
  mkdir -p .vercel/output/static
  if [ ! -f "$STUB/no-stamp" ] && [ -z "$(git status --porcelain)" ]; then
    git rev-parse HEAD > .vercel/output/static/commit.txt
  fi
fi
if [ "$sub" = deploy ]; then
  left=$(cat "$STUB/deploy-fails" 2> /dev/null || echo 0)
  if [ "$left" -gt 0 ]; then
    echo $((left - 1)) > "$STUB/deploy-fails"
    echo "vercel deploy: failed on purpose"
    exit 1
  fi
  if [ ! -f "$STUB/unreached" ]; then
    git rev-parse HEAD > "$STUB/live"
    echo 200 > "$STUB/status"
    if [ ! -f "$STUB/keep-installer" ]; then
      cp "$(git rev-parse --show-toplevel)/scripts/install.sh" "$STUB/install.sh"
    fi
  fi
fi
exit 0
EOF

cat > "$bin/npm" << 'EOF'
#!/bin/sh
echo "npm $*" >> "$STUB/calls"
[ -f "$STUB/fail-npm-$1" ] && { echo "npm $1: failed on purpose"; exit 1; }
exit 0
EOF
chmod +x "$bin/curl" "$bin/npx" "$bin/npm"
# The same stand-ins without npx, for a machine that has no Node.js.
mkdir -p "$work/bin-no-npx"
cp "$bin/curl" "$bin/npm" "$work/bin-no-npx/"

g() { git -C "$repo" -c user.name=test -c user.email=test@localhost -c commit.gpgsign=false "$@"; }

git init --quiet --bare "$origin" || exit 2
git init --quiet "$repo" || exit 2
g checkout --quiet -b main || exit 2
mkdir -p "$repo/web/site" "$repo/docs" "$repo/scripts/tests" "$repo/crates" "$repo/.github/workflows"
echo '{}' > "$repo/web/site/package.json"
printf '/out/\n.vercel\n.env*\n' > "$repo/web/site/.gitignore"
echo one > "$repo/docs/a.md"
printf '#!/bin/sh\n# The scratch installer\necho installer\n' > "$repo/scripts/install.sh"
cat > "$repo/scripts/tests/install-test.sh" << 'EOF'
#!/bin/sh
echo "installer-test" >> "$STUB/calls"
[ -f "$STUB/fail-installer-test" ] && { echo "the installer's test: failed on purpose"; exit 1; }
exit 0
EOF
echo one > "$repo/crates/x.rs"
printf 'env:\n  VERCEL_CLI: vercel@9.9.9\n' > "$repo/.github/workflows/deploy-site.yml"
g add -A && g commit --quiet -m one
c1=$(g rev-parse HEAD)
echo two > "$repo/crates/x.rs"
g commit --quiet -am two
c2=$(g rev-parse HEAD)
echo three > "$repo/docs/a.md"
g commit --quiet -am three
c3=$(g rev-parse HEAD)
g remote add origin "$origin"
g push --quiet origin main
g fetch --quiet origin
g remote set-head origin main > /dev/null || exit 2
# A commit on a branch of its own, never on main.
g checkout --quiet -b side "$c2"
echo side > "$repo/docs/a.md"
g commit --quiet -am side
side=$(g rev-parse HEAD)
g checkout --quiet main

signed_in() {
  mkdir -p "$home/Library/Application Support/com.vercel.cli" "$home/.local/share/com.vercel.cli"
  echo '{"token":"x"}' > "$home/Library/Application Support/com.vercel.cli/auth.json"
  echo '{"token":"x"}' > "$home/.local/share/com.vercel.cli/auth.json"
}
signed_out() { rm -rf "$home/Library" "$home/.local"; }
deployed_before() { mkdir -p "$state" && echo "$c1 at 2026-01-01T00:00:00Z" > "$state/deployed"; }

# fresh STATUS LIVE: the address as a case starts, serving this repository's
# installer; nothing recorded; signed in; the scratch repository at c3.
fresh() {
  rm -rf "$stub" "$state"
  mkdir -p "$stub"
  echo "$1" > "$stub/status"
  printf '%s\n' "$2" > "$stub/live"
  g checkout --quiet main
  g reset --quiet --hard "$c3"
  g clean --quiet -fdx
  g update-ref refs/remotes/origin/main "$c3"
  cp "$repo/scripts/install.sh" "$stub/install.sh"
  signed_in
  path_now=$bin
  dir_now=$repo
  schedule_now=SCHED-900
  repository_now=$work/lamplitlabs/hive
  g remote set-url origin "$origin"
  g config --unset status.showUntrackedFiles 2> /dev/null
  [ -d "$origin" ] || mv "$origin.away" "$origin"
}

# run [--plan]: the script as the runner starts it (HIVE_SCHEDULE set) in
# $dir_now; sets $code and $out. $schedule_now empty is a run by hand;
# $repository_now empty leaves the script its own repository, on GitHub.
run() {
  out=$(cd "$dir_now" && unset VERCEL_TOKEN XDG_DATA_HOME SITE_VERCEL_PROJECT SITE_VERCEL_SCOPE SITE_REMOTE \
    SITE_REPOSITORY HIVE_SCHEDULE &&
    if [ -n "$schedule_now" ]; then HIVE_SCHEDULE=$schedule_now && export HIVE_SCHEDULE; fi &&
    if [ -n "$repository_now" ]; then SITE_REPOSITORY=$repository_now && export SITE_REPOSITORY; fi &&
    PATH="$path_now:$PATH" HOME="$home" STUB="$stub" \
      VERCEL_ORG_ID=team_other VERCEL_PROJECT_ID=prj_other NOW_ORG_ID=team_old NOW_PROJECT_ID=prj_old \
      HIVE_DOCS_DIR=/somewhere/else \
      SITE=https://site.test SITE_WAIT_SECONDS=1 SITE_RETRY_SECONDS=0 SITE_UNANSWERED_RUNS=3 \
      HIVE_STATE_DIR="$state" \
      "$test_shell" "$script" "$@" 2>&1)
  code=$?
}

# calls PREFIX: how many of the stand-ins' calls start with PREFIX.
calls() {
  if [ -f "$stub/calls" ]; then
    grep -c "^$1" "$stub/calls" || true
  else
    echo 0
  fi
}

passed=0
failed=0
# check NAME WANT_CODE WANT_TEXT [CONDITION...]: the last run ended with
# WANT_CODE, said WANT_TEXT, and every CONDITION (a shell test) holds.
check() {
  name=$1
  want_code=$2
  want_text=$3
  shift 3
  ok=1
  [ "$code" = "$want_code" ] || ok=
  case "$out" in
    *"$want_text"*) ;;
    *) ok= ;;
  esac
  for cond in "$@"; do
    eval "$cond" || ok=
  done
  if [ -n "$ok" ]; then
    passed=$((passed + 1))
    echo "ok    $name"
  else
    failed=$((failed + 1))
    echo "FAIL  $name: exit $code (want $want_code), want the text '$want_text'${*:+, and: $*}"
    echo "$out" | sed 's/^/        /'
    [ -f "$stub/calls" ] && sed 's/^/        > /' "$stub/calls"
  fi
}
none='[ "$(calls npx)" = 0 ] && [ "$(calls npm)" = 0 ]'
# deploys: how many uploads were tried.
deploys() { calls "npx --yes vercel@9.9.9 deploy"; }

# What the check decides.

fresh 200 "$c3"
run
check "the address shows HEAD: up to date" 0 "Up to date" "$none"

fresh 200 "$c1"
g reset --quiet --hard "$c2"
run
check "only other paths changed since the commit there: up to date" 0 "Up to date" "$none"

fresh 200 "$c3"
g reset --quiet --hard "$c2"
run
check "the address is newer than this checkout: nothing done" 0 "newer than this checkout" "$none"

fresh 200 "$c1"
run --plan
check "--plan says what it would do and does nothing" 0 "would deploy" "$none" '[ ! -e "$state" ]'

fresh 000 ""
run
check "the address does not answer: cannot tell, green" 0 "cannot tell" "$none"

fresh 200 "<html>a parked page</html>"
run
check "the address serves another site: cannot tell, green" 0 "cannot tell" "$none"

fresh 404 ""
echo '<html>a parked page</html>' > "$stub/install.sh"
run
check "no commit.txt and not this repository's installer: cannot tell, green" 0 "does not serve this repository's installer" "$none"

fresh 404 ""
rm -f "$stub/install.sh"
run
check "no commit.txt and no installer at all: cannot tell, green" 0 "cannot tell" "$none"

fresh 000 ""
deployed_before
run
run
check "no answer twice on a machine that deployed before: still a note" 0 "cannot tell" "$none"
run
check "no answer for the third run in a row there: red" 1 "someone should look at the address" "$none"
echo 200 > "$stub/status"
printf '%s\n' "$c3" > "$stub/live"
run
check "an answer ends the count" 0 "Up to date" '[ ! -e "$state/unanswered" ]'

# A deploy, start to end.

fresh 200 "$c1"
run
order=$(grep -E '^(npx|npm|installer-test)' "$stub/calls" | sed -e 's/^npx --yes vercel@9.9.9 /vercel /' | cut -d' ' -f1-2 | tr '\n' ',')
check "behind: main is deployed, in order, with the CLI the workflow pins" 0 "shows $(printf '%.12s' "$c3") now" \
  '[ "$order" = "vercel whoami,vercel link,npm ci,npm run,vercel pull,vercel build,vercel deploy," ]' \
  'grep -q "^npx --yes vercel@9.9.9 link --yes --project hive --scope lamplitlabs$" "$stub/calls"' \
  'grep -q "^npx --yes vercel@9.9.9 pull --yes --environment=production$" "$stub/calls"' \
  'grep -q "^npx --yes vercel@9.9.9 deploy --prebuilt --prod$" "$stub/calls"' \
  'grep -q "^$c3 at " "$state/deployed"' \
  '[ -f "$stub/tree-at-build" ] && [ ! -s "$stub/tree-at-build" ]' \
  '[ ! -s "$stub/env-seen" ]' \
  '[ -z "$(g status --porcelain)" ]'

fresh 404 ""
run
check "this site without a commit.txt (a build from before the check): deployed" 0 "has no commit.txt" \
  '[ "$(deploys)" = 1 ]' '[ "$(calls installer-test)" = 1 ]'

fresh 200 "$c1"
echo 'echo a newer installer' >> "$repo/scripts/install.sh"
g commit --quiet -am "a newer installer"
g push --quiet origin main
g fetch --quiet origin
run
check "the installer changed since the commit there: its own test runs first" 0 "now, read back" \
  '[ "$(calls installer-test)" = 1 ]' '[ "$(deploys)" = 1 ]'
g push --quiet --force origin "$c3:main"
g fetch --quiet origin

fresh 200 "$c1"
echo 'echo a newer installer' >> "$repo/scripts/install.sh"
g commit --quiet -am "a newer installer"
g update-ref refs/remotes/origin/main HEAD
touch "$stub/fail-installer-test"
run
check "the installer's test fails: nothing built, nothing uploaded" 0 "the installer's test failed" \
  '[ "$(calls npm)" = 0 ]' '[ "$(deploys)" = 0 ]'

fresh 200 "$side"
run
check "the address shows a commit of a branch, not of main: main is deployed" 0 "is not on origin/main" '[ "$(deploys)" = 1 ]'

fresh 200 "0000000000000000000000000000000000000000"
run
check "the address shows a commit the upstream does not have: main is deployed" 0 "is not on origin/main" '[ "$(deploys)" = 1 ]'

fresh 200 "$side"
run --plan
check "--plan with a commit that is not on main: says a run fetches first" 0 "a run fetches first" "$none"

fresh 200 "$side"
deployed_before
mv "$origin" "$origin.away"
run
check "a commit that is not on main, and the fetch fails: cannot tell" 0 "the fetch failed" "$none"
run
run
check "and red after three such runs on a machine that deployed before" 1 "someone should look at the address" "$none"

fresh 200 "$c3"
g reset --quiet --hard "$c2"
g update-ref refs/remotes/origin/main "$c2"
run
check "the address shows a main newer than the last fetch: fetched, and nothing done" 0 "newer than this checkout" "$none"

fresh 200 "$c1"
git -C "$repo" worktree add --quiet --detach "$work/tree" "$c3"
dir_now=$work/tree
run
check "in a detached worktree, as the runner makes one: deployed" 0 "now, read back" '[ "$(deploys)" = 1 ]'
git -C "$repo" worktree remove --force "$work/tree"

fresh 200 "$c1"
echo 1 > "$stub/deploy-fails"
run
check "the first upload fails, the second goes through" 0 "now, read back" '[ "$(deploys)" = 2 ]'

fresh 200 "$c1"
printf 'env:\n  VERCEL_CLI: "vercel@8.8.8"  # pinned\n' > "$repo/.github/workflows/deploy-site.yml"
g commit --quiet -am "a quoted pin with a comment"
g update-ref refs/remotes/origin/main HEAD
run
check "a quoted pin with a comment is read as the version" 0 "now, read back" \
  'grep -q "^npx --yes vercel@8.8.8 deploy --prebuilt --prod$" "$stub/calls"'

fresh 200 "$c1"
printf 'env:\n  VERCEL_CLI: latest\n' > "$repo/.github/workflows/deploy-site.yml"
g commit --quiet -am "a pin that is no version"
g update-ref refs/remotes/origin/main HEAD
run
check "a pin that is no version: the built-in one, and it says so" 0 "could not be read" \
  'grep -q "^npx --yes vercel@65.14.0 deploy --prebuilt --prod$" "$stub/calls"'

# What it refuses to deploy, and where.

fresh 200 "$c1"
schedule_now=
run
check "run by hand: only the runner deploys" 0 "Only the runner deploys" "$none" '[ ! -e "$state" ]'

for address in "git@github.com:someone/hive.git" "https://evil.example/lamplitlabs/hive.git" \
  "$work/lamplitlabs/hive.git" "ssh://git@github.com:22/lamplitlabs/hive"; do
  fresh 200 "$c1"
  repository_now=
  g remote set-url origin "$address"
  run
  check "another repository's clone (${address#"$work"}): nothing, and the address is not even asked" 0 \
    "is not github.com/lamplitlabs/hive" "$none" '[ "$(calls curl)" = 0 ]'
done

for address in "git@github.com:lamplitlabs/hive.git" "https://github.com/lamplitlabs/hive" \
  "https://someone:secret@github.com/LamplitLabs/Hive.git/" "ssh://git@github.com/lamplitlabs/hive"; do
  fresh 200 "$c3"
  repository_now=
  g remote set-url origin "$address"
  run
  check "this repository, written as $address: the check goes on" 0 "Up to date" "$none"
done

fresh 200 "$c1"
echo four > "$repo/docs/a.md"
g commit --quiet -am "four, not pushed"
run
check "HEAD is not on the upstream's default branch: nothing deployed" 0 "only the upstream's default branch" "$none"

fresh 200 "$c1"
echo dirty > "$repo/docs/a.md"
run
check "a changed file: nothing deployed" 0 "only a clean tree" "$none"

fresh 200 "$c1"
echo new > "$repo/docs/untracked.md"
run
check "an untracked page: nothing deployed" 0 "only a clean tree" "$none"

fresh 200 "$c1"
echo new > "$repo/docs/untracked.md"
g config status.showUntrackedFiles no
run
check "an untracked page that this machine's git settings hide: nothing deployed" 0 "only a clean tree" "$none"

fresh 200 "$c1"
touch "$stub/pull-edits"
run
check "a step changed a tracked file: nothing built or uploaded" 0 "changed or added files" \
  '[ "$(calls "npx --yes vercel@9.9.9 build")" = 0 ]' '[ "$(deploys)" = 0 ]'

fresh 200 "$c1"
touch "$stub/no-stamp"
run
check "the build holds no commit.txt: nothing uploaded" 0 "holds no commit.txt" '[ "$(deploys)" = 0 ]'

# A machine that cannot deploy.

fresh 200 "$c1"
signed_out
run
check "not signed in to Vercel, never deployed here: nothing, and the address is not even asked" 0 \
  "Nothing to do here" "$none" '[ "$(calls curl)" = 0 ]'

fresh 200 "$c1"
signed_out
deployed_before
run
check "not signed in, but this machine deployed before: a note the first time" 0 "sign in again" "$none"
run
check "and red the second time" 1 "sign in again" "$none"

fresh 200 "$c1"
touch "$stub/fail-whoami"
run
check "the login is not accepted, never deployed here: green, with the CLI's own words" 0 "failed on purpose" \
  '[ "$(calls npm)" = 0 ]' '[ ! -e "$state/failing" ]'

# Only where the system directories hold git and no npx of their own: the
# case must never reach a real npx.
if (PATH=/usr/bin:/bin && command -v git > /dev/null 2>&1 && ! command -v npx > /dev/null 2>&1); then
  fresh 200 "$c1"
  path_now=$work/bin-no-npx
  real_path=$PATH
  PATH=/usr/bin:/bin
  run
  PATH=$real_path
  check "no npx on this machine: green, nothing built" 0 "has no npx" '[ "$(calls npm)" = 0 ]'
else
  echo "skip  no npx on this machine (the system directories hold an npx, or no git)"
fi

fresh 200 "$c1"
touch "$stub/fail-link"
run
check "the login cannot reach the project: green, nothing built" 0 "cannot reach the project" '[ "$(calls npm)" = 0 ]'

# Failures: a note the first time, red the second.

fresh 200 "$c1"
touch "$stub/fail-npm-run"
run
check "npm run check fails on main: a note the first time, nothing uploaded" 0 "the check is red if the next one fails too" \
  '[ "$(deploys)" = 0 ]' '[ ! -f "$state/deployed" ]' '[ -e "$state/failing" ]'
run
check "and red the second time" 1 "npm run check failed" '[ "$(deploys)" = 0 ]'
rm -f "$stub/fail-npm-run"
run
check "a run that goes through ends the count" 0 "now, read back" '[ ! -e "$state/failing" ]'

fresh 200 "$c1"
touch "$stub/fail-npm-run"
mkdir -p "$state" && echo $(($(date +%s) - 20000)) > "$state/failing"
run
check "a failure more than three hours after the last one counts as the first" 0 "the check is red if the next one fails too"

fresh 200 "$c1"
touch "$stub/fail-pull"
run
run
check "vercel pull fails twice: red" 1 "vercel pull failed" '[ "$(deploys)" = 0 ]'

fresh 200 "$c1"
touch "$stub/fail-build"
run
run
check "vercel build fails twice: red" 1 "vercel build failed" '[ "$(deploys)" = 0 ]'

fresh 200 "$c1"
echo 9 > "$stub/deploy-fails"
run
check "the upload fails three times: a note the first time" 0 "the upload failed three times" \
  '[ "$(deploys)" = 3 ]' '[ ! -f "$state/deployed" ]'
run
check "and red the second time" 1 "the upload failed three times" '[ "$(deploys)" = 6 ]'

fresh 200 "$c1"
touch "$stub/unreached"
run
run
check "uploaded twice, and the address still shows the old commit: red" 1 "still shows" '[ "$(deploys)" = 2 ]'

fresh 200 "$c1"
echo 'echo another installer' > "$stub/install.sh"
touch "$stub/keep-installer"
run
check "the address serves another installer after the upload: a note the first time" 0 "install.sh is not" '[ -s "$state/deployed" ]'
run
check "and red the second time, though commit.txt names HEAD" 1 "install.sh is not" '[ "$(deploys)" = 1 ]'

fresh 200 "$c3"
echo 'echo another installer' > "$stub/install.sh"
deployed_before
run
run
check "up to date by commit.txt, but another installer is served, twice, where this machine deploys: red" 1 \
  "install.sh is not" "$none"

fresh 200 "$c3"
echo 'echo another installer' > "$stub/install.sh"
run
check "the same on a machine that never deployed: not its business" 0 "Up to date" \
  '[ "$(calls "curl https://site.test/install.sh")" = 0 ]'

fresh 200 "$c3"
run --nope
check "an unknown option is refused" 2 "unknown option" "$none"

echo "site-shows-main-test: $passed passed, $failed failed"
[ "$failed" -eq 0 ]
