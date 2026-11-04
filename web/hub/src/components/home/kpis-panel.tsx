"use client";

import { useMemo, useState } from "react";
import { TrendingUpIcon } from "lucide-react";
import { Area, AreaChart, Bar, BarChart, CartesianGrid, Legend, ResponsiveContainer, Tooltip as ChartTooltip, XAxis, YAxis } from "recharts";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Sparkline } from "@/components/pulse/charts";
import { EmptyState, ErrorNotice, Kpi, KpiGrid, Panel, SkeletonRows } from "@/components/pulse/panel";
import { KPI_RANGE_DAYS, KPI_RANGE_LABEL, KPI_RANGES, changeText, changeTone, costText, kpiChartRows, kpiFigure, kpiPath, kpiSummaryLine, kpiTokensFigure, kpiWindow, kpisEmpty, perDayText, tokensText, type KpiDirection, type KpiFigure, type KpiKey, type KpiRange } from "@/lib/kpis";
import { hrefs } from "@/lib/nav";
import { num } from "@/lib/format";
import type { KpisPayload } from "@/lib/api";
import { useApi } from "@/lib/use-api";

type Lens = "work" | "spend";
const LENS_LABEL: Record<Lens, string> = { work: "Work", spend: "Spend" };
const segmented = "h-8! p-0.5";
const segment = "h-7 px-2.5 text-xs";
const tooltipStyle = { background: "var(--popover)", border: "1px solid var(--border)", borderRadius: "var(--radius-lg)", padding: "0.5rem 0.75rem", color: "var(--popover-foreground)", fontSize: 12, boxShadow: "var(--sh-pop)" };
const axisTick = { fill: "var(--muted-foreground)", fontSize: 11 };
const STORAGE = "pulse.home.kpis.range";

// The series the chart draws, in a fixed colour slot each (globals.css).
const WORK_SERIES: { key: "landings" | "jobs_done" | "tasks_done" | "jobs_failed"; name: string; color: string }[] = [
  { key: "landings", name: "Landings", color: "var(--series-1)" },
  { key: "jobs_done", name: "Jobs done", color: "var(--series-2)" },
  { key: "tasks_done", name: "Tasks done", color: "var(--series-3)" },
  { key: "jobs_failed", name: "Jobs failed", color: "var(--bad)" },
];

// The cells of the stat strip: what each counts, where it leads and which
// way is good news. Cost joins only when a provider priced its calls.
const CELLS: { key: KpiKey; label: string; dir: KpiDirection; href?: string; text?: (n: number) => string; perDay?: boolean }[] = [
  { key: "landings", label: "Landings", dir: "up-good", href: hrefs.codeDeliveries(), perDay: true },
  { key: "jobs_done", label: "Jobs done", dir: "up-good", href: hrefs.harness(), perDay: true },
  { key: "tasks_done", label: "Tasks done", dir: "up-good", href: hrefs.tasks({ status: "done" }), perDay: true },
  { key: "sessions", label: "Agent runs", dir: "neutral", href: hrefs.harness(), perDay: true },
];

function initialRange(): KpiRange {
  if (typeof window === "undefined") return "7d";
  const v = window.localStorage.getItem(STORAGE);
  return (KPI_RANGES as string[]).includes(v || "") ? (v as KpiRange) : "7d";
}

function cellSub(f: KpiFigure, range: KpiRange, perDay: boolean): string {
  const change = changeText(f, range);
  return perDay && f.total > 0 ? perDayText(f.perDay) + " · " + change : change;
}

