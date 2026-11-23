//! After Hive pushes a project's default branch (`land.pushed`) or puts it
//! on a pull request (`land.pr`), GitHub runs whatever workflows that push
//! starts. The daemon watches those runs for every push it made: a green
//! pipeline is recorded, a red one is a finding on the project's `pipeline`
//! thread and one fix job planned for it, so what Hive landed does not
//! stay broken on the forge unnoticed. The default branch itself is watched
//! too, whoever pushed it: every ten minutes the daemon reads the branch's
//! head on the forge and judges its runs the same way, so a bot's commit or
//! a merge made on GitHub that turned the pipeline red is Hive's to fix as
//! well. The fix is planned only once the project checkout holds that
//! commit, so the worker starts from the code that is red. GitHub is read
//! with the GitHub App's installation token for the repository when an App
//! is configured (ADR-070 step 3, `gitapp`); without one, or when the App
//! cannot read it, with the gh account that can see the repository: the
//! remembered or active login first, and when it cannot, each other
//! account `gh auth status` lists, the first that answers remembered for
//! the project (`gh_auth`).

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::gh_auth::Logins;
use crate::util::{Row, event_at, must_rows, plus_hours, plus_minutes, trunc};

/// `off`: pushes are not watched. `watch`: a red pipeline is a finding for
/// the owner. `fix`: the finding and one fix job per project.
pub(crate) const WATCH: &str = "pipeline.watch";
/// Hours a push's runs may stay queued or running before the daemon stops
/// waiting on them.
pub(crate) const WAIT_HOURS: &str = "pipeline.wait_hours";
/// Source of the fix job: one open job per project, `add_job` coalesces on it.
pub(crate) const SOURCE_PREFIX: &str = "pipeline:";
pub(crate) const BOARD_AREA: &str = "runner";
pub(crate) const BOARD_THREAD: &str = "pipeline";
/// Pushes older than this are not watched; a verdict is a `pipeline.*`
/// event, so the window only bounds how far back the daemon looks for
/// pushes it has not judged.
const WINDOW_HOURS: f64 = 48.0;
/// Minutes a push may have no workflow run at all before the daemon
/// records that nothing ran (GitHub usually lists a run within a minute).
const NO_RUNS_GRACE_MINUTES: f64 = 10.0;
/// Minutes between two reads of one push's runs.
const POLL_MINUTES: f64 = 3.0;
/// A failed run is reported while other runs still run once the push is
/// this old: a deploy may take an hour, the broken check is known now.
const FAILED_EARLY_MINUTES: f64 = 30.0;
/// Fix jobs one project may get in `FIX_LIMIT_HOURS`; past that the owner is
/// told and no more are planned until the window moves.
const FIX_LIMIT: i64 = 3;
const FIX_LIMIT_HOURS: f64 = 24.0;
/// Priority of the fix job: above the planner's improve jobs and the red
/// base fix, below the owner's tasks and security updates.
const FIX_PRIORITY: i32 = 60;
const FIX_TIER: i32 = 1;
/// Failed runs whose jobs and log the brief carries in full.
const DETAIL_RUNS: usize = 3;
const LOG_LINES: usize = 120;
const LOG_BYTES: usize = 12_000;
const GH_TIMEOUT: Duration = Duration::from_secs(60);
/// Runs GitHub starts on its own clock against the newest commit; their
/// failing says nothing about the push.
const GITHUB_OWN: &[&str] = &["Dependabot Updates", "Dependency Graph"];
/// Minutes between two reads of the default branch's head on the forge.
const HEAD_POLL_MINUTES: f64 = 10.0;
/// Hours a red head may wait for the project checkout to hold its commit
/// before the finding is posted without a fix job: a fix built on a
/// checkout that does not have the red code would fix the wrong thing.
const HEAD_CATCH_UP_HOURS: f64 = 2.0;
/// Minutes a repository no logged-in gh account can see is left alone
/// before the accounts are tried again: polling three such repositories at
/// every interval cost one daemon tick 7.3 s of 404s and said nothing new.
const UNREADABLE_MINUTES: f64 = 60.0;

pub(crate) fn fix_source(slug: &str) -> String {
    format!("{SOURCE_PREFIX}{slug}")
}

/// One workflow run GitHub reports for a commit.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Run {
    pub id: i64,
    pub workflow: String,
    pub event: String,
    pub status: String,
    pub conclusion: String,
    pub url: String,
    pub branch: String,
}

impl Run {
    fn done(&self) -> bool {
        self.status == "completed"
    }
    fn failed(&self) -> bool {
        self.done()
            && matches!(
                self.conclusion.as_str(),
                "failure" | "timed_out" | "startup_failure" | "action_required" | "stale"
            )
    }
    fn passed(&self) -> bool {
        self.done() && matches!(self.conclusion.as_str(), "success" | "skipped" | "neutral")
    }
    fn cancelled(&self) -> bool {
        self.done() && self.conclusion == "cancelled"
    }
    /// Whether the push started this run: a schedule or a hand
    /// (`workflow_dispatch`) did not, nor did GitHub's own housekeeping.
    fn counts(&self) -> bool {
        match self.event.as_str() {
            "push" | "pull_request" | "pull_request_target" | "merge_group" => true,
            "dynamic" => !GITHUB_OWN.contains(&self.workflow.as_str()),
            _ => false,
        }
    }
    fn json(&self) -> Value {
        json!({
            "id": self.id,
            "workflow": self.workflow,
            "event": self.event,
            "status": self.status,
            "conclusion": self.conclusion,
            "url": self.url,
        })
    }
}

/// One push the daemon made and has no verdict on yet, or the default
/// branch's head on the forge when someone else moved it (`by` names them).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct Push {
    pub seq: i64,
    pub at: String,
    pub sha: String,
    pub from: String,
    pub remote: String,
    pub branch: String,
    pub commits: i64,
    /// The pull request's number when the push went to `hive/landed`; 0
    /// for a push of the branch itself.
    pub number: i64,
    pub url: String,
    /// Who pushed the commit when it was not hive: the forge login, or
    /// `someone` when the forge does not say. Empty for hive's own pushes.
    pub by: String,
}

impl Push {
    fn short(&self) -> String {
        crate::recipe::short(&self.sha, 12)
    }

    /// Whether this is the default branch's head someone else pushed.
    fn head(&self) -> bool {
        !self.by.is_empty()
    }

    /// `hive`, or who pushed the head.
    fn who(&self) -> &str {
        if self.by.is_empty() { "hive" } else { &self.by }
    }

    /// `1 commit` or `3 commits`.
    fn commits_text(&self) -> String {
        format!(
            "{} {}",
            self.commits,
            crate::util::plural(self.commits as i32, "commit")
        )
    }

    /// `(2 commits, URL)` for hive's push; `(URL)` for a head whose
    /// commit count the daemon does not know.
    fn parenthetical(&self) -> String {
        let mut parts = Vec::new();
        if self.commits > 0 {
            parts.push(self.commits_text());
        }
        if !self.url.is_empty() {
            parts.push(self.url.clone());
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!(" ({})", parts.join(", "))
        }
    }

    /// `origin/main` or `pull request #7`.
    fn target(&self) -> String {
        if self.number > 0 {
            format!("pull request #{}", self.number)
        } else {
            format!("{}/{}", self.remote, self.branch)
        }
    }
}

/// The default branch's head as the forge reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Head {
    pub sha: String,
    /// The commit's committer date, which is when it reached the branch
    /// for a merge made on the forge and close enough for a push.
    pub at: String,
    pub by: String,
    pub url: String,
    pub subject: String,
}

/// What the pass asks GitHub, so tests answer in its place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ask<'a> {
    /// The workflow runs of a commit: `gh run list --commit SHA`.
    Runs { repo: &'a str, sha: &'a str },
    /// A run's jobs and their steps: `gh run view ID --json jobs`.
    Jobs { repo: &'a str, id: i64 },
    /// The failed steps' log: `gh run view ID --log-failed`.
    Log { repo: &'a str, id: i64 },
    /// The branch's head commit: `gh api repos/REPO/branches/BRANCH`.
    Head { repo: &'a str, branch: &'a str },
    /// Run the failed jobs again: `gh run rerun ID --failed`. Asked once per
    /// commit, when GitHub no longer has the failed log.
    Rerun { repo: &'a str, id: i64 },
}

/// What the pass asks GitHub, and as which gh account (`None`: the active
/// login), so tests answer in its place.
pub(crate) type Gh<'a> = dyn Fn(&Ask, Option<&str>) -> Result<String, String> + 'a;

impl Ask<'_> {
    /// The repository asked about, `OWNER/REPO`.
    fn repo(&self) -> &str {
        match self {
            Ask::Runs { repo, .. }
            | Ask::Jobs { repo, .. }
            | Ask::Log { repo, .. }
            | Ask::Head { repo, .. }
            | Ask::Rerun { repo, .. } => repo,
        }
    }
}

/// The daemon's `Gh` for one pass: the GitHub App's installation token for
/// the repository first when an App is configured (ADR-070 step 3), so CI
/// reads stop depending on this machine's gh logins; a read the App cannot
/// make (`account_failed`: not installed on the repository, no Actions
/// permission) or a token that cannot be minted falls back to `gh_ask` as
/// `user`, the remembered-login rotation as before. Once the App has failed
/// in a pass it is not asked again in that pass, and a token that could not
/// be minted is written down once as `gitapp.fallback` (the reason, never a
/// secret).
fn gh_app_first<'a>(
    s: &'a hive_store::Store,
) -> impl Fn(&Ask, Option<&str>) -> Result<String, String> + 'a {
    let app_failed = std::cell::Cell::new(false);
    move |ask: &Ask, user: Option<&str>| {
        if !app_failed.get() {
            match crate::gitapp::gh_env(s, ask.repo()) {
                None => {}
                Some(Ok(env)) => match gh_run(ask, env, " as the GitHub App") {
                    Err(e) if account_failed(&e) => app_failed.set(true),
                    answer => return answer,
                },
                Some(Err(why)) => {
                    app_failed.set(true);
                    let _ = s.event(
                        "daemon",
                        "gitapp.fallback",
                        ask.repo(),
                        json!({
                            "repo": ask.repo(),
                            "why": hive_cred::redact(&crate::executor::redact_tokens(&why), &[]),
                        }),
                    );
                }
            }
        }
        gh_ask(ask, user)
    }
}

/// Runs `gh` with the daemon's own environment - `gh` keeps its login in
/// its own config, nothing of it reaches git - as the active login, or as
/// `user` with that account's token in `GH_TOKEN`.
fn gh_ask(ask: &Ask, user: Option<&str>) -> Result<String, String> {
    let env = match user {
        Some(user) => crate::gh_auth::env_as(user)
            .map_err(|e| format!("{} as {user}: {e}", gh_what(&gh_argv(ask))))?,
        None => crate::proc::environ(),
    };
    gh_run(ask, env, "")
}

/// `gh SUBCOMMAND VERB`, the words an error starts with.
fn gh_what(argv: &[String]) -> String {
    format!("gh {} {}", argv[1], argv[2])
}

/// The `gh` command line that answers `ask`.
fn gh_argv(ask: &Ask) -> Vec<String> {
    match ask {
        Ask::Runs { repo, sha } => vec![
            "gh".into(),
            "run".into(),
            "list".into(),
            "--repo".into(),
            repo.to_string(),
            "--commit".into(),
            sha.to_string(),
            "--limit".into(),
            "50".into(),
            "--json".into(),
            "databaseId,workflowName,event,status,conclusion,url,headBranch".into(),
        ],
        Ask::Jobs { repo, id } => vec![
            "gh".into(),
            "run".into(),
            "view".into(),
            id.to_string(),
            "--repo".into(),
            repo.to_string(),
            "--json".into(),
            "jobs".into(),
        ],
        Ask::Log { repo, id } => vec![
            "gh".into(),
            "run".into(),
            "view".into(),
            id.to_string(),
            "--repo".into(),
            repo.to_string(),
            "--log-failed".into(),
        ],
        Ask::Head { repo, branch } => vec![
            "gh".into(),
            "api".into(),
            format!("repos/{repo}/branches/{branch}"),
        ],
        Ask::Rerun { repo, id } => vec![
            "gh".into(),
            "run".into(),
            "rerun".into(),
            id.to_string(),
            "--repo".into(),
            repo.to_string(),
            "--failed".into(),
        ],
    }
}

/// Runs the `gh` that answers `ask` with `env`; `who` follows the command
/// in an error (` as the GitHub App`, or nothing).
fn gh_run(ask: &Ask, env: Vec<String>, who: &str) -> Result<String, String> {
    let argv = gh_argv(ask);
    let what = format!("{}{who}", gh_what(&argv));
    match crate::proc::run_group(
        &hive_bus::CancellationToken::new(),
        &argv,
        "",
        &env,
        &[],
        GH_TIMEOUT,
    ) {
        Err(e) => Err(format!("{what}: {e}")),
        Ok(out) if out.timeout => Err(format!(
            "{what} did not answer within {}s",
            GH_TIMEOUT.as_secs()
        )),
        Ok(out) if out.exit_code != 0 => Err(format!(
            "{what} exited {}: {}",
            out.exit_code,
            trunc(out.stderr.trim(), 300)
        )),
        Ok(out) => Ok(out.stdout),
    }
}

/// The runs `gh run list --json` printed.
pub(crate) fn parse_runs(body: &str) -> Result<Vec<Run>, String> {
    let v: Value = serde_json::from_str(body.trim())
        .map_err(|e| format!("gh run list answered oddly: {e}"))?;
    let items = v
        .as_array()
        .ok_or_else(|| "gh run list answered oddly: not a list".to_string())?;
    Ok(items
        .iter()
        .map(|r| Run {
            id: r.get("databaseId").and_then(Value::as_i64).unwrap_or(0),
            workflow: text(r, "workflowName"),
            event: text(r, "event"),
            status: text(r, "status"),
            conclusion: text(r, "conclusion"),
            url: text(r, "url"),
            branch: text(r, "headBranch"),
        })
        .collect())
}

