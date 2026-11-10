"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useState } from "react";
import { MenuIcon } from "lucide-react";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { useInbox } from "@/components/hive/inbox";
import { cn } from "@/lib/utils";
import { NAV_ICON } from "./nav-icons";
import { activeItem, MORE_GROUPS, PRIMARY_ITEMS, SETTINGS_ITEM, type NavItem } from "./nav-items";
import { PresenceSwitch } from "./owner-row";
import { SystemStatus } from "./system-status";
import { LiveStatus } from "./top-bar";

const cell = "relative flex min-w-0 flex-col items-center justify-center gap-1 rounded-md px-1 py-1.5 text-xs font-medium leading-none text-muted-foreground transition-colors motion-reduce:transition-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

function Cell({ item, active, count }: { item: NavItem; active: boolean; count?: number }) {
  const Icon = NAV_ICON[item.icon];
  return (
    <Link href={item.href} aria-current={active ? "page" : undefined} className={cn(cell, active && "text-link")} aria-label={count ? item.label + ", " + count + " waiting" : undefined}>
      <span className="relative">
        <Icon aria-hidden className="size-[22px]" strokeWidth={active ? 2 : 1.75} />
        {count ? (
          <span aria-hidden className="tnum absolute -top-1.5 -right-2.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-you px-1 text-xs font-semibold text-[oklch(0.3_0.06_60)]">
            {count > 99 ? "99+" : count}
          </span>
        ) : null}
      </span>
      <span className="truncate">{item.label}</span>
    </Link>
  );
}

// Phones (spec "Phone"): Home, For you, Sessions, Tasks and More along the
// bottom, safe-area padded; More opens a sheet with the Knowledge and System
// pages, Settings, Here/Away and the status card.
export function BottomBar() {
  const pathname = usePathname();
  const { total } = useInbox();
  const [more, setMore] = useState(false);
  const active = activeItem(pathname);
  const underMore = !!active && !PRIMARY_ITEMS.includes(active);
  return (
    <nav aria-label="Main" className="fixed inset-x-0 bottom-0 z-40 border-t bg-background/95 backdrop-blur pb-[env(safe-area-inset-bottom)] lg:hidden">
      <div className="grid h-16 grid-cols-5 items-stretch px-1 pt-0.5">
        {PRIMARY_ITEMS.map((item) => (
          <Cell key={item.href} item={item} active={active === item} count={item.icon === "inbox" ? total : undefined} />
        ))}
        <button type="button" className={cn(cell, underMore && "text-link")} aria-haspopup="dialog" aria-expanded={more} onClick={() => setMore(true)}>
          <MenuIcon aria-hidden className="size-[22px]" strokeWidth={underMore ? 2 : 1.75} />
          <span>More</span>
        </button>
      </div>
      <Sheet open={more} onOpenChange={setMore}>
        <SheetContent side="bottom" className="max-h-[85dvh] overflow-y-auto rounded-t-xl pb-[max(env(safe-area-inset-bottom),1rem)]">
          <SheetHeader className="pb-1">
            <SheetTitle>More</SheetTitle>
            <SheetDescription className="sr-only">Every other page, settings and the status of this machine.</SheetDescription>
          </SheetHeader>
          <div className="grid gap-4 px-4">
            {MORE_GROUPS.map((g) => (
              <section key={g.label} aria-label={g.label ?? undefined}>
                {g.label ? <p className="mb-1 px-1 text-xs font-medium text-muted-foreground">{g.label}</p> : null}
                <ul className="grid grid-cols-2 gap-1">
                  {g.items.map((item) => {
                    const Icon = NAV_ICON[item.icon];
                    const on = active === item;
                    return (
                      <li key={item.href}>
                        <Link
                          href={item.href}
                          onClick={() => setMore(false)}
                          aria-current={on ? "page" : undefined}
                          className={cn("flex min-h-11 items-center gap-2.5 rounded-md px-2.5 text-sm font-medium text-foreground hover:bg-muted", on && "bg-accent text-link")}
                        >
                          <Icon aria-hidden className={cn("size-[18px] shrink-0", on ? "text-link" : "text-muted-foreground")} strokeWidth={1.75} />
                          <span className="truncate">{item.label}</span>
                        </Link>
                      </li>
                    );
                  })}
                </ul>
              </section>
            ))}
            <section aria-label="You">
              <div className="flex items-center justify-between gap-3 rounded-md border px-3 py-2">
                <Link href={SETTINGS_ITEM.href} onClick={() => setMore(false)} className="flex min-h-9 items-center gap-2.5 text-sm font-medium">
                  <NAV_ICON.settings aria-hidden className="size-[18px] text-muted-foreground" strokeWidth={1.75} />
                  Settings
                </Link>
                <PresenceSwitch />
              </div>
            </section>
            <section aria-label="Status" className="grid gap-2">
              <SystemStatus />
              <LiveStatus className="justify-center" />
            </section>
          </div>
        </SheetContent>
      </Sheet>
    </nav>
  );
}
