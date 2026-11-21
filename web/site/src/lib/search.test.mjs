// The search the browser runs over the docs' titles and headings.
import test from "node:test";
import assert from "node:assert/strict";

const { searchDocs } = await import("./search.ts");
const docs = await import("./docs.ts");

const index = [
  { u: "/docs/harness/", t: "The harness (how an agent runs)", g: "The agents", h: [["the-tools", "The tools"], ["hooks", "Hooks"], ["skills-as-tools", "Skills as tools"]] },
  { u: "/docs/board/", t: "The board (for the agents)", g: "The agents", h: [["events-hooks-and-channels", "Events, hooks and channels"], ["shared-skills-hive-skill", "Shared skills: hive skill"]] },
  { u: "/docs/machines/", t: "Machines and paired hands", g: "Machines and running", h: [["pairing-hands", "Pairing hands"], ["the-hand", "The hand"]] },
];

test("a page whose title holds every word comes first, then the headings that do", () => {
  assert.deepEqual(searchDocs(index, "harness"), [{ href: "/docs/harness/", title: "The harness (how an agent runs)", where: "The agents" }]);
  assert.deepEqual(
    searchDocs(index, "hooks").map((h) => h.href),
    ["/docs/harness/#hooks", "/docs/board/#events-hooks-and-channels"],
  );
  assert.deepEqual(searchDocs(index, "  HAND  ")[0], { href: "/docs/machines/", title: "Machines and paired hands", where: "Machines and running" });
});

test("a heading is found by its words and its page's together, and a page's name alone does not list all its headings", () => {
  assert.deepEqual(searchDocs(index, "harness hooks"), [{ href: "/docs/harness/#hooks", title: "Hooks", where: "The harness (how an agent runs)" }]);
  assert.deepEqual(
    searchDocs(index, "board skills").map((h) => h.href),
    ["/docs/board/#shared-skills-hive-skill"],
  );
  assert.equal(searchDocs(index, "harness").length, 1);
});

test("nothing typed finds nothing, and the list stops at its limit", () => {
  assert.deepEqual(searchDocs(index, "   "), []);
  assert.deepEqual(searchDocs(index, "zzz"), []);
  assert.equal(searchDocs(index, "the", 2).length, 2);
  assert.ok(searchDocs(docs.searchIndex(docs.loadDocs()), "the").length <= 10);
});

test("on the real index: the words a reader types reach the page that answers", () => {
  const real = docs.searchIndex(docs.loadDocs());
  const first = (q) => searchDocs(real, q)[0]?.href;
  assert.equal(first("hooks"), "/docs/harness/#hooks");
  assert.equal(first("pairing hands"), "/docs/machines/#pairing-hands");
  assert.equal(first("evolve settings"), "/docs/evolve/");
  assert.equal(first("complexity rule"), "/docs/review/#the-complexity-rule");
});
