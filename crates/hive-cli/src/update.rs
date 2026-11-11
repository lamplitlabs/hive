//! "A newer Hive is available" (TASK-087). The owner points the setting
//! `update.latest_url` at a JSON document that names the latest release - a
//! GitHub `releases/latest` answer, a GitHub releases list, or a plain
//! `{"version": "v1.2.3", "url": "https://..."}` - and the daemon reads it at
//! most once a day (hive-runner `update_check.rs`). What it read is kept in
//! the store's `meta` row [`META_KEY`] as an [`UpdateState`]; `hive version`
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
    /// The files attached to the release (GitHub `assets`), in the
    /// document's order; empty when it names none.
    pub assets: Vec<Asset>,
}

/// One file attached to a release: its name and where to download it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    /// The browser download address (`browser_download_url`, or `url` in a
    /// plain document).
    pub url: String,
    /// GitHub's API address of the asset (`url` in a GitHub answer), which
    /// `gh api` can read on a private repository; empty otherwise.
    pub api_url: String,
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
    Ok(Release {
        version,
        url,
        assets: parse_assets(doc),
    })
}

/// The `assets` list of a release object: GitHub's (`name`,
/// `browser_download_url`, API `url`) or a plain one (`name`, `url`).
/// Entries without a name or an http(s) download address are left out.
pub fn parse_assets(doc: &Value) -> Vec<Asset> {
    let Some(list) = doc.get("assets").and_then(Value::as_array) else {
        return Vec::new();
    };
    let text = |item: &Value, key: &str| {
        item.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("")
            .to_string()
    };
    list.iter()
        .filter_map(|item| {
            let name = text(item, "name");
            let browser = text(item, "browser_download_url");
            let url = text(item, "url");
            let (download, api_url) = if browser.is_empty() {
                (url, String::new())
            } else {
                (browser, url)
            };
            let api_url = if is_web_link(&api_url) {
                api_url
            } else {
                String::new()
            };
            (!name.is_empty() && !name.contains('/') && is_web_link(&download)).then_some(Asset {
                name,
                url: download,
                api_url,
            })
        })
        .collect()
}

/// The setting that says how this machine gets a newer Hive.
pub const CHANNEL_SETTING: &str = "update.channel";
/// The file a release lists every archive's SHA-256 in.
pub const CHECKSUMS_ASSET: &str = "SHA256SUMS";
/// The `meta` row that remembers the release the once-per-release prompt
/// last named, so every command does not repeat it.
pub const PROMPTED_KEY: &str = "update.prompted";

/// Where a newer Hive comes from on this machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    /// The daemon rebuilds hive from the brain's checkout (self-update).
    Source,
    /// `hive update` installs a published release.
    Release,
}

impl Channel {
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Source => "source",
            Channel::Release => "release",
        }
    }
}

/// What `update.channel` means here: `source` and `release` say so;
/// `auto` (or empty, or anything else) is source when a brain project is
/// registered and cargo is on PATH - the daemon can rebuild itself - and
/// release otherwise.
pub fn resolve_channel(setting: &str, has_brain: bool, has_cargo: bool) -> Channel {
    match setting.trim().to_ascii_lowercase().as_str() {
        "source" => Channel::Source,
        "release" => Channel::Release,
        _ if has_brain && has_cargo => Channel::Source,
        _ => Channel::Release,
    }
}

/// The target triple a release archive is built for, from Rust's
/// `std::env::consts::{ARCH, OS}`; None for a platform no release ships.
pub fn target_for(arch: &str, os: &str) -> Option<String> {
    let arch = match arch {
        "aarch64" | "arm64" => "aarch64",
        "x86_64" | "amd64" => "x86_64",
        _ => return None,
    };
    let rest = match os {
        "macos" | "darwin" => "apple-darwin",
        "linux" => "unknown-linux-gnu",
        "windows" => "pc-windows-msvc",
        _ => return None,
    };
    Some(format!("{arch}-{rest}"))
}

/// The target triple of the running build.
pub fn running_target() -> Option<String> {
    target_for(std::env::consts::ARCH, std::env::consts::OS)
}

/// The archive name a release carries for `target`:
/// `hive-<version>-<target>.tar.gz`, the version with its leading `v`.
pub fn archive_name(version: &str, target: &str) -> String {
    let v = version.trim();
    let v = if v.starts_with('v') || v.starts_with('V') {
        v.to_string()
    } else {
        format!("v{v}")
    };
    format!("hive-{v}-{target}.tar.gz")
}

