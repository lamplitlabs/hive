//! The stamp that tells cargo whether the console export in a clone was
//! built in that clone, shared by `build.rs` and the tests that build two
//! clones into one target dir.
//!
//! Several clones of this repository often build into one `CARGO_TARGET_DIR`
//! (every run on a hand did, TASK-156). Cargo keeps one fingerprint of the
//! hive-hub build script for all of them and decides whether to run it
//! before it runs, from the `rerun-if-changed` paths of the last run and
//! their mtimes; it cannot tell one clone from another by what a file says.
//! A clone whose `web/hub/src` is older than the script's last run (a clone
//! made again at a path an earlier clone used, as a retried job on a hand
//! is) therefore looked built, and its `web/hub/out` was never made: the
//! export the crate embeds was missing or a placeholder, and
//! `server::tests::embedded_export` failed on a tree that had not touched
//! the console.
//!
//! A stamp in `OUT_DIR` alone cannot fix that: `OUT_DIR` is one directory
//! for every clone of the target dir, so its mtime says nothing about which
//! clone is building. The script therefore records the manifest dir it
//! built for in `OUT_DIR/console-built-for` (what the shared target dir's
//! export was last built for, for a person reading it) and also leaves a
//! stamp in the export it built, `web/hub/out/.hive-console-stamp`, and
//! declares that one as an input. A clone that has not built its console
//! has no stamp, a declared path that does not exist is never up to date to
//! cargo, and the script runs there; removing `out/` removes the stamp with
//! it. The stamp is dated at the epoch, so it never reads as changed after
//! the run that wrote it and a clone that has built its console is not
//! rebuilt on every build. The stamp is embedded with the export, so it
//! names no path; the server never serves it (`server::Dist`).

use std::path::{Path, PathBuf};

/// The stamp's file name in the export (`web/hub/out`).
pub const STAMP: &str = ".hive-console-stamp";

/// The record's file name in the build script's `OUT_DIR`.
pub const OUT_DIR_RECORD: &str = "console-built-for";

/// The stamp of the export in `out`.
pub fn stamp_path(out: &Path) -> PathBuf {
    out.join(STAMP)
}

/// The line that makes cargo run the build script again whenever the export
/// in `out` carries no stamp: never built in this clone, removed, or
/// replaced by a placeholder.
pub fn rerun_line(out: &Path) -> String {
    format!("cargo:rerun-if-changed={}", stamp_path(out).display())
}

/// What the stamp in the export says. It goes into the binary with the
/// export, so it names no path of the machine that built it.
pub const STAMP_TEXT: &str =
    "Built by crates/hive-hub/build.rs; cargo runs the script again when this file is missing.\n";

