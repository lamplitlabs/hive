//! The revert scan of the agent-program loop (ADR-049, PLAN-050 step 2).
//!
//! A variant's primary fitness is kept landings per million tokens, and a
//! landing is kept when, `evolve.keep_days` later, the default branch still
//! has it. This tend writes the other half: one `land.reverted` event per
//! landed sha the default branch no longer keeps, once, so the outcome
//! recompute (step 5) can lower that variant's `kept`.
//!
//! For every `land.landed` of the last `evolve.keep_days × 2` days with no
//! `land.reverted` yet:
//!
//! - a commit on the default branch whose message says
//!   `This reverts commit <sha>` reverted it (`how: revert`, `by`: that
//!   commit);
//! - otherwise, a sha no longer reachable from the default branch was
//!   rolled back (`how: unreachable`) - but only while the landing's
//!   `before` commit still is reachable. When `before` is gone too the
//!   history itself was rewritten (a squash of main, a rebase) and nothing
//!   was rolled back on purpose, so the scan says so and writes nothing.
//!
//! Inert unless `evolve.mode=variants`. Git is read-only and runs through
//! `pulse_gitenv::env()` (optional locks off), so the scan never takes the
//! index lock of a checkout the owner or a landing is writing.

use std::collections::BTreeMap;
use std::time::Duration;

use regex::Regex;
use rusqlite::params;
use serde_json::Value;

use crate::util::Row;

/// Landed shas one project is checked for per pass; the window is days, so
/// this is far above a real project's rate and only bounds a pathological one.
const CANDIDATES_PER_PROJECT: usize = 500;

const GIT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
struct Landed {
    sha: String,
    job: String,
    before: String,
}

pub(crate) fn tend_revert_scan(
    s: &pulse_store::Store,
    projects: &[Row],
    now: &str,
    actor: &str,
) -> Vec<String> {
    if !s.variants_mode() {
        return Vec::new();
    }
    let keep_days = s.setting_int("evolve.keep_days").max(1);
    let since = match crate::util::parse_iso(now) {
        Ok(t) => crate::util::format_iso(t - chrono::Duration::days(keep_days * 2)),
        Err(err) => return vec![format!("revert scan: invalid now {now}: {err}")],
    };
    let mut lines = Vec::new();
    for project in projects {
        let slug = project.str("slug");
        let root = project.str("path");
        if slug.is_empty() || slug == pulse_store::COMMONS_SLUG || root.is_empty() {
            continue;
        }
        match scan_project(s, &slug, &root, &branch_of(project), &since, actor) {
            Ok(mut found) => lines.append(&mut found),
            Err(err) => lines.push(format!("revert scan: {slug}: {err}")),
        }
    }
    lines
}

fn branch_of(project: &Row) -> String {
    let b = project.str("default_branch");
    if b.is_empty() { "main".into() } else { b }
}

fn scan_project(
    s: &pulse_store::Store,
    slug: &str,
    root: &str,
    branch: &str,
    since: &str,
    actor: &str,
) -> anyhow::Result<Vec<String>> {
    let landed = candidates(s, slug, since)?;
    if landed.is_empty() {
        return Ok(Vec::new());
    }
    if !std::path::Path::new(root).is_dir() {
        return Ok(Vec::new());
    }
    let tip = format!("refs/heads/{branch}");
    let head = crate::util::git_timeout(
        root,
        GIT_TIMEOUT,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{tip}^{{commit}}"),
        ],
    );
    if !head.ok() {
        // No default branch to judge against: say nothing rather than
        // count every landing as rolled back.
        return Ok(Vec::new());
    }
    let reverts = revert_commits(root, &tip, since)?;
    let mut lines = Vec::new();
    for l in landed {
        let by = reverts
            .iter()
            .find(|(reverted, by)| *by != l.sha && same_commit(reverted, &l.sha))
            .map(|(_, by)| by.clone());
        let how = if let Some(by) = by {
            Some(("revert", by))
        } else {
            match reachable(root, &l.sha, &tip) {
                Reach::Yes => None,
                Reach::No
                    if !l.before.is_empty() && reachable(root, &l.before, &tip) == Reach::Yes =>
                {
                    Some(("unreachable", String::new()))
                }
                Reach::No => {
                    lines.push(format!(
                        "revert scan: {slug} {} is off {branch} but so is the commit it landed on; history rewritten, not counted",
                        crate::recipe::short(&l.sha, 12)
                    ));
                    None
                }
                Reach::Unknown(why) => {
                    lines.push(format!(
                        "revert scan: {slug} {} not judged: {why}",
                        crate::recipe::short(&l.sha, 12)
                    ));
                    None
                }
            }
        };
        let Some((how, by)) = how else { continue };
        let wrote = s.land_reverted(
            actor,
            &pulse_store::Revert {
                project: slug.to_string(),
                sha: l.sha.clone(),
                job: l.job.clone(),
                by: by.clone(),
                how: how.to_string(),
            },
        )?;
        if wrote {
            lines.push(if by.is_empty() {
                format!(
                    "revert scan: {slug} {} is no longer on {branch}",
                    crate::recipe::short(&l.sha, 12)
                )
            } else {
                format!(
                    "revert scan: {slug} {} reverted by {}",
                    crate::recipe::short(&l.sha, 12),
                    crate::recipe::short(&by, 12)
                )
            });
        }
    }
    Ok(lines)
}

