import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

// The mock starts a conversation the way the hub's POST /chat does, so a
// `npm run dev:mock` run can take a new owner from the new-session box to
// the open session: the answer carries the conversation, the list serves it
// first, and GET chat/<id> opens it with the first message as its transcript.
const here = dirname(fileURLToPath(import.meta.url));

async function withMock(fn, extra = {}) {
  const port = 20000 + Math.floor(Math.random() * 20000);
  const child = spawn(process.execPath, [join(here, "server.mjs")], {
    env: { ...process.env, HIVE_HUB_PORT: String(port), MOCK_DELAY_MS: "0", MOCK_JOIN: "off", ...extra },
    stdio: ["ignore", "pipe", "pipe"],
  });
  try {
    await new Promise((ok, no) => {
      const t = setTimeout(() => no(new Error("the mock did not start")), 10000);
      child.stdout.on("data", (b) => {
        if (String(b).includes("mock hub on")) {
          clearTimeout(t);
          ok();
        }
      });
      child.on("exit", (code) => no(new Error("the mock exited " + code)));
    });
    await fn("http://127.0.0.1:" + port + "/api/v1/");
  } finally {
    child.kill();
  }
}

test("POST chat starts a conversation the list and GET chat/<id> serve", async () => {
  await withMock(async (api) => {
    const empty = await fetch(api + "chat", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ text: "  " }) });
    assert.equal(empty.status, 400, "an empty message is refused");

    const r = await fetch(api + "chat", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ text: "Read the latest failing check and fix it.\nThen tell me.", project: "", auto_approve: true, lockdown: false }),
    });
    assert.equal(r.status, 201);
    const { conversation: c } = await r.json();
    assert.match(c.id, /^chat-\d{8}T\d{6}-[0-9a-f]{6}$/);
    assert.equal(c.title, "Read the latest failing check and fix it.");
    assert.equal(c.status, "idle");

    const list = await (await fetch(api + "chat")).json();
    assert.equal(list.conversations[0].id, c.id, "the new conversation leads the list");

    const d = await (await fetch(api + "chat/" + c.id + "?since=0")).json();
    assert.equal(d.conversation.id, c.id);
    assert.equal(d.messages.length, 1);
    assert.equal(d.messages[0].role, "user");
    assert.match(d.messages[0].content, /^Read the latest failing check/);
  });
});

// MOCK_FRESH=1 serves a brand-new machine: no projects, no sessions, nothing
// waiting, the provider signed out; what the run does shows up as usual.
test("MOCK_FRESH starts empty and keeps what the run adds", async () => {
  await withMock(
    async (api) => {
      const org = await (await fetch(api + "org")).json();
      assert.equal(org.projects.length, 0, "no projects");
      assert.equal(org.commons, null, "no shared board");
      const harness = await (await fetch(api + "harness")).json();
      assert.equal(harness.provider.login.logged_in, false, "provider signed out");
      const chat = await (await fetch(api + "chat")).json();
      assert.equal(chat.conversations.length + chat.tasks.length, 0, "no sessions");
      const tasks = await (await fetch(api + "tasks")).json();
      assert.equal(tasks.tasks.length, 0, "no tasks");
      const sug = await (await fetch(api + "tasks/suggested?status=owner")).json();
      assert.ok(Object.values(sug).every((v) => !Array.isArray(v) || v.length === 0), "no suggestions waiting");

      const r = await fetch(api + "chat", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ text: "Write a README" }) });
      const { conversation: c } = await r.json();
      const after = await (await fetch(api + "chat")).json();
      assert.deepEqual(after.conversations.map((x) => x.id), [c.id], "the session started in the run is listed");
    },
    { MOCK_FRESH: "1" },
  );
});