/// Stamps the export in `out` as built, dated at the epoch (see the module
/// doc), and records `manifest` (`CARGO_MANIFEST_DIR`) in `out_dir`
/// (`OUT_DIR`) when it is given.
pub fn record(out: &Path, manifest: &Path, out_dir: Option<&Path>) -> std::io::Result<()> {
    let path = stamp_path(out);
    std::fs::write(&path, STAMP_TEXT)?;
    std::fs::File::options()
        .write(true)
        .open(&path)?
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1))?;
    if let Some(dir) = out_dir {
        std::fs::write(
            dir.join(OUT_DIR_RECORD),
            format!("{}\n", manifest.display()),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// A clone of a package whose build script stands in for hive-hub's:
    /// it declares `web/src` and the stamp, "builds" `web/out` (one page and
    /// a line in `web/runs` per run, in place of npm) and stamps it with
    /// this very module.
    fn clone_at(root: &Path, name: &str) -> PathBuf {
        let clone = root.join(name);
        std::fs::create_dir_all(clone.join("src")).unwrap();
        std::fs::create_dir_all(clone.join("web/src")).unwrap();
        std::fs::write(clone.join("web/src/page.txt"), "the console's source\n").unwrap();
        std::fs::write(
            clone.join("Cargo.toml"),
            "[package]\nname = \"stamp-probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\nbuild = \"build.rs\"\n\n[workspace]\n",
        )
        .unwrap();
        std::fs::write(clone.join("src/lib.rs"), "").unwrap();
        let module = hive_testutil::manifest_dir(env!("CARGO_MANIFEST_DIR")).join("build_stamp.rs");
        std::fs::write(
            clone.join("build.rs"),
            format!(
                r#"#[path = {module:?}]
#[allow(dead_code)]
mod build_stamp;
use std::io::Write;
use std::path::Path;
fn main() {{
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR").unwrap();
    let web = Path::new(&manifest).join("web");
    println!("cargo:rerun-if-changed={{}}", web.join("src").display());
    let out = web.join("out");
    println!("{{}}", build_stamp::rerun_line(&out));
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("index.html"), "the console").unwrap();
    let mut runs = std::fs::OpenOptions::new().append(true).create(true).open(web.join("runs")).unwrap();
    writeln!(runs, "ran").unwrap();
    let out_dir = std::env::var_os("OUT_DIR").unwrap();
    build_stamp::record(&out, Path::new(&manifest), Some(Path::new(&out_dir))).unwrap();
}}
"#,
                module = module.display().to_string()
            ),
        )
        .unwrap();
        clone
    }

    /// Dates every file and folder under `dir` back to 2020, older than
    /// any build: what a clone looks like to cargo when its inputs predate
    /// the build script's last run in another clone.
    fn backdate(dir: &Path) {
        let then = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_577_836_800);
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                backdate(&path);
            }
            if let Ok(f) = std::fs::File::open(&path) {
                let _ = f.set_modified(then);
            }
        }
        if let Ok(f) = std::fs::File::open(dir) {
            let _ = f.set_modified(then);
        }
    }

    fn build(clone: &Path, target: &Path) {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let out = Command::new(cargo)
            .args(["build", "-q", "--offline"])
            .current_dir(clone)
            .env("CARGO_TARGET_DIR", target)
            .env_remove("RUSTC_WRAPPER")
            .output()
            .expect("start cargo");
        assert!(
            out.status.success(),
            "cargo build in {} failed:\n{}",
            clone.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn runs(clone: &Path) -> usize {
        std::fs::read_to_string(clone.join("web/runs"))
            .map(|s| s.lines().count())
            .unwrap_or(0)
    }

    /// TASK-156 follow-up: on a hand every run's check built into one
    /// target dir, and a second clone whose console inputs were older than
    /// the build script's last run elsewhere never built its own console.
    /// With the stamp each clone builds its console once, and no more.
    #[test]
    fn two_clones_of_one_target_dir_each_build_their_console() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("shared-target");
        let first = clone_at(tmp.path(), "first");
        let second = clone_at(tmp.path(), "second");
        backdate(&second);

        build(&first, &target);
        assert_eq!(runs(&first), 1);
        assert!(stamp_path(&first.join("web/out")).is_file());

        build(&second, &target);
        assert_eq!(
            runs(&second),
            1,
            "the second clone did not build its console"
        );
        assert!(second.join("web/out/index.html").is_file());
        let stamp = stamp_path(&second.join("web/out"));
        assert_eq!(std::fs::read_to_string(&stamp).unwrap(), STAMP_TEXT);
        let record = std::fs::read_dir(target.join("debug/build"))
            .unwrap()
            .flatten()
            .map(|e| e.path().join("out").join(OUT_DIR_RECORD))
            .find(|p| p.is_file())
            .expect("the build script's OUT_DIR record");
        assert_eq!(
            std::fs::read_to_string(record).unwrap().trim(),
            second.display().to_string(),
            "OUT_DIR names the clone the console was last built for"
        );

        // Built once is enough: the stamp does not make every build run it.
        build(&second, &target);
        assert_eq!(runs(&second), 1, "an unchanged clone ran the script again");

        // An export that went away (with its stamp) is built again, even
        // with every input older than the last run.
        std::fs::remove_dir_all(second.join("web/out")).unwrap();
        backdate(&second);
        build(&second, &target);
        assert_eq!(runs(&second), 2, "a removed export was not built again");
        assert!(second.join("web/out/index.html").is_file());
    }
}
