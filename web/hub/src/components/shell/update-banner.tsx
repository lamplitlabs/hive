"use client";

import { useCallback, useSyncExternalStore } from "react";
import { ArrowUpCircleIcon, ExternalLinkIcon, XIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { UpdatePayload } from "@/lib/api";
import { readDismissed, updateBanner, writeDismissed, type UpdateStorage } from "@/lib/update-state";
import { useApi } from "@/lib/use-api";

// "PULSE vX is available" over every page (TASK-087), when the daemon's daily
// check (setting update.latest_url) found a release newer than this build.
// Dismissing hides it for that release in this browser; the next release
// shows again. With the setting empty (the default) nothing is ever shown.
// The words and the rules live in lib/update-state.ts.
const listeners = new Set<() => void>();
function storage(): UpdateStorage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}
function subscribe(cb: () => void) {
  listeners.add(cb);
  window.addEventListener("storage", cb);
  return () => {
    listeners.delete(cb);
    window.removeEventListener("storage", cb);
  };
}
const snapshot = () => readDismissed(storage()) ?? "";

export function UpdateBanner() {
  const { data } = useApi<UpdatePayload>("update", { refresh: 15 * 60_000 });
  const dismissed = useSyncExternalStore(subscribe, snapshot, () => "");
  const dismiss = useCallback((latest: string) => {
    writeDismissed(storage(), latest);
    for (const l of listeners) l();
  }, []);
  const b = updateBanner(data, dismissed || null);
  if (!b) return null;
  return (
    <div role="status" data-testid="update-banner" className="mb-4 flex flex-wrap items-center gap-x-3 gap-y-1.5 rounded-xl border border-ok/30 bg-ok/10 px-3 py-2 text-[13px] text-foreground">
      <ArrowUpCircleIcon aria-hidden className="size-4 shrink-0 text-ok" />
      <span className="min-w-0 flex-1">
        <span className="font-medium">{b.title}</span> <span className="text-muted-foreground">{b.detail}</span>
      </span>
      {b.href ? (
        <Button asChild size="xs" variant="outline">
          <a href={b.href} target="_blank" rel="noreferrer noopener">
            Release notes
            <ExternalLinkIcon aria-hidden data-icon="inline-end" />
          </a>
        </Button>
      ) : null}
      <Button size="icon-xs" variant="ghost" aria-label={`Hide until the next release after ${b.latest}`} title="Hide until the next release" onClick={() => dismiss(b.latest)}>
        <XIcon aria-hidden />
      </Button>
    </div>
  );
}
