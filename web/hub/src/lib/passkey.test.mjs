import assert from "node:assert/strict";
import { register } from "node:module";
import { test } from "node:test";

// passkey.ts imports "./api" and "./passkey-state" without extensions, the
// way Next resolves them; the hub's test loader resolves those for node --test.
register("../components/project/tsx-test-loader.mjs", import.meta.url);

const { api, post, setWriteSigner } = await import("./api.ts");
const { installPasskeySigner, registerPasskey, withPasskey } = await import("./passkey.ts");
const { PASSKEY_HEADER, b64url, fromB64url, readPasskeyHeader, writeChallenge } = await import("./passkey-state.ts");

const enc = new TextEncoder();

// A fake authenticator with a real P-256 key: it signs the way a browser's
// passkey does (authenticatorData || SHA-256(clientDataJSON)).
async function authenticator() {
  const keys = await crypto.subtle.generateKey({ name: "ECDSA", namedCurve: "P-256" }, true, ["sign", "verify"]);
  const rawId = Uint8Array.from([7, 7, 7]).buffer;
  const asked = [];
  return {
    keys,
    asked,
    credentials: {
      async get({ publicKey }) {
        asked.push(publicKey);
        const clientDataJSON = enc.encode(
          JSON.stringify({ type: "webauthn.get", challenge: b64url(new Uint8Array(publicKey.challenge)), origin: "https://acme.hive.example" }),
        );
        const authenticatorData = new Uint8Array(37).fill(1);
        const hash = new Uint8Array(await crypto.subtle.digest("SHA-256", clientDataJSON));
        const signature = await crypto.subtle.sign({ name: "ECDSA", hash: "SHA-256" }, keys.privateKey, new Uint8Array([...authenticatorData, ...hash]));
        return { rawId, response: { authenticatorData: authenticatorData.buffer, clientDataJSON: clientDataJSON.buffer, signature } };
      },
      async create({ publicKey }) {
        asked.push(publicKey);
        const spki = await crypto.subtle.exportKey("spki", keys.publicKey);
        return { rawId, response: { getPublicKey: () => spki } };
      },
    },
  };
}

function withGlobals(vals, fn) {
  const saved = {};
  for (const k of Object.keys(vals)) {
    saved[k] = Object.getOwnPropertyDescriptor(globalThis, k);
    Object.defineProperty(globalThis, k, { value: vals[k], configurable: true, writable: true });
  }
  return fn().finally(() => {
    setWriteSigner(null);
    for (const k of Object.keys(vals)) {
      if (saved[k]) Object.defineProperty(globalThis, k, saved[k]);
      else delete globalThis[k];
    }
  });
}

test("through the front a write is signed over the brain's nonce and verifies; reads are not", async () => {
  const auth = await authenticator();
  const calls = [];
  let n = 0;
  const fetch = async (href, init = {}) => {
    calls.push({ href, init });
    if (href === "/api/v1/members/nonce") return Response.json({ nonce: "nonce-" + ++n, expires_at: "2026-11-10T12:05:00Z" });
    return Response.json({ ok: true });
  };
  await withGlobals({ location: { hostname: "acme.hive.example" }, navigator: { credentials: auth.credentials }, fetch }, async () => {
    assert.equal(installPasskeySigner(), true);
    await post("settings", { key: "capacity", value: "2" });
    await api("org");

    assert.deepEqual(calls.map((c) => c.href), ["/api/v1/members/nonce", "/api/v1/settings", "/api/v1/org"]);
    const write = calls[1].init;
    const header = readPasskeyHeader(new Headers(write.headers).get(PASSKEY_HEADER));
    assert.equal(new Headers(write.headers).get("X-Hive-Settings"), "yes", "the intent header is kept");
    assert.equal(header.nonce, "nonce-1");
    assert.equal(header.credential, "BwcH");

    // The brain's check: the challenge the passkey signed is this exact write.
    const cd = JSON.parse(new TextDecoder().decode(fromB64url(header.client_data_json)));
    const want = await writeChallenge({ nonce: "nonce-1", method: "POST", path: "/api/v1/settings", body: write.body });
    assert.equal(cd.challenge, b64url(want));
    const raw = await crypto.subtle.exportKey("raw", auth.keys.publicKey);
    const pub = await crypto.subtle.importKey("raw", raw, { name: "ECDSA", namedCurve: "P-256" }, false, ["verify"]);
    const signed = new Uint8Array([
      ...fromB64url(header.authenticator_data),
      ...new Uint8Array(await crypto.subtle.digest("SHA-256", fromB64url(header.client_data_json))),
    ]);
    assert.equal(await crypto.subtle.verify({ name: "ECDSA", hash: "SHA-256" }, pub, fromB64url(header.signature), signed), true);

    assert.equal(new Headers(calls[2].init.headers).get(PASSKEY_HEADER), null, "a read is not signed");
    assert.equal(auth.asked[0].rpId, "acme.hive.example");
  });
});

test("on this machine nothing is signed and no nonce is asked for", async () => {
  const calls = [];
  const fetch = async (href, init = {}) => {
    calls.push({ href, init });
    return Response.json({ ok: true });
  };
  await withGlobals({ location: { hostname: "127.0.0.1" }, fetch }, async () => {
    assert.equal(installPasskeySigner(), false);
    await post("tasks", { title: "x" });
    assert.deepEqual(calls.map((c) => c.href), ["/api/v1/tasks"]);
    assert.equal(new Headers(calls[0].init.headers).get(PASSKEY_HEADER), null);
    const init = { method: "POST", body: "{}" };
    assert.equal(await withPasskey("tasks", init), init, "loopback leaves the request as it was");
  });
});

test("a brain that gives no nonce stops the write with its reason", async () => {
  const auth = await authenticator();
  const fetch = async () => Response.json({ error: "forbidden", why: "not through the front" }, { status: 403 });
  await withGlobals({ location: { hostname: "acme.hive.example" }, navigator: { credentials: auth.credentials }, fetch }, async () => {
    await assert.rejects(withPasskey("tasks", { method: "POST", body: "{}" }), /HTTP 403\): not through the front/);
    assert.equal(auth.asked.length, 0, "the member is not asked to touch the passkey for nothing");
  });
});

test("registering hands back the line for hive member passkey add", async () => {
  const auth = await authenticator();
  await withGlobals({ location: { hostname: "acme.hive.example" }, navigator: { credentials: auth.credentials } }, async () => {
    const r = await registerPasskey("ana");
    const raw = new Uint8Array(await crypto.subtle.exportKey("raw", auth.keys.publicKey));
    assert.deepEqual(r, { member: "ana", credential: "BwcH", publicKey: b64url(raw) });
    assert.equal(auth.asked[0].rp.id, "acme.hive.example");
    assert.deepEqual(auth.asked[0].pubKeyCredParams, [{ type: "public-key", alg: -7 }]);
  });
});
