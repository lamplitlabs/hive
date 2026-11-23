// Keyboard logic for a list of rows (J/K roving focus, Enter opens, Esc
// blurs). Pure so node --test covers it; list-row.tsx wires it to the DOM.

export type RowKey = "next" | "prev" | "first" | "last" | "open" | "blur" | null;

// What a key means inside a list. Arrow keys work as well as J/K; Home/End
// jump. Anything with a modifier is left to the browser.
export function rowKey(key: string, mods: { meta?: boolean; ctrl?: boolean; alt?: boolean } = {}): RowKey {
  if (mods.meta || mods.ctrl || mods.alt) return null;
  switch (key) {
    case "j":
    case "J":
    case "ArrowDown":
      return "next";
    case "k":
    case "K":
    case "ArrowUp":
      return "prev";
    case "Home":
      return "first";
    case "End":
      return "last";
    case "Enter":
      return "open";
    case "Escape":
      return "blur";
    default:
      return null;
  }
}

// The row index that should take focus after `key` when row `i` of `n` has
// it (i = -1 when none does: J lands on the first, K on the last). Movement
// stops at the ends rather than wrapping, so a long list is not a loop.
// Returns i unchanged for keys that do not move.
export function nextIndex(key: string, i: number, n: number): number {
  if (n <= 0) return -1;
  const k = rowKey(key);
  const last = n - 1;
  switch (k) {
    case "next":
      return i < 0 ? 0 : Math.min(i + 1, last);
    case "prev":
      return i < 0 ? last : Math.max(i - 1, 0);
    case "first":
      return 0;
    case "last":
      return last;
    default:
      return i;
  }
}

// Keys a row's own Enter/Space should open on. Space is the button
// convention; Enter is both the button's and the list's.
export function opensRow(key: string): boolean {
  return key === "Enter" || key === " " || key === "Spacebar";
}

// ----- what the row's opener is called ---------------------------------------

// The words of a text as "Label in Name" (WCAG 2.5.3) compares them: what
// stands in brackets left out, every run of anything that is not a letter or
// a digit a gap between words, in lowercase. A control's spoken name has to
// hold the words it shows, side by side and in their order, so someone who
// says what they see reaches it.
export function nameWords(text: string): string[] {
  let t = (text || "").normalize("NFKD");
  for (let before = ""; before !== t; ) {
    before = t;
    t = t.replace(/\([^()]*\)/g, " ");
  }
  return t
    .toLowerCase()
    .split(/[^\p{L}\p{N}]+/u)
    .filter(Boolean);
}

// Whether `name` holds the words of `shown`, next to each other and in order.
export function holdsWords(name: string, shown: string): boolean {
  const need = nameWords(shown);
  if (!need.length) return true;
  const have = nameWords(name);
  for (let i = 0; i + need.length <= have.length; i++) if (need.every((w, k) => w === have[i + k])) return true;
  return false;
}

// The name of the control that opens a row. The control shows the row's
// title; `label` is what the caller wants it called, usually the title with
// the row's kind before it ("Decision waiting: Version the client API"). A
// label that holds the title's words is the name. One that does not gets the
// title put first, so the name always holds what the control shows. With no
// label the title names the control by itself (undefined). A title drawn from
// several nodes (`title` undefined) cannot be compared: the label stands.
export function openerName(label: string | undefined | null, title: string | undefined): string | undefined {
  const l = (label || "").trim();
  if (!l) return undefined;
  if (title === undefined || holdsWords(l, title)) return l;
  const head = title.trim().replace(/[.:;,\s]+$/, "");
  return head + (/[?!]$/.test(head) ? " " : ". ") + l;
}

// Whether a keydown that reached the list started inside a text field
// (where J/K are letters, not commands).
export function inTextField(tag: string | null | undefined, editable: boolean | string | null | undefined): boolean {
  const t = (tag || "").toLowerCase();
  if (t === "input" || t === "textarea" || t === "select") return true;
  return editable === true || editable === "true" || editable === "plaintext-only";
}
