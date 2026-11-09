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
