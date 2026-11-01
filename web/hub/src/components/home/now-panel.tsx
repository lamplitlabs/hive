"use client";

// The home page's compact "Now" card: what is running right now (active
// claims, grouped per agent, with their titles) and how many jobs are queued
// or failed. It uses only data the page already holds; the logic lives in
// now-state.ts so node --test can cover it.
import { ActivityIcon } from "lucide-react";
import { Panel } from "@/components/pulse/panel";
import { ProjectLink } from "@/components/pulse/who";
import type { Org, Runner } from "@/lib/api";
import { nowCountsLine, nowView } from "./now-state";

const AGENTS_SHOWN = 4;
const ITEMS_SHOWN = 3;

export function NowPanel({ org, jobs, className }: { org: Pick<Org, "active_claims" | "projects">; jobs?: Pick<Runner["jobs"], "queued"> | null; className?: string }) {
  const v = nowView(org, jobs);
  const more = v.agents.length - AGENTS_SHOWN;
  return (
    <Panel title="Now" icon={ActivityIcon} className={className} actions={<span className="text-xs tabular-nums text-muted-foreground">{nowCountsLine(v)}</span>}>
      <section aria-label="Now" className="min-w-0 text-sm">
        <p className={v.empty ? "text-muted-foreground" : "font-medium"}>{v.headline}</p>
        {v.empty ? null : (
          <ul className="mt-2 grid gap-1.5">
            {v.agents.slice(0, AGENTS_SHOWN).map((a) => (
              <li key={a.agent} className="min-w-0">
                <span className="font-medium">{a.agent}</span>
                <span className="text-muted-foreground"> · </span>
                <span className="text-muted-foreground anywhere">
                  {a.items.slice(0, ITEMS_SHOWN).map((i, n) => (
                    <span key={i.id}>
                      {n ? "; " : null}
                      {i.title}
                      {i.slug ? (
                        <>
                          {" ("}
                          <ProjectLink slug={i.slug} className="font-normal" />
                          {")"}
                        </>
                      ) : null}
                    </span>
                  ))}
                  {a.items.length > ITEMS_SHOWN ? "; +" + (a.items.length - ITEMS_SHOWN) + " more" : null}
                </span>
              </li>
            ))}
            {more > 0 ? <li className="text-xs text-muted-foreground">and {more} more {more === 1 ? "agent" : "agents"}</li> : null}
          </ul>
        )}
        {v.failed ? (
          <p className="mt-2 text-xs text-warn anywhere">
            Baseline failed:{" "}
            {v.failedProjects.map((slug, n) => (
              <span key={slug}>
                {n ? ", " : null}
                <ProjectLink slug={slug} className="font-normal text-warn" />
              </span>
            ))}
          </p>
        ) : null}
      </section>
    </Panel>
  );
}