/// The landings of `slug` since `since` that have no `land.reverted` yet,
/// oldest first, one per sha.
fn candidates(s: &pulse_store::Store, slug: &str, since: &str) -> anyhow::Result<Vec<Landed>> {
    let conn = s.conn()?;
    let mut stmt = conn.prepare(
        r#"SELECT e.detail FROM events e
		WHERE e.action = 'land.landed' AND e.at >= ?1
		AND json_valid(e.detail) AND json_extract(e.detail, '$.project') = ?2
		AND COALESCE(json_extract(e.detail, '$.sha'), '') != ''
		AND NOT EXISTS (SELECT 1 FROM events r WHERE r.action = ?3
			AND r.subject = json_extract(e.detail, '$.sha'))
		ORDER BY e.at, e.seq"#,
    )?;
    let mut rows = stmt.query(params![since, slug, pulse_store::LAND_REVERTED])?;
    let mut out: Vec<Landed> = Vec::new();
    while let Some(row) = rows.next()? {
        let text: String = row.get(0)?;
        let d: Value = serde_json::from_str(&text).unwrap_or_default();
        let field = |k: &str| {
            d.get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string()
        };
        let sha = field("sha");
        if !is_sha(&sha) || out.iter().any(|l| l.sha == sha) {
            continue;
        }
        let before = field("before");
        out.push(Landed {
            sha,
            job: field("job"),
            before: if is_sha(&before) {
                before
            } else {
                String::new()
            },
        });
        if out.len() >= CANDIDATES_PER_PROJECT {
            break;
        }
    }
    Ok(out)
}

fn is_sha(v: &str) -> bool {
    v.len() >= 7 && v.len() <= 64 && v.chars().all(|c| c.is_ascii_hexdigit())
}

/// Whether two spellings name one commit: a full sha and its abbreviation.
fn same_commit(a: &str, b: &str) -> bool {
    let (a, b) = (a.to_ascii_lowercase(), b.to_ascii_lowercase());
    a.len() >= 7 && b.len() >= 7 && (a.starts_with(&b) || b.starts_with(&a))
}

/// `(reverted sha, reverting commit)` for every `This reverts commit <sha>`
/// line on `tip` since `since` (committer date: a revert is newer than the
/// landing it reverts, which is inside the window).
fn revert_commits(root: &str, tip: &str, since: &str) -> anyhow::Result<Vec<(String, String)>> {
    let r = crate::util::git_timeout(
        root,
        GIT_TIMEOUT,
        &[
            "log",
            "--no-color",
            &format!("--since={since}"),
            "--grep=This reverts commit",
            "--format=%H%x1f%B%x1e",
            tip,
            "--",
        ],
    );
    if !r.ok() {
        anyhow::bail!(
            "git log {tip} failed: {}",
            r.err.clone().unwrap_or_else(|| r.stderr.trim().to_string())
        );
    }
    Ok(parse_reverts(&r.stdout))
}

fn parse_reverts(log: &str) -> Vec<(String, String)> {
    static RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r"(?i)This reverts commit ([0-9a-f]{7,64})").expect("revert regex")
    });
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for record in log.split('\x1e') {
        let Some((by, body)) = record.trim_start().split_once('\x1f') else {
            continue;
        };
        let by = by.trim();
        for cap in RE.captures_iter(body) {
            // `git log` is newest first; the oldest revert of a sha is the
            // one that took it out, so a later one overwrites nothing.
            out.insert(cap[1].to_ascii_lowercase(), by.to_string());
        }
    }
    out.into_iter().collect()
}

#[derive(Debug, PartialEq, Eq)]
enum Reach {
    Yes,
    No,
    Unknown(String),
}

