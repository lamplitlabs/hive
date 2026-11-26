//! `.hive/automations/site-shows-main.sh` is the check that keeps the public
//! site on main: it decides when the site is behind, deploys main with the
//! machine's own Vercel login, and reads the public address back. A wrong
//! decision there publishes something that is not on the default branch, or
//! leaves the site behind without a word, so the script's test runs here
//! with the rest of the check. It runs the script in a scratch repository
//! with stand-ins for curl, npx and npm: nothing leaves the machine.
#![cfg(unix)]

use std::path::PathBuf;
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
fn the_site_check_deploys_only_what_main_holds_and_says_when_it_cannot() {
    let out = Command::new("sh")
        .arg(root().join("scripts/tests/site-shows-main-test.sh"))
        .current_dir(root())
        // The script reads these when a person sets them; the test's cases
        // name their own.
        .env_remove("SITE")
        .env_remove("HIVE_STATE_DIR")
        .output()
        .expect("run scripts/tests/site-shows-main-test.sh");
    assert!(
        out.status.success(),
        "scripts/tests/site-shows-main-test.sh failed:\n\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
