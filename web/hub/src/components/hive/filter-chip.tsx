"use client";

import { cn } from "@/lib/utils";
import { num } from "@/lib/format";

// A filter chip with its count (spec "Components" - Filter bar: search,
// chips with counts, "More filters"). A 28 px control with 6 px corners;
// the count is the one count chip: 18 px, tabular numbers, marigold only
// for the things that wait on the owner. An empty chip is marked by a
// dashed edge and its 0, never hidden or faded, so the owner sees what the
// filter would show and can read it.
export function FilterChip({
  active,
  count,
  tone = "neutral",
  title,
  onClick,
  children,
  className,
}: {
  active: boolean;
  count?: number;
  tone?: "neutral" | "you";
  title?: string;
  onClick: () => void;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <button
      type="button"
      aria-pressed={active}
      title={title}
      onClick={onClick}
      className={cn(
        "inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md border px-2.5 text-xs font-medium whitespace-nowrap transition-colors duration-120 ease-out-soft focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none",
        active ? "border-foreground/20 bg-accent text-foreground" : "border-border bg-card text-muted-foreground hover:bg-muted hover:text-foreground",
        // Empty: a dashed edge, never a fade - it is an enabled button and its
        // words must read (TASK-173: opacity-60 left it at 2.68:1).
        !active && count === 0 && "border-dashed",
        className,
      )}
    >
      {children}
      {count !== undefined ? <CountChip n={count} tone={tone} active={active} /> : null}
    </button>
  );
}

// The count alone: an 18 px pill with tabular numbers.
export function CountChip({ n, tone = "neutral", active, className }: { n: number; tone?: "neutral" | "you"; active?: boolean; className?: string }) {
  return (
    <span
      className={cn(
        "tnum inline-flex h-[18px] min-w-[18px] items-center justify-center rounded-full px-1.5 text-xs font-medium leading-none",
        tone === "you" ? "bg-you-soft text-you-ink" : active ? "bg-foreground/10 text-foreground" : "bg-muted text-muted-foreground",
        className,
      )}
    >
      {num(n)}
    </span>
  );
}