fn reachable(root: &str, sha: &str, tip: &str) -> Reach {
    let r = crate::util::git_timeout(
        root,
        GIT_TIMEOUT,
        &["merge-base", "--is-ancestor", sha, tip],
    );
    if let Some(err) = r.err {
        return Reach::Unknown(err);
    }
    match r.code {
        0 => Reach::Yes,
        // 1: not an ancestor; 128: the object is gone (garbage collected
        // after a reset), which is not on the branch either.
        1 | 128 => Reach::No,
        n => Reach::Unknown(format!("git merge-base exited {n}: {}", r.stderr.trim())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{new_bare_repo, run, serial};
    use crate::util::event_at;

    const NOW: &str = "2026-09-06T10:00:00Z";

    fn head(root: &str) -> String {
        run(root, &["git", "rev-parse", "HEAD"]).trim().to_string()
    }

    fn commit(root: &str, file: &str, body: &str, msg: &str) -> String {
        std::fs::write(std::path::Path::new(root).join(file), body).unwrap();
        run(root, &["git", "add", "-A"]);
        run(
            root,
            &[
                "git",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-q",
                "-m",
                msg,
            ],
        );
        head(root)
    }

    fn landed(s: &pulse_store::Store, at: &str, job: &str, sha: &str, before: &str) {
        let conn = s.conn().unwrap();
        event_at(
            &conn,
            at,
            "daemon",
            "land.landed",
            &format!("pulse/agent/{job}"),
            serde_json::json!({"project": "proj", "job": job, "sha": sha, "before": before}),
        );
    }

    fn reverted(s: &pulse_store::Store) -> Vec<(String, Value)> {
        let conn = s.conn().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT subject, detail FROM events WHERE action = 'land.reverted' ORDER BY seq",
            )
            .unwrap();
        stmt.query_map([], |r| {
            let d: String = r.get(1)?;
            Ok((r.get(0)?, serde_json::from_str(&d).unwrap_or_default()))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
    }

    #[test]
    fn a_revert_on_the_default_branch_writes_one_land_reverted_and_only_in_variants_mode() {
        let _serial = serial();
        let s = crate::test_helpers::new_store();
        let (project, root) = new_bare_repo(&s);
        let base = head(&root);
        let kept = commit(&root, "kept.txt", "kept\n", "Land kept work");
        let gone = commit(&root, "gone.txt", "gone\n", "Land gone work");
        landed(&s, "2026-09-05T09:00:00Z", "job-kept", &kept, &base);
        landed(&s, "2026-09-05T10:00:00Z", "job-gone", &gone, &kept);
        run(
            &root,
            &[
                "git",
                "-c",
                "commit.gpgsign=false",
                "revert",
                "--no-edit",
                &gone,
            ],
        );
        let by = head(&root);
        let projects = [project];

        // Inert under the default evolve.mode=fixtures.
        assert!(tend_revert_scan(&s, &projects, NOW, "daemon").is_empty());
        assert!(reverted(&s).is_empty());

        s.set_setting("human", "evolve.mode", "variants").unwrap();
        let lines = tend_revert_scan(&s, &projects, NOW, "daemon");
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("reverted by"), "{lines:?}");
        let rows = reverted(&s);
        assert_eq!(rows.len(), 1, "exactly one event: {rows:?}");
        assert_eq!(rows[0].0, gone);
        assert_eq!(rows[0].1["how"], "revert");
        assert_eq!(rows[0].1["by"], by.as_str());
        assert_eq!(rows[0].1["job"], "job-gone");
        assert_eq!(rows[0].1["project"], "proj");

        // The next pass finds nothing new: once per sha.
        assert!(tend_revert_scan(&s, &projects, NOW, "daemon").is_empty());
        assert_eq!(reverted(&s).len(), 1);
    }

    #[test]
    fn a_reset_counts_as_unreachable_but_a_rewritten_history_does_not() {
        let _serial = serial();
        let s = crate::test_helpers::new_store();
        let (project, root) = new_bare_repo(&s);
        s.set_setting("human", "evolve.mode", "variants").unwrap();
        let base = head(&root);
        let dropped = commit(&root, "a.txt", "a\n", "Land a");
        landed(&s, "2026-09-05T09:00:00Z", "job-a", &dropped, &base);
        // The owner rolls the landing back: main returns to its parent.
        run(&root, &["git", "reset", "-q", "--hard", &base]);
        // A landing whose parent is gone too: history was rewritten.
        let orphan_before = "0123456789abcdef0123456789abcdef01234567";
        let orphan = "89abcdef0123456789abcdef0123456789abcdef";
        landed(&s, "2026-09-05T10:00:00Z", "job-b", orphan, orphan_before);
        // Too old for the window (keep_days 7 -> 14 days back).
        let old = commit(&root, "old.txt", "old\n", "Land old");
        run(&root, &["git", "reset", "-q", "--hard", &base]);
        landed(&s, "2026-08-01T10:00:00Z", "job-old", &old, &base);

        let projects = [project];
        let lines = tend_revert_scan(&s, &projects, NOW, "daemon");
        let rows = reverted(&s);
        assert_eq!(rows.len(), 1, "{rows:?} {lines:?}");
        assert_eq!(rows[0].0, dropped);
        assert_eq!(rows[0].1["how"], "unreachable");
        assert_eq!(rows[0].1["by"], "");
        assert!(
            lines.iter().any(|l| l.contains("history rewritten")),
            "{lines:?}"
        );
    }

    #[test]
    fn revert_messages_parse_to_the_sha_they_name() {
        let log = "bbbb\x1fRevert \"x\"\n\nThis reverts commit 1234567890abcdef1234567890abcdef12345678.\n\x1e\naaaa\x1fRevert merge\n\nThis reverts commit ABCDEF1, reversing\nchanges made to 0000000.\n\x1e";
        let got = parse_reverts(log);
        assert_eq!(
            got,
            [
                (
                    "1234567890abcdef1234567890abcdef12345678".to_string(),
                    "bbbb".to_string()
                ),
                ("abcdef1".to_string(), "aaaa".to_string()),
            ]
        );
        assert!(same_commit("abcdef1", "ABCDEF1234567890"));
        assert!(!same_commit("abcdef1", "abcdef2"));
        assert!(!same_commit("abc", "abc"));
    }
}
