"use client";

import Link from "next/link";
import { useEffect, useRef, useState } from "react";
import { AlertTriangleIcon, CheckCircle2Icon, ChevronRightIcon, InfoIcon, OctagonAlertIcon, RefreshCwIcon, XIcon } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { num } from "@/lib/format";
import type { Tone } from "@/lib/format";
import { PulseMascot } from "./mascots";
import { PageHeader } from "./page-header";

// The top of every page: what it is (the title), what it is for (`sub`, one
// line), and the one action it exists for on the right. `bar` holds the
// page's tabs or filters, right under the title. `icon` is the same glyph
// as the page's place in the navigation. Older signature kept; it renders
// through PageHeader (page-header.tsx), the redesign's page header.
export function PageTitle({ children, sub, crumb, actions, bar, className, icon: Icon, badge }: { children: React.ReactNode; sub?: React.ReactNode; crumb?: React.ReactNode; actions?: React.ReactNode; bar?: React.ReactNode; className?: string; icon?: React.ComponentType<{ className?: string; strokeWidth?: number }>; badge?: React.ReactNode }) {
  return (
    <>
      {crumb ? <div className="mt-5 -mb-3 min-w-0 text-sm text-muted-foreground anywhere sm:mt-6 sm:-mb-4">{crumb}</div> : null}
      <PageHeader
        title={children}
        subtitle={sub}
        action={actions}
        tabs={bar ? <FadingStrip className="min-w-0 overflow-x-auto pb-0.5 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">{bar}</FadingStrip> : undefined}
        badge={badge}
        lead={Icon ? <Icon className="size-5" strokeWidth={1.75} /> : undefined}
        className={className}
      />
    </>
  );
}

// A row that scrolls sideways without a scrollbar (tabs and filters on a
// phone) fades at the edge that has more to show, so a cut-off "Self-impr…"
// reads as "there is more" rather than as the end of the row. No fade when
// everything fits.
export function FadingStrip({ children, className }: { children: React.ReactNode; className?: string }) {
  const ref = useRef<HTMLDivElement>(null);
  const [edges, setEdges] = useState({ left: false, right: false });
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => {
      const max = el.scrollWidth - el.clientWidth;
      const left = el.scrollLeft > 2;
      const right = max - el.scrollLeft > 2;
      setEdges((e) => (e.left === left && e.right === right ? e : { left, right }));
    };
    const frame = requestAnimationFrame(measure);
    el.addEventListener("scroll", measure, { passive: true });
    const ro = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    ro?.observe(el);
    if (el.firstElementChild) ro?.observe(el.firstElementChild);
    return () => {
      cancelAnimationFrame(frame);
      el.removeEventListener("scroll", measure);
      ro?.disconnect();
    };
  }, []);
  const mask = edges.left && edges.right ? "linear-gradient(to right, transparent, #000 2rem, #000 calc(100% - 2rem), transparent)" : edges.left ? "linear-gradient(to right, transparent, #000 2rem)" : edges.right ? "linear-gradient(to right, #000 calc(100% - 2rem), transparent)" : undefined;
  return (
    <div ref={ref} className={className} style={mask ? { maskImage: mask, WebkitMaskImage: mask } : undefined}>
      {children}
    </div>
  );
}

export function Lead({ children, className }: { children: React.ReactNode; className?: string }) {
  return <p className={cn("mb-4 max-w-3xl text-sm leading-relaxed text-muted-foreground anywhere", className)}>{children}</p>;
}

// A titled block of related things. The title says what is inside; actions
// sit on the right of the title row; `footer` carries a quiet note.
export function Panel({ title, actions, children, className, contentClassName, dense, icon: Icon, footer }: { title?: React.ReactNode; actions?: React.ReactNode; children: React.ReactNode; className?: string; contentClassName?: string; dense?: boolean; icon?: React.ComponentType<{ className?: string }>; footer?: React.ReactNode }) {
  return (
    <Card className={cn("enter min-w-0 gap-0 rounded-lg py-0", className)}>
      {title !== undefined ? (
        <CardHeader className="flex flex-row flex-wrap items-center justify-between gap-x-4 gap-y-2 border-b border-border/70 px-4 py-3 [.border-b]:pb-3 sm:px-5">
          <div className="flex min-w-0 flex-1 basis-36 items-center gap-2">
            {Icon ? <Icon className="size-4 shrink-0 text-muted-foreground" aria-hidden /> : null}
            <CardTitle className="min-w-0 text-base font-semibold leading-snug tracking-tight anywhere">{title}</CardTitle>
          </div>
          {actions ? <div className="ml-auto flex min-w-0 max-w-full flex-wrap items-center justify-end gap-2 text-[13px] leading-relaxed text-muted-foreground anywhere">{actions}</div> : null}
        </CardHeader>
      ) : null}
      <CardContent className={cn("min-w-0", dense ? "p-0" : "px-4 py-4 sm:px-5", contentClassName)}>{children}</CardContent>
      {footer ? <div className="border-t border-border/70 bg-muted/40 px-4 py-3 text-[13px] leading-relaxed text-muted-foreground anywhere sm:px-5">{footer}</div> : null}
    </Card>
  );
}