/// The archive among `assets` for `target`: the exact
/// [`archive_name`] first, else the one `hive-*<target>*.tar.gz` (or
/// `.tgz`) when there is exactly one. Err says what the release carries.
pub fn pick_asset<'a>(
    assets: &'a [Asset],
    version: &str,
    target: &str,
) -> Result<&'a Asset, String> {
    let want = archive_name(version, target);
    if let Some(a) = assets.iter().find(|a| a.name == want) {
        return Ok(a);
    }
    let close: Vec<&Asset> = assets
        .iter()
        .filter(|a| {
            a.name.starts_with("hive")
                && a.name.contains(target)
                && (a.name.ends_with(".tar.gz") || a.name.ends_with(".tgz"))
        })
        .collect();
    if close.len() == 1 {
        return Ok(close[0]);
    }
    let names: Vec<&str> = assets.iter().map(|a| a.name.as_str()).collect();
    Err(if names.is_empty() {
        format!("release {version} has no files attached; it needs {want}")
    } else if close.is_empty() {
        format!(
            "release {version} has no archive for {target} (want {want}); it carries: {}",
            names.join(", ")
        )
    } else {
        format!(
            "release {version} has {} archives for {target}, none named {want}: {}",
            close.len(),
            close
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })
}

/// Reads a `SHA256SUMS` file (`sha256sum`/`shasum -a 256` output: the hex
/// digest, one or two spaces or ` *`, the file name) into name -> lowercase
/// digest. A `./` or directory prefix on the name is dropped; lines that
/// are not a 64-hex digest and a name are skipped.
pub fn parse_checksums(text: &str) -> std::collections::BTreeMap<String, String> {
    let mut sums = std::collections::BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        let Some((digest, rest)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let name = rest.trim_start().trim_start_matches('*').trim();
        let name = name.rsplit('/').next().unwrap_or(name);
        if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) || name.is_empty() {
            continue;
        }
        sums.insert(name.to_string(), digest.to_ascii_lowercase());
    }
    sums
}

/// Whether the archive `name` with digest `actual` (hex) matches what the
/// release's checksums list says; Err says why not.
pub fn check_digest(
    sums: &std::collections::BTreeMap<String, String>,
    name: &str,
    actual: &str,
) -> Result<(), String> {
    let Some(want) = sums.get(name) else {
        return Err(format!("{CHECKSUMS_ASSET} lists no digest for {name}"));
    };
    if want.eq_ignore_ascii_case(actual.trim()) {
        Ok(())
    } else {
        Err(format!(
            "{name} does not match {CHECKSUMS_ASSET}: expected sha256 {want}, downloaded {}",
            actual.trim().to_ascii_lowercase()
        ))
    }
}

/// The one stderr line every command prints once per release, or None:
/// nothing newer is known, this machine follows the source channel, or the
/// line for this release was already shown (`prompted` is the release the
/// last prompt named).
pub fn prompt(
    running: &str,
    state: &UpdateState,
    channel: Channel,
    prompted: &str,
) -> Option<String> {
    if channel != Channel::Release || !state.newer_than(running) {
        return None;
    }
    if prompted.trim() == state.latest.trim() {
        return None;
    }
    Some(format!(
        "A newer Hive {} is available - run: hive update",
        state.latest
    ))
}

/// http(s) links only: the hub renders it as a link and `hive version`
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

