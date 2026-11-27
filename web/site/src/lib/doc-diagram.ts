// A diagram drawn in text, as a page shows it. src/lib/diagram.ts says which
// fenced block is a diagram and what makes one badly drawn; src/lib/markdown.ts
// hands the block here for its markup; src/app/docs-diagrams.css draws it.
//
// The Markdown stays text, because GitHub, the console and a record in the
// store show it as written. A page of this site draws it: the line characters
// become strokes that join, a closed rectangle becomes a box with a ground of
// its own, an arrowhead reaches what it points at, and every word stays text
// in the cell the writer put it in. Nothing here knows any one diagram: the
// drawing is read off the grid of characters, so the next diagram somebody
// writes is drawn the same way.
// Pure, so the node tests import it as it is.
import type { Element, ElementContent } from "hast";
import { ARMS, ARROWS, diagramWords } from "./diagram.ts";

/**
 * One character of the source is one cell. The words are set in the site's
 * text font at a size where the widest run of the docs still fits the cells it
 * was typed into (a test measures it), so a word never runs into the line
 * after it; most are a fifth narrower than their cells.
 */
const CELL_W = 8;
const CELL_H = 20;
export const DIAGRAM_FONT_SIZE = 13.5;
/**
 * How wide Geist sets each character from the space to the tilde, in
 * thousandths of the font size (read from public/fonts/geist-sans-latin.woff2
 * with fontTools, as scripts/subset-fonts.py reads the fonts). The drawing
 * uses it to keep a centred run inside the cells it was typed into, and a test
 * uses it to say every run of the docs fits its cells.
 */
const GEIST_ADVANCE = [
  250, 213, 346, 478, 629, 802, 620, 178, 274, 274, 430, 558, 201, 419, 201, 480, 663, 384, 619, 613, 615, 626, 593, 524, 604, 593, 297, 297, 544, 540, 544, 559,
  906, 668, 680, 703, 694, 603, 590, 700, 713, 270, 597, 640, 580, 877, 743, 739, 650, 733, 672, 640, 552, 689, 667, 945, 606, 576, 544, 347, 455, 347, 426, 557,
  248, 551, 595, 546, 595, 561, 395, 594, 581, 244, 260, 590, 267, 877, 581, 573, 595, 595, 379, 520, 392, 575, 536, 819, 585, 537, 537, 389, 264, 389, 523,
];
/** How wide a run of words is drawn, in the units of the drawing. */
export function wordsWidth(text: string): number {
  let sum = 0;
  for (const ch of text) sum += GEIST_ADVANCE[ch.charCodeAt(0) - 32] ?? 600;
  return (sum * DIAGRAM_FONT_SIZE) / 1000;
}
/** How far a corner is rounded. */
const RADIUS = 3;
/** An arrowhead: how long it is, and half its width. */
const HEAD = 6.5;
const HEAD_HALF = 3.4;
const STROKE = 1.25;
/** The room left between a word and the line that runs up to it. */
const WORD_GAP = 6;
/** The drawing may shrink to this share of its size before its box scrolls sideways: the words are then 11 px high. */
export const DIAGRAM_FLOOR = 0.825;

/** A closed rectangle of the grid, by the rows and columns its corners stand in. */
export type DiagramBox = { top: number; left: number; bottom: number; right: number };
/**
 * A run of words: where its first character stands, how many cells it takes,
 * and whether it stands inside a box. `middle` is set where the writer centred
 * the run on something, and is where that centre lies: the words are narrower
 * than their cells, so a centred run is set from its middle and not its start.
 */
export type DiagramWords = { row: number; column: number; cells: number; text: string; boxed: boolean; middle?: number };
/** The drawing of one diagram, in the units of its own grid. */
export type DiagramDrawing = {
  width: number;
  height: number;
  boxes: DiagramBox[];
  /** The sides of the boxes, as SVG path data. */
  edges: string;
  /** Every other line: what joins the boxes, and the shaft of each arrow. */
  wires: string;
  /** The arrowheads, filled. */
  heads: string;
  /** The tip of each arrowhead, for the tests: an arrow touches what it points at. */
  tips: { x: number; y: number; way: string }[];
  words: DiagramWords[];
};

