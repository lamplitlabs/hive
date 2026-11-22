//! A program a package manager installed (docs/release.md, "Homebrew").
//!
//! Hive replaces its own program file when it updates itself: `hive update`,
//! the console's Update now and the daemon's self-update all write a new file
//! over the running one. A package manager that installed the file owns it
//! and must be the one that replaces it. It says so with a file beside the
//! program, `<program>.managed`:
//!
//! ```text
//! by=Homebrew
//! upgrade=brew upgrade hivefleet
//! path=/opt/homebrew/opt/hivefleet/bin/hive
//! ```
//!
//! `by` names the package manager, `upgrade` is the command that brings a
//! newer version, and `path` is the address that keeps naming the program
//! when the real file moves with every version (Homebrew keeps each version
//! in a directory of its own). Each line is optional: the file being there
//! is what makes the program a managed one. Hive prints the command and
//! never runs it.
//!
//! The marker is read beside the real file, symbolic links resolved. Whoever
//! can write there can replace the program itself, so the marker gives
//! nobody a right that directory had not given already.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// What the marker's name adds to the program's.
pub const SUFFIX: &str = ".managed";

/// The longest value a line may carry; a longer one reads as empty.
const MAX_VALUE: usize = 200;

/// What a marker says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Managed {
    /// Who installed the program ("Homebrew"); empty when the marker does
    /// not say.
    pub by: String,
    /// The command that upgrades it ("brew upgrade hivefleet"); empty when
    /// the marker does not say.
    pub upgrade: String,
    /// The path that names the program across upgrades; empty when the real
    /// file keeps its place.
    pub path: String,
}

impl Managed {
    /// Reads a marker's text: `key=value` lines, `#` comments, unknown keys
    /// passed over. A value with a control character in it, or a longer one
    /// than a line of a terminal, reads as empty, and so does a `path` that
    /// is not absolute.
    pub fn parse(text: &str) -> Managed {
        let mut m = Managed::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            if value.len() > MAX_VALUE || value.chars().any(char::is_control) {
                continue;
            }
            match key.trim() {
                "by" => m.by = value.to_string(),
                "upgrade" => m.upgrade = value.to_string(),
                "path" if Path::new(value).is_absolute() => m.path = value.to_string(),
                _ => {}
            }
        }
        m
    }

    /// Who installed the program, for a sentence: the marker's `by`, or "a
    /// package manager".
    pub fn owner(&self) -> &str {
        if self.by.is_empty() {
            "a package manager"
        } else {
            &self.by
        }
    }

    /// How to get a newer version, for the end of a sentence: "run: brew
    /// upgrade hivefleet", or "upgrade it with Homebrew" when the marker
    /// names no command.
    pub fn upgrade_hint(&self) -> String {
        if !self.upgrade.is_empty() {
            format!("run: {}", self.upgrade)
        } else if !self.by.is_empty() {
            format!("upgrade it with {}", self.by)
        } else {
            "upgrade it with the package manager that installed it".to_string()
        }
    }

    /// The same as a line of its own: "Upgrade it with: brew upgrade
    /// hivefleet".
    pub fn upgrade_line(&self) -> String {
        if !self.upgrade.is_empty() {
            format!("Upgrade it with: {}", self.upgrade)
        } else if !self.by.is_empty() {
            format!("Upgrade it with {}.", self.by)
        } else {
            "Upgrade it with the package manager that installed it.".to_string()
        }
    }

    /// Why Hive does not write over `program`, and what to do in its place.
    pub fn refusal(&self, program: &str) -> String {
        format!(
            "{} installed this hive and owns {program}, so Hive does not replace that file. {}",
            capitalized(self.owner()),
            self.upgrade_line()
        )
    }
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Where the marker of `program` is: beside the real file, with [`SUFFIX`]
/// after its name.
pub fn marker_path(program: &Path) -> PathBuf {
    let real = std::fs::canonicalize(program).unwrap_or_else(|_| program.to_path_buf());
    let mut name = real.into_os_string();
    name.push(SUFFIX);
    PathBuf::from(name)
}

/// What the marker beside `program` says; None when there is none. A
/// marker that is there and cannot be read still makes the program a
/// managed one, with nothing known about who manages it.
pub fn beside(program: &Path) -> Option<Managed> {
    let marker = marker_path(program);
    match std::fs::read_to_string(&marker) {
        Ok(text) => Some(Managed::parse(&text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some(Managed::default()),
    }
}

/// The running program's real file: `current_exe` with its symbolic links
/// resolved.
pub fn real_program() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(std::fs::canonicalize(&exe).unwrap_or(exe))
}

/// What the marker beside the running program says, read once: a package
/// manager does not come or go while the program runs.
pub fn this_program() -> Option<&'static Managed> {
    static THIS: OnceLock<Option<Managed>> = OnceLock::new();
    THIS.get_or_init(|| real_program().and_then(|real| beside(&real)))
        .as_ref()
}

/// The path to start `real` by, and to write into a service file: the
/// marker's `path` when it names that very file today, and `real` itself
/// otherwise (no marker, no `path`, or a `path` that leads elsewhere).
pub fn launch_path(real: &Path, managed: Option<&Managed>) -> PathBuf {
    let Some(stable) = managed.map(|m| m.path.as_str()).filter(|p| !p.is_empty()) else {
        return real.to_path_buf();
    };
    let real_file = std::fs::canonicalize(real).unwrap_or_else(|_| real.to_path_buf());
    match std::fs::canonicalize(stable) {
        Ok(target) if target == real_file => PathBuf::from(stable),
        _ => real.to_path_buf(),
    }
}

