// The drawing of a diagram: what the page makes of a block of line characters.
// The Markdown stays text; these say the drawing is read off it truly: every
// box found, every arrowhead on what it points at, every word kept.
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { toString } from "hast-util-to-string";

const { drawDiagram, diagramElement, wordsWidth, DIAGRAM_FLOOR, DIAGRAM_FONT_SIZE } = await import("./doc-diagram.ts");

const FLOW = [
  "┌────────┐  task   ┌────────┐",
  "│ owner  │ ──────→ │ lead   │",
  "└────────┘         └───┬────┘",
  "                       ↓",
  "                 ┌───────────┐",
  "                 │ a job     │",
  "                 └───────────┘",
].join("\n");

/** Whether a piece of path data draws along `y` from `from` to `to` or further. */
const runsAcross = (d, y, from, to) =>
  [...d.matchAll(/M([\d.]+) ([\d.]+)H([\d.]+)/g)].some(([, x1, yy, x2]) => Number(yy) === y && Number(x1) <= from + 0.01 && Number(x2) >= to - 0.01);

test("a closed rectangle is a box, and its sides are drawn apart from what joins the boxes", () => {
  const d = drawDiagram(FLOW);
  assert.deepEqual(d.boxes, [
    { top: 0, left: 0, bottom: 2, right: 9 },
    { top: 0, left: 19, bottom: 2, right: 28 },
    { top: 4, left: 17, bottom: 6, right: 29 },
  ]);
  assert.equal(d.width, 30 * 8, "one character is one cell, 8 wide");
  assert.equal(d.height, 7 * 20);
  // The top of the first box is one stroke between its two rounded corners, not eight dashes.
  assert.ok(runsAcross(d.edges, 10, 7, 73), d.edges);
  assert.equal((d.edges.match(/Q/g) ?? []).length, 12, "four rounded corners a box");
  assert.equal(d.wires.includes("Q"), false, "nothing that joins these boxes turns a corner");
});

test("an arrowhead touches the line it points at, and its shaft starts on the box it leaves", () => {
  const d = drawDiagram(FLOW);
  assert.deepEqual(d.tips.map((t) => t.way), ["→", "↓"]);
  // `──────→ │`: one space lies between the head and the side of the box, and the tip crosses it.
  const [right, down] = d.tips;
  assert.ok(Math.abs(right.x - (19.5 * 8 - 0.625)) < 0.01, `the tip stands on the side of the lead's box: ${right.x}`);
  assert.equal(right.y, 30);
  // `│ ──────→`: one space lies between the side of the owner's box and the shaft, and the shaft crosses it.
  assert.ok(runsAcross(d.wires, 30, 9.5 * 8, right.x - 6), d.wires);
  // The arrow that points down ends on the top of the box below it.
  assert.ok(Math.abs(down.y - (4.5 * 20 - 0.625)) < 0.01, `the tip stands on the top of the job's box: ${down.y}`);
  assert.equal(down.x, 23.5 * 8);
});

test("a line that runs up to a word is drawn up to the word, which is narrower than its cells", () => {
  // In the text one space parts the line from the word; on the page the word is set narrower, and the line follows it.
  const d = drawDiagram("│ a │ ── off ──→ │ b │");
  const off = d.words.find((w) => w.text === "off");
  const half = wordsWidth("off") / 2;
  assert.ok(half * 2 < 3 * 8, "the word is narrower than its three cells");
  // The path data is written to a tenth of a unit, so the ends of the strokes are asked for to that.
  assert.ok(runsAcross(d.wires, 10, 6 * 8, off.middle - half - 6.1), `the line before the word reaches it: ${d.wires}`);
  assert.ok(runsAcross(d.wires, 10, off.middle + half + 6.1, 13 * 8 - 6), `and so does the line after it: ${d.wires}`);
  assert.equal(runsAcross(d.wires, 10, off.middle - half, off.middle + half), false, "no line runs through the word");
});

test("an arrowhead that points at words stops just short of them; with nothing after it, it stays in its own cell", () => {
  // The word starts in its own cell, two cells on: the tip stops 6 before it.
  assert.equal(drawDiagram("a ──→ b").tips[0].x, 6 * 8 - 6);
  const alone = drawDiagram("a ──→\n│\n↓\nc");
  const [right, down] = alone.tips;
  assert.ok(right.x > 4 * 8 && right.x < 5 * 8, String(right.x));
  assert.ok(down.y > 2 * 20 && down.y < 3 * 20, String(down.y));
  assert.deepEqual(alone.boxes, []);
  // Words are narrower than their cells, so an arrow that points back at them is drawn on, up to where they end.
  const back = drawDiagram("a card for a change  ←── vote");
  const end = wordsWidth("a card for a change");
  assert.ok(end < 19 * 8 - 20, "the words end well before their cells do");
  assert.ok(Math.abs(back.tips[0].x - (end + 6)) < 0.01, `the tip stands just after the words: ${back.tips[0].x}`);
  assert.ok(runsAcross(back.wires, 10, back.tips[0].x + 6, 24 * 8), back.wires);
});

