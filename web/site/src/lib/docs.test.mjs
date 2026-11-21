// Which files of docs/ get a page and where the sidebar puts them. The
// promise: every file but the one left out on purpose has a page, a file
// that is not there yet is skipped, and a file nobody listed is never lost.
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

const docs = await import("./docs.ts");

const slugs = (plan) => plan.docs.map((d) => d.slug);
const shape = (plan) => plan.groups.map((g) => [g.title, g.docs.map((d) => d.slug)]);

test("the sidebar follows the listed order, group by group", () => {
  const plan = docs.planDocs([
    "windows-port.md",
    "settings.md",
    "security.md",
    "self-improvement.md",
    "runner.md",
    "review.md",
    "reports.md",
    "release.md",
    "recovery.md",
    "owner-reservation.md",
    "machines.md",
    "mac-app.md",
    "landing.md",
    "harness.md",
    "faq.md",
    "evolve.md",
    "development.md",
    "console.md",
    "concepts.md",
    "commands.md",
    "board.md",
    "design/computer-use.md",
    "decisions/ADR-070-web-first.md",
    "decisions/ADR-069-delivery.md",
    "decisions/ADR-071-open-tool.md",
    "context/environment.md",
  ]);
  assert.deepEqual(shape(plan), [
    ["Start", ["concepts", "faq", "console", "mac-app"]],
    ["The agents", ["board", "harness", "self-improvement", "evolve", "review", "landing"]],
    ["Machines and running", ["machines", "runner", "release", "recovery", "security"]],
    ["Reference", ["commands", "settings", "reports", "owner-reservation", "development", "windows-port"]],
    ["Decisions and design", ["decisions/adr-069-delivery", "decisions/adr-070-web-first", "decisions/adr-071-open-tool", "design/computer-use"]],
  ]);
  assert.deepEqual(plan.excluded, ["context/environment.md"]);
  assert.equal(plan.docs.find((d) => d.slug === "decisions/adr-069-delivery").path, "/docs/decisions/adr-069-delivery/");
  assert.equal(plan.docs.find((d) => d.slug === "decisions/adr-069-delivery").file, "decisions/ADR-069-delivery.md");
});

test("a listed file that does not exist yet is skipped, and an empty group is not shown", () => {
  // Today's tree: concepts, faq, commands and settings are still to come.
  const plan = docs.planDocs(["console.md", "mac-app.md", "harness.md", "landing.md", "reports.md"]);
  assert.deepEqual(shape(plan), [
    ["Start", ["console", "mac-app"]],
    ["The agents", ["harness", "landing"]],
    ["Reference", ["reports"]],
  ]);
  assert.deepEqual(shape(docs.planDocs([])), []);
});

test("a file nobody listed lands in More: nothing is dropped silently", () => {
  const files = ["harness.md", "brand-new.md", "notes/deep/idea.md", "context/other.md", "README.md", "context/environment.md"];
  const plan = docs.planDocs(files);
  assert.deepEqual(shape(plan), [
    ["The agents", ["harness"]],
    ["More", ["brand-new", "context/other", "notes/deep/idea", "readme"]],
  ]);
  // Every file but the excluded one has exactly one page.
  assert.deepEqual([...slugs(plan)].sort(), files.filter((f) => f !== "context/environment.md").map(docs.docSlug).sort());
  assert.equal(new Set(plan.docs.map((d) => d.path)).size, plan.docs.length);
});

test("files that are not Markdown get no page; two files that differ only by case are refused by name", () => {
  assert.deepEqual(slugs(docs.planDocs(["harness.md", "diagram.png", "notes.txt"])), ["harness"]);
  assert.throws(() => docs.planDocs(["Harness.md", "harness.md"]), /docs\/Harness\.md and docs\/harness\.md would both be \/docs\/harness\/: the addresses are lowercased/);
});

test("every file under the repository's docs/ has a page, except the environment recipe", () => {
  const root = docs.docsRoot();
  // Listed here on its own, not through the code under test.
  const onDisk = [];
  const walk = (dir, prefix) => {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
      const rel = prefix ? `${prefix}/${e.name}` : e.name;
      if (e.isDirectory()) walk(path.join(dir, e.name), rel);
      else if (e.name.endsWith(".md")) onDisk.push(rel);
    }
  };
  walk(root, "");
  assert.ok(onDisk.length >= 20, `docs/ holds ${onDisk.length} Markdown files`);
  assert.ok(onDisk.includes("context/environment.md"));

  const built = docs.loadDocs();
  assert.deepEqual(built.excluded, ["context/environment.md"]);
  assert.deepEqual(built.docs.map((d) => d.file).sort(), onDisk.filter((f) => f !== "context/environment.md").sort());
  for (const d of built.docs) {
    assert.equal(d.path, `/docs/${d.file.replace(/\.md$/, "").toLowerCase()}/`);
    assert.ok(d.title.length > 0, `${d.file} has a title`);
    assert.ok(d.html.length > 0, `${d.file} has a body`);
    assert.equal(d.editUrl, `https://github.com/lamplitlabs/hive/edit/main/docs/${d.file}`);
  }
  // Each page sits in exactly one group, and the groups together are the pages.
  assert.deepEqual(built.groups.flatMap((g) => g.docs.map((d) => d.file)), built.docs.map((d) => d.file));
  assert.equal(built.bySlug.get("landing").label, "Delivery");
});

