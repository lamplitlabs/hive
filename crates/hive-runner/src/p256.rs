//! ECDSA over NIST P-256 with SHA-256 (WebAuthn's ES256, COSE alg -7):
//! the verification the brain runs on a member's passkey signature
//! (ADR-070 rule 5), in plain Rust over the workspace's own `sha2`, so the
//! brain checks a write itself with no new dependency.
//!
//! Only public values pass through here - a public key, a message and a
//! signature - so the arithmetic need not be constant-time. Signing exists
//! for tests and fixtures only ([`sign_for_tests`]): the brain never holds
//! a member's private key.
//!
//! Field and scalar arithmetic are Montgomery multiplication over four
//! 64-bit limbs; points are Jacobian with the curve's a = -3.

use sha2::{Digest, Sha256};

type Limbs = [u64; 4];

const P_HEX: &str = "ffffffff00000001000000000000000000000000ffffffffffffffffffffffff";
const N_HEX: &str = "ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551";
const B_HEX: &str = "5ac635d8aa3a93e7b3ebbd55769886bc651d06b0cc53b0f63bce3c3e27d2604b";
const GX_HEX: &str = "6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296";
const GY_HEX: &str = "4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5";

/// The DER prefix of a SubjectPublicKeyInfo for an uncompressed P-256 key
/// (id-ecPublicKey, prime256v1, BIT STRING of 66 bytes), which is what a
/// browser's `AuthenticatorAttestationResponse.getPublicKey()` returns for
/// ES256; the 65-byte point `04 || X || Y` follows it.
const SPKI_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
    0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

fn limbs_from_be(bytes: &[u8; 32]) -> Limbs {
    let mut out = [0u64; 4];
    for (i, limb) in out.iter_mut().enumerate() {
        let start = 32 - 8 * (i + 1);
        let mut word = [0u8; 8];
        word.copy_from_slice(&bytes[start..start + 8]);
        *limb = u64::from_be_bytes(word);
    }
    out
}

fn limbs_to_be(a: &Limbs) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, limb) in a.iter().enumerate() {
        let start = 32 - 8 * (i + 1);
        out[start..start + 8].copy_from_slice(&limb.to_be_bytes());
    }
    out
}

fn limbs_from_hex(h: &str) -> Limbs {
    let raw = hex::decode(h).expect("curve constant is hex");
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&raw);
    limbs_from_be(&bytes)
}

fn is_zero(a: &Limbs) -> bool {
    a.iter().all(|&l| l == 0)
}

/// a >= b
fn geq(a: &Limbs, b: &Limbs) -> bool {
    for i in (0..4).rev() {
        if a[i] != b[i] {
            return a[i] > b[i];
        }
    }
    true
}

fn add_raw(a: &Limbs, b: &Limbs) -> (Limbs, bool) {
    let mut out = [0u64; 4];
    let mut carry = false;
    for i in 0..4 {
        let (s1, c1) = a[i].overflowing_add(b[i]);
        let (s2, c2) = s1.overflowing_add(carry as u64);
        out[i] = s2;
        carry = c1 || c2;
    }
    (out, carry)
}

fn sub_raw(a: &Limbs, b: &Limbs) -> (Limbs, bool) {
    let mut out = [0u64; 4];
    let mut borrow = false;
    for i in 0..4 {
        let (d1, b1) = a[i].overflowing_sub(b[i]);
        let (d2, b2) = d1.overflowing_sub(borrow as u64);
        out[i] = d2;
        borrow = b1 || b2;
    }
    (out, borrow)
}

/// Arithmetic modulo an odd 256-bit modulus, in Montgomery form (R = 2^256).
struct Modulus {
    m: Limbs,
    /// -m^-1 mod 2^64
    inv: u64,
    /// R^2 mod m
    r2: Limbs,
    /// R mod m: one in Montgomery form
    one: Limbs,
}

impl Modulus {
    fn new(m: Limbs) -> Self {
        // Newton's iteration doubles the correct low bits each round.
        let mut x: u64 = 1;
        for _ in 0..7 {
            x = x.wrapping_mul(2u64.wrapping_sub(m[0].wrapping_mul(x)));
        }
        let inv = x.wrapping_neg();
        let mut v: Limbs = [1, 0, 0, 0];
        let mut one = [0u64; 4];
        for i in 0..512 {
            if i == 256 {
                one = v;
            }
            let (d, carry) = add_raw(&v, &v);
            v = if carry || geq(&d, &m) {
                sub_raw(&d, &m).0
            } else {
                d
            };
        }
        Self { m, inv, r2: v, one }
    }

