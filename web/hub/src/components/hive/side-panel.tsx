"use client";

import { useCallback, useEffect, useSyncExternalStore } from "react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Sheet, SheetContent, SheetDescription, SheetTitle } from "@/components/ui/sheet";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

// The side panel (spec "Components" - Side panel, new): a thing opens beside
// its list instead of on a new page. On a desktop (>= 1024 px) it docks to
// the right of the list inside the page's flex row, 480 px for a detail or
// 640 for reading, sticky and full height; below that it becomes a sheet
// from the bottom. Esc closes it; the address bar keeps the open item
// through useOpenItem so a reload or a shared link lands on it.
//
// The page lays out:
//   <div className="flex items-start gap-0">
//     <div className="min-w-0 flex-1">...list...</div>
//     <SidePanel open={!!id} onClose={() => setId(null)} title=...>...</SidePanel>
//   </div>

const DESKTOP = "(min-width: 1024px)";
function subscribeDesktop(cb: () => void) {
  if (typeof window === "undefined" || !window.matchMedia) return () => {};
  const mq = window.matchMedia(DESKTOP);
  mq.addEventListener("change", cb);
  return () => mq.removeEventListener("change", cb);
}
export function useDesktop(): boolean {
  return useSyncExternalStore(
    subscribeDesktop,
    () => typeof window !== "undefined" && !!window.matchMedia && window.matchMedia(DESKTOP).matches,
    () => true,
  );
}

const WIDTH = {
  detail: "lg:w-[480px]",
  // A run's step timeline (spec "Runs"): wider than a detail, narrower than reading.
  run: "lg:w-[560px]",
  reading: "lg:w-[640px]",
} as const;

export function SidePanel({
  open,
  onClose,
  width = "detail",
  title,
  subtitle,
  lead,
  actions,
  children,
  footer,
  className,
  top = "var(--topbar-h, 3.5rem)",
}: {
  open: boolean;
  onClose: () => void;
  width?: "detail" | "run" | "reading";
  title: React.ReactNode;
  subtitle?: React.ReactNode;
  lead?: React.ReactNode;
  actions?: React.ReactNode;
  children: React.ReactNode;
  footer?: React.ReactNode;
  className?: string;
  // Where the docked panel's sticky top sits (under the top bar).
  top?: string;
}) {
  const desktop = useDesktop();

  // Esc closes the docked panel (the sheet closes itself).
  useEffect(() => {
    if (!open || !desktop) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      const t = e.target as HTMLElement | null;
      // A field with text keeps its Esc; an open menu or dialog handles its own.
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA") && (t as HTMLInputElement).value) return;
      if (document.querySelector("[role=dialog][data-state=open],[role=menu][data-state=open],[role=listbox]")) return;
      e.preventDefault();
      onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, desktop, onClose]);

  const header = (
    <div className="flex items-start gap-3 border-b px-4 py-3">
      {lead ? <span className="mt-0.5 flex shrink-0 items-center">{lead}</span> : null}
      <div className="min-w-0 flex-1">
        <h2 className="min-w-0 truncate text-base font-semibold leading-6 tracking-tight" title={typeof title === "string" ? title : undefined}>{title}</h2>
        {subtitle ? <p className="mt-0.5 min-w-0 truncate text-sm leading-5 text-muted-foreground">{subtitle}</p> : null}
      </div>
      <div className="flex shrink-0 items-center gap-1">
        {actions}
        <Tooltip>
          <TooltipTrigger asChild>
            <Button variant="ghost" size="icon-sm" aria-label="Close" onClick={onClose}>
              <X />
            </Button>
          </TooltipTrigger>
          <TooltipContent>
            Close <kbd className="ml-1 rounded border bg-muted px-1 font-mono text-xs">Esc</kbd>
          </TooltipContent>
        </Tooltip>
      </div>
    </div>
  );

  if (!desktop) {
    return (
      <Sheet open={open} onOpenChange={(v) => { if (!v) onClose(); }}>
        <SheetContent side="bottom" showCloseButton={false} className={cn("max-h-[92dvh] gap-0 p-0", className)}>
          <div className="flex justify-center pt-2" aria-hidden>
            <span className="h-1 w-10 rounded-full bg-border" />
          </div>
          <SheetTitle className="sr-only">{typeof title === "string" ? title : "Details"}</SheetTitle>
          {typeof subtitle === "string" ? <SheetDescription className="sr-only">{subtitle}</SheetDescription> : null}
          {header}
          <div className="min-h-0 flex-1 overflow-y-auto px-4 py-4">{children}</div>
          {footer ? <div className="border-t px-4 py-3">{footer}</div> : null}
        </SheetContent>
      </Sheet>
    );
  }

  if (!open) return null;
  return (
    <aside
      className={cn(
        "sticky hidden w-full shrink-0 flex-col border-l bg-card lg:flex",
        "motion-safe:animate-in motion-safe:slide-in-from-right-4 motion-safe:fade-in motion-safe:duration-200 motion-safe:ease-out",
        WIDTH[width],
        className,
      )}
      style={{ top, height: `calc(100dvh - ${top})` }}
      aria-label={typeof title === "string" ? title : "Details"}
      data-side-panel
    >
      {header}
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-4">{children}</div>
      {footer ? <div className="border-t px-4 py-3">{footer}</div> : null}
    </aside>
  );
}

// The open item's id in the address bar (`?open=<id>` by default), so the
// panel survives a reload and a link lands on it. Static export: search
// params only, no dynamic segments - the same way lib/nav.ts builds hrefs.
// Returns [id, setId]; setId(null) closes. Uses router.replace so paging
// through items does not pile up history.
export function useOpenItem(param = "open"): [string | null, (id: string | null) => void] {
  const sp = useSearchParams();
  const router = useRouter();
  const pathname = usePathname();
  const id = sp.get(param);
  const set = useCallback(
    (next: string | null) => {
      const q = new URLSearchParams(sp.toString());
      if (next === null || next === "") q.delete(param);
      else q.set(param, next);
      const s = q.toString();
      router.replace(pathname + (s ? "?" + s : ""), { scroll: false });
    },
    [sp, router, pathname, param],
  );
  return [id, set];
}
