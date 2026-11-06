// The Votes page's pure logic (spec "Votes"): the split bar, the status
// filter's words and the sort orders. No React, no DOM, so node --test runs
// it as it is.

// The URL keeps the store's words (eligible, needs votes, rejected).
export type StatusFilter = "" | "eligible" | "needs votes" | "rejected";
export type Sort = "queue" | "quorum" | "expiry" | "newest";
export type VoteKind = "suggestions" | "amendments";

export const STATUSES: { value: StatusFilter; label: string }[] = [
  { value: "", label: "All" },
  { value: "eligible", label: "Agreed" },
  { value: "needs votes", label: "Needs votes" },
  { value: "rejected", label: "Turned down" },
];

export const SORTS: { value: Sort; label: string }[] = [
  { value: "queue", label: "Next in line first" },
  { value: "quorum", label: "Closest to enough votes" },
  { value: "expiry", label: "Closing soonest" },
  { value: "newest", label: "Newest first" },
];

export const validKind = (v: string | null): VoteKind => (v === "amendments" ? "amendments" : "suggestions");
export const validStatus = (v: string | null): StatusFilter => (v === "eligible" || v === "needs votes" || v === "rejected" ? v : "");
export const validSort = (v: string | null): Sort => (v === "quorum" || v === "expiry" || v === "newest" ? v : "queue");
export const statusKey = (s: StatusFilter) => s || "all";

// "No suggestion needs votes", "No rule change was turned down".
export function noneText(status: StatusFilter, what: string): string {
  if (status === "eligible") return "No " + what + " is agreed";
  if (status === "needs votes") return "No " + what + " needs votes";
  if (status === "rejected") return "No " + what + " was turned down";
  return "No " + what;
}

// The split bar: votes for in green, votes against in red, and a tick where
// "enough votes" (the quorum) sits. The bar's length is the larger of the
// votes cast and the quorum, so a vote short of quorum leaves a gap before
// the tick and one past it fills the whole bar. Percentages, 0-100, one
// decimal at most; `tick` is null when no quorum applies.
export interface SplitBar {
  for: number;
  against: number;
  tick: number | null;
  total: number;
  enough: boolean;
  label: string;
}

const pct = (n: number, of: number) => (of > 0 ? Math.round((1000 * n) / of) / 10 : 0);

export function splitBar(votesFor: number, votesAgainst: number, needed?: number | null): SplitBar {
  const up = Math.max(0, Math.floor(votesFor || 0));
  const down = Math.max(0, Math.floor(votesAgainst || 0));
  const need = needed && needed > 0 ? Math.floor(needed) : 0;
  const cast = up + down;
  const total = Math.max(cast, need, 1);
  const label = up + " for, " + down + " against" + (need ? " (" + need + " needed)" : "");
  return {
    for: pct(up, total),
    against: pct(down, total),
    tick: need ? pct(need, total) : null,
    total,
    enough: need ? cast >= need : cast > 0,
    label,
  };
}

// The sort orders over the fields a suggestion has; `expires` and `rank`
// come from the caller so this stays free of date parsing and types.
export interface Sortable {
  date: string;
  score: number;
  voters: number;
  quorum?: number;
  expires: number | null; // ms since epoch, or null when it never closes
}

export function sortRows<T extends Sortable>(rows: T[], sort: Sort): T[] {
  if (sort === "queue") return rows;
  const sorted = [...rows];
  if (sort === "quorum") sorted.sort((a, b) => Math.max(0, (a.quorum ?? 0) - a.voters) - Math.max(0, (b.quorum ?? 0) - b.voters) || b.score - a.score);
  else if (sort === "expiry") sorted.sort((a, b) => (a.expires ?? Infinity) - (b.expires ?? Infinity));
  else sorted.sort((a, b) => (a.date < b.date ? 1 : a.date > b.date ? -1 : 0));
  return sorted;
}

// A vote's side, from the words the board uses.
export const isFor = (vote: string | undefined | null): boolean => {
  const v = (vote || "").toLowerCase();
  return v === "up" || v === "yes" || v === "+1" || v === "for";
};