    fn add(&self, a: &Limbs, b: &Limbs) -> Limbs {
        let (s, carry) = add_raw(a, b);
        if carry || geq(&s, &self.m) {
            sub_raw(&s, &self.m).0
        } else {
            s
        }
    }

    fn sub(&self, a: &Limbs, b: &Limbs) -> Limbs {
        let (d, borrow) = sub_raw(a, b);
        if borrow { add_raw(&d, &self.m).0 } else { d }
    }

    /// a * b * R^-1 mod m (CIOS).
    fn mul(&self, a: &Limbs, b: &Limbs) -> Limbs {
        let m = &self.m;
        let mut t = [0u64; 6];
        for &bi in b.iter() {
            let mut c: u64 = 0;
            for j in 0..4 {
                let uv = t[j] as u128 + (a[j] as u128) * (bi as u128) + c as u128;
                t[j] = uv as u64;
                c = (uv >> 64) as u64;
            }
            let uv = t[4] as u128 + c as u128;
            t[4] = uv as u64;
            t[5] = (uv >> 64) as u64;
            let q = t[0].wrapping_mul(self.inv);
            let uv = t[0] as u128 + (q as u128) * (m[0] as u128);
            let mut c = (uv >> 64) as u64;
            for j in 1..4 {
                let uv = t[j] as u128 + (q as u128) * (m[j] as u128) + c as u128;
                t[j - 1] = uv as u64;
                c = (uv >> 64) as u64;
            }
            let uv = t[4] as u128 + c as u128;
            t[3] = uv as u64;
            t[4] = t[5] + (uv >> 64) as u64;
        }
        let out = [t[0], t[1], t[2], t[3]];
        if t[4] != 0 || geq(&out, m) {
            sub_raw(&out, m).0
        } else {
            out
        }
    }

    fn sqr(&self, a: &Limbs) -> Limbs {
        self.mul(a, a)
    }

    /// Into Montgomery form; `a` must be below the modulus.
    fn to_mont(&self, a: &Limbs) -> Limbs {
        self.mul(a, &self.r2)
    }

    fn out_of_mont(&self, a: &Limbs) -> Limbs {
        self.mul(a, &[1, 0, 0, 0])
    }

    /// a^-1 by Fermat (the modulus is prime): a^(m-2), in Montgomery form.
    fn invert(&self, a: &Limbs) -> Limbs {
        let e = sub_raw(&self.m, &[2, 0, 0, 0]).0;
        let mut acc = self.one;
        for i in (0..256).rev() {
            acc = self.sqr(&acc);
            if (e[i / 64] >> (i % 64)) & 1 == 1 {
                acc = self.mul(&acc, a);
            }
        }
        acc
    }
}

struct Curve {
    p: Modulus,
    n: Modulus,
    /// b in Montgomery form
    b: Limbs,
    g: Point,
}

#[derive(Clone, Copy, Debug)]
struct Point {
    x: Limbs,
    y: Limbs,
    /// z == 0 is the point at infinity.
    z: Limbs,
}

impl Point {
    fn infinity() -> Self {
        Self {
            x: [0; 4],
            y: [0; 4],
            z: [0; 4],
        }
    }
    fn is_infinity(&self) -> bool {
        is_zero(&self.z)
    }
}

static CURVE: std::sync::LazyLock<Curve> = std::sync::LazyLock::new(|| {
    let p = Modulus::new(limbs_from_hex(P_HEX));
    let n = Modulus::new(limbs_from_hex(N_HEX));
    let b = p.to_mont(&limbs_from_hex(B_HEX));
    let g = Point {
        x: p.to_mont(&limbs_from_hex(GX_HEX)),
        y: p.to_mont(&limbs_from_hex(GY_HEX)),
        z: p.one,
    };
    Curve { p, n, b, g }
});

