"use client";

import { useMemo, useState } from "react";
import { ChevronDownIcon, TrendingUpIcon } from "lucide-react";
import { Area, AreaChart, Bar, BarChart, CartesianGrid, Legend, ResponsiveContainer, Tooltip as ChartTooltip, XAxis, YAxis } from "recharts";
import { Button } from "@/components/ui/button";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Sparkline } from "@/components/pulse/charts";
import { EmptyState, ErrorNotice, Kpi, KpiGrid, Panel, SkeletonRows } from "@/components/pulse/panel";
import { KPI_RANGE_DAYS, KPI_RANGE_LABEL, KPI_RANGES, changeText, changeTone, costText, kpiAutonomyLine, kpiChartRows, kpiFigure, kpiPath, kpiSummaryLine, kpiTokensFigure, kpiWindow, kpisEmpty, perDayText, tokensText, type KpiChartRow, type KpiDirection, type KpiFigure, type KpiKey, type KpiRange } from "@/lib/kpis";
import { hrefs } from "@/lib/nav";
import { num } from "@/lib/format";
import type { KpisPayload } from "@/lib/api";
import { useApi } from "@/lib/use-api";

type Lens = "work" | "spend" | "autonomy";
const LENS_LABEL: Record<Lens, string> = { work: "Work", spend: "Spend", autonomy: "Autonomy" };
const tooltipStyle = { background: "var(--popover)", border: "1px solid var(--border)", borderRadius: "var(--radius-lg)", padding: "0.5rem 0.75rem", color: "var(--popover-foreground)", fontSize: 12, boxShadow: "var(--sh-pop)" };
const axisTick = { fill: "var(--muted-foreground)", fontSize: 12 };
const STORAGE = "pulse.home.kpis.range";
const CHART_STORAGE = "pulse.home.kpis.chart";

// The series the chart draws, in a fixed colour slot each (globals.css).
type BarSeries = { key: Exclude<keyof KpiChartRow, "day" | "label" | "tokens" | "cost">; name: string; color: string };
const WORK_SERIES: BarSeries[] = [
  { key: "landings", name: "Landings", color: "var(--series-1)" },
  { key: "jobs_done", name: "Jobs done", color: "var(--series-2)" },
  { key: "tasks_done", name: "Tasks done", color: "var(--series-3)" },
  { key: "jobs_failed", name: "Jobs failed", color: "var(--bad)" },
];
// What the fleet did with no human in the loop, and the asks it did make.
const AUTONOMY_SERIES: BarSeries[] = [
  { key: "reviews", name: "Reviews", color: "var(--series-5)" },
  { key: "findings", name: "Findings", color: "var(--series-7)" },
  { key: "vuln_fixes", name: "Vulnerabilities fixed", color: "var(--series-6)" },
  { key: "owner_asks", name: "Owner asked", color: "var(--bad)" },
];
const BAR_SERIES: Record<Exclude<Lens, "spend">, BarSeries[]> = { work: WORK_SERIES, autonomy: AUTONOMY_SERIES };

// The cells of the stat strip: what each counts, where it leads and which
// way is good news. Cost joins only when a provider priced its calls.
const CELLS: { key: KpiKey; label: string; dir: KpiDirection; href?: string; text?: (n: number) => string; perDay?: boolean }[] = [
  { key: "landings", label: "Landings", dir: "up-good", href: hrefs.codeDeliveries(), perDay: true },
  { key: "jobs_done", label: "Jobs done", dir: "up-good", href: hrefs.harness(), perDay: true },
  { key: "tasks_done", label: "Tasks done", dir: "up-good", href: hrefs.tasks({ status: "done" }), perDay: true },
  { key: "sessions", label: "Agent runs", dir: "neutral", href: hrefs.harness(), perDay: true },
];

// The second strip, "On its own": what agents did with no human in the loop
// - reviews with a verdict, findings posted, vulnerabilities fixed - and,
// beside them, how often the fleet had to ask the owner (where fewer is
// better). The sub line carries a companion count from `with`: merged
// reviews under Reviews, alerts found under Vulnerabilities fixed.
const ALONE_CELLS: { key: KpiKey; label: string; dir: KpiDirection; href?: string; with?: KpiKey; sub: (n: number, companion: number) => string }[] = [
  { key: "reviews", label: "Reviews", dir: "up-good", href: hrefs.codeDeliveries(), with: "reviews_merged", sub: (n, merged) => (n > 0 ? num(merged) + " merged without the owner" : "pull requests agents reviewed") },
  { key: "findings", label: "Findings", dir: "up-good", sub: () => "bugs and evidence agents posted" },
  { key: "vuln_fixes", label: "Vulnerabilities fixed", dir: "up-good", with: "vulns_found", sub: (_n, found) => num(found) + " found" },
  { key: "owner_asks", label: "Owner asked", dir: "up-bad", sub: () => "times a human was needed" },
];
const FIGURE_KEYS: KpiKey[] = ["landings", "jobs_done", "tasks_done", "sessions", "prompt_tokens", "completion_tokens", "cost", "reviews", "reviews_merged", "findings", "vulns_found", "vuln_fixes", "owner_asks"];

