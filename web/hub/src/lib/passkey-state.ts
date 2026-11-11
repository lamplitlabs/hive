// Passkey-signed writes through the front (ADR-070 step 5, TASK-109/110).
//
// When the console is reached through the hosted front (any origin that is
// not this machine's loopback), the brain refuses a write - POST, PUT, PATCH,
// DELETE - unless it carries a member's passkey assertion
// (crates/hive-hub/src/remote.rs, checked by crates/hive-runner/src/webauthn.rs).
// The signature covers a one-use nonce the brain issued (GET /api/v1/nonce),
// the method, the path and the exact body, so the front that relays the
// request cannot forge or alter a write: it never holds the member's private
// key.
//
// Everything about the wire format lives in this module, so a change on the
// brain's side costs one edit here:
//   - the nonce:         GET /api/v1/nonce -> {nonce, expires_at, origin, rp_id}
//   - the bytes signed:  "hive-write-v1\n" nonce "\n" METHOD " " path?query "\n" body
//                        (webauthn::write_challenge)
//   - the challenge:     SHA-256 of those bytes, handed to
//                        navigator.credentials.get() as the WebAuthn challenge
//   - the headers:       X-Hive-Nonce (the nonce as issued) and, base64url,
//                        X-Hive-Passkey-Credential, -Authenticator,
//                        -Client-Data and -Signature
//   - the registration:  POST /api/v1/members (X-Hive-Members: yes) with
//                        {credential_id, member, name, role, public_key},
//                        public_key the base64url SubjectPublicKeyInfo of an
//                        ES256 (P-256) key
// src/lib/passkey-wire.json is one signed write in this format; the npm test
// checks this module produces it and hive-hub's remote.rs test checks the
// brain accepts it.
//
// Pure: no DOM, no fetch, no storage. lib/passkey.ts owns navigator and
// localStorage; the pages under app/passkey/ own the screens.

// The headers of a signed write (crates/hive-runner/src/webauthn.rs).
export const NONCE_HEADER = "X-Hive-Nonce";
export const CREDENTIAL_HEADER = "X-Hive-Passkey-Credential";
export const AUTHENTICATOR_HEADER = "X-Hive-Passkey-Authenticator";
export const CLIENT_DATA_HEADER = "X-Hive-Passkey-Client-Data";
export const SIGNATURE_HEADER = "X-Hive-Passkey-Signature";
// The confirmation header a members write carries.
export const MEMBERS_HEADER = "X-Hive-Members";
// apiHref-relative routes on the brain.
export const NONCE_PATH = "nonce";
export const MEMBERS_PATH = "members";
// The version word the challenge starts with (webauthn::CHALLENGE_DOMAIN):
// a later shape never verifies under this one, and a write signature never
// doubles as another protocol's challenge.
export const SIGNED_PREFIX = "hive-write-v1";
// COSE algorithm -7: ECDSA over P-256 with SHA-256, the one the brain checks.
export const ES256 = -7;
// How long the browser waits for the member to touch the authenticator.
export const PASSKEY_TIMEOUT_MS = 60_000;

const WRITE_METHODS = new Set(["POST", "PUT", "PATCH", "DELETE"]);

// Writes are what the brain gates; reads need only the front's session.
export function isWriteMethod(method: string | undefined): boolean {
  return WRITE_METHODS.has((method || "GET").toUpperCase());
}

// This machine's own addresses: a console opened here (the hub's own port,
// `next dev`) keeps the same-origin guard and never signs.
export function isLoopbackHost(hostname: string): boolean {
  const h = hostname.toLowerCase().replace(/^\[|\]$/g, "");
  return (
    h === "localhost" ||
    h.endsWith(".localhost") ||
    h === "::1" ||
    h === "0.0.0.0" ||
    /^127(\.\d{1,3}){3}$/.test(h)
  );
}

// Whether writes from this page must be passkey-signed: only when it is
// served from somewhere other than this machine, which for a hub with
// hub.allow_remote set means through the front. No hostname (node, a test)
// means no signing.
export function signingRequired(hostname: string | undefined | null): boolean {
  if (!hostname) return false;
  return !isLoopbackHost(hostname);
}

