import assert from "node:assert/strict";
import { test } from "node:test";
import { addTabs, afterClose, autoOpen, changedNewLines, editBlocks, extractLinks, fileName, latestPlanStep, linkLabel, makeTab, pageLabel, pageText, readPaths, stepPage, stepPath, tabId, terminalEntries, touchedFiles, visitedPages } from "./canvas.ts";

let n = 0;
const step = (kind, content, over = {}) => ({ n: ++n, at: "2026-10-27T10:0" + (n % 10) + ":00Z", kind, content, command: "", exit_code: null, duration_ms: 0, prompt_tokens: 0, completion_tokens: 0, model: "", ...over });
const reset = () => {
  n = 0;
};

test("stepPath reads the path off an edit or a write, nothing off other steps", () => {
  assert.equal(stepPath(step("edit", "src/http.ts\n<<<<<<< SEARCH\na\n=======\nb\n>>>>>>> REPLACE")), "src/http.ts");
  assert.equal(stepPath(step("edit", "src/http.ts: 1 block replaced")), "src/http.ts");
  assert.equal(stepPath(step("write", "docs/notes.md (1234 bytes)")), "docs/notes.md");
  assert.equal(stepPath(step("read", "src/http.ts:40-50")), "");
  assert.equal(stepPath(step("command", "ls")), "");
});

test("readPaths and stepPage strip ranges and keep the first line", () => {
  assert.deepEqual(readPaths(step("read", "src/http.ts:40-50\nREADME.md\n")), ["src/http.ts", "README.md"]);
  assert.equal(stepPage(step("fetch", "https://docs.rs/reqwest/latest/")), "https://docs.rs/reqwest/latest/");
  assert.equal(stepPage(step("search", "retry after header")), "retry after header");
  assert.equal(stepPage(step("command", "curl x")), "");
});

test("labels: file names, page hosts, search queries", () => {
  assert.equal(fileName("web/hub/src/lib/canvas.ts"), "canvas.ts");
  assert.equal(fileName("README.md"), "README.md");
  assert.equal(pageLabel("https://www.docs.rs/reqwest/latest/reqwest/"), "docs.rs/reqwest");
  assert.equal(pageLabel("https://example.com"), "example.com");
  assert.equal(pageLabel("retry after"), "Search: retry after");
  assert.equal(makeTab("file", "src/a.ts").id, "file:src/a.ts");
  assert.equal(makeTab("terminal").label, "Terminal");
  assert.equal(tabId("artifact", "report.html"), "artifact:report.html");
});

test("touchedFiles and visitedPages are newest first and unique", () => {
  reset();
  const steps = [step("edit", "a.ts\n<<<<<<< SEARCH\nx\n=======\ny\n>>>>>>> REPLACE"), step("read", "b.ts:1-3"), step("write", "c.ts (10 bytes)"), step("edit", "a.ts\n<<<<<<< SEARCH\nx\n=======\nz\n>>>>>>> REPLACE"), step("fetch", "https://x.org/p"), step("search", "q"), step("fetch", "https://x.org/p")];
  assert.deepEqual(touchedFiles(steps), ["a.ts", "c.ts", "b.ts"]);
  assert.deepEqual(visitedPages(steps), ["https://x.org/p", "q"]);
});

test("terminalEntries pairs each command with the observation after it", () => {
  reset();
  const steps = [
    step("command", "npm test", { command: "npm test", title: "Run the tests" }),
    step("observation", "FAIL 1 test", { command: "npm test", exit_code: 1, duration_ms: 1200 }),
    step("assistant", "Fixing it."),
    step("command", "npm test", { command: "npm test" }),
    step("observation", "PASS", { command: "npm test", exit_code: 0, duration_ms: 900 }),
    step("command", "sleep 5", { command: "sleep 5 &", depth: 1 }),
    step("observation", "started", { depth: 1 }),
    step("process", "output 3", { command: "output 3" }),
  ];
  const live = terminalEntries(steps, true);
  assert.equal(live.length, 3, "the subagent's command stays out");
  assert.equal(live[0].command, "npm test");
  assert.equal(live[0].title, "Run the tests");
  assert.equal(live[0].output, "FAIL 1 test");
  assert.equal(live[0].exit_code, 1);
  assert.equal(live[0].duration_ms, 1200);
  assert.equal(live[0].running, false);
  assert.equal(live[1].exit_code, 0);
  assert.equal(live[2].kind, "process");
  assert.equal(live[2].running, true, "the last command with no observation yet runs while the session is live");
  assert.equal(terminalEntries(steps, false)[2].running, false);
});

test("pageText finds what a fetch left; editBlocks the blocks for a path", () => {
  reset();
  const steps = [step("fetch", "https://x.org/p"), step("observation", "# Page\nbody", { exit_code: 0 }), step("edit", "a.ts\n<<<<<<< SEARCH\nold\n=======\nnew\n>>>>>>> REPLACE\n<<<<<<< SEARCH\no2\n=======\nn2\n>>>>>>> REPLACE"), step("edit", "b.ts\n<<<<<<< SEARCH\nq\n=======\nr\n>>>>>>> REPLACE")];
  assert.deepEqual(pageText(steps, "https://x.org/p"), { text: "# Page\nbody", at: steps[1].at, exit_code: 0 });
  assert.equal(pageText(steps, "https://none"), null);
  const blocks = editBlocks(steps, "a.ts");
  assert.equal(blocks.length, 2);
  assert.deepEqual([blocks[0].search, blocks[0].replace, blocks[1].search, blocks[1].replace], ["old", "new", "o2", "n2"]);
  assert.equal(editBlocks(steps, "c.ts").length, 0);
  assert.equal(latestPlanStep(steps), null);
});

