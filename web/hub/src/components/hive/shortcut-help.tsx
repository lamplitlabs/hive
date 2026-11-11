"use client";

import { useEffect, useRef } from "react";
import { useRouter } from "next/navigation";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { cn } from "@/lib/utils";
import { CHORD_MS, groupedShortcuts, isCommandKey, keyChips, matchChord, SHORTCUTS, type Shortcut } from "@/lib/shortcuts";

export { SHORTCUTS, matchChord, keyChips } from "@/lib/shortcuts";

// A key as a chip.
export function Kbd({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <kbd data-slot="kbd" className={cn("inline-flex h-5 min-w-5 items-center justify-center rounded border bg-muted px-1 font-mono text-xs font-medium text-muted-foreground", className)}>
      {children}
    </kbd>
  );
}

// "G then H", "J or K", "⌘ K" as chips.
export function Keys({ keys, className }: { keys: string; className?: string }) {
  return (
    <span className={cn("inline-flex items-center gap-1", className)}>
      {keyChips(keys).map((c, i) => (c.kind === "key" ? <Kbd key={i}>{c.text}</Kbd> : <span key={i} className="text-xs text-muted-foreground">{c.text}</span>))}
    </span>
  );
}

// The "?" dialog: every shortcut, grouped by where it works.
export function ShortcutHelp({ open, onOpenChange, shortcuts = SHORTCUTS }: { open: boolean; onOpenChange: (open: boolean) => void; shortcuts?: Shortcut[] }) {
  const groups = groupedShortcuts(shortcuts);
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent placement="float" className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Keyboard shortcuts</DialogTitle>
          <DialogDescription>Press ? anywhere to see this list.</DialogDescription>
        </DialogHeader>
        <div className="grid gap-4">
          {groups.map((g) => (
            <section key={g.where} aria-labelledby={"shortcuts-" + g.where.replace(/\W+/g, "-").toLowerCase()}>
              <h3 id={"shortcuts-" + g.where.replace(/\W+/g, "-").toLowerCase()} className="mb-1.5 text-xs font-medium uppercase tracking-wide text-muted-foreground">
                {g.where}
              </h3>
              <dl className="divide-y divide-border rounded-lg border">
                {g.items.map((s) => (
                  <div key={s.keys} className="flex items-center justify-between gap-4 px-3 py-1.5 text-sm">
                    <dt className="min-w-0 truncate text-foreground">{s.what}</dt>
                    <dd className="m-0 shrink-0">
                      <Keys keys={s.keys} />
                    </dd>
                  </div>
                ))}
              </dl>
            </section>
          ))}
        </div>
      </DialogContent>
    </Dialog>
  );
}

function inField(t: EventTarget | null): boolean {
  const el = t as HTMLElement | null;
  if (!el || typeof el.tagName !== "string") return false;
  const tag = el.tagName.toLowerCase();
  if (tag === "input" || tag === "textarea" || tag === "select") return true;
  return el.isContentEditable || !!el.closest?.("[contenteditable=''],[contenteditable=true],[contenteditable=plaintext-only]");
}

// Listens for the global shortcuts on the document: G-chords navigate (the
// pending G lives one second), ⌘K / ⌘J / ? call back to the shell, which
// opens Search, Ask or the help. Mount once, in the shell.
export function useGlobalShortcuts({ onSearch, onAsk, onHelp, enabled = true }: { onSearch?: () => void; onAsk?: () => void; onHelp?: () => void; enabled?: boolean }) {
  const router = useRouter();
  const pending = useRef<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const handlers = useRef({ onSearch, onAsk, onHelp });
  useEffect(() => {
    handlers.current = { onSearch, onAsk, onHelp };
  }, [onSearch, onAsk, onHelp]);

  useEffect(() => {
    if (!enabled) return;
    const clear = () => {
      pending.current = null;
      if (timer.current) clearTimeout(timer.current);
      timer.current = null;
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.isComposing || e.repeat) return;
      if (e.altKey) return;
      const meta = isCommandKey(e, typeof navigator !== "undefined" ? navigator.platform || "" : "");
      if (e.shiftKey && !meta && e.key !== "?") {
        // Shift+letter is a capital, not a chord, except "?" which needs Shift.
        if (e.key.length === 1) {
          clear();
          return;
        }
      }
      const r = matchChord(pending.current, e.key, meta, inField(e.target));
      if (!r) {
        if (pending.current && e.key.length === 1) clear();
        return;
      }
      if ("pending" in r) {
        clear();
        pending.current = r.pending;
        timer.current = setTimeout(clear, CHORD_MS);
        return;
      }
      clear();
      e.preventDefault();
      if ("go" in r) {
        router.push(r.go);
        return;
      }
      const h = handlers.current;
      if (r.open === "search") h.onSearch?.();
      else if (r.open === "ask") h.onAsk?.();
      else if (r.open === "help") h.onHelp?.();
    };
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
      clear();
    };
  }, [enabled, router]);
}