test("an arrowhead that stands alone after words gets a shaft, from just after the words", () => {
  const d = drawDiagram("open shared posts → one job");
  const end = wordsWidth("open shared posts");
  const tip = d.tips[0].x;
  assert.equal(tip, 20 * 8 - 6, "it stops short of the words it points at");
  assert.ok(runsAcross(d.wires, 10, end + 6.1, tip - 6), d.wires);
  assert.equal(runsAcross(d.wires, 10, end, tip - 6), false, "and does not touch the words it leaves");
});

test("every word is kept, in the cell it was typed into", () => {
  const d = drawDiagram(FLOW);
  assert.deepEqual(d.words.map((w) => [w.text, w.row, w.column, w.cells, w.boxed]), [
    ["task", 0, 12, 4, false],
    ["owner", 1, 2, 5, true],
    ["lead", 1, 21, 4, true],
    ["a job", 5, 19, 5, true],
  ]);
  // Two spaces or more part two runs of words; one space does not.
  assert.deepEqual(drawDiagram("│ one two   three │").words.map((w) => [w.text, w.column]), [["one two", 2], ["three", 12]]);
});

test("what the writer centred is set from its middle; a paragraph keeps its left edge", () => {
  // A label set into a line is centred between the two ends of the line.
  const label = drawDiagram("│ a │ ── merge ──→ │ b │").words.find((w) => w.text === "merge");
  assert.equal(label.middle, (9 + 5 / 2) * 8);
  // Words an arrow points at are where the arrow ends, not a label on it.
  assert.equal(drawDiagram("└─→ a card ────→ a job").words[0].middle, undefined);
  // The one run of a box row, with as much room before as after, is centred in the box.
  const boxed = drawDiagram(["┌──────────┐", "│   done   │", "└──────────┘"].join("\n")).words[0];
  assert.equal(boxed.middle, 6 * 8, "the middle of a box whose sides stand in columns 0 and 11");
  // A box drawn close round its words holds them in its middle too: the words are narrower than the cells.
  assert.equal(drawDiagram(["┌───────┐", "│ owner │", "└───────┘"].join("\n")).words[0].middle, 4.5 * 8);
  // A stack of boxes whose words start in one column is a list: the entry that fills its box stays on the column.
  const list = drawDiagram(["┌────────────────┐", "│ GitHub         │", "├────────────────┤", "│ model provider │", "└────────────────┘"].join("\n")).words;
  assert.deepEqual(list.map((w) => w.middle), [undefined, undefined]);
  // A label over a line is centred on the line.
  assert.equal(drawDiagram("│ a │  task   │ b │\n│ a │ ──────→ │ b │").words.find((w) => w.text === "task").middle, 9.5 * 8);
  // Words an arrow comes down to are centred on the arrow, as far as the cells they were typed into allow.
  const under = drawDiagram(["  │", "  ↓", "a job"].join("\n")).words[0];
  assert.equal(under.middle, 2.5 * 8);
  // "merge" fills its five cells: it moves towards the arrow by half a cell and no further.
  const aside = drawDiagram(["   │", "   ↓", "merge"].join("\n")).words[0];
  assert.ok(aside.middle > 2.5 * 8 && aside.middle < 3.5 * 8 && aside.middle + wordsWidth("merge") / 2 <= 5.5 * 8 + 0.01, `pulled towards the arrow, and kept by its own cells: ${aside.middle}`);
  // Far from the middle of the words, the arrow is where the words start and not what they are centred on.
  assert.equal(drawDiagram([" │", " ↓", "a long line of words"].join("\n")).words[0].middle, undefined);
  // Lines that start in one column are a paragraph: none of them is moved.
  const para = drawDiagram(["┌────────────────┐", "│   one line     │", "│   and another  │", "└────────────────┘"].join("\n")).words;
  assert.deepEqual(para.map((w) => w.middle), [undefined, undefined]);
  // A box that holds one run of words is a node: its name stands in its middle, wherever it was typed.
  assert.equal(drawDiagram(["┌──────────┐", "│ done     │", "└──────────┘"].join("\n")).words[0].middle, 6 * 8);
  // In a box of several runs, words at the start of a row are left where they are.
  assert.deepEqual(drawDiagram(["┌──────────┐", "│ done     │", "│   or not │", "└──────────┘"].join("\n")).words.map((w) => w.middle), [undefined, undefined]);
});

