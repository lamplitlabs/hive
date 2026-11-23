// Which files of docs/ get a page and where the sidebar puts them. The
// promise: every file but the one left out on purpose has a page, a file
// that is not there yet is skipped, and a file nobody listed is never lost.
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";

const docs = await import("./docs.ts");
const await_site = await import("./site.ts");

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
    "how-to/zebra.md",
    "how-to/tasks.md",
    "how-to/README.md",
    "how-to/troubleshooting.md",
    "how-to/first-project.md",
  ]);
  assert.deepEqual(shape(plan), [
    ["Start", ["concepts", "faq", "console", "mac-app"]],
    // The page that lists the guides, the guides in the order a newcomer meets them, then a guide nobody listed yet.
    ["Guides", ["how-to", "how-to/first-project", "how-to/tasks", "how-to/troubleshooting", "how-to/zebra"]],
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

test("a folder's README is the folder's own page", () => {
  assert.equal(docs.docSlug("how-to/README.md"), "how-to");
  assert.equal(docs.docPath("how-to/README.md"), "/docs/how-to/");
  assert.equal(docs.docSlug("how-to/readme.md"), "how-to");
  // A README at the top of docs/ has no folder to stand for.
  assert.equal(docs.docSlug("README.md"), "readme");
  assert.throws(() => docs.planDocs(["how-to.md", "how-to/README.md"]), /would both be \/docs\/how-to\//);
  const hub = docs.loadDocs().bySlug.get(docs.GUIDES_DIR);
  assert.equal(hub.path, "/docs/how-to/");
  assert.equal(hub.label, "All guides");
  assert.equal(hub.group, docs.GUIDES_GROUP);
});

test("every guide is listed on the page of guides, in the order the sidebar walks them", () => {
  const built = docs.loadDocs();
  const hub = built.bySlug.get(docs.GUIDES_DIR);
  const guides = built.docs.filter((d) => d.group === docs.GUIDES_GROUP && d !== hub).map((d) => d.file);
  assert.ok(guides.length >= 10, `docs/how-to/ holds ${guides.length} guides`);
  const listed = hub.links.filter((l) => l.kind === "doc" && l.doc.startsWith(`${docs.GUIDES_DIR}/`)).map((l) => l.doc);
  // The same guides, and in the same order: Previous and Next under a guide follow the page that lists them.
  assert.deepEqual([...new Set(listed)], guides);
  // A guide is a page of its own group and nowhere else, so the sidebar never lists one under More.
  assert.equal(built.groups.find((g) => g.title === docs.MORE_GROUP), undefined);
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
    assert.equal(d.path, `/docs/${d.file.replace(/\.md$/, "").toLowerCase().replace(/\/readme$/, "")}/`);
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

test("a page a later decision overtook says so in its description (ADR-070)", () => {
  const adr = docs.loadDocs().bySlug.get("decisions/adr-070-web-first-hive-hosted-brain-open-hand");
  assert.ok(adr, "the description in docs.ts names a file that exists");
  assert.match(adr.description, /^Superseded in part by ADR-071: the hosted product is withdrawn\./);
  assert.ok(adr.description.length >= 70 && adr.description.length <= 155, adr.description);
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

test("the search index holds the site's pages and every docs page with its sections, and stays a file a phone can fetch", () => {
  const built = docs.loadDocs();
  const index = docs.searchIndex(built);
  const site = await_site.STATIC_PAGES.filter((p) => p.path !== "/docs/");
  assert.equal(index.length, built.docs.length + site.length);
  assert.equal(new Set(index.map((e) => e.u)).size, index.length);
  // The guides stand first, so of two finds that are equally good the guide is shown first.
  assert.equal(index[0].g, docs.GUIDES_GROUP);

  const harness = index.find((e) => e.u === "/docs/harness/");
  assert.equal(harness.t, "The harness (how an agent runs)");
  const hooks = harness.h.find(([id]) => id === "hooks");
  assert.equal(hooks[1], "Hooks");
  assert.ok(hooks[2].split(" ").length > 5, "a section carries the words of its text");
  for (const e of index) {
    const doc = built.byPath.get(e.u);
    if (!doc) {
      assert.deepEqual(e.h, [], `${e.u} is a page of the site and has no sections in the index`);
      assert.ok(e.k.length > 20);
      continue;
    }
    assert.deepEqual(
      e.h.map(([id, text]) => [id, text]),
      doc.toc.map((t) => [t.id, t.text]),
      `${e.u} lists its h2 and h3 headings in order`,
    );
    for (const [, heading, terms = "", code = ""] of e.h) {
      assert.doesNotMatch(terms, /[^a-z0-9 ]/, `${e.u} "${heading}": words only`);
      assert.ok(terms.split(" ").length <= 160);
      assert.ok(code.split("\n").length <= 24);
    }
  }
  // A setting and a command are in the index as written.
  const names = (needle) => index.some((e) => e.h.some(([, , , code = ""]) => code.split("\n").includes(needle)));
  assert.ok(names("evolve.loop"));
  assert.ok(names("review.merge"));

  const size = JSON.stringify(index).length;
  assert.ok(size < 420_000, `the index is ${size} bytes`);
  assert.ok(zlib.gzipSync(JSON.stringify(index)).length < 140_000, "and compresses well");
});

test("the pictures of the docs are listed for the export, and each page that shows one keeps its place", () => {
  const pictures = docs.listDocPictures(docs.docsRoot());
  assert.ok(pictures.includes("how-to/images/console-home.webp"));
  assert.ok(pictures.every((p) => /\.(png|webp|jpe?g|gif|avif|svg)$/i.test(p)));
  const built = docs.loadDocs();
  const shown = new Set(built.docs.flatMap((d) => d.links.filter((l) => l.kind === "repo" && l.repoPath?.startsWith("docs/") && /\.(png|webp|jpe?g|gif|avif|svg)$/i.test(l.repoPath)).map((l) => l.repoPath.slice("docs/".length))));
  // No picture is kept that no page shows.
  assert.deepEqual([...shown].sort(), [...pictures].sort());
  const tour = built.bySlug.get("how-to/console-tour");
  const imgs = [...tour.html.matchAll(/<img\b[^>]*>/g)].map((m) => m[0]);
  assert.ok(imgs.length >= 8);
  for (const img of imgs) {
    assert.match(img, /src="\/docs-media\/how-to\/images\/console-[a-z-]+\.webp"/);
    assert.match(img, /width="1440"/);
    assert.match(img, /height="900"/);
    assert.match(img, /alt="[^"]{20,}"/);
  }
});
