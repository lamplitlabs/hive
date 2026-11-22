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

/// One program as it was when it was looked at: its real file, what the
/// marker beside it said, and the path that marker gave it, kept only when
/// that path led to this file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Program {
    pub real: Option<PathBuf>,
    pub managed: Option<Managed>,
    pub stable: Option<PathBuf>,
}

impl Program {
    /// Looks at the program whose real file is `real`.
    pub fn read(real: Option<PathBuf>) -> Program {
        let managed = real.as_deref().and_then(beside);
        let stable = real.as_deref().and_then(|real| {
            let stable = launch_path(real, managed.as_ref());
            (stable != real).then_some(stable)
        });
        Program {
            real,
            managed,
            stable,
        }
    }

    /// What the marker of `program` says: for this program what was read
    /// when it was looked at, for any other file what the disk says now.
    pub fn marker_of(&self, program: &Path) -> Option<Managed> {
        if self.real.as_deref() == Some(program) {
            return self.managed.clone();
        }
        beside(program)
    }

    /// The path to start this program by: the marker's while it led here,
    /// else the real file.
    pub fn launch(&self) -> Option<&Path> {
        self.stable.as_deref().or(self.real.as_deref())
    }
}

/// The running program, looked at once, when the process first asks, and
/// remembered. A package manager's upgrade removes the old version's
/// directory, marker included, while a daemon started from it still runs:
/// that daemon must go on knowing whose file it runs from until it has
/// restarted onto the new one. On Linux the running file's own name reads
/// as `hive (deleted)` from then on, so the name is kept from before too.
pub fn this() -> &'static Program {
    static THIS: OnceLock<Program> = OnceLock::new();
    THIS.get_or_init(|| Program::read(real_program()))
}

/// What the marker beside the running program says ([`this`]).
pub fn this_program() -> Option<&'static Managed> {
    this().managed.as_ref()
}

/// The running program's real file ([`this`]).
pub fn this_real_path() -> Option<&'static Path> {
    this().real.as_deref()
}

/// The marker's `path` for the running program, while it named this very
/// file ([`this`]); None without a marker, without a `path`, and for a
/// `path` that led elsewhere.
pub fn this_stable_path() -> Option<&'static Path> {
    this().stable.as_deref()
}

/// The path to start the running program by: [`this_stable_path`], else its
/// real file. None when the program cannot name itself.
pub fn this_launch_path() -> Option<&'static Path> {
    this().launch()
}

/// What the marker of `program` says: for the running program what
/// [`this`] remembers, for any other file what the disk says now.
pub fn of(program: &Path) -> Option<Managed> {
    this().marker_of(program)
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

    /// An upgrade removes the old version's directory while a daemon
    /// started from it still runs: what was read at the start is what
    /// counts for that program, and the disk for every other.
    #[cfg(unix)]
    #[test]
    fn a_program_remembers_its_marker_after_an_upgrade_removed_it() {
        let dir = tempfile::tempdir().unwrap();
        let keg = dir.path().join("0.1.5");
        std::fs::create_dir(&keg).unwrap();
        let program = keg.join("hive");
        std::fs::write(&program, b"#!/bin/sh\n").unwrap();
        let real = std::fs::canonicalize(&program).unwrap();
        let stable = std::fs::canonicalize(dir.path()).unwrap().join("opt-hive");
        std::os::unix::fs::symlink(&real, &stable).unwrap();
        std::fs::write(
            marker_path(&real),
            format!(
                "by=Homebrew\nupgrade=brew upgrade hivefleet\npath={}\n",
                stable.display()
            ),
        )
        .unwrap();

        let started = Program::read(Some(real.clone()));
        assert_eq!(started.managed.as_ref().unwrap().by, "Homebrew");
        assert_eq!(started.stable.as_deref(), Some(stable.as_path()));
        assert_eq!(started.launch(), Some(stable.as_path()));

        // The upgrade: a new version beside the old, the name moved to it,
        // the old directory removed.
        let next = dir.path().join("0.1.6");
        std::fs::create_dir(&next).unwrap();
        std::fs::write(next.join("hive"), b"#!/bin/sh\n").unwrap();
        std::fs::remove_file(&stable).unwrap();
        std::os::unix::fs::symlink(next.join("hive"), &stable).unwrap();
        std::fs::remove_dir_all(&keg).unwrap();

        assert_eq!(beside(&real), None, "the disk has no marker any more");
        assert_eq!(
            started.marker_of(&real).unwrap().upgrade,
            "brew upgrade hivefleet"
        );
        assert_eq!(started.launch(), Some(stable.as_path()));
        // Another file is read from the disk as it is now.
        assert_eq!(started.marker_of(&next.join("hive")), None);
        std::fs::write(marker_path(&next.join("hive")), "by=Homebrew\n").unwrap();
        assert_eq!(
            started.marker_of(&next.join("hive")).unwrap().by,
            "Homebrew"
        );

        // A program with no marker starts by its own file.
        let lone = dir.path().join("lone");
        std::fs::write(&lone, b"#!/bin/sh\n").unwrap();
        let lone = std::fs::canonicalize(&lone).unwrap();
        let unmanaged = Program::read(Some(lone.clone()));
        assert_eq!(
            (unmanaged.managed.clone(), unmanaged.stable.clone()),
            (None, None)
        );
        assert_eq!(unmanaged.launch(), Some(lone.as_path()));
        assert_eq!(Program::read(None), Program::default());
    }
}
