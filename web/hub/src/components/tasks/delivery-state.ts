// Where a task's code stands, as one line: each task gets one pull request
// (branch pulse/task-<id>); the daemon polls it, a merge closes the task.
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
}

// A line is text parts with an optional link; the component renders them.
export interface DeliveryView {
  before: string;
  link: { href: string; text: string } | null;
  after: string;
  tone: DeliveryTone;
}

function prLink(d: DeliveryInput): { href: string; text: string } | null {
  const text = d.number ? "PR #" + d.number : "the pull request";
  return d.url ? { href: d.url, text } : null;
}

function prText(d: DeliveryInput): string {
  return d.number ? "PR #" + d.number : "The pull request";
}

export function deliveryView(d: DeliveryInput | null | undefined): DeliveryView | null {
  if (!d) return null;
  const sha = d.sha ? " " + d.sha.slice(0, 12) : "";
  const note = d.note ? ": " + d.note : ".";
  const link = prLink(d);
  switch (d.state) {
    case "pr_open":
      return link ? { before: "", link, after: " open, waiting for your merge.", tone: "ok" } : { before: prText(d) + " open, waiting for your merge.", link: null, after: "", tone: "ok" };
    case "merged":
      return link ? { before: "Merged: ", link, after: ".", tone: "ok" } : { before: "Merged" + (d.number ? " (PR #" + d.number + ")" : "") + ".", link: null, after: "", tone: "ok" };
    case "held":
      return { before: "Held" + note, link: null, after: "", tone: "warn" };
    // Older work, from before each task had its own pull request.
    case "waiting":
      return { before: d.waiting === 1 ? "One finished step waits for its pull request." : (d.waiting || 0) + " finished steps wait for their pull request.", link: null, after: "", tone: "neutral" };
    case "landed":
      return { before: "Merged on this machine" + sha + ", not on GitHub yet." + (d.note ? " " + d.note : ""), link: null, after: "", tone: "warn" };
    case "asked":
      return { before: "A pull request was asked for.", link: null, after: "", tone: "neutral" };
    case "pushed":
      return { before: "Pushed to " + (d.remote || "the remote") + sha + ".", link: null, after: "", tone: "ok" };
    case "pr":
      return link ? { before: "On ", link, after: (d.remote ? " into " + d.remote : "") + ", waiting for your merge.", tone: "ok" } : { before: "On " + prText(d) + ", waiting for your merge.", link: null, after: "", tone: "ok" };
    default:
      return null;
  }
}
