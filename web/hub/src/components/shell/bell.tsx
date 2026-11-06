"use client";

import Link from "next/link";
import { BellIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { KindTile } from "@/components/pulse/kind-tile";
import { useInbox } from "@/components/pulse/inbox";
import { When } from "@/components/pulse/who";
import { forYouRows, latestRows, waitingWords } from "@/components/for-you/for-you-state";
import { rowHref } from "@/components/for-you/row-href";
import { projectLabel, type Org } from "@/lib/api";
import { hrefs } from "@/lib/nav";
import { useApi } from "@/lib/use-api";
import { cn } from "@/lib/utils";

// The top bar's bell (spec "Top bar"): the For you count in marigold and,
// opened, the five most pressing things that wait on you - machine problems
// first, then stop questions, then the newest - each a link to where it is
// answered, with "See all" to the For you page.
export function Bell({ className }: { className?: string }) {
  const inbox = useInbox();
  const { data: org } = useApi<Org>("org");
  const rows = forYouRows({ ...inbox, brain: org?.totals.brain || "" });
  const five = latestRows(rows, 5);
  const n = inbox.total;
  const name = (slug: string) => {
    const p = [...(org?.projects ?? []), ...(org?.commons ? [org.commons] : [])].find((x) => x.slug === slug);
    return p ? projectLabel(p) : slug;
  };
  return (
    <DropdownMenu>
      <Tooltip>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="icon-sm" className={cn("relative text-muted-foreground hover:text-foreground", className)} aria-label={waitingWords(n) + ". Open the latest."}>
              <BellIcon aria-hidden className="size-[18px]" strokeWidth={1.75} />
              {n > 0 ? (
                <span aria-hidden className="absolute -top-0.5 -right-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-you px-1 text-xs font-semibold leading-none text-[oklch(0.3_0.06_60)] tnum">
                  {n > 99 ? "99+" : n}
                </span>
              ) : null}
            </Button>
          </DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent side="bottom">{waitingWords(n)}</TooltipContent>
      </Tooltip>
      <DropdownMenuContent align="end" className="w-80 p-1.5">
        <DropdownMenuLabel className="flex items-center justify-between px-2 py-1.5 text-xs font-medium text-muted-foreground">
          <span>For you</span>
          {n > 0 ? <span className="tnum">{n}</span> : null}
        </DropdownMenuLabel>
        {five.length ? (
          five.map((r) => (
            <DropdownMenuItem key={r.key} asChild className="items-start gap-2.5 rounded-md px-2 py-2">
              <Link href={rowHref(r)}>
                <KindTile kind={r.kind} size={28} waiting={r.urgent} />
                <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <span className="truncate text-[13px] font-medium leading-5 text-foreground">{r.title}</span>
                  <span className="flex min-w-0 items-center gap-1.5 truncate text-xs text-muted-foreground">
                    {r.project ? <span className="truncate">{name(r.project)}</span> : <span>This machine</span>}
                    {r.at ? (
                      <>
                        <span aria-hidden>·</span>
                        <When d={r.at} />
                      </>
                    ) : null}
                  </span>
                </span>
              </Link>
            </DropdownMenuItem>
          ))
        ) : (
          <div className="px-2 py-4 text-center text-[13px] text-muted-foreground">{inbox.ready ? "Nothing waits on you." : "Checking…"}</div>
        )}
        <DropdownMenuSeparator />
        <DropdownMenuItem asChild className="justify-center rounded-md text-[13px] font-medium text-link focus:text-link">
          <Link href={hrefs.forYou()}>See all</Link>
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
