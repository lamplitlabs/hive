//! The agent-program population of ADR-049 (PLAN-050 step 1): variant rows,
//! the per-project incumbent pointer and the `land.reverted` event.
//!
//! Everything here is inert while `evolve.mode=fixtures`, the default: the
//! tables stay empty, no run is stamped with a variant and no scan writes a
//! revert. The later steps (revert scan, registration, resolver, router,
//! outcome recompute, promotion) read and write through these helpers.

use crate::settings::{BASE_VARIANT, incumbent_setting};
use crate::store::{Store, input_err};
use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

/// The event the revert scan writes, once per landed sha, when a commit on
/// the default branch reverts it or it is no longer reachable from there.
/// Its subject is the landed sha; its detail names the project, the job that
/// landed it and what reverted it.
pub const LAND_REVERTED: &str = "land.reverted";

/// `evolve.mode`: the measured fixture loop (default) or the agent-program
/// variants loop.
pub const EVOLVE_MODE: &str = "evolve.mode";
pub const MODE_FIXTURES: &str = "fixtures";
pub const MODE_VARIANTS: &str = "variants";

/// The states a variant moves through: registered, replaying, canarying on
/// real jobs, promoted to an incumbent, or retired (kept in the archive).
pub const VARIANT_STATES: &[&str] = &["proposed", "replay", "canary", "promoted", "retired"];

/// One row of `variants`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Variant {
    pub id: String,
    pub project_scope: String,
    pub kinds_scope: String,
    pub parent: String,
    pub hypothesis: String,
    pub state: String,
    pub proposer_job: String,
    pub evidence: String,
    pub dir: String,
    pub created_at: String,
    pub retired_at: String,
    pub promoted_at: String,
}

/// What a landed sha's revert is, for [`Store::land_reverted`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revert {
    pub project: String,
    /// The landed commit that was reverted.
    pub sha: String,
    /// The job whose landing it was ('' when unknown).
    pub job: String,
    /// The commit whose message says `This reverts commit <sha>`; empty when
    /// the sha is simply no longer reachable from the default branch.
    pub by: String,
    /// `revert` or `unreachable`.
    pub how: String,
}

/// The settings a variant's `settings.toml` may set (ADR-049 "The genome"):
/// the model and reasoning classes and the harness knobs that change how an
/// agent thinks; the planner's brief inputs come in as the `lead.` and
/// `plan.` families ([`VARIANT_SETTING_PREFIXES`]).
pub const VARIANT_SETTINGS: &[&str] = &[
    "worker_model",
    "light_model",
    "worker_reasoning",
    "light_reasoning",
    "harness.compact_at",
    "harness.compact_model",
    "harness.max_steps",
    "harness.reasoning",
    "harness.subagents",
    "harness.skills",
    "harness.memory",
];
/// Setting families a variant may set whole: the planner's brief inputs.
pub const VARIANT_SETTING_PREFIXES: &[&str] = &["lead.", "plan."];

/// The constitution's immutable set as settings: a variant changes how an
/// agent thinks and which model it uses, never what it is allowed to do.
/// A name equal to an entry, or under an entry ending in `.`, or (for
/// `landing`) any `landing*` name, is refused at registration.
pub const VARIANT_IMMUTABLE_SETTINGS: &[&str] = &[
    "owner.",
    "stop",
    "landing",
    "push.",
    "git.",
    "compute.",
    "harness.deny_tools",
    "harness.allow_tools",
    "evolve.",
];

/// Why a variant may not set `name`, or None when the allowlist takes it.
/// The immutable set is checked first so its refusal names it as such.
pub fn variant_setting_refusal(name: &str) -> Option<String> {
    let n = name.trim();
    let immutable = VARIANT_IMMUTABLE_SETTINGS.iter().any(|entry| {
        if let Some(prefix) = entry.strip_suffix('.') {
            n == prefix || n.starts_with(entry)
        } else if *entry == "landing" {
            n.starts_with("landing")
        } else {
            n == *entry
        }
    });
    if immutable {
        return Some(format!(
            "{n} is in the immutable set (owner.*, stop, landing*, push.*, compute.*, harness.deny_tools, harness.allow_tools, evolve.*): a variant changes how an agent thinks, never what it may do"
        ));
    }
    let allowed =
        VARIANT_SETTINGS.contains(&n) || VARIANT_SETTING_PREFIXES.iter().any(|p| n.starts_with(p));
    if !allowed {
        return Some(format!(
            "{n} is not on the variant allowlist ({}, lead.*, plan.*)",
            VARIANT_SETTINGS.join(", ")
        ));
    }
    None
}

