//! The team secret vault's cryptography (ADR-070 rule 4), shared by the
//! brain and the hand: the brain stores only what [`seal`] returns, and a
//! value is opened ([`open`]) only where a job runs, in memory, for that run.
//!
//! - A team **data key** (32 random bytes) seals each secret with
//!   ChaCha20-Poly1305; the secret's name is the associated data, so a sealed
//!   value copied under another name does not open.
//! - The data key is never stored in the clear: it is **wrapped** to each
//!   holder's X25519 public key (a device of the team, or the brain itself
//!   for LocalExec). A wrap is an ephemeral X25519 agreement, HKDF-SHA256
//!   over the shared secret with both public keys as salt, and
//!   ChaCha20-Poly1305 over the data key; only the holder's private key
//!   unwraps it.
//!
//! Every primitive is aws-lc-rs (the library rustls already builds here); no
//! cipher is written by hand. Key and plaintext buffers are zeroized when
//! dropped.

use anyhow::{Result, anyhow, bail};
use aws_lc_rs::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use aws_lc_rs::agreement::{self, PrivateKey, UnparsedPublicKey, X25519};
use aws_lc_rs::encoding::AsBigEndian;
use aws_lc_rs::hkdf;
use base64::Engine as _;
pub use zeroize::{Zeroize, Zeroizing};

/// Length of a data key and of an X25519 key, in bytes.
pub const KEY_LEN: usize = 32;
/// The prefix of a wrapped data key: the format can change without
/// guessing what an old row holds.
pub const WRAP_PREFIX: &str = "x25519.v1.";
const SEAL_AAD: &str = "hive-vault-v1:";
const WRAP_INFO: &[u8] = b"hive-vault-wrap-v1";
const TAG_LEN: usize = 16;

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD_NO_PAD
}

/// Bytes from the system's secure random source.
pub fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut out = [0u8; N];
    aws_lc_rs::rand::fill(&mut out).map_err(|_| anyhow!("no secure random bytes"))?;
    Ok(out)
}

/// The team's data key: it seals and opens every secret of the vault.
/// Its bytes are wiped when it is dropped.
pub struct DataKey(Zeroizing<[u8; KEY_LEN]>);

impl DataKey {
    /// A fresh random key; made once per team.
    pub fn generate() -> Result<Self> {
        Ok(Self(Zeroizing::new(random_bytes::<KEY_LEN>()?)))
    }

    fn aead(&self) -> Result<LessSafeKey> {
        let key = UnboundKey::new(&CHACHA20_POLY1305, self.0.as_slice())
            .map_err(|_| anyhow!("vault: data key rejected"))?;
        Ok(LessSafeKey::new(key))
    }
}

impl std::fmt::Debug for DataKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DataKey(..)")
    }
}

/// A sealed secret as the store keeps it: base64 of the ciphertext with its
/// tag, and of the nonce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sealed {
    pub ciphertext: String,
    pub nonce: String,
}

fn seal_aad(name: &str) -> Vec<u8> {
    format!("{SEAL_AAD}{name}").into_bytes()
}

/// Seals `value` as the secret `name` under the team's data key, with a
/// fresh random nonce.
pub fn seal(key: &DataKey, name: &str, value: &[u8]) -> Result<Sealed> {
    let nonce = random_bytes::<NONCE_LEN>()?;
    let mut buf = Zeroizing::new(value.to_vec());
    key.aead()?
        .seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(seal_aad(name)),
            &mut *buf,
        )
        .map_err(|_| anyhow!("vault: sealing {name} failed"))?;
    Ok(Sealed {
        ciphertext: b64().encode(&*buf),
        nonce: b64().encode(nonce),
    })
}

/// Opens the sealed secret `name`; the plaintext is wiped when dropped.
/// Fails when the key is not the one that sealed it, the row was changed, or
/// the value was sealed under another name.
pub fn open(key: &DataKey, name: &str, sealed: &Sealed) -> Result<Zeroizing<Vec<u8>>> {
    let nonce: [u8; NONCE_LEN] = b64()
        .decode(sealed.nonce.trim())
        .ok()
        .and_then(|n| n.try_into().ok())
        .ok_or_else(|| anyhow!("vault: {name}: the nonce is not {NONCE_LEN} bytes"))?;
    let mut buf = Zeroizing::new(
        b64()
            .decode(sealed.ciphertext.trim())
            .map_err(|_| anyhow!("vault: {name}: the ciphertext is not base64"))?,
    );
    if buf.len() < TAG_LEN {
        bail!("vault: {name}: the ciphertext is too short");
    }
    let n = key
        .aead()?
        .open_in_place(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(seal_aad(name)),
            &mut buf,
        )
        .map_err(|_| {
            anyhow!("vault: {name} does not open with this team key (changed, or sealed under another name or key)")
        })?
        .len();
    buf.truncate(n);
    Ok(buf)
}

