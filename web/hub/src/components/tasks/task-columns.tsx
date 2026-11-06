"use client";

// The tasks table's columns, shared by the Tasks page and a project's Tasks
// tab; a row opens the task's conversation on the Chat page.

import { type Column } from "@/components/pulse/data-table";
import { ProjectLink, StatusBadge, When, Who } from "@/components/pulse/who";
import { taskProgress, type Task } from "@/lib/api";
import { priorityWord } from "@/lib/words";

export const TASK_COLUMNS = (showProject: boolean): Column<Task>[] => [
  {
    key: "title",
    label: "Task",
    sort: (t) => t.title,
    render: (t) => (
      <span className="block min-w-40">
        <span className="block font-medium break-words">{t.title}</span>
        <span className="block text-xs text-muted-foreground">{t.id}</span>
      </span>
    ),
  },
  ...(showProject ? [{ key: "project", label: "Project", sort: (t: Task) => t.project, render: (t: Task) => <ProjectLink slug={t.project} className="font-normal" /> } as Column<Task>] : []),
  { key: "status", label: "Status", nowrap: true, sort: (t) => t.status, render: (t) => <StatusBadge status={t.status} /> },
  { key: "priority", label: "Priority", sort: (t) => t.priority, hide: "sm", render: (t) => <span title={"Priority " + t.priority}>{priorityWord(t.priority)}</span> },
  {
    key: "working",
    label: "Working on it",
    hide: "md",
    render: (t) => {
      const workers = taskProgress(t).working;
      if (t.status === "done" || t.status === "cancelled") return <span className="text-muted-foreground">{"\u2014"}</span>;
      if (workers.length) return <span className="inline-flex flex-wrap items-center gap-1.5">{workers.map((w, i) => <Who key={w.job + i} row={{ agent: w.agent }} compact />)}</span>;
      return t.lead ? <span className="text-muted-foreground" title={"Lead agent: " + t.lead}>Nobody right now</span> : <span className="text-muted-foreground">No lead agent yet</span>;
    },
  },
  {
    key: "jobs",
    label: "Steps done",
    hide: "lg",
    sort: (t) => taskProgress(t).total,
    render: (t) => {
      const p = taskProgress(t);
      return p.total ? <span className="tnum whitespace-nowrap">{p.done + " of " + p.total}{p.failed ? <span className="text-bad">{", " + p.failed + " failed"}</span> : null}</span> : <span className="text-muted-foreground">{t.status === "done" || t.status === "cancelled" ? "\u2014" : "None yet"}</span>;
    },
  },
  { key: "updated", label: "Updated", nowrap: true, sort: (t) => t.updated_at, render: (t) => <When d={t.updated_at} /> },
];
