import { test } from "node:test";
import assert from "node:assert/strict";
import { referenceOptions } from "./reference-state.ts";

test("reference options list tasks and sessions newest first and filter", () => {
  const data = {
    conversations: [{ id: "chat-1", title: "Fix lander", created_at: "2026-11-01", last_at: "2026-11-02" }],
    tasks: [{ id: "TASK-7", title: "Speed verify", created_at: "2026-11-03" }],
  };
  assert.deepEqual(referenceOptions(data, "").map((o) => o.id), ["TASK-7", "chat-1"]);
  assert.deepEqual(referenceOptions(data, "lander").map((o) => o.id), ["chat-1"]);
  assert.deepEqual(referenceOptions(data, "task-7").map((o) => o.id), ["TASK-7"]);
  assert.deepEqual(referenceOptions(undefined, ""), []);
});