/// A holder's X25519 key pair: the private half stays on its machine (a
/// mode-0600 file), the public half is what the data key is wrapped to.
pub struct DeviceKey {
    secret: Zeroizing<[u8; KEY_LEN]>,
    public: [u8; KEY_LEN],
}

impl std::fmt::Debug for DeviceKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DeviceKey({})", self.public_b64())
    }
}

fn private_key(secret: &[u8]) -> Result<PrivateKey> {
    PrivateKey::from_private_key(&X25519, secret).map_err(|_| anyhow!("vault: not an X25519 key"))
}

fn public_of(key: &PrivateKey) -> Result<[u8; KEY_LEN]> {
    key.compute_public_key()
        .map_err(|_| anyhow!("vault: no public key"))?
        .as_ref()
        .try_into()
        .map_err(|_| anyhow!("vault: the public key is not {KEY_LEN} bytes"))
}

impl DeviceKey {
    /// A fresh key pair.
    pub fn generate() -> Result<Self> {
        let key = PrivateKey::generate(&X25519).map_err(|_| anyhow!("vault: no X25519 key"))?;
        let seed =
            AsBigEndian::<aws_lc_rs::encoding::Curve25519SeedBin<'static>>::as_be_bytes(&key)
                .map_err(|_| anyhow!("vault: the X25519 key has no bytes"))?;
        let mut secret = Zeroizing::new([0u8; KEY_LEN]);
        let raw: &[u8] = seed.as_ref();
        if raw.len() != KEY_LEN {
            bail!("vault: the X25519 key is not {KEY_LEN} bytes");
        }
        secret.copy_from_slice(raw);
        Self::from_secret(&secret[..])
    }

    /// The key pair of these private-key bytes (base64 or raw 32 bytes).
    pub fn from_secret(secret: &[u8]) -> Result<Self> {
        let mut bytes = Zeroizing::new([0u8; KEY_LEN]);
        if secret.len() == KEY_LEN {
            bytes.copy_from_slice(secret);
        } else {
            let text = std::str::from_utf8(secret)
                .map_err(|_| anyhow!("vault: the device key is not base64"))?;
            let mut raw = Zeroizing::new(
                b64()
                    .decode(text.trim())
                    .map_err(|_| anyhow!("vault: the device key is not base64"))?,
            );
            if raw.len() != KEY_LEN {
                raw.zeroize();
                bail!("vault: the device key is not {KEY_LEN} bytes");
            }
            bytes.copy_from_slice(&raw);
        }
        let public = public_of(&private_key(&bytes[..])?)?;
        Ok(Self {
            secret: bytes,
            public,
        })
    }

    /// The private key as base64, for the holder's own key file only.
    pub fn secret_b64(&self) -> Zeroizing<String> {
        Zeroizing::new(b64().encode(&self.secret[..]))
    }

    /// The public key as base64: what the brain stores beside a wrap.
    pub fn public_b64(&self) -> String {
        b64().encode(self.public)
    }

