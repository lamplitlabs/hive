// The passkey wire contract between this console and the brain (TASK-110).
//
// passkey-wire.json is one passkey-signed write through the front, made with
// a fixed test key: the member registration, the nonce, the request, the
// bytes signed, the challenge and the five headers. This test checks that
// passkey-state.ts produces exactly those bytes and headers, and that the
// signature verifies; crates/hive-hub/src/remote.rs reads the same file and
// checks the brain accepts the write end to end. A change on either side
// that the other does not make fails one of the two.
//
// Regenerate (only when the wire format changes on purpose, and change the
// brain in the same commit):
//   PASSKEY_WIRE_WRITE=1 node --test src/lib/passkey-wire.test.mjs
import assert from "node:assert/strict";
import { createECDH, createPrivateKey, createPublicKey, sign, verify } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { test } from "node:test";
import {
  AUTHENTICATOR_HEADER,
  CLIENT_DATA_HEADER,
  CREDENTIAL_HEADER,
  MEMBERS_HEADER,
  NONCE_HEADER,
  NONCE_PATH,
  SIGNATURE_HEADER,
  b64url,
  fromB64url,
  memberInput,
  p256Spki,
  passkeyHeaders,
  readPasskeyHeaders,
  signedBytes,
  writeChallenge,
} from "./passkey-state.ts";

const FIXTURE = new URL("./passkey-wire.json", import.meta.url);

// The test-only private scalar: 32 bytes of 0x07, the same key
// hive-runner's webauthn tests and hive-hub's remote.rs tests call ALICE.
// It signs nothing outside tests.
const SECRET_HEX = "07".repeat(32);

const enc = new TextEncoder();
const sha256 = async (bytes) => new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));

function testKey() {
  const ecdh = createECDH("prime256v1");
  ecdh.setPrivateKey(Buffer.from(SECRET_HEX, "hex"));
  const point = ecdh.getPublicKey();
  const jwk = {
    kty: "EC",
    crv: "P-256",
    d: b64url(Buffer.from(SECRET_HEX, "hex")),
    x: b64url(point.subarray(1, 33)),
    y: b64url(point.subarray(33, 65)),
  };
  const privateKey = createPrivateKey({ key: jwk, format: "jwk" });
  const spki = new Uint8Array(createPublicKey(privateKey).export({ type: "spki", format: "der" }));
  return { privateKey, spki };
}

// The inputs: who signs, where, and which write.
const INPUT = {
  origin: "https://team.example.com",
  rp_id: "team.example.com",
  credential: b64url(enc.encode("alice-passkey")),
  member: "alice@team.example.com",
  sign_count: 1,
  nonce: b64url(Uint8Array.from({ length: 32 }, (_, i) => i)),
  method: "POST",
  path: "/api/v1/settings?via=front",
  request_headers: { "Content-Type": "application/json", "X-Hive-Settings": "yes" },
  body: '{"name":"daemon.pass_seconds","value":"90"}',
};

// What an authenticator returns for this challenge: the client data as a
// browser writes it, and authenticator data with the RP ID hash, the
// user-present and user-verified flags and the counter.
async function ceremony(challenge) {
  const clientDataJSON = enc.encode(
    JSON.stringify({ type: "webauthn.get", challenge: b64url(challenge), origin: INPUT.origin, crossOrigin: false }),
  );
  const count = new Uint8Array(4);
  new DataView(count.buffer).setUint32(0, INPUT.sign_count);
  const authenticatorData = new Uint8Array([...(await sha256(enc.encode(INPUT.rp_id))), 0x05, ...count]);
  return { clientDataJSON, authenticatorData };
}

async function generate() {
  const { privateKey, spki } = testKey();
  const write = { nonce: INPUT.nonce, method: INPUT.method, path: INPUT.path, body: INPUT.body };
  const challenge = await writeChallenge(write);
  const { clientDataJSON, authenticatorData } = await ceremony(challenge);
  const signed = new Uint8Array([...authenticatorData, ...(await sha256(clientDataJSON))]);
  const signature = new Uint8Array(sign("sha256", signed, { key: privateKey, dsaEncoding: "der" }));
  return {
    note: "One passkey-signed write through the front, made by web/hub/src/lib/passkey-wire.test.mjs with the test-only key 0x07 x 32. Checked by that test (passkey-state.ts) and by crates/hive-hub/src/remote.rs (the brain). Regenerate only with a wire change on both sides.",
    origin: INPUT.origin,
    rp_id: INPUT.rp_id,
    nonce_route: "GET /api/v1/" + NONCE_PATH,
    member: { headers: { [MEMBERS_HEADER]: "yes" }, body: memberInput({ member: INPUT.member, credential: INPUT.credential, publicKey: p256Spki(spki) }, "owner") },
    write: { ...write, headers: INPUT.request_headers },
    sign_count: INPUT.sign_count,
    signed_bytes: b64url(signedBytes(write)),
    challenge: b64url(challenge),
    headers: passkeyHeaders(INPUT.nonce, {
      credentialId: fromB64url(INPUT.credential),
      authenticatorData,
      clientDataJSON,
      signature,
    }),
  };
}

