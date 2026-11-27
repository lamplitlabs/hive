import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

// The mock's question rows carry what the hub's do (crates/hive-hub
// board.rs: decider, hard_stop, options, recommended_why, needs_options), so
// a mock run shows the answers the console really offers and not the old
// stop pair (TASK-149 item 16). A row without these would hide a fault in
// the strip the way the live hub would not.
const here = dirname(fileURLToPath(import.meta.url));
const read = async (p) => JSON.parse(await readFile(join(here, "api", p), "utf8"));
const FIELDS = ["decider", "hard_stop", "options", "recommended_why", "needs_options"];

for (const [name, rows] of [
  ["questions.json", async () => read("questions.json")],
  ["org.json questions", async () => (await read("org.json")).questions],
  ["projects/demo-app/questions.json", async () => read("projects/demo-app/questions.json")],
]) {
  test(name + ": every question row carries the hub's answer fields", async () => {
    const qs = await rows();
    assert.ok(qs.length > 0);
    for (const q of qs) {
      for (const f of FIELDS) assert.ok(Object.hasOwn(q, f), q.id + " lacks " + f);
      assert.ok(Array.isArray(q.options), q.id + ": options is a list");
      const marked = q.options.filter((o) => o.recommended).length;
      const ready = q.options.length >= 2 && marked === 1;
      assert.equal(q.needs_options === "", ready, q.id + ": needs_options is empty exactly when the answers are ready");
      // The hub's question_decider: a hard stop is the owner's, unless it is
      // no stop and names no answers to pick (then its lead must name them).
      const want = q.hard_stop ? (!q.stop && !ready ? "lead" : "owner") : "lead";
      assert.equal(q.decider, want, q.id + ": decider follows the hub's rule");
    }
  });
}

test("the mock's org rows are the questions the questions route serves", async () => {
  assert.deepEqual((await read("org.json")).questions, await read("questions.json"));
});
