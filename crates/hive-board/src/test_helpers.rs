//! Shared test fixtures (new_home, fixture, run_cmd, what, code, count_rows) for every module's #[cfg(test)] tests.
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use anyhow::Result;
use hive_cli::flags::parse_interleaved;
use hive_cli::{Buffer, Context, Error as CliError, FlagSet};
use tempfile::TempDir;

use crate::agents::{JoinInput, join_agent};
use crate::projects::register_project;

pub(crate) const TEST_NOW: &str = "2026-09-06T10:00:00Z";

static EMPTY_STORE: OnceLock<Result<Vec<u8>, String>> = OnceLock::new();

pub(crate) fn open_home(home: &Path) -> hive_store::Store {
    let _guard = hive_testutil::env_lock();
    unsafe {
        std::env::set_var("HIVE_HOME", home);
        std::env::set_var("HIVE_JOB", "");
        std::env::set_var("HIVE_AGENT", "");
        std::env::set_var("HIVE_SESSION", "");
        std::env::set_var("HIVE_PROJECT", "");
    }
    hive_store::Store::open(hive_store::Options {
        home: home.display().to_string(),
        create: true,
        now: TEST_NOW.into(),
        ..Default::default()
    })
    .expect("open test store")
}

pub(crate) fn empty_store_bytes() -> Vec<u8> {
    EMPTY_STORE
        .get_or_init(|| {
            let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
            let old_job = std::env::var("HIVE_JOB").ok();
            let _guard = hive_testutil::env_lock();
            unsafe {
                std::env::remove_var("HIVE_JOB");
            }
            let s = hive_store::Store::open(hive_store::Options {
                home: dir.path().display().to_string(),
                create: true,
                now: TEST_NOW.into(),
                ..Default::default()
            })
            .map_err(|e| e.to_string())?;
            s.close().map_err(|e| e.to_string())?;
            if let Some(v) = old_job {
                unsafe {
                    std::env::set_var("HIVE_JOB", v);
                }
            } else {
                unsafe {
                    std::env::remove_var("HIVE_JOB");
                }
            }
            fs::read(dir.path().join(hive_store::STORE_NAME)).map_err(|e| e.to_string())
        })
        .as_ref()
        .expect("empty store template")
        .clone()
}

pub(crate) fn new_home() -> (TempDir, String, hive_store::Store) {
    let dir = tempfile::tempdir().expect("temp home");
    fs::write(dir.path().join(hive_store::STORE_NAME), empty_store_bytes())
        .expect("copy empty store");
    let s = open_home(dir.path());
    let home = dir.path().display().to_string();
    (dir, home, s)
}

