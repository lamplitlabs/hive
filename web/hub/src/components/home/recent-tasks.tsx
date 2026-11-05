"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { ChevronRightIcon } from "lucide-react";
import { EmptyState } from "@/components/pulse/panel";
import { ProjectLink, StatusBadge } from "@/components/pulse/who";
import { Ago, idleLine, TaskWorkerInline } from "@/components/tasks/task-work";
import { hrefs } from "@/lib/nav";
import { plural } from "@/lib/format";
import { taskProgress, type Task } from "@/lib/api";
import { cn } from "@/lib/utils";

// The tasks the owner gave last, as a dense list: Home only has to say
// where each one stands in a line, the Tasks page holds the full cards.
// A task that waits for the owner's review comes first and is marked
// with the marigold rule (the one warm colour means "you").
export function RecentTasks({ tasks, empty }: { tasks: Task[]; empty: string }) {
  const router = useRouter();
  if (!tasks.length) return <EmptyState>{empty}</EmptyState>;
  return (
    <ul className="-mx-2 grid gap-0.5" aria-label="Recent tasks">
      {tasks.map((t) => {
        const review = t.status === "review";
        const href = hrefs.task(t.id);
        return (
          // The row opens the task; the title is the real link (keyboard,
          // middle-click), the project name inside it is a link of its own,
          // so the row itself is a div that forwards a plain click.
          <li
            key={t.id}
            onClick={(e) => {
              if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
              if ((e.target as HTMLElement).closest("a")) return;
              router.push(href);
            }}
            className={cn(
              "group/row relative flex min-w-0 cursor-pointer items-start gap-3 rounded-xl px-2 py-2.5 transition-colors hover:bg-muted/60 has-[a:focus-visible]:bg-muted/60 motion-reduce:transition-none",
              review && "before:absolute before:inset-y-2 before:left-0 before:w-0.5 before:rounded-full before:bg-you before:content-['']",
            )}
          >
            <div className="min-w-0 flex-1">
              <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
                <Link href={href} aria-label={"Task: " + t.title} className="min-w-0 text-[15px] font-medium leading-snug text-foreground anywhere focus-visible:outline-none">
                  {t.title}
                </Link>
                <StatusBadge status={t.status} />
                <ProjectLink slug={t.project} className="font-normal text-muted-foreground" />
              </div>
              <p className="mt-1 text-[13px] leading-relaxed text-muted-foreground line-clamp-2">
                <TaskLine t={t} />
              </p>
            </div>
            <ChevronRightIcon className="mt-1 size-4 shrink-0 opacity-40 transition-[transform,opacity] group-hover/row:opacity-70 motion-safe:group-hover/row:translate-x-0.5 motion-reduce:transition-none" aria-hidden />
          </li>
        );
      })}
    </ul>
  );
}

// One line on where the task stands: who is on it, else why nobody is,
// else how far its steps got and when it last moved.
function TaskLine({ t }: { t: Task }) {
  const p = taskProgress(t);
  if (p.working.length) {
    const w = p.working[0];
    return (
      <>
        <TaskWorkerInline t={t} w={w} compact />
        {p.working.length > 1 ? " " + (p.working.length === 2 ? "And 1 more agent." : "And " + (p.working.length - 1) + " more agents.") : null}
      </>
    );
  }
  if (t.status === "review") {
    return (
      <>
        {"Waiting for your review. Updated "}
        <Ago d={t.updated_at} />
        {"."}
      </>
    );
  }
  const idle = idleLine(t);
  if (idle) return <>{idle}</>;
  return (
    <>
      {p.total ? plural(p.done, "step") + " of " + p.total + " done. " : null}
      {"Updated "}
      <Ago d={t.updated_at} />
      {"."}
    </>
  );
}
