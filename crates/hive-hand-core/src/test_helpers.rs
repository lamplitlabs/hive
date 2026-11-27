//! What the core's tests share: one lock for the tests that touch process
//! state, and a command runner that fails the test loudly.

use std::path::Path;
use std::process::Command;
use std::sync::Mutex;

static SERIAL: Mutex<()> = Mutex::new(());

pub(crate) fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn run(dir: impl AsRef<Path>, argv: &[&str]) -> String {
    let out = run_output(dir, argv);
    if !out.status.success() {
        panic!(
            "{}: {}\n{}",
            argv.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// Runs a command the way [`run`] does and hands back its output, failed or not.
pub(crate) fn run_output(dir: impl AsRef<Path>, argv: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(argv[0]);
    cmd.args(&argv[1..]).current_dir(dir.as_ref());
    let env = crate::util::strip_hook_git_env(
        &std::env::vars()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>(),
    );
    cmd.env_clear();
    for kv in env {
        if let Some((k, v)) = kv.split_once('=') {
            cmd.env(k, v);
        }
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@x")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@x")
        .env("GIT_CONFIG_GLOBAL", "/dev/null");
    cmd.output().expect("run command")
}

pub(crate) fn hands_exit_code(err: anyhow::Error) -> i32 {
    err.downcast_ref::<crate::hands::HandsExit>()
        .map(|e| e.code)
        .unwrap_or(1)
}
