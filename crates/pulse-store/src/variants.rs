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

impl Store {
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
                "SELECT id, project_scope, kinds_scope, parent, hypothesis, state, proposer_job, evidence, dir, created_at, retired_at, promoted_at FROM variants WHERE id = ?",
                [id],
                |r| {
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
                },
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
}
