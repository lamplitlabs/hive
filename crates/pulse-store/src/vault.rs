//! The team secret vault on the brain (ADR-070 rule 4).
//!
//! The store keeps ciphertext only (`vault_secrets`) and the team's data key
//! only wrapped to a holder's X25519 public key (`vault_keys`). On a brain
//! that runs its own jobs (LocalExec) the holder is the brain itself: its
//! private key is a mode-0600 file under `PULSE_HOME/private/`, outside the
//! store, so a copy of the store alone opens nothing. Enrolled devices (step
//! 2's registry) add their own `device:` rows with [`Store::vault_wrap_to`].
//!
//! Nothing here ever writes a value into an event, a log line or an error:
//! events carry names only.

use crate::store::{Store, input_err};
use anyhow::{Result, anyhow};
use pulse_hand_core::vault::{self, DataKey, DeviceKey, Sealed, Zeroizing};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use serde_json::json;
use std::path::PathBuf;

/// The brain's own vault key file, under `PULSE_HOME/private/`.
pub const VAULT_KEY_FILE: &str = "vault-key";
/// The largest value the vault takes, in bytes.
pub const VAULT_MAX_VALUE: usize = 64 * 1024;

/// One secret as a list shows it: never the value.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct VaultEntry {
    pub name: String,
    pub created_by: String,
    pub created_at: String,
    pub updated_by: String,
    pub updated_at: String,
    /// Bytes of the sealed value (ciphertext and tag), not of the secret.
    pub sealed_bytes: usize,
}

/// One holder of the team key.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct VaultHolder {
    pub holder: String,
    pub kind: String,
    pub public_key: String,
    pub created_at: String,
}

/// The holder name of this brain's own key.
pub fn brain_holder() -> String {
    format!("brain:{}", vault::host())
}

impl Store {
    fn vault_key_path(&self) -> PathBuf {
        PathBuf::from(&self.home)
            .join("private")
            .join(VAULT_KEY_FILE)
    }