function initialRange(): KpiRange {
  if (typeof window === "undefined") return "7d";
  const v = window.localStorage.getItem(STORAGE);
  return (KPI_RANGES as string[]).includes(v || "") ? (v as KpiRange) : "7d";
}

function initialChart(): boolean {
  if (typeof window === "undefined") return false;
  return window.localStorage.getItem(CHART_STORAGE) === "open";
}

function cellSub(f: KpiFigure, range: KpiRange, perDay: boolean): string {
  const change = changeText(f, range);
  return perDay && f.total > 0 ? perDayText(f.perDay) + " · " + change : change;
}

// What the fleet got done and what it spent, per day: landings, jobs, tasks
// and agent runs (the harness sessions - the hub's "Sessions" page is the
// chat) with their tokens (and money, when priced), each with
// a sparkline and a comparison with the window before; under them what it
// did on its own (reviews, findings, vulnerability fixes) against how often
// it asked the owner; then one chart switchable between the work, the
// spend and the autonomy. The range is remembered, and
// so is whether the chart is open: by default the numbers alone show and
// the chart unfolds on request, so Home stays a briefing, not a dashboard.
// `range` controlled from outside (the Insights page's own range tabs) hides
// the panel's range tabs and skips the remembered one.
export function KpisPanel({ project, height = 200, range: forced }: { project?: string | null; height?: number; range?: KpiRange }) {
  const [ownRange, setRangeState] = useState<KpiRange>(initialRange);
  const range = forced ?? ownRange;
  const [lens, setLens] = useState<Lens>("work");
  const [chart, setChartState] = useState<boolean>(initialChart);
  const setChart = (open: boolean) => {
    setChartState(open);
    window.localStorage.setItem(CHART_STORAGE, open ? "open" : "closed");
  };
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
    for (const k of FIGURE_KEYS) f[k] = kpiFigure(p, k, days);
    return f;
  }, [p, days]);
  const tokens = useMemo(() => kpiTokensFigure(p, days), [p, days]);
  const rows = useMemo(() => kpiChartRows(kpiWindow(p, days)), [p, days]);
  const priced = !!p?.priced;

  const controls = (
    <div className="flex min-w-0 max-w-full flex-wrap items-center justify-end gap-2">
      {p ? <span className="mr-1 text-[13px]">{kpiSummaryLine(p, range)}</span> : null}
      {chart ? (
        <Tabs value={lens} onValueChange={(v) => setLens(v as Lens)}>
          <TabsList variant="segmented" className="h-8!" aria-label="Chart what">
            {(["work", "spend", "autonomy"] as Lens[]).map((l) => (
              <TabsTrigger key={l} value={l} className="h-7 px-2.5 text-xs">
                {LENS_LABEL[l]}
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
      ) : null}
      {forced ? null : (
        <Tabs value={range} onValueChange={(v) => setRange(v as KpiRange)}>
          <TabsList variant="segmented" className="h-8!" aria-label="Over time range">
            {KPI_RANGES.map((r) => (
              <TabsTrigger key={r} value={r} className="h-7 px-2.5 text-xs" aria-label={"Last " + KPI_RANGE_LABEL[r]}>
                {r}
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
      )}
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
        <EmptyState title="Nothing measured yet" hint="Landings, finished jobs, done tasks, the reviews, findings and vulnerability fixes agents make on their own, and the tokens they spend show up here per day once the runner has done a pass.">
          No runs, jobs or landings in the last {KPI_RANGE_LABEL[range]}.
        </EmptyState>
      </Panel>
    );
  }

  const caption = lens === "work" ? "Landings, jobs and tasks per day, last " + KPI_RANGE_LABEL[range] : lens === "autonomy" ? "Reviews, findings, vulnerabilities fixed and times the owner was asked per day, last " + KPI_RANGE_LABEL[range] : (priced ? "Tokens and cost" : "Tokens") + " per day, last " + KPI_RANGE_LABEL[range];
  const bars = lens === "spend" ? null : BAR_SERIES[lens];
  return (
    <Panel title="Over time" icon={TrendingUpIcon} actions={controls} contentClassName="pt-0">
      <KpiGrid className={"mb-0 rounded-xl " + (priced ? "md:grid-cols-6" : "md:grid-cols-5")}>
        {CELLS.map((c) => {
          const f = figures[c.key];
          return <Kpi key={c.key} n={c.text ? c.text(f.total) : f.total} label={c.label} href={c.href} tone={changeTone(f, c.dir)} spark={f.spark.length > 1 ? <Sparkline values={f.spark} w={72} h={20} /> : undefined} sub={cellSub(f, range, !!c.perDay)} />;
        })}
        <Kpi n={tokensText(tokens.total)} label="Tokens" href={hrefs.harness()} tone={changeTone(tokens, "up-bad")} spark={tokens.spark.length > 1 ? <Sparkline values={tokens.spark} w={72} h={20} /> : undefined} sub={tokens.total > 0 ? tokensText(figures.prompt_tokens.total) + " in · " + tokensText(figures.completion_tokens.total) + " out · " + changeText(tokens, range) : changeText(tokens, range)} />
        {priced ? <Kpi n={costText(figures.cost.total)} label="Spent" href={hrefs.harness()} tone={changeTone(figures.cost, "up-bad")} spark={figures.cost.spark.length > 1 ? <Sparkline values={figures.cost.spark} w={72} h={20} /> : undefined} sub={changeText(figures.cost, range)} /> : null}
      </KpiGrid>
      <section aria-labelledby="pulse-home-kpis-alone" className="mt-4 min-w-0">
        <div className="mb-2 flex min-w-0 flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
          <h3 id="pulse-home-kpis-alone" className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">On its own</h3>
          <p className="min-w-0 text-xs text-muted-foreground anywhere">{kpiAutonomyLine(p, range)}</p>
        </div>
        <KpiGrid className="mb-0 rounded-xl">
          {ALONE_CELLS.map((c) => {
            const f = figures[c.key];
            const companion = c.with ? figures[c.with].total : 0;
            return <Kpi key={c.key} n={f.total} label={c.label} href={c.href} tone={changeTone(f, c.dir)} spark={f.spark.length > 1 ? <Sparkline values={f.spark} w={72} h={20} /> : undefined} sub={c.sub(f.total, companion) + " · " + changeText(f, range)} />;
          })}
        </KpiGrid>
      </section>
      <div className="mt-3 flex justify-end">
        <Button type="button" variant="ghost" size="sm" onClick={() => setChart(!chart)} aria-expanded={chart} aria-controls="pulse-home-kpis-chart" className="text-muted-foreground">
          {chart ? "Hide the chart" : "Show the chart"}
          <ChevronDownIcon className={"size-4 transition-transform motion-reduce:transition-none " + (chart ? "rotate-180" : "")} aria-hidden />
        </Button>
      </div>
      {chart ? (
      <div id="pulse-home-kpis-chart" style={{ height }} className="mt-1 min-w-0" role="img" aria-label={caption}>
        <ResponsiveContainer width="100%" height="100%">
          {bars ? (
            <BarChart data={rows} margin={{ top: 8, right: 8, left: -16, bottom: 0 }} barCategoryGap="28%">
              <CartesianGrid strokeDasharray="3 3" vertical={false} stroke="var(--border)" />
              <XAxis dataKey="label" tick={axisTick} tickLine={false} axisLine={false} interval="preserveStartEnd" minTickGap={24} />
              <YAxis tick={axisTick} tickLine={false} axisLine={false} allowDecimals={false} width={44} />
              <ChartTooltip contentStyle={tooltipStyle} cursor={{ fill: "var(--muted)", opacity: 0.5 }} formatter={(v) => num(v as number)} />
              <Legend iconType="circle" iconSize={8} formatter={(v) => <span style={{ color: "var(--muted-foreground)" }}>{v}</span>} wrapperStyle={{ fontSize: 12, overflowWrap: "anywhere", paddingTop: 4 }} />
              {bars.map((s, i) => (
                <Bar key={s.key} dataKey={s.key} name={s.name} stackId="a" fill={s.color} stroke="var(--card)" strokeWidth={2} maxBarSize={24} isAnimationActive={false} radius={i === bars.length - 1 ? [4, 4, 0, 0] : 0} />
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
      ) : null}
      {chart ? (
      <div className="sr-only">
        <table>
          <caption>{caption}</caption>
        <thead>
          <tr>
            <th scope="col">Day</th>
            {bars ? bars.map((s) => <th key={s.key} scope="col">{s.name}</th>) : <th scope="col">Tokens</th>}
            {lens === "spend" && priced ? <th scope="col">Cost</th> : null}
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.day}>
              <th scope="row">{r.day}</th>
              {bars ? bars.map((s) => <td key={s.key}>{num(r[s.key])}</td>) : <td>{num(r.tokens)}</td>}
              {lens === "spend" && priced ? <td>{costText(r.cost)}</td> : null}
            </tr>
          ))}
          </tbody>
        </table>
      </div>
      ) : null}
    </Panel>
  );
}
