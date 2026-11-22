//! The Homebrew formulae a release writes (`scripts/brew-formula.sh`) have
//! to agree with three things that live elsewhere: the archive names the
//! release workflow publishes, the marker `hive_hand_core::managed` reads,
//! and Homebrew's own rules for a formula. The script's test checks all
//! three and needs no network and no Homebrew, so it runs here with the
//! rest: a renamed archive or a changed marker key fails the project's
//! check, not the first release after it.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

#[test]
fn the_homebrew_formulae_agree_with_the_release_and_the_program() {
    let out = Command::new("sh")
        .arg(root().join("scripts/tests/brew-formula-test.sh"))
        .current_dir(root())
        .output()
        .expect("run scripts/tests/brew-formula-test.sh");
    assert!(
        out.status.success(),
        "scripts/tests/brew-formula-test.sh failed:\n\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
