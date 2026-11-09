import type { CodeDeliveriesPayload, CodeDeliverySummary } from "./api";

const n = (value: unknown) => Number(value || 0).toLocaleString("en-US");
const many = (value: number, word: string) => n(value) + " " + word + (value === 1 ? "" : "s");
export type CodeDeliveryKpiTone = "neutral" | "ok" | "warn";

export function codeDeliveryIncompleteWindow(d: CodeDeliveriesPayload | undefined): boolean {
  if (!d) return false;
  return d.scope.scanned === 0 && d.deliveries.length === 0 && d.scope.has_more && d.scope.next_before !== null;
}

// The words for one page of results: which merges it covers and how many were checked.
export function codeDeliveryWindowLabel(d: CodeDeliveriesPayload | undefined, page: number): string {
  if (!d) return "Loading the latest merges";
  const window = page > 1 ? "Older merges, page " + page : "Latest merges";
  const scope = d.scope.project ? " in " + d.scope.project : "";
  return window + scope + ": " + many(d.scope.scanned, "merge") + " checked, up to " + n(d.scope.limit) + " at a time";
}

// The line under the "Code delivered" number on Home and the project page.
export function codeDeliveryKpiSub(d: CodeDeliveriesPayload | undefined, loading: boolean, error: string): string {
  if (error && !d) return "Couldn't load";
  if (!d) return loading ? "Loading the latest merges" : "Not loaded yet";
  if (codeDeliveryIncompleteWindow(d)) return "The search stopped early; older merges are left to check";
  if (!d.scope.scanned) return "No merges yet";
  let line = many(d.summary.code_files, "source file") + " changed in " + many(d.scope.scanned, "merge") + " checked";
  const pending = codeDeliveryPending(d);
  const unknown = Math.max(d.summary.unknown, d.unavailable.length) - pending;
  if (pending) line += "; " + n(pending) + " still being read";
  if (unknown) line += "; " + n(unknown) + " couldn't be checked";
  return line;
}

const STALE = "Not refreshed, showing earlier numbers. ";

export function codeDeliveryKpiState(d: CodeDeliveriesPayload | undefined, loading: boolean, error: string): { n: number | string; sub: string; tone: CodeDeliveryKpiTone } {
  if (!d) return { n: error ? "—" : "…", sub: codeDeliveryKpiSub(d, loading, error), tone: error ? "warn" : "neutral" };
  if (codeDeliveryIncompleteWindow(d)) return { n: "—", sub: (error ? STALE : "") + codeDeliveryKpiSub(d, loading, ""), tone: "warn" };
  const sub = (error ? STALE : "") + codeDeliveryKpiSub(d, loading, "");
  return { n: d.summary.code_deliveries, sub, tone: error ? "warn" : codeDeliveryCoverageTone(d.summary) };
}

export function codeDeliveryCoverageTone(summary: CodeDeliverySummary): CodeDeliveryKpiTone {
  if (summary.unknown || summary.duplicates) return "warn";
  if (summary.code_deliveries) return "ok";
  return "neutral";
}

// How sure the count is, as plain sentences: what was checked, what was left
// out and why, and whether a zero is a real zero.
export function codeDeliveryCoverageText(d: CodeDeliveriesPayload): string {
  if (codeDeliveryIncompleteWindow(d)) return "The search stopped before it reached any merges, and there is older history left to check. This is not a real zero.";
  const s = d.summary;
  const bits = ["Checked " + many(d.scope.scanned, "recent merge") + "."];
  if (s.docs_only || s.non_code) {
    const left = [s.docs_only ? many(s.docs_only, "merge") + " that only changed docs" : "", s.non_code ? many(s.non_code, "merge") + " with no code in it" : ""].filter(Boolean);
    bits.push("Left out " + left.join(" and ") + ".");
  }
  if (s.duplicates) bits.push("Counted " + many(s.duplicates, "repeated merge") + " once.");
  const pending = Math.min(codeDeliveryPending(d), s.unknown);
  const unknown = s.unknown - pending;
  if (pending) bits.push(many(pending, "merge") + " " + (pending === 1 ? "is" : "are") + " still being read, so " + (pending === 1 ? "it isn't" : "they aren't") + " counted yet.");
  if (unknown) bits.push(many(unknown, "merge") + " couldn't be checked, so " + (unknown === 1 ? "it isn't" : "they aren't") + " counted, not even as zero.");
  if (!d.scope.scanned) bits.push("No merges were found, so there is nothing to count yet.");
  else if (!s.code_deliveries && !s.unknown && d.unavailable.length === 0) bits.push("Every merge here was checked, so zero really means none.");
  return bits.join(" ");
}

