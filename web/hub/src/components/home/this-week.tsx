"use client";

import Link from "next/link";
import { CalendarRangeIcon } from "lucide-react";
import { SectionHeading } from "@/components/pulse/section-heading";
import type { KpisPayload } from "@/lib/api";
import { num } from "@/lib/format";
import { kpiPath } from "@/lib/kpis";
import { hrefs } from "@/lib/nav";
import { useApi } from "@/lib/use-api";
import { weekNumbers } from "./home-state";

// "This week" (spec "Home" 7): three numbers from the last seven days -
// changes landed, tasks done, and the times the fleet asked you - with a
// link to Insights, where the same series has its charts. One small card;
// the sparklines and the other metrics left Home for Insights.
export function ThisWeek() {
  const kpis = useApi<KpisPayload>(kpiPath(7), { refresh: 60000 });
  const week = weekNumbers(kpis.data?.series);
  return (
    <section aria-labelledby="home-this-week" className="min-w-0">
      <SectionHeading id="home-this-week" icon={CalendarRangeIcon} action={<Link href={hrefs.insights({ range: "7d" })}>See insights</Link>}>
        This week
      </SectionHeading>
      <dl className="grid grid-cols-3 gap-4 rounded-lg border bg-card p-4">
        <Figure label="Landed" value={week ? week.landings : null} loading={kpis.loading && !kpis.data} />
        <Figure label="Tasks done" value={week ? week.tasksDone : null} loading={kpis.loading && !kpis.data} />
        <Figure label="Asked you" value={week ? week.decisions : null} loading={kpis.loading && !kpis.data} />
      </dl>
    </section>
  );
}

function Figure({ label, value, loading }: { label: string; value: number | null; loading: boolean }) {
  return (
    <div className="min-w-0">
      <dt className="truncate text-xs text-muted-foreground">{label}</dt>
      <dd className="tnum mt-0.5 text-2xl font-semibold leading-8 tracking-tight">{value === null ? (loading ? "…" : "—") : num(value)}</dd>
    </div>
  );
}
