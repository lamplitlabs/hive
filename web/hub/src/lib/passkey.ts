// The browser half of passkey-signed writes (ADR-070 step 5, TASK-109):
// asks the brain for a nonce, has the member's passkey sign the write
// through navigator.credentials, and turns the answer into the
// X-Hive-Passkey header. The format and every decision live in
// passkey-state.ts (pure, tested); this file only touches navigator,
// location, localStorage and fetch.
import { setWriteSigner } from "./api";
import {
  NONCE_PATH,
  signingRequired,
  PASSKEY_HEADER,
  SIGNED_IN_KEY,
  b64url,
  creationOptions,
  needsSignature,
  nonceOf,
  parseSignedIn,
  passkeyHeader,
  passkeySupport,
  rawP256FromSpki,
  requestOptions,
  writeChallenge,
  type Registration,
  type SignedIn,
} from "./passkey-state";

const API = "/api/v1/";

function hostname(): string {
  return typeof location === "undefined" ? "" : location.hostname;
}

export function browserPasskeySupport(): { ok: boolean; why: string } {
  if (typeof window === "undefined") return { ok: false, why: "" };
  return passkeySupport({
    hasCredentials: !!navigator.credentials,
    hasPublicKeyCredential: typeof window.PublicKeyCredential !== "undefined",
    secureContext: window.isSecureContext,
  });
}

export function signedIn(): SignedIn | null {
  try {
    return parseSignedIn(localStorage.getItem(SIGNED_IN_KEY));
  } catch {
    return null;
  }
}

export function rememberSignedIn(s: SignedIn | null) {
  try {
    if (s) localStorage.setItem(SIGNED_IN_KEY, JSON.stringify(s));
    else localStorage.removeItem(SIGNED_IN_KEY);
  } catch {
    // private mode: every write asks which passkey to use
  }
}

// One nonce from the brain; it is spent by the write it signs.
export async function fetchNonce(): Promise<string> {
  const r = await fetch(API + NONCE_PATH, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: "{}",
    cache: "no-store",
    credentials: "same-origin",
  });
  let data: unknown = null;
  try {
    data = await r.json();
  } catch {
    // fall through to the error below
  }
  if (!r.ok) {
    const e = (data ?? {}) as { error?: string; why?: string };
    throw new Error("The brain gave no nonce (HTTP " + r.status + "): " + (e.why || e.error || "no reason given"));
  }
  return nonceOf(data);
}

export interface PasskeyAnswer {
  nonce: string;
  credential: string;
  header: string;
}

// Signs one request: a fresh nonce, the member's passkey over the request's
// challenge, the header value. Throws the browser's own error (passkeyErrorWords
// turns it into words).
export async function signRequest(method: string, path: string, body?: string | Uint8Array | null): Promise<PasskeyAnswer> {
  const nonce = await fetchNonce();
  const challenge = await writeChallenge({ nonce, method, path, body });
  const me = signedIn();
  const cred = (await navigator.credentials.get({
    publicKey: requestOptions(challenge, hostname(), me ? [me.credential] : []),
  })) as PublicKeyCredential | null;
  if (!cred) throw new Error("The browser returned no passkey.");
  const res = cred.response as AuthenticatorAssertionResponse;
  const header = passkeyHeader(nonce, {
    credentialId: new Uint8Array(cred.rawId),
    authenticatorData: new Uint8Array(res.authenticatorData),
    clientDataJSON: new Uint8Array(res.clientDataJSON),
    signature: new Uint8Array(res.signature),
  });
  return { nonce, credential: b64url(cred.rawId), header };
}

// The request init a write needs on this page: unchanged on this machine
// (loopback keeps the same-origin guard), signed through the front. A File
// or Blob body is read once so the bytes sent are the bytes signed.
export async function withPasskey(path: string, init: RequestInit = {}): Promise<RequestInit> {
  const href = path.startsWith("/") ? path : API + path.replace(/^\/+/, "");
  if (!needsSignature(hostname(), init.method, href)) return init;
  let body: string | Uint8Array | null = null;
  const raw = init.body;
  if (typeof raw === "string") body = raw;
  else if (raw instanceof Uint8Array) body = raw;
  else if (raw instanceof ArrayBuffer) body = new Uint8Array(raw);
  else if (typeof Blob !== "undefined" && raw instanceof Blob) body = new Uint8Array(await raw.arrayBuffer());
  else if (raw != null) throw new Error("This kind of request body cannot be passkey-signed.");
  const signed = await signRequest(init.method || "POST", href, body);
  const headers = new Headers(init.headers);
  headers.set(PASSKEY_HEADER, signed.header);
  return { ...init, headers, body: body == null ? raw : (body as BodyInit) };
}

// Called once by the shell: through the front every write the console
// sends (api, post, patch, uploads) is signed first; on this machine nothing
// is installed and writes go out as they always did.
export function installPasskeySigner(): boolean {
  if (!signingRequired(hostname())) return false;
  setWriteSigner(withPasskey);
  return true;
}

// Proves this browser holds a passkey and remembers which one, so later
// writes ask for it by name. The brain checks nothing here - every write is
// checked on its own - so the challenge is the hash of a fresh nonce bound to
// this page, never a write's.
export async function passkeySignIn(): Promise<string> {
  const nonce = await fetchNonce();
  const challenge = await writeChallenge({ nonce, method: "GET", path: "/passkey/", body: "" });
  const cred = (await navigator.credentials.get({
    publicKey: requestOptions(challenge, hostname()),
  })) as PublicKeyCredential | null;
  if (!cred) throw new Error("The browser returned no passkey.");
  return b64url(cred.rawId);
}

// Makes a new ES256 passkey for MEMBER on this domain and returns what the
// owner adds on the brain. The private key never leaves the authenticator.
export async function registerPasskey(member: string, exclude: string[] = []): Promise<Registration> {
  const challenge = crypto.getRandomValues(new Uint8Array(32));
  const cred = (await navigator.credentials.create({
    publicKey: creationOptions({ member, rpId: hostname(), challenge, exclude }),
  })) as PublicKeyCredential | null;
  if (!cred) throw new Error("The browser made no passkey.");
  const res = cred.response as AuthenticatorAttestationResponse;
  const spki = res.getPublicKey?.();
  if (!spki) throw new Error("This browser does not hand out the passkey's public key; use a current Safari, Chrome, Edge or Firefox.");
  return {
    member,
    credential: b64url(cred.rawId),
    publicKey: b64url(rawP256FromSpki(new Uint8Array(spki))),
  };
}
