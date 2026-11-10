import assert from "node:assert/strict";
import { test } from "node:test";
import {
  NONCE_PATH,
  PASSKEY_HEADER,
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
  passkeyAddCommand,
  passkeyErrorWords,
  passkeyHeader,
  passkeySupport,
  rawP256FromSpki,
  readPasskeyHeader,
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
  for (const h of ["acme.pulse.example", "10.0.0.5", "127.example.com", "localhost.evil.com"])
    assert.equal(isLoopbackHost(h), false, h);
  assert.equal(signingRequired(""), false);
  assert.equal(signingRequired(undefined), false);
  assert.equal(signingRequired("acme.pulse.example"), true);
  for (const m of ["POST", "put", "PATCH", "DELETE"]) assert.equal(isWriteMethod(m), true, m);
  for (const m of ["GET", "HEAD", "OPTIONS", undefined]) assert.equal(isWriteMethod(m), false, String(m));
  assert.equal(needsSignature("acme.pulse.example", "POST", "settings"), true);
  assert.equal(needsSignature("acme.pulse.example", "GET", "settings"), false);
  assert.equal(needsSignature("localhost", "POST", "settings"), false);
  // The nonce route is how a signature starts; it cannot need one.
  assert.equal(needsSignature("acme.pulse.example", "POST", NONCE_PATH), false);
  assert.equal(needsSignature("acme.pulse.example", "POST", "/api/v1/members/nonce"), false);
  assert.equal(PASSKEY_HEADER, "X-Pulse-Passkey");
});

