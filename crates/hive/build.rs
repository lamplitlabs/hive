//! Stamps the binary with a version derived from the checkout: the exact
//! semver tag at HEAD (when it is not below the workspace version), else the
//! pseudo-version `vX.Y.Z-0.yyyymmddhhmmss-<12hex>` (the patch after the
//! latest reachable tag or after the workspace version, whichever is higher)
//! or `v0.0.0-yyyymmddhhmmss-<12hex>` (neither is a plain version), each with
//! `+dirty` when the tree has uncommitted changes. The decision is
//! `version_stamp::source_version`, in a file the crate's tests compile too.
//!
//! The script belongs to the `hive` binary crate, the leaf of the
//! workspace, and the text it generates reaches `hive-cli`'s `version()`
//! and `revision()` through `hive_cli::install_build_info` at the start of
//! `hive::run_streaming`. It used to live in `hive-cli`, which eighteen
//! workspace crates depend on, so every commit on the brain's `main` moved
//! HEAD, reran the script there and recompiled the whole workspace: the
//! daemon's self-update release build after one such landing rebuilt all 28
//! workspace crates and took 310 s, about ten times a day with the fleet's
//! landing rate. Here a HEAD change recompiles this crate and relinks, and
//! the other crates stay cached.

use std::fs;
use std::path::Path;
use std::process::Command;

#[path = "src/version_stamp.rs"]
mod version_stamp;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Unix seconds -> yyyymmddhhmmss in UTC, the pseudo-version timestamp.
fn stamp(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, m, s) = (rem / 3600, rem % 3600 / 60, rem % 60);
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}{mo:02}{d:02}{h:02}{m:02}{s:02}")
}

/// A build setting value as the build-info block writes it: bare unless it contains a space,
/// tab, CR, LF, `"` or backtick, in which case it is double-quoted with `\\`, `\"`, `\n`, `\r`
/// and `\t` escapes. Versions and -ldflags values are printable ASCII, so those escapes are the
/// whole alphabet needed; `version::unquote` reads them back.
fn strconv_quote(value: &str) -> String {
    if !value.contains([' ', '\t', '\r', '\n', '"', '`']) {
        return value.to_string();
    }
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn rust_string(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// The build-info block embedded in the binary: tab-separated `path`, `mod` and `build` lines,
/// with a leading '\n' so a grep for `^mod\t...` anchors inside the binary's rodata. `hive
/// version`, `revision()` and the pair-hands inspection all read this block, so its layout must
/// stay stable. The constant is written as one string literal and the binary hands a reference
/// to it to `install_build_info`, so the whole block lands in rodata as one contiguous run of
/// bytes, which is what the inspection's grep over a hand's binary relies on.
/// The module path is `hivefleet.dev/hive` since the move to hivefleet.dev (it was
/// `github.com/lamplitlabs/hive` from the hive rename, TASK-115, and `github.com/manishtiwari25/pulse`
/// before that); the inspection (`hive-runner` pair_inspect.rs, `PAIR_LEGACY_HANDS_MODULES`) still
/// reads hands stamped with either old path, so changing it again needs a legacy alias there too.
fn write_build_info(version: &str, rev: &str, dirty: Option<bool>) {
    let module_version = if rev.is_empty() { "(devel)" } else { version };
    let mut text = format!(
        "\npath\thivefleet.dev/hive/cmd/hive\nmod\thivefleet.dev/hive\t{module_version}\t\n"
    );
    if let Ok(v) = std::env::var("HIVE_VERSION")
        && !v.is_empty()
    {
        text.push_str("build\t-ldflags=");
        text.push_str(&strconv_quote(&format!(
            "-X hivefleet.dev/hive/internal/cli.version={v}"
        )));
        text.push('\n');
    }
    if let Some(dirty) = dirty {
        text.push_str(&format!("build\tvcs.revision={rev}\n"));
        text.push_str(&format!("build\tvcs.modified={dirty}\n"));
    }
    let out = std::env::var("OUT_DIR").expect("OUT_DIR");
    fs::write(
        Path::new(&out).join("build_info_const.rs"),
        format!(
            "pub const BUILD_INFO_TEXT: &str = {};\n",
            rust_string(&text)
        ),
    )
    .expect("write build_info_const.rs");
}

fn main() {
    println!("cargo:rerun-if-env-changed=HIVE_VERSION");
    // HEAD, its ref and the tags decide the version. `.git/index` is left out on
    // purpose: every `git status` rewrites it, and a rerun here recompiles and
    // relinks the binary; `+dirty` refreshes whenever HEAD moves.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/tags");
    if let Some(r) = fs::read_to_string("../../.git/HEAD")
        .ok()
        .and_then(|head| head.strip_prefix("ref: ").map(|r| r.trim().to_string()))
    {
        println!("cargo:rerun-if-changed=../../.git/{r}");
    }
    if !Path::new("../../.git").exists() {
        write_build_info("dev", "", None);
        return;
    }
    let rev = git(&["rev-parse", "HEAD"]).unwrap_or_default();
    let short = rev.chars().take(12).collect::<String>();
    let dirty = git(&["status", "--porcelain"]).is_some_and(|s| !s.is_empty());
    let mut version = "dev".to_string();
    if short.len() == 12 {
        let secs = git(&["log", "-1", "--format=%ct", "HEAD"])
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        version = version_stamp::source_version(
            &git(&["tag", "--points-at", "HEAD"]).unwrap_or_default(),
            &git(&[
                "describe",
                "--tags",
                "--abbrev=0",
                "--match",
                "v[0-9]*.[0-9]*.[0-9]*",
            ])
            .unwrap_or_default(),
            &std::env::var("CARGO_PKG_VERSION").unwrap_or_default(),
            &stamp(secs),
            &short,
            dirty,
        );
    }
    write_build_info(&version, &rev, Some(dirty));
}
