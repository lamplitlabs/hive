//! "A newer PULSE is available" (TASK-087). The owner points the setting
//! `update.latest_url` at a JSON document that names the latest release - a
//! GitHub `releases/latest` answer, a GitHub releases list, or a plain
//! `{"version": "v1.2.3", "url": "https://..."}` - and the daemon reads it at
//! most once a day (pulse-runner `update_check.rs`). What it read is kept in
//! the store's `meta` row [`META_KEY`] as an [`UpdateState`]; `pulse version`
//! and the hub's `/api/v1/update` compare it with the running build here.
//!
//! Everything in this file is pure: no network and no clock. An empty
//! setting (the default) means no check and no network at all.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;

/// The setting that turns the check on: the URL of the release document.
pub const LATEST_URL_SETTING: &str = "update.latest_url";
/// The `meta` row the daemon keeps the last check in.
pub const META_KEY: &str = "update";
/// Hours between two reads of the release document.
pub const CHECK_HOURS: f64 = 24.0;

/// One semantic version: `MAJOR.MINOR.PATCH[-PRE][+BUILD]`, a leading `v`
/// allowed. Build metadata (`+dirty`) never changes the order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Semver {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Vec<String>,
}

/// Parses `v1.2.3`, `1.2.3-rc.1`, `v0.0.0-20261108024000-5189a0b73dbe+dirty`.
/// None for anything else (`dev`, `dev-5189a0b73dbe`, `1.2`, `v1.2.3.4`).
pub fn parse_semver(text: &str) -> Option<Semver> {
    let t = text.trim();
    let t = t
        .strip_prefix('v')
        .or_else(|| t.strip_prefix('V'))
        .unwrap_or(t);
    let t = t.split_once('+').map_or(t, |(core, _build)| core);
    let (core, pre) = match t.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (t, None),
    };
    let mut it = core.split('.');
    let mut num = || -> Option<u64> {
        let part = it.next()?;
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let (major, minor, patch) = (num()?, num()?, num()?);
    if it.next().is_some() {
        return None;
    }
    let pre = match pre {
        None => Vec::new(),
        Some(p) => {
            let ids: Vec<String> = p.split('.').map(str::to_string).collect();
            if ids.iter().any(|id| {
                id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            }) {
                return None;
            }
            ids
        }
    };
    Some(Semver {
        major,
        minor,
        patch,
        pre,
    })
}

/// Semver precedence (semver.org section 11): numbers first, then a version
/// without a pre-release ranks above the same one with it, pre-release
/// identifiers compared one by one - numeric ones as numbers and below
/// alphanumeric ones, a longer list above its prefix.
pub fn compare_semver(a: &Semver, b: &Semver) -> Ordering {
    let core = (a.major, a.minor, a.patch).cmp(&(b.major, b.minor, b.patch));
    if core != Ordering::Equal {
        return core;
    }
    match (a.pre.is_empty(), b.pre.is_empty()) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        (false, false) => {}
    }
    for (x, y) in a.pre.iter().zip(b.pre.iter()) {
        let nx = x.bytes().all(|c| c.is_ascii_digit());
        let ny = y.bytes().all(|c| c.is_ascii_digit());
        let o = match (nx, ny) {
            // Long numeric ids (timestamps) compare by length, then text,
            // so they never overflow.
            (true, true) => {
                let (x, y) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
                x.len().cmp(&y.len()).then_with(|| x.cmp(y))
            }
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => x.cmp(y),
        };
        if o != Ordering::Equal {
            return o;
        }
    }
    a.pre.len().cmp(&b.pre.len())
}

/// Whether `latest` is a newer release than `running`. False when either is
/// not a semantic version: a `dev` build cannot say what it is behind.
pub fn is_newer(latest: &str, running: &str) -> bool {
    match (parse_semver(latest), parse_semver(running)) {
        (Some(l), Some(r)) => compare_semver(&l, &r) == Ordering::Greater,
        _ => false,
    }
}

/// The latest release a document names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    /// The release page, when the document names one (http or https only).
    pub url: String,
}

