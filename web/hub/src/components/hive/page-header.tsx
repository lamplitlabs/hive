"use client";

import { cn } from "@/lib/utils";

// The top of every page (spec "Components" - Page header): a 24 px title
// with an optional 20 px lead (an icon or a project avatar), a one-line
// subtitle, ONE primary action on the right and an underline tab strip
// below. Breadcrumbs live in the top bar, not here.
export function PageHeader({
  title,
  subtitle,
  action,
  tabs,
  badge,
  lead,
  className,
  titleClassName,
}: {
  title: React.ReactNode;
  subtitle?: React.ReactNode;
  action?: React.ReactNode;
  tabs?: React.ReactNode;
  badge?: React.ReactNode;
  lead?: React.ReactNode;
  className?: string;
  titleClassName?: string;
}) {
  return (
    <header className={cn("mb-6 min-w-0 pt-5 sm:pt-6", className)}>
      <div className="flex min-w-0 items-start justify-between gap-x-4 gap-y-3 max-sm:flex-wrap">
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-x-2.5 gap-y-1">
            {lead ? <span className="flex size-5 shrink-0 items-center justify-center text-muted-foreground [&>svg]:size-5" aria-hidden>{lead}</span> : null}
            <h1 className={cn("min-w-0 truncate text-2xl font-semibold leading-8 tracking-tight", titleClassName)} title={typeof title === "string" ? title : undefined}>
              {title}
            </h1>
            {badge ? <div className="flex min-w-0 shrink-0 flex-wrap items-center gap-1.5">{badge}</div> : null}
          </div>
          {subtitle ? <p className="mt-1 min-w-0 truncate text-sm leading-5 text-muted-foreground">{subtitle}</p> : null}
        </div>
        {action ? <div className="flex shrink-0 items-center gap-2 max-sm:w-full max-sm:justify-end sm:pt-0.5">{action}</div> : null}
      </div>
      {tabs ? <div className="mt-4 min-w-0 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">{tabs}</div> : null}
    </header>
  );
}