if (process.env.PASSKEY_WIRE_WRITE === "1") writeFileSync(FIXTURE, JSON.stringify(await generate(), null, 2) + "\n");

const fixture = JSON.parse(readFileSync(FIXTURE, "utf8"));

test("the signed bytes and the challenge are the ones in the shared fixture", async () => {
  const w = fixture.write;
  const bytes = signedBytes(w);
  assert.equal(b64url(bytes), fixture.signed_bytes);
  assert.equal(
    new TextDecoder().decode(bytes),
    "hive-write-v1\n" + w.nonce + "\nPOST /api/v1/settings?via=front\n" + w.body,
  );
  assert.equal(b64url(await writeChallenge(w)), fixture.challenge);
  assert.equal(b64url(await sha256(bytes)), fixture.challenge);
  assert.equal(fixture.nonce_route, "GET /api/v1/nonce");
});

test("the registration is the fixture key's SPKI under the fixture member", async () => {
  const { spki } = testKey();
  // An owner, because the write is to settings (owner-only through the front).
  assert.deepEqual(fixture.member.body, memberInput({ member: INPUT.member, credential: INPUT.credential, publicKey: p256Spki(spki) }, "owner"));
  assert.deepEqual(fixture.member.headers, { "X-Hive-Members": "yes" });
  assert.deepEqual(Object.keys(fixture.member.body).sort(), ["credential_id", "member", "name", "public_key", "role"]);
});

test("the headers are what passkeyHeaders makes of the assertion, and its signature verifies", async () => {
  const sent = readPasskeyHeaders((name) => fixture.headers[name]);
  assert.ok(sent, "every header is there");
  assert.deepEqual(Object.keys(fixture.headers).sort(), [AUTHENTICATOR_HEADER, CLIENT_DATA_HEADER, CREDENTIAL_HEADER, NONCE_HEADER, SIGNATURE_HEADER].sort());
  assert.equal(sent.nonce, fixture.write.nonce);
  assert.equal(sent.credential, fixture.member.body.credential_id);

  // The authenticator's part is deterministic but for the signature.
  const { clientDataJSON, authenticatorData } = await ceremony(fromB64url(fixture.challenge));
  assert.deepEqual(sent.clientDataJSON, clientDataJSON);
  assert.deepEqual(sent.authenticatorData, authenticatorData);
  assert.deepEqual(
    passkeyHeaders(fixture.write.nonce, {
      credentialId: fromB64url(sent.credential),
      authenticatorData,
      clientDataJSON,
      signature: sent.signature,
    }),
    fixture.headers,
  );
  const client = JSON.parse(new TextDecoder().decode(clientDataJSON));
  assert.equal(client.challenge, fixture.challenge);
  assert.equal(client.origin, fixture.origin);

  // ES256 over authenticatorData || SHA-256(clientDataJSON), DER, under the
  // registered key - what webauthn::verify_assertion checks.
  const key = createPublicKey({ key: Buffer.from(fromB64url(fixture.member.body.public_key)), format: "der", type: "spki" });
  const signed = new Uint8Array([...authenticatorData, ...(await sha256(clientDataJSON))]);
  assert.equal(verify("sha256", signed, { key, dsaEncoding: "der" }, sent.signature), true);
  const other = new Uint8Array(signed);
  other[other.length - 1] ^= 1;
  assert.equal(verify("sha256", other, { key, dsaEncoding: "der" }, sent.signature), false);
});

test("a header the brain would refuse reads as no signature", () => {
  assert.equal(readPasskeyHeaders((name) => (name === NONCE_HEADER ? "" : fixture.headers[name])), null);
  assert.equal(readPasskeyHeaders((name) => (name === CREDENTIAL_HEADER ? undefined : fixture.headers[name])), null);
  assert.equal(readPasskeyHeaders((name) => (name === SIGNATURE_HEADER ? "not base64url!" : fixture.headers[name])), null);
});
