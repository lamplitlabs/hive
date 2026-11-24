//! How many test chunks `scripts/verify-check.sh` runs at once decides
//! whether the pre-commit verify fits its command limit on a small hand:
//! half the CPUs, at least 2 and at most 8, or `VERIFY_POOL` when the
//! hand sets it. The script's test checks that through `--pool`, which
//! builds nothing, so it runs here with the rest of the check. So does
//! `--scope`, the crates the pre-commit gate narrows the check to; its
//! cases run cargo tree on a scratch workspace, with the cargo that runs
//! this test first on PATH.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    // Read when the test runs: a binary served from a shared target dir
    // carries another checkout's compiled-in path (hive_testutil::manifest_dir).
    hive_testutil::manifest_dir(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

#[test]
fn the_verify_pool_has_a_floor_and_an_override() {
    let mut path = std::env::var_os("PATH").unwrap_or_default();
    if let Some(dir) = std::env::var_os("CARGO")
        .map(PathBuf::from)
        .and_then(|c| c.parent().map(Path::to_path_buf))
    {
        let mut dirs = vec![dir];
        dirs.extend(std::env::split_paths(&path));
        path = std::env::join_paths(dirs).expect("PATH");
    }
    let out = Command::new("sh")
        .arg(root().join("scripts/tests/verify-check-test.sh"))
        .current_dir(root())
        .env("PATH", path)
        .output()
        .expect("run scripts/tests/verify-check-test.sh");
    assert!(
        out.status.success(),
        "scripts/tests/verify-check-test.sh failed:\n\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