// An empty list is an invitation (spec "Components" - Empty state): a small
// illustration, a one-line title, one primary action and an optional second.
// Pip stands in it, curious, unless the caller brings its own `icon`
// (a Lucide glyph or any node). `children` is the one sentence under the
// title; `hint` is a command to copy.
export function EmptyState({ children, hint, icon, className, title, action, secondary, compact }: { children?: React.ReactNode; hint?: string; icon?: React.ReactNode; className?: string; title?: React.ReactNode; action?: React.ReactNode; secondary?: React.ReactNode; compact?: boolean }) {
  return (
    <div className={cn("flex min-w-0 flex-col items-center gap-1.5 rounded-lg border border-dashed border-[color-mix(in_oklch,var(--foreground)_16%,var(--border))] bg-card/60 px-5 text-center", compact ? "py-5" : "py-8", className)}>
      {icon ? (
        <span className="mb-1.5 flex size-10 items-center justify-center rounded-lg bg-muted text-muted-foreground [&>svg]:size-5" aria-hidden>
          {icon}
        </span>
      ) : (
        <PulseMascot mood="curious" size={compact ? 40 : 56} decorative className="float mb-1" />
      )}
      {title ? <div className="max-w-md truncate text-base font-semibold leading-6 text-foreground" title={typeof title === "string" ? title : undefined}>{title}</div> : null}
      {children ? <div className="max-w-md text-sm leading-5 text-muted-foreground anywhere">{children}</div> : null}
      {hint ? <code className="mt-1 max-w-full rounded-md bg-muted px-2 py-1 font-mono text-xs text-foreground anywhere">{hint}</code> : null}
      {action || secondary ? (
        <div className="mt-3 flex flex-wrap items-center justify-center gap-2">
          {action}
          {secondary}
        </div>
      ) : null}
    </div>
  );
}

// Something failed to load or to run (spec "Components" - Error): an inline
// card saying what failed, why in one line, a Retry button, and the
// technical detail folded away. `title` is the what ("The tasks didn't
// load"), `why` the one-line reason, `detail` the raw error.
export function ErrorCard({ title, why, onRetry, detail, className, retryLabel = "Retry" }: { title: React.ReactNode; why?: React.ReactNode; onRetry?: () => void; detail?: React.ReactNode; className?: string; retryLabel?: string }) {
  return (
    <div role="alert" className={cn("flex min-w-0 flex-col gap-2 rounded-lg border border-status-failed/30 bg-status-failed-soft px-4 py-3 text-sm", className)}>
      <div className="flex min-w-0 items-start gap-2.5">
        <OctagonAlertIcon className="mt-0.5 size-4 shrink-0 text-status-failed" strokeWidth={1.75} aria-hidden />
        <div className="min-w-0 flex-1">
          <div className="font-medium leading-5 text-foreground anywhere">{title}</div>
          {why ? <div className="mt-0.5 truncate text-sm leading-5 text-muted-foreground" title={typeof why === "string" ? why : undefined}>{why}</div> : null}
        </div>
        {onRetry ? (
          <Button size="sm" variant="outline" className="shrink-0" onClick={onRetry}>
            <RefreshCwIcon /> {retryLabel}
          </Button>
        ) : null}
      </div>
      {detail ? (
        <details className="group/detail min-w-0 pl-[26px]">
          <summary className="cursor-pointer select-none text-xs text-muted-foreground hover:text-foreground [&::-webkit-details-marker]:hidden">
            <ChevronRightIcon className="mr-1 inline size-3 transition-transform group-open/detail:rotate-90" aria-hidden />
            Technical detail
          </summary>
          <pre className="mt-1.5 max-h-48 overflow-auto rounded-md bg-muted p-2 font-mono text-xs leading-5 text-foreground whitespace-pre-wrap anywhere">{detail}</pre>
        </details>
      ) : null}
    </div>
  );
}

export function ErrorNotice({ error, onRetry, children }: { error?: string; onRetry?: () => void; children?: React.ReactNode }) {
  return (
    <div role="alert" className="flex flex-wrap items-start gap-2.5 rounded-xl border border-bad/30 bg-bad/[0.06] px-4 py-3 text-sm text-foreground">
      <OctagonAlertIcon className="mt-0.5 size-4 shrink-0 text-bad" aria-hidden />
      <span className="min-w-0 flex-1 leading-relaxed anywhere">{children ?? "This didn't load: " + error}</span>
      {onRetry ? (
        <Button size="xs" variant="outline" className="ml-auto" onClick={onRetry}>
          <RefreshCwIcon /> Try again
        </Button>
      ) : null}
    </div>
  );
}

