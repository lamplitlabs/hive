// Diagrams drawn in text: which fenced blocks are one, and what the checker
// calls badly drawn. Nobody sees a column that drifted by one in the source,
// so the checker has to.
import test from "node:test";
import assert from "node:assert/strict";

const { isDiagram, checkDiagram, diagramWords, findDiagrams, DIAGRAM_MAX_COLUMNS } = await import("./diagram.ts");

const GOOD = [
  "┌────────┐  task   ┌────────┐",
  "│ owner  │ ──────→ │ lead   │",
  "└────────┘         └───┬────┘",
  "                       │ jobs",
  "                       ↓",
  "                  ┌─────────┐",
  "                  │ agents  │",
  "                  └─────────┘",
].join("\n");

test("a plain block with a line character is a diagram; code and sample output are not", () => {
  assert.equal(isDiagram(GOOD), true);
  assert.equal(isDiagram(GOOD, "text"), true);
  assert.equal(isDiagram("hive status\n"), false);
  assert.equal(isDiagram("a → b\n", "text"), false, "an arrow alone is sample output, not a drawing");
  assert.equal(isDiagram(GOOD, "bash"), false, "a block with a language is code, whatever it holds");
});

test("a well-drawn diagram has no problems", () => {
  assert.deepEqual(checkDiagram(GOOD), []);
  assert.deepEqual(checkDiagram(`${GOOD}\n\n`), [], "the blank lines a fence ends with are not rows");
});

test("a label may sit on a line, and arrows may stand between words", () => {
  assert.deepEqual(checkDiagram("task ── planned ──→ jobs → pull request\n  ↓\n done"), []);
});

test("a listing of folders has no box, so its lines may start under a name", () => {
  const tree = [".", "├── crates", "│   └── hive-records", "└── docs"].join("\n");
  assert.equal(isDiagram(tree), true, "it is still drawn with the lines closed up");
  assert.deepEqual(checkDiagram(tree), []);
  assert.match(checkDiagram(`${tree}\n└── ${"x".repeat(DIAGRAM_MAX_COLUMNS)}`)[0], /columns wide/, "its width is still checked");
});

test("a vertical line that drifts by one column is named with its place", () => {
  const drifted = GOOD.replace("                       │ jobs", "                      │ jobs");
  const problems = checkDiagram(drifted);
  assert.equal(problems.length, 3, problems.join("\n"));
  assert.match(problems[0], /^line 3, column 24: ┬ reaches down, and the row below has no line or arrowhead in that column$/);
  assert.match(problems[1], /^line 4, column 23: │ reaches up/);
  assert.match(problems[2], /^line 4, column 23: │ reaches down/);
});

test("a box that is not closed is named", () => {
  const open = ["┌─────┐", "│ job │", "└─────"].join("\n");
  assert.deepEqual(checkDiagram(open), ["line 2, column 7: │ reaches down, and the row below has no line or arrowhead in that column"]);
  const gap = ["┌──── ┐", "│ job │", "└─────┘"].join("\n");
  assert.deepEqual(checkDiagram(gap), ["line 1, column 7: ┐ reaches left, and nothing meets it there"]);
});

test("a line wider than the docs column is named", () => {
  const wide = `┌${"─".repeat(DIAGRAM_MAX_COLUMNS)}┐\n└${"─".repeat(DIAGRAM_MAX_COLUMNS)}┘`;
  assert.deepEqual(checkDiagram(wide), [
    `line 1 is ${DIAGRAM_MAX_COLUMNS + 2} columns wide; a diagram is at most ${DIAGRAM_MAX_COLUMNS}`,
    `line 2 is ${DIAGRAM_MAX_COLUMNS + 2} columns wide; a diagram is at most ${DIAGRAM_MAX_COLUMNS}`,
  ]);
  assert.deepEqual(checkDiagram(wide, DIAGRAM_MAX_COLUMNS + 2), []);
});

test("a character the fonts do not hold at one width is named", () => {
  assert.match(checkDiagram("── merge ─► main")[0], /^line 1, column 11: ► is not a character a diagram is drawn with/);
  assert.match(checkDiagram("─\tx")[0], /^line 1, column 2: a tab is not a character/);
  assert.match(checkDiagram("╔═╗")[0], /^line 1, column 1: ╔ is not a character/);
});

test("the words of a diagram come out in reading order, each once", () => {
  assert.deepEqual(diagramWords(GOOD), ["task", "owner", "lead", "jobs", "agents"]);
  assert.deepEqual(diagramWords("┌───┐\n│   │\n└───┘"), []);
});

test("the diagrams of a Markdown file are found by the line their fence opens on", () => {
  const md = [
    "---", "id: ADR-900", "---", "", "# T", "",
    "```text", ...GOOD.split("\n"), "```", "",
    "```bash", 'echo "│"', "```", "",
    "```", "hive status", "  → 3 running", "```", "",
    "1. A step:", "", "   ~~~", "   ┌──┐", "   │  │", "   └──", "   ~~~", "",
  ].join("\n");
  const found = findDiagrams(md);
  assert.deepEqual(found.map((d) => [d.line, d.rows, d.columns, d.problems.length]), [[7, 8, 29, 0], [29, 3, 4, 1]]);
  assert.match(found[1].problems[0], /^line 2, column 4: │ reaches down/, "the list's indentation is not part of the drawing");
  assert.deepEqual(findDiagrams("No fence here ─ only prose.\n"), []);
  assert.deepEqual(findDiagrams("```text\n┌─┐\n└─┘\n"), [{ line: 1, rows: 3, columns: 3, words: [], problems: [] }], "a fence left open runs to the end of the file");
});