    /// Unwraps the team's data key from a wrap made to this key.
    pub fn unwrap(&self, wrapped: &str) -> Result<DataKey> {
        let body = wrapped
            .trim()
            .strip_prefix(WRAP_PREFIX)
            .ok_or_else(|| anyhow!("vault: the wrapped key is not {WRAP_PREFIX}"))?;
        let mut raw = Zeroizing::new(
            b64()
                .decode(body)
                .map_err(|_| anyhow!("vault: the wrapped key is not base64"))?,
        );
        if raw.len() != KEY_LEN + NONCE_LEN + KEY_LEN + TAG_LEN {
            bail!("vault: the wrapped key has the wrong length");
        }
        let (eph, rest) = raw.split_at_mut(KEY_LEN);
        let (nonce, sealed) = rest.split_at_mut(NONCE_LEN);
        let eph: [u8; KEY_LEN] = eph.try_into().expect("split at KEY_LEN");
        let nonce: [u8; NONCE_LEN] = nonce.try_into().expect("split at NONCE_LEN");
        let kek = wrap_key(&private_key(&self.secret[..])?, &eph, &eph, &self.public)?;
        let opened = kek
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(self.public),
                sealed,
            )
            .map_err(|_| anyhow!("vault: this device holds no wrap of the team key"))?;
        let mut key = Zeroizing::new([0u8; KEY_LEN]);
        key.copy_from_slice(opened);
        Ok(DataKey(key))
    }
}

/// The key-encryption key of one wrap: X25519 between `mine` and `peer`,
/// then HKDF-SHA256 salted with the ephemeral and the recipient's public
/// keys.
fn wrap_key(
    mine: &PrivateKey,
    peer: &[u8; KEY_LEN],
    eph_public: &[u8; KEY_LEN],
    recipient: &[u8; KEY_LEN],
) -> Result<LessSafeKey> {
    let mut salt = [0u8; 2 * KEY_LEN];
    salt[..KEY_LEN].copy_from_slice(eph_public);
    salt[KEY_LEN..].copy_from_slice(recipient);
    agreement::agree(
        mine,
        UnparsedPublicKey::new(&X25519, peer),
        anyhow!("vault: the X25519 agreement failed"),
        |shared| {
            let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, &salt).extract(shared);
            let info = [WRAP_INFO];
            let okm = prk
                .expand(&info, &CHACHA20_POLY1305)
                .map_err(|_| anyhow!("vault: HKDF failed"))?;
            Ok(LessSafeKey::new(UnboundKey::from(okm)))
        },
    )
}

/// Wraps the team's data key to the holder whose public key is
/// `recipient_b64`: only that holder's private key unwraps it.
pub fn wrap(key: &DataKey, recipient_b64: &str) -> Result<String> {
    let recipient: [u8; KEY_LEN] = b64()
        .decode(recipient_b64.trim())
        .ok()
        .and_then(|k| k.try_into().ok())
        .ok_or_else(|| anyhow!("vault: the recipient's public key is not {KEY_LEN} bytes"))?;
    let eph = PrivateKey::generate(&X25519).map_err(|_| anyhow!("vault: no X25519 key"))?;
    let eph_public = public_of(&eph)?;
    let kek = wrap_key(&eph, &recipient, &eph_public, &recipient)?;
    let nonce = random_bytes::<NONCE_LEN>()?;
    let mut buf = Zeroizing::new(key.0.to_vec());
    kek.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce),
        Aad::from(recipient),
        &mut *buf,
    )
    .map_err(|_| anyhow!("vault: wrapping the team key failed"))?;
    let mut out = Vec::with_capacity(KEY_LEN + NONCE_LEN + buf.len());
    out.extend_from_slice(&eph_public);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&buf);
    Ok(format!("{WRAP_PREFIX}{}", b64().encode(out)))
}

/// This machine's short host name: the `brain:<host>` holder of the key.
pub fn host() -> String {
    #[cfg(unix)]
    let raw = nix::unistd::gethostname()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_default();
    // Off unix: the name the system puts in the environment (Windows).
    #[cfg(not(unix))]
    let raw = std::env::var("COMPUTERNAME").unwrap_or_default();
    let short = raw.split('.').next().unwrap_or_default().to_string();
    if short.is_empty() {
        "host".into()
    } else {
        short
    }
}

/// A secret name: what the recipe's `secrets:` lists and the variable the
/// job's worker sees, so shell-safe upper case like an env name.
pub fn name_ok(name: &str) -> bool {
    crate::names::RECIPE_ENV_NAME_RE.is_match(name) && name.len() <= 64
}