/// Reads the latest release out of the document `update.latest_url`
/// answered: a GitHub `releases/latest` object (`tag_name`, `html_url`), a
/// GitHub releases list (the newest release that is neither a draft nor a
/// pre-release, by version), or `{"version", "url"}` (`tag`/`latest` and
/// `html_url` are read too). Err names what the document lacked.
pub fn parse_release(doc: &Value) -> Result<Release, String> {
    if let Some(list) = doc.as_array() {
        let mut best: Option<(Semver, Release)> = None;
        for item in list {
            if item.get("draft").and_then(Value::as_bool) == Some(true)
                || item.get("prerelease").and_then(Value::as_bool) == Some(true)
            {
                continue;
            }
            let Ok(r) = parse_release(item) else {
                continue;
            };
            let Some(v) = parse_semver(&r.version) else {
                continue;
            };
            if best
                .as_ref()
                .is_none_or(|(b, _)| compare_semver(&v, b) == Ordering::Greater)
            {
                best = Some((v, r));
            }
        }
        return best
            .map(|(_, r)| r)
            .ok_or_else(|| "the release list names no published semver release".to_string());
    }
    if !doc.is_object() {
        return Err("the document is not a JSON object or list".into());
    }
    let pick = |keys: &[&str]| {
        keys.iter()
            .filter_map(|k| doc.get(*k).and_then(Value::as_str))
            .map(str::trim)
            .find(|s| !s.is_empty())
            .unwrap_or("")
            .to_string()
    };
    let version = pick(&["tag_name", "version", "tag", "latest"]);
    if version.is_empty() {
        return Err("the document names no version (tag_name or version)".into());
    }
    if parse_semver(&version).is_none() {
        return Err(format!("{version:?} is not a semantic version"));
    }
    let url = pick(&["html_url", "url"]);
    let url = if is_web_link(&url) {
        url
    } else {
        String::new()
    };
    Ok(Release { version, url })
}

/// http(s) links only: the hub renders it as a link and `pulse version`
/// prints it for a person to open.
pub fn is_web_link(url: &str) -> bool {
    let u = url.trim();
    (u.starts_with("https://") || u.starts_with("http://"))
        && u.len() > "https://".len()
        && !u.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// What the daemon remembers of its last read of `update.latest_url`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateState {
    /// When the document was last read (or tried), UTC ISO.
    #[serde(default)]
    pub checked_at: String,
    /// The setting's value at that read; a changed setting reads again at once.
    #[serde(default)]
    pub source: String,
    /// The latest release the document named, and its page.
    #[serde(default)]
    pub latest: String,
    #[serde(default)]
    pub url: String,
    /// Why the last read failed; empty after a good one.
    #[serde(default)]
    pub error: String,
}

impl UpdateState {
    pub fn from_json(raw: &str) -> Option<Self> {
        serde_json::from_str(raw).ok()
    }

    /// Whether the latest release it knows is newer than `running`.
    pub fn newer_than(&self, running: &str) -> bool {
        !self.latest.is_empty() && is_newer(&self.latest, running)
    }
}

/// The one line `pulse version` prints under the version and the hub shows
/// in its banner, or None when nothing newer is known.
pub fn notice(running: &str, state: &UpdateState) -> Option<String> {
    if !state.newer_than(running) {
        return None;
    }
    let mut line = format!(
        "A newer PULSE is available: {} (running {running})",
        state.latest
    );
    if is_web_link(&state.url) {
        line.push_str(" - ");
        line.push_str(state.url.trim());
    }
    Some(line)
}