fn text(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The branch's head as `gh api repos/R/branches/B` reports it.
pub(crate) fn parse_head(body: &str) -> Result<Head, String> {
    let v: Value = serde_json::from_str(body.trim())
        .map_err(|e| format!("gh api branches answered oddly: {e}"))?;
    let commit = &v["commit"];
    let sha = text(commit, "sha");
    if sha.is_empty() {
        return Err("gh api branches answered oddly: no commit sha".to_string());
    }
    let inner = &commit["commit"];
    let mut by = text(&commit["author"], "login");
    if by.is_empty() {
        by = text(&commit["committer"], "login");
    }
    if by.is_empty() {
        by = text(&inner["author"], "name");
    }
    if by.is_empty() {
        by = "someone".to_string();
    }
    let mut at = text(&inner["committer"], "date");
    if at.is_empty() {
        at = text(&inner["author"], "date");
    }
    Ok(Head {
        sha,
        at,
        by,
        url: text(commit, "html_url"),
        subject: text(inner, "message")
            .lines()
            .next()
            .unwrap_or_default()
            .to_string(),
    })
}

/// `job: step, step` for every failed job of a `gh run view --json jobs` answer.
pub(crate) fn failed_steps(body: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<Value>(body.trim()) else {
        return Vec::new();
    };
    let Some(jobs) = v.get("jobs").and_then(Value::as_array) else {
        return Vec::new();
    };
    jobs.iter()
        .filter(|j| {
            matches!(
                text(j, "conclusion").as_str(),
                "failure" | "timed_out" | "startup_failure" | "action_required"
            )
        })
        .map(|j| {
            let steps: Vec<String> = j
                .get("steps")
                .and_then(Value::as_array)
                .map(|steps| {
                    steps
                        .iter()
                        .filter(|st| {
                            matches!(text(st, "conclusion").as_str(), "failure" | "timed_out")
                        })
                        .map(|st| text(st, "name"))
                        .filter(|n| !n.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            if steps.is_empty() {
                text(j, "name")
            } else {
                format!("{}: {}", text(j, "name"), steps.join(", "))
            }
        })
        .filter(|l| !l.is_empty())
        .collect()
}

/// The last lines of a failed-steps log, bounded and with secrets redacted.
pub(crate) fn log_tail(log: &str) -> String {
    let lines: Vec<&str> = log.lines().collect();
    let start = lines.len().saturating_sub(LOG_LINES);
    let tail = lines[start..].join("\n");
    let tail = crate::executor::tail(tail.trim(), LOG_BYTES);
    crate::compute::redact_secrets(&tail)
}

/// The pushes of `slug` the daemon made within the window and has not
/// judged, oldest first.
pub(crate) fn pending(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    now: &str,
) -> Vec<Push> {
    let since = plus_hours(now, -WINDOW_HOURS);
    let judged: BTreeSet<String> = must_rows(
        q,
        "SELECT json_extract(detail, '$.sha') AS sha FROM events WHERE subject = ? AND action LIKE 'pipeline.%' AND action != 'pipeline.rerun' AND at >= ?",
        &[&slug, &since],
    )
    .iter()
    .map(|r| r.str("sha"))
    .collect();
    let mut by_sha: BTreeMap<String, Push> = BTreeMap::new();
    for e in must_rows(
        q,
        "SELECT seq, at, action, detail FROM events WHERE subject = ? AND action IN ('land.pushed', 'land.pr') AND at >= ? ORDER BY seq",
        &[&slug, &since],
    ) {
        let d: Value = serde_json::from_str(&e.str("detail")).unwrap_or(Value::Null);
        let sha = text(&d, "to");
        if sha.is_empty() || judged.contains(&sha) {
            continue;
        }
        by_sha.insert(
            sha.clone(),
            Push {
                seq: e.str("seq").parse().unwrap_or(0),
                at: e.str("at"),
                sha,
                from: text(&d, "from"),
                remote: text(&d, "remote"),
                branch: if e.str("action") == "land.pr" {
                    text(&d, "base")
                } else {
                    text(&d, "branch")
                },
                commits: d.get("commits").and_then(Value::as_i64).unwrap_or(0),
                number: if e.str("action") == "land.pr" {
                    d.get("number").and_then(Value::as_i64).unwrap_or(0)
                } else {
                    0
                },
                url: text(&d, "url"),
                by: String::new(),
            },
        );
    }
    let mut pushes: Vec<Push> = by_sha.into_values().collect();
    pushes.sort_by_key(|p| p.seq);
    pushes
}

/// What the watcher remembers about one project, in `meta`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct Watch {
    /// sha -> when its runs were last read.
    #[serde(default)]
    pub polled: BTreeMap<String, String>,
    /// The error the owner was told about when the runs could not be read.
    #[serde(default)]
    pub unreadable_told: String,
    /// When the default branch's head was last read on the forge, and when
    /// it is read next.
    #[serde(default)]
    pub head_read: String,
    #[serde(default)]
    pub head_next: String,
    /// The head last seen there.
    #[serde(default)]
    pub head_sha: String,
    /// Who pushed it, when not hive.
    #[serde(default)]
    pub head_by: String,
    /// When the daemon first found the head red while the project checkout
    /// did not hold its commit yet; cleared when the head moves or is judged.
    #[serde(default)]
    pub head_waiting_since: String,
    /// sha -> when its failed jobs were rerun because GitHub no longer had
    /// their log; a commit is rerun once.
    #[serde(default)]
    pub reruns: BTreeMap<String, String>,
    /// The gh account that reads the repository when the active login
    /// cannot see it: found by trying each logged-in account once, read as
    /// first from then on, forgotten when it fails. Empty: the active login.
    #[serde(default)]
    pub gh_user: String,
    /// Until when the repository is left alone because no logged-in account
    /// could read it; empty once one can.
    #[serde(default)]
    pub unreadable_until: String,
}

impl Watch {
    /// The account the repository is read as, for the `Gh` closure.
    fn user(&self) -> Option<&str> {
        (!self.gh_user.is_empty()).then_some(self.gh_user.as_str())
    }

    /// Whether no account could read the repository within the hour.
    fn left_alone(&self, now: &str) -> bool {
        !self.unreadable_until.is_empty() && self.unreadable_until.as_str() > now
    }
}

/// Whether a `pipeline.*` verdict was recorded for the commit, ever: a head
/// may be older than the window pushes are looked for in. A rerun is not a
/// verdict: the commit is judged when the rerun finishes.
pub(crate) fn judged_sha(q: &(impl crate::util::Querier + ?Sized), slug: &str, sha: &str) -> bool {
    !must_rows(
        q,
        "SELECT seq FROM events WHERE subject = ? AND action LIKE 'pipeline.%' AND action != 'pipeline.rerun' AND json_extract(detail, '$.sha') = ? LIMIT 1",
        &[&slug, &sha],
    )
    .is_empty()
}

/// Whether the error reading a run's log says GitHub no longer has it
/// (retention passed or deleted), so only a rerun produces one.
fn log_gone(err: &str) -> bool {
    let low = err.to_ascii_lowercase();
    low.contains("http 410") || low.contains("http 404")
}

/// Whether gh's error says the account it ran as is no use for the
/// repository: GitHub answers 404 for a repository the account has no
/// access to (so the name does not say that it exists) and cannot resolve
/// its name, 403 for one it may see but not read the Actions of, 401 for a
/// token that is no longer good, and an account whose token `gh auth
/// token` cannot hand out (logged out, keyring locked, no answer in time;
/// `gh_auth::env_as` names the account and the command, never the token)
/// cannot run at all, so the next account is tried rather than this one
/// again every poll. A branch that is not there is a 404 too, but one that
/// says so, and another account would not find it either. The pull-request
/// calls (`landing_pr::gh`) rotate on the same answers.
///
/// An account an enterprise manages is one more case: it reads a public
/// repository outside its enterprise and may not write to it, so the read
/// succeeds and opening the pull request is refused with `Unauthorized: As
/// an Enterprise Managed User, you cannot access this content`. That says
/// the account is the wrong one, and another login may open it.
pub(crate) fn account_failed(err: &str) -> bool {
    let low = err.to_ascii_lowercase();
    if low.contains("branch not found") {
        return false;
    }
    low.contains("http 401")
        || low.contains("http 403")
        || low.contains("http 404")
        || low.contains("could not resolve to a repository")
        || low.contains("gh auth token for")
        || low.contains("as an enterprise managed user")
}

/// Reads the repository as the gh account that can see it. The watch's
/// remembered account goes first, the active login when it remembers none;
/// when that account cannot see the repository (`account_failed`) every
/// other logged-in account is tried once and the first that answers is
/// remembered, so the next polls go straight to it and the daemon says so
/// once, not per poll. When none can, the repository is left alone for
/// `UNREADABLE_MINUTES`. Only the reads every poll makes (the runs of a
/// push, the branch head) rotate; a run's jobs and log follow a read that
/// answered, and a 404 on a log means the log is gone, not the account.
struct Reader<'a> {
    gh: &'a Gh<'a>,
    logins: &'a dyn Fn() -> Logins,
    slug: &'a str,
    repo: &'a str,
    now: &'a str,
    /// The active login's name once `logins` has been asked, or what it is
    /// called when the status did not name it.
    active: String,
    /// The accounts the last read failed as, names only, in the order
    /// tried; empty after a read that answered.
    tried: Vec<String>,
    /// The last read failed as every account there was to try.
    exhausted: bool,
}

/// What the active login is called when `gh auth status` did not say.
const ACTIVE_UNNAMED: &str = "the active gh login";

impl Reader<'_> {
    fn read(
        &mut self,
        watch: &mut Watch,
        ask: &Ask,
        lines: &mut Vec<String>,
    ) -> Result<String, String> {
        self.exhausted = false;
        let first = watch.gh_user.clone();
        let err = match (self.gh)(ask, watch.user()) {
            Ok(body) => {
                self.tried.clear();
                watch.unreadable_until.clear();
                return Ok(body);
            }
            Err(e) if !account_failed(&e) => return Err(e),
            Err(e) => e,
        };
        let logins = (self.logins)();
        self.active = if logins.active.is_empty() {
            ACTIVE_UNNAMED.to_string()
        } else {
            logins.active.clone()
        };
        self.tried = vec![if first.is_empty() {
            self.active.clone()
        } else {
            first.clone()
        }];
        let since = format!("since {} cannot see it", self.name(&self.tried[0]));
        // The remembered account is no use any more: the active login is
        // tried as itself, then the others by name.
        let mut candidates: Vec<Option<String>> = Vec::new();
        if !first.is_empty() {
            watch.gh_user.clear();
            if first != logins.active {
                candidates.push(None);
            }
        }
        candidates.extend(logins.others(&first).into_iter().map(Some));
        for candidate in candidates {
            match (self.gh)(ask, candidate.as_deref()) {
                Ok(body) => {
                    let reading_as = match &candidate {
                        Some(user) => {
                            watch.gh_user = user.clone();
                            user.clone()
                        }
                        None => format!("{} again", self.name(&self.active)),
                    };
                    lines.push(format!(
                        "pipeline {}: reading {} as {reading_as}, {since}",
                        self.slug, self.repo
                    ));
                    self.tried.clear();
                    watch.unreadable_until.clear();
                    return Ok(body);
                }
                Err(e) if account_failed(&e) => {
                    self.tried
                        .push(candidate.unwrap_or_else(|| self.active.clone()));
                }
                Err(e) => return Err(e),
            }
        }
        self.exhausted = true;
        watch.unreadable_until = plus_minutes(self.now, UNREADABLE_MINUTES);
        Err(err)
    }

    /// How a tried account is called in prose: the active login is said to
    /// be that, so the owner knows which one `gh` would use by itself.
    fn name(&self, user: &str) -> String {
        if user == self.active && user != ACTIVE_UNNAMED {
            format!("the active gh login {user}")
        } else {
            user.to_string()
        }
    }

    /// `a`, `a and b`, `a, b and c`: the accounts the last read failed as.
    fn tried_text(&self) -> String {
        let named: Vec<String> = self.tried.iter().map(|u| self.name(u)).collect();
        match named.len() {
            0 => String::new(),
            1 => named[0].clone(),
            n => format!("{} and {}", named[..n - 1].join(", "), named[n - 1]),
        }
    }
}

/// Whether the project checkout's default branch holds the commit, so a
/// clone of it starts from the code that is red.
fn checkout_holds(root: &str, def: &str, sha: &str) -> bool {
    if root.is_empty() || sha.is_empty() || !std::path::Path::new(root).join(".git").exists() {
        return false;
    }
    crate::util::git_timeout(
        root,
        Duration::from_secs(20),
        &[
            "merge-base",
            "--is-ancestor",
            sha,
            &format!("refs/heads/{def}"),
        ],
    )
    .ok()
}

fn meta_key(slug: &str) -> String {
    format!("pipeline:{slug}")
}

pub(crate) fn load_watch(s: &hive_store::Store, slug: &str) -> Watch {
    match s.meta(&meta_key(slug)) {
        Ok((v, true)) if !v.is_empty() => serde_json::from_str(&v).unwrap_or_default(),
        _ => Watch::default(),
    }
}

fn save_watch(conn: &rusqlite::Connection, slug: &str, w: &Watch) {
    let _ = conn.execute(
        "INSERT INTO meta (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![meta_key(slug), serde_json::to_string(w).unwrap_or_default()],
    );
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Runs are queued or running, or none is listed yet: read again later.
    Wait,
    Passed,
    Failed,
    /// Nothing the push started ran (or everything was cancelled).
    None {
        cancelled: usize,
    },
    /// Still running past `pipeline.wait_hours`.
    GaveUp,
}

fn age_minutes(from: &str, now: &str) -> f64 {
    let (Ok(a), Ok(b)) = (
        chrono::DateTime::parse_from_rfc3339(from),
        chrono::DateTime::parse_from_rfc3339(now),
    ) else {
        return f64::MAX;
    };
    (b - a).num_seconds() as f64 / 60.0
}

pub(crate) fn judge(runs: &[Run], pushed_at: &str, now: &str, wait_hours: f64) -> Verdict {
    let counted: Vec<&Run> = runs.iter().filter(|r| r.counts()).collect();
    let age = age_minutes(pushed_at, now);
    let running = counted.iter().any(|r| !r.done());
    let failed = counted.iter().any(|r| r.failed());
    if running && failed {
        return if age >= FAILED_EARLY_MINUTES {
            Verdict::Failed
        } else {
            Verdict::Wait
        };
    }
    if running {
        return if age >= wait_hours * 60.0 {
            Verdict::GaveUp
        } else {
            Verdict::Wait
        };
    }
    if failed {
        return Verdict::Failed;
    }
    if counted.iter().any(|r| r.passed()) {
        return Verdict::Passed;
    }
    if counted.is_empty() && age < NO_RUNS_GRACE_MINUTES {
        return Verdict::Wait;
    }
    Verdict::None {
        cancelled: counted.iter().filter(|r| r.cancelled()).count(),
    }
}

/// ` (pushed by dependabot[bot])` for a head someone else pushed, nothing
/// for hive's own push.
fn pushed_by(push: &Push) -> String {
    if push.head() {
        format!(" (pushed by {})", push.by)
    } else {
        String::new()
    }
}

fn names(runs: &[&Run]) -> String {
    let mut seen = BTreeSet::new();
    runs.iter()
        .filter(|r| seen.insert(r.workflow.clone()))
        .map(|r| r.workflow.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The daemon's pass for one project: reads the runs of every push it made
/// and has no verdict on, records the verdict, and on a red pipeline posts
/// the finding and plans the fix job. Returns the daemon's log lines, none
/// when there was nothing to do.
pub(crate) fn tend_pipeline(
    s: &hive_store::Store,
    slug: &str,
    now: &str,
    actor: &str,
) -> Vec<String> {
    tend_pipeline_as(
        s,
        slug,
        now,
        actor,
        &gh_app_first(s),
        &crate::gh_auth::logins,
    )
}

/// `tend_pipeline` with gh answered by `gh` and the active login the only
/// account there is.
#[cfg(test)]
pub(crate) fn tend_pipeline_with(
    s: &hive_store::Store,
    slug: &str,
    now: &str,
    actor: &str,
    gh: &Gh<'_>,
) -> Vec<String> {
    tend_pipeline_as(s, slug, now, actor, gh, &Logins::default)
}

/// `tend_pipeline` with gh answered by `gh` and the logged-in accounts by
/// `logins`, which is asked only when an account cannot see the repository.
pub(crate) fn tend_pipeline_as(
    s: &hive_store::Store,
    slug: &str,
    now: &str,
    actor: &str,
    gh: &Gh<'_>,
    logins: &dyn Fn() -> Logins,
) -> Vec<String> {
    let mode = s.setting(WATCH, "fix");
    if mode == "off" {
        return Vec::new();
    }
    let Ok(conn) = s.conn() else {
        return Vec::new();
    };
    let project = crate::board::project_by_slug(&conn, slug.to_string());
    if project.str("status") != "active" {
        return Vec::new();
    }
    let Some(repo) = crate::security::github_repo(&project.str("remote")) else {
        return Vec::new();
    };
    let mut watch = load_watch(s, slug);
    if watch.left_alone(now) {
        return Vec::new();
    }
    let pushes = pending(&conn, slug, now);
    let wait_hours = crate::util::setting_int(s, WAIT_HOURS).max(1) as f64;
    let newest = pushes.last().map(|p| p.sha.clone()).unwrap_or_default();
    let mut lines = Vec::new();
    let mut judged = Vec::new();
    let mut reader = Reader {
        gh,
        logins,
        slug,
        repo: &repo,
        now,
        active: String::new(),
        tried: Vec::new(),
        exhausted: false,
    };
    for push in &pushes {
        if watch.left_alone(now) {
            break;
        }
        if watch
            .polled
            .get(&push.sha)
            .is_some_and(|at| plus_minutes(at, POLL_MINUTES).as_str() > now)
        {
            continue;
        }
        watch.polled.insert(push.sha.clone(), now.to_string());
        let runs = match reader.read(
            &mut watch,
            &Ask::Runs {
                repo: &repo,
                sha: &push.sha,
            },
            &mut lines,
        ) {
            Ok(body) => match parse_runs(&body) {
                Ok(runs) => runs,
                Err(e) => {
                    lines.push(format!("pipeline {slug}: {e}"));
                    continue;
                }
            },
            Err(e) => {
                if !reader.exhausted {
                    lines.push(format!(
                        "pipeline {slug}: the runs of {} could not be read: {}",
                        push.short(),
                        trunc(e.trim(), 300)
                    ));
                    continue;
                }
                let tried = reader.tried_text();
                lines.push(format!(
                    "pipeline {slug}: the runs of {} could not be read as {tried}: {}; {repo} is left alone until {}",
                    push.short(),
                    trunc(e.trim(), 300),
                    watch.unreadable_until
                ));
                if watch.unreadable_told != repo {
                    let summary = if reader.tried.len() == 1 {
                        format!(
                            "hive cannot read the workflow runs of {repo}: {tried} needs read access to the repository's Actions"
                        )
                    } else {
                        format!(
                            "hive cannot read the workflow runs of {repo}: none of the gh logins on this machine ({tried}) can see it"
                        )
                    };
                    let body = format!(
                        "`gh run list --repo {repo} --commit {}` answered: {}\n\nThe daemon watches the pipeline of every push it makes ({WATCH} is `{mode}`) and reads the repository's Actions as whichever gh login on this machine can see it; it tried {tried} and none could. Give one of them read access to the repository on the forge, or `gh auth login` as the user the daemon runs as with an account that has it. Until then the daemon leaves {repo} alone and tries the logins again every hour. `hive settings set {WATCH} off` stops watching.",
                        push.short(),
                        trunc(e.trim(), 300)
                    );
                    if let Ok(post) = tell_owner(s, slug, now, actor, summary, body) {
                        watch.unreadable_told = repo.clone();
                        event_at(
                            &conn,
                            now,
                            actor,
                            "pipeline.unreadable",
                            slug,
                            json!({"project": slug, "repo": repo, "post": post, "error": trunc(e.trim(), 300), "tried": reader.tried, "until": watch.unreadable_until}),
                        );
                        lines.push(format!("pipeline {slug}: told the owner ({post})"));
                    }
                }
                continue;
            }
        };
        watch.unreadable_told.clear();
        let verdict = judge(&runs, &push.at, now, wait_hours);
        let mut pass = Pass {
            s,
            conn: &conn,
            slug,
            repo: &repo,
            project: &project,
            now,
            actor,
            mode: &mode,
            fix: mode == "fix",
            superseded: push.sha != newest,
            held: "",
            gh,
            watch: &mut watch,
            lines: &mut lines,
        };
        match verdict {
            Verdict::Wait => continue,
            Verdict::Passed => pass.passed(push, &runs),
            Verdict::Failed => {
                if !pass.failed(push, &runs) {
                    // Rerun for a fresh log: no verdict yet, polled on.
                    continue;
                }
            }
            Verdict::None { cancelled } => pass.none(push, &runs, cancelled),
            Verdict::GaveUp => pass.gave_up(push, &runs, wait_hours),
        }
        watch.reruns.remove(&push.sha);
        judged.push(push.sha.clone());
    }
    watch
        .polled
        .retain(|sha, _| pushes.iter().any(|p| &p.sha == sha) && !judged.contains(sha));
    tend_head(
        &HeadPass {
            s,
            conn: &conn,
            slug,
            repo: &repo,
            project: &project,
            now,
            actor,
            mode: &mode,
            wait_hours,
            pushes: &pushes,
            gh,
        },
        &mut reader,
        &mut watch,
        &mut lines,
    );
    watch
        .reruns
        .retain(|sha, _| sha == &watch.head_sha || pushes.iter().any(|p| &p.sha == sha));
    save_watch(&conn, slug, &watch);
    lines
}

/// What one read of the default branch's head needs.
struct HeadPass<'a> {
    s: &'a hive_store::Store,
    conn: &'a rusqlite::Connection,
    slug: &'a str,
    repo: &'a str,
    project: &'a Row,
    now: &'a str,
    actor: &'a str,
    mode: &'a str,
    wait_hours: f64,
    pushes: &'a [Push],
    gh: &'a Gh<'a>,
}

/// The remote and branch the project's default branch tracks, `origin` and
/// the branch itself when it tracks nothing.
fn upstream_of(root: &str, def: &str) -> (String, String) {
    let up = crate::util::git_timeout(
        root,
        Duration::from_secs(10),
        &[
            "for-each-ref",
            "--format=%(upstream:remotename) %(upstream:remoteref)",
            &format!("refs/heads/{def}"),
        ],
    )
    .stdout
    .split_whitespace()
    .map(ToOwned::to_owned)
    .collect::<Vec<_>>();
    if up.len() == 2 {
        let branch = up[1].strip_prefix("refs/heads/").unwrap_or(&up[1]);
        (up[0].clone(), branch.to_string())
    } else {
        ("origin".to_string(), def.to_string())
    }
}

/// Reads the default branch's head on the forge every `HEAD_POLL_MINUTES`
/// and judges its runs when neither hive's own pushes nor an earlier read
/// judged that commit. A red head gets its finding and fix job once the
/// project checkout holds the commit; until then the daemon waits up to
/// `HEAD_CATCH_UP_HOURS`, then posts the finding without a job. The head
/// and its runs are read through `reader`, so a project with no push of
/// hive's pending still finds the account that sees its repository.
fn tend_head(
    h: &HeadPass<'_>,
    reader: &mut Reader<'_>,
    watch: &mut Watch,
    lines: &mut Vec<String>,
) {
    if watch.left_alone(h.now) {
        return;
    }
    if !watch.head_next.is_empty() && watch.head_next.as_str() > h.now {
        return;
    }
    let root = h.project.str("path");
    let def = {
        let d = h.project.str("default_branch");
        if d.is_empty() { "main".to_string() } else { d }
    };
    let (remote, branch) = upstream_of(&root, &def);
    watch.head_read = h.now.to_string();
    watch.head_next = plus_minutes(h.now, HEAD_POLL_MINUTES);
    let head = match reader
        .read(
            watch,
            &Ask::Head {
                repo: h.repo,
                branch: &branch,
            },
            lines,
        )
        .and_then(|b| parse_head(&b))
    {
        Ok(head) => head,
        Err(e) => {
            lines.push(if reader.exhausted {
                format!(
                    "pipeline {}: the head of {remote}/{branch} could not be read as {}: {}; {} is left alone until {}",
                    h.slug,
                    reader.tried_text(),
                    trunc(e.trim(), 300),
                    h.repo,
                    watch.unreadable_until
                )
            } else {
                format!(
                    "pipeline {}: the head of {remote}/{branch} could not be read: {}",
                    h.slug,
                    trunc(e.trim(), 300)
                )
            });
            return;
        }
    };
    if head.sha != watch.head_sha {
        watch.head_sha = head.sha.clone();
        watch.head_by = head.by.clone();
        watch.head_waiting_since.clear();
    }
    // Hive's own push: the push path judges it, with what it landed.
    if h.pushes.iter().any(|p| p.sha == head.sha) || judged_sha(h.conn, h.slug, &head.sha) {
        return;
    }
    let runs = match reader
        .read(
            watch,
            &Ask::Runs {
                repo: h.repo,
                sha: &head.sha,
            },
            lines,
        )
        .and_then(|b| parse_runs(&b))
    {
        Ok(runs) => runs,
        Err(e) => {
            lines.push(format!(
                "pipeline {}: the runs of {} could not be read: {}",
                h.slug,
                crate::recipe::short(&head.sha, 12),
                trunc(e.trim(), 300)
            ));
            return;
        }
    };
    let pushed_at = if head.at.is_empty() {
        h.now.to_string()
    } else {
        head.at.clone()
    };
    let verdict = judge(&runs, &pushed_at, h.now, h.wait_hours);
    if verdict == Verdict::Wait {
        // Runs still going: look again as often as for a push of hive's.
        watch.head_next = plus_minutes(h.now, POLL_MINUTES);
        return;
    }
    let push = Push {
        at: pushed_at,
        sha: head.sha.clone(),
        remote,
        branch,
        url: head.url.clone(),
        by: head.by.clone(),
        ..Default::default()
    };
    let mut held = String::new();
    if verdict == Verdict::Failed && !checkout_holds(&root, &def, &head.sha) {
        if watch.head_waiting_since.is_empty() {
            watch.head_waiting_since = h.now.to_string();
        }
        let waited = age_minutes(&watch.head_waiting_since, h.now);
        if waited < HEAD_CATCH_UP_HOURS * 60.0 {
            lines.push(format!(
                "pipeline {}: {remote}/{branch} is red at {} on the forge (pushed by {}); waiting for the checkout to hold it before planning the fix",
                h.slug,
                push.short(),
                head.by,
                remote = push.remote,
                branch = push.branch,
            ));
            watch.head_next = plus_minutes(h.now, POLL_MINUTES);
            return;
        }
        held = sync_hold(h.s, h.slug).unwrap_or_else(|| {
            format!(
                "the checkout at {root} does not hold {} {} hours after it reached {}/{}",
                push.short(),
                HEAD_CATCH_UP_HOURS as i64,
                push.remote,
                push.branch
            )
        });
    }
    let mut pass = Pass {
        s: h.s,
        conn: h.conn,
        slug: h.slug,
        repo: h.repo,
        project: h.project,
        now: h.now,
        actor: h.actor,
        mode: h.mode,
        fix: h.mode == "fix" && held.is_empty(),
        superseded: false,
        held: &held,
        gh: h.gh,
        watch: &mut *watch,
        lines: &mut *lines,
    };
    match verdict {
        Verdict::Wait => {}
        Verdict::Passed => pass.passed(&push, &runs),
        Verdict::Failed => {
            if !pass.failed(&push, &runs) {
                // Rerun for a fresh log: judged again when it finishes.
                watch.head_next = plus_minutes(h.now, POLL_MINUTES);
                return;
            }
        }
        Verdict::None { cancelled } => pass.none(&push, &runs, cancelled),
        Verdict::GaveUp => pass.gave_up(&push, &runs, h.wait_hours),
    }
    watch.reruns.remove(&push.sha);
    watch.head_waiting_since.clear();
}

/// The note of the landing hold that keeps the project checkout behind its
/// upstream, when there is one (`landing_hold::sync_upstream` remembers it).
fn sync_hold(s: &hive_store::Store, slug: &str) -> Option<String> {
    let (v, found) = s.meta(&format!("push-sync:{slug}")).ok()?;
    if !found || v.is_empty() {
        return None;
    }
    let d: Value = serde_json::from_str(&v).ok()?;
    let note = text(&d, "note");
    (!note.is_empty()).then(|| format!("the checkout is held behind the forge: {note}"))
}

/// Posts a finding for the owner on the project's pipeline thread.
fn tell_owner(
    s: &hive_store::Store,
    slug: &str,
    now: &str,
    actor: &str,
    summary: String,
    body: String,
) -> Result<String, String> {
    crate::board::create_post(
        s,
        now.to_string(),
        crate::board::PostOpts {
            kind: "finding".into(),
            project: slug.to_string(),
            area: BOARD_AREA.into(),
            thread: BOARD_THREAD.into(),
            summary: trunc(&summary, 236),
            body,
            for_owner: true,
            agent: crate::board::actor_agent(s, actor.to_string()),
            date: now.to_string(),
            ..Default::default()
        },
    )
    .map(|p| p.str("id"))
    .map_err(|e| crate::run::err_message(&e))
}

/// One verdict on one push.
struct Pass<'a> {
    s: &'a hive_store::Store,
    conn: &'a rusqlite::Connection,
    slug: &'a str,
    repo: &'a str,
    project: &'a Row,
    now: &'a str,
    actor: &'a str,
    mode: &'a str,
    /// `pipeline.watch = fix`: a red pipeline gets a fix job.
    fix: bool,
    /// A later push of the project is pending too: the verdict is recorded,
    /// nothing is planned for it.
    superseded: bool,
    /// Why no fix job can be planned for a red head: the project checkout
    /// does not hold the commit (the landing hold that keeps it behind).
    /// Empty when a job may be planned.
    held: &'a str,
    gh: &'a Gh<'a>,
    /// The project's watch: which commits had their failed jobs rerun.
    watch: &'a mut Watch,
    lines: &'a mut Vec<String>,
}

impl Pass<'_> {
    fn detail(&self, push: &Push, runs: &[Run]) -> serde_json::Map<String, Value> {
        let mut d = serde_json::Map::new();
        d.insert("project".into(), json!(self.slug));
        d.insert("repo".into(), json!(self.repo));
        d.insert("sha".into(), json!(push.sha));
        d.insert("remote".into(), json!(push.remote));
        d.insert("branch".into(), json!(push.branch));
        if push.number > 0 {
            d.insert("number".into(), json!(push.number));
        }
        if push.head() {
            d.insert("head".into(), json!(true));
            d.insert("by".into(), json!(push.by));
        }
        d.insert(
            "runs".into(),
            Value::Array(runs.iter().filter(|r| r.counts()).map(Run::json).collect()),
        );
        if self.superseded {
            d.insert("superseded".into(), json!(true));
        }
        d
    }

    fn passed(&mut self, push: &Push, runs: &[Run]) {
        let counted: Vec<&Run> = runs.iter().filter(|r| r.counts()).collect();
        let mut d = self.detail(push, runs);
        let rerun = self
            .watch
            .reruns
            .get(&push.sha)
            .cloned()
            .unwrap_or_default();
        if !rerun.is_empty() {
            d.insert("rerun".into(), json!(rerun));
        }
        event_at(
            self.conn,
            self.now,
            self.actor,
            "pipeline.passed",
            self.slug,
            Value::Object(d),
        );
        self.lines.push(format!(
            "pipeline {}: green at {} on {}{} ({}){}",
            self.slug,
            push.short(),
            push.target(),
            pushed_by(push),
            names(&counted),
            if rerun.is_empty() {
                String::new()
            } else {
                format!(" - the failed jobs passed when rerun at {rerun}; nothing to fix")
            }
        ));
    }

    fn none(&mut self, push: &Push, runs: &[Run], cancelled: usize) {
        let mut d = self.detail(push, runs);
        d.insert("cancelled".into(), json!(cancelled));
        event_at(
            self.conn,
            self.now,
            self.actor,
            "pipeline.none",
            self.slug,
            Value::Object(d),
        );
        self.lines.push(if cancelled > 0 {
            format!(
                "pipeline {}: every run for {} on {} was cancelled ({cancelled})",
                self.slug,
                push.short(),
                push.target()
            )
        } else {
            format!(
                "pipeline {}: no workflow ran for {} on {}",
                self.slug,
                push.short(),
                push.target()
            )
        });
    }

    fn gave_up(&mut self, push: &Push, runs: &[Run], wait_hours: f64) {
        let running: Vec<&Run> = runs.iter().filter(|r| r.counts() && !r.done()).collect();
        let mut d = self.detail(push, runs);
        d.insert("wait_hours".into(), json!(wait_hours));
        let mut post = String::new();
        if !self.superseded {
            let summary = format!(
                "pipeline of {}: {} still running {} h after the push of {} to {}",
                self.slug,
                names(&running),
                wait_hours,
                push.short(),
                push.target()
            );
            let body = format!(
                "{} h after {} pushed {} to {}{}, these runs of {} have not finished:\n\n{}\n\nA run that long usually waits for a runner that is not there (a self-hosted runner that is off, a queue that is full) or hangs on a step. The daemon stops waiting on this push ({WAIT_HOURS} is {}); the runs stay on GitHub for you to look at.",
                wait_hours,
                push.who(),
                push.short(),
                push.target(),
                push.parenthetical(),
                self.repo,
                running
                    .iter()
                    .map(|r| format!("    {} ({}): {}", r.workflow, r.status, r.url))
                    .collect::<Vec<_>>()
                    .join("\n"),
                wait_hours
            );
            match tell_owner(self.s, self.slug, self.now, self.actor, summary, body) {
                Ok(id) => post = id,
                Err(why) => self.lines.push(format!(
                    "pipeline {}: the owner could not be told: {why}",
                    self.slug
                )),
            }
        }
        if !post.is_empty() {
            d.insert("post".into(), json!(post));
        }
        event_at(
            self.conn,
            self.now,
            self.actor,
            "pipeline.gave-up",
            self.slug,
            Value::Object(d),
        );
        self.lines.push(format!(
            "pipeline {}: still running {} h after {} on {}: {}; stopped waiting",
            self.slug,
            wait_hours,
            push.short(),
            push.target(),
            names(&running)
        ));
    }

    /// Records the red verdict, tells the owner and plans the fix job.
    /// Returns false when no verdict was recorded yet: GitHub no longer had
    /// the failed log, so the failed jobs were rerun for a fresh one and the
    /// commit is judged again when they finish.
    fn failed(&mut self, push: &Push, runs: &[Run]) -> bool {
        let failed: Vec<&Run> = runs.iter().filter(|r| r.counts() && r.failed()).collect();
        let passed: Vec<&Run> = runs.iter().filter(|r| r.counts() && r.passed()).collect();
        let running: Vec<&Run> = runs.iter().filter(|r| r.counts() && !r.done()).collect();
        let mut d = self.detail(push, runs);
        d.insert(
            "failed".into(),
            Value::Array(failed.iter().map(|r| json!(r.workflow)).collect()),
        );
        if self.superseded {
            event_at(
                self.conn,
                self.now,
                self.actor,
                "pipeline.failed",
                self.slug,
                Value::Object(d),
            );
            self.lines.push(format!(
                "pipeline {}: red at {} on {} ({} failed); a later push is pending, nothing planned for this one",
                self.slug,
                push.short(),
                push.target(),
                names(&failed)
            ));
            return true;
        }
        let (details, unread) = self.failure_details(&failed);
        let rerun_at = self
            .watch
            .reruns
            .get(&push.sha)
            .cloned()
            .unwrap_or_default();
        let mut log_notes = String::new();
        if !unread.is_empty()
            && unread.len() == details.len()
            && unread.iter().all(|(_, e)| log_gone(e))
            && rerun_at.is_empty()
            && self.fix
        {
            let mut reran = Vec::new();
            let mut refused = String::new();
            for (run, _) in &unread {
                match (self.gh)(
                    &Ask::Rerun {
                        repo: self.repo,
                        id: run.id,
                    },
                    self.watch.user(),
                ) {
                    Ok(_) => reran.push(run.id),
                    Err(e) => refused = trunc(e.trim(), 300),
                }
            }
            if !reran.is_empty() {
                self.watch
                    .reruns
                    .insert(push.sha.clone(), self.now.to_string());
                d.insert("rerun".into(), json!(reran));
                d.insert("reason".into(), json!(trunc(&unread[0].1, 300)));
                event_at(
                    self.conn,
                    self.now,
                    self.actor,
                    "pipeline.rerun",
                    self.slug,
                    Value::Object(d),
                );
                self.lines.push(format!(
                    "pipeline {}: red at {} on {}{} ({} failed), but GitHub no longer has the failed log ({}); the failed jobs were rerun for a fresh one and are judged again when they finish",
                    self.slug,
                    push.short(),
                    push.target(),
                    pushed_by(push),
                    names(&failed),
                    trunc(&unread[0].1, 160)
                ));
                return false;
            }
            log_notes.push_str(&format!(
                "\nThe failed jobs could not be rerun for a fresh log: {refused}\n"
            ));
        }
        for (run, err) in &unread {
            log_notes.push_str(&format!(
                "\n{} - the failed steps' log could not be read: {}\n",
                run.workflow,
                trunc(err, 300)
            ));
        }
        if !rerun_at.is_empty() {
            log_notes.push_str(&format!(
                "\nGitHub no longer had the log of the first attempt, so hive reran the failed jobs at {rerun_at}; the runs and lines above are the rerun's.\n"
            ));
        }
        let landed = self.what_landed(push);
        let verify_cmd = self.project.str("verify_cmd");
        let mut job_id = String::new();
        let mut job_note = String::new();
        if !self.held.is_empty() {
            job_note = format!(
                "No fix job is planned yet: a worker starts from the project checkout, and {}. The daemon plans the fix once the checkout holds this commit.",
                self.held
            );
        } else if self.fix {
            let source = fix_source(self.slug);
            let since = plus_hours(self.now, -FIX_LIMIT_HOURS);
            let recent: i64 = self
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM jobs WHERE project = ? AND source = ? AND created_at >= ?",
                    rusqlite::params![self.slug, source, since],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let open = must_rows(
                self.conn,
                "SELECT id, status FROM jobs WHERE project = ? AND source = ? AND status IN ('queued', 'leased', 'blocked') ORDER BY created_at DESC, id DESC LIMIT 1",
                &[&self.slug, &source],
            )
            .into_iter()
            .next();
            if recent >= FIX_LIMIT && open.is_none() {
                job_note = format!(
                    "The daemon planned {FIX_LIMIT} fix jobs for this project's pipeline in the last {} hours and it is red again, so it plans no more until that window moves: this one is yours to look at.",
                    FIX_LIMIT_HOURS as i64
                );
            } else {
                let title = trunc(
                    &format!(
                        "Make the pipeline of {} green: {} failed at {}",
                        self.slug,
                        names(&failed),
                        push.short()
                    ),
                    160,
                );
                let brief = fix_brief(
                    self.slug,
                    self.repo,
                    push,
                    &failed,
                    &passed,
                    &running,
                    &details,
                    &log_notes,
                    &landed,
                    &verify_cmd,
                );
                match crate::jobs::add_job(
                    self.s,
                    self.slug.to_string(),
                    "task".into(),
                    title.clone(),
                    brief.clone(),
                    FIX_TIER,
                    FIX_PRIORITY,
                    source,
                    self.actor.to_string(),
                    true,
                ) {
                    Ok((job, created)) => {
                        job_id = job.str("id");
                        if !created {
                            // The open job has not been leased yet: it takes the
                            // newest failure's brief, so the worker reads what is
                            // red now, not what was.
                            let _ = self.conn.execute(
                                "UPDATE jobs SET title = ?, brief = ? WHERE id = ? AND status = 'queued'",
                                rusqlite::params![title, brief, job_id],
                            );
                            job_note = format!(
                                "The fix job {job_id} is already open for this project's pipeline and takes this failure too."
                            );
                        }
                    }
                    Err(e) => {
                        job_note = format!(
                            "The fix job could not be planned: {}",
                            crate::run::err_message(&e)
                        );
                    }
                }
            }
        } else {
            job_note = format!(
                "{WATCH} is `{}`: the daemon plans no fix job; `hive settings set {WATCH} fix` makes it plan one for the next red pipeline.",
                self.mode
            );
        }
        let summary = format!(
            "pipeline of {} red at {}: {} failed after the push to {}{}{}",
            self.slug,
            push.short(),
            names(&failed),
            push.target(),
            pushed_by(push),
            if job_id.is_empty() {
                String::new()
            } else {
                format!("; fix job {job_id} planned")
            }
        );
        let mut body = format!(
            "{} pushed {} to {} of {}{} and GitHub ran the repository's workflows; {} of {} failed:\n\n{}\n",
            push.who(),
            push.short(),
            push.target(),
            self.repo,
            push.parenthetical(),
            failed.len(),
            failed.len() + passed.len() + running.len(),
            failed
                .iter()
                .map(|r| format!(
                    "    {} ({}, {}): {}",
                    r.workflow, r.event, r.conclusion, r.url
                ))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        if !passed.is_empty() {
            body.push_str(&format!("\nPassed: {}.\n", names(&passed)));
        }
        if !running.is_empty() {
            body.push_str(&format!("\nStill running: {}.\n", names(&running)));
        }
        for (run, steps, log) in &details {
            if !steps.is_empty() {
                body.push_str(&format!(
                    "\n{}, failed jobs and steps:\n\n{}\n",
                    run.workflow,
                    steps
                        .iter()
                        .map(|l| format!("    {l}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                ));
            }
            if !log.is_empty() {
                body.push_str(&format!(
                    "\n{}, last lines of the failed steps:\n\n{}\n",
                    run.workflow,
                    log.lines()
                        .map(|l| format!("    {l}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                ));
            }
        }
        body.push_str(&log_notes);
        if !landed.is_empty() {
            body.push_str(&format!("\nWhat the push carried:\n\n{landed}\n"));
        }
        if push.head() {
            body.push_str(&format!(
                "\nThe push was not hive's; hive looks after {}'s default branch all the same, so a red pipeline there is its to make green.\n",
                self.slug
            ));
        }
        body.push_str(&format!("\n{job_note}\n"));
        let mut post = String::new();
        match tell_owner(self.s, self.slug, self.now, self.actor, summary, body) {
            Ok(id) => post = id,
            Err(why) => self.lines.push(format!(
                "pipeline {}: the owner could not be told: {why}",
                self.slug
            )),
        }
        if !job_id.is_empty() {
            d.insert("job".into(), json!(job_id));
        }
        if !post.is_empty() {
            d.insert("post".into(), json!(post));
        }
        event_at(
            self.conn,
            self.now,
            self.actor,
            "pipeline.failed",
            self.slug,
            Value::Object(d),
        );
        self.lines.push(format!(
            "pipeline {}: red at {} on {}{}: {} failed{}{}",
            self.slug,
            push.short(),
            push.target(),
            pushed_by(push),
            names(&failed),
            if job_id.is_empty() {
                String::new()
            } else {
                format!("; fix job {job_id}")
            },
            if post.is_empty() {
                String::new()
            } else {
                format!(" ({post})")
            }
        ));
        true
    }

    /// The failed jobs' steps and the log tail of the first failed runs,
    /// and beside them the runs whose log could not be read, with the error.
    #[allow(clippy::type_complexity)]
    fn failure_details(
        &self,
        failed: &[&Run],
    ) -> (Vec<(Run, Vec<String>, String)>, Vec<(Run, String)>) {
        let mut unread = Vec::new();
        let details = failed
            .iter()
            .take(DETAIL_RUNS)
            .map(|r| {
                let steps = (self.gh)(
                    &Ask::Jobs {
                        repo: self.repo,
                        id: r.id,
                    },
                    self.watch.user(),
                )
                .map(|b| failed_steps(&b))
                .unwrap_or_default();
                let log = match (self.gh)(
                    &Ask::Log {
                        repo: self.repo,
                        id: r.id,
                    },
                    self.watch.user(),
                ) {
                    Ok(l) => log_tail(&l),
                    Err(e) => {
                        unread.push(((*r).clone(), e));
                        String::new()
                    }
                };
                ((*r).clone(), steps, log)
            })
            .collect();
        (details, unread)
    }

    /// The commits the push carried, from the project checkout.
    fn what_landed(&self, push: &Push) -> String {
        let root = self.project.str("path");
        if push.from.is_empty() || push.sha.is_empty() || root.is_empty() {
            return String::new();
        }
        if !std::path::Path::new(&root).join(".git").exists() {
            return String::new();
        }
        let (code, out, _) = crate::land::sh_with_timeout(
            root,
            format!(
                "git log --no-decorate --format='%h %s' --max-count=30 {}..{}",
                crate::util::shell_quote(&push.from),
                crate::util::shell_quote(&push.sha)
            ),
            Duration::from_secs(20),
        );
        if code.as_deref() != Some(&0) {
            return String::new();
        }
        out.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| format!("    {l}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[allow(clippy::too_many_arguments)]
fn fix_brief(
    slug: &str,
    repo: &str,
    push: &Push,
    failed: &[&Run],
    passed: &[&Run],
    running: &[&Run],
    details: &[(Run, Vec<String>, String)],
    log_notes: &str,
    landed: &str,
    verify_cmd: &str,
) -> String {
    let mut b = format!(
        "{} pushed {} to {} of {}{} and the repository's GitHub Actions pipeline went red: {} of {} workflow runs failed.\n\nFailed:\n\n{}\n",
        push.who(),
        push.short(),
        push.target(),
        repo,
        push.parenthetical(),
        failed.len(),
        failed.len() + passed.len() + running.len(),
        failed
            .iter()
            .map(|r| format!(
                "    {} ({}, {}): {}",
                r.workflow, r.event, r.conclusion, r.url
            ))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    if !passed.is_empty() {
        b.push_str(&format!("\nPassed: {}.\n", names(passed)));
    }
    if !running.is_empty() {
        b.push_str(&format!(
            "\nStill running when this was planned: {}.\n",
            names(running)
        ));
    }
    for (run, steps, log) in details {
        if !steps.is_empty() {
            b.push_str(&format!(
                "\n{} - failed jobs and steps:\n\n{}\n",
                run.workflow,
                steps
                    .iter()
                    .map(|l| format!("    {l}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        if !log.is_empty() {
            b.push_str(&format!(
                "\n{} - last lines of the failed steps:\n\n{}\n",
                run.workflow,
                log.lines()
                    .map(|l| format!("    {l}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
    }
    b.push_str(log_notes);
    if !landed.is_empty() {
        b.push_str(&format!("\nWhat the push carried:\n\n{landed}\n"));
    }
    if push.head() {
        b.push_str(&format!(
            "\nThe push was not hive's ({} made it), but hive looks after {slug}'s default branch, so the red pipeline is yours to make green. Your clone holds that commit: `git log -1 {}` shows it.\n",
            push.who(),
            push.short()
        ));
    }
    b.push_str(&format!(
        "\nFind why the run failed and make the pipeline green with the smallest change that is honest about the failure: fix the code or the test that broke, repair what the workflow needs (a dependency, a lockfile, a script or file the workflow calls), or correct the workflow file itself when the workflow is what is wrong. Do not disable, skip, soften or retarget the workflow or its checks, and do not change the project's verify command. Read the full log when the tail is not enough: `gh run view RUN_ID --repo {repo} --log-failed`.\n\nIf the failure is not the code's - a network timeout, a runner that went away, a secret or a permission the workflow lacks, a decision only the owner can make - stop with a SUMMARY that opens with \"blocked\" and says what is needed and whether `gh run rerun RUN_ID --failed` would do; do not retry blindly.\n",
    ));
    if !verify_cmd.trim().is_empty() {
        b.push_str(&format!(
            "\nThe project's own check is `{verify_cmd}`; run it before you finish, and keep {slug}'s conventions.\n"
        ));
    }
    b
}

/// The last verdicts the daemon recorded for a project, newest first.
pub(crate) fn verdicts(
    q: &(impl crate::util::Querier + ?Sized),
    slug: &str,
    limit: i64,
) -> Vec<Row> {
    must_rows(
        q,
        "SELECT seq, at, action, detail FROM events WHERE subject = ? AND action LIKE 'pipeline.%' AND action != 'pipeline.unreadable' ORDER BY seq DESC LIMIT ?",
        &[&slug, &limit],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::*;
    use crate::test_helpers_daemon::{exec1, set_setting};
    use std::cell::RefCell;

    const NOW: &str = "2026-10-31T10:00:00Z";
    const SHA: &str = "8f377827f6c6f7823975f2b6db8561b7a09f42d3";
    const FROM: &str = "fc4ca8e79fbfb23c7088f64d2506805fd8489fa5";

    fn setup() -> TestStore {
        let s = new_store();
        let _ = new_repo(&s);
        exec1(
            &s,
            "UPDATE projects SET remote = 'git@github.com:o/r.git', verify_cmd = 'make check' WHERE slug = 'proj'",
            &[],
        );
        s
    }

    fn pushed(s: &hive_store::Store, at: &str, sha: &str) {
        event_at(
            &s.conn().unwrap(),
            at,
            "daemon",
            "land.pushed",
            "proj",
            json!({"project": "proj", "remote": "origin", "branch": "main", "from": FROM, "to": sha, "commits": 2}),
        );
    }

    fn run(workflow: &str, event: &str, status: &str, conclusion: &str, id: i64) -> Value {
        json!({
            "databaseId": id,
            "workflowName": workflow,
            "event": event,
            "status": status,
            "conclusion": conclusion,
            "url": format!("https://github.com/o/r/actions/runs/{id}"),
            "headBranch": "main",
        })
    }

    /// What `gh api repos/o/r/branches/main` answers: the branch's head.
    fn head_json(sha: &str, by: &str, at: &str) -> Value {
        json!({
            "name": "main",
            "commit": {
                "sha": sha,
                "html_url": format!("https://github.com/o/r/commit/{sha}"),
                "author": if by.is_empty() { Value::Null } else { json!({"login": by}) },
                "commit": {
                    "message": "Merge pull request #7 from o/dependabot/npm_and_yarn/x\n\nbump x",
                    "author": {"name": "Some One", "date": at},
                    "committer": {"name": "GitHub", "date": at}
                }
            }
        })
    }

    /// A gh that answers `run list` with `runs`, jobs/log for a failed run,
    /// and the branch's head with hive's own push (`SHA`); every ask is
    /// logged.
    fn gh(runs: Vec<Value>) -> (Box<Gh<'static>>, std::rc::Rc<RefCell<Vec<String>>>) {
        gh_with(runs, head_json(SHA, "hive-bot", "2026-10-31T09:50:00Z"))
    }

    fn runs_read(log: &RefCell<Vec<String>>) -> usize {
        log.borrow()
            .iter()
            .filter(|l| l.starts_with("runs"))
            .count()
    }

    fn gh_with(
        runs: Vec<Value>,
        head: Value,
    ) -> (Box<Gh<'static>>, std::rc::Rc<RefCell<Vec<String>>>) {
        gh_answering(runs, head, Ok(LOG.to_string()), Ok(String::new()))
    }

    const LOG: &str = "test (ubuntu)\tnpm test\tFAIL src/a.test.ts\ntest (ubuntu)\tnpm test\t  expected 2, got 3\ntest (ubuntu)\tnpm test\tToken: ghp_abcdefghijklmnopqrstuvwxyz0123456789\n";
    const LOG_GONE: &str =
        "gh run view --log-failed exited 1: failed to get run log: HTTP 410: Server Error";

    /// `gh_with`, with what `run view --log-failed` and `run rerun` answer.
    fn gh_answering(
        runs: Vec<Value>,
        head: Value,
        log_answer: Result<String, String>,
        rerun_answer: Result<String, String>,
    ) -> (Box<Gh<'static>>, std::rc::Rc<RefCell<Vec<String>>>) {
        let log = std::rc::Rc::new(RefCell::new(Vec::new()));
        let seen = log.clone();
        let f = move |ask: &Ask, _: Option<&str>| -> Result<String, String> {
            match ask {
                Ask::Rerun { repo, id } => {
                    seen.borrow_mut().push(format!("rerun {repo} {id}"));
                    rerun_answer.clone()
                }
                Ask::Head { repo, branch } => {
                    seen.borrow_mut().push(format!("head {repo} {branch}"));
                    if head.is_null() {
                        Err("gh api: HTTP 404: Branch not found".to_string())
                    } else {
                        Ok(head.to_string())
                    }
                }
                Ask::Runs { repo, sha } => {
                    seen.borrow_mut().push(format!("runs {repo} {sha}"));
                    Ok(Value::Array(runs.clone()).to_string())
                }
                Ask::Jobs { repo, id } => {
                    seen.borrow_mut().push(format!("jobs {repo} {id}"));
                    Ok(json!({"jobs": [
                        {"name": "build", "conclusion": "success", "steps": []},
                        {"name": "test (ubuntu)", "conclusion": "failure", "steps": [
                            {"name": "Checkout", "conclusion": "success"},
                            {"name": "npm test", "conclusion": "failure"}
                        ]}
                    ]})
                    .to_string())
                }
                Ask::Log { repo, id } => {
                    seen.borrow_mut().push(format!("log {repo} {id}"));
                    log_answer.clone()
                }
            }
        };
        (Box::new(f), log)
    }

    fn events(s: &hive_store::Store, action: &str) -> Vec<Value> {
        must_rows(
            &s.conn().unwrap(),
            "SELECT detail FROM events WHERE action = ? ORDER BY seq",
            &[&action],
        )
        .iter()
        .map(|r| serde_json::from_str(&r.str("detail")).unwrap())
        .collect()
    }

    fn jobs(s: &hive_store::Store) -> Vec<Row> {
        must_rows(
            &s.conn().unwrap(),
            "SELECT * FROM jobs WHERE source LIKE 'pipeline:%' ORDER BY created_at, id",
            &[],
        )
    }

    fn posts(s: &hive_store::Store) -> Vec<Row> {
        must_rows(
            &s.conn().unwrap(),
            "SELECT * FROM posts WHERE thread = 'pipeline' ORDER BY id",
            &[],
        )
    }

    #[test]
    fn a_green_pipeline_is_recorded_once_and_nothing_is_planned() {
        let s = setup();
        pushed(&s, "2026-10-31T09:50:00Z", SHA);
        let (gh, log) = gh(vec![
            run("CI", "push", "completed", "success", 1),
            run("Deploy", "push", "completed", "success", 2),
            run("Dependabot Updates", "dynamic", "completed", "failure", 3),
            run("Nightly", "schedule", "completed", "failure", 4),
        ]);
        let lines = tend_pipeline_with(&s, "proj", NOW, "daemon", &*gh);
        assert_eq!(
            lines,
            ["pipeline proj: green at 8f377827f6c6 on origin/main (CI, Deploy)".to_string()]
        );
        let passed = events(&s, "pipeline.passed");
        assert_eq!(passed.len(), 1);
        assert_eq!(passed[0]["sha"], SHA);
        assert_eq!(
            passed[0]["runs"].as_array().unwrap().len(),
            2,
            "{}",
            passed[0]
        );
        assert!(jobs(&s).is_empty());
        assert!(posts(&s).is_empty());
        assert_eq!(
            &*log.borrow(),
            &[format!("runs o/r {SHA}"), "head o/r main".to_string()]
        );
        // Judged: the next pass does not ask again.
        let lines = tend_pipeline_with(&s, "proj", NOW, "daemon", &*gh);
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(runs_read(&log), 1);
        assert!(pending(&s.conn().unwrap(), "proj", NOW).is_empty());
    }

    #[test]
    fn a_red_pipeline_tells_the_owner_and_plans_one_fix_job_with_the_failure_in_its_brief() {
        let s = setup();
        pushed(&s, "2026-10-31T09:50:00Z", SHA);
        let (gh, log) = gh(vec![
            run("CI", "push", "completed", "failure", 11),
            run("Deploy", "push", "completed", "success", 12),
        ]);
        let lines = tend_pipeline_with(&s, "proj", NOW, "daemon", &*gh);
        assert_eq!(lines.len(), 1, "{lines:?}");
        let js = jobs(&s);
        assert_eq!(js.len(), 1, "{js:?}");
        let job = &js[0];
        assert_eq!(job.str("kind"), "task");
        assert_eq!(job.str("source"), "pipeline:proj");
        assert_eq!(job.int("priority"), 60);
        assert_eq!(
            job.str("title"),
            "Make the pipeline of proj green: CI failed at 8f377827f6c6"
        );
        let brief = job.str("brief");
        assert!(
            brief.contains("CI (push, failure): https://github.com/o/r/actions/runs/11"),
            "{brief}"
        );
        assert!(brief.contains("test (ubuntu): npm test"), "{brief}");
        assert!(brief.contains("expected 2, got 3"), "{brief}");
        assert!(
            !brief.contains("ghp_abcdefghijklmnopqrstuvwxyz0123456789"),
            "the brief redacts secrets: {brief}"
        );
        assert!(brief.contains("Passed: Deploy."), "{brief}");
        assert!(brief.contains("`make check`"), "{brief}");
        assert!(brief.contains("gh run rerun RUN_ID --failed"), "{brief}");
        assert!(
            lines[0].starts_with(&format!(
                "pipeline proj: red at 8f377827f6c6 on origin/main: CI failed; fix job {}",
                job.str("id")
            )),
            "{lines:?}"
        );
        let ps = posts(&s);
        assert_eq!(ps.len(), 1, "{ps:?}");
        assert_eq!(ps[0].str("kind"), "finding");
        assert_eq!(ps[0].str("project"), "proj");
        assert!(ps[0].str("summary").starts_with("pipeline of proj red at 8f377827f6c6: CI failed after the push to origin/main; fix job "), "{}", ps[0].str("summary"));
        assert!(
            ps[0].str("body").contains("test (ubuntu): npm test"),
            "{}",
            ps[0].str("body")
        );
        let failed = events(&s, "pipeline.failed");
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0]["failed"], json!(["CI"]));
        assert_eq!(failed[0]["job"], job.str("id"));
        assert_eq!(failed[0]["post"], ps[0].str("id"));
        assert_eq!(
            &*log.borrow(),
            &[
                format!("runs o/r {SHA}"),
                "jobs o/r 11".to_string(),
                "log o/r 11".to_string(),
                "head o/r main".to_string()
            ]
        );

        // The next push fails too while the job waits: no second job, the
        // queued one takes the newer brief, the owner hears once more.
        let sha2 = "aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111";
        pushed(&s, "2026-10-31T10:20:00Z", sha2);
        let (gh2, _) = self::gh(vec![
            run("CI", "push", "completed", "failure", 21),
            run("Deploy", "push", "completed", "failure", 22),
        ]);
        let later = "2026-10-31T10:40:00Z";
        let lines = tend_pipeline_with(&s, "proj", later, "daemon", &*gh2);
        assert_eq!(lines.len(), 1, "{lines:?}");
        let js = jobs(&s);
        assert_eq!(js.len(), 1, "one open fix job per project: {js:?}");
        assert_eq!(
            js[0].str("title"),
            "Make the pipeline of proj green: CI, Deploy failed at aaaa1111aaaa"
        );
        assert!(js[0].str("brief").contains("actions/runs/22"));
        assert_eq!(posts(&s).len(), 2);
        assert!(
            posts(&s)[1]
                .str("body")
                .contains(&format!("The fix job {} is already open", js[0].str("id"))),
            "{}",
            posts(&s)[1].str("body")
        );
    }

    #[test]
    fn watch_mode_tells_the_owner_without_a_job_and_off_does_nothing() {
        let s = setup();
        pushed(&s, "2026-10-31T09:50:00Z", SHA);
        let (gh, log) = gh(vec![run("CI", "push", "completed", "failure", 11)]);
        set_setting(&s, WATCH, "off");
        assert!(tend_pipeline_with(&s, "proj", NOW, "daemon", &*gh).is_empty());
        assert!(log.borrow().is_empty());

        set_setting(&s, WATCH, "watch");
        let lines = tend_pipeline_with(&s, "proj", NOW, "daemon", &*gh);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(jobs(&s).is_empty());
        let ps = posts(&s);
        assert_eq!(ps.len(), 1);
        assert!(
            ps[0]
                .str("body")
                .contains("pipeline.watch is `watch`: the daemon plans no fix job"),
            "{}",
            ps[0].str("body")
        );
        assert_eq!(events(&s, "pipeline.failed")[0].get("job"), None);
    }

    #[test]
    fn running_waits_and_polls_every_few_minutes_then_gives_up_at_the_deadline() {
        let s = setup();
        pushed(&s, "2026-10-31T09:50:00Z", SHA);
        let (gh, log) = gh(vec![run("CI", "push", "in_progress", "", 31)]);
        assert!(tend_pipeline_with(&s, "proj", NOW, "daemon", &*gh).is_empty());
        assert_eq!(runs_read(&log), 1);
        // Two minutes later: not asked again.
        assert!(tend_pipeline_with(&s, "proj", "2026-10-31T10:02:00Z", "daemon", &*gh).is_empty());
        assert_eq!(runs_read(&log), 1);
        // Four minutes later: asked again, still waits.
        assert!(tend_pipeline_with(&s, "proj", "2026-10-31T10:04:00Z", "daemon", &*gh).is_empty());
        assert_eq!(runs_read(&log), 2);
        assert!(events(&s, "pipeline.gave-up").is_empty());
        // Past pipeline.wait_hours (3): gave up, the owner is told.
        let lines = tend_pipeline_with(&s, "proj", "2026-10-31T12:51:00Z", "daemon", &*gh);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].contains(
                "still running 3 h after 8f377827f6c6 on origin/main: CI; stopped waiting"
            ),
            "{lines:?}"
        );
        let gave = events(&s, "pipeline.gave-up");
        assert_eq!(gave.len(), 1);
        let ps = posts(&s);
        assert_eq!(ps.len(), 1);
        assert_eq!(gave[0]["post"], ps[0].str("id"));
        assert!(jobs(&s).is_empty());
        assert!(load_watch(&s, "proj").polled.is_empty());
    }

    #[test]
    fn a_failure_beside_a_long_run_is_reported_after_half_an_hour() {
        let s = setup();
        pushed(&s, "2026-10-31T09:50:00Z", SHA);
        let (gh, _) = gh(vec![
            run("CI", "push", "completed", "failure", 41),
            run("Deploy", "push", "in_progress", "", 42),
        ]);
        assert!(tend_pipeline_with(&s, "proj", NOW, "daemon", &*gh).is_empty());
        let lines = tend_pipeline_with(&s, "proj", "2026-10-31T10:21:00Z", "daemon", &*gh);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(jobs(&s).len(), 1);
        assert!(
            jobs(&s)[0]
                .str("brief")
                .contains("Still running when this was planned: Deploy.")
        );
    }

    #[test]
    fn no_run_within_the_grace_is_recorded_as_none_and_cancelled_runs_count_as_none() {
        let s = setup();
        pushed(&s, "2026-10-31T09:55:00Z", SHA);
        let (gh, _) = gh(vec![run(
            "Manual",
            "workflow_dispatch",
            "completed",
            "failure",
            51,
        )]);
        assert!(
            tend_pipeline_with(&s, "proj", NOW, "daemon", &*gh).is_empty(),
            "within the grace"
        );
        let lines = tend_pipeline_with(&s, "proj", "2026-10-31T10:06:00Z", "daemon", &*gh);
        assert_eq!(
            lines,
            ["pipeline proj: no workflow ran for 8f377827f6c6 on origin/main"]
        );
        assert_eq!(events(&s, "pipeline.none")[0]["cancelled"], 0);

        let sha2 = "bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222";
        pushed(&s, "2026-10-31T10:07:00Z", sha2);
        let (gh2, _) = self::gh(vec![run("CI", "push", "completed", "cancelled", 52)]);
        let lines = tend_pipeline_with(&s, "proj", "2026-10-31T10:30:00Z", "daemon", &*gh2);
        assert_eq!(
            lines,
            ["pipeline proj: every run for bbbb2222bbbb on origin/main was cancelled (1)"]
        );
        assert!(jobs(&s).is_empty());
        assert!(posts(&s).is_empty());
    }

    #[test]
    fn an_older_push_is_judged_but_only_the_newest_gets_the_job() {
        let s = setup();
        pushed(&s, "2026-10-31T09:00:00Z", SHA);
        let sha2 = "cccc3333cccc3333cccc3333cccc3333cccc3333";
        pushed(&s, "2026-10-31T09:30:00Z", sha2);
        let (gh, log) = gh(vec![run("CI", "push", "completed", "failure", 61)]);
        let lines = tend_pipeline_with(&s, "proj", NOW, "daemon", &*gh);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(
            lines[0].contains("a later push is pending, nothing planned"),
            "{lines:?}"
        );
        let failed = events(&s, "pipeline.failed");
        assert_eq!(failed.len(), 2);
        assert_eq!(failed[0]["superseded"], true);
        assert_eq!(failed[1].get("superseded"), None);
        assert_eq!(jobs(&s).len(), 1);
        assert_eq!(posts(&s).len(), 1);
        assert_eq!(
            log.borrow()
                .iter()
                .filter(|l| l.starts_with("runs"))
                .count(),
            2
        );
    }

    #[test]
    fn a_pull_request_push_is_watched_on_its_head_and_named_by_its_number() {
        let s = setup();
        event_at(
            &s.conn().unwrap(),
            "2026-10-31T09:50:00Z",
            "daemon",
            "land.pr",
            "proj",
            json!({"project": "proj", "remote": "origin", "branch": "main", "head": "hive/landed", "base": "main", "number": 7, "url": "https://github.com/o/r/pull/7", "from": FROM, "to": SHA, "commits": 1, "opened": true, "conflicts": false}),
        );
        let (gh, _) = gh(vec![run("CI", "pull_request", "completed", "failure", 71)]);
        let lines = tend_pipeline_with(&s, "proj", NOW, "daemon", &*gh);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].contains("on pull request #7: CI failed"),
            "{lines:?}"
        );
        assert_eq!(events(&s, "pipeline.failed")[0]["number"], 7);
        let brief = jobs(&s)[0].str("brief");
        assert!(
            brief.contains("to pull request #7 of o/r (1 commit, https://github.com/o/r/pull/7)"),
            "{brief}"
        );
    }

    #[test]
    fn the_fix_limit_hands_the_fourth_red_pipeline_of_a_day_to_the_owner() {
        let s = setup();
        // The branch's head on the forge was judged already: only the pushes
        // count here.
        let (gh, _) = gh_with(
            vec![run("CI", "push", "completed", "failure", 81)],
            head_json(FROM, "someone-else", "2026-10-30T22:00:00Z"),
        );
        event_at(
            &s.conn().unwrap(),
            "2026-10-30T23:00:00Z",
            "daemon",
            "pipeline.passed",
            "proj",
            json!({"project": "proj", "sha": FROM, "runs": []}),
        );
        for (i, at) in [
            "2026-10-31T01:00:00Z",
            "2026-10-31T03:00:00Z",
            "2026-10-31T05:00:00Z",
        ]
        .iter()
        .enumerate()
        {
            let sha = format!(
                "{i}{i}{i}{i}aaaa{i}{i}{i}{i}aaaa{i}{i}{i}{i}aaaa{i}{i}{i}{i}aaaa{i}{i}{i}{i}aaaa"
            );
            pushed(&s, at, &sha);
            let later = plus_minutes(at, 20);
            assert_eq!(
                tend_pipeline_with(&s, "proj", &later, "daemon", &*gh).len(),
                1
            );
            // The store stamps jobs with the wall clock; pin the one just
            // planned to the scenario's clock and let it fail.
            exec1(
                &s,
                "UPDATE jobs SET status = 'failed', created_at = ?, finished_at = ? WHERE status = 'queued' AND source = 'pipeline:proj'",
                &[&later, &plus_minutes(&later, 30)],
            );
        }
        assert_eq!(jobs(&s).len(), 3);
        pushed(&s, "2026-10-31T09:50:00Z", SHA);
        let lines = tend_pipeline_with(&s, "proj", NOW, "daemon", &*gh);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(jobs(&s).len(), 3, "no fourth job");
        let ps = posts(&s);
        assert_eq!(ps.len(), 4);
        assert!(
            ps[3]
                .str("body")
                .contains("planned 3 fix jobs for this project's pipeline in the last 24 hours"),
            "{}",
            ps[3].str("body")
        );
    }

    #[test]
    fn an_unreadable_repository_is_told_once_left_alone_an_hour_and_a_non_github_remote_is_skipped()
    {
        let s = setup();
        pushed(&s, "2026-10-31T09:50:00Z", SHA);
        let calls = RefCell::new(0);
        let gh = |_: &Ask, user: Option<&str>| -> Result<String, String> {
            assert_eq!(user, None, "the active login is the only account");
            *calls.borrow_mut() += 1;
            Err(
                "gh run list exited 1: HTTP 403: Resource not accessible by personal access token"
                    .into(),
            )
        };
        let lines = tend_pipeline_with(&s, "proj", NOW, "daemon", &gh);
        // The push's runs and the owner told; the head of a repository left
        // alone is not read.
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(
            lines[0],
            "pipeline proj: the runs of 8f377827f6c6 could not be read as the active gh login: gh run list exited 1: HTTP 403: Resource not accessible by personal access token; o/r is left alone until 2026-10-31T11:00:00Z"
        );
        let ps = posts(&s);
        assert_eq!(ps.len(), 1);
        assert_eq!(
            ps[0].str("summary"),
            "hive cannot read the workflow runs of o/r: the active gh login needs read access to the repository's Actions"
        );
        let unreadable = events(&s, "pipeline.unreadable");
        assert_eq!(unreadable.len(), 1);
        assert_eq!(unreadable[0]["tried"], json!(["the active gh login"]));
        assert_eq!(unreadable[0]["until"], "2026-10-31T11:00:00Z");
        assert_eq!(*calls.borrow(), 1);
        let w = load_watch(&s, "proj");
        assert_eq!(w.unreadable_until, "2026-10-31T11:00:00Z");
        assert_eq!(w.unreadable_told, "o/r");
        // Inside the hour nothing is asked.
        assert!(tend_pipeline_with(&s, "proj", "2026-10-31T10:05:00Z", "daemon", &gh).is_empty());
        assert_eq!(*calls.borrow(), 1);
        // The hour over: asked once more, the owner not told again.
        let lines = tend_pipeline_with(&s, "proj", "2026-10-31T11:00:00Z", "daemon", &gh);
        assert_eq!(lines.len(), 1, "told once: {lines:?}");
        assert_eq!(*calls.borrow(), 2);
        assert_eq!(posts(&s).len(), 1);
        assert_eq!(
            load_watch(&s, "proj").unreadable_until,
            "2026-10-31T12:00:00Z"
        );

        exec1(
            &s,
            "UPDATE projects SET remote = 'git@gitlab.com:o/r.git' WHERE slug = 'proj'",
            &[],
        );
        assert!(tend_pipeline_with(&s, "proj", "2026-10-31T12:10:00Z", "daemon", &gh).is_empty());
        assert_eq!(*calls.borrow(), 2);
    }

    /// The machine's gh logins: a fine-grained token is active beside two
    /// accounts.
    fn logins() -> Logins {
        Logins {
            active: "pat-login".into(),
            accounts: vec!["pat-login".into(), "octocat".into(), "third".into()],
        }
    }

    const NOT_FOUND: &str = "gh run list exited 1: failed to get runs: HTTP 404: Not Found (https://api.github.com/repos/o/r/actions/runs)";

    /// A gh where only the accounts named in `sees` can see the repository
    /// (`""` is the active login); every ask is logged with the account it
    /// ran as. The head is hive's own push `SHA`.
    fn gh_seen_by(
        sees: &'static [&'static str],
        runs: Vec<Value>,
    ) -> (Box<Gh<'static>>, std::rc::Rc<RefCell<Vec<String>>>) {
        let log = std::rc::Rc::new(RefCell::new(Vec::new()));
        let seen = log.clone();
        let f = move |ask: &Ask, user: Option<&str>| -> Result<String, String> {
            let who = user.unwrap_or("active");
            seen.borrow_mut().push(match ask {
                Ask::Runs { sha, .. } => format!("runs {sha} as {who}"),
                Ask::Head { branch, .. } => format!("head {branch} as {who}"),
                Ask::Jobs { id, .. } => format!("jobs {id} as {who}"),
                Ask::Log { id, .. } => format!("log {id} as {who}"),
                Ask::Rerun { id, .. } => format!("rerun {id} as {who}"),
            });
            if !sees.contains(&user.unwrap_or("")) {
                return Err(NOT_FOUND.to_string());
            }
            Ok(match ask {
                Ask::Runs { .. } => Value::Array(runs.clone()).to_string(),
                Ask::Head { .. } => head_json(SHA, "hive-bot", "2026-10-31T09:50:00Z").to_string(),
                _ => String::new(),
            })
        };
        (Box::new(f), log)
    }

    #[test]
    fn an_active_login_that_cannot_see_the_repository_gives_way_to_the_account_that_can() {
        let s = setup();
        pushed(&s, "2026-10-31T09:50:00Z", SHA);
        let (gh, log) = gh_seen_by(
            &["octocat"],
            vec![run("CI", "push", "completed", "success", 1)],
        );
        let lines = tend_pipeline_as(&s, "proj", NOW, "daemon", &*gh, &logins);
        assert_eq!(
            lines,
            [
                "pipeline proj: reading o/r as octocat, since the active gh login pat-login cannot see it".to_string(),
                "pipeline proj: green at 8f377827f6c6 on origin/main (CI)".to_string(),
            ]
        );
        // The account found reads the head too, with no second try of the
        // active login.
        assert_eq!(
            &*log.borrow(),
            &[
                format!("runs {SHA} as active"),
                format!("runs {SHA} as octocat"),
                "head main as octocat".to_string()
            ]
        );
        let w = load_watch(&s, "proj");
        assert_eq!(w.gh_user, "octocat");
        assert!(w.unreadable_until.is_empty());
        assert!(posts(&s).is_empty());
        assert!(events(&s, "pipeline.unreadable").is_empty());

        // The next push is read as that account first, and nothing is said
        // about the choice again.
        let sha2 = "aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111";
        pushed(&s, "2026-10-31T10:05:00Z", sha2);
        let lines = tend_pipeline_as(&s, "proj", "2026-10-31T10:11:00Z", "daemon", &*gh, &logins);
        assert_eq!(
            lines,
            ["pipeline proj: green at aaaa1111aaaa on origin/main (CI)".to_string()]
        );
        assert_eq!(
            &log.borrow()[3..],
            &[
                format!("runs {sha2} as octocat"),
                "head main as octocat".to_string()
            ]
        );
    }

    #[test]
    fn a_repository_no_account_can_see_is_left_alone_for_an_hour_and_the_owner_told_once() {
        let s = setup();
        pushed(&s, "2026-10-31T09:50:00Z", SHA);
        let (gh, log) = gh_seen_by(&[], vec![run("CI", "push", "completed", "success", 1)]);
        let lines = tend_pipeline_as(&s, "proj", NOW, "daemon", &*gh, &logins);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(
            lines[0],
            format!(
                "pipeline proj: the runs of 8f377827f6c6 could not be read as the active gh login pat-login, octocat and third: {NOT_FOUND}; o/r is left alone until 2026-10-31T11:00:00Z"
            )
        );
        // Every account once, and the head not at all.
        assert_eq!(
            &*log.borrow(),
            &[
                format!("runs {SHA} as active"),
                format!("runs {SHA} as octocat"),
                format!("runs {SHA} as third"),
            ]
        );
        let ps = posts(&s);
        assert_eq!(ps.len(), 1);
        assert_eq!(
            ps[0].str("summary"),
            "hive cannot read the workflow runs of o/r: none of the gh logins on this machine (the active gh login pat-login, octocat and third) can see it"
        );
        assert!(
            ps[0].str("body").contains(
                "it tried the active gh login pat-login, octocat and third and none could"
            ),
            "{}",
            ps[0].str("body")
        );
        let unreadable = events(&s, "pipeline.unreadable");
        assert_eq!(unreadable.len(), 1);
        assert_eq!(
            unreadable[0]["tried"],
            json!(["pat-login", "octocat", "third"])
        );
        let w = load_watch(&s, "proj");
        assert_eq!(w.unreadable_until, "2026-10-31T11:00:00Z");
        assert!(w.gh_user.is_empty());

        // Inside the hour: no gh call, no line.
        let lines = tend_pipeline_as(&s, "proj", "2026-10-31T10:30:00Z", "daemon", &*gh, &logins);
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(log.borrow().len(), 3);

        // The hour over: the accounts are tried once more, the owner is not
        // told again, and the repository waits another hour.
        let lines = tend_pipeline_as(&s, "proj", "2026-10-31T11:00:00Z", "daemon", &*gh, &logins);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(log.borrow().len(), 6);
        assert_eq!(posts(&s).len(), 1);
        assert_eq!(
            load_watch(&s, "proj").unreadable_until,
            "2026-10-31T12:00:00Z"
        );

        // Given access, the account that has it is found and remembered,
        // and the repository is no longer left alone.
        let (gh2, log2) = gh_seen_by(
            &["third"],
            vec![run("CI", "push", "completed", "success", 1)],
        );
        let lines = tend_pipeline_as(&s, "proj", "2026-10-31T12:00:00Z", "daemon", &*gh2, &logins);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].starts_with("pipeline proj: reading o/r as third, since the active gh login pat-login cannot see it"), "{lines:?}");
        assert_eq!(
            &*log2.borrow(),
            &[
                format!("runs {SHA} as active"),
                format!("runs {SHA} as octocat"),
                format!("runs {SHA} as third"),
                "head main as third".to_string()
            ]
        );
        let w = load_watch(&s, "proj");
        assert_eq!(w.gh_user, "third");
        assert!(w.unreadable_until.is_empty());
        assert!(w.unreadable_told.is_empty());
        assert_eq!(posts(&s).len(), 1);
    }

    #[test]
    fn a_remembered_account_that_fails_is_forgotten_and_the_accounts_tried_again() {
        let s = setup();
        pushed(&s, "2026-10-31T09:50:00Z", SHA);
        save_watch(
            &s.conn().unwrap(),
            "proj",
            &Watch {
                gh_user: "octocat".into(),
                ..Default::default()
            },
        );
        // octocat lost its access: the active login is tried as itself,
        // then the other account by name.
        let (gh, log) = gh_seen_by(
            &["third"],
            vec![run("CI", "push", "completed", "success", 1)],
        );
        let lines = tend_pipeline_as(&s, "proj", NOW, "daemon", &*gh, &logins);
        assert_eq!(
            lines[0], "pipeline proj: reading o/r as third, since octocat cannot see it",
            "{lines:?}"
        );
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(
            &*log.borrow(),
            &[
                format!("runs {SHA} as octocat"),
                format!("runs {SHA} as active"),
                format!("runs {SHA} as third"),
                "head main as third".to_string()
            ]
        );
        assert_eq!(load_watch(&s, "proj").gh_user, "third");
        assert!(posts(&s).is_empty());

        // The remembered account fails and the active login sees the
        // repository now: nothing is remembered any more.
        let sha2 = "aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111";
        pushed(&s, "2026-10-31T10:20:00Z", sha2);
        let (gh2, log2) = gh_seen_by(&[""], vec![run("CI", "push", "completed", "success", 2)]);
        let lines = tend_pipeline_as(&s, "proj", "2026-10-31T10:21:00Z", "daemon", &*gh2, &logins);
        assert_eq!(
            lines[0],
            "pipeline proj: reading o/r as the active gh login pat-login again, since third cannot see it",
            "{lines:?}"
        );
        assert_eq!(
            &*log2.borrow(),
            &[
                format!("runs {sha2} as third"),
                format!("runs {sha2} as active"),
                "head main as active".to_string()
            ]
        );
        assert!(load_watch(&s, "proj").gh_user.is_empty());
    }

    #[test]
    fn an_account_whose_token_cannot_be_read_is_passed_over_like_one_that_cannot_see() {
        let s = setup();
        pushed(&s, "2026-10-31T09:50:00Z", SHA);
        // octocat is logged in but its token cannot be had (the error
        // `gh_auth::env_as` gives, wrapped by `gh_ask`); third sees the
        // repository until it is told otherwise.
        const NO_TOKEN: &str = "gh run list as octocat: gh auth token for octocat exited 1: no oauth token found for github.com account octocat";
        let third_sees = std::cell::Cell::new(true);
        let gh = |ask: &Ask, user: Option<&str>| -> Result<String, String> {
            match user {
                Some("octocat") => Err(NO_TOKEN.to_string()),
                Some("third") if third_sees.get() => Ok(match ask {
                    Ask::Runs { .. } => {
                        Value::Array(vec![run("CI", "push", "completed", "success", 1)]).to_string()
                    }
                    Ask::Head { .. } => {
                        head_json(SHA, "hive-bot", "2026-10-31T09:50:00Z").to_string()
                    }
                    _ => String::new(),
                }),
                _ => Err(NOT_FOUND.to_string()),
            }
        };
        let lines = tend_pipeline_as(&s, "proj", NOW, "daemon", &gh, &logins);
        assert_eq!(
            lines,
            [
                "pipeline proj: reading o/r as third, since the active gh login pat-login cannot see it".to_string(),
                "pipeline proj: green at 8f377827f6c6 on origin/main (CI)".to_string(),
            ]
        );
        let w = load_watch(&s, "proj");
        assert_eq!(w.gh_user, "third");
        assert!(w.unreadable_until.is_empty());

        // third loses its access too: the account without a token counts
        // among those tried, and the repository waits the hour rather than
        // failing on the token at every poll.
        third_sees.set(false);
        let sha2 = "aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111";
        pushed(&s, "2026-10-31T10:05:00Z", sha2);
        let lines = tend_pipeline_as(&s, "proj", "2026-10-31T10:11:00Z", "daemon", &gh, &logins);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(
            lines[0].starts_with(
                "pipeline proj: the runs of aaaa1111aaaa could not be read as third, the active gh login pat-login and octocat: "
            ) && lines[0].ends_with("; o/r is left alone until 2026-10-31T11:11:00Z"),
            "{lines:?}"
        );
        assert!(
            lines[1].starts_with("pipeline proj: told the owner ("),
            "{lines:?}"
        );
        let w = load_watch(&s, "proj");
        assert_eq!(w.unreadable_until, "2026-10-31T11:11:00Z");
        assert!(w.gh_user.is_empty());
        let unreadable = events(&s, "pipeline.unreadable");
        assert_eq!(unreadable.len(), 1);
        assert_eq!(
            unreadable[0]["tried"],
            json!(["third", "pat-login", "octocat"])
        );
    }

    #[test]
    fn a_project_with_no_push_pending_finds_the_account_through_the_branch_head() {
        let s = setup();
        let (gh, log) = gh_seen_by(
            &["octocat"],
            vec![run("CI", "push", "completed", "success", 1)],
        );
        let lines = tend_pipeline_as(&s, "proj", NOW, "daemon", &*gh, &logins);
        assert_eq!(
            lines,
            [
                "pipeline proj: reading o/r as octocat, since the active gh login pat-login cannot see it".to_string(),
                "pipeline proj: green at 8f377827f6c6 on origin/main (pushed by hive-bot) (CI)".to_string(),
            ]
        );
        assert_eq!(
            &*log.borrow(),
            &[
                "head main as active".to_string(),
                "head main as octocat".to_string(),
                format!("runs {SHA} as octocat"),
            ]
        );
        assert_eq!(load_watch(&s, "proj").gh_user, "octocat");

        // A branch that is not there is nobody's access problem: no other
        // account is tried and the repository is not left alone.
        let s = setup();
        let asked = RefCell::new(Vec::new());
        let gh = |_: &Ask, user: Option<&str>| -> Result<String, String> {
            asked.borrow_mut().push(user.map(str::to_string));
            Err("gh api: Branch not found (HTTP 404)".into())
        };
        let lines = tend_pipeline_as(&s, "proj", NOW, "daemon", &gh, &logins);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].ends_with(
                "the head of origin/main could not be read: gh api: Branch not found (HTTP 404)"
            ),
            "{lines:?}"
        );
        assert_eq!(*asked.borrow(), [None]);
        assert!(load_watch(&s, "proj").unreadable_until.is_empty());
    }

    /// The project checkout's own head: a commit the checkout holds.
    fn checkout_head(s: &hive_store::Store) -> String {
        let root = crate::board::project_by_slug(&s.conn().unwrap(), "proj".into()).str("path");
        crate::util::git(&root, &["rev-parse", "HEAD"])
            .stdout
            .trim()
            .to_string()
    }

    #[test]
    fn a_red_head_someone_else_pushed_is_hives_to_fix_once_the_checkout_holds_it() {
        let s = setup();
        let head = checkout_head(&s);
        let (gh, log) = gh_with(
            vec![
                run("Android Release", "push", "completed", "failure", 91),
                run("CodeQL", "dynamic", "completed", "success", 92),
            ],
            head_json(&head, "dependabot[bot]", "2026-10-31T09:20:00Z"),
        );
        let lines = tend_pipeline_with(&s, "proj", NOW, "daemon", &*gh);
        assert_eq!(lines.len(), 1, "{lines:?}");
        let js = jobs(&s);
        assert_eq!(js.len(), 1, "{js:?}");
        let short = crate::recipe::short(&head, 12);
        assert_eq!(
            lines[0],
            format!(
                "pipeline proj: red at {short} on origin/main (pushed by dependabot[bot]): Android Release failed; fix job {} ({})",
                js[0].str("id"),
                posts(&s)[0].str("id")
            )
        );
        let brief = js[0].str("brief");
        assert!(
            brief.starts_with(&format!(
                "dependabot[bot] pushed {short} to origin/main of o/r (https://github.com/o/r/commit/{head})"
            )),
            "{brief}"
        );
        assert!(
            brief.contains("The push was not hive's (dependabot[bot] made it), but hive looks after proj's default branch"),
            "{brief}"
        );
        assert!(brief.contains(&format!("`git log -1 {short}`")), "{brief}");
        let ps = posts(&s);
        assert_eq!(ps.len(), 1);
        assert!(
            ps[0].str("summary").starts_with(&format!(
                "pipeline of proj red at {short}: Android Release failed after the push to origin/main (pushed by dependabot[bot]); fix job"
            )),
            "{}",
            ps[0].str("summary")
        );
        let failed = events(&s, "pipeline.failed");
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0]["head"], true);
        assert_eq!(failed[0]["by"], "dependabot[bot]");
        assert_eq!(failed[0]["sha"], head);
        assert_eq!(
            &*log.borrow(),
            &[
                "head o/r main".to_string(),
                format!("runs o/r {head}"),
                "jobs o/r 91".to_string(),
                "log o/r 91".to_string()
            ]
        );
        // Judged: the next reads of the head change nothing, and the head is
        // read again only ten minutes later.
        assert!(tend_pipeline_with(&s, "proj", "2026-10-31T10:05:00Z", "daemon", &*gh).is_empty());
        assert_eq!(log.borrow().len(), 4);
        assert!(tend_pipeline_with(&s, "proj", "2026-10-31T10:11:00Z", "daemon", &*gh).is_empty());
        assert_eq!(log.borrow().len(), 5);
        assert_eq!(jobs(&s).len(), 1);
        assert_eq!(posts(&s).len(), 1);
        let w = load_watch(&s, "proj");
        assert_eq!(w.head_sha, head);
        assert_eq!(w.head_by, "dependabot[bot]");
        assert_eq!(w.head_read, "2026-10-31T10:11:00Z");
    }

    #[test]
    fn a_red_head_the_checkout_lacks_waits_two_hours_then_is_reported_without_a_job() {
        let s = setup();
        let (gh, log) = gh_with(
            vec![run("CI", "push", "completed", "failure", 93)],
            head_json(FROM, "manishtiwari25", "2026-10-31T09:20:00Z"),
        );
        let lines = tend_pipeline_with(&s, "proj", NOW, "daemon", &*gh);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].contains("is red at fc4ca8e79fbf on the forge (pushed by manishtiwari25); waiting for the checkout to hold it"),
            "{lines:?}"
        );
        assert!(jobs(&s).is_empty());
        assert!(posts(&s).is_empty());
        assert!(events(&s, "pipeline.failed").is_empty());
        // Looked at again in three minutes, not ten, while it waits.
        assert_eq!(
            tend_pipeline_with(&s, "proj", "2026-10-31T10:04:00Z", "daemon", &*gh).len(),
            1
        );
        assert_eq!(log.borrow().len(), 4);
        // Two hours on, still not in the checkout: the owner is told, with the
        // landing hold that keeps the checkout behind, and no job is planned.
        s.conn()
            .unwrap()
            .execute(
                "INSERT INTO meta (key, value) VALUES ('push-sync:proj', ?)",
                rusqlite::params![json!({"note": "origin/main has 24 commits main does not; merging them conflict", "state": "conflict"}).to_string()],
            )
            .unwrap();
        let lines = tend_pipeline_with(&s, "proj", "2026-10-31T12:01:00Z", "daemon", &*gh);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("CI failed ("), "{lines:?}");
        assert!(jobs(&s).is_empty());
        let ps = posts(&s);
        assert_eq!(ps.len(), 1);
        let body = ps[0].str("body");
        assert!(
            body.contains("No fix job is planned yet: a worker starts from the project checkout, and the checkout is held behind the forge: origin/main has 24 commits main does not; merging them conflict."),
            "{body}"
        );
        assert_eq!(events(&s, "pipeline.failed").len(), 1);
        assert!(load_watch(&s, "proj").head_waiting_since.is_empty());
    }

    #[test]
    fn a_red_run_whose_log_github_lost_is_rerun_once_and_judged_when_it_finishes() {
        let s = setup();
        pushed(&s, "2026-10-31T09:50:00Z", SHA);
        let (gh, log) = gh_answering(
            vec![run("CI", "push", "completed", "failure", 11)],
            head_json(SHA, "hive-bot", "2026-10-31T09:50:00Z"),
            Err(LOG_GONE.to_string()),
            Ok(String::new()),
        );
        let lines = tend_pipeline_with(&s, "proj", NOW, "daemon", &*gh);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].starts_with("pipeline proj: red at 8f377827f6c6 on origin/main (CI failed), but GitHub no longer has the failed log (gh run view --log-failed exited 1: failed to get run log: HTTP 410: Server Error); the failed jobs were rerun"),
            "{lines:?}"
        );
        assert_eq!(
            &*log.borrow(),
            &[
                format!("runs o/r {SHA}"),
                "jobs o/r 11".to_string(),
                "log o/r 11".to_string(),
                "rerun o/r 11".to_string(),
                "head o/r main".to_string()
            ]
        );
        // No verdict, no job, no finding: the rerun decides.
        assert!(jobs(&s).is_empty());
        assert!(posts(&s).is_empty());
        assert!(events(&s, "pipeline.failed").is_empty());
        let rerun = events(&s, "pipeline.rerun");
        assert_eq!(rerun.len(), 1);
        assert_eq!(rerun[0]["sha"], SHA);
        assert_eq!(rerun[0]["rerun"], json!([11]));
        assert_eq!(rerun[0]["reason"], LOG_GONE);
        assert_eq!(load_watch(&s, "proj").reruns.get(SHA).unwrap(), NOW);
        assert!(!judged_sha(&s.conn().unwrap(), "proj", SHA));
        assert_eq!(pending(&s.conn().unwrap(), "proj", NOW).len(), 1);

        // The rerun is going: wait, and do not rerun again.
        let (gh2, log2) = gh_answering(
            vec![run("CI", "push", "in_progress", "", 11)],
            head_json(SHA, "hive-bot", "2026-10-31T09:50:00Z"),
            Err(LOG_GONE.to_string()),
            Ok(String::new()),
        );
        let lines = tend_pipeline_with(&s, "proj", "2026-10-31T10:03:30Z", "daemon", &*gh2);
        assert!(lines.is_empty(), "{lines:?}");
        assert!(
            !log2.borrow().iter().any(|l| l.starts_with("rerun")),
            "{log2:?}"
        );

        // Red again, with a log this time: the fix job carries it and says
        // whose log it is. One rerun per commit.
        let (gh3, log3) = self::gh(vec![run("CI", "push", "completed", "failure", 11)]);
        let lines = tend_pipeline_with(&s, "proj", "2026-10-31T10:07:00Z", "daemon", &*gh3);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            !log3.borrow().iter().any(|l| l.starts_with("rerun")),
            "{log3:?}"
        );
        let js = jobs(&s);
        assert_eq!(js.len(), 1, "{js:?}");
        let brief = js[0].str("brief");
        assert!(brief.contains("expected 2, got 3"), "{brief}");
        assert!(
            brief.contains(&format!(
                "GitHub no longer had the log of the first attempt, so hive reran the failed jobs at {NOW}; the runs and lines above are the rerun's."
            )),
            "{brief}"
        );
        assert!(!brief.contains("could not be read"), "{brief}");
        let failed = events(&s, "pipeline.failed");
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0]["job"], js[0].str("id"));
        assert_eq!(events(&s, "pipeline.rerun").len(), 1);
        assert_eq!(posts(&s).len(), 1);
        // The push is judged: the rerun mark goes with it.
        assert!(load_watch(&s, "proj").reruns.is_empty());
    }

    #[test]
    fn a_rerun_that_passes_is_green_with_nothing_to_fix() {
        let s = setup();
        pushed(&s, "2026-10-31T09:50:00Z", SHA);
        let (gh, _) = gh_answering(
            vec![run("CI", "push", "completed", "failure", 11)],
            head_json(SHA, "hive-bot", "2026-10-31T09:50:00Z"),
            Err(LOG_GONE.to_string()),
            Ok(String::new()),
        );
        assert_eq!(tend_pipeline_with(&s, "proj", NOW, "daemon", &*gh).len(), 1);
        let (gh2, _) = self::gh(vec![run("CI", "push", "completed", "success", 11)]);
        let lines = tend_pipeline_with(&s, "proj", "2026-10-31T10:04:00Z", "daemon", &*gh2);
        assert_eq!(
            lines,
            vec![format!(
                "pipeline proj: green at 8f377827f6c6 on origin/main (CI) - the failed jobs passed when rerun at {NOW}; nothing to fix"
            )]
        );
        let passed = events(&s, "pipeline.passed");
        assert_eq!(passed.len(), 1);
        assert_eq!(passed[0]["rerun"], NOW);
        assert!(jobs(&s).is_empty());
        assert!(posts(&s).is_empty());
    }

    #[test]
    fn a_refused_rerun_or_a_readable_error_plans_the_fix_job_with_the_log_named() {
        // GitHub refuses the rerun: the job is planned now, the brief says
        // what the log answered and that the rerun was refused.
        let s = setup();
        pushed(&s, "2026-10-31T09:50:00Z", SHA);
        let (gh, log) = gh_answering(
            vec![run("CI", "push", "completed", "failure", 11)],
            head_json(SHA, "hive-bot", "2026-10-31T09:50:00Z"),
            Err(LOG_GONE.to_string()),
            Err("gh run rerun exited 1: HTTP 403: Resource not accessible by integration".into()),
        );
        let lines = tend_pipeline_with(&s, "proj", NOW, "daemon", &*gh);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            log.borrow().contains(&"rerun o/r 11".to_string()),
            "{log:?}"
        );
        let js = jobs(&s);
        assert_eq!(js.len(), 1, "{js:?}");
        let brief = js[0].str("brief");
        assert!(
            brief.contains("The failed jobs could not be rerun for a fresh log: gh run rerun exited 1: HTTP 403"),
            "{brief}"
        );
        assert!(
            brief.contains(&format!(
                "CI - the failed steps' log could not be read: {LOG_GONE}"
            )),
            "{brief}"
        );
        assert!(
            posts(&s)[0].str("body").contains("could not be read"),
            "{}",
            posts(&s)[0].str("body")
        );
        assert!(events(&s, "pipeline.rerun").is_empty());
        assert_eq!(events(&s, "pipeline.failed").len(), 1);
        assert!(load_watch(&s, "proj").reruns.is_empty());

        // A log that could not be read for another reason is not rerun for:
        // the error goes in the brief and the job is planned.
        let s = setup();
        pushed(&s, "2026-10-31T09:50:00Z", SHA);
        let (gh, log) = gh_answering(
            vec![run("CI", "push", "completed", "failure", 11)],
            head_json(SHA, "hive-bot", "2026-10-31T09:50:00Z"),
            Err("gh run view --log-failed did not answer within 60s".into()),
            Ok(String::new()),
        );
        tend_pipeline_with(&s, "proj", NOW, "daemon", &*gh);
        assert!(
            !log.borrow().iter().any(|l| l.starts_with("rerun")),
            "{log:?}"
        );
        let js = jobs(&s);
        assert_eq!(js.len(), 1, "{js:?}");
        assert!(
            js[0].str("brief").contains(
                "CI - the failed steps' log could not be read: gh run view --log-failed did not answer within 60s"
            ),
            "{}",
            js[0].str("brief")
        );

        // In watch mode nothing is rerun: the owner is told what the log answered.
        let s = setup();
        set_setting(&s, WATCH, "watch");
        pushed(&s, "2026-10-31T09:50:00Z", SHA);
        let (gh, log) = gh_answering(
            vec![run("CI", "push", "completed", "failure", 11)],
            head_json(SHA, "hive-bot", "2026-10-31T09:50:00Z"),
            Err(LOG_GONE.to_string()),
            Ok(String::new()),
        );
        tend_pipeline_with(&s, "proj", NOW, "daemon", &*gh);
        assert!(
            !log.borrow().iter().any(|l| l.starts_with("rerun")),
            "{log:?}"
        );
        assert!(jobs(&s).is_empty());
        assert!(
            posts(&s)[0].str("body").contains(&format!(
                "CI - the failed steps' log could not be read: {LOG_GONE}"
            )),
            "{}",
            posts(&s)[0].str("body")
        );
    }

    #[test]
    fn a_red_head_whose_log_is_gone_is_rerun_and_judged_on_the_next_poll() {
        let s = setup();
        let head = checkout_head(&s);
        let (gh, log) = gh_answering(
            vec![run("Android Release", "push", "completed", "failure", 91)],
            head_json(&head, "dependabot[bot]", "2026-10-31T09:20:00Z"),
            Err(LOG_GONE.to_string()),
            Ok(String::new()),
        );
        let lines = tend_pipeline_with(&s, "proj", NOW, "daemon", &*gh);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].contains("(pushed by dependabot[bot]) (Android Release failed), but GitHub no longer has the failed log"),
            "{lines:?}"
        );
        assert_eq!(log.borrow().last().unwrap(), "rerun o/r 91");
        let w = load_watch(&s, "proj");
        assert_eq!(w.reruns.get(&head).unwrap(), NOW);
        assert_eq!(
            w.head_next, "2026-10-31T10:03:00Z",
            "polled like a push, not in {HEAD_POLL_MINUTES} minutes"
        );
        assert!(jobs(&s).is_empty());
        assert!(events(&s, "pipeline.failed").is_empty());

        let (gh2, log2) = gh_with(
            vec![run("Android Release", "push", "completed", "failure", 91)],
            head_json(&head, "dependabot[bot]", "2026-10-31T09:20:00Z"),
        );
        let lines = tend_pipeline_with(&s, "proj", "2026-10-31T10:03:00Z", "daemon", &*gh2);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            !log2.borrow().iter().any(|l| l.starts_with("rerun")),
            "{log2:?}"
        );
        let js = jobs(&s);
        assert_eq!(js.len(), 1, "{js:?}");
        assert!(
            js[0].str("brief").contains("hive reran the failed jobs at"),
            "{}",
            js[0].str("brief")
        );
        assert_eq!(events(&s, "pipeline.failed").len(), 1);
        // Judged: the rerun mark goes with the verdict.
        assert!(load_watch(&s, "proj").reruns.is_empty());
    }

    #[test]
    fn a_green_head_is_recorded_once_and_hives_own_push_is_left_to_the_push_path() {
        let s = setup();
        let (gh, log) = gh_with(
            vec![run("CI", "push", "completed", "success", 94)],
            head_json(FROM, "", "2026-10-31T09:20:00Z"),
        );
        let lines = tend_pipeline_with(&s, "proj", NOW, "daemon", &*gh);
        assert_eq!(
            lines,
            [
                "pipeline proj: green at fc4ca8e79fbf on origin/main (pushed by Some One) (CI)"
                    .to_string()
            ]
        );
        let passed = events(&s, "pipeline.passed");
        assert_eq!(passed.len(), 1);
        assert_eq!(passed[0]["by"], "Some One");
        assert!(posts(&s).is_empty());
        // A head that is hive's own pending push is not judged here.
        pushed(&s, "2026-10-31T10:20:00Z", SHA);
        let (gh2, log2) = gh_with(
            vec![run("CI", "push", "in_progress", "", 95)],
            head_json(SHA, "hive-bot", "2026-10-31T10:20:00Z"),
        );
        assert!(tend_pipeline_with(&s, "proj", "2026-10-31T10:21:00Z", "daemon", &*gh2).is_empty());
        assert_eq!(
            &*log2.borrow(),
            &[format!("runs o/r {SHA}"), "head o/r main".to_string()]
        );
        assert_eq!(log.borrow().len(), 2);
    }

    #[test]
    fn a_head_is_read_as_github_reports_it() {
        let h = parse_head(&head_json(SHA, "octocat", "2026-10-31T09:20:00Z").to_string()).unwrap();
        assert_eq!(h.sha, SHA);
        assert_eq!(h.by, "octocat");
        assert_eq!(h.at, "2026-10-31T09:20:00Z");
        assert_eq!(h.url, format!("https://github.com/o/r/commit/{SHA}"));
        assert_eq!(
            h.subject,
            "Merge pull request #7 from o/dependabot/npm_and_yarn/x"
        );
        // No forge login: the commit's author name; none at all: someone.
        assert_eq!(
            parse_head(&head_json(SHA, "", "x").to_string()).unwrap().by,
            "Some One"
        );
        assert_eq!(
            parse_head(r#"{"commit":{"sha":"abc","commit":{}}}"#)
                .unwrap()
                .by,
            "someone"
        );
        assert!(parse_head(r#"{"message":"Branch not found"}"#).is_err());
        assert!(parse_head("nope").is_err());
    }

    #[test]
    fn judge_reads_runs_as_github_reports_them() {
        let ok = run("CI", "push", "completed", "success", 1);
        let runs = parse_runs(&Value::Array(vec![ok]).to_string()).unwrap();
        assert_eq!(runs[0].workflow, "CI");
        assert_eq!(runs[0].id, 1);
        assert_eq!(
            judge(&runs, "2026-10-31T09:00:00Z", NOW, 3.0),
            Verdict::Passed
        );
        assert!(parse_runs("not json").is_err());
        assert_eq!(
            failed_steps(
                r#"{"jobs":[{"name":"a","conclusion":"failure","steps":[{"name":"s1","conclusion":"failure"},{"name":"s2","conclusion":"skipped"}]},{"name":"b","conclusion":"success","steps":[]}]}"#
            ),
            ["a: s1"]
        );
        let long = (0..300)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let tail = log_tail(&long);
        assert!(tail.starts_with("line 180"), "{}", &tail[..20]);
        assert!(tail.ends_with("line 299"));
    }

    /// A gh on PATH that writes which token each read used (`app` for the
    /// App's `ghs_` token, the account for a `tok-ACCOUNT` one) and answers
    /// the runs of a commit with one green CI run and the branch head with
    /// `SHA`; as the App it answers 404 while `dir/app-denied` exists.
    fn token_logging_gh() -> (tempfile::TempDir, hive_testutil::SetenvGuard) {
        let bin = tempfile::tempdir().expect("fake gh dir");
        let d = bin.path();
        std::fs::write(
            d.join("runs.json"),
            Value::Array(vec![run("CI", "push", "completed", "success", 1)]).to_string(),
        )
        .unwrap();
        std::fs::write(
            d.join("head.json"),
            head_json(SHA, "hive-bot", "2026-10-31T09:50:00Z").to_string(),
        )
        .unwrap();
        let script = format!(
            r#"#!/bin/sh
d='{d}'
case "$1 $2" in
  "auth token") echo "auth token $4" >> "$d/calls"; echo "tok-$4"; exit 0 ;;
  "auth status") echo "auth status" >> "$d/calls"; exit 1 ;;
esac
case "$GH_TOKEN" in
  ghs_*) who=app ;;
  tok-*) who="${{GH_TOKEN#tok-}}" ;;
  *) who=active ;;
