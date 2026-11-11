//! Passkey-signed writes (ADR-070 rule 5): the brain checks a WebAuthn
//! assertion itself before a write that reached it through the hosted front
//! changes anything, so the front - which only relays bytes - cannot forge
//! one.
//!
//! The browser asks the brain for a nonce (`GET /api/v1/nonce`), computes
//! the write's challenge with [`write_challenge`] - SHA-256 over the nonce,
//! the method, the path and the exact body - and calls
//! `navigator.credentials.get({publicKey: {challenge, rpId, userVerification:
//! "required"}})`. It sends the write with the assertion in four headers
//! ([`CREDENTIAL_HEADER`], [`AUTHENTICATOR_HEADER`], [`CLIENT_DATA_HEADER`],
//! [`SIGNATURE_HEADER`], base64url) and the nonce in [`NONCE_HEADER`].
//! [`verify_assertion`] runs the relying party's checks of WebAuthn Level 2
//! §7.2 that apply to a known credential: the client data's type, challenge
//! and origin, the authenticator data's RP ID hash and its user-present and
//! user-verified flags, the signature counter, and the ES256 signature over
//! `authenticatorData || SHA-256(clientDataJSON)` under the member's
//! registered key. Spending the nonce once is the store's
//! (`Store::nonce_take`).

use crate::p256;
use base64::Engine;
use sha2::{Digest, Sha256};

pub const NONCE_HEADER: &str = "X-Hive-Nonce";
pub const CREDENTIAL_HEADER: &str = "X-Hive-Passkey-Credential";
pub const AUTHENTICATOR_HEADER: &str = "X-Hive-Passkey-Authenticator";
pub const CLIENT_DATA_HEADER: &str = "X-Hive-Passkey-Client-Data";
pub const SIGNATURE_HEADER: &str = "X-Hive-Passkey-Signature";

/// What a write's challenge is computed over, first: a version word, so a
/// later shape never verifies under this one.
pub const CHALLENGE_DOMAIN: &str = "hive-write-v1";

/// How long a nonce the brain issued stays good.
pub const NONCE_SECONDS: i64 = 300;

const FLAG_USER_PRESENT: u8 = 0x01;
const FLAG_USER_VERIFIED: u8 = 0x04;

/// base64url without padding, the encoding WebAuthn uses throughout.
pub fn b64url(data: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}

/// Decodes base64url, padded or not.
pub fn from_b64url(text: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(text.trim().trim_end_matches('='))
        .ok()
}

/// A fresh nonce: 32 random bytes, base64url.
pub fn new_nonce() -> String {
    let bytes: [u8; 32] = rand::random();
    b64url(&bytes)
}

/// The challenge a write is signed over: SHA-256 of
/// `hive-write-v1\n<nonce>\n<METHOD> <path?query>\n` followed by the body
/// bytes as sent. Binding the method, path and body means a signature made
/// for one write verifies for no other.
pub fn write_challenge(nonce: &str, method: &str, path: &str, body: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(CHALLENGE_DOMAIN.as_bytes());
    h.update(b"\n");
    h.update(nonce.as_bytes());
    h.update(b"\n");
    h.update(method.to_ascii_uppercase().as_bytes());
    h.update(b" ");
    h.update(path.as_bytes());
    h.update(b"\n");
    h.update(body);
    h.finalize().into()
}

/// One WebAuthn assertion as the browser returned it, decoded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Assertion {
    pub credential_id: String,
    pub authenticator_data: Vec<u8>,
    pub client_data_json: Vec<u8>,
    pub signature: Vec<u8>,
}

/// What the brain expects of an assertion.
#[derive(Clone, Debug)]
pub struct Expected<'a> {
    /// The challenge the brain computed for this write.
    pub challenge: &'a [u8],
    /// The front's origin, `https://team.example.com` (`hub.allow_remote`).
    pub origin: &'a str,
    /// The member's registered public key, SubjectPublicKeyInfo DER.
    pub public_key: &'a [u8],
    /// The signature counter the brain last saw for this credential.
    pub sign_count: u32,
}

/// Why an assertion was refused, in words the hub shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal(pub String);

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn refuse<T>(why: &str) -> Result<T, Refusal> {
    Err(Refusal(why.to_string()))
}

/// The host of an `https://host[:port]` origin, lower-case.
pub fn origin_host(origin: &str) -> Option<String> {
    let url = url_host(origin)?;
    Some(url.to_ascii_lowercase())
}

