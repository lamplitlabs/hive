//! The daemon's side of the project's committed decision records: once per
//! pass, every project whose `docs/decisions` tree changed on the default
//! branch has it imported into the store (`hive migrate --records` does the
//! same by hand). A new file becomes a record with the status it was
//! committed with, and a record whose committed status is `Accepted` is
//! marked accepted in the store - merging the ADR to the default branch is
//! the owner's acceptance. A store status is never moved back: only a draft
//! or proposed record is promoted.
//!
//! The tree hash of the last import is remembered per project in a
//! `records.synced` event, so a pass on an unchanged tree reads one event
//! row and one `git rev-parse` and writes nothing.

use hive_tasks::manifest;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;

/// The folder the daemon imports.
pub(crate) const DECISIONS: &str = "docs/decisions";
/// The event that remembers the last imported tree of a project.
pub(crate) const SYNCED: &str = "records.synced";

/// Imports `slug`'s committed decision records when their tree changed;
/// says what it did.
pub(crate) fn tend_records(
    s: &hive_store::Store,
    slug: &str,
    _now: &str,
    actor: &str,
) -> Vec<String> {
    let Ok(Some((repo, branch, commit))) = manifest::repo_of(s, slug) else {
        return Vec::new();
    };
    let Some(tree) = decisions_tree(&repo, &commit) else {
        return Vec::new();
    };
    if last_synced_tree(s, slug).as_deref() == Some(tree.as_str()) {
        return Vec::new();
    }
    let short: String = commit.chars().take(12).collect();
    let summary = match hive_records::import_repo(
        s,
        &repo,
        &commit,
        slug,
        hive_records::ImportOptions {
            author: actor.to_string(),
            paths: vec![DECISIONS.to_string()],
            ..Default::default()
        },
    ) {
        Ok(summary) => summary,
        Err(e) => {
            return vec![format!(
                "records {slug}: import of {DECISIONS} at {branch}@{short} failed: {}",
                crate::run::err_message(&e)
            )];
        }
    };
    let accepted = match promote_accepted(s, slug, &repo, &commit, &branch, &short, actor) {
        Ok(ids) => ids,
        Err(e) => {
            return vec![format!(
                "records {slug}: accepting merged ADRs at {branch}@{short} failed: {}",
                crate::run::err_message(&e)
            )];
        }
    };
    let detail = json!({
        "project": slug,
        "tree": tree,
        "commit": commit,
        "branch": branch,
        "imported": summary.imported,
        "accepted": accepted,
    })
    .to_string();
    if let Err(e) = hive_bus::emit(s, actor, SYNCED, slug, detail.as_str()) {
        return vec![format!(
            "records {slug}: could not remember the imported tree: {}",
            crate::run::err_message(&e)
        )];
    }
    if summary.imported == 0 && accepted.is_empty() {
        return Vec::new();
    }
    let mut line = format!(
        "records {slug}: {DECISIONS} at {branch}@{short}: {} imported",
        summary.imported
    );
    if !accepted.is_empty() {
        line.push_str(&format!(", accepted {}", accepted.join(", ")));
    }
    vec![line]
}

/// The tree id of `docs/decisions` at `commit`; `None` when the folder is
/// absent or git cannot say.
fn decisions_tree(repo: &str, commit: &str) -> Option<String> {
    let out = hive_records::git_read(
        Path::new(repo),
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{commit}:{DECISIONS}"),
        ],
    )
    .ok()?;
    let tree = out.trim().to_string();
    (!tree.is_empty()).then_some(tree)
}

/// The tree the last `records.synced` event of `slug` recorded.
fn last_synced_tree(s: &hive_store::Store, slug: &str) -> Option<String> {
    let conn = s.conn().ok()?;
    let detail: String = conn
        .query_row(
            "SELECT detail FROM events WHERE action = ?1 AND subject = ?2 ORDER BY seq DESC LIMIT 1",
            [SYNCED, slug],
            |r| r.get(0),
        )
        .ok()?;
    let v: serde_json::Value = serde_json::from_str(&detail).ok()?;
    v.get("tree")?.as_str().map(str::to_string)
}

/// True for a committed status that says the decision is accepted
/// (`Accepted`, `Accepted: 2026-11-09`, `accepted (owner)`).
fn says_accepted(status: &str) -> bool {
    status
        .trim()
        .split([':', ' ', '('])
        .next()
        .is_some_and(|w| w.eq_ignore_ascii_case("Accepted"))
}

/// True for a store status that may move up to Accepted: anything later
/// (accepted, superseded, rejected, done) stays as it is.
fn promotable(status: &str) -> bool {
    let head = status.trim().split([':', ' ', '(']).next().unwrap_or("");
    head.is_empty() || head.eq_ignore_ascii_case("Draft") || head.eq_ignore_ascii_case("Proposed")
}