// What the fleet got done and what it spent, per day: landings, jobs, tasks
// and agent runs (the harness sessions - the hub's "Sessions" page is the
// chat) with their tokens (and money, when priced), each with
// a sparkline and a comparison with the window before, then one chart
// switchable between the work and the spend. The range is remembered.
export function KpisPanel({ project, height = 200 }: { project?: string | null; height?: number }) {
  const [range, setRangeState] = useState<KpiRange>(initialRange);
  const [lens, setLens] = useState<Lens>("work");
  // Ask for two windows so the comparison has something to compare with.
  const kpis = useApi<KpisPayload>(kpiPath(Math.min(90, KPI_RANGE_DAYS[range] * 2), project), { refresh: 60000 });
  const setRange = (r: KpiRange) => {
    setRangeState(r);
    window.localStorage.setItem(STORAGE, r);
  };
  const days = KPI_RANGE_DAYS[range];
  const p = kpis.data;
  const figures = useMemo(() => {
    const f = {} as Record<KpiKey, KpiFigure>;
    for (const k of ["landings", "jobs_done", "tasks_done", "sessions", "prompt_tokens", "completion_tokens", "cost"] as KpiKey[]) f[k] = kpiFigure(p, k, days);
    return f;
  }, [p, days]);
  const tokens = useMemo(() => kpiTokensFigure(p, days), [p, days]);
  const rows = useMemo(() => kpiChartRows(kpiWindow(p, days)), [p, days]);
  const priced = !!p?.priced;

  const controls = (
    <div className="flex min-w-0 max-w-full flex-wrap items-center justify-end gap-2">
      {p ? <span className="mr-1 text-[13px]">{kpiSummaryLine(p, range)}</span> : null}
      <Tabs value={lens} onValueChange={(v) => setLens(v as Lens)}>
        <TabsList className={segmented} aria-label="Chart what">
          {(["work", "spend"] as Lens[]).map((l) => (
            <TabsTrigger key={l} value={l} className={segment}>
              {LENS_LABEL[l]}
            </TabsTrigger>
          ))}
        </TabsList>
      </Tabs>
      <Tabs value={range} onValueChange={(v) => setRange(v as KpiRange)}>
        <TabsList className={segmented} aria-label="Over time range">
          {KPI_RANGES.map((r) => (
            <TabsTrigger key={r} value={r} className={segment} aria-label={"Last " + KPI_RANGE_LABEL[r]}>
              {r}
            </TabsTrigger>
          ))}
        </TabsList>
      </Tabs>
    </div>
  );

  if (!p) {
    return (
      <Panel title="Over time" icon={TrendingUpIcon} actions={controls}>
        {kpis.error ? <ErrorNotice error={kpis.error} onRetry={() => void kpis.reload()} /> : <SkeletonRows n={3} />}
      </Panel>
    );
  }
  if (kpisEmpty(p)) {
    return (
      <Panel title="Over time" icon={TrendingUpIcon} actions={controls}>
        <EmptyState title="Nothing measured yet" hint="Landings, finished jobs, done tasks and the tokens agents spend show up here per day once the runner has done a pass.">
          No runs, jobs or landings in the last {KPI_RANGE_LABEL[range]}.
        </EmptyState>
      </Panel>
    );
  }

  const caption = lens === "work" ? "Landings, jobs and tasks per day, last " + KPI_RANGE_LABEL[range] : (priced ? "Tokens and cost" : "Tokens") + " per day, last " + KPI_RANGE_LABEL[range];
  return (
    <Panel title="Over time" icon={TrendingUpIcon} actions={controls} contentClassName="pt-0">
      <KpiGrid className={"mb-4 rounded-xl " + (priced ? "md:grid-cols-6" : "md:grid-cols-5")}>
        {CELLS.map((c) => {
          const f = figures[c.key];
          return <Kpi key={c.key} n={c.text ? c.text(f.total) : f.total} label={c.label} href={c.href} tone={changeTone(f, c.dir)} spark={f.spark.length > 1 ? <Sparkline values={f.spark} w={72} h={20} /> : undefined} sub={cellSub(f, range, !!c.perDay)} />;
        })}
        <Kpi n={tokensText(tokens.total)} label="Tokens" href={hrefs.harness()} tone={changeTone(tokens, "up-bad")} spark={tokens.spark.length > 1 ? <Sparkline values={tokens.spark} w={72} h={20} /> : undefined} sub={tokens.total > 0 ? tokensText(figures.prompt_tokens.total) + " in · " + tokensText(figures.completion_tokens.total) + " out · " + changeText(tokens, range) : changeText(tokens, range)} />
        {priced ? <Kpi n={costText(figures.cost.total)} label="Spent" href={hrefs.harness()} tone={changeTone(figures.cost, "up-bad")} spark={figures.cost.spark.length > 1 ? <Sparkline values={figures.cost.spark} w={72} h={20} /> : undefined} sub={changeText(figures.cost, range)} /> : null}
      </KpiGrid>
      <div style={{ height }} className="min-w-0" role="img" aria-label={caption}>
        <ResponsiveContainer width="100%" height="100%">
          {lens === "work" ? (
            <BarChart data={rows} margin={{ top: 8, right: 8, left: -16, bottom: 0 }} barCategoryGap="28%">
              <CartesianGrid strokeDasharray="3 3" vertical={false} stroke="var(--border)" />
              <XAxis dataKey="label" tick={axisTick} tickLine={false} axisLine={false} interval="preserveStartEnd" minTickGap={24} />
              <YAxis tick={axisTick} tickLine={false} axisLine={false} allowDecimals={false} width={44} />
              <ChartTooltip contentStyle={tooltipStyle} cursor={{ fill: "var(--muted)", opacity: 0.5 }} formatter={(v) => num(v as number)} />
              <Legend iconType="circle" iconSize={8} formatter={(v) => <span style={{ color: "var(--muted-foreground)" }}>{v}</span>} wrapperStyle={{ fontSize: 12, overflowWrap: "anywhere", paddingTop: 4 }} />
              {WORK_SERIES.map((s, i) => (
                <Bar key={s.key} dataKey={s.key} name={s.name} stackId="a" fill={s.color} stroke="var(--card)" strokeWidth={2} maxBarSize={24} isAnimationActive={false} radius={i === WORK_SERIES.length - 1 ? [4, 4, 0, 0] : 0} />
              ))}
            </BarChart>
          ) : (
            <AreaChart data={rows} margin={{ top: 8, right: 8, left: -8, bottom: 0 }}>
              <defs>
                <linearGradient id="kpi-tokens-fill" x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stopColor="var(--series-4)" stopOpacity={0.35} />
                  <stop offset="100%" stopColor="var(--series-4)" stopOpacity={0.02} />
                </linearGradient>
              </defs>
              <CartesianGrid strokeDasharray="3 3" vertical={false} stroke="var(--border)" />
              <XAxis dataKey="label" tick={axisTick} tickLine={false} axisLine={false} interval="preserveStartEnd" minTickGap={24} />
              <YAxis yAxisId="tokens" tick={axisTick} tickLine={false} axisLine={false} width={48} tickFormatter={(v) => tokensText(v as number)} />
              {priced ? <YAxis yAxisId="cost" orientation="right" tick={axisTick} tickLine={false} axisLine={false} width={56} tickFormatter={(v) => costText(v as number)} /> : null}
              <ChartTooltip contentStyle={tooltipStyle} formatter={(v, name) => (name === "Cost" ? costText(v as number) : tokensText(v as number) + " tokens")} />
              <Legend iconType="circle" iconSize={8} formatter={(v) => <span style={{ color: "var(--muted-foreground)" }}>{v}</span>} wrapperStyle={{ fontSize: 12, overflowWrap: "anywhere", paddingTop: 4 }} />
              <Area yAxisId="tokens" type="monotone" dataKey="tokens" name="Tokens" stroke="var(--series-4)" strokeWidth={2} fill="url(#kpi-tokens-fill)" isAnimationActive={false} dot={false} activeDot={{ r: 4 }} />
              {priced ? <Area yAxisId="cost" type="monotone" dataKey="cost" name="Cost" stroke="var(--series-6)" strokeWidth={2} fill="none" isAnimationActive={false} dot={false} activeDot={{ r: 4 }} /> : null}
            </AreaChart>
          )}
        </ResponsiveContainer>
      </div>
      <div className="sr-only">
        <table>
          <caption>{caption}</caption>
        <thead>
          <tr>
            <th scope="col">Day</th>
            {lens === "work" ? WORK_SERIES.map((s) => <th key={s.key} scope="col">{s.name}</th>) : <th scope="col">Tokens</th>}
            {lens === "spend" && priced ? <th scope="col">Cost</th> : null}
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.day}>
              <th scope="row">{r.day}</th>
              {lens === "work" ? WORK_SERIES.map((s) => <td key={s.key}>{num(r[s.key])}</td>) : <td>{num(r.tokens)}</td>}
              {lens === "spend" && priced ? <td>{costText(r.cost)}</td> : null}
            </tr>
          ))}
          </tbody>
        </table>
      </div>
    </Panel>
  );
}
