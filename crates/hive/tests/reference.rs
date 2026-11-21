//! The reference pages are what this program prints. `docs/commands.md` and
//! `docs/settings.md` are generated from `hive help` and `hive settings
//! list` (`scripts/gen-reference.sh`), and the site renders them as pages.
//! A change to a command, a flag or a setting that does not regenerate them
//! fails here, with the pages' diff and the command to run.
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
fn the_reference_pages_are_what_this_program_prints() {
    let hive = env!("CARGO_BIN_EXE_hive");
    // The script gives the program a home of its own and unsets every
    // HIVE_* variable but that one, so nothing of the caller's reaches it.
    let out = Command::new("sh")
        .arg(root().join("scripts/gen-reference.sh"))
        .arg("--check")
        .current_dir(root())
        .env("HIVE_BIN", hive)
        .output()
        .expect("run scripts/gen-reference.sh");
    assert!(
        out.status.success(),
        "docs/commands.md or docs/settings.md is not what this program prints. \
         Write them again and commit the pages:\n\n  \
         HIVE_BIN={hive} sh scripts/gen-reference.sh\n\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