test("autoOpen opens what new steps touch and brings the newest to the front", () => {
  reset();
  const steps = [step("plan", "plan: 0/2 done\n- [~] a\n- [ ] b"), step("command", "npm test", { command: "npm test" }), step("observation", "ok"), step("edit", "src/a.ts\n<<<<<<< SEARCH\nx\n=======\ny\n>>>>>>> REPLACE"), step("observation", "edited")];
  const all = autoOpen(steps, 0);
  assert.deepEqual(
    all.open.map((t) => t.id),
    ["plan", "terminal", "file:src/a.ts"],
  );
  assert.equal(all.front, "file:src/a.ts");
  // Only what came after step 4 counts on the next pass.
  const later = autoOpen([...steps, step("fetch", "https://x.org/doc"), step("observation", "text")], 5);
  assert.deepEqual(
    later.open.map((t) => t.id),
    ["page:https://x.org/doc"],
  );
  assert.equal(later.front, "page:https://x.org/doc");
  // A plan alone comes forward.
  const planOnly = autoOpen([step("plan", "plan: 1/2 done\n- [x] a\n- [~] b")], 7);
  assert.equal(planOnly.front, "plan");
  // Nothing new, nothing opens.
  assert.deepEqual(autoOpen(steps, 99), { open: [], front: null });
});

test("autoOpen's first look at a long run opens the last three files and one page", () => {
  reset();
  const steps = [];
  for (const p of ["a", "b", "c", "d", "e"]) steps.push(step("write", p + ".ts (1 bytes)"));
  steps.push(step("fetch", "https://x.org/1"), step("fetch", "https://x.org/2"));
  steps.push(step("write", "a.ts (2 bytes)"));
  const first = autoOpen(steps, 0);
  assert.deepEqual(
    first.open.map((t) => t.id),
    ["file:d.ts", "file:e.ts", "page:https://x.org/2", "file:a.ts"],
  );
  assert.equal(first.front, "file:a.ts");
  // Subagent steps do not open tabs.
  assert.deepEqual(autoOpen([step("write", "z.ts (1 bytes)", { depth: 1 })], 0).open, []);
});

test("tab lists: addTabs keeps order, afterClose picks the left neighbour", () => {
  const a = makeTab("terminal");
  const b = makeTab("file", "a.ts");
  const c = makeTab("plan");
  assert.deepEqual(
    addTabs([a, b], [b, c]).map((t) => t.id),
    ["terminal", "file:a.ts", "plan"],
  );
  assert.deepEqual(afterClose([a, b, c], "file:a.ts", "file:a.ts"), { tabs: [a, c], active: "terminal" });
  assert.deepEqual(afterClose([a, b, c], "plan", "file:a.ts"), { tabs: [a, c], active: "plan" });
  assert.deepEqual(afterClose([a], "terminal", "terminal"), { tabs: [], active: "" });
  assert.deepEqual(afterClose([a, b], "terminal", "terminal"), { tabs: [b], active: "file:a.ts" });
  assert.deepEqual(afterClose([a], "terminal", "nope"), { tabs: [a], active: "terminal" });
});

test("links come out newest first, once each, with short labels", () => {
  const links = extractLinks(["See https://github.com/o/r/pull/12 and https://docs.rs/x/latest/x/.", "Opened https://github.com/o/r/pull/12.", "Home http://localhost:8765/chat/ and https://example.com/a/very/long/path/that/goes/on/and/on/for/a/while/longer"], "localhost:8765");
  assert.deepEqual(
    links.map((l) => l.url),
    ["https://example.com/a/very/long/path/that/goes/on/and/on/for/a/while/longer", "https://github.com/o/r/pull/12", "https://docs.rs/x/latest/x/"],
  );
  assert.equal(links[1].label, "o/r#12");
  assert.equal(linkLabel("https://docs.rs/x/latest/x/"), "docs.rs/x/latest/x");
  assert.equal(links[0].label.startsWith("example.com…"), true);
  assert.deepEqual(extractLinks([]), []);
});

test("changedNewLines maps a file's hunks to the new line numbers added", () => {
  const lines = [
    { kind: "meta", text: "--- a/x" },
    { kind: "hunk", text: "@@ -10,3 +10,5 @@" },
    { kind: "context", text: "a" },
    { kind: "add", text: "b" },
    { kind: "add", text: "c" },
    { kind: "del", text: "gone" },
    { kind: "context", text: "d" },
    { kind: "hunk", text: "@@ -30,1 +32,2 @@ fn x(a: i32) -> i32 {" },
    { kind: "add", text: "e" },
  ];
  assert.deepEqual([...changedNewLines(lines)], [11, 12, 32]);
});
