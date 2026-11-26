#![allow(dead_code, reason = "shared test helpers; not every one has a caller")]

use crate::test_helpers::*;
use crate::util::{self, Row};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(crate) const FAKE_PUSH_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nZmFrZSBrZXkgZm9yIHRlc3Rz\n-----END OPENSSH PRIVATE KEY-----\n";

pub(crate) fn set_setting(s: &hive_store::Store, key: &str, value: &str) {
    if let Err(err) = s.set_setting(util::HUMAN_AGENT, key, value) {
        panic!("setting {key}: {err}");
    }
}

/// A no-op: the worker/daemon tests that call it always run; the suite has no short mode.
pub(crate) fn skip_slow() {}

pub(crate) const DEFAULT_TEST_DEADLINE: Duration = Duration::from_secs(60);

/// Fails the test with a stack note when it runs past d, so a worker clone whose spawned
/// git/worker subprocess blocks reports the stuck test by name instead of letting the whole
/// test binary hit the harness timeout. Once the deadline passes the guard's thread exits the
/// test process (exit 2). Hold the returned guard for the test's life: dropping it closes
/// `done`, which cancels the deadline.
#[must_use = "bind the guard (`let _deadline = test_deadline(..)`) or the deadline is cancelled at once"]
pub(crate) fn test_deadline(d: Duration) -> TestDeadline {
    let d = test_wait(d);
    let name = std::thread::current().name().unwrap_or("test").to_string();
    let (done, wait) = crossbeam_channel::bounded::<()>(0);
    std::thread::spawn(move || {
        if wait.recv_timeout(d) == Err(crossbeam_channel::RecvTimeoutError::Timeout) {
            eprintln!("{name} ran past its {d:?} deadline");
            eprintln!("test {name} exceeded {d:?} deadline");
            std::process::exit(2);
        }
    });
    TestDeadline { _done: done }
}

pub(crate) struct TestDeadline {
    _done: crossbeam_channel::Sender<()>,
}

pub(crate) fn test_wait(d: Duration) -> Duration {
    d.mul_f64(deadline_mult())
}

/// HIVE_TEST_DEADLINE_MULT (a positive float, e.g. "2.5"); unset or invalid means 1. Values
/// below 1 are clamped to 1 so a budget can only grow.
pub(crate) fn deadline_mult() -> f64 {
    if let Ok(v) = std::env::var("HIVE_TEST_DEADLINE_MULT")
        && !v.is_empty()
        && let Ok(f) = v.parse::<f64>()
        && f > 0.0
    {
        if f < 1.0 {
            return 1.0;
        }
        return f;
    }
    1.0
}

pub(crate) fn exec1(s: &hive_store::Store, sql: &str, args: &[&dyn rusqlite::ToSql]) {
    s.conn().expect("conn").execute(sql, args).expect("exec1");
}

pub(crate) fn must_rows(
    s: &hive_store::Store,
    sql: &str,
    args: &[&dyn rusqlite::ToSql],
) -> Vec<Row> {
    util::query_rows(&s.conn().expect("conn"), sql, args).expect("rows")
}

pub(crate) fn must_one(s: &hive_store::Store, sql: &str, args: &[&dyn rusqlite::ToSql]) -> Row {
    util::query_one(&s.conn().expect("conn"), sql, args)
        .expect("row")
        .unwrap_or_default()
}

pub(crate) fn job_row(s: &hive_store::Store, id: &str) -> Row {
    crate::jobs::job_row(&s.conn().expect("conn"), id.to_string())
}

/// The fake measure tests install in place of `hive_evolve`'s: one public and one held-out score,
/// or an error; the real archive still writes the row.
#[derive(Default)]
pub(crate) struct FakeMeasureState {
    pub total: f64,
    pub err: Option<anyhow::Error>,
    pub calls: Vec<String>,
}

#[derive(Clone)]
pub(crate) struct FakeMeasure(pub Arc<Mutex<FakeMeasureState>>);

pub(crate) struct MeasureGuard(Option<Box<hive_evolve::api::MeasureFn>>);

impl Drop for MeasureGuard {
    fn drop(&mut self) {
        if let Some(old) = self.0.take() {
            let _ = hive_evolve::set_measure(old);
        }
    }
}

impl FakeMeasure {
    pub(crate) fn new(total: f64, err: Option<anyhow::Error>) -> Self {
        FakeMeasure(Arc::new(Mutex::new(FakeMeasureState {
            total,
            err,
            calls: Vec::new(),
        })))
    }

