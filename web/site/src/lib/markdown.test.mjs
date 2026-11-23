// The Markdown the docs pages are made of: heading ids, link rewriting, code
// kept as written. What breaks quietly here breaks every anchor on the site.
import test from "node:test";
import assert from "node:assert/strict";
import { unified } from "unified";
import rehypeParse from "rehype-parse";
import { toString } from "hast-util-to-string";
import { visit } from "unist-util-visit";

const md = await import("./markdown.ts");

const pages = new Map([
  ["harness.md", "/docs/harness/"],
  ["machines.md", "/docs/machines/"],
  ["landing.md", "/docs/landing/"],
  ["decisions/ADR-069-delivery.md", "/docs/decisions/adr-069-delivery/"],
  ["design/computer-use.md", "/docs/design/computer-use/"],
]);
const inRepo = new Map([
  ["crates/hive-board/src/posts.rs", "file"],
  ["crates/hive-hand", "dir"],
  ["README.md", "file"],
  ["scripts/install.sh", "file"],
  ["docs/context/environment.md", "file"],
  ["LICENSE", "file"],
  [".github/assets/console-task.png", "file"],
]);
const ctx = (file) => ({ file, pages, repoHas: (p) => inRepo.get(p) ?? null });
const render = (source, file = "harness.md") => md.renderMarkdown(source, ctx(file));
const parse = (html) => unified().use(rehypeParse, { fragment: true }).parse(html);
function elements(html, tag) {
  const found = [];
  visit(parse(html), "element", (node) => {
    if (node.tagName === tag) found.push(node);
  });
  return found;
}

test("headings get the ids GitHub gives them", () => {
  const r = render(
    [
      "# The harness (how an agent runs)",
      "## `daemon.self_update release`: a machine without source",
      "## The project's own files: `.hive/`",
      "### One git login for every machine: a GitHub App (ADR-070 step 3)",
      "## Standing jobs: `hive schedule`",
      "## (a) One local slot kept for the owner",
      "## 1. The tool",
      "## Events",
      "## Events",
      "### Events",
    ].join("\n\n"),
  );
  assert.equal(r.titleId, "the-harness-how-an-agent-runs");
  assert.deepEqual(
    r.toc.map((e) => e.id),
    [
      "daemonself_update-release-a-machine-without-source",
      "the-projects-own-files-hive",
      "one-git-login-for-every-machine-a-github-app-adr-070-step-3",
      "standing-jobs-hive-schedule",
      "a-one-local-slot-kept-for-the-owner",
      "1-the-tool",
      "events",
      "events-1",
      "events-2",
    ],
  );
  assert.deepEqual(r.ids, ["the-harness-how-an-agent-runs", ...r.toc.map((e) => e.id)]);
  // Every heading left in the body carries its id and a permalink to itself.
  for (const e of r.toc) assert.match(r.html, new RegExp(`<h[23] id="${e.id}">[^]*?<a class="anchor" href="#${e.id}"`));
  // The permalink is for the eye: it adds no text to the heading and no stop to the Tab key.
  const headings = [...elements(r.html, "h2"), ...elements(r.html, "h3")];
  assert.deepEqual(headings.map((h) => toString(h)).sort(), r.toc.map((e) => e.text).sort());
  for (const h of headings) {
    const a = h.children.at(-1);
    assert.deepEqual([a.tagName, a.properties.ariaHidden, a.properties.tabIndex, a.children.length], ["a", "true", -1, 0]);
  }
});

test("the first heading is the title and leaves the body; the page keeps one h1", () => {
  const r = render("# Releases and `hive update`\n\nHow a machine gets a newer Hive.\n\n# A second top heading\n\n## A section\n\n#### Deep\n");
  assert.equal(r.title, "Releases and hive update");
  assert.equal(r.titleHtml, "Releases and <code>hive update</code>");
  assert.equal(elements(r.html, "h1").length, 0);
  assert.deepEqual(
    r.toc,
    [
      { id: "a-second-top-heading", text: "A second top heading", level: 2 },
      { id: "a-section", text: "A section", level: 2 },
    ],
  );
  assert.deepEqual(r.paragraphs, ["How a machine gets a newer Hive."]);
});

test("a file without a heading still renders, with no title", () => {
  const r = render("Just a paragraph.\n");
  assert.equal(r.title, "");
  assert.equal(r.titleId, "");
  assert.match(r.html, /<p>Just a paragraph\.<\/p>/);
});

test("a link on the same page stays an anchor", () => {
  assert.deepEqual(md.rewriteHref("#the-tools", ctx("harness.md")), { href: "#the-tools", kind: "anchor", anchor: "the-tools" });
});

