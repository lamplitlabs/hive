// A diagram drawn in text: a fenced block of the docs whose lines are made of
// box-drawing characters. It needs no library and no picture file, so GitHub,
// this site and the console all show it as it was written, and a decision
// record kept in the store can carry one. docs/development.md says how to
// write one; this file says when a block is one and what makes it badly drawn.
// Pure, so the node tests import it as it is.

/** The widest a diagram may be: the console's reading panel shows that many characters of a code block without scrolling sideways. */
export const DIAGRAM_MAX_COLUMNS = 72;

/** Which ways each line character reaches: up, down, left, right. */
const ARMS: Readonly<Record<string, string>> = {
  "─": "lr",
  "│": "ud",
  "┌": "dr",
  "┐": "dl",
  "└": "ur",
  "┘": "ul",
  "├": "udr",
  "┤": "udl",
  "┬": "dlr",
  "┴": "ulr",
  "┼": "udlr",
};
const ARROWS = "→←↑↓";
const BOX_DRAWING = /[\u2500-\u257F]/;

/** Whether a fenced block is a diagram: plain text (no language, or `text`) that holds a box-drawing character. */
export function isDiagram(text: string, lang = ""): boolean {
  return (lang === "" || lang === "text") && BOX_DRAWING.test(text);
}

const reaches = (ch: string | undefined, way: string): boolean => ch !== undefined && (ARMS[ch] ?? "").includes(way);

/**
 * What is wrong with a diagram, one line per defect; an empty list when it is well drawn.
 *
 * - No line is wider than `max` columns.
 * - Only letters, digits and punctuation of ASCII, the eleven line characters above and the four arrows:
 *   those are the characters the fonts of the site and the console hold at one width.
 *
 * A drawing with a box in it (it holds a `┌`) is also checked for lines that do not meet. A listing of
 * folders, as `tree` prints it, has no box and is left alone: its lines start under a name on purpose.
 *
 * - A line that goes up or down meets a line, a corner or an arrowhead in the same column of the next row.
 *   A column that drifts by one is the usual defect of a diagram edited by hand, and nobody sees it in the source.
 * - A corner or a junction meets a line on each side it reaches. A plain `─` may end at a space, so a label can sit on a line.
 */
export function checkDiagram(text: string, max = DIAGRAM_MAX_COLUMNS): string[] {
  const rows = text.replace(/\n+$/, "").split("\n").map((line) => Array.from(line));
  const problems: string[] = [];
  const at = (r: number, c: number): string | undefined => rows[r]?.[c];
  const boxes = text.includes("┌");
  rows.forEach((row, r) => {
    const where = (c: number) => `line ${r + 1}, column ${c + 1}`;
    if (row.length > max) problems.push(`line ${r + 1} is ${row.length} columns wide; a diagram is at most ${max}`);
    row.forEach((ch, c) => {
      const code = ch.codePointAt(0) ?? 0;
      const plain = code >= 0x20 && code <= 0x7e;
      if (!plain && !(ch in ARMS) && !ARROWS.includes(ch)) {
        problems.push(`${where(c)}: ${ch === "\t" ? "a tab" : ch} is not a character a diagram is drawn with (ASCII, ─ │ ┌ ┐ └ ┘ ├ ┤ ┬ ┴ ┼ and → ← ↑ ↓)`);
        return;
      }
      const arms = ARMS[ch];
      if (!arms || !boxes) return;
      if (arms.includes("u") && !reaches(at(r - 1, c), "d") && at(r - 1, c) !== "↑") {
        problems.push(`${where(c)}: ${ch} reaches up, and the row above has no line or arrowhead in that column`);
      }
      if (arms.includes("d") && !reaches(at(r + 1, c), "u") && at(r + 1, c) !== "↓") {
        problems.push(`${where(c)}: ${ch} reaches down, and the row below has no line or arrowhead in that column`);
      }
      if (ch === "─") return;
      if (arms.includes("l") && !reaches(at(r, c - 1), "r") && at(r, c - 1) !== "←") {
        problems.push(`${where(c)}: ${ch} reaches left, and nothing meets it there`);
      }
      if (arms.includes("r") && !reaches(at(r, c + 1), "l") && at(r, c + 1) !== "→") {
        problems.push(`${where(c)}: ${ch} reaches right, and nothing meets it there`);
      }
    });
  });
  return problems;
}

/**
 * The words of a diagram in reading order, for the name a screen reader gets instead of the line characters:
 * each run of text between lines, arrows and wide gaps, once.
 */
export function diagramWords(text: string): string[] {
  const seen = new Set<string>();
  const words: string[] = [];
  for (const line of text.split("\n")) {
    for (const part of line.split(/[\u2500-\u257F\u2190-\u2193]+| {2,}/)) {
      const word = part.trim();
      if (word && !seen.has(word)) {
        seen.add(word);
        words.push(word);
      }
    }
  }
  return words;
}

/** One diagram of a Markdown file: the line its fence opens on (1-based), its size, its words, and what is badly drawn. */
export type FoundDiagram = { line: number; rows: number; columns: number; words: string[]; problems: string[] };

/**
 * Every diagram of a Markdown text: its fenced blocks (``` or ~~~, indented or not) that are diagrams.
 * Read line by line, with no Markdown library, so a checkout without the site's packages can still check
 * a decision record before it is filed.
 */
export function findDiagrams(markdown: string, max = DIAGRAM_MAX_COLUMNS): FoundDiagram[] {
  const found: FoundDiagram[] = [];
  const lines = markdown.replace(/^\uFEFF/, "").split(/\r?\n/);
  for (let i = 0; i < lines.length; i += 1) {
    const open = /^(\s*)(`{3,}|~{3,})\s*([^\s`]*)[^`]*$/.exec(lines[i]);
    if (!open) continue;
    const [, indent, fence, lang] = open;
    const close = new RegExp(`^\\s*${fence[0]}{${fence.length},}\\s*$`);
    const body: string[] = [];
    let j = i + 1;
    for (; j < lines.length && !close.test(lines[j]); j += 1) {
      // A block inside a list item is indented with it: that indentation is not part of the drawing.
      body.push(lines[j].startsWith(indent) ? lines[j].slice(indent.length) : lines[j].trimStart());
    }
    const text = body.join("\n");
    if (isDiagram(text, lang)) {
      found.push({
        line: i + 1,
        rows: body.length,
        columns: Math.max(0, ...body.map((row) => Array.from(row).length)),
        words: diagramWords(text),
        problems: checkDiagram(text, max),
      });
    }
    i = j;
  }
  return found;
}
