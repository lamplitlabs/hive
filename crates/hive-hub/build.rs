//! Builds the console's static export, `web/hub/out`, which `server::Dist`
//! embeds through `rust_embed`.
//!
//! The export is a build product, not a committed tree: every console
//! change used to ship ninety-odd generated files that caused most landing
//! conflicts, so cargo builds it here from `web/hub/src` and the files
//! around it. The inputs are declared with `rerun-if-changed`, which makes
//! cargo run this script (and so recompile the crate, re-embedding the
//! export) only when one of them differs from the last build; `out/` itself
//! is never an input, because this script writes it.
//!
//! With `npm` on PATH the script runs `npm ci` when `node_modules` is
//! missing or older than `package-lock.json`, then `npm run build`, keeping
//! npm's output to itself unless a step fails, in which case the cargo build
//! fails with that output: a broken console build must not ship a stale
//! bundle. Without `npm` the binary still builds, with a one-page export
//! that says how to get the console, and cargo prints a warning; that build
//! is not cached, so the first one with Node present makes the console.
//! `HIVE_HUB_SKIP_BUILD=1` skips npm and embeds whatever `out/` holds, for
//! work on the Rust side that does not touch the console.

use std::path::Path;
use std::process::Command;

/// What the export is built from, relative to `web/hub`. `mock/` is not
/// here: it does not shape the export. The same list, from the workspace
/// root, is in `BINARY_INPUTS` (crates/hive-runner/src/selfupdate.rs),
/// which tells the daemon a landing changed the binary; a test keeps the
/// two the same.
const INPUTS: &[&str] = &[
    "src",
    "public",
    "package.json",
    "package-lock.json",
    "next.config.ts",
    "tsconfig.json",
    "postcss.config.mjs",
    "components.json",
];

const SKIP_ENV: &str = "HIVE_HUB_SKIP_BUILD";
/// npm as a program to start. On Windows it is a batch file, and a batch
/// file is only found under its full name.
const NPM: &str = if cfg!(windows) { "npm.cmd" } else { "npm" };

const PLACEHOLDER: &str = "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Hive console</title></head>\n<body><p>The Hive console was not built: install Node.js 22+ and rebuild <code>hive</code>.</p></body>\n</html>\n";

fn main() {
    // Read the manifest dir when the script runs, not when it is compiled:
    // a shared CARGO_TARGET_DIR reuses this script's binary across clones,
    // and a path baked in by `env!` would point npm at whichever clone built
    // it first (often one already removed).
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let hub = Path::new(&manifest).join("../../web/hub");
    // A declared path that does not exist would make cargo rerun the script
    // on every build, so only the inputs that are there are declared.
    for input in INPUTS {
        let path = hub.join(input);
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    println!("cargo:rerun-if-env-changed={SKIP_ENV}");

    let out = hub.join("out");
    if std::env::var_os(SKIP_ENV).is_some_and(|v| v == "1") {
        if !out.join("index.html").is_file() {
            placeholder(
                &out,
                &format!("{SKIP_ENV}=1 is set and web/hub/out is empty"),
            );
        }
        return;
    }
    if !npm_available() {
        placeholder(&out, "npm is not on PATH");
        return;
    }
    if needs_install(&hub) {
        npm(&hub, &["ci", "--no-audit", "--no-fund", "--prefer-offline"]);
    }
    npm(&hub, &["run", "build"]);
    assert!(
        out.join("index.html").is_file(),
        "npm run build in {} wrote no out/index.html",
        hub.display()
    );
}

/// Whether `npm` can be started at all; a failure of any other kind is left
/// to the real run, which reports it.
fn npm_available() -> bool {
    match Command::new(NPM).arg("--version").output() {
        Ok(output) => output.status.success(),
        Err(err) => err.kind() != std::io::ErrorKind::NotFound,
    }
}

/// `node_modules` is missing, or the tree npm last installed
/// (`node_modules/.package-lock.json`) predates the lockfile it should
/// match: a fresh clone, or a landing that bumped a dependency.
fn needs_install(hub: &Path) -> bool {
    let modified = |path: &Path| std::fs::metadata(path).and_then(|m| m.modified()).ok();
    match (
        modified(&hub.join("node_modules/.package-lock.json")),
        modified(&hub.join("package-lock.json")),
    ) {
        (Some(installed), Some(lock)) => installed < lock,
        (None, _) => true,
        (Some(_), None) => false,
    }
}

fn npm(hub: &Path, args: &[&str]) {
    let output = Command::new(NPM)
        .args(args)
        .current_dir(hub)
        // Keep the build off the network and out of git's hook environment:
        // a cargo build may run from a git hook, whose GIT_* variables would
        // point any git the build happens to call at the wrong tree.
        .env("NEXT_TELEMETRY_DISABLED", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap_or_else(|err| panic!("run npm {} in {}: {err}", args.join(" "), hub.display()));
    if !output.status.success() {
        panic!(
            "npm {} in {} failed ({}); the console must build for hive to build \
             (set {SKIP_ENV}=1 to embed the existing web/hub/out instead)\n--- stdout\n{}\n--- stderr\n{}",
            args.join(" "),
            hub.display(),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// A one-page export in place of the console, so the binary still builds.
fn placeholder(out: &Path, why: &str) {
    let _ = std::fs::remove_dir_all(out);
    std::fs::create_dir_all(out).unwrap_or_else(|err| panic!("create {}: {err}", out.display()));
    std::fs::write(out.join("index.html"), PLACEHOLDER)
        .unwrap_or_else(|err| panic!("write {}/index.html: {err}", out.display()));
    println!(
        "cargo:warning=the Hive console was not built ({why}): the binary serves a placeholder page until hive is rebuilt with Node.js 22+ on PATH"
    );
    // A placeholder is not cached: a declared path that does not exist is
    // never up to date to cargo, so the script runs again on every build
    // until the console is built, and the first build after Node is
    // installed picks it up without a cargo clean.
    println!(
        "cargo:rerun-if-changed={}",
        out.join(".console-not-built").display()
    );
}