test("a title set into the side of a box does not open the box", () => {
  const d = drawDiagram(["┌─ a thread ───┐", "│ claim        │", "└──────┬───────┘", "       ↓"].join("\n"));
  assert.deepEqual(d.boxes, [{ top: 0, left: 0, bottom: 2, right: 15 }]);
  assert.deepEqual(d.words.map((w) => [w.text, w.boxed]), [["a thread", false], ["claim", true]]);
  // The side is drawn on both sides of the title, up to it, and not through it.
  const title = d.words[0];
  assert.equal(runsAcross(d.edges, 10, title.middle - 8, title.middle + 8), false, d.edges);
  // The path data is written to a tenth of a unit, so the end of the stroke is asked for to that.
  assert.ok(runsAcross(d.edges, 10, 7, title.middle - wordsWidth("a thread") / 2 - 6.1), d.edges);
});

test("a diagram indented as a whole is drawn from its first used column", () => {
  const plain = drawDiagram("┌───┐\n│ a │\n└───┘");
  const indented = drawDiagram("    ┌───┐\n    │ a │\n    └───┘\n");
  assert.deepEqual(indented, plain);
});

test("the stylesheet sets the words at the size the drawing was measured for, and shrinks it no further than the floor", () => {
  // The drawing places lines by how wide the words are at DIAGRAM_FONT_SIZE, and the box scrolls once the drawing
  // would be smaller than DIAGRAM_FLOOR: both numbers are written out in the stylesheet, so they are held together here.
  const css = fs.readFileSync(new URL("../app/docs-diagrams.css", import.meta.url), "utf8");
  assert.ok(css.includes(`font-size: ${DIAGRAM_FONT_SIZE}px;`), "the size of the words");
  assert.equal(css.split(`var(--diagram-w) * ${DIAGRAM_FLOOR}`).length - 1, 2, "the floor, for the drawing's least width and for the line that says it scrolls");
});

test("the page gets a figure: the drawing named for a screen reader, and the block as written under it", () => {
  const pre = { type: "element", tagName: "pre", properties: { tabIndex: 0 }, children: [{ type: "element", tagName: "code", properties: {}, children: [{ type: "text", value: `${FLOW}\n` }] }] };
  const figure = diagramElement(pre, FLOW);
  assert.equal(figure.tagName, "figure");
  assert.deepEqual(figure.properties.className, ["diagram"]);
  assert.equal(figure.properties.style, "--diagram-w:240px", "the stylesheet reads the drawing's own width from here");
  const [scroll, hint, details] = figure.children;
  assert.equal(scroll.properties.tabIndex, 0, "a keyboard can scroll a drawing that is wider than its box");
  const svg = scroll.children[0];
  assert.equal(svg.tagName, "svg");
  assert.equal(svg.properties.viewBox, "0 0 240 140");
  assert.equal(svg.properties.role, "img");
  assert.equal(svg.properties.ariaLabel, "Diagram: task, owner, lead, a job");
  const texts = svg.children.filter((c) => c.tagName === "text");
  assert.deepEqual(texts.map((t) => toString(t)), ["task", "owner", "lead", "a job"]);
  assert.deepEqual(texts.map((t) => [t.properties.x, t.properties.textAnchor ?? "start"]), [["116", "middle"], ["40", "middle"], ["192", "middle"], ["188", "middle"]], "a label over a line is set from the line's middle, the name of a box from the box's");
  assert.deepEqual(texts.map((t) => t.properties.className ?? null), [["diagram-note"], null, null, null], "words outside a box are set apart");
  assert.equal(svg.children.filter((c) => c.tagName === "rect").length, 3);
  assert.equal(hint.properties.ariaHidden, "true");
  assert.equal(details.tagName, "details");
  assert.equal(details.children[1], pre, "the block as it was written is kept whole");
  assert.deepEqual(pre.properties.className, ["diagram-source"]);
  assert.equal(toString(pre), `${FLOW}\n`);
  assert.ok(DIAGRAM_FONT_SIZE * DIAGRAM_FLOOR >= 11, "the words stay 11 px or more when the drawing shrinks");
});
