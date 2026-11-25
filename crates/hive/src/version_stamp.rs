//! The version `build.rs` stamps a build from source with, as one function
//! of what git and Cargo say, so that a test can call it. The build script
//! compiles this file as a module of its own (`#[path]` in `build.rs`); the
//! crate compiles it only for its tests.

/// `vX.Y.Z` as numbers; None for any other tag.
fn semver(tag: &str) -> Option<(u64, u64, u64)> {
    let rest = tag.strip_prefix('v')?;
    let mut it = rest.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    let patch = it.next()?.parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// The version of a build of the commit `short` (twelve hex digits), which
/// was committed at `stamp` (yyyymmddhhmmss, UTC); `+dirty` follows when the
/// tree had uncommitted changes.
///
/// `exact_tags` is what `git tag --points-at HEAD` printed, `nearest_tag`
/// what `git describe --tags --abbrev=0` printed (empty without a tag), and
/// `workspace` the version in `Cargo.toml`.
///
/// A release tag at HEAD is the version, unless `Cargo.toml` names a later
/// one. Any other build follows a release `vX.Y.Z` and is
/// `vX.Y.(Z+1)-0.<stamp>-<short>`, which semver orders after that release
/// and before the next one. The release it follows is the nearest tag or the
/// workspace version, whichever is higher: the release commit writes its
/// version into `Cargo.toml`, and a checkout that was fetched without tags,
/// as the daemon fetches, knows the later releases by that file alone. So no
/// build is stamped older than the workspace version. Read from the nearest
/// tag only, a build six commits past v0.1.8 called itself v0.1.7-0.<stamp>
/// in a checkout whose newest tag was v0.1.6, and `hive update` offered it
/// v0.1.8.
pub fn source_version(
    exact_tags: &str,
    nearest_tag: &str,
    workspace: &str,
    stamp: &str,
    short: &str,
    dirty: bool,
) -> String {
    let suffix = if dirty { "+dirty" } else { "" };
    let workspace = semver(&format!("v{workspace}"));
    if let Some(tag) = exact_tags
        .lines()
        .filter(|t| semver(t).is_some())
        .max_by_key(|t| semver(t))
        && semver(tag) >= workspace
    {
        return format!("{tag}{suffix}");
    }
    match semver(nearest_tag).max(workspace) {
        Some((ma, mi, pa)) => format!("v{ma}.{mi}.{}-0.{stamp}-{short}{suffix}", pa + 1),
        None => format!("v0.0.0-{stamp}-{short}{suffix}"),
    }
}

#[cfg(test)]
mod tests {
    use super::source_version;
    use hive_cli::update::is_newer;

    const STAMP: &str = "20261125014004";
    const REV: &str = "0123456789ab";

    #[test]
    fn a_build_at_a_release_tag_reads_as_that_release() {
        assert_eq!(
            source_version("v0.1.8", "v0.1.8", "0.1.8", STAMP, REV, false),
            "v0.1.8"
        );
        assert_eq!(
            source_version("v0.1.8", "v0.1.8", "0.1.8", STAMP, REV, true),
            "v0.1.8+dirty"
        );
        // Of several tags at HEAD the highest release counts; other names do not.
        assert_eq!(
            source_version(
                "nightly\nv0.1.7\nv0.1.8\nv1",
                "v0.1.8",
                "0.1.8",
                STAMP,
                REV,
                false
            ),
            "v0.1.8"
        );
        // A release tagged by hand, where Cargo.toml still names the one before.
        assert_eq!(
            source_version("v0.2.0", "v0.2.0", "0.1.8", STAMP, REV, false),
            "v0.2.0"
        );
    }

    #[test]
    fn a_build_after_a_release_is_never_older_than_the_workspace_version() {
        // The release tag is in the checkout: the patch after it, as a
        // pre-release.
        let tagged = source_version("", "v0.1.8", "0.1.8", STAMP, REV, false);
        assert_eq!(tagged, "v0.1.9-0.20261125014004-0123456789ab");
        // A checkout fetched without tags knows only an older release tag,
        // or none: Cargo.toml still names the release the build follows.
        let stale = source_version("", "v0.1.6", "0.1.8", STAMP, REV, false);
        assert_eq!(stale, tagged);
        let untagged = source_version("", "", "0.1.8", STAMP, REV, false);
        assert_eq!(untagged, tagged);
        // What `hive update` compares: the release the build already holds
        // is no update for it, the next one is.
        for version in [&tagged, &stale, &untagged] {
            assert!(!is_newer("v0.1.8", version), "{version}");
            assert!(is_newer("v0.1.9", version), "{version}");
        }
        // A tag by hand that is ahead of Cargo.toml still leads.
        assert_eq!(
            source_version("", "v0.2.0", "0.1.8", STAMP, REV, true),
            "v0.2.1-0.20261125014004-0123456789ab+dirty"
        );
        // A tag at HEAD below the version in Cargo.toml is not this tree's
        // release: the tag is then one more older tag.
        assert_eq!(
            source_version("v0.1.6", "v0.1.6", "0.1.8", STAMP, REV, false),
            tagged
        );
        // Neither a release tag nor a plain X.Y.Z in Cargo.toml.
        assert_eq!(
            source_version("nightly", "nightly", "1.0.0-rc.1", STAMP, REV, false),
            "v0.0.0-20261125014004-0123456789ab"
        );
    }
}
