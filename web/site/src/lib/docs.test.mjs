// Which files of docs/ get a page and where the sidebar puts them. The
// promise: every file but the one left out on purpose has a page, a file
// that is not there yet is skipped, and a file nobody listed is never lost.
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const docs = await import("./docs.ts");
const await_site = await import("./site.ts");
const { STOP_WORDS, searchWords } = await import("./search.ts");
const { isCodeTerm } = await import("./search-index.ts");

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
    // The page that lists the guides, then the guides: by name, while nothing says which that page lists.
    ["Guides", ["how-to", "how-to/first-project", "how-to/tasks", "how-to/troubleshooting", "how-to/zebra"]],
    ["The agents", ["board", "harness", "self-improvement", "evolve", "review", "landing"]],
    ["Machines and running", ["machines", "runner", "release", "recovery", "security"]],
    ["Reference", ["commands", "settings", "reports", "owner-reservation", "development", "windows-port"]],
    ["Decisions and design", ["decisions/adr-069-delivery", "decisions/adr-070-web-first", "decisions/adr-071-open-tool", "design/computer-use"]],
  ]);
  assert.deepEqual(plan.excluded, ["context/environment.md"]);
  // What a folder's own page lists comes in that page's order, and the rest after it by name. A file it names that is not there is passed over.
  const listed = new Map([["how-to", ["how-to/troubleshooting.md", "how-to/first-project.md", "how-to/gone.md", "how-to/troubleshooting.md", "concepts.md"]]]);
  const guides = docs.planDocs(["concepts.md", "how-to/zebra.md", "how-to/tasks.md", "how-to/README.md", "how-to/troubleshooting.md", "how-to/first-project.md"], listed);
  assert.deepEqual(shape(guides), [
    ["Start", ["concepts"]],
    ["Guides", ["how-to", "how-to/troubleshooting", "how-to/first-project", "how-to/tasks", "how-to/zebra"]],
  ]);
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
  // A group that takes a folder whole takes the folder's own page with it, first; a guide nobody listed stays with the guides.
  assert.deepEqual(shape(docs.planDocs(["decisions/ADR-001-first.md", "decisions/README.md", "design/README.md", "how-to/new-guide.md", "how-to/README.md", "new-thing.md"])), [
    ["Guides", ["how-to", "how-to/new-guide"]],
    ["Decisions and design", ["decisions", "decisions/adr-001-first", "design"]],
    ["More", ["new-thing"]],
  ]);
  const hub = docs.loadDocs().bySlug.get(docs.GUIDES_DIR);
  assert.equal(hub.path, "/docs/how-to/");
  assert.equal(hub.label, "All guides");
  assert.equal(hub.group, docs.GUIDES_GROUP);
});

test("a folder's own page says in which order the sidebar lists the folder's pages", () => {
  const sources = [
    {
      file: "how-to/README.md",
      source: "# Guides\n\nRead [Concepts](../concepts.md) first.\n\n1. [B](b.md)\n2. [A](./a.md#top)\n3. [B again](b.md)\n\n```md\n[not a link](z.md)\n```\n\n[gone](gone.md), [outside](../other/x.md), [deeper][c], <https://example.com/a.md>, [this page](README.md)\n\n[c]: more/c.md\n",
    },
    { file: "how-to/a.md", source: "# A\n\n## Top\n" },
    { file: "how-to/b.md", source: "# B\n" },
    { file: "how-to/z.md", source: "# Z\n" },
    { file: "how-to/more/c.md", source: "# C\n" },
    { file: "concepts.md", source: "# Concepts\n\nSee [a guide](how-to/a.md).\n" },
    { file: "other/x.md", source: "# X\n" },
  ];
  // Each file of the folder once, as first linked. Not the page itself, a page elsewhere, a file that is not there, or a link inside code.
  assert.deepEqual([...docs.folderLists(sources)], [["how-to", ["how-to/b.md", "how-to/a.md", "how-to/more/c.md"]]]);
  const built = docs.buildDocs(sources);
  assert.deepEqual(
    built.groups.find((g) => g.title === docs.GUIDES_GROUP).docs.map((d) => d.slug),
    ["how-to", "how-to/b", "how-to/a", "how-to/more/c", "how-to/z"],
  );
  // Previous and Next under a page walk the same order.
  assert.equal(built.bySlug.get("how-to/b").next.path, "/docs/how-to/a/");
  assert.equal(built.bySlug.get("how-to/b").prev.path, "/docs/how-to/");
});

