"use client";

import Link from "next/link";
import { Fragment, useEffect, useState, useSyncExternalStore } from "react";
import { ArrowLeftIcon, ChevronRightIcon, CircleHelpIcon, SearchIcon } from "lucide-react";
import { AskTrigger } from "@/components/ask/ask-panel";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Kbd } from "@/components/hive/shortcut-help";
import { useEventStream } from "@/lib/use-api";
import { cn } from "@/lib/utils";
import { Bell } from "./bell";
import { CommandPalette } from "./command-palette";
import { backHref, phoneTitle } from "./crumbs-state";
import { useCurrentCrumbs } from "./crumbs";

export const TOPBAR_H = "48px";

// Whether the hub's live event stream is up: a steady dot, amber while it
// reconnects. Shared by the desktop top bar and the phone's More sheet.
export function LiveStatus({ compact, className }: { compact?: boolean; className?: string }) {
  const { state } = useEventStream();
  const live = state === "live";
  const label = live ? "Live: changes show up as they happen" : state === "polling" ? "Reconnecting: refreshing every 10 seconds until the live stream is back" : "Connecting to Hive…";
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span
          tabIndex={0}
          role="status"
          aria-live="polite"
          aria-atomic="true"
          aria-label={label}
          className={cn(
            "inline-flex h-8 items-center gap-2 rounded-md px-2 text-[13px] text-muted-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring",
            compact && "w-8 justify-center px-0",
            className,
          )}
        >
          <span aria-hidden data-live={live} className={cn("live-dot size-2 shrink-0 rounded-full", live ? "bg-ok" : state === "polling" ? "bg-warn" : "bg-muted-foreground/50")} />
          <span className={compact ? "sr-only" : undefined}>{live ? "Live" : state === "polling" ? "Reconnecting" : "Connecting"}</span>
        </span>
      </TooltipTrigger>
      <TooltipContent side="bottom">{label}</TooltipContent>
    </Tooltip>
  );
}

// True once the window has scrolled at all; the bar draws its border then.
function useScrolled(): boolean {
  const [scrolled, setScrolled] = useState(false);
  useEffect(() => {
    const read = () => setScrolled(window.scrollY > 0);
    read();
    window.addEventListener("scroll", read, { passive: true });
    return () => window.removeEventListener("scroll", read);
  }, []);
  return scrolled;
}

export function Breadcrumbs({ className }: { className?: string }) {
  const crumbs = useCurrentCrumbs();
  return (
    <nav aria-label="Breadcrumb" className={cn("flex min-w-0 items-center gap-1 text-sm", className)}>
      {crumbs.map((c, i) => {
        const last = i === crumbs.length - 1;
        return (
          <Fragment key={i + ":" + c.label}>
            {i > 0 ? <ChevronRightIcon aria-hidden className="size-3.5 shrink-0 text-muted-foreground/70" /> : null}
            {c.href && !last ? (
              <Link href={c.href} className="min-w-0 max-w-56 truncate rounded-sm text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
                {c.label}
              </Link>
            ) : (
              <span className={cn("min-w-0 max-w-72 truncate", last ? "font-medium text-foreground" : "text-muted-foreground")} aria-current={last ? "page" : undefined}>
                {c.label}
              </span>
            )}
          </Fragment>
        );
      })}
    </nav>
  );
}

// The desktop top bar (spec "Top bar"): 48 px, sticky, blurred over the
// page; the breadcrumbs on the left, Ask Hive, the bell, the live dot and
// the shortcut help on the right. The border shows once the page scrolls.
export function TopBar({ onHelp }: { onHelp: () => void }) {
  const scrolled = useScrolled();
  return (
    <header
      data-scrolled={scrolled ? "" : undefined}
      className="sticky top-0 z-40 hidden h-(--topbar-h) shrink-0 items-center gap-3 border-b border-transparent bg-background/80 px-4 backdrop-blur transition-colors duration-120 ease-out-soft data-scrolled:border-border lg:flex motion-reduce:transition-none"
    >
      <Breadcrumbs className="min-w-0 flex-1" />
      <div className="flex shrink-0 items-center gap-1">
        <AskTrigger variant="bar" className="text-[13px] hover:text-foreground" />
        <Bell />
        <LiveStatus compact />
        <Tooltip>
          <TooltipTrigger asChild>
            <Button variant="ghost" size="icon-sm" className="text-muted-foreground hover:text-foreground" aria-label="Keyboard shortcuts (?)" aria-keyshortcuts="?" onClick={onHelp}>
              <CircleHelpIcon aria-hidden className="size-[18px]" strokeWidth={1.75} />
            </Button>
          </TooltipTrigger>
          <TooltipContent side="bottom" className="flex items-center gap-2">
            Keyboard shortcuts <Kbd>?</Kbd>
          </TooltipContent>
        </Tooltip>
      </div>
    </header>
  );
}

// The phone's header: the page's title (the last crumb, else the document
// title without " · Hive"), a back arrow on detail pages, Search and Ask.
// The document title as an external store: a MutationObserver on <title>
// wakes subscribers when DocumentTitle rewrites it.
function subscribeTitle(cb: () => void): () => void {
  const t = document.querySelector("title");
  if (!t) return () => {};
  const mo = new MutationObserver(cb);
  mo.observe(t, { childList: true, characterData: true, subtree: true });
  return () => mo.disconnect();
}
const readTitle = () => document.title;
const readTitleServer = () => "";

export function PhoneBar() {
  const crumbs = useCurrentCrumbs();
  const docTitle = useSyncExternalStore(subscribeTitle, readTitle, readTitleServer);
  const title = phoneTitle(crumbs, docTitle);
  const back = backHref(crumbs);
  return (
    <header className="sticky top-0 z-40 flex h-14 shrink-0 items-center gap-1 border-b bg-background/90 px-2 backdrop-blur pt-[env(safe-area-inset-top)] lg:hidden">
      {back ? (
        <Button asChild variant="ghost" size="icon" className="size-10 text-muted-foreground">
          <Link href={back} aria-label={"Back to " + (crumbs[crumbs.length - 2]?.label || "the previous page")}>
            <ArrowLeftIcon aria-hidden className="size-5" />
          </Link>
        </Button>
      ) : (
        <span className="w-1" aria-hidden />
      )}
      <p className="min-w-0 flex-1 truncate text-base font-semibold tracking-tight" title={title}>
        {title}
      </p>
      <CommandPalette variant="icon" aria-label="Search (Command K)">
        <SearchIcon aria-hidden className="size-5" />
      </CommandPalette>
      <AskTrigger variant="icon" />
    </header>
  );
}
