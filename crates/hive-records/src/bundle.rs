use crate::records::{Frontmatter, apply_columns, input_error, text_value};
use anyhow::{Result, anyhow};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

pub const BUNDLE_JSON: &str = include_str!("../seed/bundle.json");
/// The kinds `hive seed export` bundles and `hive seed apply` seeds: the
/// framework's own records, and the agent-pack kinds (a role, an agent, a
/// brief, a kind) a pack ships through the same bundle.
pub const FRAMEWORK_KINDS: &[&str] = &[
    "instruction",
    "guide",
    "skill",
    "workflow",
    "prompt",
    "template",
    "architecture",
    "role",
    "agent",
    "brief",
    "kind",
];
/// The bundle's path inside the Hive checkout; the crate embeds it from
/// here, so `hive seed export` refreshes this file.
pub const BUNDLE_FILE: &str = "crates/hive-records/seed/bundle.json";
pub const BUNDLE_VERSION: i64 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BundleRecord {
    pub id: String,
    pub kind: String,
    pub slug: String,
    pub frontmatter: Frontmatter,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bundle {
    pub version: i64,
    pub records: Vec<BundleRecord>,
}

pub fn framework_bundle() -> Result<Bundle> {
    decode_bundle(BUNDLE_JSON.as_bytes())
}

pub fn decode_bundle(data: &[u8]) -> Result<Bundle> {
    let b: Bundle = serde_json::from_slice(data).map_err(|e| anyhow!("framework bundle: {e}"))?;
    if b.version != BUNDLE_VERSION {
        return Err(anyhow!(
            "framework bundle: version {}, this hive reads {}",
            b.version,
            BUNDLE_VERSION
        ));
    }
    for r in &b.records {
        if r.id.is_empty() || r.kind.is_empty() || r.slug.is_empty() {
            return Err(anyhow!("framework bundle: a record lacks id, kind or slug"));
        }
        if !framework_kind(&r.kind) {
            return Err(anyhow!(
                "framework bundle: {} has kind {:?}, not one of {}",
                r.id,
                r.kind,
                FRAMEWORK_KINDS.join(", ")
            ));
        }
    }
    Ok(b)
}

pub fn framework_kind(kind: &str) -> bool {
    FRAMEWORK_KINDS.contains(&kind)
}

pub fn seeded_from() -> String {
    let mut rev = hive_cli::revision();
    if rev.is_empty() {
        return "hive@dev".into();
    }
    if rev.len() > 12 {
        rev.truncate(12);
    }
    format!("hive@{rev}")
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct EnsureResult {
    pub written: Vec<String>,
    pub kept: i64,
    pub from: String,
    /// Records the store held at an earlier shipped text, revised to the
    /// bundle's (`SHIPPED_BEFORE`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub revised: Vec<String>,
}

/// Bodies the framework bundle shipped before, by record id and the FNV-1a
/// hash of the body (`body_hash`). A store that seeded one of them and was
/// not edited since gets the bundle's current text as a new revision; a
/// text edited in the store matches none and is kept. Without this a store
/// keeps the first text it seeded, and a seed export from it writes the old
/// text back (WF-hive was changed in the bundle alone on 26 November).
pub const SHIPPED_BEFORE: &[(&str, u64)] = &[
    // Before fc998ab9 (TASK-135): no join job, no merge-only rebase brief.
    ("WF-hive", 0x351f_a2a6_9dfb_2898),
    // fc998ab9 (TASK-135): before a merge kept its branch on a store it
    // could not read and a join's brief left out a carried branch
    // (TASK-139).
    ("WF-hive", 0x4f6f_ca26_f0bb_ac62),
];

/// FNV-1a over the body's bytes: stable across builds and platforms, so a
/// text shipped once can be named in `SHIPPED_BEFORE`.
pub fn body_hash(body: &str) -> u64 {
    body.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

const REVISED_NOTE: &str =
    "revised from the framework bundle: the store held an earlier shipped text (ADR-038)";

pub fn ensure_framework(s: &hive_store::Store, actor: &str) -> Result<EnsureResult> {
    let mut result = EnsureResult {
        written: vec![],
        kept: 0,
        from: seeded_from(),
        revised: vec![],
    };
    let b = framework_bundle()?;
    let actor = if actor.is_empty() {
        hive_store::HUMAN_AGENT
    } else {
        actor
    };
    let mut missing = Vec::new();
    let mut outdated = Vec::new();
    for r in b.records {
        match s.get_record(&r.id, 0) {
            Ok((_, rev)) => {
                if shipped_before(&r, rev.as_ref()) {
                    outdated.push(r);
                } else {
                    result.kept += 1;
                }
            }
            Err(e) if e.downcast_ref::<hive_store::ErrNoRecord>().is_some() => missing.push(r),
            Err(e) => return Err(e),
        }
    }
    if missing.is_empty() && outdated.is_empty() {
        return Ok(result);
    }
    let _unlock = crate::lock::lock_writes(s)?;
    for r in outdated {
        // Read again under the lock: an edit in between keeps its text.
        let (mut rec, rev) = s.get_record(&r.id, 0)?;
        if !shipped_before(&r, rev.as_ref()) {
            result.kept += 1;
            continue;
        }
        apply_columns(&mut rec, &r.frontmatter, &r.body).map_err(|e| anyhow!("{}: {e}", r.id))?;
        s.put_record(rec, r.frontmatter, &r.body, actor, REVISED_NOTE)?;
        result.kept += 1;
        result.revised.push(r.id);
    }
    if !result.revised.is_empty() {
        s.event(
            actor,
            "records.framework.revised",
            hive_store::COMMONS_SLUG,
            hive_store::Detail::json(
                serde_json::json!({"revised": result.revised, "from": result.from}),
            )?,
        )?;
    }
    for r in missing {
        match s.get_record(&r.id, 0) {
            Ok(_) => {
                result.kept += 1;
                continue;
            }
            Err(e) if e.downcast_ref::<hive_store::ErrNoRecord>().is_some() => {}
            Err(e) => return Err(e),
        }
        let mut rec = hive_store::Record {
            id: r.id.clone(),
            project: hive_store::COMMONS_SLUG.into(),
            kind: r.kind.clone(),
            slug: r.slug.clone(),
            origin: format!("seed:{}", result.from),
            ..hive_store::Record::default()
        };
        apply_columns(&mut rec, &r.frontmatter, &r.body).map_err(|e| anyhow!("{}: {e}", r.id))?;
        s.put_record(
            rec,
            r.frontmatter,
            &r.body,
            actor,
            "seeded from the framework bundle (ADR-038)",
        )?;
        result.written.push(r.id);
    }
    if !result.written.is_empty() {
        s.event(
            actor,
            "records.framework.seeded",
            hive_store::COMMONS_SLUG,
            hive_store::Detail::json(
                serde_json::json!({"written": result.written.len(), "from": result.from}),
            )?,
        )?;
    }
    Ok(result)
}

/// Whether the store's current revision of a bundle record is a text the
/// bundle shipped before and nobody edited: the bundle's text replaces it.
fn shipped_before(r: &BundleRecord, current: Option<&hive_store::Revision>) -> bool {
    let Some(current) = current else {
        return false;
    };
    if current.body == r.body {
        return false;
    }
    let hash = body_hash(&current.body);
    SHIPPED_BEFORE
        .iter()
        .any(|(id, h)| *id == r.id && *h == hash)
}

pub fn secret_shaped(text: &str) -> String {
    let patterns = [
        r"\bgh[pousr]_[A-Za-z0-9]{20,}",
        r"\bgithub_pat_[A-Za-z0-9_]{20,}",
        r"\bsk-[A-Za-z0-9_-]{20,}",
        r"\bAKIA[0-9A-Z]{16}\b",
        r"\bxox[abprs]-[A-Za-z0-9-]{10,}",
        r"\bAIza[0-9A-Za-z_-]{35}",
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----",
        r"\beyJ[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{10,}",
    ];
    for pat in patterns {
        if let Some(m) = Regex::new(pat).unwrap().find(text) {
            let mut s = m.as_str().to_string();
            if s.len() > 12 {
                s.truncate(12);
                s.push('…');
            }
            return s;
        }
    }
    String::new()
}

pub fn retired_status(status: &str) -> bool {
    matches!(
        status.trim().to_ascii_lowercase().as_str(),
        "deprecated" | "retired" | "superseded"
    )
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct BundleSummary {
    pub path: String,
    pub records: usize,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<String>,
    pub unchanged: usize,
    pub skipped: Vec<String>,
}

impl BundleSummary {
    pub fn same(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}

pub fn build_bundle(s: &hive_store::Store) -> Result<(Bundle, Vec<String>)> {
    let brain = s.brain()?.ok_or_else(|| {
        input_error(
            "Cannot build the seed bundle",
            "no active project is the brain; the framework's records are the brain's",
            "hive init PATH --brain, then hive seed export",
        )
    })?;
    let mut rows = s.list_records(hive_store::RecordFilter {
        project: brain.slug,
        ..hive_store::RecordFilter::default()
    })?;
    rows.extend(s.list_records(hive_store::RecordFilter {
        project: hive_store::COMMONS_SLUG.into(),
        ..hive_store::RecordFilter::default()
    })?);
    let mut b = Bundle {
        version: BUNDLE_VERSION,
        records: vec![],
    };
    let mut skipped = Vec::new();
    for row in rows {
        if !framework_kind(&row.kind) {
            continue;
        }
        if retired_status(&row.status) {
            skipped.push(format!("{} ({})", row.id, row.status));
            continue;
        }
        let (_, rev) = s.get_record(&row.id, 0)?;
        let rev = rev.expect("current revision exists");
        let mut front = Frontmatter::new();
        for (k, v) in rev.frontmatter {
            if k != "seeded_from" && k != "seeded_by" {
                front.insert(k, v);
            }
        }
        let shape = secret_shaped(&rev.body);
        if !shape.is_empty() {
            return Err(input_error(
                &format!("Refusing to bundle {}", row.id),
                &format!("its body carries a secret-shaped value ({shape})"),
                &format!(
                    "remove the value with hive record amend {}, then hive seed export",
                    row.id
                ),
            ));
        }
        for (key, value) in &front {
            let shape = secret_shaped(&text_value(Some(value)));
            if !shape.is_empty() {
                return Err(input_error(
                    &format!("Refusing to bundle {}", row.id),
                    &format!("frontmatter {key} carries a secret-shaped value ({shape})"),
                    &format!(
                        "remove the value with hive record amend {}, then hive seed export",
                        row.id
                    ),
                ));
            }
        }
        b.records.push(BundleRecord {
            id: row.id,
            kind: row.kind,
            slug: row.slug,
            frontmatter: front,
            body: rev.body,
        });
    }
    sort_bundle(&mut b);
    Ok((b, skipped))
}

pub fn sort_bundle(b: &mut Bundle) {
    let order: BTreeMap<&str, usize> = FRAMEWORK_KINDS
        .iter()
        .enumerate()
        .map(|(i, k)| (*k, i))
        .collect();
    b.records.sort_by(|a, b| {
        order
            .get(a.kind.as_str())
            .cmp(&order.get(b.kind.as_str()))
            .then_with(|| a.id.cmp(&b.id))
    });
}

pub fn encode_bundle(b: &Bundle) -> Result<Vec<u8>> {
    let mut data = hive_store::gojson::to_vec_pretty_no_html(b)?;
    data.push(b'\n');
    Ok(data)
}

pub fn bundle_path(s: &hive_store::Store, out: &str) -> Result<PathBuf> {
    if !out.trim().is_empty() {
        return Ok(PathBuf::from(out));
    }
    let brain = s.brain()?;
    let Some(brain) = brain else {
        return Err(input_error(
            "Cannot place the seed bundle",
            "no brain checkout to write it into",
            "hive seed export --out PATH",
        ));
    };
    if brain.path.is_empty() {
        return Err(input_error(
            "Cannot place the seed bundle",
            "no brain checkout to write it into",
            "hive seed export --out PATH",
        ));
    }
    Ok(Path::new(&brain.path).join(BUNDLE_FILE))
}

pub fn export_bundle(s: &hive_store::Store, path: impl AsRef<Path>) -> Result<BundleSummary> {
    let path = path.as_ref();
    let mut summary = BundleSummary {
        path: path.display().to_string(),
        added: vec![],
        removed: vec![],
        changed: vec![],
        skipped: vec![],
        ..BundleSummary::default()
    };
    let (b, skipped) = build_bundle(s)?;
    summary.records = b.records.len();
    summary.skipped = skipped;
    let data = encode_bundle(&b)?;
    match fs::read(path) {
        Ok(old) => match decode_bundle(&old) {
            Ok(prev) => diff_bundles(&prev, &b, &mut summary),
            Err(_) => summary.added.extend(b.records.iter().map(|r| r.id.clone())),
        },
        Err(_) => summary.added.extend(b.records.iter().map(|r| r.id.clone())),
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, data)?;
    Ok(summary)
}

pub fn diff_bundles(prev: &Bundle, next: &Bundle, summary: &mut BundleSummary) {
    let before: BTreeMap<_, _> = prev.records.iter().map(|r| (&r.id, r)).collect();
    let mut after = BTreeSet::new();
    for r in &next.records {
        after.insert(&r.id);
        match before.get(&r.id) {
            None => summary.added.push(r.id.clone()),
            Some(old) if same_bundle_record(old, r) => summary.unchanged += 1,
            Some(_) => summary.changed.push(r.id.clone()),
        }
    }
    for r in &prev.records {
        if !after.contains(&r.id) {
            summary.removed.push(r.id.clone());
        }
    }
    summary.removed.sort();
}

fn same_bundle_record(a: &BundleRecord, b: &BundleRecord) -> bool {
    a.kind == b.kind
        && a.slug == b.slug
        && a.body == b.body
        && hive_store::gojson::to_vec(&a.frontmatter).ok()
            == hive_store::gojson::to_vec(&b.frontmatter).ok()
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct BundleStatus {
    pub records: usize,
    pub present: usize,
    pub missing: Vec<String>,
    pub stale: Vec<String>,
    pub unshipped: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub brain: String,
}

pub fn framework_status(s: &hive_store::Store) -> Result<BundleStatus> {
    let mut status = BundleStatus {
        missing: vec![],
        stale: vec![],
        unshipped: vec![],
        ..BundleStatus::default()
    };
    let b = framework_bundle()?;
    status.records = b.records.len();
    let mut shipped = BTreeMap::new();
    for r in b.records {
        shipped.insert(r.id.clone(), r.clone());
        match s.get_record(&r.id, 0) {
            Ok(_) => status.present += 1,
            Err(e) if e.downcast_ref::<hive_store::ErrNoRecord>().is_some() => {
                status.missing.push(r.id)
            }
            Err(e) => return Err(e),
        }
    }
    let Some(brain) = s.brain()? else {
        return Ok(status);
    };
    status.brain = brain.slug;
    let (current, _) = build_bundle(s)?;
    for r in current.records {
        match shipped.get(&r.id) {
            None => status.unshipped.push(r.id),
            Some(old) if !same_bundle_record(old, &r) => status.stale.push(r.id),
            Some(_) => {}
        }
    }
    Ok(status)
}

pub use build_bundle as BuildBundle;
pub use bundle_path as BundlePath;
pub use encode_bundle as encodeBundle;
pub use ensure_framework as EnsureFramework;
pub use export_bundle as ExportBundle;
pub use framework_bundle as FrameworkBundle;
pub use framework_status as FrameworkStatus;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct TestStore {
        _home: tempfile::TempDir,
        store: hive_store::Store,
    }
    impl std::ops::Deref for TestStore {
        type Target = hive_store::Store;
        fn deref(&self) -> &Self::Target {
            &self.store
        }
    }
    fn temp_store() -> TestStore {
        let home = tempfile::tempdir().unwrap();
        hive_testutil::neutralize_runner_environment();
        let store = hive_store::Store::open(hive_store::Options {
            home: home.path().display().to_string(),
            create: true,
            now: "2026-09-06T21:00:00Z".into(),
            ..hive_store::Options::default()
        })
        .unwrap();
        TestStore { _home: home, store }
    }

    fn add_project(s: &hive_store::Store, slug: &str, path: &std::path::Path, brain: bool) {
        s.conn()
            .unwrap()
            .execute(
                "INSERT INTO projects (slug,name,path,brain,added_at) VALUES (?,?,?,?,?)",
                rusqlite::params![slug, slug, path.display().to_string(), brain, s.now()],
            )
            .unwrap();
    }

    #[test]
    fn framework_bundle_carries_the_framework() {
        let _serial = crate::test_helpers::serial();
        let b = framework_bundle().unwrap();
        assert!(
            b.version == BUNDLE_VERSION && b.records.len() >= 60,
            "bundle version {} with {} records",
            b.version,
            b.records.len()
        );
        let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
        let mut ids: BTreeMap<String, BundleRecord> = BTreeMap::new();
        for (i, r) in b.records.iter().enumerate() {
            *kinds.entry(r.kind.clone()).or_default() += 1;
            assert!(!ids.contains_key(&r.id), "{} occurs twice", r.id);
            ids.insert(r.id.clone(), r.clone());
            assert!(
                framework_kind(&r.kind) && !r.slug.is_empty() && !r.body.trim().is_empty(),
                "record {i} is not a framework record: {}",
                r.id
            );
            for key in ["seeded_from", "seeded_by"] {
                assert!(
                    !r.frontmatter.contains_key(key),
                    "{} carries {key}; the bundle is the source, not a copy",
                    r.id
                );
            }
            let shape = secret_shaped(&r.body);
            assert!(shape.is_empty(), "{} carries a {shape}", r.id);
            assert!(
                !retired_status(&crate::records::text_value(r.frontmatter.get("status"))),
                "{} is {:?} and still bundled",
                r.id,
                r.frontmatter.get("status")
            );
            assert!(
                !r.body.contains("/Users/") && !r.body.contains("/home/"),
                "{} names a home directory",
                r.id
            );
        }
        // The pack kinds (role, agent, brief, kind) ship empty until a pack
        // lands; the framework's own kinds are always there.
        for kind in [
            "instruction",
            "guide",
            "skill",
            "workflow",
            "prompt",
            "template",
            "architecture",
        ] {
            assert!(
                kinds.get(kind).copied().unwrap_or(0) > 0,
                "no {kind} record in the bundle"
            );
        }
        for id in [
            "INSTR-agents",
            "GUIDE-new-repo",
            "TPL-evolution-constitution",
            "TPL-evolution-evaluators",
            "TPL-adr",
            "TPL-plan",
            "WF-hive",
            "ARCH-overview",
        ] {
            assert!(ids.contains_key(id), "bundle lacks {id}");
        }
        let mut sorted = b.clone();
        sort_bundle(&mut sorted);
        for i in 0..b.records.len() {
            assert_eq!(
                b.records[i].id, sorted.records[i].id,
                "bundle is not in export order at {i}: {} vs {}",
                b.records[i].id, sorted.records[i].id
            );
        }
        // The file on disk is the bundle re-encoded: an export that changes nothing rewrites nothing.
        let encoded = encode_bundle(&b).unwrap();
        assert!(
            encoded == BUNDLE_JSON.as_bytes(),
            "seed/bundle.json is not in encodeBundle's form; run hive seed export"
        );
    }

    /// WF-hive as fc998ab9 shipped it: the current text without what
    /// TASK-139 added.
    fn wf_hive_at_fc998ab9() -> String {
        let b = framework_bundle().unwrap();
        let body = b
            .records
            .iter()
            .find(|r| r.id == "WF-hive")
            .unwrap()
            .body
            .clone();
        let old = body
            .replace(
                ", and while the\nstore cannot say which are open (`branch_kept` names why; a later pass\ndeletes it)",
                "",
            )
            .replace(
                "merged again, nor named in the join's brief; a second conflict of either\nkind is held for the owner. A stacked pull request rebuilt after the one\nbelow closed unmerged keeps the merge commits of a join on it.\n",
                "merged again; a second conflict of either kind is held for the owner.\n",
            );
        assert_ne!(old, body, "the TASK-139 text is in the bundle");
        old
    }

    /// A store that seeded WF-hive before the bundle changed gets the new
    /// text as a revision, once; a text edited in the store is kept
    /// (TASK-139: the bundle alone changed, the stores kept revision 1).
    #[test]
    fn a_seeded_record_at_an_earlier_shipped_text_is_revised_and_an_edited_one_kept() {
        let _serial = crate::test_helpers::serial();
        let old = wf_hive_at_fc998ab9();
        assert_eq!(body_hash(&old), 0x4f6f_ca26_f0bb_ac62, "the fc998ab9 text");
        let b = framework_bundle().unwrap();
        let shipped = b
            .records
            .iter()
            .find(|r| r.id == "WF-hive")
            .unwrap()
            .clone();
        let s = temp_store();
        ensure_framework(&s, "alice").unwrap();
        let (rec, rev) = s.get_record("WF-hive", 0).unwrap();
        s.put_record(
            rec,
            rev.unwrap().frontmatter,
            &old,
            "alice",
            "seeded from the framework bundle (ADR-038)",
        )
        .unwrap();
        let (_, rev) = s.get_record("WF-hive", 0).unwrap();
        assert_eq!(rev.unwrap().body, old);

        let result = ensure_framework(&s, "daemon").unwrap();
        assert_eq!(result.revised, ["WF-hive"]);
        assert!(result.written.is_empty());
        assert_eq!(result.kept as usize, b.records.len());
        let (rec, rev) = s.get_record("WF-hive", 0).unwrap();
        let rev = rev.unwrap();
        assert_eq!(rev.body, shipped.body);
        assert_eq!(rev.author, "daemon");
        assert_eq!(rev.note, REVISED_NOTE);
        assert_eq!(rec.current_rev, 3);
        assert_eq!(s.revisions("WF-hive").unwrap().len(), 3);

        // Once: the store holds the bundle's text now.
        let again = ensure_framework(&s, "daemon").unwrap();
        assert!(again.revised.is_empty());
        assert_eq!(s.revisions("WF-hive").unwrap().len(), 3);

        // An owner's edit is a text the bundle never shipped: kept.
        let (rec, rev) = s.get_record("WF-hive", 0).unwrap();
        let mine = format!("{old}\nMy own line.\n");
        s.put_record(rec, rev.unwrap().frontmatter, &mine, "alice", "mine")
            .unwrap();
        let kept = ensure_framework(&s, "daemon").unwrap();
        assert!(kept.revised.is_empty());
        assert_eq!(s.get_record("WF-hive", 0).unwrap().1.unwrap().body, mine);
    }

    #[test]
    fn ensure_framework_seeds_a_fresh_store_once_and_keeps_what_is_there() {
        let _serial = crate::test_helpers::serial();
        let b = framework_bundle().unwrap();
        let s = temp_store();
        let mine = hive_store::Record {
            id: "TPL-adr".into(),
            project: "app".into(),
            kind: "template".into(),
            slug: "adr".into(),
            title: "My ADR template".into(),
            origin: "store".into(),
            ..hive_store::Record::default()
        };
        s.put_record(
            mine,
            [
                ("id".into(), json!("TPL-adr")),
                ("title".into(), json!("My ADR template")),
            ]
            .into(),
            "# Mine\n",
            "alice",
            "",
        )
        .unwrap();
        let result = ensure_framework(&s, "alice").unwrap();
        assert_eq!(result.written.len(), b.records.len() - 1);
        assert_eq!(result.kept, 1);
        assert!(result.from.starts_with("hive@"));
        let (rec, rev) = crate::records::get(&s, "INSTR-agents", 0).unwrap();
        assert_eq!(rec.project, hive_store::COMMONS_SLUG);
        assert_eq!(rec.origin, format!("seed:{}", result.from));
        assert_eq!(rev.note, "seeded from the framework bundle (ADR-038)");
        let again = ensure_framework(&s, "").unwrap();
        assert!(again.written.is_empty());
        assert_eq!(again.kept as usize, b.records.len());
        let status = framework_status(&s).unwrap();
        assert_eq!(status.present, b.records.len());
        assert!(status.missing.is_empty());
    }

    #[test]
    fn framework_records_serve_the_loop_and_the_guide_without_a_brain() {
        let _serial = crate::test_helpers::serial();
        let s = temp_store();
        let project_dir = tempfile::tempdir().unwrap();
        add_project(&s, "lamplitlabs", project_dir.path(), false);
        s.conn()
            .unwrap()
            .execute(
                "UPDATE projects SET name = 'Lamplit Labs' WHERE slug = 'lamplitlabs'",
                [],
            )
            .unwrap();
        assert!(crate::seed::seed_loop_records(&s, "lamplitlabs", "alice").is_err());
        ensure_framework(&s, "alice").unwrap();
        let summary = crate::seed::seed_loop_records(&s, "lamplitlabs", "alice").unwrap();
        assert_eq!(summary.constitution.action, "copied");
        assert_eq!(summary.constitution.source, "TPL-evolution-constitution@r1");
        assert_eq!(summary.evaluators.action, "copied");
        assert_eq!(summary.evaluators.source, "TPL-evolution-evaluators@r1");
        let (_, rev) = crate::records::get(&s, "EVO-constitution-lamplitlabs", 0).unwrap();
        assert!(!rev.body.contains("<Project>"));
        assert!(rev.body.contains("Lamplit Labs"));
        assert_eq!(
            rev.frontmatter["seeded_from"],
            serde_json::json!("commons/TPL-evolution-constitution@r1")
        );
        let out = hive_cli::Buffer::new();
        let err = hive_cli::Buffer::new();
        let mut c = hive_cli::Context {
            store: Some(s.clone()),
            home: s.home.clone(),
            out: Box::new(out.clone()),
            err: Box::new(err),
            json: false,
            quiet: false,
            color: false,
            agent: String::new(),
            project: "lamplitlabs".into(),
            session: String::new(),
            now: String::new(),
            args: vec![],
        };
        crate::register::run_guide(&mut c, &hive_cli::FlagSet::new(""), &[]).unwrap();
        assert!(out.string().starts_with("# Hive"));
        assert!(!out.string().contains("No instruction records"));
    }

    #[test]
    fn export_bundle_writes_the_framework_records_and_refuses_secrets() {
        let _serial = crate::test_helpers::serial();
        let s = temp_store();
        let brain_dir = tempfile::tempdir().unwrap();
        add_project(&s, "brain", brain_dir.path(), true);
        let put = |id: &str, project: &str, kind: &str, slug: &str, status: &str, body: &str| {
            let front = [
                ("id".into(), json!(id)),
                ("title".into(), json!(slug.to_ascii_uppercase())),
                ("status".into(), json!(status)),
            ]
            .into();
            let rec = hive_store::Record {
                id: id.into(),
                project: project.into(),
                kind: kind.into(),
                slug: slug.into(),
                title: slug.to_ascii_uppercase(),
                status: status.into(),
                origin: "store".into(),
                ..hive_store::Record::default()
            };
            s.put_record(rec, front, body, "tester", "").unwrap();
        };
        put(
            "INSTR-agents",
            "brain",
            "instruction",
            "agents",
            "",
            "# Agents\n",
        );
        put("TPL-adr", "brain", "template", "adr", "Proposed", "# ADR\n");
        put(
            "PROMPT-old",
            "brain",
            "prompt",
            "old",
            "Deprecated",
            "# Old\n",
        );
        put(
            "ADR-001",
            "brain",
            "adr",
            "001-x",
            "Accepted",
            "# Not framework\n",
        );
        put(
            "WF-shared",
            hive_store::COMMONS_SLUG,
            "workflow",
            "shared",
            "",
            "# Seeded first\n",
        );
        let (b, skipped) = build_bundle(&s).unwrap();
        assert_eq!(
            b.records.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["INSTR-agents", "WF-shared", "TPL-adr"]
        );
        assert!(skipped[0].starts_with("PROMPT-old"));
        let path = bundle_path(&s, "").unwrap();
        assert_eq!(path, brain_dir.path().join(BUNDLE_FILE));
        let summary = export_bundle(&s, &path).unwrap();
        assert_eq!(summary.records, 3);
        assert_eq!(summary.added.len(), 3);
        assert!(!summary.same());
        let again = export_bundle(&s, &path).unwrap();
        assert!(again.same());
        assert_eq!(again.unchanged, 3);
        put(
            "TPL-adr",
            "brain",
            "template",
            "adr",
            "Proposed",
            "# ADR\n\nToken: AKIAABCDEFGHIJKLMNOP\n",
        );
        assert!(
            build_bundle(&s)
                .unwrap_err()
                .to_string()
                .contains("secret-shaped")
        );
        assert!(retired_status("Deprecated"));
        assert!(!retired_status("Accepted"));
    }

    #[test]
    fn pack_kinds_travel_in_the_bundle() {
        let _serial = crate::test_helpers::serial();
        let s = temp_store();
        let brain_dir = tempfile::tempdir().unwrap();
        add_project(&s, "brain", brain_dir.path(), true);
        let role_body = "---\ntitle: Content lead\nevery: 1w\n---\n## Your concern: content\n";
        for (id, kind, slug, body) in [
            ("ROLE-content", "role", "content", role_body),
            (
                "AGENT-reviewer",
                "agent",
                "reviewer",
                "---\nname: reviewer\ndescription: reviews\n---\nRead the diff.\n",
            ),
            ("BRIEF-weekly", "brief", "weekly", "Once a week.\n"),
            ("KIND-bench", "kind", "bench", "A benchmark job.\n"),
            ("ARCH-overview", "architecture", "overview", "# Overview\n"),
        ] {
            let rec = hive_store::Record {
                id: id.into(),
                project: "brain".into(),
                kind: kind.into(),
                slug: slug.into(),
                title: slug.into(),
                origin: "store".into(),
                ..hive_store::Record::default()
            };
            s.put_record(rec, Frontmatter::new(), body, "tester", "")
                .unwrap();
        }
        let (b, skipped) = build_bundle(&s).unwrap();
        assert!(skipped.is_empty(), "{skipped:?}");
        assert_eq!(
            b.records.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec![
                "ARCH-overview",
                "ROLE-content",
                "AGENT-reviewer",
                "BRIEF-weekly",
                "KIND-bench"
            ],
            "pack kinds follow the framework's, in FRAMEWORK_KINDS order"
        );
        let back = decode_bundle(&encode_bundle(&b).unwrap()).unwrap();
        assert_eq!(back, b);
        assert_eq!(back.records[1].body, role_body);
    }
}
