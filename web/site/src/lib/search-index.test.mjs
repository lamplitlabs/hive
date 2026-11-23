// The search index as the build makes it: which words and which code of a
// section are worth a reader's typing, and what is left out to keep the file
// small.
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
      // No "local", "server" (the heading), "models" (the title), "you", "can", "the" (said everywhere), "it", "no", "2".
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

test("a word that stands in most sections is left out: it would find everything and cost bytes", () => {
  const sections = Array.from({ length: 100 }, (_, i) => section(`s${i}`, `Part ${i}`, `the runner starts agents ${i % 50 === 0 ? "ollama" : ""} word${i}`));
  const [entry] = buildSearchIndex([page("/docs/x/", "X", sections)]);
  const all = entry.h.map(([, , terms = ""]) => terms);
  assert.ok(all.every((t) => !/\b(runner|starts|agents)\b/.test(t)), "said in every section");
  assert.equal(all.filter((t) => /\bollama\b/.test(t)).length, 2, "said in two");
  assert.match(all[7], /^word7$/);
});

test("a very long section is cut: at most 160 words and 24 pieces of code", () => {
  const text = Array.from({ length: 400 }, (_, i) => `term${i}`).join(" ");
  const code = Array.from({ length: 60 }, (_, i) => `setting.name_${i}`);
  const [entry] = buildSearchIndex([page("/docs/x/", "X", [section("long", "Long", text, [...code, ...code])])]);
  const [, , terms, named] = entry.h[0];
  assert.equal(terms.split(" ").length, 160);
  assert.equal(named.split("\n").length, 24);
  assert.equal(new Set(named.split("\n")).size, 24);
});