const NOTICE_TONE: Record<Tone, string> = {
  neutral: "border-border bg-muted/50",
  ok: "border-ok/30 bg-ok/[0.07]",
  warn: "border-warn/35 bg-warn/[0.08]",
  bad: "border-bad/30 bg-bad/[0.06]",
  info: "border-info/25 bg-info/[0.06]",
  accent: "border-accent-2/30 bg-accent-2/[0.06]",
  you: "border-[color-mix(in_oklch,var(--you)_55%,var(--border))] bg-you-soft",
};
const NOTICE_ICON: Record<Tone, string> = {
  neutral: "text-muted-foreground",
  ok: "text-ok",
  warn: "text-warn",
  bad: "text-bad",
  info: "text-info",
  accent: "text-accent-2",
  you: "text-you-ink",
};

// A notice the owner may put away gets an × at its end; dismissLabel names
// what goes away for screen readers ("Dismiss the goal note for blog").
export function Notice({ tone = "info", children, className, onDismiss, dismissLabel = "Dismiss" }: { tone?: Tone; children: React.ReactNode; className?: string; onDismiss?: () => void; dismissLabel?: string }) {
  const Icon = tone === "warn" ? AlertTriangleIcon : tone === "bad" ? OctagonAlertIcon : tone === "ok" ? CheckCircle2Icon : InfoIcon;
  return (
    <div className={cn("flex items-start gap-2.5 rounded-xl border px-4 py-3 text-sm leading-relaxed text-foreground anywhere", NOTICE_TONE[tone], className)}>
      <Icon className={cn("mt-0.5 size-4 shrink-0", NOTICE_ICON[tone])} aria-hidden />
      <div className={cn("min-w-0", onDismiss && "flex-1")}>{children}</div>
      {onDismiss ? (
        <Button size="icon-xs" variant="ghost" className="-my-1 -mr-2 shrink-0 rounded-full text-muted-foreground hover:text-foreground" onClick={onDismiss} aria-label={dismissLabel}>
          <XIcon />
        </Button>
      ) : null}
    </div>
  );
}

// Details on demand, one look everywhere: a chevron that turns when open.
// `inline` is a quiet text toggle ("Show details", "Prefer the terminal?");
// `row` is a full-width header for a folded section inside a card. Native
// <details>, so it works without script and keeps its state per render.
export function Disclosure({ summary, children, variant = "inline", open, onToggle, className, contentClassName, hint }: { summary: React.ReactNode; children: React.ReactNode; variant?: "inline" | "row"; open?: boolean; onToggle?: (open: boolean) => void; className?: string; contentClassName?: string; hint?: React.ReactNode }) {
  return (
    <details className={cn("min-w-0", className)} open={open} onToggle={onToggle ? (e) => onToggle(e.currentTarget.open) : undefined}>
      <summary
        className={cn(
          "flex cursor-pointer list-none items-center gap-1.5 select-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring [&::-webkit-details-marker]:hidden",
          variant === "inline" ? "w-fit rounded-md text-[13px] font-medium text-muted-foreground hover:text-foreground" : "w-full rounded-lg px-1 py-2 text-sm font-medium text-foreground hover:bg-muted/50",
        )}
      >
        <ChevronRightIcon className="size-3.5 shrink-0 text-muted-foreground transition-transform duration-150 motion-reduce:transition-none [details[open]>summary>&]:rotate-90" aria-hidden />
        <span className="min-w-0 anywhere">{summary}</span>
        {hint ? <span className="ml-auto shrink-0 pl-3 text-right text-xs font-normal text-muted-foreground">{hint}</span> : null}
      </summary>
      <div className={cn("mt-2 min-w-0", variant === "row" && "px-1 pb-1", contentClassName)}>{children}</div>
    </details>
  );
}

export function Stack({ children, className, gap = 3 }: { children: React.ReactNode; className?: string; gap?: 1 | 2 | 3 | 4 }) {
  return <div className={cn("grid", gap === 1 ? "gap-1" : gap === 2 ? "gap-2" : gap === 4 ? "gap-4" : "gap-3", className)}>{children}</div>;
}

