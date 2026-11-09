//! The hand carries none of the brain (ADR-070 rule 1): no brief text in
//! the binary, no brain crate in its dependency graph, and the check that
//! guards the graph fails on a manifest that pulls the brain in.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Phrases only the brain's worker brief holds
/// (crates/pulse-runner/src/brief.rs); `scripts/check-hand-deps.sh strings`
/// looks for the same three in the release binary.
const SENTINELS: [&str; 3] = [
    "Show your work when it has a face",
    "Clean up after yourself before you finish",
    "Never edit",
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|w| *w == needle)
        .count()
}

/// `sh scripts/check-hand-deps.sh ARGS` from the workspace root, with the
/// cargo that runs this test first on PATH.
fn check(args: &[&str]) -> std::process::Output {
    let mut path = std::env::var_os("PATH").unwrap_or_default();
    if let Some(dir) = std::env::var_os("CARGO")
        .map(PathBuf::from)
        .and_then(|c| c.parent().map(Path::to_path_buf))
    {
        let mut dirs = vec![dir];
        dirs.extend(std::env::split_paths(&path));
        path = std::env::join_paths(dirs).expect("PATH");
    }
    Command::new("sh")
        .arg(root().join("scripts/check-hand-deps.sh"))
        .args(args)
        .current_dir(root())
        .env("PATH", path)
        .output()
        .expect("run scripts/check-hand-deps.sh")
}

#[test]
fn the_binary_carries_no_brief_text() {
    let bin = std::fs::read(env!("CARGO_BIN_EXE_pulse-hand")).expect("read pulse-hand");
    for phrase in SENTINELS {
        assert_eq!(
            count(&bin, phrase.as_bytes()),
            0,
            "pulse-hand carries the brief's {phrase:?}"
        );
    }
    // The phrases are real: the brain's brief still says each of them, so a
    // rename there cannot leave this test checking for nothing.
    let brief = std::fs::read_to_string(root().join("crates/pulse-runner/src/brief.rs"))
        .expect("read brief.rs");
    for phrase in SENTINELS {
        assert!(brief.contains(phrase), "brief.rs no longer says {phrase:?}");
    }
}

#[test]
fn the_binary_answers_its_version_and_refuses_an_unknown_verb() {
    let bin = env!("CARGO_BIN_EXE_pulse-hand");
    let out = Command::new(bin).arg("--version").output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("pulse-hand "));
    let work = tempfile::tempdir().unwrap();
    let out = Command::new(bin)
        .arg("no-such-verb")
        .env("PULSE_HANDS_WORKDIR", work.path())
        .env_remove("SSH_ORIGINAL_COMMAND")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{out:?}");
}

#[test]
fn the_dependency_check_passes_the_hand_and_fails_a_bad_manifest() {
    let out = check(&["self-test"]);
    assert!(
        out.status.success(),
        "self-test: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let out = check(&["deps"]);
    assert!(
        out.status.success(),
        "pulse-hand links the brain: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The brain itself fails the same check, naming what it links.
    let out = check(&["deps", "--package", "pulse-runner"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("pulse-evolve"));
}
