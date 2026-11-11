//! The team's members and the nonces their passkey-signed writes spend
//! (ADR-070 rule 5). A member is one WebAuthn credential: the hosted front
//! registers the passkey and syncs the row here (members and roles come
//! from the team's Clerk organization); the brain keeps the public key and
//! the signature counter, and verifies every write that reaches it through
//! the front against them (`hive_runner::webauthn`). No private key and no
//! session token is ever stored.
use crate::store::{Store, input_err};
use anyhow::Result;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const MEMBERS_DDL: &[&str] = &[
    r####"CREATE TABLE IF NOT EXISTS members (
  credential_id TEXT PRIMARY KEY,     -- the passkey's WebAuthn credential id, base64url
  member TEXT NOT NULL,               -- who holds it: the front's user id (Clerk)
  name TEXT NOT NULL DEFAULT '',      -- display name
  role TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('owner','member')),
  public_key TEXT NOT NULL,           -- ES256 public key, SubjectPublicKeyInfo DER, base64url
  sign_count INTEGER NOT NULL DEFAULT 0, -- the authenticator's counter at its last verified write
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','revoked')),
  created_by TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  last_used_at TEXT NOT NULL DEFAULT ''
);"####,
    r####"CREATE TABLE IF NOT EXISTS write_nonces (
  nonce TEXT PRIMARY KEY,             -- base64url, issued by GET /api/v1/nonce, spent by one signed write
  issued_at TEXT NOT NULL,
  expires_at TEXT NOT NULL
);"####,
];

/// The most unspent nonces the brain keeps at once.
pub const NONCE_CAP: i64 = 1000;

/// The roles a member holds in the team (the front's organization roles).
pub const MEMBER_ROLES: &[&str] = &["owner", "member"];

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Member {
    pub credential_id: String,
    pub member: String,
    pub name: String,
    pub role: String,
    pub public_key: String,
    pub sign_count: i64,
    pub status: String,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
    pub last_used_at: String,
}

impl Member {
    pub fn active(&self) -> bool {
        self.status == "active"
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct MemberInput {
    pub credential_id: String,
    pub member: String,
    pub name: String,
    pub role: String,
    pub public_key: String,
}

const MEMBER_COLUMNS: &str = "credential_id, member, name, role, public_key, sign_count, status, created_by, created_at, updated_at, last_used_at";

fn member_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Member> {
    Ok(Member {
        credential_id: r.get(0)?,
        member: r.get(1)?,
        name: r.get(2)?,
        role: r.get(3)?,
        public_key: r.get(4)?,
        sign_count: r.get(5)?,
        status: r.get(6)?,
        created_by: r.get(7)?,
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
        last_used_at: r.get(10)?,
    })
}

fn base64url_shaped(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn later(at: &str, seconds: i64) -> String {
    let base = DateTime::parse_from_rfc3339(at)
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    (base + Duration::seconds(seconds)).to_rfc3339_opts(SecondsFormat::Secs, true)
}

impl Store {
    /// Adds a member's passkey or replaces its holder, name, role and key;
    /// a replaced key starts its counter again and an earlier revocation
    /// is lifted.
    pub fn member_put(&self, actor: &str, input: &MemberInput) -> Result<Member> {
        let role = if input.role.trim().is_empty() {
            "member"
        } else {
            input.role.trim()
        };
        if !MEMBER_ROLES.contains(&role) {
            return Err(input_err(format!(
                "a member's role is owner or member, not {role:?}"
            )));
        }
        let credential_id = input.credential_id.trim();
        if !base64url_shaped(credential_id) {
            return Err(input_err(
                "a member's credential id is the passkey's id, base64url",
            ));
        }
        let public_key = input.public_key.trim();
        if !base64url_shaped(public_key) {
            return Err(input_err(
                "a member's public key is SubjectPublicKeyInfo DER, base64url",
            ));
        }
        let member = input.member.trim();
        if member.is_empty() {
            return Err(input_err("a member needs the front's user id"));
        }
        let now = self.now();
        let tx = self.begin()?;
        tx.execute(
            "INSERT INTO members (credential_id, member, name, role, public_key, sign_count, status, created_by, created_at, updated_at) VALUES (?, ?, ?, ?, ?, 0, 'active', ?, ?, ?) ON CONFLICT(credential_id) DO UPDATE SET member = excluded.member, name = excluded.name, role = excluded.role, sign_count = CASE WHEN public_key = excluded.public_key THEN sign_count ELSE 0 END, public_key = excluded.public_key, status = 'active', updated_at = excluded.updated_at",
            params![credential_id, member, input.name.trim(), role, public_key, actor, now, now],
        )?;
        tx.event(
            actor,
            "member.added",
            credential_id,
            json!({"member": member, "role": role}).to_string().as_str(),
        )?;
        tx.commit()?;
        self.member(credential_id)?
            .ok_or_else(|| anyhow::anyhow!("member {credential_id} vanished after its write"))
    }

    pub fn member(&self, credential_id: &str) -> Result<Option<Member>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!("SELECT {MEMBER_COLUMNS} FROM members WHERE credential_id = ?"),
                [credential_id],
                member_row,
            )
            .optional()?)
    }

