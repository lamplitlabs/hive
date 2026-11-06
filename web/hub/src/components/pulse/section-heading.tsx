"use client";

import { cn } from "@/lib/utils";

// The heading of a block within a page: 16 px semibold with a 16 px icon,
// a count chip, a one-line note and an action link on the right (spec
// "Components" - Section heading). `as` picks the heading level; the
// default h2 sits under the page header's h1.
export function SectionHeading({
  children,
  count,
  action,
  note,
  className,
  icon: Icon,
  as: Heading = "h2",
  id,
  countTone = "neutral",
}: {
  children: React.ReactNode;
  count?: number | string;
  // Marigold only for the things that wait on the owner (spec "Count chip").
  countTone?: "neutral" | "you";
  action?: React.ReactNode;
  note?: React.ReactNode;
  className?: string;
  icon?: React.ComponentType<{ className?: string; strokeWidth?: number; "aria-hidden"?: boolean }>;
  as?: "h2" | "h3" | "h4";
  id?: string;
}) {
  const hasCount = count !== undefined && count !== null && count !== "";
  return (
    <div className={cn("mb-3 flex min-w-0 items-center justify-between gap-x-4 gap-y-1", className)}>
      <div className="flex min-w-0 flex-1 items-baseline gap-x-3 gap-y-0.5 max-sm:flex-wrap">
        <Heading id={id} className="flex min-w-0 shrink-0 items-center gap-2 text-base font-semibold leading-6 tracking-tight">
          {Icon ? <Icon className="size-4 shrink-0 text-muted-foreground" strokeWidth={1.75} aria-hidden /> : null}
          <span className="truncate">{children}</span>
          {hasCount ? <span className={cn("tnum inline-flex h-5 shrink-0 items-center rounded-md px-1.5 text-xs font-medium", countTone === "you" ? "bg-you-soft text-you-ink" : "bg-muted text-muted-foreground")}>{count}</span> : null}
        </Heading>
        {note ? <p className="min-w-0 flex-1 truncate text-sm leading-6 text-muted-foreground max-sm:basis-full">{note}</p> : null}
      </div>
      {action ? <div className="flex shrink-0 items-center gap-2 text-sm leading-6 text-link [&_a]:text-link [&_a:hover]:underline">{action}</div> : null}
    </div>
  );
}
