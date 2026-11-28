"use client";

// The tasks table's columns (spec "Tasks" - List view): Title, Project,
// Status, Priority, Lead, Steps, Updated. Shared by the Tasks page and a
// project's Tasks tab; a row opens the task's conversation on the Chat page.
// Project and Lead hide on a narrow screen.

import { ProjectAvatar } from "@/components/hive/avatar";
import { type Column } from "@/components/hive/data-table";
import { Avatar, StatusBadge, When } from "@/components/hive/who";
import { taskProgress, type Task } from "@/lib/api";
import { agentName } from "@/lib/words";
import { PriorityMark, ProgressRing } from "./task-card";
import { stepsLine } from "./tasks-state";

export const TASK_COLUMNS = (showProject: boolean): Column<Task>[] => [
  {
    key: "title",
    label: "Title",
    sort: (t) => t.title,
    render: (t) => (
      <span className="block min-w-40">
        {/* Two lines at most, broken anywhere: a long title (or one long word) never widens the table. */}
        <span className="line-clamp-2 text-sm font-medium leading-5 anywhere" title={t.title}>
          {t.title}
        </span>
        <span className="block font-mono text-xs text-muted-foreground">{t.id}</span>
      </span>
    ),
  },
  ...(showProject
    ? [
        {
          key: "project",
          label: "Project",
          hide: "md",
          nowrap: true,
          sort: (t: Task) => t.project,
          render: (t: Task) => (
            <span className="inline-flex items-center gap-2">
              <ProjectAvatar slug={t.project} size={20} />
              <span className="truncate">{t.project}</span>
            </span>
          ),
        } as Column<Task>,
      ]
    : []),
  { key: "status", label: "Status", nowrap: true, sort: (t) => t.status, render: (t) => <StatusBadge status={t.status} size="sm" /> },
  { key: "priority", label: "Priority", nowrap: true, sort: (t) => t.priority, render: (t) => <PriorityMark p={t.priority} className="text-sm" /> },
  {
    key: "lead",
    label: "Lead",
    hide: "lg",
    nowrap: true,
    sort: (t) => t.lead,
    render: (t) =>
      t.lead ? (
        <span className="inline-flex items-center gap-2" title={t.lead}>
          <Avatar seed={t.lead} size={20} />
          <span className="truncate">{agentName(t.lead, t.project)}</span>
        </span>
      ) : (
        <span className="text-muted-foreground">{"\u2014"}</span>
      ),
  },
  {
    key: "steps",
    label: "Steps",
    nowrap: true,
    sort: (t) => {
      const p = taskProgress(t);
      return p.total ? p.done / p.total : -1;
    },
    render: (t) => {
      const p = taskProgress(t);
      if (!p.total) return <span className="text-muted-foreground">{t.status === "done" || t.status === "cancelled" ? "\u2014" : "None yet"}</span>;
      return (
        <span className="inline-flex items-center gap-1.5 tnum" aria-label={p.done + " of " + p.total + " steps done" + (p.failed ? ", " + p.failed + " failed" : "")}>
          <ProgressRing done={p.done} total={p.total} size={16} />
          <span>{stepsLine(p.done, p.total).replace(/ steps?$/, "")}</span>
          {p.failed ? <span className="text-bad">{", " + p.failed + " failed"}</span> : null}
        </span>
      );
    },
  },
  { key: "updated", label: "Updated", nowrap: true, sort: (t) => t.updated_at, render: (t) => <When d={t.updated_at} /> },
];