// The nonce route itself cannot be signed - it is how a signature starts.
export function needsSignature(
  hostname: string | undefined | null,
  method: string | undefined,
  apiPath: string,
): boolean {
  if (!signingRequired(hostname) || !isWriteMethod(method)) return false;
  return apiPath.replace(/^\/+/, "").replace(/^api\/v1\//, "").split("?")[0] !== NONCE_PATH;
}

const utf8 = new TextEncoder();

function concat(parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}

// A request body as the bytes the brain will read: strings are UTF-8 (JSON
// writes), binary bodies (uploads, audio) are taken as they are.
export function bodyBytes(body: string | Uint8Array | ArrayBuffer | null | undefined): Uint8Array {
  if (body == null) return new Uint8Array(0);
  if (typeof body === "string") return utf8.encode(body);
  if (body instanceof Uint8Array) return body;
  return new Uint8Array(body);
}

export interface WriteToSign {
  nonce: string;
  method: string;
  // The request's path with its query, as the brain sees it:
  // "/api/v1/settings", "/api/v1/voice/model?id=base".
  path: string;
  body?: string | Uint8Array | ArrayBuffer | null;
}

// The exact bytes the member's passkey signs over (through the challenge).
export function signedBytes(w: WriteToSign): Uint8Array {
  const head = SIGNED_PREFIX + "\n" + w.nonce + "\n" + w.method.toUpperCase() + " " + w.path + "\n";
  return concat([utf8.encode(head), bodyBytes(w.body)]);
}

// The WebAuthn challenge: SHA-256 of the signed bytes. crypto.subtle is in
// every browser on a secure origin and in node 20+.
export async function writeChallenge(w: WriteToSign): Promise<Uint8Array> {
  const digest = await crypto.subtle.digest("SHA-256", signedBytes(w) as BufferSource);
  return new Uint8Array(digest);
}

export function b64url(bytes: ArrayBuffer | Uint8Array): string {
  const b = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  let s = "";
  for (const x of b) s += String.fromCharCode(x);
  return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

export function fromB64url(text: string): Uint8Array {
  const t = text.trim().replace(/-/g, "+").replace(/_/g, "/");
  if (!/^[A-Za-z0-9+/]*$/.test(t)) throw new Error("not base64url: " + text.slice(0, 16));
  const s = atob(t + "=".repeat((4 - (t.length % 4)) % 4));
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
  return out;
}

// What navigator.credentials.get() returned, reduced to the parts the brain
// checks (all raw bytes).
export interface Assertion {
  credentialId: Uint8Array;
  authenticatorData: Uint8Array;
  clientDataJSON: Uint8Array;
  signature: Uint8Array;
}

// The headers of one signed write: the nonce as the brain issued it and the
// assertion's parts, base64url (webauthn::NONCE_HEADER and the four after it).
export function passkeyHeaders(nonce: string, a: Assertion): Record<string, string> {
  return {
    [NONCE_HEADER]: nonce,
    [CREDENTIAL_HEADER]: b64url(a.credentialId),
    [AUTHENTICATOR_HEADER]: b64url(a.authenticatorData),
    [CLIENT_DATA_HEADER]: b64url(a.clientDataJSON),
    [SIGNATURE_HEADER]: b64url(a.signature),
  };
}

// What the brain reads back from those headers (remote.rs assertion_of), for
// tests and the mock brain: null when any is missing, as the brain refuses it.
export interface SignedWriteHeaders {
  nonce: string;
  credential: string;
  authenticatorData: Uint8Array;
  clientDataJSON: Uint8Array;
  signature: Uint8Array;
}

export function readPasskeyHeaders(get: (name: string) => string | null | undefined): SignedWriteHeaders | null {
  const nonce = get(NONCE_HEADER) || "";
  const credential = get(CREDENTIAL_HEADER) || "";
  if (!nonce || !credential) return null;
  try {
    return {
      nonce,
      credential,
      authenticatorData: fromB64url(get(AUTHENTICATOR_HEADER) || ""),
      clientDataJSON: fromB64url(get(CLIENT_DATA_HEADER) || ""),
      signature: fromB64url(get(SIGNATURE_HEADER) || ""),
    };
  } catch {
    return null;
  }
}

// The nonce reply (GET /api/v1/nonce). Tolerant of a brain that answers
// { nonce } alone.
export interface NonceReply {
  nonce: string;
  expires_at?: string;
  origin?: string;
  rp_id?: string;
}

export function nonceOf(reply: unknown): string {
  const n = (reply as { nonce?: unknown } | null)?.nonce;
  if (typeof n !== "string" || !n) throw new Error("the brain's nonce reply has no nonce");
  return n;
}

// navigator.credentials.get() options for one write. allow names the
// passkey this browser signed in with, so the member is not asked to pick.
export function requestOptions(
  challenge: Uint8Array,
  rpId: string,
  allow: string[] = [],
): PublicKeyCredentialRequestOptions {
  return {
    challenge: challenge as BufferSource,
    rpId,
    timeout: PASSKEY_TIMEOUT_MS,
    userVerification: "required",
    allowCredentials: allow.map((id) => ({ type: "public-key", id: fromB64url(id) as BufferSource })),
  };
}

// navigator.credentials.create() options for registering a member's passkey
// on this domain. The challenge is random: the brain never sees this
// ceremony; what it trusts is the public key the owner registers with
// POST /api/v1/members (memberAddCommand).
export function creationOptions(input: {
  member: string;
  rpId: string;
  rpName?: string;
  challenge: Uint8Array;
  exclude?: string[];
}): PublicKeyCredentialCreationOptions {
  return {
    challenge: input.challenge as BufferSource,
    rp: { id: input.rpId, name: input.rpName || "Hive" },
    user: {
      id: utf8.encode(input.member) as BufferSource,
      name: input.member,
      displayName: input.member,
    },
    pubKeyCredParams: [{ type: "public-key", alg: ES256 }],
    timeout: PASSKEY_TIMEOUT_MS,
    attestation: "none",
    authenticatorSelection: { residentKey: "preferred", userVerification: "required" },
    excludeCredentials: (input.exclude ?? []).map((id) => ({ type: "public-key", id: fromB64url(id) as BufferSource })),
  };
}

// A member name: what the owner knows this member by.
// Letters, digits, dot, dash, underscore and @, 1-64 characters.
export function memberNameProblem(name: string): string {
  const n = name.trim();
  if (!n) return "Enter your member name, the one the owner knows you by.";
  if (n.length > 64) return "A member name is at most 64 characters.";
  if (!/^[A-Za-z0-9._@-]+$/.test(n)) return "Use letters, digits, dots, dashes, underscores or @ only - no spaces.";
  return "";
}

// The DER SubjectPublicKeyInfo prefix of every uncompressed P-256 key:
// SEQUENCE { SEQUENCE { id-ecPublicKey, prime256v1 }, BIT STRING 0x00 0x04 ... }.
const P256_SPKI_PREFIX = Uint8Array.from([
  0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03,
  0x01, 0x07, 0x03, 0x42, 0x00,
]);

// AuthenticatorAttestationResponse.getPublicKey() hands back SPKI, which is
// what the brain registers (it also takes the bare 65-byte point). Anything
// but an uncompressed P-256 key is refused here, with words, instead of by
// the brain later.
export function rawP256FromSpki(spki: Uint8Array): Uint8Array {
  if (spki.length !== P256_SPKI_PREFIX.length + 65 || !P256_SPKI_PREFIX.every((b, i) => spki[i] === b))
    throw new Error("this passkey's key is not a P-256 (ES256) key; Hive takes ES256 passkeys only");
  const point = spki.slice(P256_SPKI_PREFIX.length);
  if (point[0] !== 0x04) throw new Error("this passkey's key is not an uncompressed P-256 point");
  return point;
}

// The public_key the brain registers: the SPKI itself, base64url, once it
// is known to be a P-256 key.
export function p256Spki(spki: Uint8Array): string {
  rawP256FromSpki(spki);
  return b64url(spki);
}

// What the register page made: the member's name, the passkey's id and its
// public key (SubjectPublicKeyInfo DER), both base64url.
export interface Registration {
  member: string;
  credential: string;
  publicKey: string;
}

// The body of POST /api/v1/members (hive_store::MemberInput): the brain
// trusts only keys registered this way. The register page knows the member
// by the name they typed, so that is both the member id and the display
// name; the role is the owner's call and defaults to member.
export interface MemberInput {
  credential_id: string;
  member: string;
  name: string;
  role: string;
  public_key: string;
}

export function memberInput(r: Registration, role: "owner" | "member" = "member"): MemberInput {
  return { credential_id: r.credential, member: r.member, name: r.member, role, public_key: r.publicKey };
}

// Where the owner's own console listens by default (`hive hub`).
export const HUB_URL = "http://127.0.0.1:8765";

// Single-quoted for any POSIX shell; member names are already restricted,
// the quoting only guards the copy-paste.
function shq(s: string): string {
  return /^[A-Za-z0-9._@=/+:-]+$/.test(s) ? s : "'" + s.replace(/'/g, "'\\''") + "'";
}

// The line the owner runs on the brain's machine to register the passkey:
// the members route on the console's own port, with its confirmation header.
export function memberAddCommand(r: Registration, hub: string = HUB_URL): string {
  return [
    "curl -sS -X POST",
    shq(hub.replace(/\/+$/, "") + "/api/v1/" + MEMBERS_PATH),
    "-H 'Content-Type: application/json'",
    "-H " + shq(MEMBERS_HEADER + ": yes"),
    "-d",
    shq(JSON.stringify(memberInput(r))),
  ].join(" ");
}

// GET members as the brain lists them: one row per registered passkey
// (hive_store::Member), revoked ones kept with status "revoked".
export interface MemberPasskey {
  credential: string;
  member: string;
  name: string;
  role: string;
  revoked: boolean;
}

const str = (v: unknown): string => (typeof v === "string" ? v : "");

export function membersOf(payload: unknown): MemberPasskey[] {
  const raw = (payload as { members?: unknown } | null)?.members;
  if (!Array.isArray(raw)) return [];
  return raw
    .filter((m): m is Record<string, unknown> => !!m && typeof m === "object")
    .map((m) => ({
      credential: str(m.credential_id),
      member: str(m.member),
      name: str(m.name) || str(m.member),
      role: str(m.role) || "member",
      revoked: str(m.status) === "revoked",
    }))
    .filter((m) => m.credential && m.name);
}

// Who a passkey belongs to on this brain, and whether it may sign.
export type PasskeyStanding =
  | { state: "active"; member: string; role: string }
  | { state: "revoked"; member: string }
  | { state: "unknown" };

export function standingOf(members: MemberPasskey[], credential: string): PasskeyStanding {
  const p = members.find((m) => m.credential === credential);
  if (!p) return { state: "unknown" };
  if (p.revoked) return { state: "revoked", member: p.name };
  return { state: "active", member: p.name, role: p.role };
}

// The passkey this browser signed in with, kept in localStorage so every
// later write asks for that one passkey.
export interface SignedIn {
  credential: string;
  member: string;
  at: string;
}

export const SIGNED_IN_KEY = "hive.passkey";

export function parseSignedIn(raw: string | null | undefined): SignedIn | null {
  if (!raw) return null;
  try {
    const v = JSON.parse(raw) as Partial<SignedIn>;
    if (typeof v.credential !== "string" || !v.credential) return null;
    return { credential: v.credential, member: str(v.member), at: str(v.at) };
  } catch {
    return null;
  }
}

// What the sign-in page says after the ceremony.
export function signInWords(s: PasskeyStanding): { tone: "ok" | "warn" | "bad"; text: string } {
  switch (s.state) {
    case "active":
      return { tone: "ok", text: "Signed in as " + s.member + ". Your changes from this browser are signed with this passkey." };
    case "revoked":
      return { tone: "bad", text: "This passkey belonged to " + s.member + " but was revoked. Register a new one and ask the owner to add it." };
    default:
      return {
        tone: "warn",
        text: "This brain does not know this passkey yet. Register it, then ask the owner to run the command the register page shows.",
      };
  }
}

// A WebAuthn failure in words. The browser reports every refusal - a
// cancelled prompt, a timeout, no matching passkey - as NotAllowedError on
// purpose, so the words cover all of them.
export function passkeyErrorWords(err: unknown): string {
  const name = (err as { name?: unknown } | null)?.name;
  const msg = err instanceof Error ? err.message : String(err ?? "");
  switch (name) {
    case "NotAllowedError":
      return "The passkey prompt was cancelled or timed out, or this device has no passkey for this site. Try again, or register a passkey here first.";
    case "InvalidStateError":
      return "This device already holds a passkey for this member on this site. Use it to sign in instead.";
    case "SecurityError":
      return "The browser refused passkeys on this address. Passkeys need https and the front's own domain (not an IP address).";
    case "NotSupportedError":
      return "This device has no authenticator that can make an ES256 passkey.";
    case "AbortError":
      return "The passkey prompt was closed before it finished.";
    default:
      return msg || "The passkey step failed.";
  }
}

// Whether this browser can do any of this at all.
export function passkeySupport(env: {
  hasCredentials: boolean;
  hasPublicKeyCredential: boolean;
  secureContext: boolean;
}): { ok: boolean; why: string } {
  if (!env.secureContext)
    return { ok: false, why: "Passkeys need a secure page: open the console through the front's https address." };
  if (!env.hasPublicKeyCredential || !env.hasCredentials)
    return { ok: false, why: "This browser cannot use passkeys. Use a current Safari, Chrome, Edge or Firefox." };
  return { ok: true, why: "" };
}

// The note at the top of the passkey pages: why the page cannot work here,
// or that on this machine nothing is signed (the loopback console keeps the
// same-origin guard), or nothing at all through the front.
export function passkeyPageNote(
  hostname: string,
  support: { ok: boolean; why: string },
): { tone: "info" | "warn"; text: string } | null {
  if (!signingRequired(hostname))
    return {
      tone: "info",
      text: "You are on the brain's own machine, so your changes here are not signed. These pages are for the console reached through the front, at your team's address.",
    };
  if (!support.ok) return { tone: "warn", text: support.why };
  return null;
}
