//! The daily "is there a newer Hive?" read (TASK-087). With
//! `update.latest_url` set, the daemon's maintenance pass reads that JSON
//! document at most once every `hive_cli::update::CHECK_HOURS` (sooner only
//! when the setting changed) and keeps what it found in the `meta` row
//! `update`; `hive version` and the hub's banner read it from there. An
//! empty setting - the default - means no request at all. Nothing is
//! downloaded or installed: the check only says that a release exists;
//! `hive update` installs it (release_update.rs). A GitHub API address a
//! private repository refuses is read again through `gh api`.

use hive_cli::update::{
    CHECK_HOURS, LATEST_URL_SETTING, META_KEY, UpdateState, notice, parse_release,
};
use serde_json::Value;
use std::sync::{LazyLock, RwLock};

/// Fetches the release document: the URL in, the parsed JSON (or why not) out.
pub(crate) type Fetch = fn(&str) -> Result<Value, String>;

/// The fetcher the daemon uses - HTTP, and `gh api` for a private GitHub
/// repository (release_update.rs); tests swap it for one that never leaves
/// the process.
static FETCH: LazyLock<RwLock<Fetch>> =
    LazyLock::new(|| RwLock::new(crate::release_update::fetch_json));

#[cfg(test)]
pub(crate) fn set_fetch(f: Fetch) -> Fetch {
    std::mem::replace(&mut *FETCH.write().unwrap_or_else(|e| e.into_inner()), f)
}

/// Whether a check is due at `now` for `url`, given what the last one left.
pub(crate) fn due(state: Option<&UpdateState>, url: &str, now: &str) -> bool {
    let Some(st) = state else {
        return true;
    };
    if st.source != url || st.checked_at.is_empty() {
        return true;
    }
    crate::util::plus_hours(&st.checked_at, CHECK_HOURS).as_str() <= now
}

