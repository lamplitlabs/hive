"use client";

import { useCallback, useState, useSyncExternalStore } from "react";
import { ArrowUpCircleIcon, DownloadIcon, ExternalLinkIcon, XIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { daemonUpdateText, pendingUpdate } from "@/components/hive/daemon-control-state";
import { errorText, writes, type RunningPayload, type UpdatePayload } from "@/lib/api";
import { cn } from "@/lib/utils";
import { UPDATE_POLL_IDLE_MS, UPDATE_POLL_INSTALLING_MS, readDismissed, updateApplyView, updateBanner, writeDismissed, type UpdateStorage } from "@/lib/update-state";
import { useApi } from "@/lib/use-api";

// "Hive vX is available" over every page (TASK-087), when the daemon's daily
// check (setting update.latest_url) found a release newer than this build.
// Dismissing hides it for that release in this browser; the next release
// shows again. With the setting empty (the default) nothing is ever shown.
// On update.channel release it offers "Update now" (TASK-093): the hub
// installs the release the way `hive update` does, and the banner follows
// it to the runner's restart. The words and the rules live in
// lib/update-state.ts.
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

const TONE = { info: "text-muted-foreground", ok: "text-ok", bad: "text-bad" } as const;

export function UpdateBanner() {
  const [posting, setPosting] = useState(false);
  const [postError, setPostError] = useState("");
  const [fast, setFast] = useState(false);
  const { data, reload } = useApi<UpdatePayload>("update", { refresh: fast ? UPDATE_POLL_INSTALLING_MS : UPDATE_POLL_IDLE_MS });
  const installed = data?.apply?.state === "installed";
  // Once the release is in place, the runner's own words say what it waits for.
  const running = useApi<RunningPayload>(installed ? "running" : null, { refresh: 30000 });
  const dismissed = useSyncExternalStore(subscribe, snapshot, () => "");
  const dismiss = useCallback((latest: string) => {
    writeDismissed(storage(), latest);
    for (const l of listeners) l();
  }, []);
  const daemon = running.data?.daemon;
  const waitText = daemon ? daemonUpdateText(pendingUpdate(daemon)) : "";
  const view = updateApplyView(data, { posting, postError, waitText });
  // Ask every two seconds while the hub installs, every 15 minutes otherwise
  // (state adjusted while rendering, React's pattern for derived state).
  if (view.installing !== fast) setFast(view.installing);
  const apply = useCallback(async () => {
    setPosting(true);
    setPostError("");
    try {
      await writes.applyUpdate();
    } catch (e) {
      const t = errorText(e);
      setPostError(t.why || t.error);
    } finally {
      setPosting(false);
      reload();
    }
  }, [reload]);
  const b = updateBanner(data, dismissed || null);
  if (!b) return null;
  return (
    // One quiet line, not a coloured box: an update is good news, never an
    // alarm. The build this machine runs is in the tooltip, for whoever wants it.
    <div role="status" data-testid="update-banner" className="mb-4 flex flex-wrap items-center gap-x-3 gap-y-1.5 rounded-lg border bg-card px-3 py-1.5 text-[13px] text-foreground">
      <ArrowUpCircleIcon aria-hidden className="size-4 shrink-0 text-ok" />
      {/* The words keep a line's worth of room: on a phone the buttons wrap under them instead of squeezing them to a word a line. */}
      <span className="min-w-0 flex-1 basis-56 anywhere" title={b.detail}>
        <span className="font-medium">{b.title}</span> <span className="sr-only">{b.detail}</span>
        {view.line ? (
          <span data-testid="update-apply-line" className={cn("block leading-snug", view.tone ? TONE[view.tone] : "text-muted-foreground")}>
            {view.line}
          </span>
        ) : null}
      </span>
      {view.button ? (
        <Button size="xs" data-testid="update-apply" disabled={view.busy} title={view.buttonTitle} onClick={() => void apply()}>
          <DownloadIcon aria-hidden data-icon="inline-start" />
          {view.button}
        </Button>
      ) : null}
      {b.href ? (
        <Button asChild size="xs" variant="ghost" className="text-muted-foreground">
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
