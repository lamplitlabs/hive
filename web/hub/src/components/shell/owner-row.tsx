"use client";

import Link from "next/link";
import { useState } from "react";
import { SettingsIcon } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { OwnerAvatar } from "@/components/hive/avatar";
import { showApiError } from "@/components/hive/confirm";
import { writes, type Org } from "@/lib/api";
import { hrefs } from "@/lib/nav";
import { invalidate, useApi } from "@/lib/use-api";
import { cn } from "@/lib/utils";
import { RailTip } from "./rail-tip";
import { AwakeStatus } from "./awake";
import { ThemeToggle } from "./theme-toggle";
import { SETTINGS_ITEM } from "./nav-items";

// The foot of the sidebar (spec "App shell" - Owner row): your avatar and
// name, a Here / Away switch (owner.presence - it replaces Home's away
// banner), then Settings, the keep-awake cup and the theme. On the rail the
// pieces stack. The avatar keeps its room when the row is full: the name
// folds away first (a container query on the link), never under the switch.

const PRESENCE_PATHS = ["org", "settings", "events", "inbox"];
const invalidatePresence = () => invalidate((p) => PRESENCE_PATHS.some((path) => p === path || p.startsWith(path + "?") || p.startsWith(path + "/")));

export function PresenceSwitch({ className, size = "sm" }: { className?: string; size?: "sm" | "default" }) {
  const { data: org } = useApi<Org>("org");
  const [busy, setBusy] = useState(false);
  const here = org?.owner_presence !== "away";
  const canEdit = org?.can_decide !== false;
  const set = async (present: boolean) => {
    setBusy(true);
    const id = toast.loading(present ? "Marking you as here…" : "Marking you as away…");
    try {
      await writes.setSetting("owner.presence", present ? "present" : "away");
      toast.success(present ? "Welcome back. Hive asks again before it adds a tool, a skill or a lead." : "You're away. Hive adds the tools and skills a project needs on its own and tells you on the board; hard stops still wait.", { id });
      invalidatePresence();
    } catch (e) {
      toast.dismiss(id);
      showApiError(e, "Hive couldn't change your presence. Try again, or change it in Settings.");
    } finally {
      setBusy(false);
    }
  };
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <label className={cn("inline-flex cursor-pointer select-none items-center gap-2 text-xs text-muted-foreground", className)}>
          <span className={cn("tnum", here ? "text-foreground" : "text-warn")}>{here ? "Here" : "Away"}</span>
          <Switch size={size} checked={here} disabled={busy || !org || !canEdit} onCheckedChange={(v) => void set(v)} aria-label={here ? "You are here. Switch off to mark yourself away." : "You are away. Switch on to mark yourself here."} />
        </label>
      </TooltipTrigger>
      <TooltipContent side="top" className="max-w-64">
        {here ? "Here: Hive asks before it adds a tool, a skill or a lead. Each project's lead decides routine asks either way; hard stops wait for you." : "Away: Hive adds the tools and skills a project needs on its own and tells you on the board. Each project's lead decides routine asks either way; hard stops still wait for you."}
      </TooltipContent>
    </Tooltip>
  );
}

export function OwnerRow({ active }: { active: boolean }) {
  const { data: org } = useApi<Org>("org");
  const name = (org?.owner_name || "").trim() || "You";
  return (
    <div className="flex items-center gap-1 px-1 collapsed:flex-col collapsed:px-0">
      <RailTip label={name} hint="Your name and presence; change them in Settings">
        <Link href={hrefs.settings("owner")} className="@container/owner flex min-w-8 flex-1 items-center gap-2 rounded-md px-1 py-1 text-[13px] font-medium text-foreground hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring collapsed:flex-none collapsed:px-0 collapsed:[container-type:normal]" >
          {/* Named by its words: "Owner: " for a screen reader, then the name. The face repeats the name, so it is for the eye only. */}
          <span aria-hidden className="contents">
            <OwnerAvatar name={name} size={24} />
          </span>
          <span className="sr-only">Owner: </span>
          <span className="min-w-0 truncate @max-[4rem]/owner:sr-only collapsed:sr-only">{name}</span>
        </Link>
      </RailTip>
      <PresenceSwitch className="mr-1 collapsed:hidden" />
      <RailTip label={SETTINGS_ITEM.label} hint={SETTINGS_ITEM.hint}>
        <Button asChild variant="ghost" size="icon-sm" className={cn("text-muted-foreground hover:text-foreground", active && "bg-accent text-link")}>
          <Link href={SETTINGS_ITEM.href} aria-label="Settings" aria-current={active ? "page" : undefined}>
            <SettingsIcon aria-hidden className="size-[18px]" strokeWidth={1.75} />
          </Link>
        </Button>
      </RailTip>
      {/* The keep-awake cup: only while the setting is on (steaming while held). */}
      <AwakeStatus />
      <ThemeToggle />
    </div>
  );
}
