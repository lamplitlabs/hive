//! The names an environment recipe may use: a pinned tool (`name@version`)
//! and an `env:` variable that is shell-safe and does not say secret. The
//! brain checks a recipe with these before it stores one; the hand checks
//! what it is handed again before it runs anything.

use regex::Regex;
use std::sync::LazyLock;

pub static RECIPE_TOOL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9][a-z0-9_.:-]*@[A-Za-z0-9][A-Za-z0-9_.+-]*$").unwrap());
pub static RECIPE_ENV_NAME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Z_][A-Z0-9_]*$").unwrap());
pub static RECIPE_SECRET_NAME_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"TOKEN|SECRET|PASSWORD|PASSWD|_KEY$|_KEY_|CREDENTIAL|PRIVATE").unwrap()
});
/// A name a recipe's `env:` may set: shell-safe and not one that says secret.
pub fn recipe_env_name_ok(name: &str) -> bool {
    RECIPE_ENV_NAME_RE.is_match(name) && !RECIPE_SECRET_NAME_RE.is_match(name)
}

/// The prefixes of the branches Hive's agents work on: `hive/`, and
/// `pulse/` for a branch made before the rename. A pull request opened
/// then is still open under the old prefix, and it is Hive's own: the
/// reviewer reads it as one, the owner's paths are closed to it, and a hand
/// may push it.
pub const AGENT_BRANCH_PREFIXES: [&str; 2] = ["hive/", "pulse/"];

/// Whether `branch` is one Hive's agents work on, under either prefix.
pub fn agent_branch(branch: &str) -> bool {
    agent_branch_rest(branch).is_some()
}

/// What follows the prefix of an agent's branch: `aurora/job-1` of
/// `hive/aurora/job-1`, `task-TASK-7` of `pulse/task-TASK-7`. `None` for a
/// branch under neither prefix. Whoever reads a job, a task or a
/// conversation out of a branch name reads it from here, so a rule that
/// holds for a `hive/` branch holds for its `pulse/` twin.
pub fn agent_branch_rest(branch: &str) -> Option<&str> {
    AGENT_BRANCH_PREFIXES
        .iter()
        .find_map(|prefix| branch.strip_prefix(prefix))
}

#[cfg(test)]
mod tests {
    #[test]
    fn agent_branches_carry_either_prefix() {
        assert!(super::agent_branch("hive/aurora/job-1"));
        assert!(super::agent_branch("pulse/task-TASK-116"));
        for other in ["main", "hive", "pulsed/x", "feature/hive/x", "evolve/B-1"] {
            assert!(!super::agent_branch(other), "{other}");
        }
        assert_eq!(
            super::agent_branch_rest("hive/aurora/job-1"),
            Some("aurora/job-1")
        );
        assert_eq!(
            super::agent_branch_rest("pulse/task-TASK-7"),
            Some("task-TASK-7")
        );
        assert_eq!(super::agent_branch_rest("evolve/B-1"), None);
    }
}