fn url_host(origin: &str) -> Option<&str> {
    let rest = origin.strip_prefix("https://")?;
    let host = rest.split(':').next()?;
    if host.is_empty() || rest.contains('/') {
        return None;
    }
    Some(host)
}

/// Whether `rp_id_hash` is SHA-256 of the origin's host or of a parent
/// domain of it (WebAuthn lets a relying party use a registrable suffix of
/// its origin's host: `example.com` for `team.example.com`). A bare
/// top-level label never counts.
fn rp_id_matches(rp_id_hash: &[u8], origin: &str) -> bool {
    let Some(host) = origin_host(origin) else {
        return false;
    };
    let mut candidate = host.as_str();
    loop {
        if Sha256::digest(candidate.as_bytes()).as_slice() == rp_id_hash {
            return true;
        }
        match candidate.split_once('.') {
            Some((_, parent)) if parent.contains('.') => candidate = parent,
            _ => return false,
        }
    }
}

/// Verifies `a` against `e` and returns the authenticator's new signature
/// counter, which the caller stores.
pub fn verify_assertion(a: &Assertion, e: &Expected) -> Result<u32, Refusal> {
    let Ok(client) = serde_json::from_slice::<serde_json::Value>(&a.client_data_json) else {
        return refuse("the passkey's client data is not JSON");
    };
    if client.get("type").and_then(|v| v.as_str()) != Some("webauthn.get") {
        return refuse("the passkey's client data is not an assertion (type webauthn.get)");
    }
    let challenge = client
        .get("challenge")
        .and_then(|v| v.as_str())
        .and_then(from_b64url);
    if challenge.as_deref() != Some(e.challenge) {
        return refuse(
            "the passkey signed a different challenge: the nonce, method, path or body changed",
        );
    }
    let origin = client.get("origin").and_then(|v| v.as_str()).unwrap_or("");
    if origin.trim_end_matches('/') != e.origin.trim_end_matches('/') {
        return refuse("the passkey signed for another origin than the front's");
    }
    if client.get("crossOrigin").and_then(|v| v.as_bool()) == Some(true) {
        return refuse("the passkey signed inside a cross-origin frame");
    }
    let data = &a.authenticator_data;
    if data.len() < 37 {
        return refuse("the passkey's authenticator data is too short");
    }
    if !rp_id_matches(&data[..32], e.origin) {
        return refuse("the passkey belongs to another relying party than the front");
    }
    let flags = data[32];
    if flags & FLAG_USER_PRESENT == 0 {
        return refuse("the passkey did not see the member present");
    }
    if flags & FLAG_USER_VERIFIED == 0 {
        return refuse("the passkey did not verify the member (user verification is required)");
    }
    let count = u32::from_be_bytes([data[33], data[34], data[35], data[36]]);
    if (count != 0 || e.sign_count != 0) && count <= e.sign_count {
        return refuse(
            "the passkey's signature counter did not move forward; the credential may be cloned",
        );
    }
    let Some(key) = p256::PublicKey::parse(e.public_key) else {
        return refuse("the member's registered key is not a P-256 public key");
    };
    let mut signed = data.clone();
    signed.extend_from_slice(&Sha256::digest(&a.client_data_json));
    if !p256::verify(&key, &signed, &a.signature) {
        return refuse("the passkey signature does not verify under the member's key");
    }
    Ok(count)
}

