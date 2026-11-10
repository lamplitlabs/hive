//! The team secret vault (ADR-070 rule 4): ciphertext and wrapped keys only.
//! A value is never stored in the clear, and the team's data key is stored
//! only wrapped to each holder's public key (pulse-hand-core `vault.rs`).

pub const VAULT_DDL: &[&str] = &[
    r####"CREATE TABLE IF NOT EXISTS vault_secrets (
  name TEXT PRIMARY KEY,               -- the variable the job's worker sees, e.g. ANTHROPIC_API_KEY
  ciphertext TEXT NOT NULL,            -- base64 ChaCha20-Poly1305 ciphertext and tag; the name is its associated data
  nonce TEXT NOT NULL,                 -- base64, fresh for every write
  created_by TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL,
  updated_by TEXT NOT NULL DEFAULT '',
  updated_at TEXT NOT NULL
)"####,
    r####"CREATE TABLE IF NOT EXISTS vault_keys (
  holder TEXT PRIMARY KEY,             -- who can unwrap: brain:<host>, device:<id>, owner
  kind TEXT NOT NULL DEFAULT 'brain' CHECK (kind IN ('brain','device','owner')),
  public_key TEXT NOT NULL,            -- base64 X25519 public key of the holder
  wrapped TEXT NOT NULL,               -- the team data key wrapped to that key (x25519.v1.)
  created_by TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL
)"####,
];
