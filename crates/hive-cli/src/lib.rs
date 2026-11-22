//! Command registry, context, help, tables, flags and version helpers shared by Hive commands.

pub mod cli;
pub mod flags;
pub mod help;
pub mod limits;
/// A program a package manager installed is left to it; the hand reads the
/// same marker, so the module lives with the executor path too.
pub use hive_hand_core::managed;
/// Process groups and the straggler sweep live with the executor path
/// (hive-hand-core, ADR-070); the name stays for every caller.
pub use hive_hand_core::procs;
pub mod signed;
pub mod table;
pub mod update;
pub mod version;

pub use cli::*;
pub use flags::{Flag, FlagError, FlagSet, FlagValue, format_duration, parse_duration};
pub use help::{GLOBAL_USAGE, TITLE, edit_distance, help, help_command, suggest};
pub use signed::{SIGNING_IDENTIFIER, code_signed, executable};
pub use table::{Style, table};
pub use version::{
    BuildInfo, BuildSetting, Module, build_version, install_build_info, parse_build_info,
    read_build_info, revision, version,
};
#[cfg(test)]
pub(crate) mod test_helpers {
    pub(crate) fn serial() -> impl Drop {
        hive_testutil::env_lock()
    }
}