test("a link to another doc becomes that doc's page, anchor kept", () => {
  const to = (href, file = "harness.md") => md.rewriteHref(href, ctx(file));
  assert.equal(to("machines.md").href, "/docs/machines/");
  assert.equal(to("./machines.md").href, "/docs/machines/");
  assert.equal(to("machines.md#pairing-hands").href, "/docs/machines/#pairing-hands");
  assert.deepEqual(to("landing.md#credentials"), { href: "/docs/landing/#credentials", kind: "doc", doc: "landing.md", anchor: "credentials" });
  // Into a folder, out of a folder, and between folders. The address is lowercased, the file name is not.
  assert.equal(to("decisions/ADR-069-delivery.md").href, "/docs/decisions/adr-069-delivery/");
  assert.equal(to("../machines.md#the-hand", "decisions/ADR-069-delivery.md").href, "/docs/machines/#the-hand");
  assert.equal(to("../design/computer-use.md", "decisions/ADR-069-delivery.md").href, "/docs/design/computer-use/");
  // GitHub reads a leading slash as the repository's root.
  assert.equal(to("/docs/harness.md#hooks", "machines.md").href, "/docs/harness/#hooks");
});

test("a link into the repository becomes its file on GitHub", () => {
  const to = (href, file = "board.md") => md.rewriteHref(href, ctx(file));
  assert.deepEqual(to("../crates/hive-board/src/posts.rs"), {
    href: "https://github.com/lamplitlabs/hive/blob/main/crates/hive-board/src/posts.rs",
    kind: "repo",
    anchor: undefined,
  });
  assert.equal(to("../crates/hive-hand/").href, "https://github.com/lamplitlabs/hive/tree/main/crates/hive-hand");
  assert.equal(to("../README.md").href, "https://github.com/lamplitlabs/hive/blob/main/README.md");
  assert.equal(to("../README.md#how-it-works").href, "https://github.com/lamplitlabs/hive/blob/main/README.md#how-it-works");
  assert.equal(to("../../LICENSE", "decisions/ADR-069-delivery.md").href, "https://github.com/lamplitlabs/hive/blob/main/LICENSE");
  // Written from the repository's root, as prose often does.
  assert.equal(to("scripts/install.sh").href, "https://github.com/lamplitlabs/hive/blob/main/scripts/install.sh");
  assert.equal(to("scripts/install.sh").problem, undefined);
  // The one doc that has no page is still a file of the repository.
  assert.equal(to("context/environment.md").href, "https://github.com/lamplitlabs/hive/blob/main/docs/context/environment.md");
});

test("a README section the site covers points at the site's page", () => {
  assert.deepEqual(md.rewriteHref("../README.md#install", ctx("console.md")), { href: "/install/", kind: "site" });
});

test("a link that is broken in the Markdown is rendered and reported, not hidden", () => {
  const gone = md.rewriteHref("../crates/nowhere.rs", ctx("board.md"));
  assert.equal(gone.href, "https://github.com/lamplitlabs/hive/blob/main/crates/nowhere.rs");
  assert.match(gone.problem, /crates\/nowhere\.rs, which is not in the repository/);
  const missingDoc = md.rewriteHref("no-such-doc.md#x", ctx("board.md"));
  assert.equal(missingDoc.kind, "repo");
  assert.match(missingDoc.problem, /docs\/no-such-doc\.md, which is not in the repository/);
  assert.match(md.rewriteHref("../../outside.md", ctx("board.md")).problem, /leaves the repository/);
});

test("external links keep their address and get rel=noopener noreferrer; the site's own address stays here", () => {
  const r = render(
    "# T\n\nSee [mini-swe-agent](https://github.com/SWE-agent/mini-swe-agent), https://example.com/a, [the site](https://hivefleet.dev/install/), [mail](mailto:a@example.com), [posts](../crates/hive-board/src/posts.rs) and [machines](machines.md).\n",
  );
  const links = elements(r.html, "a").map((a) => [a.properties.href, (a.properties.rel ?? []).join(" ")]);
  assert.deepEqual(links, [
    ["https://github.com/SWE-agent/mini-swe-agent", "noopener noreferrer"],
    ["https://example.com/a", "noopener noreferrer"],
    ["/install/", ""],
    ["mailto:a@example.com", ""],
    ["https://github.com/lamplitlabs/hive/blob/main/crates/hive-board/src/posts.rs", "noopener noreferrer"],
    ["/docs/machines/", ""],
  ]);
  assert.deepEqual(
    r.links.map((l) => l.kind),
    ["external", "external", "site", "other", "repo", "doc"],
  );
});

