//! Names from before the product was called Hive, and the one place that
//! says how each is still read. A machine set up then keeps its home
//! (`~/.pulse`, `PULSE_HOME`, `pulse.sqlite`: see `resolve_home` and
//! `adopt_legacy_store`), and a repository set up then keeps the folder its
//! own files live in: `.pulse/` is read wherever `.hive/` is, so nothing in
//! it has to move before the project works again.
//!
//! The rule is the same everywhere: the current name first. Where both
//! folders hold a file of one name, the one under `.hive/` is read and the
//! other is reported. What Hive writes goes under `.hive/`, except an edit
//! of a file that already lives under `.pulse/`, which stays where it is.
//! Every guard that names `.hive/` names `.pulse/` too, so reading the
//! older folder opens nothing the newer one keeps shut.

use crate::store::LEGACY_NAME;

/// The folder a project keeps its own Hive files in, relative to the
/// repository root: automations, skills, agents, rules, hooks.
pub const PROJECT_DIR: &str = ".hive";

/// [`PROJECT_DIR`] under the name it had before the rename.
pub const LEGACY_PROJECT_DIR: &str = ".pulse";

/// Both folders, the current one first.
pub const PROJECT_DIRS: [&str; 2] = [PROJECT_DIR, LEGACY_PROJECT_DIR];

/// Where a repository's skill folders live, relative to its root, in search
/// order: the first `SKILL.md` of a name wins. The layout is the one Claude
/// Code, Copilot and the agents.md convention share; the daemon, the
/// harness and the toolkit all read this list.
pub const REPO_SKILL_DIRS: &[&str] = &[
    ".hive/skills",
    ".pulse/skills",
    ".claude/skills",
    ".github/skills",
    ".agents/skills",
];

/// The prefixes of the branches Hive's agents work on and the test for
/// one: `hive/`, and `pulse/` for a branch made before the rename. They
/// live with the hand, which checks a push against them.
pub use hive_hand_core::names::{AGENT_BRANCH_PREFIXES, agent_branch, agent_branch_rest};

/// What follows the project's folder in `path` (relative to the repository
/// root, forward slashes): `.hive/automations/a.sh` and
/// `.pulse/automations/a.sh` both give `automations/a.sh`. `None` for a
/// path outside both folders, and for the folder itself.
pub fn under_project_dir(path: &str) -> Option<&str> {
    PROJECT_DIRS
        .iter()
        .find_map(|dir| path.strip_prefix(dir)?.strip_prefix('/'))
}

/// `HIVE_X` under the name it had before the rename, `PULSE_X`; `None` for
/// a name that is not one of Hive's.
pub fn legacy_env_name(key: &str) -> Option<String> {
    let rest = key.strip_prefix("HIVE_")?;
    Some(format!("{}_{rest}", LEGACY_NAME.to_ascii_uppercase()))
}

/// Adds every `HIVE_X` of `env` again as `PULSE_X`, for a script or a hook
/// written before the rename that reads the old names. A `PULSE_X` already
/// there is replaced: a value inherited from an outer environment must not
/// disagree with the one this run was given.
pub fn mirror_legacy_env(env: &mut std::collections::BTreeMap<String, String>) {
    let twins: Vec<(String, String)> = env
        .iter()
        .filter_map(|(k, v)| legacy_env_name(k).map(|name| (name, v.clone())))
        .collect();
    env.extend(twins);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_under_either_folder_and_never_the_folder_itself() {
        assert_eq!(
            under_project_dir(".hive/automations/a.sh"),
            Some("automations/a.sh")
        );
        assert_eq!(
            under_project_dir(".pulse/automations/a.sh"),
            Some("automations/a.sh")
        );
        assert_eq!(under_project_dir(".hive/"), Some(""));
        for outside in [
            ".hive",
            ".pulse",
            ".hivemind/a",
            ".pulsed/a",
            "src/.hive/a",
            "a",
        ] {
            assert_eq!(under_project_dir(outside), None, "{outside}");
        }
    }

    #[test]
    fn hive_variables_are_mirrored_under_their_old_names() {
        assert_eq!(
            legacy_env_name("HIVE_STATE_DIR").as_deref(),
            Some("PULSE_STATE_DIR")
        );
        assert_eq!(legacy_env_name("PATH"), None);
        assert_eq!(legacy_env_name("XHIVE_HOME"), None);
        let mut env = std::collections::BTreeMap::from([
            ("HIVE_HOME".to_string(), "/h".to_string()),
            ("HIVE_CLI".to_string(), "/h/bin/hive".to_string()),
            ("PULSE_HOME".to_string(), "/stale".to_string()),
            ("PATH".to_string(), "/bin".to_string()),
        ]);
        mirror_legacy_env(&mut env);
        assert_eq!(env["PULSE_HOME"], "/h");
        assert_eq!(env["PULSE_CLI"], "/h/bin/hive");
        assert_eq!(env["HIVE_HOME"], "/h");
        assert_eq!(env.len(), 5);
    }
}