impl Curve {
    fn double(&self, a: &Point) -> Point {
        let f = &self.p;
        if a.is_infinity() || is_zero(&a.y) {
            return Point::infinity();
        }
        let delta = f.sqr(&a.z);
        let gamma = f.sqr(&a.y);
        let beta = f.mul(&a.x, &gamma);
        let t = f.mul(&f.sub(&a.x, &delta), &f.add(&a.x, &delta));
        let alpha = f.add(&f.add(&t, &t), &t);
        let beta2 = f.add(&beta, &beta);
        let beta4 = f.add(&beta2, &beta2);
        let beta8 = f.add(&beta4, &beta4);
        let x3 = f.sub(&f.sqr(&alpha), &beta8);
        let yz = f.add(&a.y, &a.z);
        let z3 = f.sub(&f.sub(&f.sqr(&yz), &gamma), &delta);
        let g1 = f.sqr(&gamma);
        let g2 = f.add(&g1, &g1);
        let g4 = f.add(&g2, &g2);
        let g8 = f.add(&g4, &g4);
        let y3 = f.sub(&f.mul(&alpha, &f.sub(&beta4, &x3)), &g8);
        Point {
            x: x3,
            y: y3,
            z: z3,
        }
    }

    fn add(&self, a: &Point, b: &Point) -> Point {
        let f = &self.p;
        if a.is_infinity() {
            return *b;
        }
        if b.is_infinity() {
            return *a;
        }
        let z1z1 = f.sqr(&a.z);
        let z2z2 = f.sqr(&b.z);
        let u1 = f.mul(&a.x, &z2z2);
        let u2 = f.mul(&b.x, &z1z1);
        let s1 = f.mul(&a.y, &f.mul(&b.z, &z2z2));
        let s2 = f.mul(&b.y, &f.mul(&a.z, &z1z1));
        let h = f.sub(&u2, &u1);
        let r = f.sub(&s2, &s1);
        if is_zero(&h) {
            return if is_zero(&r) {
                self.double(a)
            } else {
                Point::infinity()
            };
        }
        let h2 = f.sqr(&h);
        let h3 = f.mul(&h, &h2);
        let u1h2 = f.mul(&u1, &h2);
        let x3 = f.sub(&f.sub(&f.sqr(&r), &h3), &f.add(&u1h2, &u1h2));
        let y3 = f.sub(&f.mul(&r, &f.sub(&u1h2, &x3)), &f.mul(&s1, &h3));
        let z3 = f.mul(&f.mul(&a.z, &b.z), &h);
        Point {
            x: x3,
            y: y3,
            z: z3,
        }
    }

    /// k1*P1 + k2*P2 (Shamir's trick); k are plain integers.
    fn mul2(&self, k1: &Limbs, p1: &Point, k2: &Limbs, p2: &Point) -> Point {
        let both = self.add(p1, p2);
        let mut acc = Point::infinity();
        for i in (0..256).rev() {
            acc = self.double(&acc);
            let b1 = (k1[i / 64] >> (i % 64)) & 1 == 1;
            let b2 = (k2[i / 64] >> (i % 64)) & 1 == 1;
            acc = match (b1, b2) {
                (true, true) => self.add(&acc, &both),
                (true, false) => self.add(&acc, p1),
                (false, true) => self.add(&acc, p2),
                (false, false) => acc,
            };
        }
        acc
    }

    /// The affine x of a point, as a plain integer below p.
    fn affine_x(&self, a: &Point) -> Limbs {
        let f = &self.p;
        let zinv = f.invert(&a.z);
        f.out_of_mont(&f.mul(&a.x, &f.sqr(&zinv)))
    }

    fn affine(&self, a: &Point) -> (Limbs, Limbs) {
        let f = &self.p;
        let zinv = f.invert(&a.z);
        let zinv2 = f.sqr(&zinv);
        let x = f.out_of_mont(&f.mul(&a.x, &zinv2));
        let y = f.out_of_mont(&f.mul(&a.y, &f.mul(&zinv2, &zinv)));
        (x, y)
    }

    fn on_curve(&self, x: &Limbs, y: &Limbs) -> bool {
        let f = &self.p;
        let x3 = f.mul(&f.sqr(x), x);
        let three_x = f.add(&f.add(x, x), x);
        let rhs = f.add(&f.sub(&x3, &three_x), &self.b);
        f.sqr(y) == rhs
    }