test("previous and next walk the sidebar from the first page to the last", () => {
  const built = docs.loadDocs();
  assert.equal(built.docs[0].prev, undefined);
  assert.equal(built.docs.at(-1).next, undefined);
  for (let i = 1; i < built.docs.length; i += 1) {
    assert.equal(built.docs[i].prev.path, built.docs[i - 1].path);
    assert.equal(built.docs[i - 1].next.path, built.docs[i].path);
  }
});

test("no link in the docs is broken in the Markdown itself", () => {
  // A doc that points at a file or a heading that is not there is a defect of
  // the doc. The page still renders the link as written; this names it.
  const { problems } = docs.loadDocs();
  assert.deepEqual(problems, [], `broken in the source Markdown:\n${problems.join("\n")}`);
});

test("a broken anchor is reported with the file and the link that wrote it", () => {
  const built = docs.buildDocs([
    { file: "a.md", source: "# A\n\n## One\n\nSee [b](b.md#two), [gone](b.md#three), [here](#one) and [nowhere](#four).\n" },
    { file: "b.md", source: "# B\n\n## Two\n\nBack to [a](a.md#a).\n" },
  ]);
  assert.deepEqual(built.problems, [
    "docs/a.md: [b.md#three] points at #three, and docs/b.md has no such heading",
    "docs/a.md: [#four] points at #four, and this page has no such heading",
  ]);
  // Rendered as written all the same.
  assert.match(built.bySlug.get("a").html, /href="\/docs\/b\/#three"/);
});

test("titles: the heading and the site's name, as much of both as sixty characters hold", () => {
  assert.equal(docs.docMetaTitle("Development"), "Development · Hive docs");
  assert.equal(docs.docMetaTitle("Security notes: where your code and your data live"), "Security notes: where your code and your data live · Hive");
  assert.equal(docs.docMetaTitle("ADR-069: Delivery by pull request with an agent reviewer"), "ADR-069: Delivery by pull request with an agent reviewer");
  const long = docs.docMetaTitle(
    "ADR-070: Web-first Hive - a hosted brain per team, an open hand enrolled once per machine, one web login that every machine inherits",
  );
  assert.equal(long, "ADR-070: Web-first Hive - a hosted brain per team · Hive");
  assert.ok(long.length <= 60);
});

test("sidebar words: the heading up to its bracket or colon, an ADR whole", () => {
  assert.equal(docs.docLabel("harness", "The harness (how an agent runs)"), "The harness");
  assert.equal(docs.docLabel("review", "The reviewer: a job reads every pull request"), "The reviewer");
  assert.equal(docs.docLabel("landing", "Delivery: every branch becomes a pull request"), "Delivery");
  assert.equal(docs.docLabel("landing", "Something else entirely"), "Delivery");
  assert.equal(docs.docLabel("development", "Development"), "Development");
  assert.equal(docs.docLabel("decisions/adr-069-x", "ADR-069: Delivery by pull request with an agent reviewer"), "ADR-069: Delivery by pull request with an agent reviewer");
});

test("descriptions: the first paragraph, more of the page when it is too short, never past 155", () => {
  const first = "Every finished branch becomes a pull request on the project's upstream; this page is about who merges it.";
  assert.equal(docs.describeDoc("The reviewer", [first, "A second paragraph."]), first);
  const joined = docs.describeDoc("Windows port", ["Hive builds on macOS today.", "The code builds on Linux too, and this page lists every unix-only site."]);
  assert.equal(joined, "Hive builds on macOS today. The code builds on Linux too, and this page lists every unix-only site.");
  const bare = docs.describeDoc("Notes", []);
  assert.ok(bare.length >= 70 && bare.length <= 155, bare);
  assert.match(bare, /^Notes\./);
  const cut = docs.describeDoc("Long", ["word ".repeat(80)]);
  assert.ok(cut.length >= 70 && cut.length <= 155 && cut.endsWith("…"));
});

test("two docs with the same words still get different titles and descriptions", () => {
  const same = "# Notes\n\nThe same opening paragraph, long enough to stand as a description of the page it opens.\n";
  const built = docs.buildDocs([
    { file: "a/notes.md", source: same },
    { file: "b/notes.md", source: same },
  ]);
  const [a, b] = built.docs;
  assert.notEqual(a.metaTitle, b.metaTitle);
  assert.notEqual(a.description, b.description);
  assert.ok(b.metaTitle.length <= 60 && b.description.length <= 155);
});

test("the search index holds every page's title and its h2 and h3 headings, and stays small", () => {
  const built = docs.loadDocs();
  const index = docs.searchIndex(built);
  assert.equal(index.length, built.docs.length);
  const harness = index.find((e) => e.u === "/docs/harness/");
  assert.equal(harness.t, "The harness (how an agent runs)");
  assert.ok(harness.h.some(([id, text]) => id === "hooks" && text === "Hooks"));
  for (const e of index) {
    const doc = built.byPath.get(e.u);
    for (const [id] of e.h) assert.ok(doc.ids.includes(id), `${e.u}#${id} is a heading of the page`);
  }
  assert.ok(JSON.stringify(index).length < 60_000, "the index is a small file");
});
