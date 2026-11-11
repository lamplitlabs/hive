import assert from "node:assert/strict";
import { test } from "node:test";
import {
  NONCE_HEADER,
  NONCE_PATH,
  SIGNATURE_HEADER,
  b64url,
  creationOptions,
  fromB64url,
  isLoopbackHost,
  isWriteMethod,
  memberNameProblem,
  membersOf,
  needsSignature,
  nonceOf,
  parseSignedIn,
  memberAddCommand,
  memberInput,
  p256Spki,
  passkeyErrorWords,
  passkeyHeaders,
  passkeySupport,
  rawP256FromSpki,
  readPasskeyHeaders,
  requestOptions,
  signInWords,
  signedBytes,
  signingRequired,
  standingOf,
  writeChallenge,
} from "./passkey-state.ts";

const text = (b) => new TextDecoder().decode(b);

test("only writes from a page served off this machine are signed", () => {
  for (const h of ["localhost", "127.0.0.1", "127.8.9.10", "[::1]", "::1", "0.0.0.0", "hub.localhost"])
    assert.equal(isLoopbackHost(h), true, h);
  for (const h of ["acme.hive.example", "10.0.0.5", "127.example.com", "localhost.evil.com"])
    assert.equal(isLoopbackHost(h), false, h);
  assert.equal(signingRequired(""), false);
  assert.equal(signingRequired(undefined), false);
  assert.equal(signingRequired("acme.hive.example"), true);
  for (const m of ["POST", "put", "PATCH", "DELETE"]) assert.equal(isWriteMethod(m), true, m);
  for (const m of ["GET", "HEAD", "OPTIONS", undefined]) assert.equal(isWriteMethod(m), false, String(m));
  assert.equal(needsSignature("acme.hive.example", "POST", "settings"), true);
  assert.equal(needsSignature("acme.hive.example", "GET", "settings"), false);
  assert.equal(needsSignature("localhost", "POST", "settings"), false);
  // The nonce route is how a signature starts; it cannot need one.
  assert.equal(needsSignature("acme.hive.example", "POST", NONCE_PATH), false);
  assert.equal(needsSignature("acme.hive.example", "POST", "/api/v1/nonce"), false);
  assert.equal(needsSignature("acme.hive.example", "POST", "/api/v1/members"), true);
  assert.equal(NONCE_HEADER, "X-Hive-Nonce");
});

test("the signed bytes bind nonce, method, path and the exact body", async () => {
  const w = { nonce: "n-1", method: "post", path: "/api/v1/settings", body: '{"key":"capacity","value":"2"}' };
  assert.equal(text(signedBytes(w)), 'hive-write-v1\nn-1\nPOST /api/v1/settings\n{"key":"capacity","value":"2"}');
  assert.equal(text(signedBytes({ ...w, body: undefined })), "hive-write-v1\nn-1\nPOST /api/v1/settings\n");
  // Binary bodies are signed as they are.
  const bin = signedBytes({ ...w, body: Uint8Array.from([0, 255, 10]) });
  assert.deepEqual([...bin.slice(-3)], [0, 255, 10]);

  const c = await writeChallenge(w);
  assert.equal(c.length, 32);
  assert.deepEqual(c, await writeChallenge({ ...w }));
  // Any change to what is signed changes the challenge.
  for (const other of [
    { ...w, nonce: "n-2" },
    { ...w, method: "DELETE" },
    { ...w, path: "/api/v1/decide" },
    { ...w, path: "/api/v1/settings?x=1" },
    { ...w, body: '{"key":"capacity","value":"3"}' },
  ])
    assert.notDeepEqual(await writeChallenge(other), c, JSON.stringify(other));
});

test("base64url round-trips without padding or +/", () => {
  const bytes = Uint8Array.from([251, 255, 191, 0, 1]);
  const s = b64url(bytes);
  assert.equal(s, "-_-_AAE");
  assert.deepEqual(fromB64url(s), bytes);
  assert.deepEqual(fromB64url(""), new Uint8Array(0));
  assert.throws(() => fromB64url("not base64!"), /not base64url/);
});

test("the headers carry every part the brain checks and read back", () => {
  const a = {
    credentialId: Uint8Array.from([1, 2, 3]),
    authenticatorData: Uint8Array.from([4, 5]),
    clientDataJSON: new TextEncoder().encode('{"type":"webauthn.get"}'),
    signature: Uint8Array.from([0x30, 0x44]),
  };
  const h = passkeyHeaders("n-1", a);
  assert.deepEqual(h, {
    "X-Hive-Nonce": "n-1",
    "X-Hive-Passkey-Credential": "AQID",
    "X-Hive-Passkey-Authenticator": "BAU",
    "X-Hive-Passkey-Client-Data": b64url(a.clientDataJSON),
    "X-Hive-Passkey-Signature": "MEQ",
  });
  for (const v of Object.values(h)) assert.match(v, /^[A-Za-z0-9_-]+$/, "each value is one base64url token");
  assert.deepEqual(readPasskeyHeaders((n) => h[n]), {
    nonce: "n-1",
    credential: "AQID",
    authenticatorData: a.authenticatorData,
    clientDataJSON: a.clientDataJSON,
    signature: a.signature,
  });
  assert.equal(readPasskeyHeaders((n) => (n === SIGNATURE_HEADER ? "!" : h[n])), null);
});