/// One maintenance-pass step: reads the release document when due, keeps
/// what it found, and returns the daemon's lines - one when a newer release
/// first shows up, one when a read fails for a new reason.
pub(crate) fn tend_update_check(s: &hive_store::Store, now: &str, running: &str) -> Vec<String> {
    let url = s.setting(LATEST_URL_SETTING, "").trim().to_string();
    if url.is_empty() {
        return Vec::new();
    }
    let before = match s.meta(META_KEY) {
        Ok((raw, true)) => UpdateState::from_json(&raw),
        _ => None,
    };
    if !due(before.as_ref(), &url, now) {
        return Vec::new();
    }
    let fetch = *FETCH.read().unwrap_or_else(|e| e.into_inner());
    let mut next = UpdateState {
        checked_at: now.to_string(),
        source: url.clone(),
        ..UpdateState::default()
    };
    // A failed read keeps the release the last good one named, so a flaky
    // network does not hide a notice the owner already has.
    if let Some(b) = before.as_ref().filter(|b| b.source == url) {
        next.latest = b.latest.clone();
        next.url = b.url.clone();
    }
    match fetch(&url).and_then(|doc| parse_release(&doc)) {
        Ok(r) => {
            next.latest = r.version;
            next.url = r.url;
        }
        Err(e) => next.error = e,
    }
    let _ = s.set_meta(META_KEY, &serde_json::to_string(&next).unwrap_or_default());
    let mut lines = Vec::new();
    let known = before
        .as_ref()
        .filter(|b| b.source == url)
        .map(|b| b.latest.as_str())
        .unwrap_or("");
    if next.latest != known
        && let Some(line) = notice(running, &next)
    {
        lines.push(format!("update check: {line}"));
    }
    if !next.error.is_empty() && before.as_ref().is_none_or(|b| b.error != next.error) {
        lines.push(format!(
            "update check: could not read {LATEST_URL_SETTING}: {}",
            next.error
        ));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{TEST_NOW, new_store};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static CALLS: AtomicUsize = AtomicUsize::new(0);

    fn github(url: &str) -> Result<Value, String> {
        CALLS.fetch_add(1, Ordering::SeqCst);
        assert!(url.starts_with("https://"), "{url}");
        Ok(json!({"tag_name": "v0.2.0", "html_url": "https://github.com/o/r/releases/tag/v0.2.0"}))
    }

    fn down(_: &str) -> Result<Value, String> {
        CALLS.fetch_add(1, Ordering::SeqCst);
        Err("connection refused".into())
    }

    fn stored(s: &hive_store::Store) -> UpdateState {
        UpdateState::from_json(&s.meta(META_KEY).unwrap().0).unwrap()
    }

    #[test]
    fn due_once_a_day_and_when_the_url_changes() {
        let st = UpdateState {
            checked_at: "2026-09-06T10:00:00Z".into(),
            source: "https://a.test/x".into(),
            ..UpdateState::default()
        };
        assert!(due(None, "https://a.test/x", TEST_NOW));
        assert!(!due(Some(&st), "https://a.test/x", "2026-09-07T09:59:59Z"));
        assert!(due(Some(&st), "https://a.test/x", "2026-09-07T10:00:00Z"));
        assert!(due(Some(&st), "https://b.test/y", "2026-09-06T10:00:01Z"));
    }

    #[test]
    fn the_check_stays_off_without_a_url_and_reads_once_a_day_with_one() {
        let s = new_store();
        let old = set_fetch(github);
        CALLS.store(0, Ordering::SeqCst);
        // Off by default: no request, nothing stored.
        assert!(tend_update_check(&s, TEST_NOW, "v0.1.0").is_empty());
        assert_eq!(CALLS.load(Ordering::SeqCst), 0);
        assert!(!s.meta(META_KEY).unwrap().1);

        let url = "https://api.github.com/repos/o/r/releases/latest";
        s.set_setting("human", LATEST_URL_SETTING, url).unwrap();
        let lines = tend_update_check(&s, TEST_NOW, "v0.1.0");
        assert_eq!(
            lines,
            vec!["update check: A newer Hive is available: v0.2.0 (running v0.1.0) - https://github.com/o/r/releases/tag/v0.2.0".to_string()]
        );
        assert_eq!(CALLS.load(Ordering::SeqCst), 1);
        let st = stored(&s);
        assert_eq!(
            (
                st.checked_at.as_str(),
                st.source.as_str(),
                st.latest.as_str()
            ),
            (TEST_NOW, url, "v0.2.0")
        );
        assert_eq!(
            hive_cli::update::notice("v0.1.0", &hive_cli::update::stored_state(&s).unwrap())
                .unwrap(),
            "A newer Hive is available: v0.2.0 (running v0.1.0) - https://github.com/o/r/releases/tag/v0.2.0"
        );

        // Within the day: no second request, no second line.
        assert!(tend_update_check(&s, "2026-09-07T09:00:00Z", "v0.1.0").is_empty());
        assert_eq!(CALLS.load(Ordering::SeqCst), 1);
        // A day on, it reads again; the same release says nothing new.
        assert!(tend_update_check(&s, "2026-09-07T10:00:00Z", "v0.1.0").is_empty());
        assert_eq!(CALLS.load(Ordering::SeqCst), 2);

        // A failed read keeps the release it knew and says why, once.
        set_fetch(down);
        let lines = tend_update_check(&s, "2026-09-08T10:00:00Z", "v0.1.0");
        assert_eq!(
            lines,
            vec![format!(
                "update check: could not read {LATEST_URL_SETTING}: connection refused"
            )]
        );
        let st = stored(&s);
        assert_eq!(
            (st.latest.as_str(), st.error.as_str()),
            ("v0.2.0", "connection refused")
        );
        assert!(tend_update_check(&s, "2026-09-09T10:00:00Z", "v0.1.0").is_empty());

        // Turning it off hides the notice the store still holds.
        s.set_setting("human", LATEST_URL_SETTING, "").unwrap();
        assert!(hive_cli::update::stored_state(&s).is_none());
        set_fetch(old);
    }
}