// The reason the runner gives a pull request merged on the forge whose
// changed files it has not gathered yet (pulse-codedelivery
// REASON_EVIDENCE_PENDING); it gathers them on its next passes.
export const CODE_DELIVERY_PENDING_REASON = "The merged pull request's changed files are not gathered yet";

export function codeDeliveryPending(d: CodeDeliveriesPayload | undefined): number {
  return d ? d.unavailable.filter((row) => row.reason === CODE_DELIVERY_PENDING_REASON).length : 0;
}

export type CodeDeliveryEmpty = { kind: "incomplete" | "pending" | "unknown" | "no-merges" | "no-code"; title: string; body: string; tone: CodeDeliveryKpiTone };

// What the page says when a page of results holds no code change, and why:
// a zero is only called real when every merge was read and none changed
// code, and no merges at all is said as that, not as "every merge checked".
export function codeDeliveryEmptyState(d: CodeDeliveriesPayload): CodeDeliveryEmpty | null {
  if (d.deliveries.length) return null;
  const s = d.summary;
  const where = d.scope.project ? " in " + d.scope.project : "";
  if (codeDeliveryIncompleteWindow(d)) {
    return { kind: "incomplete", title: "The search stopped early", body: "It ran out of room before it reached any merges, so this is not a real zero. Use Older to keep looking further back.", tone: "warn" };
  }
  const pending = codeDeliveryPending(d);
  if (pending) {
    return {
      kind: "pending",
      title: "Still reading the latest merges",
      body: many(pending, "merged pull request") + where + (pending === 1 ? " is" : " are") + " waiting for the runner to read which files " + (pending === 1 ? "it" : "they") + " changed. It reads a few on each pass, so the count fills in within minutes. This is not a real zero.",
      tone: "warn",
    };
  }
  if (s.unknown || d.unavailable.length) {
    return { kind: "unknown", title: "Nothing proven yet", body: "Some merges here couldn't be checked, so this is not a real zero. See the list below, or look at older merges.", tone: "warn" };
  }
  if (!d.scope.scanned) {
    return {
      kind: "no-merges",
      title: "No merges yet",
      body: "No finished work has merged" + where + " yet, so there is nothing to count. Work that agents finish shows up here once its pull request merges.",
      tone: "neutral",
    };
  }
  const left = [s.docs_only ? many(s.docs_only, "merge") + " only changed docs" : "", s.non_code ? many(s.non_code, "merge") + " changed no code" : ""].filter(Boolean);
  return {
    kind: "no-code",
    title: "No code changes here",
    body: "Every merge here was checked, and none of them changed source code, tests or styles" + (left.length ? " (" + left.join(", ") + ")" : "") + ". Finished work that changes code shows up here as it merges.",
    tone: "neutral",
  };
}

export function shortCommit(sha: string): string {
  return sha ? sha.slice(0, 12) : "unknown commit";
}

export function codeDeliveryCommitURL(remote: string | null | undefined, sha: string): string | null {
  if (!remote || !/^[a-f0-9]{40}(?:[a-f0-9]{24})?$/i.test(sha)) return null;
  let source = remote.trim();
  if (/[\s\\]/.test(source)) return null;
  const scp = /^(?:[^@/:]+@)?(github\.com|gitlab\.com|bitbucket\.org):(.+)$/i.exec(source);
  if (scp) source = "ssh://" + scp[1] + "/" + scp[2];
  let repo: URL;
  try {
    repo = new URL(source);
  } catch (error) {
    if (error instanceof TypeError) return null;
    throw error;
  }
  if (!["https:", "http:", "ssh:", "git:"].includes(repo.protocol) || repo.port) return null;
  const parts = repo.pathname.replace(/\/+$/, "").replace(/\.git$/i, "").slice(1).split("/");
  if (parts.some((part) => !/^[a-z0-9_.-]+$/i.test(part) || /^\.+$/.test(part))) return null;
  let route: string;
  switch (repo.hostname.toLowerCase()) {
    case "github.com":
      if (parts.length !== 2) return null;
      route = "commit";
      break;
    case "gitlab.com":
      if (parts.length < 2) return null;
      route = "-/commit";
      break;
    case "bitbucket.org":
      if (parts.length !== 2) return null;
      route = "commits";
      break;
    default:
      return null;
  }
  return "https://" + repo.hostname.toLowerCase() + "/" + parts.join("/") + "/" + route + "/" + sha.toLowerCase();
}