const num = (n: number): string => String(Math.round(n * 10) / 10);

/** The drawing read off the characters of a diagram. */
export function drawDiagram(text: string): DiagramDrawing {
  let rows = text.split("\n").map((row) => Array.from(row.replace(/\s+$/, "")));
  while (rows.length && rows[rows.length - 1].length === 0) rows.pop();
  while (rows.length && rows[0].length === 0) rows.shift();
  // A diagram indented as a whole is drawn from its first used column, so it can be centred.
  const indent = Math.min(...rows.filter((row) => row.length).map((row) => row.findIndex((ch) => ch !== " ")), Infinity);
  if (Number.isFinite(indent) && indent > 0) rows = rows.map((row) => row.slice(indent));
  const columns = Math.max(0, ...rows.map((row) => row.length));

  const at = (r: number, c: number): string => rows[r]?.[c] ?? " ";
  const has = (r: number, c: number, way: string): boolean => (ARMS[at(r, c)] ?? "").includes(way);
  /** A word or a space: what a title set into the side of a box is made of. */
  const plain = (r: number, c: number): boolean => !(at(r, c) in ARMS) && !ARROWS.includes(at(r, c));

  // The boxes: from every character that turns down and right, the smallest rectangle whose four sides are drawn.
  const joinedAcross = (r: number, left: number, right: number): boolean => {
    if (!has(r, left, "r") || !has(r, right, "l")) return false;
    for (let c = left + 1; c < right; c += 1) if (!(has(r, c, "l") && has(r, c, "r")) && !plain(r, c)) return false;
    return has(r, left + 1, "l") && has(r, right - 1, "r");
  };
  const joinedDown = (c: number, top: number, bottom: number): boolean => {
    for (let r = top + 1; r < bottom; r += 1) if (!(has(r, c, "u") && has(r, c, "d"))) return false;
    return has(top, c, "d") && has(bottom, c, "u");
  };
  const closeBox = (top: number, left: number, right: number): DiagramBox | null => {
    for (let bottom = top + 1; bottom < rows.length; bottom += 1) {
      if (!has(bottom, left, "u")) return null;
      if (joinedAcross(bottom, left, right) && joinedDown(right, top, bottom)) return { top, left, bottom, right };
      if (!has(bottom, left, "d")) return null;
    }
    return null;
  };
  const boxes: DiagramBox[] = [];
  for (let r = 0; r < rows.length; r += 1) {
    for (let c = 0; c < columns; c += 1) {
      if (!(has(r, c, "d") && has(r, c, "r"))) continue;
      for (let right = c + 1; right < columns; right += 1) {
        if (has(r, right, "l") && has(r, right, "d") && joinedAcross(r, c, right)) {
          const box = closeBox(r, c, right);
          if (box) {
            boxes.push(box);
            break;
          }
        }
        if (!(has(r, right, "l") && has(r, right, "r")) && !plain(r, right)) break;
      }
    }
  }

  // Which joins between two neighbouring cells are the side of a box. `across[r][c]` joins (r, c) to (r, c + 1); `down[r][c]` joins (r, c) to (r + 1, c).
  const across = rows.map(() => new Set<number>());
  const down = rows.map(() => new Set<number>());
  for (const box of boxes) {
    for (let c = box.left; c < box.right; c += 1) {
      across[box.top].add(c);
      across[box.bottom].add(c);
    }
    for (let r = box.top; r < box.bottom; r += 1) {
      down[r].add(box.left);
      down[r].add(box.right);
    }
  }
  const sideOfBox = (r: number, c: number, way: string): boolean =>
    way === "r" ? across[r].has(c) : way === "l" ? across[r].has(c - 1) : way === "d" ? down[r].has(c) : (down[r - 1]?.has(c) ?? false);

  // The words: every run of characters that is neither a line nor an arrowhead, split where the writer left two spaces or more.
  const holds = (b: DiagramBox, r: number, c: number): boolean => r > b.top && r < b.bottom && c > b.left && c < b.right;
  const inBox = (r: number, c: number): boolean => boxes.some((b) => holds(b, r, c));
  const words: DiagramWords[] = [];
  rows.forEach((row, r) => {
    let c = 0;
    while (c < row.length) {
      if (!plain(r, c) || row[c] === " ") {
        c += 1;
        continue;
      }
      let end = c;
      for (let next = c; next < row.length && plain(r, next); next += 1) {
        if (row[next] !== " ") end = next;
        else if (row[next + 1] === " " || row[next + 1] === undefined) break;
      }
      words.push({ row: r, column: c, cells: end - c + 1, text: row.slice(c, end + 1).join(""), boxed: inBox(r, c) });
      c = end + 1;
    }
  });

  // What the writer centred stays centred. Lines of words that start in one column are a paragraph and keep their left edge.
  const starts = new Set(words.map((w) => `${w.row} ${w.column}`));
  const boxOf = (w: DiagramWords): DiagramBox | undefined => boxes.filter((b) => holds(b, w.row, w.column)).sort((a, b) => a.right - a.left - (b.right - b.left))[0];
  const within = (box: DiagramBox): DiagramWords[] => words.filter((o) => boxOf(o) === box);
  /** Typed with as much room before it as after it, and more than the one space that parts any word from the side of its box. */
  const typedCentred = (w: DiagramWords, box: DiagramBox): boolean => {
    const lead = w.column - box.left - 1;
    return lead >= 2 && Math.abs(lead - (box.right - (w.column + w.cells))) <= 1;
  };
  /** The boxes that stand on one another with `box`, sharing a side: the cells of a list. */
  const stackOf = (box: DiagramBox): DiagramBox[] => {
    const stack = [box];
    for (let grown = true; grown; ) {
      grown = false;
      for (const b of boxes) {
        if (stack.includes(b) || b.left !== box.left || b.right !== box.right) continue;
        if (stack.some((s) => s.bottom === b.top || s.top === b.bottom)) {
          stack.push(b);
          grown = true;
        }
      }
    }
    return stack;
  };
  const flat = (r: number, c: number): boolean => at(r, c) === "─" || at(r, c) === "→" || at(r, c) === "←";
  const upright = (r: number, c: number): boolean => has(r, c, "u") || has(r, c, "d") || at(r, c) === "↓" || at(r, c) === "↑";
  for (const w of words) {
    if (starts.has(`${w.row - 1} ${w.column}`) || starts.has(`${w.row + 1} ${w.column}`)) continue;
    const end = w.column + w.cells - 1;
    const centre = w.column + w.cells / 2;
    // A run centred on something beside its own middle stays within half a cell of the cells it was typed into
    // (a space or a line always follows them), and inside the drawing.
    const half = wordsWidth(w.text) / 2;
    const lowest = Math.max(0, (w.column - 0.5) * CELL_W) + half;
    const highest = Math.min(columns, end + 1.5) * CELL_W - half;
    const kept = (middle: number): number => Math.max(lowest, Math.min(middle, highest));
    // A label set into a line, with the line on both sides of it: `── merge ──→`.
    if (at(w.row, w.column - 1) === " " && at(w.row, w.column - 2) === "─" && at(w.row, end + 1) === " " && at(w.row, end + 2) === "─") {
      w.middle = centre * CELL_W;
      continue;
    }
    // In a box. A box that holds one run of words and nothing else is a node, and its name stands in its middle:
    // the words are narrower than the cells they were typed into, so a name typed one space from the left side
    // would hang there. Boxes that stand on one another are the cells of a list and keep the column they were
    // typed on, unless the writer centred every one of them. In a box of several rows, a run alone on its row
    // stays centred where it was typed centred.
    const box = boxOf(w);
    if (box) {
      const stack = stackOf(box);
      const mates = within(box);
      const alone = mates.filter((o) => o.row === w.row).length === 1;
      const centred =
        stack.length > 1
          ? stack.every((b) => within(b).every((o) => typedCentred(o, b)))
          : mates.length === 1 || (alone && typedCentred(w, box));
      if (centred) w.middle = ((box.left + box.right + 1) / 2) * CELL_W;
      continue;
    }
    // A label over or under a line that runs across (`  task  ` over `──────→`): centred on the line when it stands near its middle.
    let onLine = false;
    for (const r of [w.row + 1, w.row - 1]) {
      const mid = Math.floor(centre);
      if (!flat(r, mid)) continue;
      let from = mid;
      let to = mid;
      while (flat(r, from - 1)) from -= 1;
      while (flat(r, to + 1)) to += 1;
      if (from <= w.column && to >= end && Math.abs((from + to + 1) / 2 - centre) <= 1.5) {
        w.middle = kept(((from + to + 1) / 2) * CELL_W);
        onLine = true;
        break;
      }
    }
    if (onLine) continue;
    // A run that starts in the column of another run a few rows off belongs to a block of words set on that column.
    if ([2, 3].some((k) => starts.has(`${w.row - k} ${w.column}`) || starts.has(`${w.row + k} ${w.column}`))) continue;
    // Words an arrow or a line comes down to, or leaves from: centred on it when it stands near their middle.
    const joins: number[] = [];
    for (let c = w.column; c <= end; c += 1) if (upright(w.row - 1, c) || upright(w.row + 1, c)) joins.push(c);
    if (joins.length === 1 && Math.abs(joins[0] + 0.5 - centre) <= 1.5) w.middle = kept((joins[0] + 0.5) * CELL_W);
  }

  /** Where a run of words is drawn, from its first edge to its last: the words are narrower than their cells. */
  const drawn = (w: DiagramWords): [number, number] => {
    const wide = wordsWidth(w.text);
    const from = w.middle === undefined ? w.column * CELL_W : w.middle - wide / 2;
    return [from, from + wide];
  };
  const wordAt = (r: number, c: number): DiagramWords | undefined => words.find((w) => w.row === r && c >= w.column && c < w.column + w.cells);

  // Straight pieces are gathered by the line they lie on and joined end to end, so a side of a box is one stroke and not sixty.
  type Kind = "edges" | "wires";
  const pieces = new Map<string, [number, number][]>();
  const curves: Record<Kind, string[]> = { edges: [], wires: [] };
  const piece = (kind: Kind, flat: boolean, line: number, from: number, to: number) => {
    const key = `${kind} ${flat ? "h" : "v"} ${num(line)}`;
    const list = pieces.get(key) ?? [];
    list.push(from <= to ? [from, to] : [to, from]);
    pieces.set(key, list);
  };
  const heads: string[] = [];
  const tips: DiagramDrawing["tips"] = [];

  for (let r = 0; r < rows.length; r += 1) {
    for (let c = 0; c < rows[r].length; c += 1) {
      const ch = rows[r][c];
      const x = (c + 0.5) * CELL_W;
      const y = (r + 0.5) * CELL_H;
      const arms = ARMS[ch];
      if (arms) {
        const turn = arms.length === 2 && /[ud]/.test(arms) && /[lr]/.test(arms);
        // A corner is rounded: the two arms stop short of the middle of the cell and a curve joins them.
        const short = turn ? RADIUS : 0;
        const kindOf = (way: string): Kind => (sideOfBox(r, c, way) ? "edges" : "wires");
        for (const way of arms) {
          if (way === "l") piece(kindOf(way), true, y, c * CELL_W, x - short);
          else if (way === "r") piece(kindOf(way), true, y, x + short, (c + 1) * CELL_W);
          else if (way === "u") piece(kindOf(way), false, x, r * CELL_H, y - short);
          else piece(kindOf(way), false, x, y + short, (r + 1) * CELL_H);
        }
        // A line that starts or ends one space from the side of a box (`│ ──→ │`) is drawn up to that side:
        // the space is how the text keeps the two characters apart, not a gap in what is meant.
        if (ch === "─") {
          for (const dir of [-1, 1]) {
            const upright = has(r, c + 2 * dir, "u") || has(r, c + 2 * dir, "d");
            if (at(r, c + dir) === " " && upright) piece("wires", true, y, x + dir * (CELL_W / 2), (c + 2 * dir + 0.5) * CELL_W);
          }
        }
        if (turn) {
          const dy = arms.includes("u") ? -RADIUS : RADIUS;
          const dx = arms.includes("l") ? -RADIUS : RADIUS;
          const kind: Kind = [...arms].every((way) => sideOfBox(r, c, way)) ? "edges" : "wires";
          curves[kind].push(`M${num(x)} ${num(y + dy)}Q${num(x)} ${num(y)} ${num(x + dx)} ${num(y)}`);
        }
        continue;
      }
      const way = ARROWS.indexOf(ch);
      if (way < 0) continue;
      // An arrowhead: its shaft starts where the line before it ends, and its tip touches the line it points at
      // when that line is the next thing in its way (one space may lie between, as in `──→ │`). When words are
      // the next thing, up to two spaces on, the tip stops just short of them, wherever the page set them.
      if (way < 2) {
        const dir = way === 0 ? 1 : -1;
        const crossing = (cc: number): boolean => has(r, cc, "u") || has(r, cc, "d");
        const target = crossing(c + dir) ? c + dir : at(r, c + dir) === " " && crossing(c + 2 * dir) ? c + 2 * dir : null;
        const fed = has(r, c - dir, dir > 0 ? "r" : "l");
        let tip = x + dir * (CELL_W / 2 - 0.5);
        if (target !== null) {
          tip = (target + 0.5) * CELL_W - dir * (STROKE / 2);
        } else {
          let next = c + dir;
          while (at(r, next) === " " && Math.abs(next - c) <= 2) next += dir;
          const pointed = wordAt(r, next);
          if (pointed) tip = dir > 0 ? Math.max(tip, drawn(pointed)[0] - WORD_GAP) : Math.min(tip, drawn(pointed)[1] + WORD_GAP);
        }
        // An arrowhead that stands alone after words (`open posts → a job`) gets a shaft, from just after the words.
        let start = x - dir * (fed ? CELL_W / 2 : CELL_W / 2 - 1.5);
        if (!fed && at(r, c - dir) === " ") {
          const left = wordAt(r, c - 2 * dir);
          start = left ? (dir > 0 ? drawn(left)[1] + WORD_GAP : drawn(left)[0] - WORD_GAP) : x - dir * (CELL_W - 1);
        }
        if (dir * (tip - dir * (HEAD - 1) - start) > 0) piece("wires", true, y, start, tip - dir * (HEAD - 1));
        heads.push(`M${num(tip)} ${num(y)}L${num(tip - dir * HEAD)} ${num(y - HEAD_HALF)}V${num(y + HEAD_HALF)}Z`);
        tips.push({ x: tip, y, way: ch });
      } else {
        const dir = way === 3 ? 1 : -1;
        const target = has(r + dir, c, "l") || has(r + dir, c, "r") ? r + dir : null;
        const fed = has(r - dir, c, dir > 0 ? "d" : "u");
        const start = y - dir * (CELL_H / 2 - (fed ? 0 : 1.5));
        const tip = target === null ? y + dir * (CELL_H / 2 - 1.5) : (target + 0.5) * CELL_H - dir * (STROKE / 2);
        piece("wires", false, x, start, tip - dir * (HEAD - 1));
        heads.push(`M${num(x)} ${num(tip)}L${num(x - HEAD_HALF)} ${num(tip - dir * HEAD)}H${num(x + HEAD_HALF)}Z`);
        tips.push({ x, y: tip, way: ch });
      }
    }
  }

  // The words are narrower than the cells they were typed into, so a line that ran up to a word in the text
  // (`── merge ──→`, `a branch ──→`) would stop well short of it on the page: it is drawn on, up to the word.
  for (const w of words) {
    const end = w.column + w.cells - 1;
    const y = (w.row + 0.5) * CELL_H;
    const [from, to] = drawn(w);
    if (at(w.row, w.column - 1) === " " && has(w.row, w.column - 2, "r") && from - WORD_GAP > (w.column - 1) * CELL_W) {
      piece(across[w.row].has(w.column - 2) ? "edges" : "wires", true, y, (w.column - 1) * CELL_W, from - WORD_GAP);
    }
    if (at(w.row, end + 1) === " " && has(w.row, end + 2, "l") && to + WORD_GAP < (end + 2) * CELL_W) {
      piece(across[w.row].has(end + 1) ? "edges" : "wires", true, y, to + WORD_GAP, (end + 2) * CELL_W);
    }
  }

  const paths: Record<Kind, string[]> = { edges: [], wires: [] };
  for (const [key, list] of pieces) {
    const [kind, flat, line] = key.split(" ");
    list.sort((a, b) => a[0] - b[0]);
    const joined: [number, number][] = [];
    for (const [from, to] of list) {
      const last = joined[joined.length - 1];
      if (last && from <= last[1] + 0.01) last[1] = Math.max(last[1], to);
      else joined.push([from, to]);
    }
    for (const [from, to] of joined) {
      if (to - from < 0.01) continue;
      paths[kind as Kind].push(flat === "h" ? `M${num(from)} ${line}H${num(to)}` : `M${line} ${num(from)}V${num(to)}`);
    }
  }

  return {
    width: columns * CELL_W,
    height: rows.length * CELL_H,
    boxes,
    edges: [...paths.edges, ...curves.edges].join(""),
    wires: [...paths.wires, ...curves.wires].join(""),
    heads: heads.join(""),
    tips,
    words,
  };
}

