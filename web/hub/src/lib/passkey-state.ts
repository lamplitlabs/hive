// Passkey-signed writes through the front (ADR-070 step 5, TASK-109).
//
// When the console is reached through the hosted front (any origin that is
// not this machine's loopback), the brain refuses a write - POST, PUT, PATCH,
// DELETE - unless it carries a member's passkey signature in the
// X-Hive-Passkey header (crates/hive-hub/src/front.rs). The signature covers
// a one-use nonce the brain issued (POST /api/v1/members/nonce), the method,
// the path and the exact body, so the front that relays the request cannot
// forge or alter a write: it never holds the member's private key.
//
// Everything about the wire format lives in this module, so a change on the
// brain's side costs one edit here:
//   - the bytes signed:  "hive-write\n" nonce "\n" METHOD "\n" path "\n" body
//   - the challenge:     SHA-256 of those bytes, handed to
//                        navigator.credentials.get() as the WebAuthn challenge
//   - the header:        base64url of the JSON {v, nonce, credential,
//                        authenticator_data, client_data_json, signature}
//   - the registration:  `hive member passkey add MEMBER --credential ID
//                        --public-key KEY`, KEY being the base64url of the
//                        65-byte uncompressed P-256 point (ES256 only)
//
// Pure: no DOM, no fetch, no storage. lib/passkey.ts owns navigator and
// localStorage; the pages under app/passkey/ own the screens.

export const PASSKEY_HEADER = "X-Hive-Passkey";
// apiHref-relative routes on the brain.
export const NONCE_PATH = "members/nonce";
export const MEMBERS_PATH = "members";
// The prefix that keeps a write signature from ever doubling as another
// protocol's challenge.
export const SIGNED_PREFIX = "hive-write";
export const HEADER_VERSION = 1;
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
  const head = [SIGNED_PREFIX, w.nonce, w.method.toUpperCase(), w.path, ""].join("\n");
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

export interface PasskeyHeaderBody {
  v: number;
  nonce: string;
  credential: string;
  authenticator_data: string;
  client_data_json: string;
  signature: string;
}

// The X-Hive-Passkey value for one signed write.
export function passkeyHeader(nonce: string, a: Assertion): string {
  const body: PasskeyHeaderBody = {
    v: HEADER_VERSION,
    nonce,
    credential: b64url(a.credentialId),
    authenticator_data: b64url(a.authenticatorData),
    client_data_json: b64url(a.clientDataJSON),
    signature: b64url(a.signature),
  };
  return b64url(utf8.encode(JSON.stringify(body)));
}

// The inverse, for tests and for the mock brain.
export function readPasskeyHeader(value: string): PasskeyHeaderBody {
  const body = JSON.parse(new TextDecoder().decode(fromB64url(value))) as PasskeyHeaderBody;
  if (body.v !== HEADER_VERSION) throw new Error("unknown passkey header version " + body.v);
  return body;
}

// The nonce reply. Tolerant of a brain that answers { nonce } alone.
export interface NonceReply {
  nonce: string;
  expires_at?: string;
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
// ceremony; what it trusts is the public key the owner adds with
// `hive member passkey add`.
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

// A member name the brain will take: what `hive member add` was given.
// Letters, digits, dot, dash, underscore and @, 1-64 characters.
export function memberNameProblem(name: string): string {
  const n = name.trim();
  if (!n) return "Enter the member name the owner added with hive member add.";
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

// AuthenticatorAttestationResponse.getPublicKey() hands back SPKI; the brain
// stores the 65-byte point. Anything but an uncompressed P-256 key is
// refused here, with words, instead of by the brain later.
export function rawP256FromSpki(spki: Uint8Array): Uint8Array {
  if (spki.length !== P256_SPKI_PREFIX.length + 65 || !P256_SPKI_PREFIX.every((b, i) => spki[i] === b))
    throw new Error("this passkey's key is not a P-256 (ES256) key; Hive takes ES256 passkeys only");
  const point = spki.slice(P256_SPKI_PREFIX.length);
  if (point[0] !== 0x04) throw new Error("this passkey's key is not an uncompressed P-256 point");
  return point;
}

// What the register page shows: the line the owner runs on the brain.
export interface Registration {
  member: string;
  credential: string;
  publicKey: string;
}

// Single-quoted for any POSIX shell; member names are already restricted,
// the quoting only guards the copy-paste.
function shq(s: string): string {
  return /^[A-Za-z0-9._@=/+-]+$/.test(s) ? s : "'" + s.replace(/'/g, "'\\''") + "'";
}

export function passkeyAddCommand(r: Registration): string {
  return [
    "hive member passkey add",
    shq(r.member),
    "--credential",
    shq(r.credential),
    "--public-key",
    shq(r.publicKey),
  ].join(" ");
}

// GET members, as far as the front needs it: who holds which passkey. The
// brain lists passkeys without their keys; a member who was removed stays in
// the list for the audit trail but cannot sign.
export interface MemberPasskey {
  credential: string;
  label: string;
  revoked: boolean;
}

export interface FrontMember {
  name: string;
  role: string;
  removed: boolean;
  passkeys: MemberPasskey[];
}

const str = (v: unknown): string => (typeof v === "string" ? v : "");

export function membersOf(payload: unknown): FrontMember[] {
  const raw = (payload as { members?: unknown } | null)?.members;
  if (!Array.isArray(raw)) return [];
  return raw
    .filter((m): m is Record<string, unknown> => !!m && typeof m === "object")
    .map((m) => ({
      name: str(m.name),
      role: str(m.role) || "member",
      removed: !!m.removed || !!str(m.removed_at),
      passkeys: (Array.isArray(m.passkeys) ? m.passkeys : [])
        .filter((p): p is Record<string, unknown> => !!p && typeof p === "object")
        .map((p) => ({
          credential: str(p.credential) || str(p.credential_id) || str(p.id),
          label: str(p.label),
          revoked: !!p.revoked || !!str(p.revoked_at),
        }))
        .filter((p) => p.credential),
    }))
    .filter((m) => m.name);
}

// Who a passkey belongs to on this brain, and whether it may sign.
export type PasskeyStanding =
  | { state: "active"; member: string; role: string }
  | { state: "revoked"; member: string }
  | { state: "member-removed"; member: string }
  | { state: "unknown" };

export function standingOf(members: FrontMember[], credential: string): PasskeyStanding {
  for (const m of members) {
    const p = m.passkeys.find((k) => k.credential === credential);
    if (!p) continue;
    if (m.removed) return { state: "member-removed", member: m.name };
    if (p.revoked) return { state: "revoked", member: m.name };
    return { state: "active", member: m.name, role: m.role };
  }
  return { state: "unknown" };
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
    case "member-removed":
      return { tone: "bad", text: s.member + " is no longer a member of this team, so this passkey cannot sign changes." };
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