/// The marker's `path` for the running program, while it names this very
/// file; None without a marker, without a `path`, and for a `path` that
/// leads elsewhere.
pub fn this_stable_path() -> Option<PathBuf> {
    let real = real_program()?;
    let stable = launch_path(&real, this_program());
    (stable != real).then_some(stable)
}

/// The path to start the running program by: [`this_stable_path`], else its
/// real file. None when the program cannot name itself.
pub fn this_launch_path() -> Option<PathBuf> {
    this_stable_path().or_else(real_program)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("hive");
        std::fs::write(&program, b"#!/bin/sh\n").unwrap();
        (dir, program)
    }

    #[test]
    fn a_marker_says_who_how_and_where() {
        let m = Managed::parse(
            "# written by the formula\nby = Homebrew\nupgrade=brew upgrade hivefleet\npath=/opt/homebrew/opt/hivefleet/bin/hive\nfuture=ignored\nno equals sign\n",
        );
        assert_eq!(
            m,
            Managed {
                by: "Homebrew".into(),
                upgrade: "brew upgrade hivefleet".into(),
                path: "/opt/homebrew/opt/hivefleet/bin/hive".into(),
            }
        );
        assert_eq!(m.owner(), "Homebrew");
        assert_eq!(m.upgrade_hint(), "run: brew upgrade hivefleet");
        assert_eq!(
            m.refusal("/opt/x/hive"),
            "Homebrew installed this hive and owns /opt/x/hive, so Hive does not replace that file. Upgrade it with: brew upgrade hivefleet"
        );
    }

    #[test]
    fn an_empty_marker_still_names_a_way_on() {
        let m = Managed::parse("");
        assert_eq!(m, Managed::default());
        assert_eq!(m.owner(), "a package manager");
        assert_eq!(
            m.upgrade_hint(),
            "upgrade it with the package manager that installed it"
        );
        assert_eq!(
            m.refusal("/usr/lib/hive/hive"),
            "A package manager installed this hive and owns /usr/lib/hive/hive, so Hive does not replace that file. Upgrade it with the package manager that installed it."
        );
        let named = Managed::parse("by=apt");
        assert_eq!(named.upgrade_hint(), "upgrade it with apt");
        assert!(
            named
                .refusal("/usr/lib/hive/hive")
                .ends_with(". Upgrade it with apt.")
        );
    }

    #[test]
    fn values_that_could_not_be_printed_or_followed_read_as_empty() {
        let long = "x".repeat(MAX_VALUE + 1);
        let m = Managed::parse(&format!(
            "by=Home\u{1b}[31mbrew\nupgrade={long}\npath=opt/hivefleet/bin/hive\n"
        ));
        assert_eq!(m, Managed::default());
        // A tab inside a value is a control character too.
        assert_eq!(Managed::parse("upgrade=brew\tupgrade").upgrade, "");
    }

    #[cfg(unix)]
    #[test]
    fn the_marker_is_read_beside_the_real_file() {
        let (dir, program) = scratch();
        assert_eq!(beside(&program), None);
        let real = std::fs::canonicalize(&program).unwrap();
        assert_eq!(
            marker_path(&program),
            PathBuf::from(format!("{}{SUFFIX}", real.display()))
        );
        std::fs::write(marker_path(&program), "by=Homebrew\n").unwrap();
        assert_eq!(beside(&program).unwrap().by, "Homebrew");
        // Through a symbolic link the marker is still the real file's.
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&program, &link).unwrap();
        assert_eq!(beside(&link).unwrap().by, "Homebrew");
        // A marker that cannot be read as text is a marker all the same.
        std::fs::remove_file(marker_path(&program)).unwrap();
        std::fs::create_dir(marker_path(&program)).unwrap();
        assert_eq!(beside(&program), Some(Managed::default()));
    }

    #[cfg(unix)]
    #[test]
    fn the_launch_path_is_the_markers_only_while_it_names_this_file() {
        let (dir, program) = scratch();
        let real = std::fs::canonicalize(&program).unwrap();
        assert_eq!(launch_path(&real, None), real);
        assert_eq!(launch_path(&real, Some(&Managed::default())), real);

        let stable = dir.path().join("opt-bin-hive");
        std::os::unix::fs::symlink(&program, &stable).unwrap();
        let m = Managed {
            path: stable.display().to_string(),
            ..Managed::default()
        };
        assert_eq!(launch_path(&real, Some(&m)), stable);

        // The stable name now leads to another version: not this file's.
        let next = dir.path().join("hive-next");
        std::fs::write(&next, b"#!/bin/sh\n").unwrap();
        std::fs::remove_file(&stable).unwrap();
        std::os::unix::fs::symlink(&next, &stable).unwrap();
        assert_eq!(launch_path(&real, Some(&m)), real);
        // And a name that leads nowhere is not followed either.
        std::fs::remove_file(&stable).unwrap();
        assert_eq!(launch_path(&real, Some(&m)), real);
    }
}
