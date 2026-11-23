"use client";

import { useMemo } from "react";
import { RadioIcon } from "lucide-react";
import { EmptyState, Panel } from "./panel";
import { Avatar, When } from "./who";
import { useApi, useLiveEvents } from "@/lib/use-api";
import type { EventsPayload, StoreEvent } from "@/lib/api";
import { statusTone, type Tone } from "@/lib/format";
import { settingLabel } from "@/lib/setting-labels";
import { statusWord, threadName } from "@/lib/words";
import { MOVED, eventDetailText, eventVerb, rec, scalar, str } from "@/lib/event-words";
import { cn } from "@/lib/utils";

function actionTone(a: string): Tone {
  if (a === "mention" || a === "post.question" || a === "post.handoff") return "warn";
  if (a === "hook.failed") return "bad";
  if (a === "land.pushed" || a === "land.landed" || a === "land.batch") return "ok";
  if (a === "land.merged" || a === "review.merged") return "ok";
  if (a === "land.held" || a === "land.reverted") return "warn";
  // A pull request that needs someone: failing checks, a conflict, your call.
  if (a === "land.pr-checks" || a === "land.pr-conflict" || a === "land.missing" || a === "review.owner" || a === "review.changes") return "warn";
  // Housekeeping: a run's own cleanup, a worker retiring once its run is over
  // and the daemon's sweep are routine, not failures.
  if (a === "agent.cleanup" || a === "agent.retire" || a === "daemon.sweep" || a.endsWith(".swept")) return "neutral";
  if (a === "post" || a.startsWith("post.")) return "neutral";
  if (/\.(fail|failed|stop|stopped|deny|denied|cancelled|removed|pruned)$/.test(a)) return "bad";
  if (/\.(add|added|create|created)$/.test(a)) return "ok";
  if (/^decide\.|\.(decide|decided|approve|approved)$/.test(a)) return "info";
  return statusTone(a) === "neutral" ? "neutral" : statusTone(a);
}

const DOT: Record<Tone, string> = {
  neutral: "bg-muted-foreground/50",
  ok: "bg-ok",
  warn: "bg-warn",
  bad: "bg-bad",
  info: "bg-info",
  accent: "bg-accent-2",
  you: "bg-you",
};

// A value kept private ("[owner-private]") is not printed.
const hidden = (v: string) => /^\[.*\]$/.test(v);
const isSetting = (a: string) => a === "setting" || a === "settings.set";
const isMachine = (a: string) => a.startsWith("executor.");

function verb(e: StoreEvent): string {
  const d = rec(e.detail);
  const a = e.action;
  if (a === "task.status" && str(d?.to)) return MOVED[str(d?.to)] ?? "moved a task to " + statusWord(str(d?.to)).toLowerCase();
  if (isSetting(a)) {
    const value = scalar(e.detail) || scalar(d?.value);
    if (!e.subject) return "changed a setting";
    return "changed \u201c" + settingLabel(e.subject) + "\u201d" + (value && !hidden(value) ? " to " + value : "");
  }
  return eventVerb(e);
}

function actorName(e: StoreEvent): string {
  if (e.action === "mention") return e.subject || "An agent";
  const a = (e.actor || "").trim();
  if (!a) return "Hive";
  if (a === "human" || a === "owner") return "You";
  return a;
}

// The second line: what the event was about, in its own words when it has
// any, else where it happened (on the all-projects feed).
function detailText(e: StoreEvent, project?: string): string {
  const d = e.detail;
  if (d === null || d === undefined || d === "") return "";
  // A setting's value is already in the sentence.
  if (isSetting(e.action)) return "";
  if (typeof d !== "object") return scalar(d);
  const o = rec(d);
  if (!o) return "";
  const where = !project && str(o.project) ? str(o.project) : "";
  if ((e.action === "post" || e.action.startsWith("post.")) && str(o.thread)) return "In \u201c" + threadName(str(o.thread)) + "\u201d" + (where ? " on " + where : "");
  // Events whose own fields make the line: a red base's failing checks, a
  // slow stage's name and time (lib/event-words.ts).
  const own = eventDetailText(e);
  if (own) return own;
  for (const k of ["summary", "title", "reason", "words", "note", "text"]) if (str(o[k])) return str(o[k]);
  return where ? "In " + where : "";
}

// The subject rides along in brackets when people know it by name (TASK-001,
// ADR-001, EVO-constitution, a project). Post and job ids, message ids and
// timestamps stay in the tooltip; some actions already name it in the verb.
function subjectText(e: StoreEvent, project?: string): string {
  const s = (e.subject || "").trim();
  if (!s || s === project) return "";
  if (e.action === "mention" || e.action === "decide" || e.action === "message.sent" || e.action === "agent.retire" || e.action === "daemon.slow-stage" || isSetting(e.action) || isMachine(e.action)) return "";
  if (/^\d{8}T\d{6}Z/.test(s) || /^(job|msg)-/i.test(s) || /^[0-9a-f]{12,}$/i.test(s)) return "";
  return s;
}

