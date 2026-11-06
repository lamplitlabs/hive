"use client";

import { Circle, CircleCheck, CircleDot, CirclePause, CircleX, Clock, Info, LoaderCircle, TriangleAlert, type LucideIcon } from "lucide-react";
import { cn } from "@/lib/utils";
import { statusWord } from "@/lib/words";
import { STATUS_CLASS, statusKind, type StatusKind } from "./status-badge-state";

export { statusKind, STATUS_CLASS, STATUS_KINDS } from "./status-badge-state";
export type { StatusKind } from "./status-badge-state";

// A status is always an icon plus a word, never colour alone (spec "Icons
// and logos"). The spinner turns only for things actually running and
// stops under reduced motion.
export const STATUS_ICON: Record<StatusKind, LucideIcon> = {
  running: LoaderCircle,
  done: CircleCheck,
  waits: CircleDot,
  queued: Clock,
  paused: CirclePause,
  failed: CircleX,
  warning: TriangleAlert,
  info: Info,
  neutral: Circle,
};

export const STATUS_KIND_WORD: Record<StatusKind, string> = {
  running: "Running",
  done: "Done",
  waits: "Waits on you",
  queued: "Queued",
  paused: "Paused",
  failed: "Failed",
  warning: "Needs fixing",
  info: "Note",
  neutral: "—",
};

// `status` is a raw store state ("in_progress", "eligible") or one of the
// nine kinds; `label` overrides the word. Renders nothing for an empty
// status so callers can pass what they have.
export function StatusBadge({ status, label, size = "md", className, title }: { status?: string | null; label?: React.ReactNode; size?: "sm" | "md"; className?: string; title?: string }) {
  const raw = (status || "").trim();
  if (!raw && label === undefined) return null;
  const kind = statusKind(raw);
  const Icon = STATUS_ICON[kind];
  const word = label ?? (raw ? statusWord(raw) : STATUS_KIND_WORD[kind]);
  const spaced = raw.replace(/_/g, " ").toLowerCase();
  const tip = title ?? (typeof word === "string" && word.toLowerCase() !== spaced && raw ? raw : undefined);
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center gap-1 rounded-md font-medium whitespace-nowrap",
        size === "sm" ? "h-5 px-1 text-[11px]" : "h-6 px-1.5 text-xs",
        STATUS_CLASS[kind],
        className,
      )}
      title={tip}
      data-status={kind}
    >
      <Icon className={cn("shrink-0", size === "sm" ? "size-3" : "size-3.5", kind === "running" && "motion-safe:animate-spin")} strokeWidth={1.75} aria-hidden />
      {word}
    </span>
  );
}

// The status icon alone, for a tight spot (a row's right edge). Carries the
// word as its accessible label so colour is never the only signal.
export function StatusIcon({ status, className }: { status?: string | null; className?: string }) {
  const raw = (status || "").trim();
  if (!raw) return null;
  const kind = statusKind(raw);
  const Icon = STATUS_ICON[kind];
  const word = statusWord(raw);
  return (
    <Icon
      className={cn("size-4 shrink-0", STATUS_CLASS[kind].split(" ")[1], kind === "running" && "motion-safe:animate-spin", className)}
      strokeWidth={1.75}
      role="img"
      aria-label={word}
    >
      <title>{word}</title>
    </Icon>
  );
}