    /// An integer reduced below n (one subtraction suffices for anything
    /// below 2^256, as n > 2^255).
    fn reduce_n(&self, a: &Limbs) -> Limbs {
        if geq(a, &self.n.m) {
            sub_raw(a, &self.n.m).0
        } else {
            *a
        }
    }
}

/// A P-256 public key that parsed and lies on the curve.
#[derive(Clone, Debug)]
pub struct PublicKey {
    point: Point,
}

impl PublicKey {
    /// Accepts the SubjectPublicKeyInfo DER of a P-256 key (what the
    /// browser's `getPublicKey()` hands back for ES256) or the bare
    /// uncompressed point `04 || X || Y`. Refuses anything off the curve.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let point = if bytes.len() == SPKI_PREFIX.len() + 65 && bytes.starts_with(&SPKI_PREFIX) {
            &bytes[SPKI_PREFIX.len()..]
        } else {
            bytes
        };
        if point.len() != 65 || point[0] != 0x04 {
            return None;
        }
        let mut xb = [0u8; 32];
        let mut yb = [0u8; 32];
        xb.copy_from_slice(&point[1..33]);
        yb.copy_from_slice(&point[33..65]);
        let (x, y) = (limbs_from_be(&xb), limbs_from_be(&yb));
        let c = &*CURVE;
        if geq(&x, &c.p.m) || geq(&y, &c.p.m) {
            return None;
        }
        let (xm, ym) = (c.p.to_mont(&x), c.p.to_mont(&y));
        if !c.on_curve(&xm, &ym) {
            return None;
        }
        Some(Self {
            point: Point {
                x: xm,
                y: ym,
                z: c.p.one,
            },
        })
    }

    /// The key as SubjectPublicKeyInfo DER.
    pub fn to_spki(&self) -> Vec<u8> {
        let c = &*CURVE;
        let mut out = SPKI_PREFIX.to_vec();
        out.push(0x04);
        out.extend_from_slice(&limbs_to_be(&c.p.out_of_mont(&self.point.x)));
        out.extend_from_slice(&limbs_to_be(&c.p.out_of_mont(&self.point.y)));
        out
    }
}

/// Reads an ASN.1 DER `SEQUENCE { INTEGER r, INTEGER s }` as two integers
/// below 2^256. Long-form lengths never occur at this size and are refused.
fn parse_der_signature(sig: &[u8]) -> Option<(Limbs, Limbs)> {
    if sig.len() < 8 || sig[0] != 0x30 || sig[1] as usize != sig.len() - 2 || sig[1] >= 0x80 {
        return None;
    }
    let mut rest = &sig[2..];
    let mut ints = Vec::with_capacity(2);
    for _ in 0..2 {
        if rest.len() < 2 || rest[0] != 0x02 || rest[1] >= 0x80 {
            return None;
        }
        let len = rest[1] as usize;
        if len == 0 || rest.len() < 2 + len {
            return None;
        }
        let mut value = &rest[2..2 + len];
        if value[0] & 0x80 != 0 {
            return None; // negative
        }
        while value.len() > 1 && value[0] == 0 {
            value = &value[1..];
        }
        if value.len() > 32 {
            return None;
        }
        let mut bytes = [0u8; 32];
        bytes[32 - value.len()..].copy_from_slice(value);
        ints.push(limbs_from_be(&bytes));
        rest = &rest[2 + len..];
    }
    if !rest.is_empty() {
        return None;
    }
    Some((ints[0], ints[1]))
}

fn der_integer(out: &mut Vec<u8>, a: &Limbs) {
    let bytes = limbs_to_be(a);
    let mut v: &[u8] = &bytes;
    while v.len() > 1 && v[0] == 0 {
        v = &v[1..];
    }
    let pad = v[0] & 0x80 != 0;
    out.push(0x02);
    out.push((v.len() + pad as usize) as u8);
    if pad {
        out.push(0);
    }
    out.extend_from_slice(v);
}

fn digest_scalar(msg: &[u8]) -> Limbs {
    let mut h = [0u8; 32];
    h.copy_from_slice(&Sha256::digest(msg));
    CURVE.reduce_n(&limbs_from_be(&h))
}