test("a link with a scheme a public page should not carry is taken off and reported", () => {
  const r = render("# T\n\n[run](javascript:alert(1)) and [file](file:///etc/passwd) and [data](data:text/html,x) stay words; [mail](mailto:a@example.com) stays a link.\n");
  const links = elements(r.html, "a").map((a) => a.properties.href);
  assert.deepEqual(links, [undefined, undefined, undefined, "mailto:a@example.com"]);
  assert.doesNotMatch(r.html, /javascript:|file:\/\/|data:text/);
  assert.equal(r.links.filter((l) => /uses a scheme the site does not link to/.test(l.problem ?? "")).length, 3);
});

test("a picture kept in the repository is served by GitHub, with its alt text", () => {
  const r = render("# T\n\n![The console's Home page](img/home.png) ![](../.github/assets/console-task.png) ![logo](https://example.com/logo.svg)\n", "console.md");
  const imgs = elements(r.html, "img").map((i) => [i.properties.src, i.properties.alt, i.properties.loading]);
  assert.deepEqual(imgs, [
    ["https://github.com/lamplitlabs/hive/raw/main/docs/img/home.png", "The console's Home page", "lazy"],
    ["https://github.com/lamplitlabs/hive/raw/main/.github/assets/console-task.png", "", "lazy"],
    ["https://example.com/logo.svg", "logo", "lazy"],
  ]);
  // The first is not in the test's repository: reported, and still rendered.
  assert.match(r.links[0].problem, /docs\/img\/home\.png, which is not in the repository/);
});