/// The one line `hive version` prints under the version and the hub shows
/// in its banner, or None when nothing newer is known.
pub fn notice(running: &str, state: &UpdateState) -> Option<String> {
    if !state.newer_than(running) {
        return None;
    }
    let mut line = format!(
        "A newer Hive is available: {} (running {running})",
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
pub fn stored_state(store: &hive_store::Store) -> Option<UpdateState> {
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
                url: "https://github.com/o/r/releases/tag/v0.3.0".into(),
                assets: Vec::new(),
            }
        );
        let plain = json!({"version": "1.4.0", "url": "https://example.test/hive"});
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
                url: "https://x.test/41".into(),
                assets: Vec::new(),
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
            "A newer Hive is available: v0.2.0 (running v0.1.0) - https://github.com/o/r/releases/tag/v0.2.0"
        );
        assert_eq!(notice("v0.2.0", &st), None);
        assert_eq!(notice("dev", &st), None);
        st.url = String::new();
        assert_eq!(
            notice("v0.1.0", &st).unwrap(),
            "A newer Hive is available: v0.2.0 (running v0.1.0)"
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

    fn asset(name: &str) -> Asset {
        Asset {
            name: name.into(),
            url: format!("https://dl.test/{name}"),
            api_url: String::new(),
        }
    }

    #[test]
    fn assets_come_from_github_and_plain_documents() {
        let gh = json!({
            "tag_name": "v0.3.0",
            "assets": [
                {"name": "hive-v0.3.0-aarch64-apple-darwin.tar.gz",
                 "url": "https://api.github.com/repos/o/r/releases/assets/7",
                 "browser_download_url": "https://github.com/o/r/releases/download/v0.3.0/hive-v0.3.0-aarch64-apple-darwin.tar.gz"},
                {"name": "SHA256SUMS", "url": "https://dl.test/SHA256SUMS"},
                {"name": "", "url": "https://dl.test/x"},
                {"name": "../evil", "url": "https://dl.test/evil"},
                {"name": "bad-link", "url": "file:///etc/passwd"}
            ]
        });
        let r = parse_release(&gh).unwrap();
        assert_eq!(
            r.assets,
            vec![
                Asset {
                    name: "hive-v0.3.0-aarch64-apple-darwin.tar.gz".into(),
                    url: "https://github.com/o/r/releases/download/v0.3.0/hive-v0.3.0-aarch64-apple-darwin.tar.gz".into(),
                    api_url: "https://api.github.com/repos/o/r/releases/assets/7".into(),
                },
                Asset {
                    name: "SHA256SUMS".into(),
                    url: "https://dl.test/SHA256SUMS".into(),
                    api_url: String::new(),
                },
            ]
        );
        assert!(parse_assets(&json!({"assets": "nope"})).is_empty());
    }

    #[test]
    fn asset_selection_follows_the_running_target() {
        assert_eq!(
            target_for("aarch64", "macos").as_deref(),
            Some("aarch64-apple-darwin")
        );
        assert_eq!(
            target_for("x86_64", "macos").as_deref(),
            Some("x86_64-apple-darwin")
        );
        assert_eq!(
            target_for("x86_64", "linux").as_deref(),
            Some("x86_64-unknown-linux-gnu")
        );
        assert_eq!(target_for("riscv64", "linux"), None);
        assert_eq!(target_for("aarch64", "freebsd"), None);
        assert_eq!(
            archive_name("0.3.0", "aarch64-apple-darwin"),
            "hive-v0.3.0-aarch64-apple-darwin.tar.gz"
        );

        let assets = vec![
            asset("SHA256SUMS"),
            asset("hive-v0.3.0-x86_64-apple-darwin.tar.gz"),
            asset("hive-v0.3.0-aarch64-apple-darwin.tar.gz"),
        ];
        let arm = pick_asset(&assets, "v0.3.0", "aarch64-apple-darwin").unwrap();
        assert_eq!(arm.name, "hive-v0.3.0-aarch64-apple-darwin.tar.gz");
        let intel = pick_asset(&assets, "v0.3.0", "x86_64-apple-darwin").unwrap();
        assert_eq!(intel.name, "hive-v0.3.0-x86_64-apple-darwin.tar.gz");
        // An archive named differently but alone for the target is taken.
        let odd = vec![asset("hive-0.3.0-aarch64-apple-darwin.tgz")];
        assert_eq!(
            pick_asset(&odd, "v0.3.0", "aarch64-apple-darwin")
                .unwrap()
                .name,
            "hive-0.3.0-aarch64-apple-darwin.tgz"
        );
        let err = pick_asset(&assets, "v0.3.0", "x86_64-unknown-linux-gnu").unwrap_err();
        assert!(
            err.contains("no archive for x86_64-unknown-linux-gnu"),
            "{err}"
        );
        assert!(
            err.contains("hive-v0.3.0-aarch64-apple-darwin.tar.gz"),
            "{err}"
        );
        let err = pick_asset(&[], "v0.3.0", "aarch64-apple-darwin").unwrap_err();
        assert!(err.contains("no files attached"), "{err}");
        let two = vec![
            asset("hive-a-aarch64-apple-darwin.tar.gz"),
            asset("hive-b-aarch64-apple-darwin.tar.gz"),
        ];
        let err = pick_asset(&two, "v0.3.0", "aarch64-apple-darwin").unwrap_err();
        assert!(err.contains("2 archives"), "{err}");
    }

    /// A Linux brain (the hosted image of ADR-070 step 6) takes the
    /// linux-gnu archive of a release that carries one beside the macOS
    /// ones, for its own architecture (TASK-107).
    #[test]
    fn a_linux_target_picks_the_linux_gnu_archive() {
        let assets = vec![
            asset("SHA256SUMS"),
            asset("hive-v0.4.0-aarch64-apple-darwin.tar.gz"),
            asset("hive-v0.4.0-x86_64-apple-darwin.tar.gz"),
            asset("hive-v0.4.0-x86_64-unknown-linux-gnu.tar.gz"),
            asset("hive-v0.4.0-aarch64-unknown-linux-gnu.tar.gz"),
        ];
        let x86 = target_for("x86_64", "linux").unwrap();
        assert_eq!(
            pick_asset(&assets, "v0.4.0", &x86).unwrap().name,
            "hive-v0.4.0-x86_64-unknown-linux-gnu.tar.gz"
        );
        let arm = target_for("aarch64", "linux").unwrap();
        assert_eq!(
            pick_asset(&assets, "0.4.0", &arm).unwrap().name,
            "hive-v0.4.0-aarch64-unknown-linux-gnu.tar.gz"
        );
        // A Mac still takes its own archive from the same release.
        assert_eq!(
            pick_asset(&assets, "v0.4.0", "aarch64-apple-darwin")
                .unwrap()
                .name,
            "hive-v0.4.0-aarch64-apple-darwin.tar.gz"
        );
    }

    #[test]
    fn checksums_parse_and_verify() {
        let a = "a".repeat(64);
        let b = "B".repeat(64);
        let text = format!(
            "{a}  hive-v0.3.0-aarch64-apple-darwin.tar.gz\n{b} *./dist/hive-v0.3.0-x86_64-apple-darwin.tar.gz\nnot-a-digest  x\n\n{}  short\n",
            "c".repeat(10)
        );
        let sums = parse_checksums(&text);
        assert_eq!(sums.len(), 2, "{sums:?}");
        assert_eq!(
            sums["hive-v0.3.0-x86_64-apple-darwin.tar.gz"],
            "b".repeat(64)
        );
        assert!(check_digest(&sums, "hive-v0.3.0-aarch64-apple-darwin.tar.gz", &a).is_ok());
        assert!(check_digest(&sums, "hive-v0.3.0-x86_64-apple-darwin.tar.gz", &b).is_ok());
        let err = check_digest(&sums, "hive-v0.3.0-aarch64-apple-darwin.tar.gz", &b).unwrap_err();
        assert!(err.contains("does not match SHA256SUMS"), "{err}");
        let err = check_digest(&sums, "missing.tar.gz", &a).unwrap_err();
        assert!(err.contains("lists no digest for missing.tar.gz"), "{err}");
    }

    #[test]
    fn channel_defaults_to_source_only_where_the_daemon_can_build() {
        assert_eq!(resolve_channel("", true, true), Channel::Source);
        assert_eq!(resolve_channel("auto", true, false), Channel::Release);
        assert_eq!(resolve_channel("auto", false, true), Channel::Release);
        assert_eq!(resolve_channel("release", true, true), Channel::Release);
        assert_eq!(resolve_channel(" Source ", false, false), Channel::Source);
        assert_eq!(Channel::Release.as_str(), "release");
    }

    #[test]
    fn prompt_shows_once_per_release_on_the_release_channel() {
        let st = UpdateState {
            latest: "v0.2.0".into(),
            url: "https://x.test/v0.2.0".into(),
            ..UpdateState::default()
        };
        assert_eq!(
            prompt("v0.1.0", &st, Channel::Release, "").unwrap(),
            "A newer Hive v0.2.0 is available - run: hive update"
        );
        // Shown for v0.1.x before: a new release prompts again.
        assert!(prompt("v0.1.0", &st, Channel::Release, "v0.1.5").is_some());
        assert_eq!(prompt("v0.1.0", &st, Channel::Release, "v0.2.0"), None);
        assert_eq!(prompt("v0.1.0", &st, Channel::Source, ""), None);
        assert_eq!(prompt("v0.2.0", &st, Channel::Release, ""), None);
        assert_eq!(prompt("dev", &st, Channel::Release, ""), None);
    }
}