esac
echo "$1 $2 as $who" >> "$d/calls"
if [ "$who" = app ] && [ -e "$d/app-denied" ]; then
  echo "GraphQL: Could not resolve to a Repository with the name 'o/r'. (repository)" >&2
  exit 1
fi
case "$1" in
  run) cat "$d/runs.json" ;;
  api) cat "$d/head.json" ;;
esac
"#,
            d = d.display()
        );
        let path = d.join("gh");
        std::fs::write(&path, script).expect("fake gh script");
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod gh");
        }
        let guard = hive_testutil::setenv(
            "PATH",
            format!(
                "{}:{}",
                d.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        );
        (bin, guard)
    }

    fn gh_calls(bin: &tempfile::TempDir) -> Vec<String> {
        std::fs::read_to_string(bin.path().join("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// The fake App API, the endpoint pointed at it and the App configured
    /// on `s`; None without openssl.
    fn app_on(
        s: &hive_store::Store,
        keys: &std::path::Path,
    ) -> Option<(
        hive_testutil::httptest::Server,
        crate::gitapp::tests::Seen,
        crate::gitapp::tests::EndpointGuard,
    )> {
        let (key, _) = crate::gitapp::tests::app_key(keys)?;
        crate::gitapp::tests::configure(s, &key);
        let clock: std::sync::Arc<dyn Fn() -> i64 + Send + Sync> = std::sync::Arc::new(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64
        });
        let (api, seen) = crate::gitapp::tests::fake_github(clock);
        let ep = crate::gitapp::tests::point_at(&api.url(), "https://github.com");
        Some((api, seen, ep))
    }

    fn remember_login(s: &hive_store::Store, user: &str) {
        let w = Watch {
            gh_user: user.to_string(),
            ..Watch::default()
        };
        save_watch(&s.conn().unwrap(), "proj", &w);
    }

    #[test]
    fn with_a_github_app_ci_is_read_with_its_token_and_no_remembered_login_is_tried() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = setup();
        let keys = tempfile::tempdir().unwrap();
        let Some((_api, seen, _ep)) = app_on(&s, keys.path()) else {
            return;
        };
        let (bin, _path) = token_logging_gh();
        pushed(&s, "2026-10-31T09:50:00Z", SHA);
        remember_login(&s, "octocat");
        let lines = tend_pipeline(&s, "proj", NOW, "daemon");
        assert_eq!(
            lines,
            ["pipeline proj: green at 8f377827f6c6 on origin/main (CI)".to_string()]
        );
        // Both reads ran with the App's GH_TOKEN; the remembered login's
        // token was never asked for, nor the machine's logins listed.
        assert_eq!(
            gh_calls(&bin),
            ["run list as app", "api repos/o/r/branches/main as app"]
        );
        // One installation token, scoped to the one repository, minted once
        // and reused for the second read.
        let mints: Vec<_> = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, uri, _, _)| m == "POST" && uri.ends_with("/access_tokens"))
            .map(|(_, _, _, body)| body.clone())
            .collect();
        assert_eq!(mints.len(), 1, "{mints:?}");
        assert!(mints[0].contains("\"r\""), "{mints:?}");
        assert!(events(&s, "gitapp.fallback").is_empty());
        // The token never reaches the log lines or the watch.
        let watch = serde_json::to_string(&load_watch(&s, "proj")).unwrap();
        assert!(!watch.contains("ghs_"), "{watch}");
        assert!(!lines.concat().contains("ghs_"));
    }

    #[test]
    fn a_read_the_github_app_cannot_make_falls_back_to_the_remembered_login() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let s = setup();
        let keys = tempfile::tempdir().unwrap();
        let Some((_api, _seen, _ep)) = app_on(&s, keys.path()) else {
            return;
        };
        let (bin, _path) = token_logging_gh();
        std::fs::write(bin.path().join("app-denied"), "").unwrap();
        pushed(&s, "2026-10-31T09:50:00Z", SHA);
        remember_login(&s, "octocat");
        let lines = tend_pipeline(&s, "proj", NOW, "daemon");
        assert_eq!(
            lines,
            ["pipeline proj: green at 8f377827f6c6 on origin/main (CI)".to_string()]
        );
        // The App is asked once; once it cannot see the repository the pass
        // reads as the remembered login, which stays remembered.
        let calls: Vec<String> = gh_calls(&bin)
            .into_iter()
            .filter(|c| !c.starts_with("auth "))
            .collect();
        assert_eq!(
            calls,
            [
                "run list as app",
                "run list as octocat",
                "api repos/o/r/branches/main as octocat"
            ]
        );
        assert_eq!(load_watch(&s, "proj").gh_user, "octocat");

        // A key that cannot be read is a `gitapp.fallback` with the reason,
        // and the reads go to the remembered login without asking gh as the
        // App at all.
        std::fs::remove_file(bin.path().join("calls")).unwrap();
        std::fs::remove_file(bin.path().join("app-denied")).unwrap();
        crate::gitapp::clear_cache();
        let key = keys.path().join("app.pem");
        crate::test_helpers::run(".", &["chmod", "644", key.to_str().unwrap()]);
        let sha2 = "aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111";
        pushed(&s, "2026-10-31T10:20:00Z", sha2);
        let _ = tend_pipeline(&s, "proj", "2026-10-31T10:30:00Z", "daemon");
        let calls: Vec<String> = gh_calls(&bin)
            .into_iter()
            .filter(|c| !c.starts_with("auth "))
            .collect();
        assert!(!calls.is_empty());
        assert!(calls.iter().all(|c| c.ends_with("as octocat")), "{calls:?}");
        let fallback = events(&s, "gitapp.fallback");
        assert_eq!(fallback.len(), 1, "{fallback:?}");
        assert_eq!(fallback[0]["repo"], "o/r");
        let why = fallback[0]["why"].as_str().unwrap_or_default();
        assert!(why.contains(crate::gitapp::PRIVATE_KEY_REF), "{why}");
        assert!(!why.contains("BEGIN"), "{why}");
    }
}
