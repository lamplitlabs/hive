// Pure logic for the home page's "Now" card (now-panel.tsx). It reads only
// what the page already holds - the org payload (active claims, project
// cards) and, when loaded, the runner's job counts - and turns it into what
// the card shows. Kept free of `@/` imports: the node test loader resolves
// imports relative to the importing file, so a `.tsx` cannot be tested under
// `node --test`, but this module can (now-state.test.mjs).
import type { Claim, Org, Runner } from "../../lib/api";

export interface NowItem {
  id: string;
  title: string;
  slug: string;
}

export interface NowAgent {
  agent: string;
  items: NowItem[];
}

export interface NowView {
  // Active claims: the work agents hold right now.
  running: number;
  agents: NowAgent[];
  // Jobs waiting for a slot (runner count when loaded, else open baseline
  // jobs the project cards report as queued).
  queued: number;
  // Projects whose newest baseline job failed recently.
  failed: number;
  failedProjects: string[];
  empty: boolean;
  headline: string;
}

export const NOW_EMPTY = "Nothing running";

function plural(n: number, one: string, many = one + "s"): string {
  return n + " " + (n === 1 ? one : many);
}

// Who holds the claim: the agent name, else the display name, else the runner.
export function claimAgent(c: Claim): string {
  return (c.agent || c.name || c.runner || "").trim() || "unknown agent";
}

// What the claim is about: its summary, else its thread key.
export function claimTitle(c: Claim): string {
  return (c.summary || "").trim() || (c.thread_key || "").trim() || "(untitled)";
}

export function nowView(org: Pick<Org, "active_claims" | "projects">, jobs?: Pick<Runner["jobs"], "queued"> | null): NowView {
  const claims = org.active_claims ?? [];
  const projects = org.projects ?? [];
  const byAgent = new Map<string, NowItem[]>();
  for (const c of claims) {
    const agent = claimAgent(c);
    const items = byAgent.get(agent) ?? [];
    items.push({ id: c.id || agent + "/" + items.length, title: claimTitle(c), slug: c.slug || "" });
    byAgent.set(agent, items);
  }
  const agents = [...byAgent.entries()]
    .map(([agent, items]) => ({ agent, items }))
    .sort((a, b) => b.items.length - a.items.length || a.agent.localeCompare(b.agent));
  const queued = typeof jobs?.queued === "number" ? jobs.queued : projects.filter((p) => p.baseline_job?.status === "queued").length;
  const failedProjects = projects.filter((p) => !!p.baseline_failed_at && !p.missing).map((p) => p.slug);
  const running = claims.length;
  return {
    running,
    agents,
    queued,
    failed: failedProjects.length,
    failedProjects,
    empty: running === 0,
    headline: running === 0 ? NOW_EMPTY : plural(running, "claim") + " running · " + plural(agents.length, "agent"),
  };
}

// The counts line under the headline: queued and failed, always shown.
export function nowCountsLine(v: Pick<NowView, "queued" | "failed">): string {
  return v.queued + " queued · " + v.failed + " failed";
}
