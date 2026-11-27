"use client";

import { useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Notice } from "@/components/hive/panel";
import { writes, type Compute } from "@/lib/api";
import { invalidate, useApi } from "@/lib/use-api";
import { refileOffer } from "./compute-state";

// TASK-165 item 2: after a limit change, the requests a limit refused today are offered again -
// Rent them now (as the owner's own, which no limit refuses) or leave them. Nothing shows when
// none was refused today.
export function RefileOffer({ canEdit }: { canEdit: boolean }) {
  const q = useApi<Compute>("compute", { refresh: 30000 });
  const [busy, setBusy] = useState(false);
  const refused = q.data?.refused_today ?? [];
  const words = refileOffer(refused.length);
  if (!words) return null;
  const rent = async () => {
    setBusy(true);
    try {
      const r = await writes.refileToday();
      toast.success(r.filed.length === 1 ? "Asked for the machine again." : "Asked for " + r.filed.length + " machines again.");
      invalidate("compute");
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="px-4 pt-4 sm:px-5">
      <Notice tone="info">
        <span className="flex flex-wrap items-center gap-3">
          <span>{words}</span>
          <Button size="sm" disabled={!canEdit || busy} onClick={() => void rent()}>
            {refused.length === 1 ? "Rent it now" : "Rent them now"}
          </Button>
        </span>
      </Notice>
    </div>
  );
}