    pub(crate) fn install(&self) -> MeasureGuard {
        let f = self.clone();
        let old =
            hive_evolve::set_measure(Box::new(move |_ctx, _s, project, repo, ref_name, _opts| {
                let mut f = f.0.lock().expect("fake measure");
                f.calls.push(format!("{project}@{ref_name}"));
                if let Some(err) = &f.err {
                    return Err(anyhow::anyhow!("{err:#}"));
                }
                Ok(hive_evolve::Scores {
                    project: project.into(),
                    repo: repo.into(),
                    ref_name: ref_name.into(),
                    total: f.total,
                    heldout: 1,
                    sandbox: "fake-sandbox".into(),
                    fixtures: vec![
                        hive_evolve::FixtureScore {
                            name: "starter-readme".into(),
                            kind: "public".into(),
                            score: f.total,
                            weight: 1.0,
                            ..Default::default()
                        },
                        hive_evolve::FixtureScore {
                            name: "PRIVATE-NAME".into(),
                            kind: "heldout".into(),
                            score: 1.0,
                            weight: 1.0,
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                })
            }));
        MeasureGuard(Some(old))
    }

    pub(crate) fn calls(&self) -> Vec<String> {
        self.0.lock().expect("fake measure").calls.clone()
    }

    pub(crate) fn set_err(&self, err: anyhow::Error) {
        self.0.lock().expect("fake measure").err = Some(err);
    }
}

pub(crate) fn baseline_jobs(s: &hive_store::Store, slug: &str) -> Vec<Row> {
    must_rows(
        s,
        "SELECT * FROM jobs WHERE project = ? AND kind = 'baseline' ORDER BY created_at, rowid",
        &[&slug],
    )
}

pub(crate) fn loop_records(s: &hive_store::Store, slug: &str) {
    for (name, body) in [
        ("constitution", "# Constitution\n\nloop: on\n"),
        ("evaluators", "Product scope: allowed\n"),
    ] {
        s.put_record(
            hive_store::Record {
                id: format!("EVO-{name}-{slug}"),
                project: slug.to_string(),
                kind: "evolution".to_string(),
                slug: name.to_string(),
                ..Default::default()
            },
            BTreeMap::new(),
            body,
            util::HUMAN_AGENT,
            "fixture",
        )
        .expect("put loop record");
    }
}

pub(crate) fn only_baseline(s: &hive_store::Store) {
    set_setting(s, "jobs_per_day", "none");
    let _ = crate::jobs::add_job(
        s,
        "proj".into(),
        "reflect".into(),
        "Daily reflect".into(),
        "done elsewhere".into(),
        0,
        1,
        format!("reflect:proj:{}", &TEST_NOW[..10]),
        "daemon".into(),
        true,
    )
    .expect("add reflect");
    exec1(
        s,
        "UPDATE jobs SET status = 'done', finished_at = ? WHERE kind = 'reflect'",
        &[&TEST_NOW],
    );
}

pub(crate) fn push_fixture(s: &hive_store::Store) -> (Row, String, String) {
    let (proj, repo) = new_bare_repo(s);
    let bare = Path::new(&s.home)
        .join("tmp")
        .join(format!("origin-{}.git", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(bare.parent().unwrap()).expect("bare parent");
    let bare = bare.display().to_string();
    run(".", &["git", "init", "--bare", "-q", "-b", "main", &bare]);
    run(&repo, &["git", "remote", "add", "origin", &bare]);
    run(&repo, &["git", "push", "-q", "-u", "origin", "main"]);
    (proj, repo, bare)
}

pub(crate) fn push_branch(s: &hive_store::Store, proj: &Row, repo: &str, name: &str) -> Row {
    join(s, "worker");
    run(repo, &["git", "checkout", "-q", "-b", name, "main"]);
    std::fs::write(
        Path::new(repo).join(format!("{}.txt", name.replace('/', "_"))),
        format!("{name}\n"),
    )
    .expect("write branch file");
    run(repo, &["git", "add", "-A"]);
    run(
        repo,
        &["git", "commit", "-q", "-m", &format!("work on {name}")],
    );
    run(repo, &["git", "checkout", "-q", "main"]);
    let (j, _) = hive_board::add_job(
        s,
        hive_board::JobInput {
            project: proj.str("slug"),
            kind: "task".into(),
            title: format!("t {name}"),
            brief: "b".into(),
            tier: 1,
            created_by: "worker".into(),
            ..Default::default()
        },
    )
    .expect("add job");
    exec1(
        s,
        "UPDATE jobs SET status = 'done', finished_at = ?, branch = ? WHERE id = ?",
        &[&TEST_NOW, &name, &j.id],
    );
    job_row(s, &j.id)
}

pub(crate) fn ref_of(dir: &str, r#ref: &str) -> String {
    run(dir, &["git", "rev-parse", r#ref]).trim().to_string()
}

pub(crate) fn bare_ref(bare: &str, r#ref: &str) -> String {
    run(".", &["git", "--git-dir", bare, "rev-parse", r#ref])
        .trim()
        .to_string()
}

pub(crate) fn key_file(mode: u32) -> String {
    let path = tempfile::tempdir()
        .expect("key temp")
        .keep()
        .join("push_key");
    std::fs::write(&path, FAKE_PUSH_KEY).expect("write key");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).expect("chmod key");
    }
    #[cfg(not(unix))]
    let _ = mode;
    path.display().to_string()
}

/// An `ssh` on PATH that runs the command git hands it on this machine -
/// in `HIVE_TEST_SSH_ROOT` when a test sets it, the directory its
/// `git@host:PATH` urls are served from - after writing its arguments to
/// the record file returned, and the mode and the first bytes of the key it
/// was given beside it. With `HIVE_TEST_SSH_REFUSE` set it says that
/// instead and exits 255, as ssh does when the host turns the key away.
pub(crate) fn fake_ssh() -> (String, Vec<hive_testutil::SetenvGuard>) {
    let dir = tempfile::tempdir().expect("ssh temp").keep();
    let record = dir.join("record");
    let script = r#"#!/bin/sh
printf '%s\n' "$@" > "$HIVE_TEST_SSH_RECORD"
if [ -n "$HIVE_TEST_SSH_REFUSE" ]; then echo "$HIVE_TEST_SSH_REFUSE" >&2; exit 255; fi
key=""
prev=""
for a in "$@"; do
  if [ "$prev" = "-i" ]; then key="$a"; fi
  prev="$a"
done
if [ -n "$key" ]; then
  if [ ! -f "$key" ]; then echo "fake ssh: key $key missing" >&2; exit 255; fi
  mode=$(stat -f '%Lp' "$key" 2>/dev/null || stat -c '%a' "$key")
  echo "$mode" > "$HIVE_TEST_SSH_RECORD.mode"
  head -c 30 "$key" > "$HIVE_TEST_SSH_RECORD.head"
fi
for a in "$@"; do last="$a"; done
[ -n "$HIVE_TEST_SSH_ROOT" ] && cd "$HIVE_TEST_SSH_ROOT"
exec sh -c "$last"
"#;
    let ssh = dir.join("ssh");
    std::fs::write(&ssh, script).expect("write fake ssh");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).expect("chmod ssh");
    }
    let mut paths = vec![dir];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let env = vec![
        hive_testutil::setenv("PATH", std::env::join_paths(paths).expect("join path")),
        hive_testutil::setenv("HIVE_TEST_SSH_RECORD", &record),
    ];
    (record.display().to_string(), env)
}

pub(crate) fn evolve_plan_ready(s: &hive_store::Store) {
    set_setting(s, "evolve.loop", "1");
    set_setting(s, "quorum", "2");
    // The fake experiments' cards are fixture-measured and name no check:
    // the card-proof gate (evolve.require_check, default on) is off here, and
    // the tests of that gate turn it on themselves.
    set_setting(s, hive_store::EVOLVE_REQUIRE_CHECK, "off");
    let constitution = "# Constitution\n\n## Editable scope\n| File class | Paths | Stage | Autonomy |\n| --- | --- | --- | --- |\n| operating | `README.md` | 2 | standing |\n\n## Immutable paths\n| Path | Why |\n| --- | --- |\n| `docs/evolution/**` | owner-owned |\n";
    for (slug, body) in [
        ("constitution", constitution),
        ("evaluators", "Product scope: off\n"),
    ] {
        s.put_record(
            hive_store::Record {
                id: format!("EVO-{slug}-proj"),
                project: "proj".into(),
                kind: "evolution".into(),
                slug: slug.into(),
                ..Default::default()
            },
            BTreeMap::new(),
            body,
            util::HUMAN_AGENT,
            "fixture",
        )
        .expect("put evo record");
    }
    let dir = Path::new(&s.home).join("private").join("heldout-plan");
    std::fs::create_dir_all(&dir).expect("heldout dir");
    hive_evolve::setup(s, &dir.display().to_string()).expect("heldout setup");
    set_setting(s, "evolve.heldout_dir", &dir.display().to_string());
    for (slot, judge) in [
        (1, "starter-hardstop-history"),
        (2, "starter-hardstop-publish"),
    ] {
        hive_evolve::write_heldout(
            s,
            hive_evolve::HeldoutWrite {
                request: "Please make the requested repository change without exposing private fixture text.".into(),
                judge: judge.into(),
                slot,
                ..Default::default()
            },
        )
        .expect("heldout write");
    }
    let scores = hive_evolve::Scores {
        project: "proj".into(),
        total: 0.8,
        fixtures: vec![hive_evolve::FixtureScore {
            name: "starter-readme".into(),
            kind: "public".into(),
            score: 0.8,
            weight: 1.0,
            ..Default::default()
        }],
        ..Default::default()
    };
    hive_evolve::archive_row(
        s,
        hive_evolve::ArchiveRow {
            project: "proj".into(),
            hypothesis: hive_evolve::BASELINE_HYPOTHESIS.into(),
            branch: "main".into(),
            baseline: Some(scores.clone()),
            variant: Some(scores),
            tolerance: 0.02,
            kept: true,
            run_id: "run-baseline".into(),
            note: "baseline measurement".into(),
            ..Default::default()
        },
    )
    .expect("baseline archive");
    s.close().expect("close helper connections");
}

pub(crate) fn finish_measured_experiment(s: &hive_store::Store, job: Row, kept: bool) -> String {
    let run_id = format!("run-{}", job.str("id"));
    exec1(
        s,
        "INSERT INTO runs (id, job_id, agent, project, started_at, status) VALUES (?, ?, 'proposer', 'proj', ?, 'done')",
        &[&run_id, &job.str("id"), &TEST_NOW],
    );
    let mut variant = 0.7;
    if kept {
        variant = 0.9;
    }
    let (suggestion, hypothesis) = crate::experiment::experiment_suggestion(job.clone());
    let id = hive_evolve::archive_row(
        s,
        hive_evolve::ArchiveRow {
            project: "proj".into(),
            suggestion_post: suggestion,
            hypothesis,
            branch: job.str("branch"),
            baseline: Some(hive_evolve::Scores {
                total: 0.8,
                ..Default::default()
            }),
            variant: Some(hive_evolve::Scores {
                total: variant,
                ..Default::default()
            }),
            tolerance: 0.02,
            kept,
            run_id: run_id.clone(),
            note: "fake measured outcome".into(),
            ..Default::default()
        },
    )
    .unwrap_or_else(|err| panic!("{err}"));
    exec1(
        s,
        "UPDATE jobs SET status = 'done', finished_at = ?, result = 'measured; finalization pending' WHERE id = ?",
        &[&TEST_NOW, &job.str("id")],
    );
    let mut action = "land.dropped";
    if kept {
        action = "land.landed";
    }
    let project = must_one(s, "SELECT * FROM projects WHERE slug = 'proj'", &[]);
    if let Err(err) = crate::landing_evidence::record_landing(
        s,
        project,
        job.str("branch"),
        "daemon".into(),
        action.into(),
        " | fake finalization".into(),
        crate::land::LandOpts {
            job: job.clone(),
            archive_id: id.clone(),
            run_id,
            ..Default::default()
        },
        BTreeMap::from([("fake".to_string(), serde_json::json!(true))]),
    ) {
        panic!("{err}");
    }
    id
}

pub(crate) fn evolve_suggestion(s: &hive_store::Store) -> hive_board::Post {
    for name in ["proposer", "voter-one", "voter-two"] {
        join(s, name);
    }
    let agent_row = |name: &str| hive_store::Agent {
        name: name.into(),
        runner: "r".into(),
        model: "m".into(),
        kind: "worker".into(),
        ..Default::default()
    };
    let post = hive_board::create_post(
        s,
        hive_board::PostInput {
            kind: "suggestion".into(),
            project: "proj".into(),
            area: "evolution".into(),
            thread: "daemon-plan".into(),
            agent: "proposer".into(),
            summary: "Tune README".into(),
            files: "README.md".into(),
            // The proposer is a worker, so the planner gates want a Signal: line naming
            // existing evidence, as they do for the runner's own planner.
            body: "Class: operating-layer\nFile: README.md\nVariable: one constant\nHypothesis: README gets clearer\nTier: 1\nSignal: README.md\n".into(),
            agent_row: Some(agent_row("proposer")),
            ..Default::default()
        },
    )
    .expect("suggestion");
    for agent in ["voter-one", "voter-two"] {
        hive_board::create_post(
            s,
            hive_board::PostInput {
                kind: "vote".into(),
                project: "proj".into(),
                area: "evolution".into(),
                thread: "daemon-plan".into(),
                agent: agent.into(),
                summary: "measured".into(),
                on: post.id.clone(),
                vote: "up".into(),
                agent_row: Some(agent_row(agent)),
                ..Default::default()
            },
        )
        .expect("vote");
    }
    post
}

pub(crate) type MeasureCalls = Arc<Mutex<i32>>;

pub(crate) fn measured_keep_fixture() -> (
    crate::test_helpers::TestStore,
    Row,
    Row,
    String,
    MeasureCalls,
) {
    let s = new_store();
    let (mut project, root) = new_repo(&s);
    evolve_plan_ready(&s);
    evolve_suggestion(&s);
    let planned = hive_evolve::plan_experiment(
        &s,
        "proj",
        hive_evolve::PlanOptions {
            agent: "voter-one".into(),
            agent_row: Some(hive_store::Agent {
                name: "voter-one".into(),
                runner: "r".into(),
                model: "m".into(),
                kind: "worker".into(),
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .expect("plan experiment");
    assert!(planned.created, "planned fake experiment: {planned:?}");
    let job = job_row(&s, &planned.job_id);
    run(&root, &["git", "switch", "-q", "-c", &job.str("branch")]);
    std::fs::write(Path::new(&root).join("README.md"), "# measured variant\n").expect("variant");
    run(
        &root,
        &["git", "commit", "-q", "-am", "fake experiment variant"],
    );
    run(&root, &["git", "switch", "-q", "main"]);
    exec1(
        &s,
        "INSERT INTO runs (id, job_id, project, agent, started_at, status) VALUES (?, ?, 'proj', 'proposer', ?, 'done')",
        &[&format!("run-{}", job.str("id")), &job.str("id"), &TEST_NOW],
    );
    exec1(
        &s,
        "UPDATE jobs SET status = 'done', finished_at = ? WHERE id = ?",
        &[&TEST_NOW, &job.str("id")],
    );
    project.insert("verify_cmd", serde_json::json!("printf verified"));
    exec1(
        &s,
        "UPDATE projects SET verify_cmd = ? WHERE slug = 'proj'",
        &[&project.str("verify_cmd")],
    );
    // The verification is wrapped with the `daemon.min_free_mb` disk prelude (default
    // 10240 MB); these tests measure landing, not the host's free space, so turn the
    // probe off or they fail spuriously on a nearly full machine.
    set_setting(&s, "daemon.min_free_mb", "0");
    let calls = Arc::new(Mutex::new(0));
    let calls2 = calls.clone();
    let _old = hive_evolve::set_measure(Box::new(move |_ctx, _s, slug, _repo, r#ref, _opts| {
        *calls2.lock().unwrap() += 1;
        let score = if r#ref == "main" { 0.8 } else { 0.9 };
        Ok(hive_evolve::Scores {
            project: slug.into(),
            total: score,
            ..Default::default()
        })
    }));
    (
        s.clone(),
        project,
        job_row(&s, &planned.job_id),
        root,
        calls,
    )
}

pub(crate) fn defer_measured_keep(s: &hive_store::Store, project: Row, job: Row, root: &str) {
    set_setting(s, "landing.auto_stash", "off");
    std::fs::write(
        Path::new(root).join("README.md"),
        "# unrelated local edit\n",
    )
    .expect("local edit");
    let (state, note) =
        crate::experiment::land_experiment(s, project, job, "daemon".into(), TEST_NOW.into(), true);
    assert!(
        state == "deferred" && note.contains("archive B-002"),
        "fake measured keep did not defer landing: {state}: {note}"
    );
}

// The run history and queue the restart window's measures are tested
// against (restart.rs): finished runs `run_fits` measures, queued jobs and
// dispatches of the owner's and the fleet's work, shared with the daemon
// loop's own tests.

pub(crate) fn finished_run(
    s: &hive_store::Store,
    n: usize,
    age_minutes: f64,
    status: &str,
    mode: &str,
    seconds: f64,
) {
    let finished = crate::util::plus_minutes(TEST_NOW, -age_minutes);
    crate::util::exec1(
        &*s.conn().unwrap(),
        "INSERT INTO runs (id, job_id, agent, project, started_at, finished_at, status, seconds, mode) VALUES (?, ?, 'aurora', 'proj', ?, ?, ?, ?, ?)",
        &[
            &format!("run-{n}"),
            &format!("job-{n}"),
            &finished,
            &finished,
            &status,
            &seconds,
            &mode,
        ],
    );
}

/// The finished job `job-{n}` of `kind` on `proj`, so `run_fits` can
/// tell the kinds of the runs `finished_run` seeds apart.
pub(crate) fn finished_job_of_kind(s: &hive_store::Store, n: usize, kind: &str) {
    finished_job_on(s, n, "proj", kind);
}

/// The finished job `job-{n}` of `kind` on `project`.
pub(crate) fn finished_job_on(s: &hive_store::Store, n: usize, project: &str, kind: &str) {
    crate::util::exec1(
        &*s.conn().unwrap(),
        "INSERT INTO jobs (id, project, kind, title, brief, status, created_at) VALUES (?, ?, ?, 'a job', '', 'done', ?)",
        &[&format!("job-{n}"), &project, &kind, &TEST_NOW],
    );
}

/// A finished real run of a job of `kind` on `proj`.
pub(crate) fn finished_run_of_kind(s: &hive_store::Store, n: usize, kind: &str, seconds: f64) {
    finished_job_of_kind(s, n, kind);
    finished_run(s, n, 10.0 + n as f64, "done", "real", seconds);
}

/// The pairs of `project` and each kind in `kinds`.
pub(crate) fn pairs(project: &str, kinds: &[&str]) -> Vec<(String, String)> {
    kinds
        .iter()
        .map(|kind| (project.to_string(), kind.to_string()))
        .collect()
}

/// A store with `projects` registered and active, so `kind_fits` measures them.
pub(crate) fn store_with_projects(projects: &[&str]) -> crate::test_helpers::TestStore {
    let s = new_store();
    let c = s.conn().unwrap();
    for p in projects {
        crate::util::exec1(
            &*c,
            "INSERT OR IGNORE INTO projects (slug, name, path, added_at, added_by) VALUES (?, ?, ?, '2026-01-01T00:00:00Z', 'owner')",
            &[p, p, &format!("/tmp/{p}")],
        );
    }
    s
}

/// A queued job of `kind` on `project` from `source`.
pub(crate) fn queued_job(s: &hive_store::Store, id: &str, project: &str, kind: &str, source: &str) {
    crate::util::exec1(
        &*s.conn().unwrap(),
        "INSERT INTO jobs (id, project, kind, title, brief, status, source, created_at) VALUES (?, ?, ?, 'queued', '', 'queued', ?, ?)",
        &[&id, &project, &kind, &source, &TEST_NOW],
    );
}

/// A dispatch of a job of `kind` on `project`, from `source`.
pub(crate) fn dispatch_of(
    slot: &str,
    project: &str,
    kind: &str,
    source: &str,
) -> crate::daemon::Dispatch {
    let mut job = Row::new();
    job.0
        .insert("kind".into(), serde_json::Value::String(kind.into()));
    job.0
        .insert("project".into(), serde_json::Value::String(project.into()));
    job.0
        .insert("source".into(), serde_json::Value::String(source.into()));
    crate::daemon::Dispatch {
        slot: slot.into(),
        worker: format!("w-{slot}"),
        job,
        started_at: TEST_NOW.into(),
        ..Default::default()
    }
}

/// A `gh` on PATH that answers like GitHub for the delivery pass: `pr
/// create` numbers pull requests from 7 and remembers each one's head and
/// base, `pr edit N --base` moves the base, `pr list --head` finds the
/// open one for a head, `pr list --state all` and `pr view N` print the
/// states the test wrote (`set_state`), with each head's tip read from the
/// bare remote named by `track_remote` and the forge's judgement of it as
/// the test set it (`set_mergeable`, `set_checks`), and every call is
/// logged. Drop it to take it off PATH.
pub(crate) struct FakeGh {
    dir: std::path::PathBuf,
    _bin: tempfile::TempDir,
    _path: hive_testutil::SetenvGuard,
}

impl FakeGh {
    /// The pull-request calls made so far, as their first three words
    /// (`pr list --head`, `pr create --head`, `pr list --state`, `pr view
    /// 7`).
    pub(crate) fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.join("gh.log"))
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with("pr "))
            .map(|l| l.split_whitespace().take(3).collect::<Vec<_>>().join(" "))
            .collect()
    }

    /// Makes every read of pull requests (`pr list --state`, `pr view`)
    /// fail with `why` on stderr until [`FakeGh::heal`]: the forge or gh
    /// being down.
    pub(crate) fn fail_reads(&self, why: &str) {
        std::fs::write(self.dir.join("fail"), why).expect("fake gh failure");
    }

    /// Makes `pr create` fail with `why` on stderr and open nothing, until
    /// [`FakeGh::heal`]: gh or the forge refusing the pull request.
    pub(crate) fn fail_creates(&self, why: &str) {
        std::fs::write(self.dir.join("fail-create"), why).expect("fake gh failure");
    }

    /// Calls answer again after [`FakeGh::fail_reads`] and
    /// [`FakeGh::fail_creates`].
    pub(crate) fn heal(&self) {
        let _ = std::fs::remove_file(self.dir.join("fail"));
        let _ = std::fs::remove_file(self.dir.join("fail-create"));
    }

    /// Sets pull request `n`'s state: `OPEN`, `MERGED` or `CLOSED`.
    pub(crate) fn set_state(&self, n: i64, state: &str) {
        std::fs::write(self.dir.join("state").join(n.to_string()), state).expect("pr state");
    }

    /// Names the bare repository the pull requests' heads live in: each
    /// head's `headRefOid` is read from it. Without this the fake answers
    /// no sha, as an older gh would.
    pub(crate) fn track_remote(&self, bare: &str) {
        std::fs::write(self.dir.join("bare"), bare).expect("fake gh remote");
    }

    /// Pins pull request `n`'s `headRefOid` to `sha` whatever the remote
    /// holds (the forge's listing lagging a push); an empty `sha` unpins
    /// it.
    pub(crate) fn pin_head_sha(&self, n: i64, sha: &str) {
        let path = self.dir.join("oid").join(n.to_string());
        if sha.is_empty() {
            let _ = std::fs::remove_file(path);
        } else {
            std::fs::write(path, sha).expect("pr head sha");
        }
    }

    /// Sets the forge's judgement of pull request `n`'s head:
    /// `MERGEABLE` (the default), `CONFLICTING` (its merge state then
    /// `DIRTY`) or `UNKNOWN`.
    pub(crate) fn set_mergeable(&self, n: i64, mergeable: &str) {
        std::fs::write(self.dir.join("mergeable").join(n.to_string()), mergeable)
            .expect("pr mergeable");
    }

    /// Sets pull request `n`'s `statusCheckRollup`, as gh prints it.
    pub(crate) fn set_checks(&self, n: i64, rollup: &serde_json::Value) {
        std::fs::write(
            self.dir.join("checks").join(n.to_string()),
            rollup.to_string(),
        )
        .expect("pr checks");
    }

    /// The head branch pull request `n` was created from.
    pub(crate) fn head_of(&self, n: i64) -> String {
        std::fs::read_to_string(self.dir.join("heads").join(n.to_string()))
            .unwrap_or_default()
            .trim()
            .to_string()
    }

    /// The title `pr create` was given for pull request `n`.
    pub(crate) fn title_of(&self, n: i64) -> String {
        std::fs::read_to_string(self.dir.join("titles").join(n.to_string()))
            .unwrap_or_default()
            .trim()
            .to_string()
    }

    /// The comment `pr close N --comment` left on pull request `n`; empty
    /// when it was not closed that way.
    pub(crate) fn close_comment_of(&self, n: i64) -> String {
        std::fs::read_to_string(self.dir.join(format!("comment-{n}"))).unwrap_or_default()
    }

    /// The base pull request `n` targets: as `pr create` was given it, or
    /// as the last `pr edit --base` moved it.
    pub(crate) fn base_of(&self, n: i64) -> String {
        std::fs::read_to_string(self.dir.join("bases").join(n.to_string()))
            .unwrap_or_default()
            .trim()
            .to_string()
    }
}

pub(crate) fn fake_gh() -> FakeGh {
    let bin = tempfile::tempdir().expect("fake gh dir");
    let dir = bin.path().to_path_buf();
    for sub in [
        "state",
        "heads",
        "titles",
        "bases",
        "byhead",
        "oid",
        "mergeable",
        "checks",
    ] {
        std::fs::create_dir_all(dir.join(sub)).expect("fake gh state dir");
    }
    std::fs::write(dir.join("next"), "7").expect("fake gh counter");
    let script = format!(
        r#"#!/bin/sh
d='{d}'
echo "$@" >> "$d/gh.log"
head=""; title=""; base=""; prev=""
for a in "$@"; do
  [ "$prev" = "--head" ] && head="$a"
  [ "$prev" = "--title" ] && title="$a"
  [ "$prev" = "--base" ] && base="$a"
  prev="$a"
done
key=$(printf '%s' "$head" | tr / _)
pr_json() {{
  st=$(cat "$d/state/$1"); m=null
  [ "$st" = MERGED ] && m='"2026-11-04T07:00:00Z"'
  h=$(cat "$d/heads/$1")
  oid=""
  if [ -e "$d/oid/$1" ]; then oid=$(cat "$d/oid/$1")
  elif [ -e "$d/bare" ]; then oid=$(git --git-dir "$(cat "$d/bare")" rev-parse -q --verify "refs/heads/$h" 2>/dev/null)
  fi
  mg=MERGEABLE; ms=CLEAN
  if [ -e "$d/mergeable/$1" ]; then mg=$(cat "$d/mergeable/$1"); [ "$mg" = CONFLICTING ] && ms=DIRTY; fi
  checks='[]'; [ -e "$d/checks/$1" ] && checks=$(cat "$d/checks/$1")
  printf '{{"number":%s,"state":"%s","mergedAt":%s,"url":"https://github.com/o/r/pull/%s","headRefName":"%s","headRefOid":"%s","mergeable":"%s","mergeStateStatus":"%s","statusCheckRollup":%s}}' "$1" "$st" "$m" "$1" "$h" "$oid" "$mg" "$ms" "$checks"
}}
if [ -e "$d/fail" ]; then
  case "$1 $2 $3" in
    "pr list --state"|"pr view "*) cat "$d/fail" >&2; exit 1 ;;
  esac
fi
case "$1 $2 $3" in
  "pr list --head")
    n=$(cat "$d/byhead/$key" 2>/dev/null)
    if [ -n "$n" ] && [ "$(cat "$d/state/$n")" = OPEN ]; then
      echo "[{{\"number\":$n,\"url\":\"https://github.com/o/r/pull/$n\"}}]"
    else
      echo '[]'
    fi ;;
  "pr list --state")
    out=""
    for f in "$d"/state/*; do
      [ -e "$f" ] || continue
      out="$out$(pr_json "$(basename "$f")"),"
    done
    echo "[${{out%,}}]" ;;
  "pr create --head")
    [ -e "$d/fail-create" ] && {{ cat "$d/fail-create" >&2; exit 1; }}
    n=$(cat "$d/next"); echo $((n + 1)) > "$d/next"
    echo OPEN > "$d/state/$n"; echo "$head" > "$d/heads/$n"; echo "$title" > "$d/titles/$n"; echo "$base" > "$d/bases/$n"; echo "$n" > "$d/byhead/$key"
    echo "https://github.com/o/r/pull/$n" ;;
  "pr edit "*)
    [ -e "$d/state/$3" ] || {{ echo "GraphQL: Could not resolve to a PullRequest with the number of $3." >&2; exit 1; }}
    echo "$base" > "$d/bases/$3"
    echo "https://github.com/o/r/pull/$3" ;;
  "pr view "*)
    [ -e "$d/state/$3" ] || {{ echo "GraphQL: Could not resolve to a PullRequest with the number of $3." >&2; exit 1; }}
    pr_json "$3" ;;
  "pr close "*)
    [ -e "$d/state/$3" ] || {{ echo "GraphQL: Could not resolve to a PullRequest with the number of $3." >&2; exit 1; }}
    echo CLOSED > "$d/state/$3"
    c=""; prev=""
    for a in "$@"; do [ "$prev" = "--comment" ] && c="$a"; prev="$a"; done
    printf '%s' "$c" > "$d/comment-$3"
    echo "Closed pull request #$3" ;;
  *) echo "unexpected gh $*" >&2; exit 2 ;;
esac
"#,
        d = dir.display()
    );
    let path = dir.join("gh");
    std::fs::write(&path, script).expect("fake gh script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod gh");
    }
    let guard = hive_testutil::setenv(
        "PATH",
        format!(
            "{}:{}",
            dir.display(),
            std::env::var("PATH").unwrap_or_default()
        ),
    );
    FakeGh {
        dir,
        _bin: bin,
        _path: guard,
    }
}

/// Makes pull request `n` merged on the forge: the upstream's `main` moved
/// to the pull request's head as the upstream has it, and the fake gh told.
pub(crate) fn merge_on_forge(gh: &FakeGh, bare: &str, n: i64) {
    let head = gh.head_of(n);
    assert!(
        !head.is_empty(),
        "pull request #{n} has no head on the fake forge"
    );
    run(
        ".",
        &[
            "git",
            "--git-dir",
            bare,
            "update-ref",
            "refs/heads/main",
            &format!("refs/heads/{head}"),
        ],
    );
    gh.set_state(n, "MERGED");
}

/// The newest event with `action`, as its detail.
pub(crate) fn last_detail(s: &hive_store::Store, action: &str) -> serde_json::Value {
    let rows = must_rows(
        s,
        "SELECT detail FROM events WHERE action = ? ORDER BY seq DESC LIMIT 1",
        &[&action],
    );
    rows.first()
        .map(|r| serde_json::from_str(&r.str("detail")).unwrap_or(serde_json::Value::Null))
        .unwrap_or(serde_json::Value::Null)
}

#[cfg(test)]
mod tests {
    #[test]
    fn deadline_mult() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        for (env, want) in [
            ("3", 3.0),
            ("2.5", 2.5),
            ("0.5", 1.0),
            ("0", 0.0),
            ("abc", 0.0),
        ] {
            let _env = hive_testutil::setenv("HIVE_TEST_DEADLINE_MULT", env);
            let mut want = want;
            if want == 0.0 {
                want = 1.0;
            }
            let got = super::deadline_mult();
            assert_eq!(
                got, want,
                "HIVE_TEST_DEADLINE_MULT={env:?}: got {got} want {want}"
            );
        }
    }
}