/// Whether `der_sig` is a valid ECDSA P-256 / SHA-256 signature of `msg`
/// under `key`. A malformed signature is simply not valid.
pub fn verify(key: &PublicKey, msg: &[u8], der_sig: &[u8]) -> bool {
    let c = &*CURVE;
    let Some((r, s)) = parse_der_signature(der_sig) else {
        return false;
    };
    if is_zero(&r) || is_zero(&s) || geq(&r, &c.n.m) || geq(&s, &c.n.m) {
        return false;
    }
    let e = digest_scalar(msg);
    let n = &c.n;
    let w = n.invert(&n.to_mont(&s));
    let u1 = n.out_of_mont(&n.mul(&n.to_mont(&e), &w));
    let u2 = n.out_of_mont(&n.mul(&n.to_mont(&r), &w));
    let point = c.mul2(&u1, &c.g, &u2, &key.point);
    if point.is_infinity() {
        return false;
    }
    c.reduce_n(&c.affine_x(&point)) == r
}

fn secret_scalar(secret: &[u8; 32]) -> Option<Limbs> {
    let d = limbs_from_be(secret);
    if is_zero(&d) || geq(&d, &CURVE.n.m) {
        None
    } else {
        Some(d)
    }
}

/// The public key of a private scalar. For tests and fixtures, like
/// [`sign_for_tests`].
pub fn public_key_for_tests(secret: &[u8; 32]) -> Option<PublicKey> {
    let c = &*CURVE;
    let d = secret_scalar(secret)?;
    let q = c.mul2(&d, &c.g, &[0; 4], &c.g);
    let (x, y) = c.affine(&q);
    Some(PublicKey {
        point: Point {
            x: c.p.to_mont(&x),
            y: c.p.to_mont(&y),
            z: c.p.one,
        },
    })
}