    pub fn members(&self) -> Result<Vec<Member>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {MEMBER_COLUMNS} FROM members ORDER BY member, credential_id"
        ))?;
        let rows = stmt.query_map([], member_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Revokes a passkey: no write it signs verifies again. Returns false
    /// when no active member holds it.
    pub fn member_revoke(&self, actor: &str, credential_id: &str) -> Result<bool> {
        let tx = self.begin()?;
        let n = tx.execute(
            "UPDATE members SET status = 'revoked', updated_at = ? WHERE credential_id = ? AND status = 'active'",
            params![self.now(), credential_id],
        )?;
        if n > 0 {
            tx.event(actor, "member.revoked", credential_id, "")?;
        }
        tx.commit()?;
        Ok(n > 0)
    }

    /// Records a verified write: the counter moves to `sign_count` only if
    /// it moves forward (or both are zero, for authenticators that keep
    /// none), so two writes racing with one assertion cannot both pass.
    pub fn member_used(&self, credential_id: &str, sign_count: i64) -> Result<bool> {
        let conn = self.conn()?;
        let n = conn.execute(
            "UPDATE members SET sign_count = ?1, last_used_at = ?2 WHERE credential_id = ?3 AND status = 'active' AND (sign_count < ?1 OR (sign_count = 0 AND ?1 = 0))",
            params![sign_count, self.now(), credential_id],
        )?;
        Ok(n == 1)
    }

    /// Keeps a nonce the brain hands out for `seconds`, and drops the ones
    /// that ran out. Returns when it expires, or None while
    /// [`NONCE_CAP`] unspent nonces are outstanding (the route is open to
    /// reads, so it must not grow the store without bound).
    pub fn nonce_issue(&self, nonce: &str, seconds: i64) -> Result<Option<String>> {
        let now = self.now();
        let expires = later(&now, seconds);
        let tx = self.begin()?;
        tx.execute("DELETE FROM write_nonces WHERE expires_at <= ?", [&now])?;
        let open: i64 = tx.query_row("SELECT COUNT(*) FROM write_nonces", [], |r| r.get(0))?;
        if open >= NONCE_CAP {
            tx.commit()?;
            return Ok(None);
        }
        tx.execute(
            "INSERT INTO write_nonces (nonce, issued_at, expires_at) VALUES (?, ?, ?)",
            params![nonce, now, expires],
        )?;
        tx.commit()?;
        Ok(Some(expires))
    }

    /// Spends a nonce: true once for a nonce this brain issued that has not
    /// expired, false for an unknown, spent or expired one.
    pub fn nonce_take(&self, nonce: &str) -> Result<bool> {
        let conn = self.conn()?;
        let n = conn.execute(
            "DELETE FROM write_nonces WHERE nonce = ? AND expires_at > ?",
            params![nonce, self.now()],
        )?;
        Ok(n == 1)
    }
}