/// Plays a member's passkey in tests: the assertion a browser would return
/// for `challenge` at `origin` with the private scalar `secret`.
pub fn assertion_for_tests(
    secret: &[u8; 32],
    credential_id: &str,
    origin: &str,
    challenge: &[u8],
    sign_count: u32,
) -> Assertion {
    let host = origin_host(origin).unwrap_or_default();
    let client = serde_json::json!({
        "type": "webauthn.get",
        "challenge": b64url(challenge),
        "origin": origin,
        "crossOrigin": false,
    });
    let client_data_json = serde_json::to_vec(&client).expect("client data");
    let mut authenticator_data = Sha256::digest(host.as_bytes()).to_vec();
    authenticator_data.push(FLAG_USER_PRESENT | FLAG_USER_VERIFIED);
    authenticator_data.extend_from_slice(&sign_count.to_be_bytes());
    let mut signed = authenticator_data.clone();
    signed.extend_from_slice(&Sha256::digest(&client_data_json));
    let signature = p256::sign_for_tests(secret, &signed).expect("test key signs");
    Assertion {
        credential_id: credential_id.to_string(),
        authenticator_data,
        client_data_json,
        signature,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: &str = "https://team.example.com";
    const SECRET: [u8; 32] = [7; 32];
    const OTHER: [u8; 32] = [9; 32];

    fn key(secret: &[u8; 32]) -> Vec<u8> {
        p256::public_key_for_tests(secret).unwrap().to_spki()
    }

    #[test]
    fn a_valid_assertion_verifies_and_returns_its_counter() {
        let challenge = write_challenge("n1", "post", "/api/v1/settings", br#"{"a":1}"#);
        let a = assertion_for_tests(&SECRET, "cred", ORIGIN, &challenge, 5);
        let pk = key(&SECRET);
        let e = Expected {
            challenge: &challenge,
            origin: ORIGIN,
            public_key: &pk,
            sign_count: 4,
        };
        assert_eq!(verify_assertion(&a, &e), Ok(5));
        // Authenticators that keep no counter send zero every time.
        let a0 = assertion_for_tests(&SECRET, "cred", ORIGIN, &challenge, 0);
        let e0 = Expected {
            sign_count: 0,
            ..e.clone()
        };
        assert_eq!(verify_assertion(&a0, &e0), Ok(0));
        // A parent domain is a valid RP ID for the origin.
        let mut parent = a.clone();
        parent.authenticator_data[..32].copy_from_slice(&Sha256::digest(b"example.com"));
        let mut signed = parent.authenticator_data.clone();
        signed.extend_from_slice(&Sha256::digest(&parent.client_data_json));
        parent.signature = p256::sign_for_tests(&SECRET, &signed).unwrap();
        assert_eq!(verify_assertion(&parent, &e), Ok(5));
    }

    #[test]
    fn a_wrong_key_another_write_or_another_origin_is_refused() {
        let challenge = write_challenge("n1", "POST", "/api/v1/decide", b"{}");
        let pk = key(&SECRET);
        let e = Expected {
            challenge: &challenge,
            origin: ORIGIN,
            public_key: &pk,
            sign_count: 0,
        };
        let wrong_key = assertion_for_tests(&OTHER, "cred", ORIGIN, &challenge, 1);
        let err = verify_assertion(&wrong_key, &e).unwrap_err().0;
        assert!(err.contains("does not verify"), "{err}");

        let other_body = write_challenge("n1", "POST", "/api/v1/decide", b"{\"x\":1}");
        let a = assertion_for_tests(&SECRET, "cred", ORIGIN, &other_body, 1);
        let err = verify_assertion(&a, &e).unwrap_err().0;
        assert!(err.contains("different challenge"), "{err}");

        let a = assertion_for_tests(&SECRET, "cred", "https://evil.example.net", &challenge, 1);
        let err = verify_assertion(&a, &e).unwrap_err().0;
        assert!(err.contains("another origin"), "{err}");

        let mut a = assertion_for_tests(&SECRET, "cred", ORIGIN, &challenge, 1);
        a.authenticator_data[32] = FLAG_USER_PRESENT;
        let err = verify_assertion(&a, &e).unwrap_err().0;
        assert!(err.contains("user verification"), "{err}");

        let a = assertion_for_tests(&SECRET, "cred", ORIGIN, &challenge, 3);
        let stale = Expected {
            sign_count: 3,
            ..e.clone()
        };
        let err = verify_assertion(&a, &stale).unwrap_err().0;
        assert!(err.contains("counter"), "{err}");

        let mut tampered = assertion_for_tests(&SECRET, "cred", ORIGIN, &challenge, 1);
        tampered.authenticator_data[36] ^= 0x10;
        let err = verify_assertion(&tampered, &e).unwrap_err().0;
        assert!(err.contains("does not verify"), "{err}");
    }

    #[test]
    fn challenges_bind_every_part_of_the_write() {
        let base = write_challenge("n", "POST", "/p", b"b");
        assert_eq!(base, write_challenge("n", "post", "/p", b"b"));
        for other in [
            write_challenge("m", "POST", "/p", b"b"),
            write_challenge("n", "DELETE", "/p", b"b"),
            write_challenge("n", "POST", "/q", b"b"),
            write_challenge("n", "POST", "/p", b"c"),
        ] {
            assert_ne!(base, other);
        }
        let a = new_nonce();
        assert_eq!(from_b64url(&a).unwrap().len(), 32);
        assert_ne!(a, new_nonce());
        assert_eq!(
            origin_host("https://Team.Example.com:8443").as_deref(),
            Some("team.example.com")
        );
        assert_eq!(origin_host("http://team.example.com"), None);
        assert_eq!(origin_host("https://team.example.com/path"), None);
    }
}