// The live timeline: the last events from the store, newest first, merged
// from the initial GET and everything the SSE stream delivered since. With a
// project, both halves are that project's: the GET asks the hub for it and
// the stream (one connection for the whole hub) is filtered here.
export function LiveFeed({ project, limit = 40, title = "What just happened", height }: { project?: string; limit?: number; title?: string; height?: number }) {
  const { events: live, state } = useLiveEvents();
  const { data, error, loading } = useApi<EventsPayload>("events?limit=" + limit + (project ? "&project=" + encodeURIComponent(project) : ""), { refresh: 30000 });
  const rows = useMemo(() => {
    const seen = new Set<number>();
    const out: StoreEvent[] = [];
    for (const e of [...live, ...(data?.events ?? [])]) {
      if (seen.has(e.id)) continue;
      // Step events belong to the Harness page; here they would bury everything else.
      if (e.action === "harness.step") continue;
      if (project && !matchesProject(e, project)) continue;
      seen.add(e.id);
      out.push(e);
    }
    return out.sort((a, b) => b.id - a.id).slice(0, limit);
  }, [live, data, project, limit]);
  return (
    <Panel
      title={title}
      icon={RadioIcon}
      actions={
        <span className="inline-flex items-center gap-1.5 text-xs" title={state === "live" ? "New events arrive as they happen" : state === "polling" ? "The live stream is down; the list refreshes every 30 seconds" : "Connecting to the live stream"}>
          <span className={cn("size-2 rounded-full", state === "live" ? "bg-ok" : state === "polling" ? "bg-warn" : "bg-muted-foreground")} aria-hidden />
          {state === "live" ? "Live" : state === "polling" ? "Refreshing every 30 s" : "Connecting"}
        </span>
      }
      dense
    >
      {loading && !data ? (
        <div className="px-4 py-3 text-sm text-muted-foreground sm:px-5">Loading what happened…</div>
      ) : error && !rows.length ? (
        <div className="px-4 py-3 text-sm text-muted-foreground sm:px-5">{"The activity list didn't load (" + error + "). The rest of the page still refreshes on its own."}</div>
      ) : rows.length === 0 ? (
        <div className="p-4">
          <EmptyState>{project ? "Nothing has happened in " + project + " yet. New posts, decisions and task updates show up here as they happen." : "Nothing has happened yet. New posts, decisions and task updates show up here as they happen."}</EmptyState>
        </div>
      ) : (
        // A list that scrolls takes the keyboard's focus, so the arrow keys can scroll it.
        <ul className="scroll-thin divide-y divide-border/70 overflow-y-auto focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring" style={height ? { maxHeight: height } : undefined} tabIndex={height ? 0 : undefined} aria-label={height ? "Events, newest first" : undefined}>
          {rows.map((e) => {
            const who = actorName(e);
            const text = detailText(e, project);
            const subject = subjectText(e, project);
            const tone = actionTone(e.action);
            return (
              <li key={e.id} className="flex min-w-0 items-start gap-3 px-4 py-2.5 sm:px-5" title={[e.action, e.subject].filter(Boolean).join(" ")}>
                <Avatar seed={who} size={24} human={who === "You"} className="mt-0.5" />
                <div className="min-w-0 flex-1">
                  <p className="text-sm leading-snug anywhere">
                    <span className="font-medium text-foreground">{who}</span> <span className="text-muted-foreground">{verb(e)}</span>
                    {subject ? <span className="text-muted-foreground">{" (" + subject + ")"}</span> : null}
                    {tone !== "neutral" ? <span className={cn("ml-1.5 inline-block size-1.5 -translate-y-px rounded-full align-middle", DOT[tone])} aria-hidden /> : null}
                  </p>
                  {text ? <p className="mt-0.5 line-clamp-2 text-[13px] leading-snug text-muted-foreground anywhere">{text}</p> : null}
                </div>
                <When d={e.at} className="shrink-0 pt-0.5 text-xs text-muted-foreground" />
              </li>
            );
          })}
        </ul>
      )}
    </Panel>
  );
}

// The same rule the hub applies to GET /events?project= (bus.Filter): an
// event belongs to a project when its detail names it or its subject is the
// project itself (project.add, project.removed). Events that name no project
// - settings, executors, decisions, jobs of other projects - are not its.
function matchesProject(e: StoreEvent, project: string): boolean {
  if (e.subject === project) return true;
  const d = e.detail;
  return typeof d === "object" && d !== null && (d as Record<string, unknown>).project === project;
}
