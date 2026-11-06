"use client";

import Link from "next/link";
import { ChevronRightIcon } from "lucide-react";
import { daemonControlState, daemonHeadline } from "@/components/pulse/daemon-control-state";
import type { HarnessStatus, Runner, ServicePayload, SettingsPayload } from "@/lib/api";
import { hrefs } from "@/lib/nav";
import { providerInfo } from "@/lib/providers";
import { useApi, useNow } from "@/lib/use-api";
import { cn } from "@/lib/utils";
import { RailTip } from "./rail-tip";

// The two facts every page should show without a click, at the foot of the
// sidebar: can agents work on this machine (the runner, with what it is
// doing), and which model provider they use (and whether it is connected).
// Each line links to where it is fixed - the runner controls on Home, the
// provider card or the Connect a provider wizard in Settings. On the rail
// they shrink to two dots with the same words as tooltips.

type Tone = "ok" | "warn" | "bad" | "neutral";
const DOT: Record<Tone, string> = { ok: "bg-ok", warn: "bg-warn", bad: "bg-bad", neutral: "bg-muted-foreground/50" };

export interface StatusLine {
  key: string;
  label: string;
  sub: string;
  tone: Tone;
  href: string;
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
    href: state.pause === "on" ? hrefs.settings("stop") : "/#daemon-controls",
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
      }
    : harness.error && !h
      ? { key: "provider", label: "Model provider", sub: "Couldn't check", tone: "neutral", href: hrefs.settings("harness") }
      : null;

  return providerLine ? [runnerLine, providerLine] : [runnerLine];
}

export function SystemStatus() {
  const lines = useSystemStatus();
  return (
    <div className="grid gap-0.5 rounded-xl border bg-card p-1 collapsed:border-0 collapsed:bg-transparent collapsed:p-0">
      {lines.map((l) => (
        <RailTip key={l.key} label={l.label} hint={l.sub || undefined}>
          <Link
            href={l.href}
            className="group flex min-w-0 items-center gap-2.5 rounded-lg px-2 py-1.5 transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring collapsed:justify-center collapsed:px-0 collapsed:py-2"
            aria-label={l.label + (l.sub ? ": " + l.sub : "")}
          >
            <span aria-hidden className={cn("size-2 shrink-0 rounded-full", DOT[l.tone])} />
            <span className="min-w-0 flex-1 collapsed:sr-only">
              <span className="block truncate text-[13px] font-medium leading-tight text-foreground">{l.label}</span>
              {l.sub ? <span className={cn("block truncate text-xs leading-tight", l.tone === "warn" || l.tone === "bad" ? "text-warn" : "text-muted-foreground")}>{l.sub}</span> : null}
            </span>
            <ChevronRightIcon aria-hidden className="size-3.5 shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 collapsed:hidden" />
          </Link>
        </RailTip>
      ))}
    </div>
  );
}
