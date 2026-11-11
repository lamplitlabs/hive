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

// Whether a keydown that reached the list started inside a text field
// (where J/K are letters, not commands).
export function inTextField(tag: string | null | undefined, editable: boolean | string | null | undefined): boolean {
  const t = (tag || "").toLowerCase();
  if (t === "input" || t === "textarea" || t === "select") return true;
  return editable === true || editable === "true" || editable === "plaintext-only";
}
