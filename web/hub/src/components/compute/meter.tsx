"use client";

import { cn } from "@/lib/utils";
import { meterTone, type MeterTone } from "./compute-state";

const FILL: Record<MeterTone, string> = {
  ok: "bg-status-done",
  warning: "bg-status-waits",
  failed: "bg-status-failed",
};

// A small meter (spec "Machines"): a 13 px label, the value to its right and
// a 4 px bar in the muted tint, filled green / marigold / red by how full it
// is (compute-state meterTone). `pct` null draws an empty bar and the value
// reads "—" unless `text` says otherwise - the console never invents a
// number the runner did not report. Without a number the bar is no meter to
// a screen reader either (a meter has to say its value): it is left out, and
// the dash is read as "not reported".
export function Meter({ label, pct, text, className, title }: { label: string; pct: number | null | undefined; text?: string; className?: string; title?: string }) {
  const has = pct !== null && pct !== undefined && Number.isFinite(pct);
  const value = has ? Math.max(0, Math.min(100, Math.round(pct))) : 0;
  const tone = meterTone(has ? value : null);
  const word = text ?? (has ? value + "%" : "—");
  return (
    <div className={cn("grid min-w-0 gap-1", className)} title={title}>
      <div className="flex items-baseline justify-between gap-2 text-[13px] leading-4">
        <span className="truncate text-muted-foreground">{label}</span>
        <span className={cn("tnum shrink-0 font-medium", has ? "text-foreground" : "text-muted-foreground")}>
          {has || text !== undefined ? (
            word
          ) : (
            <>
              <span aria-hidden>{word}</span>
              <span className="sr-only">not reported</span>
            </>
          )}
        </span>
      </div>
      <div
        className="h-1 w-full overflow-hidden rounded-full bg-muted"
        {...(has ? { role: "meter", "aria-label": label, "aria-valuemin": 0, "aria-valuemax": 100, "aria-valuenow": value, "aria-valuetext": word } : { "aria-hidden": true })}
        data-tone={has ? tone : "none"}
      >
        {has ? <div className={cn("h-full rounded-full transition-[width] duration-300 ease-out-soft motion-reduce:transition-none", FILL[tone])} style={{ width: value + "%" }} /> : null}
      </div>
    </div>
  );
}
