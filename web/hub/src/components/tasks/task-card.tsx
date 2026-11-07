"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState } from "react";
import { ArrowDownIcon, ArrowUpIcon, BanIcon, CircleDashedIcon, Columns2Icon, MessageCircleQuestionIcon, MessagesSquareIcon, MinusIcon, MoreHorizontalIcon, PanelRightOpenIcon, PencilIcon, type LucideIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { ProjectAvatar } from "@/components/pulse/avatar";
import { ConfirmDialog } from "@/components/pulse/confirm";
import { CardGrid, ItemCard, ItemCardActions, ItemCardTitle } from "@/components/pulse/item-card";
import { EmptyState } from "@/components/pulse/panel";
import { Avatar, StatusBadge } from "@/components/pulse/who";
import { agentName, priorityWord } from "@/lib/words";
import { hrefs } from "@/lib/nav";
import { taskProgress, writes, type Task } from "@/lib/api";
import { cn } from "@/lib/utils";
import { AmendTaskDialog } from "./task-dialogs";
import { Ago, idleLine, TaskWorkingLine } from "./task-work";
import { priorityIcon, ringDash, stepsLine } from "./tasks-state";

const isClosed = (t: Task) => t.status === "done" || t.status === "cancelled";

// The cancel confirmation, shared by the card menu and the task sheet.
export function CancelTaskDialog({ t, open, onOpenChange }: { t: Task; open: boolean; onOpenChange: (o: boolean) => void }) {
  return (
    <ConfirmDialog
      open={open}
      onOpenChange={onOpenChange}
      title="Cancel this task?"
      description={
        <>
          <span className="block font-medium text-foreground anywhere">{t.title}</span>
          <span className="mt-2 block">The lead agent stops working on it and any step still waiting to start is withdrawn. A step that is already running finishes first. You can&apos;t undo this; give a new task instead.</span>
        </>
      }
      action="Cancel the task"
      dismiss="Keep the task"
      destructive
      words={{ label: "Why are you cancelling it?", placeholder: "For example: no longer needed, or replaced by TASK-012", required: true }}
      success="Task cancelled. The lead agent has been told."
      onConfirm={(reason) => writes.cancelTask(t.id, reason)}
      invalidates={(p) => p.startsWith("tasks") || p.startsWith("events")}
    />
  );
}

// The owner's moves on a task - talk to the lead, change the task, cancel
// it - without opening it first. At most two buttons show; the rest sit
// under "…". Edit and Cancel confirm. `chat={false}` drops the Session
// button when the card already offers one. A done task's reads "Report":
// its page opens with the report PULSE built beside the session.
export function TaskQuickActions({ t, onOpen, size = "xs", showOpen, chat = true }: { t: Task; onOpen?: () => void; size?: "xs" | "sm"; showOpen?: boolean; chat?: boolean }) {
  const [amend, setAmend] = useState(false);
  const [cancel, setCancel] = useState(false);
  const closed = isClosed(t);
  // Two visible at most: Open takes a slot when asked for, Session the other.
  const open = Boolean(showOpen && onOpen);
  return (
    <>
      {open ? (
        <Button size={size} variant="outline" onClick={onOpen}>
          <PanelRightOpenIcon /> Open
        </Button>
      ) : null}
      {chat ? (
        <Button size={size} variant="outline" asChild>
          {t.status === "done" ? (
            <Link href={hrefs.task(t.id)} aria-label={"The report of " + t.title + ", beside its session"} data-report-link>
              <Columns2Icon /> Report
            </Link>
          ) : (
            <Link href={hrefs.task(t.id)} aria-label={"Session with the lead agent about " + t.title}>
              <MessagesSquareIcon /> Session
            </Link>
          )}
        </Button>
      ) : null}
      <DropdownMenu>
        <Tooltip>
          <TooltipTrigger asChild>
            <DropdownMenuTrigger asChild>
              <Button size={size === "xs" ? "icon-xs" : "icon-sm"} variant="ghost" aria-label={"More actions for " + t.title}>
                <MoreHorizontalIcon />
              </Button>
            </DropdownMenuTrigger>
          </TooltipTrigger>
          <TooltipContent>More actions</TooltipContent>
        </Tooltip>
        <DropdownMenuContent align="end" className="w-48">
          <DropdownMenuItem disabled={closed} onSelect={() => setAmend(true)}>
            <PencilIcon /> Edit the task
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem variant="destructive" disabled={closed} onSelect={() => setCancel(true)}>
            <BanIcon /> Cancel the task
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      {amend ? <AmendTaskDialog task={t} open={amend} onOpenChange={setAmend} /> : null}
      <CancelTaskDialog t={t} open={cancel} onOpenChange={setCancel} />
    </>
  );
}

// A small ring with the done arc drawn in ink: the card's progress glyph.
// The words beside it carry the meaning; the ring is decoration for them.
export function ProgressRing({ done, total, size = 20, className }: { done: number; total: number; size?: 16 | 20 | 24; className?: string }) {
  const stroke = 2;
  const r = (size - stroke) / 2;
  const { dash } = ringDash(done, total, r);
  const full = total > 0 && done >= total;
  return (
    <svg width={size} height={size} viewBox={"0 0 " + size + " " + size} className={cn("shrink-0 -rotate-90", className)} aria-hidden>
      <circle cx={size / 2} cy={size / 2} r={r} fill="none" stroke="currentColor" strokeWidth={stroke} className="text-border" />
      <circle cx={size / 2} cy={size / 2} r={r} fill="none" stroke="currentColor" strokeWidth={stroke} strokeLinecap="round" strokeDasharray={dash} className={cn("transition-[stroke-dasharray] duration-300 motion-reduce:transition-none", full ? "text-ok" : "text-primary")} />
    </svg>
  );
}

// "1 of 2 steps" beside its ring; a failed step is said in words. The old
// bar stays one prop away (`bar`) for the task sheet's wide layout.
export function TaskProgressBar({ t, className, bar }: { t: Task; className?: string; bar?: boolean }) {
  const p = taskProgress(t);
  if (!p.total) return null;
  const label = p.done + " of " + p.total + " steps done";
  if (bar)
    return (
      <div className={cn("grid gap-1.5", className)}>
        <div className="flex flex-wrap items-center justify-between gap-x-3 text-[13px] text-muted-foreground">
          <span>{stepsLine(p.done, p.total) + " done"}</span>
          {p.failed ? <span className="text-bad">{p.failed === 1 ? "1 step failed" : p.failed + " steps failed"}</span> : null}
        </div>
        <Progress value={Math.min(100, (100 * p.done) / Math.max(1, p.total))} className="h-1.5" aria-label={label} />
      </div>
    );
  return (
    <span className={cn("inline-flex min-w-0 items-center gap-1.5 text-[13px] text-muted-foreground", className)} role="img" aria-label={label + (p.failed ? ", " + p.failed + " failed" : "")}>
      <ProgressRing done={p.done} total={p.total} size={16} />
      <span className="tnum whitespace-nowrap">{stepsLine(p.done, p.total)}</span>
      {p.failed ? <span className="text-bad whitespace-nowrap">{p.failed === 1 ? "1 failed" : p.failed + " failed"}</span> : null}
    </span>
  );
}

const PRIORITY_ICON: Record<ReturnType<typeof priorityIcon>, LucideIcon> = { up: ArrowUpIcon, flat: MinusIcon, down: ArrowDownIcon };

// The priority as its mark and its word ("High"); urgent reads in red.
export function PriorityMark({ p, wordless, className }: { p: number; wordless?: boolean; className?: string }) {
  const Icon = PRIORITY_ICON[priorityIcon(p)];
  const word = priorityWord(p);
  return (
    <span className={cn("inline-flex items-center gap-1 whitespace-nowrap", p >= 50 ? "text-bad" : p >= 30 ? "text-foreground" : "text-muted-foreground", className)} title={"Priority " + word.toLowerCase() + " (" + p + ")"}>
      <Icon className="size-3.5 shrink-0" strokeWidth={2} aria-hidden />
      <span className={cn(wordless && "sr-only")}>{word}</span>
    </span>
  );
}

// The ask on a task in review: the lead got stuck and waits in the chat,
// where its last message offers answers to tap.
// `onChat` opens the conversation in place (the task sheet) instead of
// following the link to the task's conversation on the Chat page.
export function TaskNeedsYou({ t, className, onChat }: { t: Task; className?: string; onChat?: () => void }) {
  return (
    <div className={cn("surface-you flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2 rounded-lg border px-3 py-2.5", className)}>
      <p className="flex min-w-0 flex-1 basis-52 items-start gap-2 text-sm leading-relaxed text-foreground">
        <MessageCircleQuestionIcon className="mt-0.5 size-4 shrink-0 text-you-ink" aria-hidden />
        <span className="min-w-0 anywhere">The lead agent stopped and waits for your decision. Pick an answer in the session, or write one.</span>
      </p>
      {onChat ? (
        <Button size="sm" onClick={onChat}>
          <MessagesSquareIcon /> Open the session
        </Button>
      ) : (
        <Button size="sm" asChild>
          <Link href={hrefs.task(t.id)}>
            <MessagesSquareIcon /> Open the session
          </Link>
        </Button>
      )}
    </div>
  );
}

// The lead agent's face with its name in the tooltip; a task without a
// lead yet shows a dashed ring instead.
function LeadFace({ t }: { t: Task }) {
  if (!t.lead)
    return (
      <Tooltip>
        <TooltipTrigger asChild>
          <span className="inline-flex size-5 shrink-0 items-center justify-center rounded-full border border-dashed border-border text-muted-foreground" aria-label="No lead agent yet" role="img">
            <CircleDashedIcon className="size-3" aria-hidden />
          </span>
        </TooltipTrigger>
        <TooltipContent>No lead agent yet</TooltipContent>
      </Tooltip>
    );
  const name = agentName(t.lead, t.project);
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className="inline-flex shrink-0" role="img" aria-label={"Lead agent " + name}>
          <Avatar seed={t.lead} size={20} />
        </span>
      </TooltipTrigger>
      <TooltipContent>{"Lead agent: " + name}</TooltipContent>
    </Tooltip>
  );
}