#[cfg(test)]
mod tests {
    use crate::{MemberInput, Options, Store};

    fn store() -> (Store, tempfile::TempDir) {
        // Inside a job the store refuses to create one; tests make their own.
        unsafe { std::env::set_var("HIVE_JOB", "") };
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(Options {
            home: dir.path().display().to_string(),
            create: true,
            now: "2026-09-06T12:00:00Z".into(),
            ..Options::default()
        })
        .unwrap();
        (s, dir)
    }

    fn input(cred: &str, key: &str) -> MemberInput {
        MemberInput {
            credential_id: cred.into(),
            member: "user_1".into(),
            name: "Ada".into(),
            role: "owner".into(),
            public_key: key.into(),
        }
    }

    #[test]
    fn members_keep_a_key_and_a_counter_that_only_moves_forward() {
        let _serial = crate::test_helpers::serial();
        let (s, _dir) = store();
        let m = s.member_put("human", &input("cred-1", "AAAA")).unwrap();
        assert_eq!(
            (m.role.as_str(), m.sign_count, m.active()),
            ("owner", 0, true)
        );
        assert!(s.member_used("cred-1", 3).unwrap());
        assert!(!s.member_used("cred-1", 3).unwrap(), "same counter twice");
        assert!(!s.member_used("cred-1", 2).unwrap(), "counter went back");
        assert_eq!(s.member("cred-1").unwrap().unwrap().sign_count, 3);
        // The same key again keeps its counter; a new key starts over.
        s.member_put("human", &input("cred-1", "AAAA")).unwrap();
        assert_eq!(s.member("cred-1").unwrap().unwrap().sign_count, 3);
        s.member_put("human", &input("cred-1", "BBBB")).unwrap();
        assert_eq!(s.member("cred-1").unwrap().unwrap().sign_count, 0);
        assert!(
            s.member_used("cred-1", 0).unwrap(),
            "counterless authenticators"
        );
        assert!(s.member_revoke("human", "cred-1").unwrap());
        assert!(!s.member_revoke("human", "cred-1").unwrap());
        assert!(!s.member_used("cred-1", 9).unwrap(), "revoked");
        assert_eq!(s.members().unwrap().len(), 1);
        for bad in [
            MemberInput {
                role: "admin".into(),
                ..input("c", "K")
            },
            input("not base64!", "K"),
            input("c", ""),
            MemberInput {
                member: " ".into(),
                ..input("c", "K")
            },
        ] {
            assert!(s.member_put("human", &bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_nonce_is_spent_once_and_never_after_it_expires() {
        let _serial = crate::test_helpers::serial();
        let (s, _dir) = store();
        assert_eq!(
            s.nonce_issue("n1", 300).unwrap().as_deref(),
            Some("2026-09-06T12:05:00Z")
        );
        assert!(s.nonce_take("n1").unwrap());
        assert!(!s.nonce_take("n1").unwrap(), "replayed");
        assert!(!s.nonce_take("never-issued").unwrap());
        s.nonce_issue("n2", 0).unwrap();
        assert!(!s.nonce_take("n2").unwrap(), "expired");
        let conn = s.conn().unwrap();
        conn.execute(
            &format!("WITH RECURSIVE k(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM k WHERE i < {}) INSERT INTO write_nonces SELECT 'cap' || i, '2026-09-06T12:00:00Z', '2026-09-06T13:00:00Z' FROM k", crate::NONCE_CAP),
            [],
        )
        .unwrap();
        drop(conn);
        assert_eq!(s.nonce_issue("n3", 300).unwrap(), None, "at the cap");
        assert!(s.nonce_take("cap1").unwrap());
        assert!(s.nonce_issue("n3", 300).unwrap().is_some(), "room again");
    }
}
