"use client";

import Link from "next/link";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/hive/confirm";
import { ListRow, ListRows } from "@/components/hive/list-row";
import { ServiceDialog } from "@/components/hive/service";
import { StatusIcon } from "@/components/hive/status-badge";
import { Mono } from "@/components/hive/who";
import { writes, type ServicePayload } from "@/lib/api";
import { hrefs } from "@/lib/nav";
import { invalidate } from "@/lib/use-api";
import { cn } from "@/lib/utils";
import type { AttentionFix, AttentionRow } from "./home-state";

// The attention strip (spec "Home" 2): one row per thing that is wrong, with
// its icon, one sentence and the button that fixes it. Shown only when
// something is wrong; the page computes the rows with attentionRows(). Each
// row takes its own tone: red for what stops all work (the runner), orange
// for what needs a step (an expired sign-in, a page that didn't load).
export function AttentionStrip({ rows, service, onRetry }: { rows: AttentionRow[]; service: ServicePayload | undefined; onRetry: () => void }) {
  if (!rows.length) return null;
  const failed = rows.some((r) => r.tone === "failed");
  return (
    <section aria-label="Needs attention" className="min-w-0">
      <ListRows ariaLabel="Problems and their fixes" className={failed ? "border-status-failed/30" : "border-status-warning/30"}>
        {rows.map((row) => (
          // The fix sits in the row's status slot, which is always visible: a
          // problem's button does not wait for a hover the way row actions do.
          <ListRow key={row.id} lead={<StatusIcon status={row.tone} className="size-5" />} title={<span title={row.why || undefined}>{row.text}</span>} status={<FixButton fix={row.fix} label={row.action} why={row.why} service={service} onRetry={onRetry} />} ariaLabel={row.text + ". " + row.action} className={cn("py-2", row.tone === "failed" ? "bg-status-failed-soft/40" : "bg-status-warning-soft/40")}>
            {row.why || undefined}
          </ListRow>
        ))}
      </ListRows>
    </section>
  );
}

// The button that applies one fix, with the dialog it needs. The fixes reuse
// the writes the runner controls make: Resume is the same confirm as
// Settings' (stop = 0), Start runner opens the service dialog that installs
// or starts the background service, Retry reloads the inputs that failed,
// and the rest are links. The setup card's "Keep Hive running" step uses
// the same button.
export function FixButton({ fix, label, why, service, onRetry, size = "sm", variant }: { fix: AttentionFix; label: string; why?: string; service: ServicePayload | undefined; onRetry?: () => void; size?: "xs" | "sm" | "default"; variant?: "default" | "outline" }) {
  const [open, setOpen] = useState(false);
  if (fix.kind === "resume") {
    return (
      <>
        <Button size={size} variant={variant} disabled={!fix.enabled} title={why || undefined} onClick={() => setOpen(true)}>
          {label}
        </Button>
        <ConfirmDialog
          open={open}
          onOpenChange={setOpen}
          title="Resume the runner?"
          description={
            <>
              Agents can start new work again. This is the same as <Mono>hive start</Mono>. It doesn&apos;t start a runner that isn&apos;t running or remove any STOP file, and your limits, project stops and approvals still apply.
            </>
          }
          action="Resume"
          success="Resumed. The runner picks this up on its next check-in."
          onConfirm={() => writes.setSetting("stop", "0")}
          invalidates={(p) => ["settings", "runner", "service", "running", "compute", "org", "events"].some((path) => p === path || p.startsWith(path + "?"))}
        />
      </>
    );
  }
  if (fix.kind === "service") {
    if (!service)
      return (
        <Button size={size} variant={variant ?? "outline"} asChild>
          <Link href={hrefs.settings("daemon")}>{label}</Link>
        </Button>
      );
    return (
      <>
        <Button size={size} variant={variant} onClick={() => setOpen(true)}>
          {label}
        </Button>
        <ServiceDialog open={open} onOpenChange={setOpen} payload={service} action={fix.action} />
      </>
    );
  }
  if (fix.kind === "retry") {
    return (
      <Button size={size} variant={variant ?? "outline"} onClick={onRetry ?? retryInputs}>
        {label}
      </Button>
    );
  }
  return (
    <Button size={size} variant={variant} asChild>
      <Link href={fix.href}>{label}</Link>
    </Button>
  );
}

// The page's Retry: every input the inbox reads, reloaded together.
export function retryInputs() {
  invalidate();
}