test("a signature over the challenge verifies with the member's public key, as the brain checks it", async () => {
  // The brain's check (WebAuthn level 2, 7.2): the signature is over
  // authenticatorData || SHA-256(clientDataJSON), and clientDataJSON's
  // challenge is the base64url of our write challenge.
  const keys = await crypto.subtle.generateKey({ name: "ECDSA", namedCurve: "P-256" }, true, ["sign", "verify"]);
  const w = { nonce: "n-9", method: "POST", path: "/api/v1/tasks", body: '{"title":"x"}' };
  const challenge = await writeChallenge(w);
  const clientDataJSON = new TextEncoder().encode(
    JSON.stringify({ type: "webauthn.get", challenge: b64url(challenge), origin: "https://acme.hive.example" }),
  );
  const authenticatorData = new Uint8Array(37).fill(7);
  const cdHash = new Uint8Array(await crypto.subtle.digest("SHA-256", clientDataJSON));
  const signed = new Uint8Array([...authenticatorData, ...cdHash]);
  const signature = new Uint8Array(await crypto.subtle.sign({ name: "ECDSA", hash: "SHA-256" }, keys.privateKey, signed));
  const headers = passkeyHeaders(w.nonce, { credentialId: Uint8Array.from([9]), authenticatorData, clientDataJSON, signature });
  const header = readPasskeyHeaders((n) => headers[n]);

  // What the brain does with the header:
  const cd = JSON.parse(text(header.clientDataJSON));
  assert.equal(cd.challenge, b64url(await writeChallenge({ ...w, nonce: header.nonce })));
  const raw = rawP256FromSpki(new Uint8Array(await crypto.subtle.exportKey("spki", keys.publicKey)));
  const pub = await crypto.subtle.importKey("raw", raw, { name: "ECDSA", namedCurve: "P-256" }, false, ["verify"]);
  const again = new Uint8Array([
    ...header.authenticatorData,
    ...new Uint8Array(await crypto.subtle.digest("SHA-256", header.clientDataJSON)),
  ]);
  assert.equal(await crypto.subtle.verify({ name: "ECDSA", hash: "SHA-256" }, pub, header.signature, again), true);
  // A changed body no longer matches the challenge the passkey signed.
  assert.notEqual(cd.challenge, b64url(await writeChallenge({ ...w, body: '{"title":"y"}' })));
});

test("the registered key is the P-256 SPKI; other keys are refused in words", async () => {
  const keys = await crypto.subtle.generateKey({ name: "ECDSA", namedCurve: "P-256" }, true, ["sign", "verify"]);
  const spki = new Uint8Array(await crypto.subtle.exportKey("spki", keys.publicKey));
  const raw = new Uint8Array(await crypto.subtle.exportKey("raw", keys.publicKey));
  assert.deepEqual(rawP256FromSpki(spki), raw);
  assert.equal(raw.length, 65);
  assert.equal(p256Spki(spki), b64url(spki));
  assert.equal(fromB64url(p256Spki(spki)).length, 91);

  const p384 = await crypto.subtle.generateKey({ name: "ECDSA", namedCurve: "P-384" }, true, ["sign", "verify"]);
  assert.throws(
    () => rawP256FromSpki(new Uint8Array(new ArrayBuffer(0))),
    /ES256 passkeys only/,
  );
  await assert.rejects(
    (async () => rawP256FromSpki(new Uint8Array(await crypto.subtle.exportKey("spki", p384.publicKey))))(),
    /ES256 passkeys only/,
  );
});

test("the register page's command is one line the owner can paste", () => {
  const r = { member: "ana@acme.dev", credential: "AQID-_", publicKey: "MFkw" };
  assert.deepEqual(memberInput(r), { credential_id: "AQID-_", member: "ana@acme.dev", name: "ana@acme.dev", role: "member", public_key: "MFkw" });
  assert.equal(memberInput(r, "owner").role, "owner");
  assert.equal(
    memberAddCommand(r),
    "curl -sS -X POST http://127.0.0.1:8765/api/v1/members -H 'Content-Type: application/json' -H 'X-Hive-Members: yes' -d " +
      `'{"credential_id":"AQID-_","member":"ana@acme.dev","name":"ana@acme.dev","role":"member","public_key":"MFkw"}'`,
  );
  assert.match(memberAddCommand(r, "http://127.0.0.1:9000/"), /^curl -sS -X POST http:\/\/127\.0\.0\.1:9000\/api\/v1\/members /);
  assert.match(memberAddCommand({ ...r, member: "it's" }), /"member":"it'\\''s"/);
  assert.equal(memberNameProblem(""), "Enter your member name, the one the owner knows you by.");
  assert.match(memberNameProblem("ana smith"), /no spaces/);
  assert.match(memberNameProblem("a".repeat(65)), /64/);
  assert.equal(memberNameProblem(" ana@acme.dev "), "");
});

