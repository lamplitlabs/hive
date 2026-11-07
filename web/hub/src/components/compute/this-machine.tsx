"use client";

import Link from "next/link";
import { useState } from "react";
import { CirclePauseIcon, LaptopIcon, PlayIcon, SettingsIcon } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { showApiError } from "@/components/pulse/confirm";
import { ConfirmDialog } from "@/components/pulse/confirm";
import { BrandLogo, logoFor } from "@/components/pulse/logo";
import { StatusBadge } from "@/components/pulse/status-badge";
import { awakeText, awakeView } from "@/components/shell/awake";
import { writes, type Executor, type Runner, type SettingsPayload } from "@/lib/api";
import { num, plural } from "@/lib/format";
import { hrefs } from "@/lib/nav";
import { invalidate, useApi } from "@/lib/use-api";
import { cn } from "@/lib/utils";
import { RUNNER_STATE_WORD, diskMeter, keepAwakeOn, runnerState, slotsMeter, thisMachineOs } from "./compute-state";
import { Meter } from "./meter";

const CHANGES = (p: string) => ["settings", "runner", "service", "running", "compute", "org", "events"].some((path) => p === path || p.startsWith(path + "?"));

// This machine as the hero card of the Machines page (spec "Machines"): the
// system's logo and the host's name, the runner's state with its Start /
// Pause button, three small meters and the keep-awake switch. `runner`
// undefined is the state still loading; null is a load that failed.
//
// The runner heartbeat carries no CPU or memory figures, so those two
// meters read "—" until it does (the disk one follows the daemon's
// `disk:` headroom line, which it writes only while space is short).
export function ThisMachineCard({ runner, host, local, className }: { runner: Runner | undefined | null; host?: string | null; local?: Executor | null; className?: string }) {
  const settings = useApi<SettingsPayload>("settings", { refresh: 30000 });
  const canEdit = !!settings.data?.can_edit;
  const stopSetting = settings.data?.settings.find((s) => s.name === "stop");
  const awakeSetting = settings.data?.settings.find((s) => s.name === "daemon.keep_awake");
  const [pending, setPending] = useState<"pause" | "resume" | null>(null);
  const [awakeBusy, setAwakeBusy] = useState(false);

  const r = runner ?? null;
  const available = !!r?.available;
  const state = available ? runnerState(r) : "stopped";
  const paused = state === "paused" || stopSetting?.value === "1";
  const badge = paused ? "paused" : state;
  const os = thisMachineOs(local, host, typeof navigator !== "undefined" ? navigator.userAgent : "");
  const logo = os === "macos" ? "apple" : logoFor(os);
  const name = (host || "").trim() || local?.name || "This machine";
  const slots = local ? slotsMeter(local) : r ? { pct: r.hands.slots ? Math.round(((r.hands.slots - r.hands.free) / r.hands.slots) * 100) : null, text: r.hands.slots ? num(r.hands.free) + " of " + num(r.hands.slots) + " free" : "—" } : { pct: null, text: "—" };
  const disk = diskMeter(r?.daemon?.disk);
  const awake = awakeView(r ?? undefined);
  const awakeOn = keepAwakeOn(awakeSetting?.value);
  const busy = r ? r.jobs.running + r.jobs.leased : 0;
  const queued = r?.jobs.queued ?? 0;
  const line = !r
    ? runner === undefined
      ? "Checking the runner…"
      : "The runner's state didn't load."
    : !available
      ? "The runner hasn't started on this machine."
      : paused
        ? "Work is paused; nothing new starts until you resume."
        : state === "running"
          ? (busy ? num(busy) + " running" : "Nothing running") + (queued ? ", " + plural(queued, "job") + " queued" : "")
          : r.daemon.text || "The runner is off.";

  const setAwake = async (on: boolean) => {
    if (!awakeSetting) return;
    setAwakeBusy(true);
    try {
      await writes.setSetting("daemon.keep_awake", on ? "on" : "off");
      toast.success(on ? "The machine stays awake while the runner works." : "The machine sleeps as its power settings say.");
      invalidate(CHANGES);
    } catch (e) {
      showApiError(e);
    } finally {
      setAwakeBusy(false);
    }
  };

  return (
    <section aria-label="This machine" className={cn("grid min-w-0 gap-5 rounded-lg border bg-card p-6", className)}>
      <div className="flex min-w-0 flex-wrap items-start gap-4">
        <span className="inline-flex size-12 shrink-0 items-center justify-center rounded-lg bg-muted text-foreground" aria-hidden>
          {logo ? <BrandLogo id={logo} size={24} title={os || undefined} /> : <LaptopIcon className="size-6" strokeWidth={1.75} />}
        </span>
        <div className="min-w-0 flex-1 basis-56">
          <div className="flex min-w-0 flex-wrap items-center gap-2">
            <h2 className="min-w-0 truncate text-xl font-semibold leading-7 tracking-tight" title={name}>{name}</h2>
            <StatusBadge status={badge} label={RUNNER_STATE_WORD[paused ? "paused" : state]} />
          </div>
          <p className="mt-0.5 truncate text-sm text-muted-foreground" title={r?.capacity || undefined}>{line}</p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {available && (paused || state === "running") ? (
            paused ? (
              <Button size="sm" disabled={!canEdit || !stopSetting} onClick={() => setPending("resume")}>
                <PlayIcon /> Start
              </Button>
            ) : (
              <Button size="sm" variant="outline" disabled={!canEdit || !stopSetting} onClick={() => setPending("pause")}>
                <CirclePauseIcon /> Pause
              </Button>
            )
          ) : (
            <Button asChild size="sm" variant="outline">
              <Link href="/#daemon-controls">
                <PlayIcon /> Start
              </Link>
            </Button>
          )}
          <Tooltip>
            <TooltipTrigger asChild>
              <Button asChild size="icon-sm" variant="ghost" aria-label="Runner settings">
                <Link href={hrefs.settings("daemon")}>
                  <SettingsIcon />
                </Link>
              </Button>
            </TooltipTrigger>
            <TooltipContent>Runner settings</TooltipContent>
          </Tooltip>
        </div>
      </div>

      <div className="grid gap-4 sm:grid-cols-3">
        <Meter label="CPU" pct={null} title="The runner does not report CPU use yet" />
        <Meter label="Memory" pct={null} title="The runner does not report memory use yet" />
        <Meter label="Disk" pct={disk.pct} text={disk.text} title={r?.daemon?.disk || "The runner reports disk only while headroom is short"} />
      </div>

      <div className="flex min-w-0 flex-wrap items-center gap-x-4 gap-y-2 border-t pt-4 text-sm">
        <span className="text-muted-foreground">{"Agent slots: " + slots.text}</span>
        <label className="ml-auto flex items-center gap-2.5">
          <span className="min-w-0 text-sm font-medium text-foreground" title={awakeText(awake)}>
            Keep awake
          </span>
          <Switch size="sm" checked={awakeOn} disabled={!canEdit || !awakeSetting || awakeBusy} onCheckedChange={(v) => void setAwake(v)} aria-label="Keep this machine awake while the runner works" />
        </label>
      </div>

      <ConfirmDialog
        open={pending !== null}
        onOpenChange={(o) => !o && setPending(null)}
        title={pending === "pause" ? "Pause all work?" : "Start work again?"}
        description={pending === "pause" ? "Agents stop starting new work on this machine. Runs already going finish first. You can start again at any time." : "Agents start picking up work again on their next pass."}
        action={pending === "pause" ? "Pause" : "Start"}
        destructive={pending === "pause"}
        success={pending === "pause" ? "Paused. Nothing new starts until you press Start." : "Started. The runner picks this up on its next check-in."}
        onConfirm={() => writes.setSetting("stop", pending === "pause" ? "1" : "0")}
        invalidates={CHANGES}
      />
    </section>
  );
}