/// Why `name` cannot be a secret, or `None` when it can. Names Hive itself
/// sets for a worker are taken.
pub fn name_problem(name: &str) -> Option<String> {
    if !name_ok(name) {
        return Some(format!(
            "`{name}` is not a secret name: upper-case letters, digits and `_`, starting with a letter or `_`, at most 64 characters"
        ));
    }
    if name.starts_with("HIVE_")
        || ["PATH", "HOME", "TMPDIR", "GIT_DIR", "GIT_INDEX_FILE"].contains(&name)
    {
        return Some(format!(
            "`{name}` is a name Hive sets for every worker; pick another"
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sealed_secret_opens_under_its_name_and_key_only() {
        let key = DataKey::generate().unwrap();
        let sealed = seal(&key, "ANTHROPIC_API_KEY", b"sk-test-value").unwrap();
        assert!(!sealed.ciphertext.contains("sk-test-value"));
        let opened = open(&key, "ANTHROPIC_API_KEY", &sealed).unwrap();
        assert_eq!(&opened[..], b"sk-test-value");
        // Another name, another key, a changed row: none opens.
        assert!(open(&key, "OPENAI_API_KEY", &sealed).is_err());
        let other = DataKey::generate().unwrap();
        assert!(open(&other, "ANTHROPIC_API_KEY", &sealed).is_err());
        let mut changed = sealed.clone();
        changed.ciphertext.replace_range(0..2, "AA");
        assert!(open(&key, "ANTHROPIC_API_KEY", &changed).is_err());
        // Two seals of one value never share a nonce or a ciphertext.
        let again = seal(&key, "ANTHROPIC_API_KEY", b"sk-test-value").unwrap();
        assert_ne!(again.nonce, sealed.nonce);
        assert_ne!(again.ciphertext, sealed.ciphertext);
        // An empty value round-trips too.
        let empty = seal(&key, "EMPTY", b"").unwrap();
        assert!(open(&key, "EMPTY", &empty).unwrap().is_empty());
    }

    #[test]
    fn the_team_key_wrapped_to_two_devices_unwraps_on_each_and_not_on_a_third() {
        let team = DataKey::generate().unwrap();
        let sealed = seal(&team, "GH_TOKEN", b"ghp_example").unwrap();
        let laptop = DeviceKey::generate().unwrap();
        let hand = DeviceKey::generate().unwrap();
        let stranger = DeviceKey::generate().unwrap();
        let to_laptop = wrap(&team, &laptop.public_b64()).unwrap();
        let to_hand = wrap(&team, &hand.public_b64()).unwrap();
        assert!(to_laptop.starts_with(WRAP_PREFIX));
        assert_ne!(to_laptop, to_hand);
        for (device, wrapped) in [(&laptop, &to_laptop), (&hand, &to_hand)] {
            let key = device.unwrap(wrapped).unwrap();
            assert_eq!(
                &open(&key, "GH_TOKEN", &sealed).unwrap()[..],
                b"ghp_example"
            );
        }
        // A third device unwraps neither, and a device cannot use another's wrap.
        assert!(stranger.unwrap(&to_laptop).is_err());
        assert!(stranger.unwrap(&to_hand).is_err());
        assert!(laptop.unwrap(&to_hand).is_err());
        // Garbage is refused, not panicked on.
        assert!(laptop.unwrap("x25519.v1.AAAA").is_err());
        assert!(laptop.unwrap("plain").is_err());
        assert!(wrap(&team, "short").is_err());
    }

    #[test]
    fn a_device_key_round_trips_through_its_file_text() {
        let key = DeviceKey::generate().unwrap();
        let again = DeviceKey::from_secret(key.secret_b64().as_bytes()).unwrap();
        assert_eq!(again.public_b64(), key.public_b64());
        let team = DataKey::generate().unwrap();
        let wrapped = wrap(&team, &key.public_b64()).unwrap();
        assert!(again.unwrap(&wrapped).is_ok());
        assert!(DeviceKey::from_secret(b"not base64 at all!").is_err());
        assert!(format!("{key:?}").starts_with("DeviceKey("));
        assert_eq!(format!("{team:?}"), "DataKey(..)");
    }

    #[test]
    fn secret_names_are_shell_safe_and_never_hive_s_own() {
        for ok in ["ANTHROPIC_API_KEY", "GH_TOKEN", "_X", "A1"] {
            assert_eq!(name_problem(ok), None, "{ok}");
        }
        for bad in [
            "lower",
            "1ABC",
            "A-B",
            "",
            "HIVE_JOB",
            "PATH",
            &"A".repeat(65),
        ] {
            assert!(name_problem(bad).is_some(), "{bad}");
        }
    }
}
