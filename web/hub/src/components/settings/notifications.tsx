"use client";

import { useEffect, useState } from "react";
import { BellRingIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Notice } from "@/components/hive/panel";
import { appNotifier, notifyPermission, setNotifyPrefs, tryChime, useNotifyPrefs } from "@/components/shell/owner-notifier";
import { notifyOn, notifyStatus, type NotifyPrefs } from "@/lib/owner-notify";
import { cn } from "@/lib/utils";

// Settings → Notifications: the owner's own choices for this browser (or
// Hive.app), kept in localStorage like the theme - not a Hive setting,
// since another browser or another person on the console decides for
// itself. The notifier in the shell reads the same choices.

const ROWS: { key: keyof NotifyPrefs; title: string; help: string }[] = [
  { key: "waits", title: "When something waits for you", help: "A question, a decision, a proposal, a machine request or suggested work that needs your call; a task whose lead waits for your reply; an agent in a session that asks you something or asks to run a tool." },
  { key: "finished", title: "When a task or session finishes", help: "A task done, a session its agent finished, and an agent's reply that ended its turn. Not the session you are looking at, and not one you closed yourself." },
  { key: "sound", title: "Play a sound", help: "A short chime with each notification. In Hive.app, the Mac's notification sound." },
];

export function NotificationsCard() {
  const prefs = useNotifyPrefs();
  const [permission, setPermission] = useState("default");
  const [app, setApp] = useState(false);
  const [supported, setSupported] = useState(true);
  useEffect(() => {
    let live = true;
    const read = () =>
      void notifyPermission(false).then((p) => {
        if (!live) return;
        setPermission(p);
        setApp(!!appNotifier());
        setSupported(p !== "unsupported");
      });
    read();
    // The owner may allow or block notifications in the browser's own
    // settings while the page is open; look again when they come back.
    window.addEventListener("focus", read);
    return () => {
      live = false;
      window.removeEventListener("focus", read);
    };
  }, []);

  const change = async (key: keyof NotifyPrefs, value: boolean) => {
    const next = { ...prefs, [key]: value };
    setNotifyPrefs(next);
    // Turning one on asks, from this click, whether the console may notify.
    if (value && key !== "sound" && notifyOn(next)) setPermission(await notifyPermission(true));
    if (key === "sound" && value) tryChime();
  };

  const on = notifyOn(prefs);
  const status = notifyStatus({ app, supported, permission, on });
  return (
    <div className="min-w-0 overflow-hidden rounded-lg border bg-card">
      <div className="divide-y">
        {ROWS.map((r) => {
          const checked = prefs[r.key];
          const disabled = r.key === "sound" && !on;
          return (
            <div key={r.key} className="grid gap-3 px-4 py-4 sm:px-5 md:grid-cols-[minmax(0,1fr)_auto] md:items-start md:gap-x-8">
              <div className="flex min-w-0 flex-col gap-1">
                <span className="text-sm font-medium leading-snug text-foreground anywhere">{r.title}</span>
                <p className="text-[13px] leading-relaxed text-muted-foreground anywhere">{r.help}</p>
              </div>
              <div className="flex min-h-9 items-center gap-2.5 md:justify-end">
                <Switch id={"notify-" + r.key} checked={checked} disabled={disabled} onCheckedChange={(v) => void change(r.key, v)} aria-label={r.title} />
                <label htmlFor={"notify-" + r.key} className={cn("text-sm", checked && !disabled ? "font-medium text-foreground" : "text-muted-foreground")}>
                  {checked ? "On" : "Off"}
                </label>
              </div>
            </div>
          );
        })}
      </div>
      <div className="grid gap-3 border-t px-4 py-4 sm:px-5">
        <Notice tone={status.tone}>{status.text}</Notice>
        <p className="text-[13px] leading-relaxed text-muted-foreground">
          These choices belong to this {app ? "app" : "browser"}, not to Hive: another browser decides for itself. A notification comes only while a console {app ? "window" : "tab"} is open, and each thing notifies once.
        </p>
        {on && prefs.sound ? (
          <div>
            <Button variant="outline" size="sm" onClick={() => tryChime()}>
              <BellRingIcon aria-hidden /> Play the sound
            </Button>
          </div>
        ) : null}
      </div>
    </div>
  );
}