test("the signed bytes bind nonce, method, path and the exact body", async () => {
  const w = { nonce: "n-1", method: "post", path: "/api/v1/settings", body: '{"key":"capacity","value":"2"}' };
  assert.equal(text(signedBytes(w)), 'pulse-write\nn-1\nPOST\n/api/v1/settings\n{"key":"capacity","value":"2"}');
  assert.equal(text(signedBytes({ ...w, body: undefined })), "pulse-write\nn-1\nPOST\n/api/v1/settings\n");
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

test("the header carries every part the brain checks and reads back", () => {
  const a = {
    credentialId: Uint8Array.from([1, 2, 3]),
    authenticatorData: Uint8Array.from([4, 5]),
    clientDataJSON: new TextEncoder().encode('{"type":"webauthn.get"}'),
    signature: Uint8Array.from([0x30, 0x44]),
  };
  const h = passkeyHeader("n-1", a);
  assert.match(h, /^[A-Za-z0-9_-]+$/, "a header value is one base64url token");
  assert.deepEqual(readPasskeyHeader(h), {
    v: 1,
    nonce: "n-1",
    credential: "AQID",
    authenticator_data: "BAU",
    client_data_json: b64url(a.clientDataJSON),
    signature: "MEQ",
  });
  assert.throws(() => readPasskeyHeader(b64url(new TextEncoder().encode('{"v":2}'))), /version 2/);
});

test("a signature over the challenge verifies with the member's public key, as the brain checks it", async () => {
  // The brain's check (WebAuthn level 2, 7.2): the signature is over
  // authenticatorData || SHA-256(clientDataJSON), and clientDataJSON's
  // challenge is the base64url of our write challenge.
  const keys = await crypto.subtle.generateKey({ name: "ECDSA", namedCurve: "P-256" }, true, ["sign", "verify"]);
  const w = { nonce: "n-9", method: "POST", path: "/api/v1/tasks", body: '{"title":"x"}' };
  const challenge = await writeChallenge(w);
  const clientDataJSON = new TextEncoder().encode(
    JSON.stringify({ type: "webauthn.get", challenge: b64url(challenge), origin: "https://acme.pulse.example" }),
  );
  const authenticatorData = new Uint8Array(37).fill(7);
  const cdHash = new Uint8Array(await crypto.subtle.digest("SHA-256", clientDataJSON));
  const signed = new Uint8Array([...authenticatorData, ...cdHash]);
  const signature = new Uint8Array(await crypto.subtle.sign({ name: "ECDSA", hash: "SHA-256" }, keys.privateKey, signed));
  const header = readPasskeyHeader(
    passkeyHeader(w.nonce, { credentialId: Uint8Array.from([9]), authenticatorData, clientDataJSON, signature }),
  );

  // What the brain does with the header:
  const cd = JSON.parse(text(fromB64url(header.client_data_json)));
  assert.equal(cd.challenge, b64url(await writeChallenge({ ...w, nonce: header.nonce })));
  const raw = rawP256FromSpki(new Uint8Array(await crypto.subtle.exportKey("spki", keys.publicKey)));
  const pub = await crypto.subtle.importKey("raw", raw, { name: "ECDSA", namedCurve: "P-256" }, false, ["verify"]);
  const again = new Uint8Array([
    ...fromB64url(header.authenticator_data),
    ...new Uint8Array(await crypto.subtle.digest("SHA-256", fromB64url(header.client_data_json))),
  ]);
  assert.equal(await crypto.subtle.verify({ name: "ECDSA", hash: "SHA-256" }, pub, fromB64url(header.signature), again), true);
  // A changed body no longer matches the challenge the passkey signed.
  assert.notEqual(cd.challenge, b64url(await writeChallenge({ ...w, body: '{"title":"y"}' })));
});

test("the registered key is the 65-byte P-256 point; other keys are refused in words", async () => {
  const keys = await crypto.subtle.generateKey({ name: "ECDSA", namedCurve: "P-256" }, true, ["sign", "verify"]);
  const spki = new Uint8Array(await crypto.subtle.exportKey("spki", keys.publicKey));
  const raw = new Uint8Array(await crypto.subtle.exportKey("raw", keys.publicKey));
  assert.deepEqual(rawP256FromSpki(spki), raw);
  assert.equal(raw.length, 65);

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
  assert.equal(
    passkeyAddCommand({ member: "ana@acme.dev", credential: "AQID-_", publicKey: "BAAA" }),
    "pulse member passkey add ana@acme.dev --credential AQID-_ --public-key BAAA",
  );
  assert.equal(
    passkeyAddCommand({ member: "it's", credential: "x", publicKey: "y" }),
    "pulse member passkey add 'it'\\''s' --credential x --public-key y",
  );
  assert.equal(memberNameProblem(""), "Enter the member name the owner added with pulse member add.");
  assert.match(memberNameProblem("ana smith"), /no spaces/);
  assert.match(memberNameProblem("a".repeat(65)), /64/);
  assert.equal(memberNameProblem(" ana@acme.dev "), "");
});

test("WebAuthn options ask for ES256, user verification and the signed-in passkey", () => {
  const challenge = new Uint8Array(32).fill(1);
  const get = requestOptions(challenge, "acme.pulse.example", ["AQID"]);
  assert.equal(get.rpId, "acme.pulse.example");
  assert.equal(get.userVerification, "required");
  assert.deepEqual([...get.allowCredentials[0].id], [1, 2, 3]);
  assert.deepEqual(requestOptions(challenge, "x").allowCredentials, []);

  const create = creationOptions({ member: "ana", rpId: "acme.pulse.example", challenge, exclude: ["AQID"] });
  assert.deepEqual(create.pubKeyCredParams, [{ type: "public-key", alg: -7 }]);
  assert.equal(create.rp.id, "acme.pulse.example");
  assert.equal(create.rp.name, "PULSE");
  assert.equal(create.user.name, "ana");
  assert.equal(create.attestation, "none");
  assert.equal(create.excludeCredentials.length, 1);
});

test("members and their passkeys: who may sign", () => {
  const members = membersOf({
    members: [
      { name: "owner", role: "owner", passkeys: [{ credential: "K1", label: "laptop" }] },
      { name: "ana", passkeys: [{ credential_id: "K2", revoked_at: "2026-11-10T00:00:00Z" }, { id: "K3" }] },
      { name: "bo", removed_at: "2026-11-09T00:00:00Z", passkeys: [{ credential: "K4" }] },
      { name: "", passkeys: [] },
      null,
    ],
  });
  assert.deepEqual(members.map((m) => m.name), ["owner", "ana", "bo"]);
  assert.equal(members[1].role, "member");
  assert.deepEqual(standingOf(members, "K1"), { state: "active", member: "owner", role: "owner" });
  assert.deepEqual(standingOf(members, "K2"), { state: "revoked", member: "ana" });
  assert.deepEqual(standingOf(members, "K3"), { state: "active", member: "ana", role: "member" });
  assert.deepEqual(standingOf(members, "K4"), { state: "member-removed", member: "bo" });
  assert.deepEqual(standingOf(members, "nope"), { state: "unknown" });
  assert.deepEqual(membersOf(null), []);
  assert.deepEqual(membersOf({ members: "x" }), []);

  assert.equal(signInWords(standingOf(members, "K1")).tone, "ok");
  assert.match(signInWords(standingOf(members, "K1")).text, /Signed in as owner/);
  assert.equal(signInWords({ state: "revoked", member: "ana" }).tone, "bad");
  assert.equal(signInWords({ state: "member-removed", member: "bo" }).tone, "bad");
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
