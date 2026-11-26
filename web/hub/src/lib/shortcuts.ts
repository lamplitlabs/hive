// Keyboard shortcuts (spec "Shell" and "Dark mode, motion and
// accessibility"): G-chords go places, ⌘K searches, ⌘J asks, "?" lists
// them. Pure: no DOM; shortcut-help.tsx listens and renders.

export type Shortcut = { keys: string; what: string; where: string };

export const SHORTCUTS: Shortcut[] = [
  { keys: "G S", what: "Sessions", where: "Go to" },
  { keys: "G T", what: "Tasks", where: "Go to" },
  { keys: "⌘K", what: "Search", where: "Anywhere" },
  { keys: "⌘J", what: "Ask Hive", where: "Anywhere" },
  { keys: "?", what: "Keyboard shortcuts", where: "Anywhere" },
  { keys: "J / K", what: "Move down / up", where: "In a list" },
  { keys: "Enter", what: "Open", where: "In a list" },
  { keys: "Y / N", what: "Answer a decision yes / no", where: "In a list" },
  { keys: "Esc", what: "Close the panel", where: "In a panel" },
];

// The order the help dialog groups by.
export const SHORTCUT_GROUPS = ["Go to", "Anywhere", "In a list", "In a panel"] as const;

export function groupedShortcuts(list: Shortcut[] = SHORTCUTS): { where: string; items: Shortcut[] }[] {
  const out: { where: string; items: Shortcut[] }[] = [];
  const order = [...SHORTCUT_GROUPS, ...list.map((s) => s.where).filter((w) => !(SHORTCUT_GROUPS as readonly string[]).includes(w))];
  for (const where of order) {
    if (out.some((g) => g.where === where)) continue;
    const items = list.filter((s) => s.where === where);
    if (items.length) out.push({ where, items });
  }
  return out;
}

// Render "⌘K" / "G H" / "J / K" as the chips to show: ["⌘", "K"], ["G", "then", "H"], ["J", "or", "K"].
export function keyChips(keys: string): { kind: "key" | "word"; text: string }[] {
  if (keys.includes(" / ")) {
    const [a, b] = keys.split(" / ");
    return [
      { kind: "key", text: a },
      { kind: "word", text: "or" },
      { kind: "key", text: b },
    ];
  }
  if (/^G [A-Z]$/.test(keys)) {
    return [
      { kind: "key", text: "G" },
      { kind: "word", text: "then" },
      { kind: "key", text: keys.slice(2) },
    ];
  }
  if (keys.startsWith("⌘") && keys.length > 1) {
    return [
      { kind: "key", text: "⌘" },
      { kind: "key", text: keys.slice(1) },
    ];
  }
  return [{ kind: "key", text: keys }];
}

export const GO: Record<string, string> = {
  s: "/chat/",
  t: "/tasks/",
};

export type Action = { go: string } | { open: "search" | "ask" | "help" };
export type ChordResult = Action | { pending: "g" } | null;

// How long a G waits for its second key.
export const CHORD_MS = 1000;

// Read one keydown. `prev` is the pending chord key ("g") or null; `meta`
// is ⌘ (or Ctrl on other systems); `inField` says the key was typed into
// an input, textarea or contenteditable - there only ⌘K and ⌘J count, so
// typing "g h" into a message never navigates.
export function matchChord(prev: string | null, key: string, meta: boolean, inField: boolean): ChordResult {
  const k = key.length === 1 ? key.toLowerCase() : key;
  if (meta) {
    if (k === "k") return { open: "search" };
    if (k === "j") return { open: "ask" };
    return null;
  }
  if (inField) return null;
  if (prev === "g") {
    const to = GO[k];
    return to ? { go: to } : null;
  }
  if (k === "g") return { pending: "g" };
  if (key === "?") return { open: "help" };
  return null;
}

// Which modifier counts as the command key: ⌘ on a Mac, Ctrl elsewhere.
export function isCommandKey(e: { metaKey: boolean; ctrlKey: boolean }, platform: string): boolean {
  return /mac|iphone|ipad/i.test(platform) ? e.metaKey : e.ctrlKey;
}