/// What [`Store::insert_variant`] registers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewVariant {
    pub id: String,
    /// Islands (project slugs) the variant may canary on; empty: every one.
    pub projects: Vec<String>,
    /// Job kinds it may run; empty: the phase-1 trial kinds.
    pub kinds: Vec<String>,
    pub parent: String,
    pub hypothesis: String,
    pub proposer_job: String,
    pub evidence: Vec<String>,
    /// Where the genome lives (`~/.pulse/variants/<id>`).
    pub dir: String,
}

/// The event registration, retirement and promotion write; the subject is
/// the variant id.
pub const VARIANT_REGISTERED: &str = "evolve.registered";
pub const VARIANT_RETIRED: &str = "evolve.retired";
pub const VARIANT_PROMOTED: &str = "evolve.promoted";

fn scan_variant(r: &rusqlite::Row<'_>) -> rusqlite::Result<Variant> {
    Ok(Variant {
        id: r.get(0)?,
        project_scope: r.get(1)?,
        kinds_scope: r.get(2)?,
        parent: r.get(3)?,
        hypothesis: r.get(4)?,
        state: r.get(5)?,
        proposer_job: r.get(6)?,
        evidence: r.get(7)?,
        dir: r.get(8)?,
        created_at: r.get(9)?,
        retired_at: r.get(10)?,
        promoted_at: r.get(11)?,
    })
}

const VARIANT_COLS: &str = "id, project_scope, kinds_scope, parent, hypothesis, state, proposer_job, evidence, dir, created_at, retired_at, promoted_at";

impl Variant {
    /// The islands of `project_scope`; empty means every project.
    pub fn projects(&self) -> Vec<String> {
        serde_json::from_str(&self.project_scope).unwrap_or_default()
    }
    pub fn kinds(&self) -> Vec<String> {
        serde_json::from_str(&self.kinds_scope).unwrap_or_default()
    }
    /// Whether the variant may run on island `slug`.
    pub fn on_island(&self, slug: &str) -> bool {
        let p = self.projects();
        p.is_empty() || p.iter().any(|x| x == slug)
    }
}

