// Where a task's code stands, as one line: each task gets one pull request
// (branch hive/task-<id>) that the reviewer agent reads; it merges the
// simple ones itself and hands the rest to the owner, who presses Merge.
// Older work can still carry the landing states (landed, pushed, pr, asked,
// waiting). Pure, so node --test can check it.

export type DeliveryTone = "neutral" | "ok" | "warn";

export interface DeliveryInput {
  state: string;
  waiting?: number;
  sha?: string;
  remote?: string;
  url?: string;
  number?: number;
  note?: string;
  // Why the reviewer handed the pull request to the owner (needs_owner).
  reasons?: string[];
  // The job queued to answer the reviewer's requested changes.
  follow_up?: string;
  base?: string;
  can_merge?: boolean;
}

// A line is text parts with an optional link; the component renders them.
// `canMerge` offers the Merge button: the reviewer left the pull request to
// the owner, or its review failed and someone has to decide.
export interface DeliveryView {
  before: string;
  link: { href: string; text: string } | null;
  after: string;
  tone: DeliveryTone;
  canMerge: boolean;
}

// The states whose Merge the hub accepts, for a hub older than `can_merge`.
const MERGEABLE = new Set(["needs_owner", "review_failed"]);

function prLink(d: DeliveryInput): { href: string; text: string } | null {
  const text = d.number ? "PR #" + d.number : "the pull request";
  return d.url ? { href: d.url, text } : null;
}

function prText(d: DeliveryInput): string {
  return d.number ? "PR #" + d.number : "The pull request";
}

// Notes arrive with or without their full stop; the line ends with one.
function sentence(s: string): string {
  return /[.!?…]$/.test(s) ? s : s + ".";
}

function view(before: string, link: DeliveryView["link"], after: string, tone: DeliveryTone, canMerge = false): DeliveryView {
  return { before, link, after, tone, canMerge };
}

// The pull request named once, as a link when the hub has its url, and the
// words around it; `lead` opens the sentence when the name is not its start.
function prLine(d: DeliveryInput, lead: string, after: string, tone: DeliveryTone, canMerge = false): DeliveryView {
  const link = prLink(d);
  if (link) return view(lead, link, after, tone, canMerge);
  const name = lead ? (d.number ? "PR #" + d.number : "the pull request") : prText(d);
  return view(lead + name + after, null, "", tone, canMerge);
}

export function deliveryView(d: DeliveryInput | null | undefined): DeliveryView | null {
  if (!d) return null;
  const sha = d.sha ? " " + d.sha.slice(0, 12) : "";
  const note = d.note ? ": " + d.note : ".";
  const link = prLink(d);
  const canMerge = d.can_merge ?? MERGEABLE.has(d.state);
  switch (d.state) {
    case "pr_open":
      return prLine(d, "", " open, waiting for review.", "neutral");
    case "in_review":
      return prLine(d, "", " is being reviewed.", "neutral");
    case "needs_owner": {
      const why = (d.reasons || []).filter(Boolean).join("; ");
      const tail = (why ? " \u2014 " + why : "") + (d.note ? ". " + d.note : "");
      return prLine(d, "Needs your review: ", sentence(tail), "warn", canMerge);
    }
    case "merge_asked":
      return prLine(d, "Merging ", "\u2026", "neutral");
    case "merge_waiting":
      return prLine(d, "Approved: ", d.note ? " \u2014 " + sentence(d.note.replace(/^approved;\s*/i, "")) : " merges soon.", "neutral");
    case "changes_requested": {
      const tail = (d.note ? ": " + sentence(d.note) : ".") + (d.follow_up ? " Follow-up " + d.follow_up + " is queued." : "");
      return prLine(d, "Changes requested on ", tail, "neutral");
    }
    case "review_failed":
      return prLine(d, "The review of ", " failed" + (d.note ? ": " + sentence(d.note) : "."), "warn", canMerge);
    case "merged":
      return link ? view("Merged: ", link, ".", "ok") : view("Merged" + (d.number ? " (PR #" + d.number + ")" : "") + ".", null, "", "ok");
    case "held":
      return view("Held" + note, null, "", "warn");
    // Older work, from before each task had its own pull request.
    case "waiting":
      return view(d.waiting === 1 ? "One finished step waits for its pull request." : (d.waiting || 0) + " finished steps wait for their pull request.", null, "", "neutral");
    case "landed":
      return view("Merged on this machine" + sha + ", not on GitHub yet." + (d.note ? " " + d.note : ""), null, "", "warn");
    case "asked":
      return view("A pull request was asked for.", null, "", "neutral");
    case "pushed":
      return view("Pushed to " + (d.remote || "the remote") + sha + ".", null, "", "ok");
    case "pr":
      return link ? view("On ", link, (d.remote ? " into " + d.remote : "") + ", waiting for your merge.", "ok") : view("On " + prText(d) + ", waiting for your merge.", null, "", "ok");
    default:
      return null;
  }
}