    /// This brain's X25519 key, made on first use (mode 0600 in a 0700
    /// directory).
    fn vault_device_key(&self, create: bool) -> Result<Option<DeviceKey>> {
        let path = self.vault_key_path();
        match std::fs::read(&path) {
            Ok(raw) => {
                let raw = Zeroizing::new(raw);
                return DeviceKey::from_secret(&raw)
                    .map(Some)
                    .map_err(|e| anyhow!("{}: {e}", path.display()));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(anyhow!("{}: {e}", path.display())),
        }
        if !create {
            return Ok(None);
        }
        let key = DeviceKey::generate()?;
        let dir = path.parent().expect("private dir");
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)?;
            use std::io::Write;
            f.write_all(key.secret_b64().as_bytes())?;
        }
        #[cfg(not(unix))]
        std::fs::write(&path, key.secret_b64().as_bytes())?;
        Ok(Some(key))
    }

    /// The team's data key, unwrapped with this brain's key. Made (and
    /// wrapped to this brain) the first time a secret is written; `None`
    /// when the vault is empty and `create` is false.
    fn vault_data_key(&self, actor: &str, create: bool) -> Result<Option<DataKey>> {
        let holder = brain_holder();
        let conn = self.conn()?;
        let wrapped: Option<String> = conn
            .query_row(
                "SELECT wrapped FROM vault_keys WHERE holder = ?",
                [&holder],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(w) = wrapped {
            let key = self.vault_device_key(false)?.ok_or_else(|| {
                anyhow!(
                    "the vault's key for {holder} is missing ({}); the secrets cannot be opened on this machine - set them again",
                    self.vault_key_path().display()
                )
            })?;
            return key.unwrap(&w).map(Some);
        }
        let others: i64 = conn.query_row("SELECT COUNT(*) FROM vault_keys", [], |r| r.get(0))?;
        if others > 0 {
            return Err(anyhow!(
                "this machine ({holder}) holds no wrap of the team's vault key; a holder must wrap it to this machine first"
            ));
        }
        if !create {
            return Ok(None);
        }
        let device = self
            .vault_device_key(true)?
            .ok_or_else(|| anyhow!("no vault key"))?;
        let data = DataKey::generate()?;
        let wrapped = vault::wrap(&data, &device.public_b64())?;
        conn.execute(
            "INSERT INTO vault_keys (holder, kind, public_key, wrapped, created_by, created_at) VALUES (?, 'brain', ?, ?, ?, ?)",
            params![holder, device.public_b64(), wrapped, actor, self.now()],
        )?;
        self.event(
            actor,
            "secret.key",
            &holder,
            json!({"holder": holder, "kind": "brain", "public_key": device.public_b64()}),
        )?;
        Ok(Some(data))
    }

    /// Wraps the team key to another holder's public key (an enrolled
    /// device, the owner): it can then open the vault where it runs.
    pub fn vault_wrap_to(
        &self,
        actor: &str,
        holder: &str,
        kind: &str,
        public_key: &str,
    ) -> Result<()> {
        if !["device", "owner", "brain"].contains(&kind) {
            return Err(input_err(format!("unknown vault holder kind {kind:?}")));
        }
        let data = self
            .vault_data_key(actor, true)?
            .ok_or_else(|| anyhow!("no vault key"))?;
        let wrapped = vault::wrap(&data, public_key)?;
        self.conn()?.execute(
            "INSERT INTO vault_keys (holder, kind, public_key, wrapped, created_by, created_at) VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(holder) DO UPDATE SET kind = excluded.kind, public_key = excluded.public_key, wrapped = excluded.wrapped, created_by = excluded.created_by, created_at = excluded.created_at",
            params![holder, kind, public_key.trim(), wrapped, actor, self.now()],
        )?;
        self.event(
            actor,
            "secret.key",
            holder,
            json!({"holder": holder, "kind": kind, "public_key": public_key.trim()}),
        )
    }

    /// The holders of the team key.
    pub fn vault_holders(&self) -> Result<Vec<VaultHolder>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT holder, kind, public_key, created_at FROM vault_keys ORDER BY holder",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(VaultHolder {
                holder: r.get(0)?,
                kind: r.get(1)?,
                public_key: r.get(2)?,
                created_at: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Seals `value` as the secret `name` and stores the ciphertext. The
    /// value never reaches an event or an error; `secret.set` names it only.
    pub fn vault_set(&self, actor: &str, name: &str, value: &[u8]) -> Result<VaultEntry> {
        let name = name.trim();
        if let Some(why) = vault::name_problem(name) {
            return Err(input_err(why));
        }
        if value.is_empty() {
            return Err(input_err(format!("the value of {name} is empty")));
        }
        if value.len() > VAULT_MAX_VALUE {
            return Err(input_err(format!(
                "the value of {name} is over {} KiB",
                VAULT_MAX_VALUE / 1024
            )));
        }
        let data = self
            .vault_data_key(actor, true)?
            .ok_or_else(|| anyhow!("no vault key"))?;
        let sealed = vault::seal(&data, name, value)?;
        let now = self.now();
        let tx = self.begin()?;
        let existed: bool = tx
            .query_row("SELECT 1 FROM vault_secrets WHERE name = ?", [name], |_| {
                Ok(true)
            })
            .optional()?
            .unwrap_or(false);
        tx.execute(
            "INSERT INTO vault_secrets (name, ciphertext, nonce, created_by, created_at, updated_by, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?4, ?5)
             ON CONFLICT(name) DO UPDATE SET ciphertext = excluded.ciphertext, nonce = excluded.nonce, updated_by = excluded.updated_by, updated_at = excluded.updated_at",
            params![name, sealed.ciphertext, sealed.nonce, actor, now],
        )?;
        tx.event(
            actor,
            "secret.set",
            name,
            crate::store::Detail::json(json!({"name": name, "replaced": existed}))?,
        )?;
        tx.commit()?;
        self.vault_entry(name)?
            .ok_or_else(|| anyhow!("{name} vanished after it was set"))
    }

    /// One secret's row, without its value.
    pub fn vault_entry(&self, name: &str) -> Result<Option<VaultEntry>> {
        Ok(self
            .vault_entries()?
            .into_iter()
            .find(|e| e.name == name.trim()))
    }

    /// Every secret, without values, by name.
    pub fn vault_entries(&self) -> Result<Vec<VaultEntry>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT name, created_by, created_at, updated_by, updated_at, length(ciphertext) FROM vault_secrets ORDER BY name",
        )?;
        let rows = stmt.query_map([], |r| {
            let b64_len: i64 = r.get(5)?;
            Ok(VaultEntry {
                name: r.get(0)?,
                created_by: r.get(1)?,
                created_at: r.get(2)?,
                updated_by: r.get(3)?,
                updated_at: r.get(4)?,
                sealed_bytes: (b64_len.max(0) as usize) * 3 / 4,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Removes the secret `name`; false when there was none.
    pub fn vault_remove(&self, actor: &str, name: &str) -> Result<bool> {
        let name = name.trim();
        let tx = self.begin()?;
        let n = tx.execute("DELETE FROM vault_secrets WHERE name = ?", [name])?;
        if n > 0 {
            tx.event(
                actor,
                "secret.removed",
                name,
                crate::store::Detail::json(json!({"name": name}))?,
            )?;
        }
        tx.commit()?;
        Ok(n > 0)
    }

    /// Opens the named secrets with this brain's key, in memory, for one
    /// run (LocalExec). Every value is wiped when the map is dropped. A name
    /// the vault does not hold is an error naming it; no value is ever in
    /// an error.
    pub fn vault_open(&self, names: &[String]) -> Result<Vec<(String, Zeroizing<String>)>> {
        if names.is_empty() {
            return Ok(Vec::new());
        }
        let data = self.vault_data_key("", false)?.ok_or_else(|| {
            anyhow!(
                "the vault is empty; set {} with `pulse secret set NAME`",
                names.join(", ")
            )
        })?;
        let conn = self.conn()?;
        let mut out = Vec::new();
        for name in names {
            let row: Option<(String, String)> = conn
                .query_row(
                    "SELECT ciphertext, nonce FROM vault_secrets WHERE name = ?",
                    [name],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let Some((ciphertext, nonce)) = row else {
                return Err(anyhow!(
                    "the vault holds no {name}; set it with `pulse secret set {name}`"
                ));
            };
            let plain = vault::open(&data, name, &Sealed { ciphertext, nonce })?;
            let text = String::from_utf8(plain.to_vec())
                .map_err(|_| anyhow!("{name} is not UTF-8 text, so it cannot be an env value"))?;
            out.push((name.clone(), Zeroizing::new(text)));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Options;

    fn new_store() -> (tempfile::TempDir, Store) {
        let _guard = crate::test_helpers::serial();
        unsafe { std::env::set_var("PULSE_JOB", "") };
        let home = tempfile::tempdir().unwrap();
        let s = Store::open(Options {
            home: home.path().display().to_string(),
            create: true,
            now: "2026-11-10T12:00:00Z".into(),
            ..Options::default()
        })
        .unwrap();
        (home, s)
    }

    fn events(s: &Store) -> Vec<(String, String, String)> {
        let conn = s.conn().unwrap();
        let mut stmt = conn
            .prepare("SELECT action, subject, detail FROM events WHERE action LIKE 'secret.%' ORDER BY seq")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn the_store_holds_ciphertext_only_and_opens_with_the_brain_key() {
        let (home, s) = new_store();
        let value = "sk-ant-vault-test-0123456789";
        s.vault_set("owner", "ANTHROPIC_API_KEY", value.as_bytes())
            .unwrap();
        // Nowhere in the store file, in the clear.
        drop(s.conn().unwrap());
        let conn = s.conn().unwrap();
        let (ct, nonce): (String, String) = conn
            .query_row(
                "SELECT ciphertext, nonce FROM vault_secrets WHERE name = 'ANTHROPIC_API_KEY'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(!ct.contains(value) && !nonce.contains(value));
        let key_file = home.path().join("private").join(VAULT_KEY_FILE);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&key_file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let opened = s.vault_open(&["ANTHROPIC_API_KEY".into()]).unwrap();
        assert_eq!(opened[0].1.as_str(), value);
        // Events name the secret, never its value.
        let ev = events(&s);
        let actions: Vec<_> = ev.iter().map(|e| e.0.as_str()).collect();
        assert_eq!(actions, ["secret.key", "secret.set"]);
        assert!(ev.iter().all(|e| !e.2.contains(value)));
        // A replace is said so; the list carries no value.
        s.vault_set("owner", "ANTHROPIC_API_KEY", b"second")
            .unwrap();
        assert!(events(&s).last().unwrap().2.contains("\"replaced\": true"));
        let list = s.vault_entries().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].created_by, "owner");
        assert!(!format!("{list:?}").contains("second"));
        // Without the brain's key file the vault does not open.
        std::fs::remove_file(&key_file).unwrap();
        let err = s.vault_open(&["ANTHROPIC_API_KEY".into()]).unwrap_err();
        assert!(err.to_string().contains("missing"), "{err}");
    }

    #[test]
    fn bad_names_empty_values_and_missing_secrets_are_refused_by_name() {
        let (_home, s) = new_store();
        assert!(s.vault_set("owner", "lower", b"x").is_err());
        assert!(s.vault_set("owner", "PULSE_JOB", b"x").is_err());
        assert!(s.vault_set("owner", "EMPTY", b"").is_err());
        let err = s.vault_open(&["GH_TOKEN".into()]).unwrap_err().to_string();
        assert!(err.contains("vault is empty"), "{err}");
        s.vault_set("owner", "OPENAI_API_KEY", b"sk-1").unwrap();
        let err = s.vault_open(&["GH_TOKEN".into()]).unwrap_err().to_string();
        assert!(err.contains("holds no GH_TOKEN"), "{err}");
        assert!(s.vault_remove("owner", "OPENAI_API_KEY").unwrap());
        assert!(!s.vault_remove("owner", "OPENAI_API_KEY").unwrap());
        assert_eq!(events(&s).last().unwrap().0, "secret.removed");
        assert!(s.vault_entries().unwrap().is_empty());
    }

    #[test]
    fn a_second_holder_unwraps_the_same_team_key() {
        let (_home, s) = new_store();
        s.vault_set("owner", "GH_TOKEN", b"ghp_x").unwrap();
        let device = DeviceKey::generate().unwrap();
        s.vault_wrap_to("owner", "device:laptop", "device", &device.public_b64())
            .unwrap();
        let holders = s.vault_holders().unwrap();
        assert_eq!(holders.len(), 2);
        let wrapped: String = s
            .conn()
            .unwrap()
            .query_row(
                "SELECT wrapped FROM vault_keys WHERE holder = 'device:laptop'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let data = device.unwrap(&wrapped).unwrap();
        let (ct, nonce): (String, String) = s
            .conn()
            .unwrap()
            .query_row(
                "SELECT ciphertext, nonce FROM vault_secrets WHERE name = 'GH_TOKEN'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let plain = vault::open(
            &data,
            "GH_TOKEN",
            &Sealed {
                ciphertext: ct,
                nonce,
            },
        )
        .unwrap();
        assert_eq!(&plain[..], b"ghp_x");
        assert!(
            s.vault_wrap_to("owner", "x", "stranger", &device.public_b64())
                .is_err()
        );
    }
}