test("code blocks keep their text exactly, highlighted or not", () => {
  const bash = `hive settings set machine.labels gpu,android   # what <this> machine offers & more\nexport PATH="$HOME/.hive/bin:$PATH"\n\tif [ -z "$X" ]; then echo 'a  b'; fi`;
  const plain = `system prompt (+ the clone's AGENTS.md) + the job's brief\n  ─► model answers <one> & "two"\n`;
  const json = `{"hooks": [{"event": "pre_tool", "tools": ["bash"]}]}`;
  const r = render(`# T\n\n\`\`\`bash\n${bash}\n\`\`\`\n\n\`\`\`text\n${plain}\`\`\`\n\n\`\`\`\n${plain}\`\`\`\n\n\`\`\`json\n${json}\n\`\`\`\n\n\`\`\`unknownlang\n${json}\n\`\`\`\n`);
  const blocks = elements(r.html, "pre").map((pre) => toString(pre));
  assert.deepEqual(blocks, [`${bash}\n`, plain, plain, `${json}\n`, `${json}\n`]);
  // The bash and json blocks are highlighted at build time; the plain ones carry no spans.
  const pres = elements(r.html, "pre");
  const spans = (pre) => {
    let n = 0;
    visit(pre, "element", (node) => {
      if (node.tagName === "span") n += 1;
    });
    return n;
  };
  assert.ok(spans(pres[0]) > 0, "bash is highlighted");
  assert.equal(spans(pres[1]), 0);
  assert.equal(spans(pres[2]), 0);
  assert.ok(spans(pres[3]) > 0, "json is highlighted");
  assert.equal(spans(pres[4]), 0);
  assert.equal(md.highlightCode(bash, "bash").replace(/<[^>]+>/g, "").replace(/&#x3C;|&lt;/g, "<").replace(/&#x26;|&amp;/g, "&").replace(/&gt;/g, ">"), bash);
});

test("GitHub's flavour: tables scroll in their own box, task lists, strikethrough, autolinks", () => {
  const r = render("# T\n\n| Setting | Default |\n| --- | --- |\n| `a.b` | `1` |\n\n- [x] done\n- [ ] not yet\n\n~~gone~~ and www.example.com\n");
  const wrap = elements(r.html, "div")[0];
  assert.deepEqual(wrap.properties.className, ["table-scroll"]);
  assert.equal(wrap.properties.tabIndex, 0);
  assert.equal(wrap.properties.role, undefined);
  assert.equal(wrap.children[0].tagName, "table");
  const boxes = elements(r.html, "input");
  // A screen reader says "Done, checkbox, checked" or "not checked", not a bare checkbox.
  assert.deepEqual(boxes.map((b) => [b.properties.ariaLabel, Boolean(b.properties.checked)]), [["Done", true], ["Done", false]]);
  assert.equal(elements(r.html, "del").length, 1);
  assert.equal(elements(r.html, "a")[0].properties.href, "http://www.example.com");
  assert.equal(elements(r.html, "pre").length, 0);
});

test("a table with an empty first heading names its rows in its first column; any other table is left alone", () => {
  const cells = (html) => elements(html, "tr").map((row) => row.children.filter((c) => c.type === "element").map((c) => `${c.tagName}${c.properties.scope ? `[${c.properties.scope}]` : ""}`));
  const matrix = render("# T\n\n| | `require` | `off` |\n| --- | --- | --- |\n| The brief | asks | does not ask |\n| A pull request | waits | merges |\n");
  assert.deepEqual(cells(matrix.html), [
    ["td", "th", "th"],
    ["th[row]", "td", "td"],
    ["th[row]", "td", "td"],
  ]);
  const plain = render("# T\n\n| Setting | Default |\n| --- | --- |\n| `a.b` | `1` |\n");
  assert.deepEqual(cells(plain.html), [
    ["th", "th"],
    ["td", "td"],
  ]);
});

test("raw HTML in a doc is shown as the text it is: a placeholder is never dropped and never run", () => {
  const r = render("# T\n\nThe branch is `hive/<agent>/<job>` and the note says *Review failed: <error>* for EVO-<slug>.\n\n<!-- a comment -->\n\n<script>alert(1)</script>\n");
  assert.match(r.html, /Review failed: &#x3C;error>|Review failed: &lt;error&gt;/);
  assert.match(r.html, /EVO-(&#x3C;|&lt;)slug/);
  assert.doesNotMatch(r.html, /<script/);
  assert.doesNotMatch(r.html, /a comment/);
  assert.equal(elements(r.html, "script").length, 0);
});

test("the YAML block an ADR opens with is read and left out of the page", () => {
  const src = "---\nid: ADR-069\ntitle: Delivery by pull request\nstatus: Proposed\ndate: 2026-11-04\nareas:\n  - runner\n---\n\n# ADR-069: Delivery by pull request\n\n## Context\n\nSince ADR-024 the daemon merged.\n";
  const { data, body } = md.splitFrontMatter(src);
  assert.deepEqual(data, { id: "ADR-069", title: "Delivery by pull request", status: "Proposed", date: "2026-11-04" });
  assert.ok(body.trimStart().startsWith("# ADR-069"));
  const r = render(src, "decisions/ADR-069-delivery.md");
  assert.equal(r.title, "ADR-069: Delivery by pull request");
  assert.equal(r.frontMatter.status, "Proposed");
  assert.doesNotMatch(r.html, /areas|runner/);
  assert.deepEqual(md.splitFrontMatter("# No block\n\n---\n\nText\n"), { data: {}, body: "# No block\n\n---\n\nText\n" });
});

test("a description is cut at a sentence when one ends late enough, else at a word", () => {
  const long =
    "Every finished branch becomes a pull request on the project's upstream; this page is about who merges it. A review job reads each pull request Hive opens and merges it when the change is one the agents may decide on.";
  assert.equal(md.cutClean(long, 155, 70), "Every finished branch becomes a pull request on the project's upstream; this page is about who merges it.");
  const noStop = "Suggestions collect votes and a quorum of voters decides each of them while the planner takes every eligible suggestion that passes the gates and queues one worker job per card for it";
  const cut = md.cutClean(noStop, 155, 70);
  assert.ok(cut.length <= 155 && cut.endsWith("…"), cut);
  assert.ok(noStop.startsWith(cut.slice(0, -1)));
  assert.doesNotMatch(cut, /\s…$/);
  // A first sentence too short to describe the page is not where the cut falls.
  const short = `Design note. ${"The surrounding behaviour is described elsewhere in more words than fit here ".repeat(4)}`;
  assert.ok(md.cutClean(short, 155, 70).length > 70);
  assert.equal(md.cutClean("  Short   text. ", 155), "Short text.");
  // "e.g." ends no sentence.
  assert.doesNotMatch(md.cutClean(`${"word ".repeat(18)}e.g. ${"more ".repeat(30)}`, 155, 70), /e\.g\.$/);
});

test("a long title is cut at a clause", () => {
  const adr = "ADR-070: Web-first Hive - a hosted brain per team, an open hand enrolled once per machine, one web login that every machine inherits";
  assert.equal(md.cutTitle(adr, 53), "ADR-070: Web-first Hive - a hosted brain per team");
  assert.equal(md.cutTitle("Short", 53), "Short");
  const words = md.cutTitle("A very long heading that has no clause to break at anywhere within its many words", 40);
  assert.ok(words.length <= 40 && words.endsWith("…"));
});
