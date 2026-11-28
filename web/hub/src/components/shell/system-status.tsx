"use client";

import Link from "next/link";
import { ChevronRightIcon, CpuIcon, TriangleAlertIcon } from "lucide-react";
import { daemonControlState, daemonHeadline } from "@/components/hive/daemon-control-state";
import { BrandLogo, logoFor } from "@/components/hive/logo";
import type { HarnessStatus, Runner, ServicePayload, SettingsPayload } from "@/lib/api";
import { hrefs } from "@/lib/nav";
import { providerInfo } from "@/lib/providers";
import { useApi, useNow } from "@/lib/use-api";
import { cn } from "@/lib/utils";
import { RailTip } from "./rail-tip";

// The two facts every page should show without a click, at the foot of the
// sidebar (spec "App shell" - Status card): can agents work on this machine
// (the runner, Cpu icon and its state), and which model provider they use
// (its logo and the main model). Each line links to where it is fixed - the
// runner controls (or the pause, while work is paused), the provider card or
// the Connect a provider wizard, all in Settings. On the rail they shrink to
// the two icons with a tone dot and the same words as tooltips.

type Tone = "ok" | "warn" | "bad" | "neutral";
const DOT: Record<Tone, string> = { ok: "bg-ok", warn: "bg-warn", bad: "bg-bad", neutral: "bg-muted-foreground/50" };

export interface StatusLine {
  key: "runner" | "provider";
  label: string;
  sub: string;
  tone: Tone;
  href: string;
  /** The provider's name as the harness reports it, for its logo. */
  provider?: string;
}

function plural(n: number, one: string, many = one + "s") {
  return n.toLocaleString("en-US") + " " + (n === 1 ? one : many);
}

export function useSystemStatus(): StatusLine[] {
  const runner = useApi<Runner>("runner", { refresh: 30000 });
  const service = useApi<ServicePayload>("service", { refresh: 60000 });
  const settings = useApi<SettingsPayload>("settings", { refresh: 60000 });
  const harness = useApi<HarnessStatus>("harness", { refresh: 60000 });
  const now = useNow(60000);

  const state = daemonControlState({ runner, service, settings }, now);
  const headline = daemonHeadline(state);
  const jobs = runner.data?.jobs;
  const busy = jobs ? [jobs.running ? plural(jobs.running, "run") + " going" : "", jobs.queued ? jobs.queued.toLocaleString("en-US") + " queued" : ""].filter(Boolean).join(", ") : "";
  const runnerLine: StatusLine = {
    key: "runner",
    label: headline,
    sub: state.pause === "on" ? "Nothing new starts" : state.label === "running" || state.label === "alive" ? busy || "Waiting for work" : state.label === "checking" ? "" : "Agents can't start work",
    tone: state.pause === "on" ? "bad" : state.label === "checking" ? "neutral" : state.tone === "ok" ? "ok" : "warn",
    href: state.pause === "on" ? hrefs.settings("stop") : hrefs.runnerControls(),
  };

  const h = harness.data;
  const p = h?.provider;
  const info = p ? providerInfo(p.name, p.label) : null;
  const login = p?.login;
  const expired = !!login?.logged_in && !!login.expires_at && new Date(login.expires_at).getTime() < now;
  const connected = !!login?.logged_in && !expired;
  const providerLine: StatusLine | null = p && info
    ? {
        key: "provider",
        label: info.title,
        sub: connected ? (h.models.worker ? h.models.worker : "Default model") : expired ? "Sign-in expired" : "Not connected",
        tone: connected ? "ok" : "warn",
        href: connected ? hrefs.settings("harness") : "/settings/?connect=" + encodeURIComponent(p.name) + "#harness",
        provider: p.name,
      }
    : harness.error && !h
      ? { key: "provider", label: "Model provider", sub: "Couldn't check", tone: "neutral", href: hrefs.settings("harness") }
      : null;

  return providerLine ? [runnerLine, providerLine] : [runnerLine];
}