// One task as a card (spec "Tasks"): the lead's face and the project's
// avatar on the first line with the priority mark, the title at 14 px, one
// meta line - status, the progress ring, when it last moved - and at most
// two buttons. A task asked you keeps its marigold edge.
export function TaskCard({ t, showProject, onOpen, active, compact }: { t: Task; showProject?: boolean; onOpen?: (t: Task) => void; active?: boolean; compact?: boolean }) {
  const router = useRouter();
  const review = t.status === "review";
  const idle = idleLine(t);
  // Always a button, never a link: the card carries links and buttons of its
  // own, and an anchor cannot nest them. Opening a task means its
  // conversation on the Chat page unless the page says otherwise.
  const open = onOpen ? () => onOpen(t) : () => router.push(hrefs.task(t.id));
  const p = taskProgress(t);
  return (
    <ItemCard onOpen={open} active={active} highlight={review} ariaLabel={"Task: " + t.title} className="gap-2.5 rounded-lg p-4 sm:p-4">
      <div className="flex min-w-0 items-center gap-2">
        <LeadFace t={t} />
        {showProject ? (
          <Tooltip>
            <TooltipTrigger asChild>
              <span className="inline-flex shrink-0" role="img" aria-label={"Project " + t.project}>
                <ProjectAvatar slug={t.project} size={20} />
              </span>
            </TooltipTrigger>
            <TooltipContent>{t.project}</TooltipContent>
          </Tooltip>
        ) : null}
        <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">{t.id}</span>
        {/* Only a priority that differs from normal earns a mark on the card;
            the title's chevron is the card's one "open" cue. */}
        {t.priority >= 30 || t.priority < 10 ? <PriorityMark p={t.priority} className="text-xs" /> : null}
      </div>
      <ItemCardTitle chevron className="text-sm font-medium leading-5">
        {t.title}
      </ItemCardTitle>
      <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-[13px] text-muted-foreground">
        <StatusBadge status={t.status} size="sm" />
        {p.total ? <TaskProgressBar t={t} /> : null}
        <span className="ml-auto whitespace-nowrap" title={t.current_rev > 1 ? "Version " + t.current_rev : undefined}>
          <Ago d={t.updated_at} />
        </span>
      </div>
      {!compact ? <TaskWorkingLine t={t} compact /> : null}
      {!compact && idle ? (
        <p className="flex min-w-0 items-start gap-2 text-[13px] leading-relaxed text-muted-foreground">
          <CircleDashedIcon className="mt-[3px] size-3.5 shrink-0" aria-hidden />
          <span className="min-w-0 anywhere">{idle}</span>
        </p>
      ) : null}
      {review && !compact ? <TaskNeedsYou t={t} /> : null}
      <ItemCardActions className="justify-end pt-0.5">
        <TaskQuickActions t={t} onOpen={open} chat={!review} />
      </ItemCardActions>
    </ItemCard>
  );
}

export function TaskCards({ tasks, showProject, onOpen, activeId, cols = 2, compact, empty, emptyTitle, emptyAction }: { tasks: Task[]; showProject?: boolean; onOpen?: (t: Task) => void; activeId?: string | null; cols?: 1 | 2 | 3; compact?: boolean; empty?: React.ReactNode; emptyTitle?: React.ReactNode; emptyAction?: React.ReactNode }) {
  if (!tasks.length)
    return (
      <EmptyState title={emptyTitle} action={emptyAction}>
        {empty ?? "No tasks here."}
      </EmptyState>
    );
  return (
    <CardGrid cols={cols}>
      {tasks.map((t) => (
        <TaskCard key={t.id} t={t} showProject={showProject} onOpen={onOpen} active={activeId === t.id} compact={compact} />
      ))}
    </CardGrid>
  );
}
