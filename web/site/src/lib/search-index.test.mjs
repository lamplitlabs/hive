// The search index as the build makes it: every word of a section that a
// reader may type and all the code it names, the words it says most first.
// Only the words of grammar are left out.
import test from "node:test";
import assert from "node:assert/strict";

const { buildSearchIndex, isCodeTerm } = await import("./search-index.ts");

test("code worth finding: a command, a flag, a setting, a file of Hive's, an environment variable", () => {
  for (const yes of ["hive pair", "hive evolve status", "hive-hand join", "hive schedule add --every", "--dry-run", "evolve.loop", "harness.max_steps", "worker_model", "minutes_per_job", "HIVE_HOME", "~/.hive/hive.sqlite", ".hive/automations/", ".hive/hive.yaml"]) {
    assert.ok(isCodeTerm(yes), yes);
  }
  for (const no of ["1", "ask", "main", "npm test", "my-app", "<slug>", "hive pair ADMIN@HOST --name NAME", "hive", "the runner is off", "TASK-001", "a".repeat(60)]) {
    assert.ok(!isCodeTerm(no), no);
  }
});

const page = (path, title, sections, extra = {}) => ({ path, title, group: "Guides", sections, ...extra });
const section = (id, heading, text, code = []) => ({ id, heading, text, code });

test("a section keeps the words that say something its heading and its page's title do not", () => {
  const [entry] = buildSearchIndex([
    page("/docs/how-to/models/", "Choose models and set budgets", [
      section("local", "Use a local server", "You can use a local server such as Ollama. It needs no key, and the models stay on this machine. See 2 of 3.", ["worker_model", "ollama", "hive settings set"]),
      section("empty", "Read more", ""),
    ]),
  ]);
  assert.deepEqual(entry, {
    u: "/docs/how-to/models/",
    t: "Choose models and set budgets",
    g: "Guides",
    h: [
      // No "use", "local", "server" (the heading), "models" (the title), "you", "can", "the", "it", "no" (words of grammar), "2" (one letter).
      ["local", "Use a local server", "ollama needs key stay machine see", "worker_model\nhive settings set"],
      ["empty", "Read more"],
    ],
  });
});

test("a page's keywords and its opening are kept beside its sections", () => {
  const [entry] = buildSearchIndex([
    page("/docs/how-to/operations/", "Keep Hive running", [], { keywords: "uninstall upgrade", lead: { text: "Hive survives a reboot when it runs as services.", code: ["hive service install", "x"] } }),
  ]);
  assert.equal(entry.k, "uninstall upgrade");
  assert.equal(entry.x, "survives reboot runs services");
  assert.equal(entry.c, "hive service install");
  assert.deepEqual(entry.h, []);
});

test("a word said in every section is kept in each: the search must never say the docs lack a word they hold", () => {
  const sections = Array.from({ length: 100 }, (_, i) => section(`s${i}`, `Part ${i}`, `the runner starts agents ${i % 50 === 0 ? "ollama" : ""} word${i}`));
  const [entry] = buildSearchIndex([page("/docs/x/", "X", sections)]);
  const all = entry.h.map(([, , terms = ""]) => terms.split(" "));
  assert.ok(all.every((t) => t.includes("runner") && t.includes("starts") && t.includes("agents")), "said in every section, kept in every section");
  assert.equal(all.filter((t) => t.includes("ollama")).length, 2, "said in two");
  assert.ok(all.every((t) => !t.includes("the")), "a word of grammar is in none");
  assert.deepEqual(all[7], ["runner", "starts", "agents", "word7"]);
});

test("a very long section keeps every word and all the code it names, each once", () => {
  const text = Array.from({ length: 400 }, (_, i) => `term${i}`).join(" ");
  const code = Array.from({ length: 60 }, (_, i) => `setting.name_${i}`);
  const [entry] = buildSearchIndex([page("/docs/x/", "X", [section("long", "Long", `${text} ${text}`, [...code, ...code])])]);
  const [, , terms, named] = entry.h[0];
  assert.equal(terms.split(" ").length, 400);
  assert.deepEqual(named.split("\n"), code);
});

test("the words a section says most come first, and among equals the one it says first", () => {
  // As they are first said: paste, secret, check, key, checked, fails.
  const text = "Paste a secret, then check the key. The key is checked once: a key that fails is a secret no more.";
  const [entry] = buildSearchIndex([page("/docs/x/", "X", [section("s", "S", text)])]);
  assert.equal(entry.h[0][2], "key secret paste check checked fails");
});

test("a short word and a number are words a reader types; a single letter is not", () => {
  const [entry] = buildSearchIndex([page("/docs/x/", "X", [section("s", "S", "Run gh auth login, open port 8765 on v2, then answer y or n.")])]);
  assert.equal(entry.h[0][2], "run gh auth login open port 8765 v2 answer");
});

test("a record of a decision is marked, so that the search lists it after the pages that say what holds", () => {
  const [adr, guide] = buildSearchIndex([page("/docs/decisions/adr-1/", "ADR-1", [], { record: true }), page("/docs/how-to/x/", "X", [], { record: false })]);
  assert.equal(adr.r, 1);
  assert.equal("r" in guide, false);
});