/// Marks accepted every ADR of `slug` whose file on `commit` says
/// `status: Accepted` while the store still holds it as draft or proposed;
/// returns the ids it moved.
fn promote_accepted(
    s: &hive_store::Store,
    slug: &str,
    repo: &str,
    commit: &str,
    branch: &str,
    short: &str,
    actor: &str,
) -> anyhow::Result<Vec<String>> {
    let root = Path::new(repo);
    let listing = hive_records::git_read(root, &["ls-tree", "-r", "-z", commit, "--", DECISIONS])?;
    let mut oids = BTreeMap::new();
    for entry in listing.split('\0') {
        let Some((meta, file)) = entry.split_once('\t') else {
            continue;
        };
        let fields: Vec<_> = meta.split_whitespace().collect();
        if fields.len() >= 3 && fields[1] == "blob" && file.ends_with(".md") {
            oids.insert(file.to_string(), fields[2].to_string());
        }
    }
    let blobs = hive_records::git_cat_blobs(root, &oids)?;
    // The committed ids whose file says Accepted, by path and by id.
    let mut wanted: Vec<(String, String)> = Vec::new();
    for (file, oid) in &oids {
        let Some(text) = blobs.get(oid) else {
            continue;
        };
        let Ok((front, _)) = hive_records::parse_frontmatter(text) else {
            continue;
        };
        if !says_accepted(&hive_records::text_value(front.get("status"))) {
            continue;
        }
        let mut id = hive_records::text_value(front.get("id"));
        if id.is_empty() {
            let base = Path::new(file)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let parts: Vec<_> = base.split('-').collect();
            if parts.len() > 1 && parts[0] == "ADR" {
                id = format!("ADR-{}", parts[1]);
            }
        }
        wanted.push((file.clone(), id));
    }
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    let _unlock = hive_records::lock_writes(s)?;
    let records = s.list_records(hive_store::RecordFilter {
        project: slug.to_string(),
        kind: "adr".into(),
        ..Default::default()
    })?;
    let mut moved = Vec::new();
    for (file, id) in wanted {
        let Some(rec) = records
            .iter()
            .find(|r| r.path == file)
            .or_else(|| records.iter().find(|r| !id.is_empty() && r.id == id))
        else {
            continue;
        };
        if !promotable(&rec.status) || moved.contains(&rec.id) {
            continue;
        }
        let (mut rec, rev) = s.get_record(&rec.id, 0)?;
        let Some(mut rev) = rev else {
            continue;
        };
        rec.status = "Accepted".into();
        rev.frontmatter.insert(
            "status".into(),
            serde_json::Value::String("Accepted".into()),
        );
        let note = format!("accepted: {file} merged to {branch} at {short} with status Accepted");
        s.put_record(rec.clone(), rev.frontmatter, &rev.body, actor, &note)?;
        moved.push(rec.id);
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::*;

    fn commit(dir: &str, path: &str, text: &str, message: &str) {
        let full = std::path::Path::new(dir).join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(&full, text).unwrap();
        for args in [vec!["add", "-A", "."], vec!["commit", "-q", "-m", message]] {
            let mut full = vec!["-C", dir];
            full.extend(args);
            hive_testutil::git(&full).unwrap();
        }
    }

    fn adr(id: &str, status: &str) -> String {
        format!(
            "---\nid: {id}\ntitle: Decision {id}\nstatus: {status}\ndate: 2026-11-09\nareas:\n  - runner\n---\n\n# {id}\n\nWe decide.\n"
        )
    }

    fn record_events(s: &hive_store::Store) -> i32 {
        count(
            s,
            "SELECT COUNT(*) AS n FROM events WHERE action LIKE 'record%'",
            &[],
        )
    }

    fn status_of(s: &hive_store::Store, id: &str) -> String {
        s.get_record(id, 0).unwrap().0.status
    }

    fn setup() -> (TestStore, String) {
        let s = new_store();
        let (_proj, work) = new_bare_repo(&s);
        hive_testutil::git(&["-C", &work, "config", "user.email", "t@example.com"]).unwrap();
        hive_testutil::git(&["-C", &work, "config", "user.name", "t"]).unwrap();
        (s, work)
    }

    #[test]
    fn tend_records_imports_a_merged_accepted_adr_once() {
        let _serial = crate::test_helpers::serial();
        let (s, work) = setup();
        // No decisions folder: nothing to do, nothing remembered.
        assert!(tend_records(&s, "proj", TEST_NOW, "daemon").is_empty());
        assert_eq!(record_events(&s), 0);

        commit(
            &work,
            "docs/decisions/ADR-071-merged.md",
            &adr("ADR-071", "Accepted"),
            "adr 71",
        );
        commit(
            &work,
            "docs/decisions/ADR-072-open.md",
            &adr("ADR-072", "Proposed"),
            "adr 72",
        );
        let lines = tend_records(&s, "proj", TEST_NOW, "daemon");
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("2 imported"), "{lines:?}");
        assert_eq!(status_of(&s, "ADR-071"), "Accepted");
        assert_eq!(status_of(&s, "ADR-072"), "Proposed");
        let after_first = record_events(&s);
        assert!(after_first > 0);

        // The second pass on the same tree imports nothing and writes no
        // record event.
        assert!(tend_records(&s, "proj", TEST_NOW, "daemon").is_empty());
        assert_eq!(record_events(&s), after_first);
    }

    #[test]
    fn tend_records_accepts_a_merged_status_change_and_never_downgrades() {
        let _serial = crate::test_helpers::serial();
        let (s, work) = setup();
        commit(
            &work,
            "docs/decisions/ADR-073-later.md",
            &adr("ADR-073", "Proposed"),
            "adr 73",
        );
        commit(
            &work,
            "docs/decisions/ADR-074-old.md",
            &adr("ADR-074", "Accepted"),
            "adr 74",
        );
        tend_records(&s, "proj", TEST_NOW, "daemon");
        assert_eq!(status_of(&s, "ADR-073"), "Proposed");
        // The owner supersedes ADR-074 in the store.
        s.set_record_status("ADR-074", "Superseded", hive_store::HUMAN_AGENT)
            .unwrap();

        // The owner merges ADR-073 as Accepted (and touches ADR-074, still
        // Accepted in the file).
        commit(
            &work,
            "docs/decisions/ADR-073-later.md",
            &adr("ADR-073", "Accepted"),
            "accept 73",
        );
        let lines = tend_records(&s, "proj", TEST_NOW, "daemon");
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("accepted ADR-073"), "{lines:?}");
        assert_eq!(status_of(&s, "ADR-073"), "Accepted");
        let (_, rev) = s.get_record("ADR-073", 0).unwrap();
        let rev = rev.unwrap();
        assert_eq!(rev.frontmatter["status"], json!("Accepted"));
        assert!(rev.note.contains("merged to main"), "{rev:?}");
        assert_eq!(status_of(&s, "ADR-074"), "Superseded");

        // A Proposed file never moves an accepted record back.
        commit(
            &work,
            "docs/decisions/ADR-073-later.md",
            &adr("ADR-073", "Proposed"),
            "revert 73",
        );
        assert!(tend_records(&s, "proj", TEST_NOW, "daemon").is_empty());
        assert_eq!(status_of(&s, "ADR-073"), "Accepted");
    }

    #[test]
    fn one_daemon_pass_brings_a_merged_adr_into_a_fresh_store() {
        let _serial = crate::test_helpers::serial();
        let (s, work) = setup();
        // The machine sweep walks the real $HOME runner folders; this pass
        // only needs the per-project tend steps.
        s.set_setting(hive_store::HUMAN_AGENT, "daemon.sweep", "off")
            .unwrap();
        commit(
            &work,
            "docs/decisions/ADR-070-web-first.md",
            &adr("ADR-070", "Accepted"),
            "adr 70",
        );
        let out = once_daemon(&s, crate::daemon::DaemonOpts::default());
        assert!(
            out.contains("records proj: docs/decisions at main@"),
            "{out}"
        );
        let listed = hive_records::list(
            &s,
            hive_store::RecordFilter {
                project: "proj".into(),
                kind: "adr".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(listed.len(), 1, "{listed:?}");
        assert_eq!(
            (listed[0].id.as_str(), listed[0].status.as_str()),
            ("ADR-070", "Accepted")
        );
        assert_eq!(
            count(
                &s,
                "SELECT COUNT(*) AS n FROM events WHERE action = 'records.synced' AND subject = 'proj'",
                &[]
            ),
            1
        );
    }

    #[test]
    fn statuses_read_as_accepted_or_promotable() {
        assert!(says_accepted("Accepted"));
        assert!(says_accepted("accepted: 2026-11-09"));
        assert!(!says_accepted("Proposed"));
        assert!(!says_accepted(""));
        assert!(promotable(""));
        assert!(promotable("Proposed"));
        assert!(promotable("draft"));
        assert!(!promotable("Accepted"));
        assert!(!promotable("Superseded by ADR-080"));
        assert!(!promotable("Rejected"));
    }
}
