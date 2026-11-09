// The Records page's pure logic (spec "Records"): the kind groups, the
// contents list of a record and the filter arithmetic. No React, no DOM, so
// node --test runs it as it is.

import type { Kind } from "../pulse/kind-tile-state";

// The kinds an owner writes or decides on; every other kind is the
// framework's own documentation ("Built-in guides").
export const OWNER_KINDS: string[] = ["adr", "plan", "wu", "feature", "memory", "evolution"];

// The groups of the default list, in order. Decisions waiting on the owner
// are pinned above them in their own group (see `groupRecords`).
export interface RecordGroup {
  key: string;
  label: string;
  kind: Kind;
  kinds: string[]; // record kinds the group holds; [] = every kind not in another group
}

export const RECORD_GROUPS: RecordGroup[] = [
  { key: "adr", label: "Decisions", kind: "decision", kinds: ["adr"] },
  { key: "plan", label: "Plans", kind: "plan", kinds: ["plan"] },
  { key: "feature", label: "Features", kind: "feature", kinds: ["feature"] },
  { key: "wu", label: "Work items", kind: "work", kinds: ["wu"] },
  { key: "memory", label: "Memory", kind: "memory", kinds: ["memory"] },
  { key: "evolution", label: "Self-improvement", kind: "record", kinds: ["evolution"] },
  { key: "framework", label: "Built-in guides", kind: "guide", kinds: [] },
];

export const WAITING_GROUP = { key: "waiting", label: "Waiting on you", kind: "decision" as Kind };

export interface Groupable {
  kind: string;
  status: string;
  updated_at?: string;
  date?: string;
}

export const waitsOnYou = (r: Pick<Groupable, "kind" | "status">) => r.kind === "adr" && r.status === "Proposed";
// The name decide-record.tsx and the inbox (inbox-state.ts) know it by.
export const awaitsDecision = waitsOnYou;

const when = (r: Groupable) => r.updated_at || r.date || "";
export const byUpdated = <T extends Groupable>(a: T, b: T) => (when(a) < when(b) ? 1 : when(a) > when(b) ? -1 : 0);

// Rows into their groups, newest first inside each; empty groups are
// dropped. The waiting decisions come first in their own group and are not
// repeated under Decisions.
export function groupRecords<T extends Groupable>(rows: T[]): { group: RecordGroup | typeof WAITING_GROUP; rows: T[] }[] {
  const waiting = rows.filter(waitsOnYou).sort(byUpdated);
  const rest = rows.filter((r) => !waitsOnYou(r));
  const out: { group: RecordGroup | typeof WAITING_GROUP; rows: T[] }[] = [];
  if (waiting.length) out.push({ group: WAITING_GROUP, rows: waiting });
  const claimed = new Set(RECORD_GROUPS.flatMap((g) => g.kinds));
  for (const g of RECORD_GROUPS) {
    const mine = (g.kinds.length ? rest.filter((r) => g.kinds.includes(r.kind)) : rest.filter((r) => !claimed.has(r.kind))).sort(byUpdated);
    if (mine.length) out.push({ group: g, rows: mine });
  }
  return out;
}

// The contents of a record: its h2 and h3 headings in order, with the id
// the rendered heading gets (`slugify`) so the list can scroll to it.
// Fenced code is skipped so a `# comment` in a shell block is not a heading.
// Setext headings (underlined with === or ---) count as h1/h2.
export interface Heading {
  level: 2 | 3;
  text: string;
  id: string;
}

export function slugify(text: string): string {
  return text
    .toLowerCase()
    .replace(/[`*_~[\]()]/g, "")
    .replace(/[^\p{L}\p{N}]+/gu, "-")
    .replace(/^-+|-+$/g, "");
}

export function headings(md: string): Heading[] {
  const out: Heading[] = [];
  const seen = new Map<string, number>();
  const push = (level: number, raw: string) => {
    if (level !== 2 && level !== 3) return;
    const text = raw.replace(/\s+#+\s*$/, "").replace(/`/g, "").trim();
    if (!text) return;
    const base = slugify(text) || "section";
    const n = seen.get(base) || 0;
    seen.set(base, n + 1);
    out.push({ level, text, id: n ? base + "-" + n : base });
  };
  const lines = (md || "").split(/\r?\n/);
  let fence: string | null = null;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]!;
    const f = /^\s{0,3}(```+|~~~+)/.exec(line);
    if (f) {
      if (!fence) fence = f[1]![0]!;
      else if (f[1]![0] === fence) fence = null;
      continue;
    }
    if (fence) continue;
    const atx = /^\s{0,3}(#{1,6})\s+(.*)$/.exec(line);
    if (atx) {
      push(atx[1]!.length, atx[2]!);
      continue;
    }
    const next = lines[i + 1] || "";
    if (line.trim() && /^\s{0,3}-{3,}\s*$/.test(next) && !/^\s*[-*+]\s/.test(line)) {
      push(2, line);
      i++;
    } else if (line.trim() && /^\s{0,3}={3,}\s*$/.test(next)) {
      i++; // an h1: the panel's title already names the record
    }
  }
  return out;
}

// A record is "long" when a contents list earns its place: three headings or
// more, or a body past ~1,500 characters with any headings at all.
export function wantsContents(md: string, hs: Heading[] = headings(md)): boolean {
  return hs.length >= 3 || (hs.length > 0 && (md || "").length > 1500);
}