impl Store {
    /// Every variant, oldest first (the archive is never pruned).
    pub fn variants(&self) -> Result<Vec<Variant>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {VARIANT_COLS} FROM variants ORDER BY created_at, id"
        ))?;
        Ok(stmt
            .query_map([], scan_variant)?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Registers a variant in state `proposed` and writes `evolve.registered`.
    /// The caller has already checked the genome; this checks the row.
    pub fn insert_variant(&self, actor: &str, v: &NewVariant) -> Result<Variant> {
        if !crate::settings::valid_variant_id(&v.id) || v.id == BASE_VARIANT {
            return Err(input_err(format!(
                "{:?} is not a variant id (letters, digits, . _ -; not base)",
                v.id
            )));
        }
        if v.hypothesis.trim().is_empty() {
            return Err(input_err("a variant needs a one-line hypothesis"));
        }
        let tx = self.begin()?;
        let parent_known: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM variants WHERE id = ?)",
            [&v.parent],
            |r| r.get(0),
        )?;
        if !parent_known {
            tx.rollback()?;
            return Err(input_err(format!(
                "parent {:?} is not a registered variant",
                v.parent
            )));
        }
        let exists: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM variants WHERE id = ?)",
            [&v.id],
            |r| r.get(0),
        )?;
        if exists {
            tx.rollback()?;
            return Err(input_err(format!("variant {} is already registered", v.id)));
        }
        let now = self.now();
        tx.execute(
            "INSERT INTO variants (id, project_scope, kinds_scope, parent, hypothesis, state, proposer_job, evidence, dir, created_at) VALUES (?, ?, ?, ?, ?, 'proposed', ?, ?, ?, ?)",
            params![
                v.id,
                serde_json::to_string(&v.projects)?,
                serde_json::to_string(&v.kinds)?,
                v.parent,
                v.hypothesis.trim(),
                v.proposer_job,
                serde_json::to_string(&v.evidence)?,
                v.dir,
                now
            ],
        )?;
        tx.event(
            actor,
            VARIANT_REGISTERED,
            &v.id,
            serde_json::json!({
                "parent": v.parent,
                "projects": v.projects,
                "kinds": v.kinds,
                "hypothesis": v.hypothesis.trim(),
                "dir": v.dir,
            }),
        )?;
        tx.commit()?;
        self.variant(&v.id)?
            .ok_or_else(|| anyhow::anyhow!("variant {} vanished after insert", v.id))
    }

    /// Retires `id` (it stays in the archive as a stepping stone) and moves
    /// every project pointer that named it back to its parent when that is
    /// a live promoted variant, else to `base`. Returns the projects whose
    /// pointer moved, each with where it went.
    pub fn retire_variant(
        &self,
        actor: &str,
        id: &str,
        reason: &str,
    ) -> Result<Vec<(String, String)>> {
        if id == BASE_VARIANT {
            return Err(input_err(
                "base is the empty overlay every project falls back to; it is never retired",
            ));
        }
        let Some(v) = self.variant(id)? else {
            return Err(input_err(format!("no variant {id}")));
        };
        if v.state == "retired" {
            return Err(input_err(format!(
                "variant {id} was already retired at {}",
                v.retired_at
            )));
        }
        let back = match self.variant(&v.parent)? {
            Some(p) if p.state == "promoted" => p.id,
            _ => BASE_VARIANT.to_string(),
        };
        let now = self.now();
        let tx = self.begin()?;
        let moved: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT name FROM settings WHERE name LIKE 'evolve.incumbent.%' AND TRIM(value) = ? ORDER BY name",
            )?;
            stmt.query_map([id], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        for name in &moved {
            tx.execute(
                "UPDATE settings SET value = ?, updated_at = ?, updated_by = ? WHERE name = ?",
                params![back, now, actor, name],
            )?;
            tx.event(actor, "setting", name, back.as_str())?;
        }
        tx.execute(
            "UPDATE variants SET state = 'retired', retired_at = ? WHERE id = ?",
            params![now, id],
        )?;
        let moved: Vec<(String, String)> = moved
            .into_iter()
            .map(|n| {
                (
                    n.trim_start_matches(crate::settings::INCUMBENT_PREFIX)
                        .to_string(),
                    back.clone(),
                )
            })
            .collect();
        tx.event(
            actor,
            VARIANT_RETIRED,
            id,
            serde_json::json!({
                "reason": reason,
                "was": v.state,
                "pointers": moved.iter().map(|(p, to)| serde_json::json!({"project": p, "to": to})).collect::<Vec<_>>(),
            }),
        )?;
        tx.commit()?;
        Ok(moved)
    }

    /// Makes `id` the incumbent of `slug`: the pointer moves and the row is
    /// marked promoted. Returns the incumbent it replaced.
    pub fn promote_variant(
        &self,
        actor: &str,
        id: &str,
        slug: &str,
        reason: &str,
    ) -> Result<String> {
        let Some(v) = self.variant(id)? else {
            return Err(input_err(format!("no variant {id}")));
        };
        if v.state == "retired" {
            return Err(input_err(format!(
                "variant {id} is retired; register a new variant instead"
            )));
        }
        if !v.on_island(slug) {
            return Err(input_err(format!(
                "variant {id} does not cover project {slug} (its islands: {})",
                v.projects().join(", ")
            )));
        }
        let was = self.incumbent(slug);
        if was == id {
            return Err(input_err(format!(
                "variant {id} is already the incumbent of {slug}"
            )));
        }
        let name = incumbent_setting(slug);
        crate::settings::validate_setting(&name, id)?;
        let now = self.now();
        let tx = self.begin()?;
        tx.execute("INSERT INTO settings (name, value, updated_at, updated_by) VALUES (?, ?, ?, ?) ON CONFLICT(name) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at, updated_by = excluded.updated_by", params![name, id, now, actor])?;
        tx.event(actor, "setting", &name, id)?;
        if v.state != "promoted" {
            tx.execute(
                "UPDATE variants SET state = 'promoted', promoted_at = ? WHERE id = ?",
                params![now, id],
            )?;
        }
        tx.event(
            actor,
            VARIANT_PROMOTED,
            id,
            serde_json::json!({"project": slug, "was": was, "reason": reason}),
        )?;
        tx.commit()?;
        Ok(was)
    }

    /// True when the owner has turned the agent-program loop on
    /// (`evolve.mode=variants`); every variant path checks this first.
    pub fn variants_mode(&self) -> bool {
        self.setting(EVOLVE_MODE, "").trim() == MODE_VARIANTS
    }

    /// The variant `slug`'s jobs run unless the router picks a canary: its
    /// `evolve.incumbent.<slug>` pointer, or `base` while that is unset or empty.
    pub fn incumbent(&self, slug: &str) -> String {
        let v = self.setting(&incumbent_setting(slug), "");
        let v = v.trim();
        if v.is_empty() {
            BASE_VARIANT.to_string()
        } else {
            v.to_string()
        }
    }

    /// The `base` row, the empty overlay every project starts on. Created on
    /// first use; returns whether this call created it.
    pub fn ensure_base_variant(&self) -> Result<bool> {
        let conn = self.conn()?;
        let n = conn.execute(
            "INSERT OR IGNORE INTO variants (id, hypothesis, state, created_at) VALUES (?, ?, 'promoted', ?)",
            params![
                BASE_VARIANT,
                "the empty overlay: the agent program as the repository and the store define it",
                self.now()
            ],
        )?;
        Ok(n > 0)
    }

    pub fn variant(&self, id: &str) -> Result<Option<Variant>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!("SELECT {VARIANT_COLS} FROM variants WHERE id = ?"),
                [id],
                scan_variant,
            )
            .optional()?)
    }

    /// Writes one `land.reverted` event for `r.sha`, once: a sha that already
    /// has one is left alone and the call returns false.
    pub fn land_reverted(&self, actor: &str, r: &Revert) -> Result<bool> {
        let sha = r.sha.trim();
        if sha.len() < 7 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(input_err(format!(
                "{LAND_REVERTED} needs the landed commit's sha, not {sha:?}"
            )));
        }
        let tx = self.begin()?;
        let seen: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM events WHERE action = ? AND subject = ?)",
            params![LAND_REVERTED, sha],
            |row| row.get(0),
        )?;
        if seen {
            tx.rollback()?;
            return Ok(false);
        }
        let how = if r.how.is_empty() {
            if r.by.is_empty() {
                "unreachable"
            } else {
                "revert"
            }
        } else {
            r.how.as_str()
        };
        tx.event(
            actor,
            LAND_REVERTED,
            sha,
            serde_json::json!({
                "project": r.project,
                "job": r.job,
                "sha": sha,
                "by": r.by,
                "how": how,
            }),
        )?;
        tx.commit()?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Options, validate_setting};

    const NOW: &str = "2026-11-03T09:00:00Z";

    fn open() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(Options {
            home: dir.path().display().to_string(),
            create: true,
            now: NOW.into(),
            ..Options::default()
        })
        .unwrap();
        (dir, s)
    }

    fn columns(s: &Store, table: &str) -> Vec<String> {
        let conn = s.conn().unwrap();
        let mut stmt = conn
            .prepare("SELECT name FROM pragma_table_info(?)")
            .unwrap();
        stmt.query_map([table], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn a_fresh_store_has_the_variant_tables_and_the_run_column_and_stays_inert() {
        let _serial = crate::test_helpers::serial();
        let (_dir, s) = open();
        assert_eq!(
            columns(&s, "variants"),
            [
                "id",
                "project_scope",
                "kinds_scope",
                "parent",
                "hypothesis",
                "state",
                "proposer_job",
                "evidence",
                "dir",
                "created_at",
                "retired_at",
                "promoted_at"
            ]
        );
        assert_eq!(
            columns(&s, "variant_outcomes"),
            [
                "variant",
                "island",
                "kind",
                "trials",
                "landed",
                "kept",
                "tokens",
                "failed",
                "failed_by_cause",
                "interventions",
                "flakes",
                "breaches",
                "posterior_json",
                "feedback",
                "computed_at"
            ]
        );
        assert!(columns(&s, "runs").contains(&"variant".to_string()));
        // Inert by default: fixtures mode, no rows, every project on base.
        assert_eq!(s.setting(EVOLVE_MODE, ""), MODE_FIXTURES);
        assert!(!s.variants_mode());
        assert_eq!(s.incumbent("demo"), BASE_VARIANT);
        let conn = s.conn().unwrap();
        let rows: i64 = conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM variants) + (SELECT COUNT(*) FROM variant_outcomes)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            rows, 0,
            "nothing writes variants under evolve.mode=fixtures"
        );
        conn.execute(
            "INSERT INTO runs (id, job_id, agent, project, started_at) VALUES ('r1', 'j1', 'a', 'demo', ?)",
            [NOW],
        )
        .unwrap();
        let variant: String = conn
            .query_row("SELECT variant FROM runs WHERE id = 'r1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(variant, "", "a run is stamped with no variant by default");
    }

    #[test]
    fn an_older_store_gains_the_variant_column_and_tables_on_open() {
        let _serial = crate::test_helpers::serial();
        let (dir, s) = open();
        {
            let conn = s.conn().unwrap();
            conn.execute_batch(
                "DROP TABLE variant_outcomes; DROP TABLE variants; ALTER TABLE runs DROP COLUMN variant;",
            )
            .unwrap();
        }
        assert!(!columns(&s, "runs").contains(&"variant".to_string()));
        s.close().unwrap();
        drop(s);
        let s = Store::open(Options {
            home: dir.path().display().to_string(),
            now: NOW.into(),
            ..Options::default()
        })
        .unwrap();
        s.ensure_records().unwrap();
        assert!(columns(&s, "runs").contains(&"variant".to_string()));
        assert!(!columns(&s, "variants").is_empty());
        assert!(!columns(&s, "variant_outcomes").is_empty());
    }

    #[test]
    fn variants_are_kept_and_outcomes_name_a_variant() {
        let _serial = crate::test_helpers::serial();
        let (_dir, s) = open();
        assert!(s.ensure_base_variant().unwrap());
        assert!(!s.ensure_base_variant().unwrap(), "created once");
        let base = s.variant(BASE_VARIANT).unwrap().unwrap();
        assert_eq!(base.state, "promoted");
        assert_eq!(base.parent, "");
        assert_eq!(base.project_scope, "[]");
        assert!(s.variant("nope").unwrap().is_none());
        let conn = s.conn().unwrap();
        let err = conn
            .execute("DELETE FROM variants WHERE id = 'base'", [])
            .unwrap_err();
        assert!(err.to_string().contains("never deleted"), "{err}");
        let err = conn
            .execute(
                "INSERT INTO variants (id, state, created_at) VALUES ('v1', 'alive', ?)",
                [NOW],
            )
            .unwrap_err();
        assert!(err.to_string().contains("CHECK"), "{err}");
        for state in VARIANT_STATES {
            conn.execute(
                "INSERT INTO variants (id, parent, state, created_at) VALUES (?, 'base', ?, ?)",
                params![format!("v-{state}"), state, NOW],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO variant_outcomes (variant, island, kind, trials, landed, kept, tokens, computed_at) VALUES ('base', 'demo', 'task', 4, 2, 1, 1000, ?)",
            [NOW],
        )
        .unwrap();
        let err = conn
            .execute(
                "INSERT INTO variant_outcomes (variant, island, kind, trials, landed, kept, tokens, computed_at) VALUES ('base', 'demo', 'task', 5, 2, 1, 1000, ?)",
                [NOW],
            )
            .unwrap_err();
        assert!(err.to_string().contains("UNIQUE"), "{err}");
        let err = conn
            .execute(
                "INSERT INTO variant_outcomes (variant, island, kind, computed_at) VALUES ('ghost', 'demo', 'task', ?)",
                [NOW],
            )
            .unwrap_err();
        assert!(err.to_string().contains("FOREIGN KEY"), "{err}");
    }

    #[test]
    fn the_incumbent_pointer_is_a_per_project_setting() {
        let _serial = crate::test_helpers::serial();
        let (_dir, s) = open();
        assert_eq!(s.incumbent("demo"), "base");
        s.set_setting("human", "evolve.incumbent.demo", "v-20261103T090000Z-sable")
            .unwrap();
        assert_eq!(s.incumbent("demo"), "v-20261103T090000Z-sable");
        assert_eq!(s.incumbent("other"), "base", "the pointer is per project");
        s.set_setting("human", "evolve.incumbent.demo", "  ")
            .unwrap();
        assert_eq!(s.incumbent("demo"), "base", "empty means base");
        assert!(
            validate_setting("evolve.incumbent.demo", "two words")
                .unwrap_err()
                .to_string()
                .contains("takes a variant id")
        );
        let err = validate_setting("evolve.incumbent.", "base")
            .unwrap_err()
            .to_string();
        assert!(err.contains("evolve.incumbent.<project slug>"), "{err}");
        let err = validate_setting("evolve.incumbent.Demo X", "base")
            .unwrap_err()
            .to_string();
        assert!(err.contains("Unknown setting"), "{err}");
        // The family is not part of the listed catalogue.
        assert!(
            !crate::extra_setting_names()
                .iter()
                .any(|n| n.starts_with("evolve.incumbent"))
        );
    }

    #[test]
    fn a_revert_is_recorded_once_per_sha() {
        let _serial = crate::test_helpers::serial();
        let (_dir, s) = open();
        let r = Revert {
            project: "demo".into(),
            sha: "0123456789abcdef0123456789abcdef01234567".into(),
            job: "job-1".into(),
            by: "fedcba9876543210fedcba9876543210fedcba98".into(),
            how: String::new(),
        };
        assert!(s.land_reverted("pulse", &r).unwrap());
        assert!(!s.land_reverted("pulse", &r).unwrap(), "exactly one event");
        let conn = s.conn().unwrap();
        let (n, detail): (i64, String) = conn
            .query_row(
                "SELECT COUNT(*), MAX(detail) FROM events WHERE action = 'land.reverted' AND subject = ?",
                [&r.sha],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(n, 1);
        let d: serde_json::Value = serde_json::from_str(&detail).unwrap();
        assert_eq!(d["how"], "revert");
        assert_eq!(d["job"], "job-1");
        assert_eq!(d["project"], "demo");
        let gone = Revert {
            sha: "abcdef1".into(),
            ..Revert::default()
        };
        assert!(s.land_reverted("pulse", &gone).unwrap());
        let how: String = conn
            .query_row(
                "SELECT json_extract(detail, '$.how') FROM events WHERE action = 'land.reverted' AND subject = 'abcdef1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(how, "unreachable");
        for bad in ["", "HEAD", "abc", "not-a-sha"] {
            let r = Revert {
                sha: bad.into(),
                ..Revert::default()
            };
            assert!(s.land_reverted("pulse", &r).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_variant_settings_have_rules_and_defaults() {
        let _serial = crate::test_helpers::serial();
        let (_dir, s) = open();
        for (name, def) in [
            ("evolve.mode", "fixtures"),
            ("evolve.canary_share", "20"),
            ("evolve.canaries_per_island", "3"),
            ("evolve.keep_days", "7"),
            ("evolve.guard_margin", "10"),
            ("evolve.min_trials", "40"),
            ("evolve.promote_confidence", "0.95"),
            ("evolve.replay_set", "20"),
            ("evolve.trigger_failures", "5"),
        ] {
            assert_eq!(s.setting(name, ""), def, "{name}");
            assert!(crate::evolve_setting_names().contains(&name.to_string()));
            assert_eq!(validate_setting(name, def).unwrap(), def, "{name}");
        }
        assert_eq!(
            validate_setting("evolve.mode", " Variants ").unwrap(),
            "variants"
        );
        assert!(validate_setting("evolve.mode", "both").is_err());
        assert_eq!(
            validate_setting("evolve.canary_share", "100").unwrap(),
            "100"
        );
        assert_eq!(validate_setting("evolve.canary_share", "0").unwrap(), "0");
        let err = validate_setting("evolve.canary_share", "101").unwrap_err();
        assert!(err.to_string().contains("is a percent"), "{err}");
        assert!(validate_setting("evolve.guard_margin", "150").is_err());
        assert!(validate_setting("evolve.canary_share", "-1").is_err());
        assert!(validate_setting("evolve.keep_days", "0").is_err());
        assert!(validate_setting("evolve.min_trials", "0").is_err());
        assert!(validate_setting("evolve.promote_confidence", "1.2").is_err());
        assert_eq!(
            validate_setting("evolve.promote_confidence", "0.99").unwrap(),
            "0.99"
        );
        s.set_setting("human", "evolve.mode", "variants").unwrap();
        assert!(s.variants_mode());
    }

    #[test]
    fn a_variant_may_set_the_allowlist_and_never_the_immutable_set() {
        for name in [
            "worker_model",
            "light_reasoning",
            "harness.compact_at",
            "harness.max_steps",
            "lead.idle_hours",
            "plan.operating_layer_per_day",
        ] {
            assert_eq!(variant_setting_refusal(name), None, "{name}");
        }
        for name in [
            "owner.presence",
            "stop",
            "landing",
            "landing.push",
            "push.credential",
            "git.github_app.private_key_ref",
            "compute.provider",
            "harness.deny_tools",
            "harness.allow_tools",
            "evolve.mode",
            "evolve.incumbent.demo",
        ] {
            let why = variant_setting_refusal(name).unwrap_or_default();
            assert!(why.contains("immutable set"), "{name}: {why}");
            assert!(why.starts_with(name), "names the key: {why}");
        }
        for name in ["provider", "workers", "harness.max_cost", "quorum"] {
            let why = variant_setting_refusal(name).unwrap_or_default();
            assert!(
                why.contains("not on the variant allowlist"),
                "{name}: {why}"
            );
        }
    }

    #[test]
    fn variants_register_retire_and_promote_through_the_pointer() {
        let _serial = crate::test_helpers::serial();
        let (_dir, s) = open();
        s.ensure_base_variant().unwrap();
        let new = NewVariant {
            id: "v-one".into(),
            projects: vec!["demo".into()],
            parent: BASE_VARIANT.into(),
            hypothesis: "a cheaper compaction model keeps kept landings per token up".into(),
            dir: "/tmp/v-one".into(),
            ..NewVariant::default()
        };
        let v = s.insert_variant("human", &new).unwrap();
        assert_eq!(v.state, "proposed");
        assert_eq!(v.projects(), ["demo"]);
        assert!(v.on_island("demo") && !v.on_island("other"));
        let err = s.insert_variant("human", &new).unwrap_err().to_string();
        assert!(err.contains("already registered"), "{err}");
        for (bad, want) in [
            (
                NewVariant {
                    id: "base".into(),
                    ..new.clone()
                },
                "not a variant id",
            ),
            (
                NewVariant {
                    id: "two words".into(),
                    ..new.clone()
                },
                "not a variant id",
            ),
            (
                NewVariant {
                    id: "v-2".into(),
                    parent: "ghost".into(),
                    ..new.clone()
                },
                "not a registered variant",
            ),
            (
                NewVariant {
                    id: "v-3".into(),
                    hypothesis: " ".into(),
                    ..new.clone()
                },
                "hypothesis",
            ),
        ] {
            let err = s.insert_variant("human", &bad).unwrap_err().to_string();
            assert!(err.contains(want), "{err}");
        }
        let err = s
            .promote_variant("human", "v-one", "other", "")
            .unwrap_err()
            .to_string();
        assert!(err.contains("does not cover project other"), "{err}");
        assert_eq!(
            s.promote_variant("human", "v-one", "demo", "test").unwrap(),
            "base"
        );
        assert_eq!(s.incumbent("demo"), "v-one");
        assert_eq!(s.variant("v-one").unwrap().unwrap().state, "promoted");
        assert!(s.promote_variant("human", "v-one", "demo", "").is_err());
        let moved = s.retire_variant("human", "v-one", "rollback").unwrap();
        assert_eq!(moved, [("demo".to_string(), "base".to_string())]);
        assert_eq!(s.incumbent("demo"), "base");
        let v = s.variant("v-one").unwrap().unwrap();
        assert_eq!(v.state, "retired");
        assert_eq!(v.retired_at, NOW);
        assert!(s.retire_variant("human", "v-one", "").is_err());
        assert!(s.retire_variant("human", "base", "").is_err());
        assert!(s.promote_variant("human", "v-one", "demo", "").is_err());
        let conn = s.conn().unwrap();
        let actions: Vec<String> = conn
            .prepare("SELECT action FROM events WHERE subject = 'v-one' ORDER BY seq")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            actions,
            [VARIANT_REGISTERED, VARIANT_PROMOTED, VARIANT_RETIRED]
        );
        assert_eq!(s.variants().unwrap().len(), 2);
    }
}