test("every guide is listed on the page of guides, and the sidebar walks them in that page's order", () => {
  const built = docs.loadDocs();
  const hub = built.bySlug.get(docs.GUIDES_DIR);
  const guides = built.docs.filter((d) => d.group === docs.GUIDES_GROUP && d !== hub).map((d) => d.file);
  assert.ok(guides.length > 0, "docs/how-to/ holds guides");
  // The guides the page links, as it first links each. Not the page itself: it may link to a heading of its own.
  const listed = [...new Set(hub.links.filter((l) => l.kind === "doc" && docs.docSlug(l.doc).startsWith(`${docs.GUIDES_DIR}/`)).map((l) => l.doc))];
  // A guide the page does not list is one nobody who reads the page finds.
  assert.deepEqual(guides.filter((f) => !listed.includes(f)), [], "docs/how-to/README.md links every guide: a new guide needs a line there");
  // The same order, whichever it is: Previous and Next under a guide follow the page that lists them.
  assert.deepEqual(guides, listed);
  assert.equal(built.groups.find((g) => g.title === docs.GUIDES_GROUP).docs[0], hub);
  // A guide is a page of its own group and nowhere else, so the sidebar never lists one under More.
  // (Any other new file of docs/ does land there, with a page of its own and no change to the site.)
  const more = built.groups.find((g) => g.title === docs.MORE_GROUP)?.docs ?? [];
  assert.deepEqual(more.filter((d) => d.file.startsWith(`${docs.GUIDES_DIR}/`)).map((d) => d.file), []);
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

test("every diagram in the docs is well drawn", () => {
  // A column that drifted by one, an open box, a line wider than the console's reading panel, a character the
  // fonts hold at another width: none of it shows in the Markdown, all of it shows on the page.
  // `node scripts/check-diagrams.mjs` lists every diagram with its place.
  const { drawing } = docs.loadDocs();
  assert.deepEqual(drawing, [], `badly drawn in the source Markdown:\n${drawing.join("\n")}`);
});

test("every diagram in the docs is drawn whole: no box, arrowhead or word of the text is lost", async () => {
  // The page draws a diagram from its characters (doc-diagram.ts). Whatever somebody writes next has to come out
  // whole too: as many boxes as the text closes, an arrowhead for every arrow, and every word, letter for letter.
  const { drawDiagram, wordsWidth } = await import("./doc-diagram.ts");
  const { findDiagrams } = await import("./diagram.ts");
  const { docs: pages } = docs.loadDocs();
  let seen = 0;
  for (const page of pages) {
    if (!page.diagrams.length) continue;
    const source = fs.readFileSync(path.join(docs.docsRoot(), page.file), "utf8");
    const lines = source.split(/\r?\n/);
    assert.equal((page.html.match(/<figure class="diagram"/g) ?? []).length, page.diagrams.length, `${page.file}: one drawing a diagram`);
    for (const found of findDiagrams(source)) {
      const fence = /^(\s*)(`{3,}|~{3,})/.exec(lines[found.line - 1]);
      const body = lines.slice(found.line, found.line + found.rows).map((row) => (row.startsWith(fence[1]) ? row.slice(fence[1].length) : row.trimStart()));
      const text = body.join("\n");
      const d = drawDiagram(text);
      const at = `docs/${page.file}:${found.line}`;
      const letters = (value) => [...value].filter((ch) => !/[\s\u2500-\u257F\u2190-\u2193]/.test(ch)).sort().join("");
      assert.equal(letters(d.words.map((w) => w.text).join("")), letters(text), `${at}: the words of the drawing are the words of the text`);
      assert.equal(d.tips.length, [...text].filter((ch) => "→←↑↓".includes(ch)).length, `${at}: an arrowhead for every arrow`);
      // A box is a rectangle of the text: a corner or a branch at each of its four corners, and no box cuts across another.
      // The drawing starts at the first used column of the block, so the boxes are counted from there.
      const indent = Math.min(...body.filter((row) => row.trim()).map((row) => row.length - row.trimStart().length));
      const grid = body.map((row) => [...row.slice(indent)]);
      for (const b of d.boxes) {
        const corners = [grid[b.top][b.left], grid[b.top][b.right], grid[b.bottom][b.left], grid[b.bottom][b.right]];
        assert.ok(corners.every((ch) => "┌┐└┘├┤┬┴┼".includes(ch)), `${at}: a box stands on four corners: ${corners.join("")}`);
        for (const o of d.boxes) {
          // Two boxes may share a side: a box with a rule across it is two boxes, one on the other.
          const apart = o.right <= b.left || o.left >= b.right || o.bottom <= b.top || o.top >= b.bottom;
          const inside = (x, y) => x.left >= y.left && x.right <= y.right && x.top >= y.top && x.bottom <= y.bottom;
          assert.ok(apart || inside(o, b) || inside(b, o), `${at}: two boxes cut across each other`);
        }
      }
      // A box is closed at its lower right by a character that reaches up and left; there are no more boxes than those.
      assert.ok(d.boxes.length <= [...text].filter((ch) => "┘┤┴┼".includes(ch)).length, `${at}: no more boxes than corners that close one`);
      if (text.includes("┘") && text.includes("┌")) assert.ok(d.boxes.length > 0, `${at}: the boxes of the text are found`);
      assert.ok(d.width <= 72 * 8, `${at}: no wider than 72 characters`);
      // The words are set in the site's text font, which is narrower than the cells: no run may be wider than the cells
      // it was typed into, or it would run into the line after it. A quarter of a cell is let go: a space always follows.
      for (const w of d.words) {
        const wide = wordsWidth(w.text);
        assert.ok(wide <= w.cells * 8 + 2, `${at}: "${w.text}" is ${wide.toFixed(1)} wide in cells ${w.cells * 8} wide`);
        // A run set from its middle stays within half a cell of its cells, or in its box when it is centred there.
        if (w.middle !== undefined && !w.boxed) assert.ok(w.middle - wide / 2 >= w.column * 8 - 4.01 && w.middle + wide / 2 <= (w.column + w.cells) * 8 + 4.01, `${at}: "${w.text}" is centred out of its cells`);
      }
      seen += 1;
    }
  }
  assert.ok(seen >= 30, `the docs hold their diagrams: ${seen}`);
});

test("a badly drawn diagram is reported by file and line", () => {
  const built = docs.buildDocs([{ file: "concepts.md", source: "# Concepts\n\nWhat Hive is.\n\n```text\n┌──┐\n│  │\n└──\n```\n" }]);
  assert.deepEqual(built.drawing, ["docs/concepts.md:5: the diagram: line 2, column 4: │ reaches down, and the row below has no line or arrowhead in that column"]);
  assert.deepEqual(built.docs[0].diagrams.map((d) => d.line), [5]);
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

test("the search index holds the site's pages and every docs page with its sections, and leaves out nothing a reader may type", () => {
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
    // Nothing a reader may type is left out of a section: every word of its text but the words of grammar and
    // single letters is in its page's title, its heading or its words, and every command and setting it names is kept.
    const title = searchWords(e.t);
    const sources = doc.sections.slice(1);
    e.h.forEach(([, heading, terms = "", code = ""], i) => {
      const where = `${e.u} "${heading}"`;
      assert.doesNotMatch(terms, /[^a-z0-9 ]/, `${where}: words only`);
      const kept = terms ? terms.split(" ") : [];
      assert.equal(new Set(kept).size, kept.length, `${where}: each word once`);
      assert.deepEqual(kept.filter((w) => STOP_WORDS.has(w) || w.length < 2), [], `${where}: no word of grammar`);
      const findable = new Set([...title, ...searchWords(heading), ...kept]);
      assert.deepEqual(searchWords(sources[i].text).filter((w) => w.length > 1 && !STOP_WORDS.has(w) && !findable.has(w)), [], `${where}: a word of its text is not in the index`);
      assert.deepEqual(code ? code.split("\n") : [], [...new Set(sources[i].code.filter(isCodeTerm))], `${where}: the commands and settings it names`);
    });
  }
  // A setting and a command are in the index as written.
  const names = (needle) => index.some((e) => e.h.some(([, , , code = ""]) => code.split("\n").includes(needle)));
  assert.ok(names("evolve.loop"));
  assert.ok(names("review.merge"));

  // The decisions and the design notes are marked as records, and nothing else is.
  assert.deepEqual(index.filter((e) => e.r).map((e) => e.g), built.docs.filter((d) => d.group === docs.RECORDS_GROUP).map(() => docs.RECORDS_GROUP));

  // The index grows with the docs and is not capped. It stays well under the text it is made from: a section's words once each.
  const text = built.docs.reduce((n, d) => n + d.sections.reduce((m, s) => m + s.text.length, 0), 0);
  const size = JSON.stringify(index).length;
  assert.ok(size < text * 0.6, `the index is ${size} bytes for ${text} bytes of text`);
});

test("only raster pictures of the docs are carried: a file that can hold script is never one of them", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "hive-docs-"));
  try {
    fs.mkdirSync(path.join(root, "how-to", "images"), { recursive: true });
    for (const f of ["b.png", "A.WEBP", "diagram.svg", "notes.txt", ".hidden.png", "how-to/page.html", "how-to/images/shot.jpg"]) fs.writeFileSync(path.join(root, f), "x");
    assert.deepEqual(docs.listDocPictures(root), ["A.WEBP", "b.png", "how-to/images/shot.jpg"]);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
  assert.deepEqual(docs.listDocPictures(root), []);
});

test("the pictures of the docs are listed for the export, and each page that shows one keeps its place", () => {
  const pictures = docs.listDocPictures(docs.docsRoot());
  assert.ok(pictures.includes("how-to/images/console-sessions.webp"));
  const built = docs.loadDocs();
  const shown = new Set(built.docs.flatMap((d) => d.links.filter((l) => l.kind === "repo" && l.repoPath?.startsWith("docs/") && /\.(png|webp|jpe?g|gif|avif)$/i.test(l.repoPath)).map((l) => l.repoPath.slice("docs/".length))));
  // No picture is kept that no page shows.
  assert.deepEqual([...shown].sort(), [...pictures].sort());
  // A page shows its pictures from the site itself, each with the size read from its file and with words for those who cannot see it.
  const tour = built.bySlug.get("how-to/console-tour");
  const imgs = [...tour.html.matchAll(/<img\b[^>]*>/g)].map((m) => m[0]);
  assert.ok(imgs.length > 0);
  for (const img of imgs) {
    assert.match(img, /src="\/docs-media\/how-to\/images\/[^"]+"/);
    assert.match(img, /width="\d+"/);
    assert.match(img, /height="\d+"/);
    assert.match(img, /alt="[^"]+"/);
  }
});