/// An ECDSA signature (DER) of `msg` with a private scalar, the nonce
/// derived from the key and the message. It exists so tests can play the
/// browser's passkey; the brain only ever verifies.
pub fn sign_for_tests(secret: &[u8; 32], msg: &[u8]) -> Option<Vec<u8>> {
    let c = &*CURVE;
    let d = secret_scalar(secret)?;
    let e = digest_scalar(msg);
    let n = &c.n;
    for counter in 0u32..16 {
        let mut h = Sha256::new();
        h.update(secret);
        h.update(Sha256::digest(msg));
        h.update(counter.to_be_bytes());
        let mut kb = [0u8; 32];
        kb.copy_from_slice(&h.finalize());
        let k = c.reduce_n(&limbs_from_be(&kb));
        if is_zero(&k) {
            continue;
        }
        let point = c.mul2(&k, &c.g, &[0; 4], &c.g);
        let r = c.reduce_n(&c.affine_x(&point));
        if is_zero(&r) {
            continue;
        }
        let rd = n.mul(&n.to_mont(&r), &n.to_mont(&d));
        let sum = n.add(&n.to_mont(&e), &rd);
        let s = n.out_of_mont(&n.mul(&n.invert(&n.to_mont(&k)), &sum));
        if is_zero(&s) {
            continue;
        }
        let mut body = Vec::with_capacity(72);
        der_integer(&mut body, &r);
        der_integer(&mut body, &s);
        let mut out = vec![0x30, body.len() as u8];
        out.extend_from_slice(&body);
        return Some(out);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    /// Keys and signatures made by Python's `cryptography` (OpenSSL), so
    /// this verifier is checked against an implementation it shares no
    /// code with: (private scalar, SPKI base64url, message, DER signature).
    /// The messages are what was signed, from before the rename: they
    /// keep saying "pulse", or the signatures stop matching.
    const OPENSSL_VECTORS: &[(&str, &str, &str, &str)] = &[
        (
            "adea26bf1c8fefa5fac1be6f7c7f235d93e9a30f43c0f97fe0dce27cb466962e",
            "MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEiIsFLzYMjWBNznztq5-m16moYflHEUS50ADh_3pJhIuKp01NhXsrRm_MDu4F7bRWA6Fv5hgAe-vuw4xVywrbEA",
            "pulse vector 0",
            "3046022100eb7130fc0ff1149a0706689d44603267ad77d72a87b9aa25a41eee694540733e022100f7640254f403e8f51fcd254919a7e970e5bfe92f9291be682adc1a371b614206",
        ),
        (
            "50b09f602d5120dcdc6ec403e867003c1342a4b4797f141cf881bdc1b40cf7be",
            "MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAE2ykmuQ_xIQV-ZcvgC6ajTVBz2J0nykSE_CIkMXDBXEvvyLUg2a5slr-zIXgIzAyF_FfLIAUPs_YkVj1lY3IpCw",
            "pulse vector 1",
            "3046022100c51f4f2d863af19135eb05b2fe011e5845b54a6894f922e18fb5fd6ce55cba47022100c3e51322afb210ed7463b2456100d6ec6c8b6c154e7118cd1c15a3f58d1bd4fe",
        ),
        (
            "7de985a7dda8fb9891d2c12aa912d0b974afd4965a48a5c13fea973a0324f27c",
            "MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEHmG3ebggle3cFJQgqVdTthGtnz8DIgLjBzCsuhWWfHmvM_mWwYb4JnhApMi6CFdWx_5IQkPixWKPu-wO1L_eZg",
            "pulse vector 2",
            "304502201a15bb287654ac29c766c09dc5ff09dc5cfa5e2ea2e183963458665e78f67fa10221009149310b570355f2de7324d807b5c7f54efa7d8961e7d1ea2df56bbc9556a401",
        ),
    ];

    fn b64(s: &str) -> Vec<u8> {
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(s)
            .unwrap()
    }

    fn secret(h: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        out.copy_from_slice(&hex::decode(h).unwrap());
        out
    }

    #[test]
    fn verifies_signatures_openssl_made_and_refuses_altered_ones() {
        for (d, spki, msg, sig) in OPENSSL_VECTORS {
            let key = PublicKey::parse(&b64(spki)).expect("the vector's key parses");
            let sig = hex::decode(sig).unwrap();
            assert!(verify(&key, msg.as_bytes(), &sig), "{msg}: valid");
            assert!(
                !verify(&key, b"another message", &sig),
                "{msg}: other message"
            );
            let mut bent = sig.clone();
            let last = bent.len() - 1;
            bent[last] ^= 1;
            assert!(!verify(&key, msg.as_bytes(), &bent), "{msg}: altered s");
            assert_eq!(key.to_spki(), b64(spki), "{msg}: round trip");
            let derived = public_key_for_tests(&secret(d)).unwrap();
            assert_eq!(derived.to_spki(), b64(spki), "{msg}: d*G is the key");
        }
        // A signature is bound to its key.
        let other = PublicKey::parse(&b64(OPENSSL_VECTORS[1].1)).unwrap();
        let sig = hex::decode(OPENSSL_VECTORS[0].3).unwrap();
        assert!(!verify(&other, OPENSSL_VECTORS[0].2.as_bytes(), &sig));
    }

    #[test]
    fn signs_what_it_verifies_and_refuses_malformed_input() {
        let d = secret(OPENSSL_VECTORS[2].0);
        let key = public_key_for_tests(&d).unwrap();
        for msg in [&b""[..], b"x", b"a longer message over the block size of sha-256, twice over: 0123456789abcdef0123456789abcdef"] {
            let sig = sign_for_tests(&d, msg).unwrap();
            assert!(verify(&key, msg, &sig));
        }
        let sig = sign_for_tests(&d, b"m").unwrap();
        assert!(!verify(&key, b"m", &sig[..sig.len() - 1]), "truncated");
        assert!(!verify(&key, b"m", &[]), "empty");
        assert!(
            !verify(
                &key,
                b"m",
                &[0x30, 0x06, 0x02, 0x01, 0x00, 0x02, 0x01, 0x01]
            ),
            "r = 0"
        );
        assert!(sign_for_tests(&[0; 32], b"m").is_none(), "zero key");
        // Off-curve and malformed keys are refused.
        let mut spki = key.to_spki();
        let last = spki.len() - 1;
        spki[last] ^= 1;
        assert!(PublicKey::parse(&spki).is_none(), "off the curve");
        assert!(PublicKey::parse(&spki[..40]).is_none(), "short");
        let raw = &key.to_spki()[SPKI_PREFIX.len()..];
        assert!(PublicKey::parse(raw).is_some(), "the bare point parses");
    }
}