export function SkeletonPage({ rows = 3 }: { rows?: number }) {
  return (
    <div className="grid gap-4 pt-6 sm:pt-8" aria-busy="true" aria-live="polite">
      <span className="sr-only">Loading…</span>
      <div className="flex items-center gap-3">
        <PulseMascot mood="busy" size={44} decorative className="float" />
        <Skeleton className="h-9 w-64 rounded-lg" />
      </div>
      <Skeleton className="h-5 w-full max-w-xl rounded-md" />
      <Skeleton className="mt-2 h-20 rounded-lg" />
      {Array.from({ length: rows }).map((_, i) => (
        <Skeleton key={i} className="h-36 rounded-lg" />
      ))}
    </div>
  );
}

export function SkeletonRows({ n = 5, className }: { n?: number; className?: string }) {
  return (
    <div className={cn("grid gap-2", className)} aria-busy="true">
      {Array.from({ length: n }).map((_, i) => (
        <Skeleton key={i} className="h-10 rounded-lg" />
      ))}
    </div>
  );
}

const KPI_TONE: Record<Tone, string> = {
  neutral: "text-foreground",
  ok: "text-foreground",
  warn: "text-warn",
  bad: "text-bad",
  info: "text-foreground",
  accent: "text-foreground",
  you: "text-foreground",
};
const KPI_DOT: Record<Tone, string> = {
  neutral: "",
  ok: "bg-ok",
  warn: "bg-warn",
  bad: "bg-bad",
  info: "bg-info",
  accent: "bg-accent-2",
  you: "bg-you",
};

// One number in a stat strip: a plain label, the value, a short line under
// it. Inside a KpiGrid the cells share hairlines instead of being separate
// cards. The value stays in the accessible text even when clamped.
export function Kpi({ n, label, sub, href, tone = "neutral", spark, icon: Icon }: { n: number | string; label: string; sub?: React.ReactNode; href?: string; tone?: Tone; spark?: React.ReactNode; icon?: React.ComponentType<{ className?: string }> }) {
  const value = typeof n === "number" ? num(n) : n;
  const body = (
    <>
      <div className="flex min-w-0 items-center justify-between gap-2">
        <span className="flex min-w-0 items-center gap-1.5 text-[13px] leading-snug text-muted-foreground">
          {Icon ? <Icon className="size-3.5 shrink-0" aria-hidden /> : null}
          <span className="min-w-0 anywhere">{label}</span>
        </span>
        {spark ? <span className="shrink-0 text-muted-foreground">{spark}</span> : null}
      </div>
      <div className="mt-1.5 flex min-w-0 items-center gap-2">
        {KPI_DOT[tone] ? <span className={cn("size-2 shrink-0 rounded-full", KPI_DOT[tone])} aria-hidden /> : null}
        <span key={value} title={value} className={cn("tick line-clamp-2 min-w-0 origin-left text-xl font-semibold leading-tight tracking-tight anywhere", KPI_TONE[tone])}>{value}</span>
      </div>
      {sub ? <div title={typeof sub === "string" ? sub : undefined} className="mt-1 text-xs leading-relaxed text-muted-foreground anywhere">{sub}</div> : null}
    </>
  );
  const cls = "group/kpi block min-w-0 bg-card px-4 py-3.5 shadow-[1px_0_0_0_var(--border),0_1px_0_0_var(--border)]";
  return href ? (
    <Link href={href} className={cn(cls, "transition-colors hover:bg-muted/50 focus-visible:relative focus-visible:z-10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset motion-reduce:transition-none")}>
      {body}
    </Link>
  ) : (
    <div className={cls}>{body}</div>
  );
}

// A stat strip: one bordered block, cells divided by hairlines. Two columns
// on a phone, four from tablets up (override with className).
export function KpiGrid({ children, className }: { children: React.ReactNode; className?: string }) {
  return <div className={cn("mb-6 grid min-w-0 grid-cols-2 overflow-hidden rounded-lg border bg-card md:grid-cols-4", className)}>{children}</div>;
}

export function Toolbar({ children, className }: { children: React.ReactNode; className?: string }) {
  return <div className={cn("mb-4 flex flex-wrap items-center gap-2", className)}>{children}</div>;
}

export function Count({ children }: { children: React.ReactNode }) {
  return <span className="ml-auto rounded-full bg-muted px-2 py-0.5 text-xs whitespace-nowrap text-muted-foreground tnum">{children}</span>;
}

export function Kv({ rows, className }: { rows: [React.ReactNode, React.ReactNode][]; className?: string }) {
  return (
    <dl className={cn("grid grid-cols-1 text-sm sm:grid-cols-[minmax(6rem,auto)_1fr] sm:gap-x-4 sm:gap-y-1.5", className)}>
      {rows.map(([k, v], i) => (
        <div key={i} className="contents">
          <dt className={cn("text-muted-foreground first-letter:uppercase max-sm:text-xs", i > 0 && "max-sm:mt-2.5")}>{k}</dt>
          <dd className="m-0 min-w-0 text-foreground anywhere">{v}</dd>
        </div>
      ))}
    </dl>
  );
}