pub(crate) fn git_repo() -> TempDir {
    let dir = tempfile::tempdir().expect("git tempdir");
    let run = |args: &[&str]| {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(dir.path()).args(args).env_clear();
        for kv in hive_testutil::git_env() {
            if let Some((k, v)) = kv.split_once('=') {
                cmd.env(k, v);
            }
        }
        cmd.env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@x")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@x");
        let out = cmd.output().expect("git command");
        assert!(
            out.status.success(),
            "git {args:?}: {}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run(&["init", "-q", "-b", "main"]);
    fs::write(dir.path().join("README.md"), "# t\n").expect("write readme");
    run(&["add", "."]);
    run(&["commit", "-q", "-m", "init"]);
    dir
}

pub(crate) fn fixture() -> (
    TempDir,
    String,
    hive_store::Store,
    hive_store::Project,
    TempDir,
) {
    hive_bus::reset_default();
    let (home_dir, home, s) = new_home();
    let repo = git_repo();
    let (p, _) = register_project(
        &s,
        &repo.path().display().to_string(),
        "demo",
        "",
        false,
        "",
        "human",
        "",
    )
    .expect("register project");
    for name in ["alice", "bob"] {
        join_agent(
            &s,
            JoinInput {
                name: name.into(),
                runner: "copilot".into(),
                model: "m".into(),
                session: format!("sess-{name}"),
                ..Default::default()
            },
        )
        .expect("join agent");
    }
    (home_dir, home, s, p, repo)
}

pub(crate) fn run_cmd(
    s: &hive_store::Store,
    mut c: Option<Context>,
    argv: &[&str],
) -> (String, anyhow::Result<()>) {
    crate::commands::register_commands();
    let cmd = hive_cli::lookup(argv[0]).unwrap_or_else(|| panic!("no command {}", argv[0]));
    let mut fs = FlagSet::new(argv[0]);
    if let Some(flags) = cmd.flags {
        flags(&mut fs);
    }
    if fs.lookup("project").is_none() {
        fs.string_var("project", "", "");
    }
    if fs.lookup("agent").is_none() {
        fs.string_var("agent", "", "");
    }
    if fs.lookup("session").is_none() {
        fs.string_var("session", "", "");
    }
    let args: Vec<String> = argv[1..].iter().map(|s| (*s).to_string()).collect();
    let positional = match parse_interleaved(&mut fs, &args) {
        Ok(p) => p,
        Err(e) => return (String::new(), Err(e.into())),
    };
    let out = Buffer::new();
    let err = Buffer::new();
    let mut ctx = c.take().unwrap_or(Context {
        store: None,
        home: String::new(),
        out: Box::new(out.clone()),
        err: Box::new(err.clone()),
        json: false,
        quiet: false,
        color: false,
        agent: String::new(),
        project: String::new(),
        session: String::new(),
        now: TEST_NOW.into(),
        args: vec![],
    });
    ctx.store = Some(s.clone());
    ctx.home = s.home.clone();
    ctx.out = Box::new(out.clone());
    if ctx.err.write_all(&[]).is_err() {
        ctx.err = Box::new(err);
    }
    ctx.args = positional.clone();
    if !fs.get_string("project").is_empty() {
        ctx.project = fs.get_string("project");
    }
    if !fs.get_string("agent").is_empty() {
        ctx.agent = fs.get_string("agent");
    }
    if !fs.get_string("session").is_empty() {
        ctx.session = fs.get_string("session");
    }
    let result = (cmd.run)(&mut ctx, &fs, &positional);
    (out.string(), result)
}

pub(crate) fn what_any(err: &anyhow::Error) -> String {
    err.downcast_ref::<CliError>()
        .map(|e| e.what.clone())
        .unwrap_or_else(|| err.to_string())
}

pub(crate) fn code(err: Option<&anyhow::Error>) -> i32 {
    match err {
        None => 0,
        Some(e) => e.downcast_ref::<CliError>().map_or(1, CliError::exit_code),
    }
}

pub(crate) fn count_rows(
    s: &hive_store::Store,
    table: &str,
    where_sql: &str,
    args: &[&dyn rusqlite::ToSql],
) -> i64 {
    let q = if where_sql.is_empty() {
        format!("SELECT COUNT(*) FROM {table}")
    } else {
        format!("SELECT COUNT(*) FROM {table} WHERE {where_sql}")
    };
    s.conn()
        .expect("conn")
        .query_row(&q, args, |r| r.get(0))
        .expect("count rows")
}

/// The `EXPLAIN QUERY PLAN` detail lines SQLite gives for `sql`, every
/// parameter bound to a placeholder string; what a test reads to pin the
/// index a query must keep using.
pub(crate) fn query_plan(conn: &rusqlite::Connection, sql: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap_or_else(|e| panic!("{e}: {sql}"));
    let args = vec!["x".to_string(); stmt.parameter_count()];
    stmt.query_map(rusqlite::params_from_iter(args), |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

pub(crate) fn no_tty() {
    crate::commands::set_stdin_for_tests(Some(Vec::new()));
}

pub(crate) fn serial() -> impl Drop {
    hive_testutil::env_lock()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_home_with_runner_environment() {
        let _serial = crate::test_helpers::serial();
        // Hold the (reentrant) env lock for the whole test so no parallel
        // test observes HIVE_AGENT=parent between the set_var and
        // new_home's reset.
        let _guard = hive_testutil::env_lock();
        let parent = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("HIVE_HOME", parent.path());
            for name in ["HIVE_JOB", "HIVE_AGENT", "HIVE_SESSION", "HIVE_PROJECT"] {
                std::env::set_var(name, "parent");
            }
        }
        let (_dir, home, _s) = new_home();
        assert_eq!(std::env::var("HIVE_HOME").unwrap(), home);
        for name in ["HIVE_JOB", "HIVE_AGENT", "HIVE_SESSION", "HIVE_PROJECT"] {
            assert_eq!(std::env::var(name).unwrap_or_default(), "");
        }
    }
}
