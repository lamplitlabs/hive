"use client";

import { useMemo } from "react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { CodeDeliveredKpi } from "@/components/code-delivery/report";
import { KpisPanel } from "@/components/home/kpis-panel";
import { ActivityChart } from "@/components/pulse/charts";
import { ErrorNotice, Kpi, KpiGrid, SkeletonRows } from "@/components/pulse/panel";
import { PageHeader } from "@/components/pulse/page-header";
import { ProjectAvatar } from "@/components/pulse/avatar";
import { useCrumbs } from "@/components/shell/crumbs";
import { allProjects, projectLabel, projectPath, type Org, type Timeline } from "@/lib/api";
import { num, plural } from "@/lib/format";
import { KPI_RANGE_LABEL, type KpiRange } from "@/lib/kpis";
import { hrefs } from "@/lib/nav";
import { useApi } from "@/lib/use-api";

// Insights (/insights/): the over-time view that used to close the Home
// page - the KPIs with their sparklines and chart, the at-a-glance counts
// and the posts-per-day activity - behind one range and one project filter
// kept in the address bar (hrefs.insights). The KPI panel reads the page's
// range; the activity chart has its own day/week/month switch because the
// timeline only carries 30 days and 24 hours.

// The spec's 7 / 30 / 90 days; lib/kpis.ts labels the first two already and
// the kpis route caps at 90 (KPIS_MAX_DAYS), so the 90-day window skips the
// "compared with the window before" line (there is no 180-day payload).
type Range = "7d" | "30d" | "90d";
const RANGES: Range[] = ["7d", "30d", "90d"];
const RANGE_LABEL: Record<Range, string> = { "7d": "7 days", "30d": "30 days", "90d": "90 days" };
const KPI_RANGE: Record<Range, KpiRange> = { "7d": "7d", "30d": "30d", "90d": "30d" };

export function Insights() {
  const sp = useSearchParams();
  const router = useRouter();
  const pathname = usePathname();
  const project = sp.get("project") || "";
  const rangeParam = sp.get("range") || "";
  const range: Range = (RANGES as string[]).includes(rangeParam) ? (rangeParam as Range) : "30d";

  const org = useApi<Org>("org");
  const tl = useApi<Timeline>(project ? projectPath(project, "timeline") : "timeline", { refresh: 60000 });
  const o = org.data;
  const projects = useMemo(() => allProjects(o), [o]);
  const card = project ? projects.find((p) => p.slug === project) : undefined;
  const name = card ? projectLabel(card) : project;

  useCrumbs(project ? [{ label: "Insights", href: hrefs.insights() }, { label: name }] : [{ label: "Insights" }]);

  const setParams = (next: { project?: string | null; range?: string | null }) => {
    const q = new URLSearchParams(sp.toString());
    for (const [k, v] of Object.entries(next)) {
      if (v) q.set(k, v);
      else q.delete(k);
    }
    const s = q.toString();
    router.replace(pathname + (s ? "?" + s : ""), { scroll: false });
  };

  const totals = o?.totals;
  const kpiRange = KPI_RANGE[range];
  return (
    <div>
      <PageHeader title="Insights" subtitle="How your projects are doing over time" />
      <div className="mb-6 flex min-w-0 flex-wrap items-center gap-2">
        <Tabs value={range} onValueChange={(v) => setParams({ range: v === "30d" ? null : v })} className="shrink-0">
          <TabsList variant="segmented" aria-label="Range">
            {RANGES.map((r) => (
              <TabsTrigger key={r} value={r} aria-label={"Last " + RANGE_LABEL[r]}>
                {r.replace("d", "")} days
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
        <Select value={project || "__all"} onValueChange={(v) => setParams({ project: v === "__all" ? null : v })}>
          <SelectTrigger aria-label="Project" className="w-auto max-w-56">
            <SelectValue placeholder="Every project">
              {project ? (
                <>
                  <ProjectAvatar slug={project} name={name} size={20} />
                  <span className="truncate">{name}</span>
                </>
              ) : (
                "Every project"
              )}
            </SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="__all">Every project</SelectItem>
            {projects.map((p) => (
              <SelectItem key={p.slug} value={p.slug}>
                <ProjectAvatar slug={p.slug} name={p.name} size={20} /> {projectLabel(p)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {range === "90d" ? <span className="text-xs text-muted-foreground">The KPIs compare 30-day windows; 90 days shows the longest the hub keeps.</span> : null}
      </div>

      <div className="grid min-w-0 gap-6">
        <section aria-label="At a glance">
          {org.error && !o ? (
            <ErrorNotice error={org.error} onRetry={() => void org.reload()} />
          ) : !totals ? (
            <SkeletonRows n={2} />
          ) : (
            <KpiGrid className="mb-0">
              {card ? (
                <>
                  <Kpi n={card.active_claims?.length ?? card.claims_active} label="Agents working" sub={num(card.agents) + " known on this project"} href={hrefs.project(card.slug, "work")} />
                  <Kpi n={card.suggestions_needing_votes + card.amendments_open} label="Open votes" sub="Suggestions and rule changes" href={hrefs.votes({ project: card.slug })} />
                  <CodeDeliveredKpi project={card.slug} />
                  <Kpi n={card.posts_24h} label="Posts today" sub={card.questions_open ? plural(card.questions_open, "question") + " still open" : num(card.posts) + " in all"} href={hrefs.project(card.slug, "board")} />
                </>
              ) : (
                <>
                  <Kpi n={totals.agents_active} label="Agents active" sub={num(totals.agents) + " known in all"} />
                  <Kpi n={totals.votes_open} label="Open votes" sub="Suggestions and rule changes" href={hrefs.votes()} />
                  <CodeDeliveredKpi />
                  <Kpi n={totals.posts_24h} label="Posts today" sub={totals.questions_open ? plural(totals.questions_open, "question") + " still open" : num(totals.posts) + " in all"} />
                </>
              )}
            </KpiGrid>
          )}
        </section>

        <section aria-label={"Over time, last " + KPI_RANGE_LABEL[kpiRange]}>
          <KpisPanel project={project || undefined} range={kpiRange} height={220} />
        </section>

        <section aria-label="Activity">
          {tl.error && !tl.data ? <ErrorNotice error={tl.error} onRetry={() => void tl.reload()} /> : tl.data ? <ActivityChart tl={tl.data} bys={project ? ["total", "kind"] : ["total", "project", "kind"]} height={200} title={project ? "Posts per day on " + name : "Activity"} /> : <SkeletonRows n={4} />}
        </section>
      </div>
    </div>
  );
}