// The line's glyph: the Cpu for the runner, the provider's brand mark (or a
// neutral monogram when the brand has none) for the model provider. A tone
// dot sits in the glyph's corner so the state reads at a glance on the rail.
function StatusGlyph({ line }: { line: StatusLine }) {
  const logo = line.key === "provider" ? logoFor(line.provider ?? line.label) : null;
  return (
    // For the eye only: the line's own words follow it and name the link.
    <span aria-hidden className="relative inline-flex size-7 shrink-0 items-center justify-center rounded-md bg-muted text-foreground">
      {line.key === "runner" ? (
        <CpuIcon aria-hidden className="size-4" strokeWidth={1.75} />
      ) : logo ? (
        <BrandLogo id={logo} size={16} title={line.label} />
      ) : (
        <span aria-hidden className="text-xs font-semibold leading-none tracking-tight text-muted-foreground">
          {(line.label || "?").slice(0, 1).toUpperCase()}
        </span>
      )}
      <span aria-hidden className={cn("absolute -right-0.5 -bottom-0.5 size-2 rounded-full ring-2 ring-sidebar", DOT[line.tone])} />
    </span>
  );
}

// `problemsOnly` (the sidebar) draws only the lines that need a fix, and
// nothing at all while the runner and the provider are fine; two problems
// fold into one line that leads to Sessions, where each has its fix button.
// The phone's More sheet shows both lines always.
export function SystemStatus({ className, problemsOnly }: { className?: string; problemsOnly?: boolean }) {
  const all = useSystemStatus();
  const lines = problemsOnly ? all.filter((l) => l.tone === "warn" || l.tone === "bad") : all;
  if (!lines.length) return null;
  if (problemsOnly && lines.length > 1) {
    const words = lines.length + " things need a fix";
    return (
      <RailTip label={words} hint={lines.map((l) => l.label + ": " + l.sub).join(" · ")}>
        <Link
          href="/chat/"
          title={lines.map((l) => l.label + ": " + l.sub).join("\n")}
          className={cn("group flex min-w-0 items-center gap-2.5 rounded-lg border border-warn/30 bg-warn/10 px-2 py-1.5 transition-colors hover:bg-warn/15 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring collapsed:justify-center collapsed:border-0 collapsed:bg-transparent collapsed:px-0", className)}
        >
          <span aria-hidden className="relative inline-flex size-7 shrink-0 items-center justify-center rounded-md bg-card text-warn">
            <TriangleAlertIcon className="size-4" strokeWidth={1.75} />
          </span>
          <span className="min-w-0 flex-1 collapsed:sr-only">
            <span className="block truncate text-[13px] font-medium leading-tight text-foreground">{words}</span>{" "}
            <span className="block truncate text-xs leading-tight text-muted-foreground">{lines.map((l) => l.label).join(", ")}</span>
          </span>
          <ChevronRightIcon aria-hidden className="size-3.5 shrink-0 text-muted-foreground collapsed:hidden" />
        </Link>
      </RailTip>
    );
  }
  return (
    <div className={cn("grid gap-0.5 rounded-lg border bg-card p-1 collapsed:border-0 collapsed:bg-transparent collapsed:p-0", className)}>
      {lines.map((l) => (
        <RailTip key={l.key} label={l.label} hint={l.sub || undefined}>
          <Link
            href={l.href}
            className="group flex min-w-0 items-center gap-2.5 rounded-md px-1.5 py-1 transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring collapsed:justify-center collapsed:px-0 collapsed:py-1.5"
          >
            <StatusGlyph line={l} />
            {/* The link is named by its two lines; on the rail they stay for a screen reader. */}
            <span className="min-w-0 flex-1 collapsed:sr-only">
              <span className="block truncate text-[13px] font-medium leading-tight text-foreground">{l.label}</span>{" "}
              {l.sub ? <span className={cn("block truncate text-xs leading-tight", l.tone === "warn" || l.tone === "bad" ? "text-warn" : "text-muted-foreground")}>{l.sub}</span> : null}
            </span>
            <ChevronRightIcon aria-hidden className="size-3.5 shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 collapsed:hidden" />
          </Link>
        </RailTip>
      ))}
    </div>
  );
}