const el = (tagName: string, properties: Element["properties"], children: ElementContent[] = []): Element => ({ type: "element", tagName, properties, children });

/**
 * The element that takes the place of `pre`, the <pre> of a fenced block whose
 * characters are `text`: the diagram drawn, in a box that scrolls sideways
 * where a screen is too narrow for it, and under it the block as it was
 * written, folded away, for whoever wants to copy it. A screen reader gets the
 * diagram's words, not sixty line characters.
 */
export function diagramElement(pre: Element, text: string): Element {
  const d = drawDiagram(text);
  const shapes: ElementContent[] = [];
  for (const b of d.boxes) {
    shapes.push(
      el("rect", {
        className: ["diagram-box"],
        x: num((b.left + 0.5) * CELL_W),
        y: num((b.top + 0.5) * CELL_H),
        width: num((b.right - b.left) * CELL_W),
        height: num((b.bottom - b.top) * CELL_H),
        rx: num(RADIUS),
      }),
    );
  }
  if (d.edges) shapes.push(el("path", { className: ["diagram-edge"], d: d.edges }));
  if (d.wires) shapes.push(el("path", { className: ["diagram-wire"], d: d.wires }));
  if (d.heads) shapes.push(el("path", { className: ["diagram-head"], d: d.heads }));
  for (const w of d.words) {
    const properties: Element["properties"] = { x: num(w.middle ?? w.column * CELL_W), y: num((w.row + 0.5) * CELL_H + DIAGRAM_FONT_SIZE * 0.35) };
    if (w.middle !== undefined) properties.textAnchor = "middle";
    // Words outside every box are set apart from the names in the boxes; in a diagram that has no box they are all there is.
    if (!w.boxed && d.boxes.length) properties.className = ["diagram-note"];
    // The backticks that mark a name as code in the text are not drawn: the page sets every word of a diagram alike.
    shapes.push(el("text", properties, [{ type: "text", value: w.text.replace(/`/g, "") }]));
  }
  const drawing = el(
    "svg",
    {
      className: ["diagram-drawing"],
      viewBox: `0 0 ${num(d.width)} ${num(d.height)}`,
      width: num(d.width),
      height: num(d.height),
      role: "img",
      ariaLabel: `Diagram: ${diagramWords(text).join(", ")}`,
    },
    shapes,
  );
  // The block as it was written keeps its own markup: the stylesheet closes its lines up, as it did when the block was the diagram.
  pre.properties.className = ["diagram-source"];
  return el("figure", { className: ["diagram"], style: `--diagram-w:${num(d.width)}px` }, [
    el("div", { className: ["diagram-scroll"], tabIndex: 0 }, [drawing]),
    // Shown by the stylesheet only where the drawing is wider than its box; a screen reader has the whole diagram already.
    el("p", { className: ["diagram-hint"], ariaHidden: "true" }, [{ type: "text", value: "Wider than the screen: slide it sideways to see the rest." }]),
    el("details", { className: ["diagram-text"] }, [el("summary", {}, [{ type: "text", value: "Show as text" }]), pre]),
  ]);
}