test("WebAuthn options ask for ES256, user verification and the signed-in passkey", () => {
  const challenge = new Uint8Array(32).fill(1);
  const get = requestOptions(challenge, "acme.hive.example", ["AQID"]);
  assert.equal(get.rpId, "acme.hive.example");
  assert.equal(get.userVerification, "required");
  assert.deepEqual([...get.allowCredentials[0].id], [1, 2, 3]);
  assert.deepEqual(requestOptions(challenge, "x").allowCredentials, []);

  const create = creationOptions({ member: "ana", rpId: "acme.hive.example", challenge, exclude: ["AQID"] });
  assert.deepEqual(create.pubKeyCredParams, [{ type: "public-key", alg: -7 }]);
  assert.equal(create.rp.id, "acme.hive.example");
  assert.equal(create.rp.name, "Hive");
  assert.equal(create.user.name, "ana");
  assert.equal(create.attestation, "none");
  assert.equal(create.excludeCredentials.length, 1);
});

test("members and their passkeys: who may sign", () => {
  // GET /api/v1/members lists one row per passkey (hive_store::Member).
  const members = membersOf({
    members: [
      { credential_id: "K1", member: "user_owner", name: "owner", role: "owner", status: "active", sign_count: 4 },
      { credential_id: "K2", member: "user_ana", name: "ana", role: "member", status: "revoked" },
      { credential_id: "K3", member: "user_ana", name: "", role: "", status: "active" },
      { credential_id: "", member: "user_x", name: "x" },
      null,
    ],
  });
  assert.deepEqual(members.map((m) => m.credential), ["K1", "K2", "K3"]);
  assert.equal(members[2].name, "user_ana", "the member id stands in for an empty name");
  assert.equal(members[2].role, "member");
  assert.deepEqual(standingOf(members, "K1"), { state: "active", member: "owner", role: "owner" });
  assert.deepEqual(standingOf(members, "K2"), { state: "revoked", member: "ana" });
  assert.deepEqual(standingOf(members, "K3"), { state: "active", member: "user_ana", role: "member" });
  assert.deepEqual(standingOf(members, "nope"), { state: "unknown" });
  assert.deepEqual(membersOf(null), []);
  assert.deepEqual(membersOf({ members: "x" }), []);

  assert.equal(signInWords(standingOf(members, "K1")).tone, "ok");
  assert.match(signInWords(standingOf(members, "K1")).text, /Signed in as owner/);
  assert.equal(signInWords({ state: "revoked", member: "ana" }).tone, "bad");
  assert.match(signInWords({ state: "unknown" }).text, /Register it/);
});

test("the nonce reply, the stored sign-in and the browser's refusals", () => {
  assert.equal(nonceOf({ nonce: "abc", expires_at: "2026-11-10T12:05:00Z" }), "abc");
  assert.throws(() => nonceOf({}), /no nonce/);
  assert.throws(() => nonceOf(null), /no nonce/);

  assert.deepEqual(parseSignedIn('{"credential":"K1","member":"owner","at":"t"}'), { credential: "K1", member: "owner", at: "t" });
  assert.equal(parseSignedIn("{"), null);
  assert.equal(parseSignedIn('{"member":"x"}'), null);
  assert.equal(parseSignedIn(null), null);

  assert.match(passkeyErrorWords({ name: "NotAllowedError" }), /cancelled or timed out/);
  assert.match(passkeyErrorWords({ name: "SecurityError" }), /https/);
  assert.match(passkeyErrorWords({ name: "InvalidStateError" }), /already holds/);
  assert.equal(passkeyErrorWords(new Error("boom")), "boom");
  assert.equal(passkeyErrorWords(undefined), "The passkey step failed.");

  assert.deepEqual(passkeySupport({ hasCredentials: true, hasPublicKeyCredential: true, secureContext: true }), { ok: true, why: "" });
  assert.match(passkeySupport({ hasCredentials: true, hasPublicKeyCredential: true, secureContext: false }).why, /secure page/);
  assert.match(passkeySupport({ hasCredentials: false, hasPublicKeyCredential: false, secureContext: true }).why, /cannot use passkeys/);
});

test("the passkey pages say when they cannot or need not sign", async () => {
  const { passkeyPageNote } = await import("./passkey-state.ts");
  const ok = { ok: true, why: "" };
  assert.equal(passkeyPageNote("acme.hive.example", ok), null);
  assert.equal(passkeyPageNote("localhost", ok).tone, "info");
  assert.match(passkeyPageNote("127.0.0.1", ok).text, /not signed/);
  assert.deepEqual(passkeyPageNote("acme.hive.example", { ok: false, why: "no passkeys" }), { tone: "warn", text: "no passkeys" });
});