/// The state kept in `store`'s meta row, when the check has ever run and
/// `update.latest_url` is still set (turning it off hides an old notice).
pub fn stored_state(store: &pulse_store::Store) -> Option<UpdateState> {
    if store.setting(LATEST_URL_SETTING, "").trim().is_empty() {
        return None;
    }
    match store.meta(META_KEY) {
        Ok((raw, true)) => UpdateState::from_json(&raw),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn semver_parses_tags_and_pseudo_versions() {
        let v = parse_semver("v1.2.3").unwrap();
        assert_eq!((v.major, v.minor, v.patch), (1, 2, 3));
        assert!(v.pre.is_empty());
        let p = parse_semver("v0.0.0-20261108024000-5189a0b73dbe+dirty").unwrap();
        assert_eq!(p.pre, vec!["20261108024000-5189a0b73dbe".to_string()]);
        assert_eq!(parse_semver("1.2.3-rc.1").unwrap().pre, vec!["rc", "1"]);
        for bad in [
            "",
            "dev",
            "dev-5189a0b73dbe",
            "1.2",
            "v1.2.3.4",
            "v1.x.3",
            "1.2.3-",
            "1.2.3-a..b",
            "-1.2.3",
        ] {
            assert_eq!(parse_semver(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn semver_orders_by_precedence() {
        let ordered = [
            "v0.0.0-20261108024000-5189a0b73dbe",
            "v0.1.0",
            "v1.0.0-alpha",
            "v1.0.0-alpha.1",
            "v1.0.0-alpha.beta",
            "v1.0.0-beta",
            "v1.0.0-beta.2",
            "v1.0.0-beta.11",
            "v1.0.0-rc.1",
            "v1.0.0",
            "v1.0.1-0.20261108024000-5189a0b73dbe",
            "v1.0.1",
            "v1.2.0",
            "v1.10.0",
            "v2.0.0",
        ];
        for pair in ordered.windows(2) {
            let (a, b) = (
                parse_semver(pair[0]).unwrap(),
                parse_semver(pair[1]).unwrap(),
            );
            assert_eq!(compare_semver(&a, &b), Ordering::Less, "{pair:?}");
            assert_eq!(compare_semver(&b, &a), Ordering::Greater, "{pair:?}");
        }
        let a = parse_semver("v1.2.3+dirty").unwrap();
        let b = parse_semver("1.2.3").unwrap();
        assert_eq!(compare_semver(&a, &b), Ordering::Equal);
    }

    #[test]
    fn newer_needs_two_semvers() {
        assert!(is_newer("v0.2.0", "v0.1.9"));
        assert!(is_newer("v0.1.0", "v0.0.0-20261108024000-5189a0b73dbe"));
        // The build after v1.0.0 (pseudo v1.0.1-0.<stamp>) is behind v1.0.1.
        assert!(is_newer(
            "v1.0.1",
            "v1.0.1-0.20261108024000-5189a0b73dbe+dirty"
        ));
        assert!(!is_newer("v1.0.0", "v1.0.1-0.20261108024000-5189a0b73dbe"));
        assert!(!is_newer("v0.1.0", "v0.1.0"));
        assert!(!is_newer("v0.1.0", "v0.2.0"));
        assert!(!is_newer("v9.9.9", "dev"));
        assert!(!is_newer("v9.9.9", "dev-5189a0b73dbe-dirty"));
        assert!(!is_newer("latest", "v0.1.0"));
    }

    #[test]
    fn release_reads_github_and_plain_documents() {
        let gh = json!({"tag_name": "v0.3.0", "html_url": "https://github.com/o/r/releases/tag/v0.3.0", "draft": false});
        assert_eq!(
            parse_release(&gh).unwrap(),
            Release {
                version: "v0.3.0".into(),
                url: "https://github.com/o/r/releases/tag/v0.3.0".into()
            }
        );
        let plain = json!({"version": "1.4.0", "url": "https://example.test/pulse"});
        assert_eq!(parse_release(&plain).unwrap().version, "1.4.0");
        let list = json!([
            {"tag_name": "v0.5.0-rc.1", "prerelease": true, "html_url": "https://x.test/rc"},
            {"tag_name": "v0.6.0", "draft": true, "html_url": "https://x.test/draft"},
            {"tag_name": "v0.4.0", "html_url": "https://x.test/4"},
            {"tag_name": "nightly", "html_url": "https://x.test/n"},
            {"tag_name": "v0.4.1", "html_url": "https://x.test/41"},
        ]);
        assert_eq!(
            parse_release(&list).unwrap(),
            Release {
                version: "v0.4.1".into(),
                url: "https://x.test/41".into()
            }
        );
        // A link that is not a web page is dropped, the version stays.
        let odd = json!({"version": "v1.0.0", "url": "javascript:alert(1)"});
        assert_eq!(parse_release(&odd).unwrap().url, "");
        assert!(parse_release(&json!({"name": "x"})).is_err());
        assert!(parse_release(&json!({"tag_name": "nightly"})).is_err());
        assert!(parse_release(&json!([])).is_err());
        assert!(parse_release(&json!("v1.0.0")).is_err());
        assert!(parse_release(&Value::Null).is_err());
    }

    #[test]
    fn notice_names_the_release_and_its_link() {
        let mut st = UpdateState {
            checked_at: "2026-11-09T02:00:00Z".into(),
            source: "https://api.github.com/repos/o/r/releases/latest".into(),
            latest: "v0.2.0".into(),
            url: "https://github.com/o/r/releases/tag/v0.2.0".into(),
            error: String::new(),
        };
        assert_eq!(
            notice("v0.1.0", &st).unwrap(),
            "A newer PULSE is available: v0.2.0 (running v0.1.0) - https://github.com/o/r/releases/tag/v0.2.0"
        );
        assert_eq!(notice("v0.2.0", &st), None);
        assert_eq!(notice("dev", &st), None);
        st.url = String::new();
        assert_eq!(
            notice("v0.1.0", &st).unwrap(),
            "A newer PULSE is available: v0.2.0 (running v0.1.0)"
        );
        st.latest = String::new();
        assert_eq!(notice("v0.1.0", &st), None);
        let back = UpdateState::from_json(&serde_json::to_string(&st).unwrap()).unwrap();
        assert_eq!(back, st);
        assert_eq!(
            UpdateState::from_json("{}").unwrap(),
            UpdateState::default()
        );
    }
}
