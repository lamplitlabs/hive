"use client";

// Conversations: the window where the owner asks an agent to work and watches
// it go - like the Copilot or Claude Code apps, inside the hub. The hub runs
// the agent (`/api/v1/chat`); this file only shows what it says, answers what
// it asks, and keeps the transcript growing without a reload.

import Link from "next/link";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import {
  ArchiveIcon,
  ArrowUpIcon,
  BotIcon,
  BrainIcon,
  BugIcon,
  CheckIcon,
  ChevronDownIcon,
  ChevronRightIcon,
  CircleAlertIcon,
  CircleCheckIcon,
  CircleDashedIcon,
  CirclePauseIcon,
  CircleStopIcon,
  CornerDownRightIcon,
  CpuIcon,
  FilePenIcon,
  FileTextIcon,
  FolderGitIcon,
  GitBranchIcon,
  GitPullRequestIcon,
  GlobeIcon,
  InfoIcon,
  ListChecksIcon,
  ListEndIcon,
  LoaderCircleIcon,
  LockIcon,
  LockOpenIcon,
  MessageSquareIcon,
  MessageSquarePlusIcon,
  PaperclipIcon,
  PencilIcon,
  RefreshCwIcon,
  RotateCcwIcon,
  SearchIcon,
  ShieldCheckIcon,
  ShieldQuestionIcon,
  SparklesIcon,
  SquareDashedMousePointerIcon,
  TerminalIcon,
  UsersIcon,
  WrenchIcon,
  XIcon,
  ZapIcon,
  type LucideIcon,
} from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import {
  Canvas,
  PlanList,
  useCanvas,
  type CanvasControl,
} from "@/components/chat/canvas";
import {
  ArtifactList,
  ChangeList,
  ReviewAllButton,
  bytesWord,
  fileIcon,
  useChatDiff,
  type ChatDiff,
  type ChatView,
} from "@/components/chat/changes";
import {
  STEP_ICON,
  STEP_LABEL,
  exitWords,
  mergeSteps,
} from "@/components/harness/harness";
import { ConfirmDialog } from "@/components/pulse/confirm";
import { Markdown } from "@/components/pulse/markdown";
import { PulseMascot } from "@/components/pulse/mascots";
import {
  EmptyState,
  ErrorNotice,
  Notice,
  SkeletonRows,
} from "@/components/pulse/panel";
import { PageHeader } from "@/components/pulse/page-header";
import { Avatar, ProjectLink, When } from "@/components/pulse/who";
import { ProjectAvatar } from "@/components/pulse/avatar";
import { BrandLogo } from "@/components/pulse/logo";
import { SectionHeading } from "@/components/pulse/section-heading";
import { StatusIcon } from "@/components/pulse/status-badge";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { logoFor } from "@/lib/logos";
import { BackButton, Pane, Split } from "@/components/chat/split";
import { CommandPalette } from "@/components/shell/command-palette";
import { MicButton, useDictation } from "@/components/chat/mic";
import { TaskView } from "@/components/chat/task-view";
import {
  LOCKDOWN_BADGE,
  LOCKDOWN_BAR,
  lockdownFlipMessage,
  lockdownHint,
  lockdownOf,
} from "@/components/chat/lockdown-state";
import {
  RESTART_NOTE,
  SESSION_STATE_WORD,
  dismissTip,
  groupToolCalls,
  sessionState,
  tipDismissed,
  toolDuration,
  toolGroupLabel,
  toolIcon,
  unfinished,
  type SessionState,
  type ToolIconName,
} from "@/components/chat/session-state";
import {
  QUEUE_LABEL,
  QUEUE_TITLE,
  SEND_LABEL,
  SEND_TITLE,
  STEERED_HINT,
  STEER_LABEL,
  STEER_TITLE,
  composeKeysHint,
  composeMode,
  composePlaceholder,
  sentLabel,
  showsNow,
  steeredOf,
  turnOpen,
  type ComposeMode,
} from "@/components/chat/compose-mode";
import type {
  ChatArtifact,
  ChatAttachment,
  ChatChanges,
  ChatListResponse,
  ChatPending,
  ChatShowResponse,
  ChatTask,
  Conversation,
  ConversationLive,
  ConversationMessage,
  HarnessModel,
  HarnessModelsResponse,
  HarnessSessionResponse,
  HarnessStep,
  PrState,
  ProjectsPayload,
} from "@/lib/api";
import {
  ApiError,
  CHAT_ATTACHMENTS_PER_MESSAGE,
  CHAT_ATTACHMENT_LIMIT,
  api,
  chatArtifactHref,
  chatAttachmentHref,
  chatAttachments,
  chatReport,
  chatUpload,
  enc,
  errorText,
  isChatImage,
  projectLabel,
  query,
  writes,
} from "@/lib/api";
import { ReportLink } from "@/components/chat/job-evidence";
import {
  hasReportPage,
  REPORT_PAGE,
  REPORT_WORD,
  reportTip,
} from "@/components/chat/job-evidence-state";
import { num, plural, sentence, span } from "@/lib/format";
import { messageStatus, type MessageStatus } from "@/lib/message-status";
import {
  queuedBesides,
  reconcileSent,
  sentMessage,
  sentShowsNow,
  type SentMessage,
} from "@/lib/sent-messages";
import { hrefs } from "@/lib/nav";
import {
  GROUP_ORDER,
  PR_NEEDS_OWNER,
  TONE_TEXT,
  WAITING_FOR_PR,
  entryGroup,
  foldedEntries,
  listEntries,
  prMood,
  relShort,
  reportVerb,
  sessionWords,
  taskFinished,
  taskMood,
  taskTitleOf,
  type ListGroup,
  type Mood,
  type Tone,
} from "@/components/chat/task-groups";
import { makeTab } from "@/lib/canvas";
import {
  pickLabel,
  pickPlace,
  splitPicks,
  withPicks,
  type ElementPick,
} from "@/lib/pick";
import {
  ACTION_KINDS,
  beatsOf,
  brief,
  countWords,
  groupNotes,
  streamTail,
} from "@/lib/transcript";
import type { ActionItem } from "@/lib/transcript";
import { invalidate, useApi, useNow } from "@/lib/use-api";
import { cn } from "@/lib/utils";

// ----- words ----------------------------------------------------------------

// One sentence per state, the same on the list, the header and the tab title.
function moodOf(c: Conversation, live?: ConversationLive | null): Mood {
  const l = live ?? c.live;
  const status = l?.status || c.status;
  const pending = l?.pending ?? null;
  const pr = prMood(c.pr?.state);
  // The owner's Merge on the conversation's pull request comes before
  // whatever its agent is doing: the reviewer handed the pull request over
  // and nothing moves until the owner decides.
  if (pr && c.pr && PR_NEEDS_OWNER.has(c.pr.state)) return pr;
  if (status === "waiting" && pending?.kind === "approval")
    return {
      word: "Needs a yes",
      tone: "wait",
      pip: "waiting",
      dot: "bg-warn",
    };
  if (status === "waiting")
    return { word: "Asked you", tone: "wait", pip: "waiting", dot: "bg-warn" };
  if (status === "running")
    return { word: "Working", tone: "run", pip: "busy", dot: "bg-primary" };
  if (status === "archived")
    return {
      word: "Archived",
      tone: "neutral",
      pip: "asleep",
      dot: "bg-muted-foreground/40",
    };
  // Nobody at it: the pull request opened from its branch says where the
  // work stands; without one, commits main lacks wait for the daemon to open
  // it (task-groups.ts groupOf puts both in the In review section).
  if (pr) return pr;
  if (c.waiting_to_land && !c.pr) return WAITING_FOR_PR;
  if (status === "closed")
    return { word: "Finished", tone: "neutral", pip: "happy", dot: "bg-ok/50" };
  if (status === "ended")
    return {
      word: "Session ended",
      tone: "neutral",
      pip: "curious",
      dot: "bg-muted-foreground/60",
    };
  // PULSE stopped the turn to restart and nothing carried it on: it waits
  // for a message ("continue" is enough), so it is the owner's turn.
  if (status === "idle" && c.note === RESTART_NOTE)
    return {
      word: SESSION_STATE_WORD.stopped,
      tone: "wait",
      pip: "waiting",
      dot: "bg-warn",
    };
  return {
    word: c.turns ? "Idle" : "New",
    tone: "ok",
    pip: "happy",
    dot: "bg-ok",
  };
}

// The list's sections (task-groups.ts): needs your review, working, in
// review, open, finished, archived. Every entry is a conversation - a plain
// one, one with sub-conversations under it, or a task with its steps under
// it - in the section of its most pressing member.
const GROUP_TITLES: Record<ListGroup, string> = {
  needs_review: "Needs your review",
  working: "Working",
  review: "In review",
  open: "Open",
  finished: "Finished",
  archived: "Archived",
};

// The state a list row leads with (spec "Sessions"): the spinner while it
// works, a marigold dot when it asks you, a tick when it is done, and the
// quiet speech bubble otherwise. The word rides in the icon's label.
function SessionStateIcon({
  state,
  className,
}: {
  state: SessionState;
  className?: string;
}) {
  if (state === "open")
    return (
      <MessageSquareIcon
        className={cn("size-4 shrink-0 text-muted-foreground", className)}
        strokeWidth={1.75}
        role="img"
        aria-label={SESSION_STATE_WORD.open}
      >
        <title>{SESSION_STATE_WORD.open}</title>
      </MessageSquareIcon>
    );
  if (state === "stopped")
    return (
      <CirclePauseIcon
        className={cn("size-4 shrink-0 text-warn", className)}
        strokeWidth={1.75}
        role="img"
        aria-label={SESSION_STATE_WORD.stopped}
      >
        <title>{SESSION_STATE_WORD.stopped}</title>
      </CirclePauseIcon>
    );
  return (
    <StatusIcon
      status={
        state === "working" ? "running" : state === "asks" ? "waiting" : "done"
      }
      className={className}
    />
  );
}

// A task's state in the list's vocabulary: its mood tone says whose turn.
function taskState(t: ChatTask): SessionState {
  const m = taskMood(t);
  if (m.tone === "wait") return "asks";
  if (taskFinished(t)) return "done";
  if (m.tone === "run" || t.sessions.some((s) => s.status === "running"))
    return "working";
  return "open";
}

// One row of the list: 44 px (36 compact), the state icon, the project's
// avatar, the title and the age at the right; a teal bar marks the open one.
// The column is narrow, so a long title takes a second line rather than
// ending after a few words. Tasks and conversations share it.
function SessionRow({
  state,
  project,
  title,
  age,
  active,
  marks,
  onOpen,
  titleAttr,
  framed,
  sealed,
}: {
  state: SessionState;
  project: string;
  title: string;
  age: string;
  active: boolean;
  marks?: ReactNode;
  onOpen: () => void;
  titleAttr?: string;
  framed?: boolean;
  sealed?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onOpen}
      aria-current={active ? "page" : undefined}
      title={titleAttr}
      className={cn(
        "relative flex min-h-(--row-h) w-full min-w-0 items-center gap-2.5 px-3 py-1.5 text-left transition-colors duration-120 ease-out-soft motion-reduce:transition-none",
        active
          ? "bg-accent before:absolute before:inset-y-0 before:left-0 before:w-0.5 before:bg-link before:content-['']"
          : framed
            ? "bg-accent/50 hover:bg-muted/60"
            : "hover:bg-muted/60",
        sealed && "lockdown-surface",
      )}
      data-lockdown={sealed || undefined}
    >
      <SessionStateIcon state={state} />
      {project ? (
        <ProjectAvatar slug={project} name={project} size={20} />
      ) : (
        <span
          className="inline-flex size-5 shrink-0 items-center justify-center rounded-[5px] border border-dashed border-border text-muted-foreground"
          role="img"
          aria-label="No project yet"
          title="No project yet"
        >
          <SparklesIcon className="size-3" aria-hidden />
        </span>
      )}
      <span
        className="line-clamp-2 min-w-0 flex-1 text-sm font-medium leading-5 text-foreground anywhere"
        title={title}
      >
        {title}
      </span>
      {marks}
      <span className="shrink-0 text-xs tabular-nums text-muted-foreground">
        {age}
      </span>
    </button>
  );
}

const LET_AGENT_DECIDE = "__decide__";

// The way out of a session whose memory holds something the provider
// refuses (a reply cut mid-stream that left a broken tool call behind),
// without restarting the hub: the session ends, the next message loads
// the conversation back from its saved transcript and repairs it.
const RESTART_SESSION_HINT =
  "Start the agent's session over from the saved transcript. Keeps the transcript and the workspace; the next message reloads the session from the transcript, repairing anything the provider refused.";

function stepRange(
  m: ConversationMessage,
): { session: string; from: number; to: number } | null {
  const session = typeof m.extra?.session === "string" ? m.extra.session : "";
  const r = m.extra?.steps;
  if (!session || !Array.isArray(r) || r.length !== 2) return null;
  const from = Number(r[0]);
  const to = Number(r[1]);
  if (!Number.isFinite(from) || !Number.isFinite(to) || to < 0) return null;
  return { session, from, to };
}

// Steps worth showing under a reply: what the agent wrote along the way,
// the tools it used and what it noted - not the run's exit, and not the
// question it asked (the transcript carries that as its own row). The reply
// itself is the message; its step stays only for the time it took to write.
function turnSteps(
  all: HarnessStep[],
  from: number,
  to: number | null,
  reply: string,
): HarnessStep[] {
  return all
    .filter((s) => {
      if (s.n <= from || (to !== null && s.n > to)) return false;
      if (s.kind === "exit" && !(s.depth ?? 0)) return false;
      if (isQuestionStep(s)) return false;
      return true;
    })
    .map((s) =>
      s.kind === "assistant" && reply && s.content.trim() === reply.trim()
        ? { ...s, content: "" }
        : s,
    );
}

// The question tool leaves a note step behind ("question: ... | choices");
// the transcript shows the question itself, so the note would be a repeat.
function isQuestionStep(s: HarnessStep): boolean {
  return (
    s.kind === "note" &&
    (s.command === "question" || /^question:/i.test(s.content.trimStart()))
  );
}

// The folds behind the transcript - ACTION_KINDS, beatsOf, groupNotes,
// thoughtSeconds, streamTail, brief, ActionItem and Beat - live in
// @/lib/transcript, plain TypeScript the node test suite imports directly.

// A row's two parts: what the agent did in plain words (for a command, the
// description it gave the bash tool) and the exact request, dimmed after.
function actionWords(s: HarnessStep): { words: string; detail: string } {
  switch (s.kind) {
    case "command":
      return {
        words: (s.title ?? "").trim() || "Ran a command",
        detail: s.command || firstLine(s.content),
      };
    case "subagent":
      return {
        words: STEP_LABEL.subagent,
        detail: [s.command, firstLine(s.content)].filter(Boolean).join(": "),
      };
    case "exit":
      return {
        words:
          s.content === "submitted"
            ? "The helper reported back."
            : "The helper stopped: " + (s.content || "unknown") + ".",
        detail: "",
      };
    case "note":
      return { words: firstLine(sentence(s.content)), detail: "" };
    case "error":
      return { words: "Something went wrong", detail: firstLine(s.content) };
    default:
      return {
        words: STEP_LABEL[s.kind] ?? s.kind,
        detail: firstLine(s.content) || s.command,
      };
  }
}

function firstLine(t: string): string {
  return (t || "").split("\n").find((l) => l.trim()) ?? "";
}

function Activity({
  steps,
  live = false,
}: {
  steps: HarnessStep[];
  live?: boolean;
}) {
  const beats = useMemo(() => beatsOf(steps), [steps]);
  if (!beats.length) return null;
  return (
    <div className="grid min-w-0 gap-3">
      {beats.map((b, i) => {
        const last = i === beats.length - 1;
        if (b.kind === "said") {
          return (
            <div
              key={b.step.n}
              className="min-w-0 max-w-[72ch] text-sm leading-relaxed text-foreground"
            >
              <Markdown
                source={b.step.content}
                className="[&_pre]:whitespace-pre-wrap"
              />
            </div>
          );
        }
        if (b.kind === "plan") return <PlanList key={b.step.n} step={b.step} />;
        return (
          <ActionRun
            key={"a" + b.items[0].step.n}
            items={b.items}
            running={live && last}
          />
        );
      })}
    </div>
  );
}

const SHOW_LAST = 8;

function NoteFold({ items }: { items: ActionItem[] }) {
  const depth = Math.min(items[0].step.depth ?? 0, 3);
  const first = actionWords(items[0].step).words;
  return (
    <li
      className="min-w-0"
      style={depth ? { marginLeft: depth * 20 } : undefined}
    >
      <details className="group/notes min-w-0">
        <summary className="flex min-w-0 cursor-pointer list-none items-center gap-2.5 rounded-md px-1.5 py-1 text-[13px] leading-snug text-muted-foreground select-none hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring [&::-webkit-details-marker]:hidden">
          <InfoIcon
            className="size-3.5 shrink-0 text-muted-foreground/70"
            aria-hidden
          />
          <span className="shrink-0">
            {plural(items.length, "note")} from PULSE
          </span>
          <span
            className="min-w-0 flex-1 truncate text-muted-foreground/60 group-open/notes:hidden"
            title={first}
          >
            {first}
          </span>
        </summary>
        <ol className="grid min-w-0" role="list">
          {items.map((it) => (
            <ActionRow
              key={it.step.n + ":" + (it.step.depth ?? 0)}
              item={it}
              running={false}
            />
          ))}
        </ol>
      </details>
    </li>
  );
}

// A run of tool calls, newest rows always in view: a long run folds its
// earlier rows under one line rather than filling the screen. Inside, two
// or more plain tool calls in a row fold under one "N tool calls · Xs"
// row (groupToolCalls); the run's tail stays open while it is running.
function ActionRun({
  items,
  running,
}: {
  items: ActionItem[];
  running: boolean;
}) {
  const rows = (list: ActionItem[], tailRunning: boolean) => {
    const groups = groupNotes(list);
    const row = (g: (typeof groups)[number], i: number) =>
      Array.isArray(g) ? (
        <NoteFold key={"notes" + g[0].step.n} items={g} />
      ) : (
        <ActionRow
          key={g.step.n + ":" + (g.step.depth ?? 0)}
          item={g}
          running={
            tailRunning &&
            i === groups.length - 1 &&
            ACTION_KINDS.has(g.step.kind) &&
            !g.result
          }
        />
      );
    // Each entry carries its group and index; a note fold is never a tool
    // call (its first step is a note), so it always stands alone.
    const folded = groupToolCalls(
      groups.map((g, i) => {
        const one = Array.isArray(g) ? g[0] : g;
        return { g, i, step: one.step, result: one.result, thought: one.thought };
      }),
    );
    return (
      <ol className="grid min-w-0" role="list">
        {folded.map((f) =>
          f.kind === "one" ? (
            row(f.item.g, f.item.i)
          ) : (
            <ToolCallGroup
              key={"calls" + f.items[0].step.n}
              n={f.items.length}
              ms={f.ms}
              open={tailRunning && f.items[f.items.length - 1].i === groups.length - 1}
            >
              {f.items.map((e) => row(e.g, e.i))}
            </ToolCallGroup>
          ),
        )}
      </ol>
    );
  };
  if (items.length <= SHOW_LAST) return rows(items, running);
  const earlier = items.slice(0, items.length - SHOW_LAST);
  return (
    <div className="grid min-w-0">
      <details className="group/more min-w-0">
        <summary className="flex w-fit min-h-8 cursor-pointer list-none items-center gap-2.5 rounded-md px-1.5 text-[13px] leading-snug text-muted-foreground select-none hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring [&::-webkit-details-marker]:hidden">
          <ChevronRightIcon
            className="size-4 shrink-0 transition-transform duration-150 group-open/more:rotate-90 motion-reduce:transition-none"
            aria-hidden
          />
          {plural(earlier.length, "earlier step")} - {countWords(earlier)}
        </summary>
        {rows(earlier, false)}
      </details>
      {rows(items.slice(-SHOW_LAST), running)}
    </div>
  );
}

// "3 tool calls · 4.2s": consecutive tool calls under one 32 px row that
// unfolds to the list. Open while the run is live so the newest call shows.
function ToolCallGroup({
  n,
  ms,
  open: open0,
  children,
}: {
  n: number;
  ms: number;
  open: boolean;
  children: ReactNode;
}) {
  const [open] = useState(open0);
  return (
    <li className="min-w-0">
      <details className="group/calls min-w-0" open={open || undefined}>
        <summary className="flex min-h-8 min-w-0 cursor-pointer list-none items-center gap-2.5 rounded-md px-1.5 text-[13px] leading-snug text-muted-foreground select-none hover:bg-muted/50 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring [&::-webkit-details-marker]:hidden">
          <WrenchIcon className="size-4 shrink-0" aria-hidden />
          <span className="min-w-0 truncate tnum">{toolGroupLabel(n, ms)}</span>
          <ChevronRightIcon
            className="ml-auto size-4 shrink-0 transition-transform duration-150 group-open/calls:rotate-90 motion-reduce:transition-none"
            aria-hidden
          />
        </summary>
        <ol className="grid min-w-0 border-l border-border/70 pl-1.5" role="list">
          {children}
        </ol>
      </details>
    </li>
  );
}

const OUTPUT_LINES = 60;

// The lucide icon behind a tool call row's icon name (session-state.ts
// decides the name; a step's kind maps straight onto a tool).
const TOOL_ICON: Record<ToolIconName, LucideIcon> = {
  Terminal: TerminalIcon,
  FilePen: FilePenIcon,
  FileText: FileTextIcon,
  Globe: GlobeIcon,
  Search: SearchIcon,
  Bot: BotIcon,
  Wrench: WrenchIcon,
};

// One line of the transcript, the way the Copilot app lists a tool call: a
// 32 px row with the tool's icon, what the agent did in plain words, the
// exact request in mono after it, how long it took at the right and a
// chevron. A click (or Enter) folds the request and its output open.
function ActionRow({ item, running }: { item: ActionItem; running: boolean }) {
  const { step: s, result } = item;
  const depth = Math.min(s.depth ?? 0, 3);
  const indent = depth ? { marginLeft: depth * 20 } : undefined;
  if (item.thought) {
    return (
      <li
        className="flex min-h-8 min-w-0 items-center gap-2.5 px-1.5 text-[13px] leading-snug text-muted-foreground"
        style={indent}
      >
        <CircleDashedIcon
          className="size-4 shrink-0 text-muted-foreground/70"
          aria-hidden
        />
        <span>Thought for {brief(item.thought)}</span>
      </li>
    );
  }
  const quiet = !ACTION_KINDS.has(s.kind);
  const Icon =
    s.kind === "exit"
      ? CircleCheckIcon
      : quiet
        ? (STEP_ICON[s.kind] ?? InfoIcon)
        : TOOL_ICON[toolIcon(s.kind)];
  const { words, detail } = actionWords(s);
  const exit = result ? exitWords(result.exit_code) : null;
  const outLines = result?.content ? result.content.split("\n") : [];
  const bad = !!exit?.bad || s.kind === "error";
  const took = quiet ? "" : toolDuration(result?.duration_ms || s.duration_ms);
  const foldable = !quiet || (s.content || "").trim().length > words.length;
  const head = (
    <>
      <Icon
        className={cn(
          "size-4 shrink-0",
          bad
            ? "text-status-failed"
            : quiet
              ? "text-muted-foreground/70"
              : "text-muted-foreground",
        )}
        aria-hidden
      />
      <span
        className={cn(
          "min-w-0 truncate",
          quiet ? "text-muted-foreground" : "text-foreground",
        )}
        title={words}
      >
        {words}
      </span>
      {detail ? (
        <span
          className="min-w-0 flex-1 truncate font-mono text-xs text-muted-foreground/70"
          title={detail}
        >
          {detail}
        </span>
      ) : null}
      {running ? (
        <StatusIcon status="running" className="ml-auto size-3.5" />
      ) : bad ? (
        <span
          className="ml-auto flex shrink-0 items-center gap-1 text-xs text-status-failed"
          title={result ? exitWords(result.exit_code)?.text : undefined}
        >
          <CircleAlertIcon className="size-3.5" aria-hidden /> Failed
        </span>
      ) : took ? (
        <span className="ml-auto shrink-0 text-xs text-muted-foreground tnum">
          {took}
        </span>
      ) : null}
      {foldable ? (
        <ChevronRightIcon
          className={cn(
            "size-4 shrink-0 text-muted-foreground transition-transform duration-150 group-open/row:rotate-90 motion-reduce:transition-none",
            !running && !bad && !took && "ml-auto",
          )}
          aria-hidden
        />
      ) : null}
    </>
  );
  if (!foldable) {
    return (
      <li
        className="flex min-h-8 min-w-0 items-center gap-2.5 px-1.5 text-[13px] leading-snug"
        style={indent}
      >
        {head}
      </li>
    );
  }
  return (
    <li className="min-w-0" style={indent}>
      <details className="group/row min-w-0">
        <summary className="flex min-h-8 min-w-0 cursor-pointer list-none items-center gap-2.5 rounded-md px-1.5 text-[13px] leading-snug select-none hover:bg-muted/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring [&::-webkit-details-marker]:hidden">
          {head}
        </summary>
        <div className="mt-1 mb-2 ml-6 grid min-w-0 gap-1.5">
          {s.kind === "command" ? (
            <pre className="min-w-0 rounded-lg bg-zinc-950 px-3 py-2 font-mono text-xs leading-relaxed whitespace-pre-wrap text-zinc-100 anywhere dark:bg-black">
              <span className="text-emerald-300 select-none">$ </span>
              {s.command}
            </pre>
          ) : (s.content || s.command) && !quiet ? (
            <pre className="min-w-0 rounded-lg border bg-muted/30 px-3 py-2 font-mono text-xs leading-relaxed whitespace-pre-wrap text-foreground/90 anywhere">
              {s.content || s.command}
            </pre>
          ) : quiet ? (
            <p className="min-w-0 text-[13px] leading-relaxed whitespace-pre-wrap text-muted-foreground anywhere">
              {s.content}
            </p>
          ) : null}
          {result ? (
            <pre className="max-h-72 min-w-0 overflow-auto rounded-lg border bg-background/60 px-3 py-2 font-mono text-xs leading-relaxed whitespace-pre-wrap text-foreground/80 anywhere">
              {outLines.length
                ? outLines.slice(0, OUTPUT_LINES).join("\n")
                : "(no output)"}
              {outLines.length > OUTPUT_LINES
                ? "\n… " +
                  num(outLines.length - OUTPUT_LINES) +
                  " more lines (the Harness page has them all)"
                : ""}
            </pre>
          ) : null}
          {result && (exit || result.duration_ms >= 1000) ? (
            <p className="text-xs text-muted-foreground tnum">
              {[
                exit?.text,
                result.duration_ms >= 1000
                  ? span(result.duration_ms / 1000)
                  : "",
              ]
                .filter(Boolean)
                .join(" · ")}
            </p>
          ) : null}
        </div>
      </details>
    </li>
  );
}

// The checklist the agent keeps with its plan tool: todo, doing, done.
function maxN(rows: { n: number }[]): number {
  return rows.reduce((m, s) => Math.max(m, s.n), -1);
}

function fail(e: unknown, what: string) {
  const t = errorText(e);
  toast.error(what, {
    description: [t.why, t.fix].filter(Boolean).join(" ") || t.error,
  });
}

// ----- the model ------------------------------------------------------------

const PROVIDER_DEFAULT = "__default__";

// Models an agent can run on: ones that chat and call tools. The provider's
// own default stays first so a conversation that never chose keeps working
// after a sign-in to a different provider.
function agentModels(models: HarnessModel[]): HarnessModel[] {
  return models.filter((m) => m.chat !== false && m.tool_calls !== false);
}

function modelWords(m: HarnessModel): string {
  const bits = [
    m.vendor,
    m.premium
      ? m.premium === 1
        ? "1 premium request"
        : num(m.premium) + " premium requests"
      : "no premium requests",
    m.preview ? "preview" : "",
  ].filter(Boolean);
  return bits.join(" · ");
}

// The provider's mark before a model name: its brand logo when lib/logos
// knows the vendor, a plain chip icon when it does not.
function ModelLogo({
  vendor,
  className,
}: {
  vendor: string | null | undefined;
  className?: string;
}) {
  const id = logoFor(vendor);
  return id ? (
    <BrandLogo id={id} size={16} className={className} />
  ) : (
    <CpuIcon className={cn("size-4", className)} aria-hidden />
  );
}

function ModelPicker({
  value,
  onChange,
  models,
  disabled,
  compact,
  label,
}: {
  value: string;
  onChange: (model: string) => void;
  models: HarnessModel[];
  disabled?: boolean;
  compact?: boolean;
  label: string;
}) {
  const rows = agentModels(models);
  const known = !value || rows.some((m) => m.id === value);
  const def = rows.find((m) => m.default);
  const picked = value ? rows.find((m) => m.id === value) : def;
  const current = value ? picked?.name || value : "Provider default";
  return (
    <Select
      value={value || PROVIDER_DEFAULT}
      onValueChange={(v) => onChange(v === PROVIDER_DEFAULT ? "" : v)}
      disabled={disabled}
    >
      <SelectTrigger
        size={compact ? "sm" : "default"}
        className={cn(compact ? "min-w-0 max-w-56 gap-1 px-2.5" : "w-60")}
        aria-label={label}
        title={label}
      >
        {/* The trigger shows the name alone; the items below carry a second
            line. The logo rides inside the value: Radix drops a className
            on SelectValue, so a value beside it would float to the middle
            of the trigger's justify-between row. */}
        <SelectValue placeholder="Model">
          <span className="flex min-w-0 items-center gap-2">
            <ModelLogo
              vendor={picked?.vendor}
              className="shrink-0 text-muted-foreground"
            />
            <span className="truncate">{current}</span>
          </span>
        </SelectValue>
      </SelectTrigger>
      <SelectContent
        position="popper"
        align={compact ? "end" : "start"}
        className="max-h-80 w-72"
      >
        <SelectItem value={PROVIDER_DEFAULT}>
          <span className="grid">
            <span>
              Provider default{def ? " · " + (def.name || def.id) : ""}
            </span>
            <span className="text-xs text-muted-foreground">
              Whatever the sign-in picks; changes with it.
            </span>
          </span>
        </SelectItem>
        {!known ? <SelectItem value={value}>{value}</SelectItem> : null}
        {rows.map((m) => (
          <SelectItem key={m.id} value={m.id}>
            <span className="flex min-w-0 items-start gap-2">
              <ModelLogo
                vendor={m.vendor}
                className="mt-0.5 text-muted-foreground"
              />
              <span className="grid min-w-0">
                <span className="flex items-center gap-1.5">
                  {m.name || m.id}
                  {m.default ? (
                    <span className="rounded-full bg-primary/10 px-1.5 text-xs font-medium text-primary">
                      default
                    </span>
                  ) : null}
                  {m.premium ? (
                    <span className="rounded-full bg-you-soft px-1.5 text-xs font-medium text-you-ink">
                      {m.premium === 1 ? "premium" : m.premium + "× premium"}
                    </span>
                  ) : null}
                </span>
                {!compact ? (
                  <span className="text-xs text-muted-foreground">
                    {modelWords(m)}
                  </span>
                ) : null}
              </span>
            </span>
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

// ----- the effort -----------------------------------------------------------

const SETTING_EFFORT = "__setting__";

// The reasoning efforts a conversation can ask of its model, in the order
// the providers rank them; "" leaves the harness setting in charge.
const EFFORTS: { id: string; name: string; words: string }[] = [
  {
    id: "default",
    name: "Model default",
    words: "Sends nothing; the model thinks as much as it does on its own.",
  },
  {
    id: "minimal",
    name: "Minimal",
    words: "The quickest answers; for small, clear asks.",
  },
  { id: "low", name: "Low", words: "Little thinking; fast and cheap." },
  { id: "medium", name: "Medium", words: "A balance of speed and care." },
  {
    id: "high",
    name: "High",
    words: "Longer thinking for hard, many-step work.",
  },
  {
    id: "xhigh",
    name: "Extra high",
    words: "The most thinking the model offers; slow and dear.",
  },
];

function effortName(id: string): string {
  return EFFORTS.find((e) => e.id === id)?.name || id;
}

function EffortPicker({
  value,
  onChange,
  disabled,
  compact,
  label,
}: {
  value: string;
  onChange: (effort: string) => void;
  disabled?: boolean;
  compact?: boolean;
  label: string;
}) {
  const current = value ? effortName(value) : "Effort from Settings";
  return (
    <Select
      value={value || SETTING_EFFORT}
      onValueChange={(v) => onChange(v === SETTING_EFFORT ? "" : v)}
      disabled={disabled}
    >
      <SelectTrigger
        size={compact ? "sm" : "default"}
        className={cn(compact ? "min-w-0 max-w-52 gap-1 px-2.5" : "w-52")}
        aria-label={label}
        title={label}
      >
        {/* The icon rides inside the value, as in the model picker. */}
        <SelectValue placeholder="Effort">
          <span className="flex min-w-0 items-center gap-2">
            <BrainIcon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
            <span className="truncate">{current}</span>
          </span>
        </SelectValue>
      </SelectTrigger>
      <SelectContent
        position="popper"
        align={compact ? "end" : "start"}
        className="max-h-80 w-72"
      >
        <SelectItem value={SETTING_EFFORT}>
          <span className="grid">
            <span>Effort from Settings</span>
            <span className="text-xs text-muted-foreground">
              The reasoning effort set in Settings; changes with it.
            </span>
          </span>
        </SelectItem>
        {EFFORTS.map((e) => (
          <SelectItem key={e.id} value={e.id}>
            <span className="grid">
              <span>{e.name}</span>
              {!compact ? (
                <span className="text-xs text-muted-foreground">
                  {e.words}
                </span>
              ) : null}
            </span>
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

// ----- attachments ----------------------------------------------------------

// Files picked for the next message, before they are uploaded.
interface Picked {
  key: string;
  file: File;
}

// Adds files to the pick, honouring the hub's limits; says why one was left out.
function pickFiles(
  cur: Picked[],
  incoming: FileList | File[] | null | undefined,
): Picked[] {
  if (!incoming) return cur;
  const next = [...cur];
  const tooBig: string[] = [];
  let full = false;
  for (const f of Array.from(incoming)) {
    if (!f || !f.name) continue;
    if (f.size > CHAT_ATTACHMENT_LIMIT) {
      tooBig.push(f.name);
      continue;
    }
    if (next.length >= CHAT_ATTACHMENTS_PER_MESSAGE) {
      full = true;
      break;
    }
    if (next.some((p) => p.file.name === f.name && p.file.size === f.size))
      continue;
    next.push({
      key:
        f.name +
        ":" +
        f.size +
        ":" +
        f.lastModified +
        ":" +
        Math.random().toString(36).slice(2, 7),
      file: f,
    });
  }
  if (tooBig.length)
    toast.error(
      tooBig.length === 1
        ? tooBig[0] + " is over 8 MB"
        : plural(tooBig.length, "file") + " are over 8 MB",
      {
        description:
          "Attachments go up to 8 MB each. Put a bigger file in the project and name its path instead.",
      },
    );
  if (full)
    toast.error("That is enough for one message", {
      description:
        "Up to " +
        CHAT_ATTACHMENTS_PER_MESSAGE +
        " files go with a message. Send these, then attach the rest.",
    });
  return next;
}

// The chips under a message: what was attached, each a link to the file.
// Images (a pasted screenshot, a picked photo) show a thumbnail that opens
// the full file in a new tab; every other file keeps the download chip.
function AttachmentChips({
  id,
  list,
  onUser,
}: {
  id: string;
  list: ChatAttachment[];
  onUser?: boolean;
}) {
  if (!list.length) return null;
  const images = list.filter(isChatImage);
  const files = list.filter((a) => !isChatImage(a));
  return (
    <div className={cn("grid min-w-0 gap-1.5", onUser && "justify-items-end")}>
      {images.length ? (
        <ul
          className={cn(
            "flex min-w-0 flex-wrap gap-1.5",
            onUser && "justify-end",
          )}
          aria-label={plural(images.length, "image")}
        >
          {images.map((a) => (
            <li key={a.name} className="min-w-0">
              <a
                href={chatAttachmentHref(id, a.name)}
                target="_blank"
                rel="noreferrer"
                className={cn(
                  "block overflow-hidden rounded-lg border transition-colors",
                  onUser
                    ? "border-primary-foreground/25 hover:border-primary-foreground/50"
                    : "border-border hover:border-foreground/40",
                )}
                title={a.name + " (" + bytesWord(a.size) + ")"}
              >
                {/* plain img: the hub serves the file itself, no next/image loader involved */}
                {/* eslint-disable-next-line @next/next/no-img-element */}
                <img
                  src={chatAttachmentHref(id, a.name)}
                  alt={a.name}
                  loading="lazy"
                  className="h-28 w-auto max-w-56 object-cover"
                />
              </a>
            </li>
          ))}
        </ul>
      ) : null}
      {files.length ? (
        <ul
          className={cn(
            "flex min-w-0 flex-wrap gap-1.5",
            onUser && "justify-end",
          )}
          aria-label={plural(files.length, "attachment")}
        >
          {files.map((a) => {
            const Icon = fileIcon(a.name);
            return (
              <li key={a.name} className="min-w-0">
                <a
                  href={chatAttachmentHref(id, a.name)}
                  target="_blank"
                  rel="noreferrer"
                  className={cn(
                    "flex min-w-0 max-w-64 items-center gap-1.5 rounded-lg border px-2 py-1 text-xs transition-colors",
                    onUser
                      ? "border-primary-foreground/25 bg-primary-foreground/10 text-primary-foreground hover:bg-primary-foreground/20"
                      : "border-border bg-card text-foreground hover:bg-muted/60",
                  )}
                  title={a.name + " (" + bytesWord(a.size) + ")"}
                >
                  <Icon className="size-3.5 shrink-0 opacity-80" aria-hidden />
                  <span className="min-w-0 truncate font-medium">{a.name}</span>
                  <span
                    className={cn(
                      "shrink-0 tnum",
                      onUser ? "opacity-75" : "text-muted-foreground",
                    )}
                  >
                    {bytesWord(a.size)}
                  </span>
                </a>
              </li>
            );
          })}
        </ul>
      ) : null}
    </div>
  );
}

// Three dots that breathe, for "the agent is thinking".
function TypingDots({ className }: { className?: string }) {
  return (
    <span
      className={cn("inline-flex items-center gap-0.5", className)}
      aria-hidden
    >
      <span className="typing-dot size-1.5 rounded-full bg-current" />
      <span className="typing-dot size-1.5 rounded-full bg-current" />
      <span className="typing-dot size-1.5 rounded-full bg-current" />
    </span>
  );
}

// ----- page -----------------------------------------------------------------

export function ChatPageClient() {
  const sp = useSearchParams();
  const router = useRouter();
  const pathname = usePathname();
  const id = sp.get("id") || "";
  // A task opens in place like a conversation and stays in the address, so a
  // link to it from a report or another window lands on the same view. One
  // thing is open at a time: opening either clears the other.
  const task = sp.get("task") || "";
  const [showArchived, setShowArchived] = useState(false);
  const list = useApi<ChatListResponse>(
    "chat" + (showArchived ? "?all=1" : ""),
    { refresh: 5000 },
  );
  const projects = useApi<ProjectsPayload>("projects", { refresh: 60000 });

  const open = useCallback(
    (next: string | null) => {
      router.replace(pathname + query({ id: next || undefined }), {
        scroll: false,
      });
    },
    [router, pathname],
  );
  const openTask = useCallback(
    (next: string | null) => {
      router.replace(pathname + query({ task: next || undefined }), {
        scroll: false,
      });
    },
    [router, pathname],
  );

  const rows = useMemo(() => {
    const all = list.data?.conversations ?? [];
    return [...all].sort((a, b) =>
      (b.last_at || b.created_at).localeCompare(a.last_at || a.created_at),
    );
  }, [list.data]);
  const tasks = useMemo(() => {
    const all = list.data?.tasks ?? [];
    return [...all].sort((a, b) =>
      (b.last_at || b.created_at).localeCompare(a.last_at || a.created_at),
    );
  }, [list.data]);
  const current = rows.find((c) => c.id === id);
  const attention =
    rows.filter((c) => (c.live?.status || c.status) === "waiting").length +
    tasks.filter((t) => t.needs_you).length;

  // The tab says when an agent waits on you, even from another window
  // (the shell names the page; this only adds the count).
  useEffect(() => {
    if (typeof document === "undefined") return;
    const base = "Sessions · PULSE";
    document.title = attention ? `(${attention}) ${base}` : base;
  }, [attention]);

  // An older hub has no /chat routes (404) or no conversations table (503).
  const unavailable =
    !!list.error &&
    !list.data &&
    /not available yet|not found|HTTP 404|HTTP 503/i.test(list.error);

  const listNode = (
    <ConversationList
      rows={rows}
      tasks={tasks}
      loading={list.loading && !list.data}
      current={id}
      currentTask={task}
      onOpen={open}
      onOpenTask={openTask}
      onNew={() => open(null)}
      showArchived={showArchived}
      onToggleArchived={() => setShowArchived((v) => !v)}
    />
  );
  const opened = !!(id || task);

  // One Split for the page, whatever is open: the list inside it stays
  // mounted across conversations, so opening one leaves the list where it
  // was - scrolled, folded, unmoved - instead of drawing it anew. With
  // nothing open the page reads like every other one: the shell gives chat
  // a wider column with 24 px gutters, so the landing caps itself at the
  // other pages' 1240 px (less two 24 px gutters) and pads 16 px more, to
  // start on their 40 px edge.
  return (
    <div
      className={cn(
        "grid min-w-0 gap-4",
        opened
          ? "lg:flex lg:h-[calc(100dvh-1rem)] lg:min-h-[34rem] lg:flex-col"
          : "lg:mx-auto lg:w-full lg:max-w-[1192px] lg:px-4 lg:pb-10",
      )}
    >
      <PageHeader
        title="Sessions"
        className={opened ? "mb-0 pt-4 sm:pt-5" : "mb-2"}
        subtitle={
          opened ? undefined : "Ask an agent to do something and follow every step"
        }
        action={
          // On a phone the list is off-screen while a conversation is open,
          // so the header carries the button; on wide screens the list's
          // own button is always in view.
          opened ? (
            <Button
              type="button"
              size="sm"
              className="lg:hidden"
              onClick={() => open(null)}
            >
              <MessageSquarePlusIcon /> New session
            </Button>
          ) : undefined
        }
      />
      {unavailable ? (
        <Notice tone="warn">
          Conversations need a hub built with this feature. Update PULSE (
          <code>pulse update</code>) and restart the hub.
        </Notice>
      ) : list.error && !list.data ? (
        <ErrorNotice error={list.error} onRetry={() => void list.reload()}>
          The session list did not load.
        </ErrorNotice>
      ) : null}
      <Split list={listNode} listOnMobile={!opened} fill={opened}>
        {task ? (
          <TaskView
            key={task}
            id={task}
            onBack={() => openTask(null)}
            onGone={() => openTask(null)}
          />
        ) : id ? (
          <ConversationView
            key={id}
            id={id}
            seed={current}
            projects={projects.data}
            onBack={() => open(null)}
            onGone={() => open(null)}
          />
        ) : (
          <Pane
            main={
              <NewConversation
                projects={projects.data}
                onCreated={(c) => open(c.id)}
                onTaskCreated={(t) => openTask(t)}
                busyElsewhere={
                  rows.filter((c) => (c.live?.status || c.status) === "running")
                    .length
                }
              />
            }
          />
        )}
      </Split>
    </div>
  );
}

// ----- the list -------------------------------------------------------------

function ConversationList({
  rows,
  tasks,
  loading,
  current,
  currentTask,
  onOpen,
  onOpenTask,
  onNew,
  showArchived,
  onToggleArchived,
}: {
  rows: Conversation[];
  tasks: ChatTask[];
  loading: boolean;
  current: string;
  currentTask: string;
  onOpen: (id: string) => void;
  onOpenTask: (id: string) => void;
  onNew: () => void;
  showArchived: boolean;
  onToggleArchived: () => void;
}) {
  const now = useNow(15000);
  // Finished conversations start hidden: they pile up and crowd out what is
  // still moving. The section header keeps the count and a Show/Hide.
  const [showFinished, setShowFinished] = useState(false);
  // One list: tasks and conversations together, by what moved last, in the
  // section of their state - the same words for both.
  const entries = useMemo(() => listEntries(rows, tasks), [rows, tasks]);
  const groups = GROUP_ORDER.map((g) => ({
    g,
    entries: entries.filter((e) => entryGroup(e) === g),
  })).filter((x) => x.entries.length);
  const composing = !current && !currentTask;
  return (
    <div className="grid min-w-0 gap-3">
      <div className="flex items-center gap-2">
        <Button
          type="button"
          variant={composing ? "secondary" : "default"}
          className="min-w-0 flex-1 justify-start gap-2"
          onClick={onNew}
          aria-current={composing ? "page" : undefined}
        >
          <MessageSquarePlusIcon className="size-4" />
          New session
        </Button>
        <Tooltip>
          <TooltipTrigger asChild>
            <CommandPalette
              variant="icon"
              className="size-9 shrink-0 rounded-md border bg-card"
            />
          </TooltipTrigger>
          <TooltipContent>Find a session (⌘K)</TooltipContent>
        </Tooltip>
      </div>
      {loading ? (
        <SkeletonRows n={4} />
      ) : entries.length === 0 ? (
        <EmptyState
          icon={<PulseMascot mood="happy" size={56} decorative />}
          hint="Start one on the right. What an agent hands on - a sub-session, a task's steps - is listed under it."
        >
          No sessions yet.
        </EmptyState>
      ) : (
        groups.map(({ g, entries: all }) => {
          const foldable = g === "finished";
          const folded = foldable && !showFinished;
          const shown = foldedEntries(all, g, folded, currentTask || current);
          return (
            <section
              key={g}
              className="grid min-w-0 gap-1.5"
              aria-label={GROUP_TITLES[g]}
            >
              <SectionHeading
                as="h3"
                count={all.length}
                className="mb-0 px-1 [&_h3]:text-sm"
                action={
                  foldable ? (
                    <button
                      type="button"
                      className="text-xs text-link underline-offset-4 hover:underline"
                      aria-expanded={!folded}
                      onClick={() => setShowFinished((v) => !v)}
                    >
                      {folded ? "Show" : "Hide"}
                    </button>
                  ) : undefined
                }
              >
                {GROUP_TITLES[g]}
              </SectionHeading>
              {shown.length ? (
                <ul
                  className="grid min-w-0 divide-y divide-border overflow-hidden rounded-lg border bg-card"
                  role="list"
                >
                  {shown.map((e, i) =>
                    e.kind === "task" ? (
                      <TaskItem
                        key={e.task.id}
                        t={e.task}
                        index={i}
                        active={e.task.id === currentTask}
                        now={now}
                        onOpen={onOpenTask}
                      />
                    ) : (
                      <ConversationItem
                        key={e.family.c.id}
                        c={e.family.c}
                        subs={e.family.children}
                        index={i}
                        active={e.family.c.id === current}
                        current={current}
                        now={now}
                        onOpen={onOpen}
                      />
                    ),
                  )}
                </ul>
              ) : null}
            </section>
          );
        })
      )}
      {!loading ? (
        <button
          type="button"
          className="justify-self-start px-1 pt-1 text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
          onClick={onToggleArchived}
        >
          {showArchived ? "Hide archived" : "Show archived"}
        </button>
      ) : null}
    </div>
  );
}

// The pull request under a card: its number as a link to the forge, where
// its review stands, and - when the reviewer handed it to the owner or its
// review failed - the Merge the owner presses. The hub records the ask and
// the daemon merges on GitHub as the owner on its next pass, so the row
// reads "Merging" after the click rather than "Merged". A row of its own
// because the card is one button and a link cannot sit inside a button.
function PrRow({ pr, project }: { pr: PrState; project: string }) {
  const [merge, setMerge] = useState(false);
  const m = prMood(pr.state);
  const word = m ? m.word : pr.state === "merged" ? "Merged" : pr.state === "closed" ? "Closed" : "";
  const tone: Tone = m ? m.tone : pr.state === "merged" ? "ok" : "neutral";
  const name = "PR #" + pr.number;
  return (
    <div
      className="flex min-w-0 items-center gap-2 border-t bg-muted/20 px-3 py-1.5 text-xs"
      data-testid="list-pr"
    >
      <GitPullRequestIcon
        className="size-3.5 shrink-0 text-muted-foreground"
        aria-hidden
      />
      <a
        href={pr.url}
        target="_blank"
        rel="noreferrer"
        className="shrink-0 font-medium text-foreground underline-offset-2 hover:underline"
        title={pr.title ? pr.title + " - open on GitHub" : "Open on GitHub"}
      >
        {name}
      </a>
      <span className={cn("min-w-0 truncate", TONE_TEXT[tone])}>{word}</span>
      {pr.can_merge && project ? (
        <>
          <Button
            type="button"
            size="xs"
            className="ml-auto"
            onClick={() => setMerge(true)}
          >
            Merge
          </Button>
          <ConfirmDialog
            open={merge}
            onOpenChange={setMerge}
            title={"Merge " + name + " into " + (pr.base || "the default branch") + "?"}
            description={
              <>
                {pr.title ? (
                  <span className="block font-medium text-foreground anywhere">
                    {pr.title}
                  </span>
                ) : null}
                <span className="mt-2 block">
                  The daemon merges it on GitHub as you.
                </span>
              </>
            }
            action="Merge"
            onConfirm={() => writes.mergeProjectPr(project, pr.number)}
            success={"Merging " + name + "\u2026"}
            invalidates={(p) =>
              p.startsWith("chat") || p.startsWith("tasks") || p.startsWith("events") || p.startsWith("projects")
            }
          />
        </>
      ) : null}
    </div>
  );
}

// A task in the list, the same card as a conversation: the row is the
// owner's conversation with the lead agent (the team icon is the one mark
// of a task), and under it one line per step the lead cut - the agent on
// it and its state - the way a conversation lists its sub-conversations. A
// finished task keeps its steps folded until asked.
function TaskItem({
  t,
  index,
  active,
  now,
  onOpen,
}: {
  t: ChatTask;
  index: number;
  active: boolean;
  now: number;
  onOpen: (id: string) => void;
}) {
  const m = taskMood(t);
  const [unfolded, setUnfolded] = useState(false);
  const steps = t.sessions;
  const showSteps =
    steps.length > 0 && (!taskFinished(t) || unfolded || active);
  return (
    <li
      className="enter min-w-0"
      style={{ animationDelay: Math.min(index, 8) * 30 + "ms" }}
    >
      <div className="grid min-w-0 gap-0">
        <SessionRow
          state={taskState(t)}
          project={t.project}
          title={t.title || t.id}
          age={relShort(t.last_at || t.created_at, now)}
          active={active}
          onOpen={() => onOpen(t.id)}
          titleAttr={
            (t.lead
              ? `With ${t.lead}, the lead agent of ${t.project}`
              : `A task for the ${t.project} team`) +
            " · " +
            m.word
          }
          marks={
            <UsersIcon
              className="size-3.5 shrink-0 text-muted-foreground"
              aria-label="with the project's team"
            />
          }
        />
        {showSteps ? (
          <ul
            className="grid min-w-0 gap-px border-t bg-muted/20 py-1"
            role="list"
            aria-label={`Steps of ${t.title || t.id}`}
          >
            {steps.map((s) => {
              const w = sessionWords(s, t.pr?.state);
              const body = (
                <>
                  <span
                    className="w-3.5 shrink-0 border-l border-b border-border/70 self-stretch rounded-bl"
                    aria-hidden
                  />
                  {s.agent ? (
                    <Avatar seed={s.agent} size={16} />
                  ) : (
                    <CircleDashedIcon
                      className="size-4 shrink-0 text-muted-foreground"
                      aria-hidden
                    />
                  )}
                  {/* The step's title, then its state under it: side by side, a long state ("Waiting for its pull request") left the title a couple of letters. */}
                  <span className="grid min-w-0 flex-1">
                    <span className="truncate text-foreground">
                      {s.title || s.kind}
                    </span>
                    <span
                      className={cn("truncate font-medium", TONE_TEXT[w.tone])}
                    >
                      {w.word}
                    </span>
                  </span>
                  <span className="shrink-0 self-start tnum text-muted-foreground">
                    {relShort(s.updated_at || s.started_at, now)}
                  </span>
                </>
              );
              const cls =
                "flex w-full min-w-0 items-center gap-2 px-3 py-1.5 text-left text-xs transition-colors hover:bg-muted/50";
              return (
                <li key={s.job} className="min-w-0">
                  {s.session ? (
                    <Link
                      href={hrefs.harness({ id: s.session })}
                      className={cls}
                      title={`${s.agent || "an agent"} on this step: open its run`}
                    >
                      {body}
                    </Link>
                  ) : (
                    <button
                      type="button"
                      onClick={() => onOpen(t.id)}
                      className={cls}
                      title="No agent has taken this step yet"
                    >
                      {body}
                    </button>
                  )}
                </li>
              );
            })}
          </ul>
        ) : steps.length ? (
          <button
            type="button"
            onClick={() => setUnfolded(true)}
            className="border-t bg-muted/20 px-3 py-1 text-left text-xs text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground"
          >
            {plural(steps.length, "step")} · show
          </button>
        ) : null}
        {t.pr ? <PrRow pr={t.pr} project={t.project} /> : null}
      </div>
    </li>
  );
}

// A conversation in the list, with its sub-conversations (the ones its
// agent started, or the owner from its page) as lines under it, each
// opening on its own: the family reads like a task and its steps.
function ConversationItem({
  c,
  subs = [],
  index,
  active,
  current = "",
  now,
  onOpen,
}: {
  c: Conversation;
  subs?: Conversation[];
  index: number;
  active: boolean;
  current?: string;
  now: number;
  onOpen: (id: string) => void;
}) {
  const m = moodOf(c);
  const sealed = lockdownOf(c, c.live);
  const childActive = subs.some((k) => k.id === current);
  return (
    <li
      className="enter min-w-0"
      style={{ animationDelay: Math.min(index, 8) * 30 + "ms" }}
    >
      <div className="grid min-w-0 gap-0">
        <SessionRow
          state={sessionState(c)}
          project={c.project}
          title={c.title || "Untitled"}
          age={relShort(c.last_at || c.created_at, now)}
          active={active}
          framed={childActive}
          sealed={sealed}
          onOpen={() => onOpen(c.id)}
          titleAttr={
            m.word +
            " · " +
            (c.status === "closed" && c.note
              ? c.note
              : c.project || "no project yet")
          }
          marks={
            sealed ? (
              <span className="lockdown-badge" title={LOCKDOWN_BADGE}>
                <LockIcon className="size-3" aria-hidden />
                <span className="sr-only">{LOCKDOWN_BADGE}</span>
                <span aria-hidden>Lockdown</span>
              </span>
            ) : undefined
          }
        />
        {subs.length ? (
          <ul
            className="grid min-w-0 gap-px border-t bg-muted/20 py-1"
            role="list"
            aria-label={`Sub-sessions of ${c.title || c.id}`}
          >
            {subs.map((k) => {
              const km = moodOf(k);
              const ks = sessionState(k);
              const kActive = k.id === current;
              return (
                <li key={k.id} className="min-w-0">
                  <button
                    type="button"
                    onClick={() => onOpen(k.id)}
                    aria-current={kActive ? "page" : undefined}
                    className={cn(
                      "relative flex min-h-8 w-full min-w-0 items-center gap-2 px-3 py-1 text-left text-xs transition-colors hover:bg-muted/50",
                      kActive &&
                        "bg-accent before:absolute before:inset-y-0 before:left-0 before:w-0.5 before:bg-link before:content-['']",
                    )}
                    title={`${k.title || k.id}: open the sub-session · ${km.word}`}
                  >
                    <span
                      className="w-3.5 shrink-0 border-l border-b border-border/70 self-stretch rounded-bl"
                      aria-hidden
                    />
                    <SessionStateIcon
                      state={sessionState(k)}
                      className="size-3.5"
                    />
                    <span className="min-w-0 flex-1 truncate text-foreground">
                      {k.title || "Untitled"}
                    </span>
                    {lockdownOf(k, k.live) ? (
                      <LockIcon
                        className="lockdown-mark size-3.5 shrink-0 text-lock"
                        aria-label={LOCKDOWN_BADGE}
                      />
                    ) : null}
                    {/* Which sub-session is still at it, or stopped: the
                        word beside the icon, so the family reads at a glance. */}
                    {ks === "working" || ks === "asks" || ks === "stopped" ? (
                      <span
                        className={cn("shrink-0 font-medium", TONE_TEXT[km.tone])}
                        aria-hidden
                      >
                        {ks === "stopped" ? "Stopped" : km.word}
                      </span>
                    ) : null}
                    <span className="shrink-0 tabular-nums text-muted-foreground">
                      {relShort(k.last_at || k.created_at, now)}
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        ) : null}
        {c.pr ? <PrRow pr={c.pr} project={c.project} /> : null}
      </div>
    </li>
  );
}

// ----- new conversation -----------------------------------------------------

const STARTERS: { icon: LucideIcon; text: string }[] = [
  {
    icon: SearchIcon,
    text: "Look through the open tasks and pick the most useful one to do next.",
  },
  { icon: BugIcon, text: "Read the latest failing check and fix it." },
  {
    icon: FileTextIcon,
    text: "Write a short guide for a new contributor and put it in docs/.",
  },
  {
    icon: SparklesIcon,
    text: "Review what landed this week and tell me what looks risky.",
  },
];

// The one-time explainer under the composer: three short points in one
// muted card, gone for good once dismissed (session-state.ts keeps the flag).
const NEW_SESSION_TIP: { icon: LucideIcon; title: string; body: string }[] = [
  {
    icon: GitBranchIcon,
    title: "Its own branch",
    body: "Each session works in a worktree on pulse/chat-… so your checkout stays clean. Land it when you like.",
  },
  {
    icon: ShieldCheckIcon,
    title: "Asks when it must",
    body: "Questions and risky commands pause the agent until you answer here. Answer from your phone, too.",
  },
  {
    icon: ListChecksIcon,
    title: "Hands work on in view",
    body: "Big work fans out: the agent starts sub-sessions, or the project's team takes it as steps. Each shows under its session in the list, open to watch or steer.",
  },
];

// The dismissal is read as an external store: the static export renders
// it hidden (no window), the browser shows it until dismissTip ran here
// or in another tab (the storage event).
const tipListeners = new Set<() => void>();
function subscribeTip(cb: () => void) {
  tipListeners.add(cb);
  window.addEventListener("storage", cb);
  return () => {
    tipListeners.delete(cb);
    window.removeEventListener("storage", cb);
  };
}
function dismissTipHere() {
  dismissTip();
  for (const cb of tipListeners) cb();
}

function NewSessionTip() {
  const dismissed = useSyncExternalStore(
    subscribeTip,
    () => tipDismissed(),
    () => true,
  );
  if (dismissed) return null;
  return (
    <aside
      className="enter relative grid gap-3 rounded-lg border bg-muted/50 px-4 py-3 pr-12 sm:grid-cols-3"
      aria-label="How sessions work"
      data-testid="new-session-tip"
    >
      {NEW_SESSION_TIP.map((f) => (
        <div key={f.title} className="grid gap-1">
          <span className="flex items-center gap-2 text-sm font-medium text-foreground">
            <f.icon className="size-4 text-muted-foreground" aria-hidden />{" "}
            {f.title}
          </span>
          <p className="text-[13px] leading-relaxed text-muted-foreground">
            {f.body}
          </p>
        </div>
      ))}
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            className="absolute top-2 right-2 text-muted-foreground"
            aria-label="Dismiss tip"
            onClick={dismissTipHere}
          >
            <XIcon />
          </Button>
        </TooltipTrigger>
        <TooltipContent>Dismiss tip</TooltipContent>
      </Tooltip>
    </aside>
  );
}

// Who takes the request: one agent in a conversation here, or the project's
// team - a task the lead agent plans and splits into steps, each a
// conversation of its own under the task in the list.
type Who = "agent" | "team";

function NewConversation({
  projects,
  onCreated,
  onTaskCreated,
  busyElsewhere,
}: {
  projects?: ProjectsPayload;
  onCreated: (c: Conversation) => void;
  onTaskCreated: (id: string) => void;
  busyElsewhere: number;
}) {
  const [text, setText] = useState("");
  const [project, setProject] = useState(LET_AGENT_DECIDE);
  const [who, setWho] = useState<Who>("agent");
  const [model, setModel] = useState("");
  const [reasoning, setReasoning] = useState("");
  const [auto, setAuto] = useState(true);
  const [lockdown, setLockdown] = useState(false);
  const [busy, setBusy] = useState(false);
  const textarea = useRef<HTMLTextAreaElement | null>(null);
  const insertSpokenText = useDictation(textarea, text, setText);
  const list = (projects?.projects ?? []).filter(
    (p) => p.status !== "inactive",
  );
  const models = useApi<HarnessModelsResponse>("harness/models", {
    refresh: 60000,
  });
  const team = who === "team";
  // A task belongs to a project from the start: the lead is the project's.
  const needsProject = team && project === LET_AGENT_DECIDE;
  const canStart = !!text.trim() && !busy && !needsProject;

  const start = async () => {
    const t = text.trim();
    if (!canStart) return;
    setBusy(true);
    if (team) {
      const id = toast.loading("Starting…");
      try {
        const r = await writes.newTask({
          project,
          title: taskTitleOf(t),
          brief: t,
          priority: 10,
        });
        toast.success(
          `Started. The ${project} lead agent is on it now and replies in the session.`,
          { id, description: r.task.id },
        );
        invalidate(
          (p) =>
            p.startsWith("chat") ||
            p.startsWith("tasks") ||
            p.startsWith("events") ||
            p.startsWith("projects/"),
        );
        onTaskCreated(r.task.id);
      } catch (e) {
        toast.dismiss(id);
        fail(e, "The session did not start");
      } finally {
        setBusy(false);
      }
      return;
    }
    try {
      const r = await writes.chatCreate({
        text: t,
        project: project === LET_AGENT_DECIDE ? "" : project,
        auto_approve: auto,
        lockdown,
        model,
        reasoning,
      });
      invalidate("chat");
      onCreated(r.conversation);
    } catch (e) {
      fail(e, "The session did not start");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="enter grid min-w-0 gap-4">
      <section className="relative overflow-hidden rounded-xl border bg-card p-5 shadow-card sm:p-6">
        <div className="flex min-w-0 flex-col gap-5 sm:flex-row sm:items-start">
          <PulseMascot
            mood={busy ? "busy" : text.trim() ? "curious" : "happy"}
            size={84}
            className="float shrink-0 self-center sm:self-start"
            title="Pip is ready"
          />
          {/* grid-cols-1 (minmax(0,1fr)): an auto track would grow to the widest starter chip and push the card past a phone's edge. */}
          <div className="grid min-w-0 flex-1 grid-cols-1 gap-4">
            <div className="grid gap-1">
              <h2 className="text-xl font-semibold tracking-tight text-foreground">
                What should the agent do?
              </h2>
              <p className="text-sm leading-relaxed text-muted-foreground">
                Say it the way you would to a colleague. The agent reads the
                project, works in its own branch, and reports back here.{" "}
                {busyElsewhere
                  ? `${plural(busyElsewhere, "other session")} ${busyElsewhere === 1 ? "is" : "are"} working right now.`
                  : ""}
              </p>
            </div>
            <Textarea
              ref={textarea}
              value={text}
              onChange={(e) => setText(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                  e.preventDefault();
                  void start();
                }
              }}
              placeholder="For example: add a dark mode toggle to the settings page and make sure the tests still pass."
              className="min-h-28 text-sm"
              autoFocus
              aria-label="What should the agent do"
            />
            <div className="flex flex-wrap items-center gap-1.5">
              <MicButton disabled={busy} onText={insertSpokenText} />
              {STARTERS.map((s) => (
                <Button
                  key={s.text}
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={() => setText(s.text)}
                  className="max-w-full justify-start font-normal text-muted-foreground hover:text-foreground"
                >
                  <s.icon aria-hidden />
                  <span className="truncate">{s.text}</span>
                </Button>
              ))}
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <Select value={who} onValueChange={(v) => setWho(v as Who)}>
                <SelectTrigger
                  className="w-auto min-w-40"
                  aria-label="Who does it"
                  data-testid="new-who"
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="agent">
                    <Avatar seed="agent" size={16} /> One agent
                  </SelectItem>
                  <SelectItem value="team">
                    <UsersIcon
                      className="size-4 text-muted-foreground"
                      aria-hidden
                    />{" "}
                    The project&apos;s team
                  </SelectItem>
                </SelectContent>
              </Select>
              <Select value={project} onValueChange={setProject}>
                <SelectTrigger
                  className={cn(
                    "w-auto min-w-44 max-w-64",
                    needsProject && "border-warn/60",
                  )}
                  aria-label="Project"
                  aria-invalid={needsProject || undefined}
                >
                  <SelectValue placeholder="Pick a project" />
                </SelectTrigger>
                <SelectContent>
                  {team ? null : (
                    <SelectItem value={LET_AGENT_DECIDE}>
                      <SparklesIcon className="size-4 text-primary" aria-hidden />{" "}
                      Let the agent decide
                    </SelectItem>
                  )}
                  {list.map((p) => (
                    <SelectItem key={p.slug} value={p.slug}>
                      <ProjectAvatar slug={p.slug} name={p.name} size={20} />{" "}
                      <span className="truncate">{projectLabel(p)}</span>
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {team ? null : (
                <>
                  <ModelPicker
                    value={model}
                    onChange={setModel}
                    models={models.data?.models ?? []}
                    label="Model for this session"
                  />
                  <EffortPicker
                    value={reasoning}
                    onChange={setReasoning}
                    label="Reasoning effort for this session"
                  />
                </>
              )}
            </div>
            {team ? (
              <p className="text-xs leading-relaxed text-muted-foreground">
                {needsProject
                  ? "Pick the project: its lead agent plans the steps and hands each to an agent."
                  : "The project's lead agent plans the steps and hands each to an agent; they show under it in the list. The team uses the project's own settings."}
              </p>
            ) : null}
            <div className="flex flex-wrap items-end gap-x-4 gap-y-3">
              {team ? null : (
                // The two switches stack on one left edge. Each label is a
                // padded box (Lockdown's turns into the sealed surface when
                // on), so the stack bleeds left by the box's padding and
                // border to line the switches up with the pickers above.
                <div className="-ml-[9px] grid min-w-0 gap-1">
                  <label className="flex items-center gap-2 rounded-xl border border-transparent px-2 py-1.5 text-sm text-foreground">
                    <Switch
                      checked={auto}
                      onCheckedChange={setAuto}
                      aria-label="Approve tool calls automatically"
                    />
                    <span className="grid">
                      <span className="font-medium">
                        Approve tools automatically
                      </span>
                      <span className="text-xs text-muted-foreground">
                        {auto
                          ? "Runs commands and edits files without stopping to ask."
                          : "Stops before each risky command and waits for your yes."}
                      </span>
                    </span>
                  </label>
                  <label
                    className={cn(
                      "flex items-center gap-2 rounded-xl border border-transparent px-2 py-1.5 text-sm text-foreground transition-colors",
                      lockdown && "lockdown-surface border-lock",
                    )}
                    data-testid="new-lockdown"
                  >
                    <Switch
                      checked={lockdown}
                      onCheckedChange={setLockdown}
                      aria-label="Lockdown (sandbox) mode"
                    />
                    <span className="grid">
                      <span className="flex items-center gap-1.5 font-medium">
                        {lockdown ? (
                          <LockIcon
                            className="lockdown-mark size-3.5 text-lock"
                            aria-hidden
                          />
                        ) : (
                          <LockOpenIcon
                            className="size-3.5 text-muted-foreground"
                            aria-hidden
                          />
                        )}
                        Lockdown (sandbox)
                      </span>
                      <span className="max-w-xs text-xs text-muted-foreground">
                        {lockdownHint(lockdown)}
                      </span>
                    </span>
                  </label>
                </div>
              )}
              <Button
                type="button"
                className="ml-auto"
                onClick={() => void start()}
                disabled={!canStart}
              >
                {busy ? (
                  <LoaderCircleIcon className="animate-spin" />
                ) : (
                  <ArrowUpIcon />
                )}{" "}
                Start
                <kbd className="ml-1 hidden rounded border border-primary-foreground/30 px-1 font-mono text-xs opacity-80 sm:inline">
                  ⌘↵
                </kbd>
              </Button>
            </div>
          </div>
        </div>
      </section>
      <NewSessionTip />
    </div>
  );
}

// ----- one conversation -----------------------------------------------------

interface Loaded {
  conversation: Conversation;
  live: ConversationLive;
  sessionId: string;
  messages: ConversationMessage[];
  // What the owner sent that the transcript does not hold yet: shown at
  // once, taken out when its row arrives (sent-messages.ts).
  sent: SentMessage[];
  steps: Record<string, HarnessStep[]>;
  artifacts: ChatArtifact[];
  changes: ChatChanges | null;
  // The conversation this one is a sub-conversation of, by title (empty
  // for a top-level one), and its own sub-conversations.
  parentTitle: string;
  children: Conversation[];
}

function ConversationView({
  id,
  seed,
  projects,
  onBack,
  onGone,
}: {
  id: string;
  seed?: Conversation;
  projects?: ProjectsPayload;
  onBack: () => void;
  onGone: () => void;
}) {
  const [d, setD] = useState<Loaded | null>(null);
  const [error, setError] = useState("");
  const dRef = useRef<Loaded | null>(null);
  const inflight = useRef(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const goneRef = useRef(onGone);
  useEffect(() => {
    goneRef.current = onGone;
  }, [onGone]);

  // One incremental fetch: messages past what we hold, steps past the last
  // one we hold for the current session. A session change starts the steps
  // over (the rebuilt agent writes a new harness session).
  const pull = useCallback(async () => {
    if (inflight.current) return;
    inflight.current = true;
    try {
      const cur = dRef.current;
      const requestedAt = Date.now();
      const since = cur ? Math.max(0, maxN(cur.messages)) : 0;
      const after =
        cur && cur.sessionId ? maxN(cur.steps[cur.sessionId] ?? []) : -1;
      let r = await api<ChatShowResponse>(
        "chat/" + enc(id) + query({ since, after }),
      );
      if (
        cur &&
        r.session_id &&
        r.session_id !== cur.sessionId &&
        !cur.steps[r.session_id]?.length
      ) {
        r = await api<ChatShowResponse>(
          "chat/" + enc(id) + query({ since, after: -1 }),
        );
      }
      const prev = dRef.current;
      const messages = mergeMessages(prev?.messages ?? [], r.messages);
      const sent = reconcileSent(
        prev?.sent ?? [],
        r.messages,
        r.live,
        requestedAt,
      );
      const steps = { ...(prev?.steps ?? {}) };
      if (r.session_id)
        steps[r.session_id] = mergeSteps(steps[r.session_id] ?? [], r.steps);
      const next: Loaded = {
        conversation: r.conversation,
        live: r.live,
        sessionId: r.session_id,
        messages,
        sent,
        steps,
        artifacts: r.artifacts,
        changes: r.changes,
        parentTitle: r.parent_title ?? "",
        children: r.children ?? [],
      };
      dRef.current = next;
      setD(next);
      setError("");
    } catch (e) {
      const t = errorText(e);
      setError(t.error || "The session did not load.");
      if (e instanceof ApiError && e.status === 404) goneRef.current();
    } finally {
      inflight.current = false;
    }
  }, [id]);

  // Steps of an older session arrive on demand, when a past turn is opened.
  const loadSession = useCallback(async (sid: string) => {
    const cur = dRef.current;
    if (!cur || cur.steps[sid]?.length) return;
    try {
      const r = await api<HarnessSessionResponse>(
        "harness/sessions/" + enc(sid),
      );
      const now = dRef.current;
      if (!now) return;
      const next = { ...now, steps: { ...now.steps, [sid]: r.steps } };
      dRef.current = next;
      setD(next);
    } catch {
      // The transcript stays readable without the old steps.
    }
  }, []);

  const status =
    d?.live.status ||
    d?.conversation.status ||
    seed?.live?.status ||
    seed?.status ||
    "";
  const fast = status === "running" || status === "waiting";
  useEffect(() => {
    let stopped = false;
    const tick = async () => {
      if (stopped) return;
      if (typeof document === "undefined" || !document.hidden) await pull();
      if (stopped) return;
      const s = dRef.current?.live.status || "";
      // A working agent streams; a second apart is what the eye follows.
      timer.current = setTimeout(
        () => void tick(),
        s === "running" || s === "waiting" ? 1000 : 5000,
      );
    };
    void tick();
    const onVis = () => {
      if (!document.hidden) void pull();
    };
    document.addEventListener("visibilitychange", onVis);
    return () => {
      stopped = true;
      if (timer.current) clearTimeout(timer.current);
      document.removeEventListener("visibilitychange", onVis);
    };
  }, [pull, fast]);

  // Any write answers with the row; take it and pull right away. A send
  // also names what was sent, so the transcript shows it now rather than
  // when the poll brings its row: until then the page showed the old
  // transcript and a bare "Working…", as if the message had gone nowhere.
  const apply = useCallback(
    (r: { conversation: Conversation }, sent?: SentMessage) => {
      const cur = dRef.current;
      if (cur) {
        const live = r.conversation.live ?? cur.live;
        const next = {
          ...cur,
          conversation: r.conversation,
          live,
          sent: sent ? [...cur.sent, sent] : cur.sent,
        };
        dRef.current = next;
        setD(next);
      }
      invalidate("chat");
      void pull();
    },
    [pull],
  );

  if (error && !d) {
    return (
      <Pane
        main={
          <div className="grid gap-3">
            <BackButton onBack={onBack} />
            <ErrorNotice error={error} onRetry={() => void pull()}>
              This session did not load.
            </ErrorNotice>
          </div>
        }
      />
    );
  }
  if (!d) {
    return (
      <Pane
        main={
          <div className="grid gap-3">
            <BackButton onBack={onBack} />
            <SkeletonRows n={6} />
          </div>
        }
      />
    );
  }
  return (
    <ConversationBody
      d={d}
      projects={projects}
      onBack={onBack}
      apply={apply}
      pull={pull}
      loadSession={loadSession}
      error={error}
    />
  );
}

function mergeMessages(
  cur: ConversationMessage[],
  next: ConversationMessage[],
): ConversationMessage[] {
  if (!next.length) return cur;
  const seen = new Set(cur.map((m) => m.n));
  const add = next.filter((m) => !seen.has(m.n));
  if (!add.length) return cur;
  return [...cur, ...add].sort((a, b) => a.n - b.n);
}

function ConversationBody({
  d,
  projects,
  onBack,
  apply,
  pull,
  loadSession,
  error,
}: {
  d: Loaded;
  projects?: ProjectsPayload;
  onBack: () => void;
  apply: (r: { conversation: Conversation }, sent?: SentMessage) => void;
  pull: () => Promise<void>;
  loadSession: (sid: string) => Promise<void>;
  error: string;
}) {
  const c = d.conversation;
  const live = d.live;
  const mood = moodOf(c, live);
  const running = live.status === "running";
  const waiting = live.status === "waiting" && !!live.pending;
  // What waits behind the turn, less what the transcript already shows as
  // just sent.
  const queued = queuedBesides(live.queued, d.sent);
  // What was steered into the running turn and not taken up yet: its row
  // is in the transcript already, so this is a hint under it, less what
  // the page still shows as just sent.
  const steered = queuedBesides(steeredOf(live), d.sent);
  const archived = c.status === "archived";
  // Lockdown (sandbox) mode: the live word follows a flip at once.
  const sealed = lockdownOf(c, live);
  // Finished by the agent or the owner; the composer stays open since a
  // message reopens it.
  const closed = live.status === "closed";
  const closedBy = closed
    ? [...d.messages]
        .reverse()
        .find((m) => m.role === "note" && typeof m.extra.closed === "string")
        ?.extra.closed
    : undefined;
  // Sub-sessions still at it, asking, or stopped by a restart once this one
  // is finished: the banner names them, since the list's tick hides them.
  const left = unfinished(d.children);
  const [busy, setBusy] = useState("");
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState(c.title);
  const [stick, setStick] = useState(true);
  const scroller = useRef<HTMLDivElement | null>(null);
  const models = useApi<HarnessModelsResponse>("harness/models", {
    refresh: 60000,
  });

  // The canvas beside the transcript: the file the agent edits, its
  // terminal, the pages it reads and its plan open there by themselves; the
  // branch's changes, the artifacts, the project and the sub-conversations
  // are a "+" away. A file from the Changes list or an artifact opens there
  // too, so the transcript never leaves the main area.
  const diff = useChatDiff(c.id, d.changes);
  const steps = useMemo(
    () => d.steps[d.sessionId] ?? [],
    [d.steps, d.sessionId],
  );
  const ctl = useCanvas({
    session: d.sessionId,
    steps,
    live: running,
    initial: [
      ...(d.changes ? [makeTab("changes")] : []),
      ...(d.artifacts.length ? [makeTab("artifacts")] : []),
      ...(d.children.length ? [makeTab("subs")] : []),
      makeTab("project"),
    ],
    ensure: d.changes ? [makeTab("changes")] : [],
  });
  // A finished session shows its report (PULSE builds it as the session is
  // shown finished): it opens in the canvas on the way in and the moment the
  // session is done.
  const reportReady = (closed || archived) && hasReportPage(d.artifacts);
  const openCanvas = ctl.open;
  const shownReport = useRef(false);
  useEffect(() => {
    if (!reportReady || shownReport.current) return;
    shownReport.current = true;
    openCanvas("artifact", REPORT_PAGE);
  }, [reportReady, openCanvas]);
  const texts = useMemo(
    () => [
      ...d.messages.map((m) => m.content),
      ...steps.filter((st) => st.kind === "fetch").map((st) => st.content),
    ],
    [d.messages, steps],
  );

  // Pick & Polish: elements picked in the canvas's Browser tab wait in the
  // composer as chips and go with the next message, so "make this teal"
  // names exactly which this. The Browser tab can also put words into the
  // composer (its ask for the pick helper); `asked.n` makes each ask new.
  const [picks, setPicks] = useState<ElementPick[]>([]);
  const [asked, setAsked] = useState({ text: "", n: 0 });
  const pickSink = {
    picks,
    add: (p: ElementPick) =>
      setPicks((cur) =>
        cur.some((x) => x.url === p.url && x.selector === p.selector)
          ? cur
          : [...cur, p],
      ),
    remove: (key: string) =>
      setPicks((cur) => cur.filter((x) => x.key !== key)),
  };

  // Follow the transcript while the reader sits at the bottom; stop when
  // they scroll up to read something, like a terminal. A message just sent
  // counts too, so it comes into view at once.
  const lastKey =
    (d.messages.length ? d.messages[d.messages.length - 1].n : 0) +
    ":" +
    d.sent.length +
    ":" +
    live.stream.length +
    ":" +
    (live.pending?.id ?? 0) +
    ":" +
    (d.steps[d.sessionId]?.length ?? 0);
  useEffect(() => {
    // The transcript's own box scrolls, not the page: scrollIntoView on
    // an end marker would also pull every overflowing ancestor along.
    const el = scroller.current;
    if (stick && el) el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
  }, [lastKey, stick]);

  const act = async (
    what: string,
    run: () => Promise<{ conversation: Conversation }>,
    done?: string,
  ) => {
    if (busy) return;
    setBusy(what);
    try {
      apply(await run());
      if (done) toast.success(done);
    } catch (e) {
      fail(e, "That did not go through");
    } finally {
      setBusy("");
    }
  };

  const saveTitle = () => {
    const t = title.trim();
    setEditing(false);
    if (!t || t === c.title) {
      setTitle(c.title);
      return;
    }
    void act("title", () => writes.chatTitle(c.id, t));
  };

  const changeModel = (model: string) => {
    if (model === (c.model || "")) return;
    void act(
      "model",
      () => writes.chatModel(c.id, model),
      model
        ? "Switched to " +
            (models.data?.models.find((m) => m.id === model)?.name || model) +
            ". It takes effect on the next turn."
        : "Back to the provider's default model on the next turn.",
    );
  };

  const changeReasoning = (reasoning: string) => {
    if (reasoning === (c.reasoning || "")) return;
    void act(
      "reasoning",
      () => writes.chatReasoning(c.id, reasoning),
      reasoning
        ? effortName(reasoning) + " effort from the next turn."
        : "Back to the effort from Settings on the next turn.",
    );
  };

  const modelName = c.model
    ? models.data?.models.find((m) => m.id === c.model)?.name || c.model
    : "";
  const changeCount = d.changes
    ? d.changes.commits + d.changes.dirty.length
    : 0;

  // The model, the auto switch and Stop sit in the composer's bottom row,
  // where the Copilot app keeps them; the header is the title and the state.
  const tools = (
    <>
      <ModelPicker
        value={c.model || ""}
        onChange={changeModel}
        models={models.data?.models ?? []}
        disabled={archived || busy === "model"}
        compact
        label={
          modelName
            ? "Model: " + modelName + " - change it for the next turn"
            : "Model: provider default - pick one for the next turn"
        }
      />
      <EffortPicker
        value={c.reasoning || ""}
        onChange={changeReasoning}
        disabled={archived || busy === "reasoning"}
        compact
        label={
          c.reasoning
            ? "Effort: " +
              effortName(c.reasoning) +
              " - change it for the next turn"
            : "Effort: from Settings - pick one for the next turn"
        }
      />
      <label
        className="flex h-7 shrink-0 items-center gap-1.5 rounded-full border border-border/70 bg-background/60 px-2.5 text-xs text-muted-foreground"
        title={
          live.auto_approve
            ? "Tools run without asking"
            : "Risky tools wait for your yes"
        }
      >
        <Switch
          size="sm"
          checked={live.auto_approve}
          disabled={archived || !!busy}
          onCheckedChange={(on) =>
            void act(
              "auto",
              () => writes.chatAuto(c.id, on),
              on
                ? "Tools now run without asking."
                : "The agent will ask before risky tools.",
            )
          }
          aria-label="Approve tools automatically"
        />
        Auto
      </label>
      <label
        className={cn(
          "flex h-7 shrink-0 items-center gap-1.5 rounded-full border px-2.5 text-xs transition-colors",
          sealed
            ? "border-lock bg-lock/10 text-lock"
            : "border-border/70 bg-background/60 text-muted-foreground",
        )}
        title={
          sealed
            ? "Lockdown (sandbox) is on: " + lockdownHint(true)
            : "Lockdown (sandbox) is off: " + lockdownHint(false)
        }
        data-testid="lockdown-toggle"
      >
        <Switch
          size="sm"
          checked={sealed}
          disabled={archived || !!busy}
          onCheckedChange={(on) =>
            void act(
              "lockdown",
              () => writes.chatLockdown(c.id, on),
              lockdownFlipMessage(on, running || waiting),
            )
          }
          aria-label="Lockdown (sandbox) mode"
        />
        {sealed ? (
          <LockIcon className="lockdown-mark size-3" aria-hidden />
        ) : (
          <LockOpenIcon className="size-3" aria-hidden />
        )}
        Lockdown
      </label>
      {running || waiting ? (
        <Button
          type="button"
          size="sm"
          variant="outline"
          className="h-7 shrink-0 rounded-full"
          onClick={() => void act("cancel", () => writes.chatCancel(c.id))}
          disabled={!!busy}
        >
          <CircleStopIcon className="text-bad" /> Stop
        </Button>
      ) : null}
    </>
  );

  const section = (
    <section className="flex min-h-0 min-w-0 flex-col gap-3 lg:flex-1">
      <BackButton onBack={onBack} />
      <header
        className={cn(
          "enter grid min-w-0 gap-2 rounded-lg border bg-card px-4 py-3 shadow-card transition-colors",
          mood.tone === "wait" && "surface-you",
          running && "border-primary/40",
          sealed && "lockdown-surface lockdown-scan",
        )}
        data-lockdown={sealed || undefined}
      >
        <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2">
          {sealed ? (
            <span className="lockdown-ring shrink-0" aria-hidden>
              <PulseMascot
                mood={mood.pip}
                size={44}
                className={cn("shrink-0", running && "float")}
                title={mood.word}
              />
            </span>
          ) : (
            <PulseMascot
              mood={mood.pip}
              size={44}
              className={cn("shrink-0", running && "float")}
              title={mood.word}
            />
          )}
          <div className="grid min-w-[14rem] flex-1 gap-0.5">
            {editing ? (
              <form
                className="flex min-w-0 items-center gap-1.5"
                onSubmit={(e) => {
                  e.preventDefault();
                  saveTitle();
                }}
              >
                <Input
                  value={title}
                  onChange={(e) => setTitle(e.target.value)}
                  onBlur={saveTitle}
                  autoFocus
                  aria-label="Session title"
                  className="h-8 max-w-md"
                  maxLength={120}
                />
                <Button
                  type="submit"
                  size="xs"
                  variant="ghost"
                  aria-label="Save title"
                >
                  <CheckIcon />
                </Button>
              </form>
            ) : (
              <button
                type="button"
                onClick={() => setEditing(true)}
                className="group flex min-w-0 items-center gap-1.5 text-left"
                title="Rename"
              >
                <span
                  className="truncate text-base font-semibold tracking-tight text-foreground"
                  title={c.title}
                >
                  {c.title || "Untitled"}
                </span>
                {sealed ? (
                  <span className="lockdown-badge" title={LOCKDOWN_BADGE}>
                    <LockIcon className="size-3" aria-hidden />
                    <span className="sr-only">{LOCKDOWN_BADGE}</span>
                    <span aria-hidden>Lockdown</span>
                  </span>
                ) : null}
                <PencilIcon
                  className="size-3.5 shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100"
                  aria-hidden
                />
              </button>
            )}
            <p className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-xs text-muted-foreground">
              <span
                className={cn(
                  "inline-flex items-center gap-1 font-medium",
                  mood.tone === "wait" ? "text-you-ink" : TONE_TEXT[mood.tone],
                )}
              >
                {running ? (
                  <LoaderCircleIcon className="size-3 animate-spin" />
                ) : (
                  <span
                    className={cn("live-dot size-1.5 rounded-full", mood.dot)}
                    aria-hidden
                  />
                )}
                {mood.word}
                {running ? <TurnClock since={live.turn_started_at} /> : null}
              </span>
              <span aria-hidden>·</span>
              {c.project ? (
                <span className="inline-flex min-w-0 items-center gap-1">
                  <ProjectLink slug={c.project} />{" "}
                  {c.branch ? (
                    <code className="truncate rounded bg-muted px-1 py-px font-mono text-xs">
                      {c.branch}
                    </code>
                  ) : null}
                </span>
              ) : (
                <span>no project yet</span>
              )}
              {c.parent ? (
                <>
                  <span aria-hidden>·</span>
                  <span className="inline-flex min-w-0 items-center gap-1">
                    <CornerDownRightIcon
                      className="size-3 shrink-0"
                      aria-hidden
                    />
                    part of{" "}
                    <Link
                      href={hrefs.conversations(c.parent)}
                      className="truncate underline-offset-4 hover:text-foreground hover:underline"
                      title="Open the session this one is part of"
                    >
                      {d.parentTitle || c.parent}
                    </Link>
                  </span>
                </>
              ) : null}
              {c.turns ? (
                <>
                  <span aria-hidden>·</span>
                  <span>{plural(c.turns, "turn")}</span>
                </>
              ) : null}
              {c.cost ? (
                <>
                  <span aria-hidden>·</span>
                  <span title="model credits used">
                    {num(Math.round(c.cost * 100) / 100)} credits
                  </span>
                </>
              ) : null}
            </p>
          </div>
          {!archived && !closed ? (
            <Button
              type="button"
              size="sm"
              variant="ghost"
              className="h-7 shrink-0 rounded-full"
              onClick={() =>
                void act(
                  "close",
                  () => writes.chatClose(c.id),
                  running || waiting
                    ? "Finished; the turn was stopped. Write again to reopen it."
                    : "Finished. Write again to reopen it.",
                )
              }
              disabled={!!busy}
              title="Finish this session; a message reopens it"
            >
              <CircleCheckIcon />
              <span className="sr-only sm:not-sr-only">Finish</span>
            </Button>
          ) : null}
          {!archived ? (
            <Button
              type="button"
              size="sm"
              variant="ghost"
              className="h-7 shrink-0 rounded-full"
              onClick={() =>
                void act(
                  "restart",
                  () => writes.chatRestart(c.id),
                  "Session restarted. Your next message reloads the session from the saved transcript.",
                )
              }
              disabled={!!busy || running || waiting}
              title={
                running || waiting
                  ? "Stop the turn first; then the session can be restarted from the saved transcript"
                  : RESTART_SESSION_HINT
              }
            >
              <RotateCcwIcon />
              <span className="sr-only sm:not-sr-only">Restart session</span>
            </Button>
          ) : null}
          {!archived ? (
            <Button
              type="button"
              size="sm"
              variant="ghost"
              className="h-7 shrink-0 rounded-full"
              onClick={() =>
                void act(
                  "archive",
                  () => writes.chatArchive(c.id),
                  "Archived. The branch stays until you land or delete it.",
                )
              }
              disabled={!!busy}
              title="Archive this session"
            >
              <ArchiveIcon />
              <span className="sr-only sm:not-sr-only">Archive</span>
            </Button>
          ) : null}
        </div>
        {sealed ? (
          <p
            className="flex min-w-0 items-start gap-2 rounded-lg border border-lock bg-lock/[0.07] px-3 py-1.5 text-xs text-foreground"
            role="status"
            data-testid="lockdown-bar"
          >
            <LockIcon
              className="lockdown-mark mt-px size-3.5 shrink-0 text-lock"
              aria-hidden
            />
            <span className="min-w-0">{LOCKDOWN_BAR}</span>
          </p>
        ) : null}
      </header>
      {error ? (
        <Notice tone="warn">{"Live updates paused: " + error}</Notice>
      ) : null}
      {archived ? (
        <Notice tone="info">
          This session is archived. Its branch and files stay where they
          were; start a new conversation to carry on.
        </Notice>
      ) : null}
      {closed && !archived ? (
        <Notice tone="info">
          {closedBy === "owner"
            ? "You finished this session."
            : (closedBy === "pulse" ? "PULSE" : "The agent") +
              " finished this session" +
              (c.note ? ": " + c.note.replace(/[.\s]*$/, "") + "." : ".")}{" "}
          Write below to reopen it, or archive it to put it away.
          {reportReady ? (
            <span className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1">
              <button
                type="button"
                className="font-medium text-link underline-offset-2 hover:underline"
                onClick={() => ctl.open("artifact", REPORT_PAGE)}
                data-report-link
              >
                {REPORT_WORD}
              </button>
              <a
                href={chatArtifactHref(c.id, REPORT_PAGE)}
                target="_blank"
                rel="noreferrer"
                className="text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
              >
                Open it in a new tab
              </a>
            </span>
          ) : null}
        </Notice>
      ) : null}
      {(closed || archived) && left.length ? (
        <Notice tone="warn">
          {left.length === 1
            ? "A sub-session didn't finish: "
            : plural(left.length, "sub-session") + " didn't finish: "}
          {left.map((k, i) => {
            const ks = sessionState(k);
            return (
              <span key={k.id}>
                {i ? "; " : ""}
                <Link
                  href={hrefs.conversations(k.id)}
                  className="font-medium underline underline-offset-2"
                >
                  {k.title || "Untitled"}
                </Link>{" "}
                {ks === "stopped"
                  ? "(stopped by a restart; open it and send \u201ccontinue\u201d)"
                  : ks === "asks"
                    ? "(waits for your answer)"
                    : "(still working)"}
              </span>
            );
          })}
          . {left.length === 1 ? "Its" : "Their"} work isn&apos;t in this
          session&apos;s pull request.
        </Notice>
      ) : null}

      <div
        className={cn(
          "relative flex min-h-0 min-w-0 flex-col overflow-hidden rounded-xl border bg-card shadow-card lg:flex-1",
          sealed && "border-lock",
        )}
      >
        <div
          aria-hidden
          className="pointer-events-none absolute inset-x-0 top-0 z-10 h-8 bg-gradient-to-b from-card to-transparent"
        />
        <div
          ref={scroller}
          onScroll={(e) => {
            const el = e.currentTarget;
            setStick(el.scrollHeight - el.scrollTop - el.clientHeight < 80);
          }}
          className={cn(
            "relative max-h-[calc(100dvh-24rem)] min-h-[20rem] min-w-0 flex-1 overflow-y-auto px-3 py-5 sm:px-6 sm:py-6 lg:max-h-none lg:min-h-0",
            sealed && "lockdown-field",
          )}
          role="log"
          aria-live="polite"
          aria-label="Transcript"
        >
          <Transcript d={d} loadSession={loadSession} />
          {running || live.status === "waiting" ? (
            <LiveTurn d={d} waiting={waiting} />
          ) : null}
          {waiting && live.pending ? (
            <PendingCard
              id={c.id}
              pending={live.pending}
              busy={busy}
              act={act}
            />
          ) : null}
          {steered.length ? (
            <ul
              className="mx-auto mt-3 grid w-full max-w-3xl gap-1"
              aria-label="Steered messages"
            >
              {steered.map((q, i) => (
                <li
                  key={i}
                  className="flex min-w-0 items-center gap-2 rounded-lg border border-dashed border-primary/40 px-2.5 py-1.5 text-xs text-muted-foreground"
                  title={STEERED_HINT}
                >
                  <ZapIcon className="size-3 shrink-0 text-primary" aria-hidden />{" "}
                  <span className="shrink-0">Steered:</span>{" "}
                  <span className="min-w-0 truncate text-foreground">{q}</span>
                  <span className="ml-auto hidden shrink-0 sm:inline">
                    {STEERED_HINT}
                  </span>
                </li>
              ))}
            </ul>
          ) : null}
          {queued.length ? (
            <ul
              className="mx-auto mt-3 grid w-full max-w-3xl gap-1"
              aria-label="Queued messages"
            >
              {queued.map((q, i) => (
                <li
                  key={i}
                  className="flex min-w-0 items-center gap-2 rounded-lg border border-dashed px-2.5 py-1.5 text-xs text-muted-foreground"
                >
                  <LoaderCircleIcon className="size-3 shrink-0 animate-spin" />{" "}
                  <span className="shrink-0">Queued:</span>{" "}
                  <span className="min-w-0 truncate text-foreground">{q}</span>
                </li>
              ))}
            </ul>
          ) : null}
        </div>
        {!stick ? (
          <button
            type="button"
            onClick={() => setStick(true)}
            className="pop absolute bottom-3 left-1/2 -translate-x-1/2 rounded-full border bg-card px-3 py-1 text-xs font-medium shadow-pop hover:bg-muted/60"
          >
            <ChevronDownIcon className="mr-1 inline size-3" aria-hidden /> Jump
            to the latest
          </button>
        ) : null}
      </div>

      <Composer
        conversation={c}
        live={live}
        disabled={archived}
        closed={closed}
        tools={tools}
        picks={picks}
        onRemovePick={pickSink.remove}
        insert={asked}
        onSent={(r, sent) => {
          setPicks([]);
          apply(r, sent);
        }}
      />
    </section>
  );

  const canvas = (
    <Canvas
      ctl={ctl}
      label="Work of this session"
      session={d.sessionId}
      steps={steps}
      live={running}
      diff={diff}
      changes={d.changes}
      chatId={c.id}
      artifacts={d.artifacts}
      texts={texts}
      picks={archived ? undefined : pickSink}
      onAsk={(text) => setAsked((a) => ({ text, n: a.n + 1 }))}
      menu={[
        "terminal",
        "browser",
        "plan",
        "changes",
        "artifacts",
        "subs",
        "project",
        "links",
      ]}
      counts={{
        changes: changeCount,
        artifacts: d.artifacts.length,
        subs: d.children.length,
      }}
      panels={{
        changes: (
          <ChangesPanel
            d={d}
            diff={diff}
            ctl={ctl}
            busy={busy}
            setBusy={setBusy}
            apply={apply}
          />
        ),
        artifacts: <ArtifactsPanel d={d} ctl={ctl} pull={pull} />,
        subs: (
          <SubConversations d={d} busy={busy} setBusy={setBusy} pull={pull} />
        ),
        project: (
          <ProjectPanel
            d={d}
            projects={projects}
            apply={apply}
            busy={busy}
            setBusy={setBusy}
          />
        ),
      }}
    />
  );

  return <Pane main={section} wide={ctl.wide} panel={canvas} />;
}

function TurnClock({ since }: { since: string }) {
  const now = useNow(1000);
  const t = Date.parse(since);
  if (!Number.isFinite(t)) return null;
  return (
    <span className="tnum text-muted-foreground">
      {span(Math.max(0, Math.round((now - t) / 1000)))}
    </span>
  );
}

// ----- transcript -----------------------------------------------------------

function Transcript({
  d,
  loadSession,
}: {
  d: Loaded;
  loadSession: (sid: string) => Promise<void>;
}) {
  if (!d.messages.length && !d.sent.length) {
    return (
      <EmptyState
        icon={<PulseMascot mood="busy" size={56} decorative />}
        hint="The first reply appears here."
      >
        Starting up…
      </EmptyState>
    );
  }
  // While a question waits, its card below carries the words; the transcript
  // row for it comes back with the answer.
  const last = d.messages[d.messages.length - 1];
  const pending = d.live.pending;
  const rows =
    last &&
    pending?.kind === "question" &&
    last.role === "question" &&
    last.content === pending.question
      ? d.messages.slice(0, -1)
      : d.messages;
  return (
    <ol className="mx-auto grid w-full max-w-3xl min-w-0 gap-6" role="list">
      {rows.map((m, i) => (
        <MessageRow
          key={m.n}
          m={m}
          status={messageStatus(m, rows.slice(i + 1))}
          d={d}
          loadSession={loadSession}
        />
      ))}
      {d.sent.map((s) => (
        <SentRow key={s.key} s={s} id={d.conversation.id} />
      ))}
    </ol>
  );
}

// What the owner just sent, in its place at the end of the transcript
// until the poll brings its row (then MessageRow takes over). Same bubble
// as a user row; the line under it says the hub has it.
function SentRow({ s, id }: { s: SentMessage; id: string }) {
  const said = splitPicks(s.text);
  return (
    <li
      className="enter group/msg grid min-w-0 justify-items-end gap-1"
      aria-label={
        s.steered ? "Steered, reaching the agent" : "Sent, awaiting the agent"
      }
    >
      <div className="grid min-w-0 max-w-[72ch] gap-2 rounded-lg bg-muted px-4 py-3 text-sm leading-relaxed text-foreground">
        <div className="whitespace-pre-wrap anywhere">{said.text}</div>
        <PickChips picks={said.picks} />
        <AttachmentChips id={id} list={s.attachments} onUser={false} />
      </div>
      <p className="flex items-center gap-1 text-xs text-muted-foreground">
        {s.steered ? (
          <ZapIcon className="size-3 text-primary" aria-hidden />
        ) : null}
        {sentLabel("sent", !!s.steered)}
      </p>
    </li>
  );
}

// The elements a message picked on a page (Pick & Polish), as chips under
// the words: the element's name and what it said, the page on hover.
function PickChips({
  picks,
}: {
  picks: { url: string; element: string; text: string; selector: string }[];
}) {
  if (!picks.length) return null;
  return (
    <ul className="flex min-w-0 flex-wrap gap-1.5" aria-label="Picked elements">
      {picks.map((p, i) => (
        <li
          key={i}
          className="flex min-w-0 max-w-72 items-center gap-1.5 rounded-lg border border-primary/30 bg-card/70 px-2 py-0.5 text-xs"
          title={[p.selector, pickPlace(p.url)].filter(Boolean).join("\n")}
        >
          <SquareDashedMousePointerIcon
            className="size-3.5 shrink-0 text-primary"
            aria-hidden
          />
          <span className="min-w-0 truncate font-mono">{p.element}</span>
          {p.text ? (
            <span className="min-w-0 truncate text-muted-foreground">
              “{p.text.length > 40 ? p.text.slice(0, 39) + "…" : p.text}”
            </span>
          ) : null}
        </li>
      ))}
    </ul>
  );
}

// The transcript reads like the Copilot app's: what you wrote on the right
// in a quiet bubble, what the agent did and said on the left as plain text
// and one line per tool call - no cards, no avatars, the time on hover.
function MessageRow({
  m,
  status,
  d,
  loadSession,
}: {
  m: ConversationMessage;
  status: MessageStatus;
  d: Loaded;
  loadSession: (sid: string) => Promise<void>;
}) {
  const role = m.role;
  const when = (
    <span className="text-xs text-muted-foreground opacity-0 transition-opacity group-hover/msg:opacity-100 motion-reduce:transition-none">
      <When d={m.at} />
    </span>
  );
  if (role === "user" || role === "answer") {
    const files = chatAttachments(m);
    // Steered in mid-turn (the composer's Steer while the agent ran): the
    // agent read it before its next step rather than starting a turn on it.
    const steered = m.extra?.steered === true;
    // Picks made in the Browser tab travel as blocks in the text; the
    // row shows them as the chips they were.
    const said = splitPicks(m.content);
    return (
      <li className="enter group/msg grid min-w-0 justify-items-end gap-1">
        <div
          className={cn(
            "grid min-w-0 max-w-[72ch] gap-2 rounded-lg px-4 py-3 text-sm leading-relaxed",
            role === "answer"
              ? "border border-primary/30 bg-primary/[0.06] text-foreground"
              : "bg-muted text-foreground",
          )}
        >
          <div className="whitespace-pre-wrap anywhere">
            {role === "answer" ? (
              <span className="mr-1 text-xs font-medium text-primary">
                You answered:
              </span>
            ) : null}
            {said.text}
          </div>
          <PickChips picks={said.picks} />
          <AttachmentChips id={d.conversation.id} list={files} onUser={false} />
        </div>
        {when}
        <p className="flex items-center gap-1 text-xs text-muted-foreground">
          {steered ? (
            <ZapIcon className="size-3 text-primary" aria-hidden />
          ) : null}
          {sentLabel(status, steered)}
        </p>
      </li>
    );
  }
  if (role === "assistant") {
    return (
      <li className="enter group/msg grid min-w-0 gap-3">
        <TurnSteps m={m} d={d} loadSession={loadSession} />
        <div className="min-w-0 max-w-[72ch] text-sm leading-relaxed">
          <Markdown
            source={m.content}
            className="[&_pre]:whitespace-pre-wrap"
          />
        </div>
        {when}
      </li>
    );
  }
  if (role === "question") {
    const choices = Array.isArray(m.extra?.choices)
      ? (m.extra.choices as string[])
      : [];
    return (
      <li className="enter grid min-w-0 gap-3">
        <TurnSteps m={m} d={d} loadSession={loadSession} />
        <div className="surface-you max-w-[72ch] rounded-lg border px-4 py-3 text-sm leading-relaxed">
          <span className="mr-1 text-xs font-semibold text-you-ink">
            Asked you:
          </span>
          <span className="whitespace-pre-wrap anywhere">{m.content}</span>
          {choices.length ? (
            <span className="mt-1 block text-xs text-muted-foreground">
              Choices: {choices.join(" / ")}
            </span>
          ) : null}
        </div>
      </li>
    );
  }
  if (role === "approval") {
    const decision = String(m.extra?.decision ?? "");
    const ok = decision === "allow" || decision === "always";
    return (
      <li className="enter flex min-w-0 items-center gap-2.5 px-1.5 text-[13px] text-muted-foreground">
        {ok ? (
          <ShieldCheckIcon className="size-3.5 shrink-0 text-ok" />
        ) : (
          <CircleAlertIcon className="size-3.5 shrink-0 text-bad" />
        )}
        <span className="min-w-0 truncate">
          <span className="font-medium text-foreground">
            {ok
              ? decision === "always"
                ? "Allowed, always"
                : "Allowed"
              : "Denied"}
          </span>{" "}
          <code className="rounded bg-muted px-1 font-mono text-xs">
            {String(m.extra?.tool ?? "")}
          </code>{" "}
          {m.content}
          {m.extra?.reason ? <span> - {String(m.extra.reason)}</span> : null}
        </span>
      </li>
    );
  }
  // A report from a sub-conversation: its words, in a bubble on the left
  // that names it and opens it, since its agent speaks here through the
  // parent's.
  const report = chatReport(m);
  if (report) {
    const title =
      report.title ||
      d.children.find((k) => k.id === report.child)?.title ||
      "";
    return (
      <li className="enter group/msg grid min-w-0 gap-3">
        <TurnSteps m={m} d={d} loadSession={loadSession} />
        <div className="grid min-w-0 max-w-[72ch] gap-1 rounded-lg border border-border/70 bg-muted/40 px-4 py-3 text-sm leading-relaxed">
          <Link
            href={hrefs.conversations(report.child)}
            className="inline-flex min-w-0 items-center gap-1 text-xs font-semibold text-primary underline-offset-4 hover:underline"
            title="Open the sub-session"
          >
            <CornerDownRightIcon className="size-3 shrink-0" aria-hidden />
            <span className="min-w-0 truncate">
              {title || "Sub-session"}
            </span>
            <span className="shrink-0 font-medium text-muted-foreground">
              · {reportVerb(report.kind)}
            </span>
          </Link>
          <Markdown
            source={m.content}
            className="[&_pre]:whitespace-pre-wrap"
          />
        </div>
        {when}
      </li>
    );
  }
  // A message from the conversation that started this one (its agent
  // steering this sub-conversation): on the right like the owner's, named.
  const from = typeof m.extra?.from === "string" ? m.extra.from : "";
  if (from) {
    return (
      <li className="enter group/msg grid min-w-0 justify-items-end gap-1">
        <div className="grid min-w-0 max-w-[72ch] gap-1 rounded-lg border border-border/70 bg-muted/60 px-4 py-3 text-sm leading-relaxed text-foreground">
          <Link
            href={hrefs.conversations(from)}
            className="inline-flex items-center gap-1 text-xs font-medium text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
            title="Open the session this one is part of"
          >
            <CornerDownRightIcon
              className="size-3 shrink-0 -scale-x-100"
              aria-hidden
            />
            From {d.parentTitle || "the session this one is part of"}
          </Link>
          <div className="whitespace-pre-wrap anywhere">{m.content}</div>
        </div>
        {when}
      </li>
    );
  }
  // The start of a sub-conversation: one line that opens it.
  const child = typeof m.extra?.child === "string" ? m.extra.child : "";
  if (child) {
    const k = d.children.find((x) => x.id === child);
    const km = k ? moodOf(k) : null;
    return (
      <li className="enter grid min-w-0 gap-3">
        <TurnSteps m={m} d={d} loadSession={loadSession} />
        <div className="flex min-w-0 items-start gap-2.5 px-1.5 text-[13px] leading-relaxed text-muted-foreground">
          <CornerDownRightIcon
            className="mt-0.5 size-3.5 shrink-0 text-primary"
            aria-hidden
          />
          <span className="min-w-0 anywhere">
            Started sub-session{" "}
            <Link
              href={hrefs.conversations(child)}
              className="font-medium text-primary underline-offset-4 hover:underline"
            >
              {String(m.extra?.title ?? "") || k?.title || child}
            </Link>
            {km ? (
              <span className={cn("ml-2 font-medium", TONE_TEXT[km.tone])}>
                {km.word}
              </span>
            ) : null}
          </span>
        </div>
      </li>
    );
  }
  // A note: the hub's own voice (session ended, moved into a project, stopped).
  const moved = m.extra?.project ? String(m.extra.project) : "";
  return (
    <li className="enter grid min-w-0 gap-3">
      <TurnSteps m={m} d={d} loadSession={loadSession} />
      <div className="flex min-w-0 items-start gap-2.5 px-1.5 text-[13px] leading-relaxed text-muted-foreground">
        {moved ? (
          <FolderGitIcon className="mt-0.5 size-3.5 shrink-0 text-primary" />
        ) : (
          <CircleAlertIcon className="mt-0.5 size-3.5 shrink-0" />
        )}
        <span className="min-w-0 whitespace-pre-wrap anywhere">
          {m.content}
          {moved ? (
            <>
              {" "}
              <Link
                href={hrefs.project(moved)}
                className="text-primary underline-offset-4 hover:underline"
              >
                Open {moved}
              </Link>
            </>
          ) : null}
        </span>
      </div>
    </li>
  );
}

// What the agent did on the way to this message, in the transcript itself.
// The steps of the session the conversation runs in are already here; an
// older session's load on request, one line until they do.
function TurnSteps({
  m,
  d,
  loadSession,
}: {
  m: ConversationMessage;
  d: Loaded;
  loadSession: (sid: string) => Promise<void>;
}) {
  const r = stepRange(m);
  const [asked, setAsked] = useState(false);
  if (!r) return null;
  const all = d.steps[r.session];
  if (!all) {
    if (asked) return <SkeletonRows n={2} />;
    return (
      <button
        type="button"
        onClick={() => {
          setAsked(true);
          void loadSession(r.session);
        }}
        className="flex w-fit items-center gap-2.5 rounded-md px-1.5 py-1 text-[13px] text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <ChevronRightIcon className="size-3.5 shrink-0" aria-hidden /> Show what
        it did
      </button>
    );
  }
  const steps = turnSteps(
    all,
    r.from,
    r.to,
    m.role === "assistant" ? m.content : "",
  );
  if (!steps.length) return null;
  return <Activity steps={steps} />;
}

// The running turn, in the order it happens: what the agent wrote, the
// tools it ran (one line each), more words - and last the text still being
// written, with a caret. When it waits on a question, the card below
// carries the question itself.
function LiveTurn({ d, waiting }: { d: Loaded; waiting: boolean }) {
  const live = d.live;
  const all = d.steps[d.sessionId];
  const question =
    waiting && live.pending?.kind === "question" ? live.pending.question : "";
  const steps = useMemo(() => {
    const base = turnSteps(all ?? [], live.turn_from, null, "");
    if (!question) return base;
    // The words that led to the question often repeat it; the card below
    // carries the question, so the last segment is cut where it starts.
    for (let i = base.length - 1; i >= 0; i--) {
      const s = base[i];
      if (s.kind !== "assistant" || !s.content.trim()) continue;
      const cut = beforeQuestion(s.content, question);
      if (cut === s.content) break;
      const copy = base.slice();
      copy[i] = { ...s, content: cut };
      return copy;
    }
    return base;
  }, [all, live.turn_from, question]);
  const last = steps.length ? steps[steps.length - 1] : null;
  const tail = streamTail(live.stream, steps);
  const stream = question ? beforeQuestion(tail, question) : tail;
  return (
    <div className="enter mx-auto mt-6 grid w-full max-w-3xl min-w-0 gap-3">
      {steps.length ? <Activity steps={steps} live={!waiting} /> : null}
      {stream.trim() ? (
        <div className="min-w-0 max-w-[72ch] text-sm leading-relaxed">
          <Markdown source={stream} className="[&_pre]:whitespace-pre-wrap" />
          {waiting ? null : (
            <span
              className="ml-0.5 inline-block h-4 w-1.5 translate-y-0.5 animate-pulse rounded-sm bg-primary/70"
              aria-hidden
            />
          )}
        </div>
      ) : null}
      <div
        className={cn(
          "flex min-w-0 items-center gap-2.5 px-1.5 text-[13px]",
          waiting ? "text-you-ink" : "text-muted-foreground",
        )}
      >
        {waiting ? (
          <ShieldQuestionIcon className="size-3.5 shrink-0" />
        ) : (
          <TypingDots className="shrink-0 text-primary" />
        )}
        <span className="min-w-0 truncate">
          {waiting
            ? "Paused - it needs you (below)"
            : last && !stream.trim()
              ? lastStepWords(last)
              : "Working…"}
        </span>
        <TurnClock since={live.turn_started_at} />
      </div>
    </div>
  );
}

// What the model said before it asked: a reply that repeats the question
// (and lists the choices after it) would show twice beside the card, so
// the stream is cut where the question starts.
function beforeQuestion(stream: string, question: string): string {
  const q = question.trim();
  if (!q) return stream;
  const i = stream.indexOf(q);
  if (i >= 0) return stream.slice(0, i).trimEnd();
  const first = q.split(/(?<=[.?!])\s/)[0];
  if (first && first.length >= 24) {
    const j = stream.indexOf(first);
    if (j >= 0) return stream.slice(0, j).trimEnd();
  }
  return stream;
}

function lastStepWords(s: HarnessStep): string {
  switch (s.kind) {
    case "command":
      return "Running: " + ((s.title ?? "").trim() || s.command || s.content);
    case "observation":
      return "Reading the result…";
    case "plan":
      return "Planning: " + ((s.content || "").split("\n")[0] || "");
    case "subagent":
      return "Asked a helper: " + (s.command || s.content);
    case "assistant":
      return "Thinking…";
    case "note":
      return s.content;
    default:
      return (s.kind + ": " + (s.command || s.content)).slice(0, 160);
  }
}

// ----- the agent waits ------------------------------------------------------

function PendingCard({
  id,
  pending,
  busy,
  act,
}: {
  id: string;
  pending: ChatPending;
  busy: string;
  act: (
    what: string,
    run: () => Promise<{ conversation: Conversation }>,
    done?: string,
  ) => Promise<void>;
}) {
  const [text, setText] = useState("");
  const [reason, setReason] = useState("");
  const [denying, setDenying] = useState(false);
  if (pending.kind === "question") {
    return (
      <div className="mx-auto mt-4 w-full max-w-3xl min-w-0">
        <div
          className="pop surface-you grid min-w-0 gap-3 rounded-lg border-2 px-4 py-3 shadow-card"
          role="group"
          aria-label="The agent asked a question"
        >
          <div className="flex items-start gap-2">
            <ShieldQuestionIcon className="mt-0.5 size-4 shrink-0 text-you-ink" />
            <p className="min-w-0 text-sm leading-relaxed font-medium whitespace-pre-wrap text-foreground anywhere">
              {pending.question}
            </p>
          </div>
          {pending.choices.length ? (
            <div
              className="grid min-w-0 gap-1.5"
              role="group"
              aria-label="Choices"
            >
              {pending.choices.map((ch) => (
                <Button
                  key={ch}
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={!!busy}
                  onClick={() =>
                    void act("answer", () => writes.chatAnswer(id, ch))
                  }
                  className="pop h-auto min-h-8 w-full min-w-0 justify-start py-1.5 text-left whitespace-normal anywhere"
                >
                  {ch}
                </Button>
              ))}
            </div>
          ) : null}
          <form
            className="flex min-w-0 gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              const t = text.trim();
              if (!t) return;
              void act("answer", () => writes.chatAnswer(id, t)).then(() =>
                setText(""),
              );
            }}
          >
            <Input
              value={text}
              onChange={(e) => setText(e.target.value)}
              placeholder={
                pending.choices.length
                  ? "Or answer in your own words…"
                  : "Your answer…"
              }
              aria-label="Your answer"
              autoFocus
            />
            <Button
              type="submit"
              size="default"
              disabled={!text.trim() || !!busy}
            >
              {busy === "answer" ? (
                <LoaderCircleIcon className="animate-spin" />
              ) : (
                <ArrowUpIcon />
              )}{" "}
              Answer
            </Button>
          </form>
        </div>
      </div>
    );
  }
  return (
    <div className="mx-auto mt-4 w-full max-w-3xl min-w-0">
      <div
        className="pop surface-you grid min-w-0 gap-3 rounded-lg border-2 px-4 py-3 shadow-card"
        role="group"
        aria-label="The agent needs an approval"
      >
        <div className="flex items-start gap-2">
          <ShieldCheckIcon className="mt-0.5 size-4 shrink-0 text-you-ink" />
          <div className="grid min-w-0 gap-1">
            <p className="text-sm leading-relaxed font-medium text-foreground">
              May it run{" "}
              <code className="rounded bg-muted px-1 font-mono text-[13px]">
                {pending.tool}
              </code>
              ?
            </p>
            <p className="text-sm leading-relaxed whitespace-pre-wrap text-foreground anywhere">
              {pending.summary}
            </p>
            {pending.reason ? (
              <p className="text-[13px] text-muted-foreground">
                {pending.reason}
              </p>
            ) : null}
          </div>
        </div>
        {pending.detail ? (
          <pre className="max-h-56 min-w-0 overflow-auto rounded-xl bg-zinc-950 px-3 py-2.5 font-mono text-[13px] leading-relaxed whitespace-pre-wrap text-zinc-100 anywhere dark:bg-black">
            {pending.detail}
          </pre>
        ) : null}
        {denying ? (
          <form
            className="flex min-w-0 gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              void act("approve", () =>
                writes.chatApprove(id, "deny", reason.trim()),
              ).then(() => {
                setDenying(false);
                setReason("");
              });
            }}
          >
            <Input
              value={reason}
              onChange={(e) => setReason(e.target.value)}
              placeholder="Why not? (the agent reads this)"
              aria-label="Reason"
              autoFocus
            />
            <Button type="submit" variant="destructive" disabled={!!busy}>
              Deny
            </Button>
            <Button
              type="button"
              variant="ghost"
              onClick={() => setDenying(false)}
            >
              Back
            </Button>
          </form>
        ) : (
          <div className="flex flex-wrap items-center gap-2">
            <Button
              type="button"
              disabled={!!busy}
              onClick={() =>
                void act("approve", () => writes.chatApprove(id, "allow"))
              }
            >
              <CheckIcon /> Allow once
            </Button>
            {pending.always ? (
              <Button
                type="button"
                variant="outline"
                disabled={!!busy}
                onClick={() =>
                  void act(
                    "approve",
                    () => writes.chatApprove(id, "always"),
                    "Allowed for the rest of this session.",
                  )
                }
                title={pending.always}
              >
                Always allow
              </Button>
            ) : null}
            <Button
              type="button"
              variant="ghost"
              disabled={!!busy}
              onClick={() => setDenying(true)}
              className="text-bad hover:text-bad"
            >
              <XIcon /> Deny
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}

// ----- composer -------------------------------------------------------------

// Composer: the text on top, the tools underneath - attach, dictate, the
// model, the auto switch and Stop on the left, send on the right - as the
// Copilot app lays it out.
function Composer({
  conversation,
  live,
  disabled,
  closed = false,
  tools,
  picks = [],
  onRemovePick,
  insert,
  onSent,
}: {
  conversation: Conversation;
  live: ConversationLive;
  disabled: boolean;
  closed?: boolean;
  tools?: ReactNode;
  /** Elements picked in the Browser tab, sent with the next message. */
  picks?: ElementPick[];
  onRemovePick?: (key: string) => void;
  /** Words another part of the page puts into the box; a new `n` is a new insertion. */
  insert?: { text: string; n: number };
  onSent: (r: { conversation: Conversation }, sent?: SentMessage) => void;
}) {
  const [text, setText] = useState("");
  const [picked, setPicked] = useState<Picked[]>([]);
  const [busy, setBusy] = useState("");
  // Which way the message in flight goes, so the right button spins.
  const [sending, setSending] = useState<ComposeMode | "">("");
  const [over, setOver] = useState(false);
  const fileInput = useRef<HTMLInputElement | null>(null);
  const textarea = useRef<HTMLTextAreaElement | null>(null);
  const running = turnOpen(live.status);
  const sealed = lockdownOf(conversation, live);
  const canSend =
    (!!text.trim() || picked.length > 0 || picks.length > 0) &&
    !busy &&
    !disabled;

  // Words put in from outside (the Browser tab's ask for its helper) join
  // what is there, and the box takes focus so Enter sends them.
  const [inserted, setInserted] = useState(0);
  if (insert && insert.n !== inserted) {
    setInserted(insert.n);
    if (insert.text)
      setText((cur) => (cur.trim() ? cur.replace(/\s*$/, "\n\n") : "") + insert.text);
  }
  useEffect(() => {
    if (insert?.n) textarea.current?.focus();
  }, [insert?.n]);

  // Dictated words go where the caret is (or over the selection), and the
  // caret moves past them so the next take follows on.
  const insertSpokenText = useDictation(textarea, text, setText);

  const addFiles = (files: FileList | File[] | null | undefined) => {
    if (disabled) return;
    setPicked((cur) => pickFiles(cur, files));
  };

  // Files go up one at a time, then the message names what the hub kept
  // (a name the hub renumbered comes back renumbered). The sent message
  // goes up with the answer so the transcript shows it at once.
  //
  // With a turn open the message goes one of two ways (compose-mode.ts):
  // steer puts it into the running turn, where the agent reads it before
  // its next step; queue holds it for after. Idle, there is only send.
  const send = async (mode: ComposeMode = composeMode(live.status, false)) => {
    if (!canSend) return;
    // The picked elements ride inside the text as blocks the agent reads
    // and the transcript folds back into chips (lib/pick.ts).
    const t = withPicks(text.trim(), picks);
    const files = picked;
    setSending(mode);
    setBusy(files.length ? "upload" : mode);
    try {
      const kept: ChatAttachment[] = [];
      for (let i = 0; i < files.length; i++) {
        setBusy("upload " + (i + 1) + "/" + files.length);
        kept.push(await chatUpload(conversation.id, files[i].file));
      }
      setBusy(mode);
      const sentAt = Date.now();
      const names = kept.map((a) => a.name);
      const r =
        mode === "steer"
          ? await writes.chatSteer(conversation.id, t, names)
          : await writes.chatSend(conversation.id, t, names);
      setText("");
      setPicked([]);
      // A steer's row is in the transcript already, so the page shows it
      // now (unless the hub had to queue it after all, which its answer
      // says). Behind a running turn a queued message is listed as such;
      // sent to an idle agent it has no place yet, so the transcript
      // shows it.
      const shown = sentMessage(t, kept, sentAt, mode === "steer");
      onSent(
        r,
        showsNow(mode, live.status, r.conversation.live, shown.text)
          ? shown
          : undefined,
      );
    } catch (e) {
      fail(
        e,
        mode === "steer"
          ? "The message did not reach the agent"
          : "The message did not send",
      );
    } finally {
      setBusy("");
      setSending("");
    }
  };

  const uploading = busy.startsWith("upload");
  return (
    <form
      className="grid min-w-0 shrink-0 gap-1.5"
      onSubmit={(e) => {
        e.preventDefault();
        void send();
      }}
      onDragOver={(e) => {
        if (disabled || !e.dataTransfer.types.includes("Files")) return;
        e.preventDefault();
        setOver(true);
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(e) => {
        if (disabled || !e.dataTransfer.files.length) return;
        e.preventDefault();
        setOver(false);
        addFiles(e.dataTransfer.files);
      }}
    >
      <div
        className={cn(
          "grid min-w-0 gap-2 rounded-lg border bg-card p-2 shadow-card transition-colors focus-within:border-ring focus-within:ring-3 focus-within:ring-ring/25",
          over && "border-primary ring-3 ring-primary/25",
          sealed &&
            "border-lock focus-within:border-lock focus-within:ring-lock/20",
        )}
      >
        {picks.length ? (
          <ul
            className="flex min-w-0 flex-wrap items-center gap-1.5 px-1 pt-1"
            aria-label="Picked elements"
          >
            {picks.map((p) => (
              <li
                key={p.key}
                className="pop flex min-w-0 max-w-72 items-center gap-1.5 rounded-lg border border-primary/30 bg-primary/[0.06] py-1 pr-1 pl-2 text-xs"
                title={[p.selector, pickPlace(p.url)].filter(Boolean).join("\n")}
              >
                <SquareDashedMousePointerIcon
                  className="size-3.5 shrink-0 text-primary"
                  aria-hidden
                />
                <span className="min-w-0 truncate font-mono text-foreground">
                  {pickLabel(p)}
                </span>
                <button
                  type="button"
                  onClick={() => onRemovePick?.(p.key)}
                  disabled={!!busy}
                  className="rounded-md p-0.5 text-muted-foreground hover:bg-muted hover:text-foreground"
                  aria-label={"Remove " + pickLabel(p)}
                >
                  <XIcon className="size-3" />
                </button>
              </li>
            ))}
          </ul>
        ) : null}
        {picked.length ? (
          <ul
            className="flex min-w-0 flex-wrap gap-1.5 px-1 pt-1"
            aria-label="Files to attach"
          >
            {picked.map((p) => {
              const Icon = fileIcon(p.file.name);
              return (
                <li
                  key={p.key}
                  className="pop flex min-w-0 max-w-64 items-center gap-1.5 rounded-lg border bg-muted/40 py-1 pr-1 pl-2 text-xs"
                >
                  <Icon
                    className="size-3.5 shrink-0 text-muted-foreground"
                    aria-hidden
                  />
                  <span
                    className="min-w-0 truncate font-medium text-foreground"
                    title={p.file.name}
                  >
                    {p.file.name}
                  </span>
                  <span className="shrink-0 text-muted-foreground tnum">
                    {bytesWord(p.file.size)}
                  </span>
                  <button
                    type="button"
                    onClick={() =>
                      setPicked((cur) => cur.filter((x) => x.key !== p.key))
                    }
                    disabled={!!busy}
                    className="rounded-md p-0.5 text-muted-foreground hover:bg-muted hover:text-foreground"
                    aria-label={"Remove " + p.file.name}
                  >
                    <XIcon className="size-3" />
                  </button>
                </li>
              );
            })}
          </ul>
        ) : null}
        <Textarea
          ref={textarea}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (
              e.key === "Enter" &&
              !e.shiftKey &&
              !e.nativeEvent.isComposing
            ) {
              e.preventDefault();
              // Idle: Enter sends. With a turn open: Enter steers it,
              // ⌘/Ctrl+Enter queues for after.
              void send(composeMode(live.status, e.metaKey || e.ctrlKey));
            }
          }}
          onPaste={(e) => {
            const files = Array.from(e.clipboardData?.files ?? []);
            if (!files.length) return;
            e.preventDefault();
            addFiles(files);
          }}
          placeholder={
            disabled
              ? "This session is archived."
              : running
                ? composePlaceholder(true)
                : closed
                  ? "Write to reopen this session…"
                  : picks.length
                    ? "Say what to change about what you picked, or just send it."
                    : picked.length
                      ? "Say what to do with the files, or just send them."
                      : composePlaceholder(false)
          }
          disabled={disabled}
          rows={1}
          className="min-h-9 max-h-48 resize-none border-0 bg-transparent px-2.5 py-2 text-sm shadow-none focus-visible:ring-0 dark:bg-transparent"
          aria-label="Message"
        />
        <div className="flex min-w-0 items-center gap-1.5">
          <input
            ref={fileInput}
            type="file"
            multiple
            className="sr-only"
            tabIndex={-1}
            aria-hidden
            onChange={(e) => {
              addFiles(e.target.files);
              e.target.value = "";
            }}
          />
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                type="button"
                size="icon"
                variant="ghost"
                onClick={() => fileInput.current?.click()}
                disabled={disabled || !!busy}
                aria-label="Attach files"
                className="shrink-0 text-muted-foreground hover:text-foreground"
              >
                <PaperclipIcon />
              </Button>
            </TooltipTrigger>
            <TooltipContent>
              Attach files (up to 8 MB each). You can also drop or paste them.
            </TooltipContent>
          </Tooltip>
          <MicButton disabled={disabled || !!busy} onText={insertSpokenText} />
          <div className="flex min-w-0 flex-1 flex-wrap items-center gap-1.5">
            {tools}
          </div>
          {running ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  type="button"
                  size="icon"
                  variant="outline"
                  onClick={() => void send("queue")}
                  disabled={!canSend}
                  aria-label={QUEUE_LABEL}
                  className="shrink-0"
                >
                  {sending === "queue" ? (
                    <LoaderCircleIcon className="animate-spin" />
                  ) : (
                    <ListEndIcon />
                  )}
                </Button>
              </TooltipTrigger>
              <TooltipContent>{QUEUE_TITLE}</TooltipContent>
            </Tooltip>
          ) : null}
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                type="submit"
                size="icon"
                disabled={!canSend}
                aria-label={running ? STEER_LABEL : SEND_LABEL}
                className="shrink-0"
              >
                {sending && sending !== "queue" ? (
                  <LoaderCircleIcon className="animate-spin" />
                ) : running ? (
                  <ZapIcon />
                ) : (
                  <ArrowUpIcon />
                )}
              </Button>
            </TooltipTrigger>
            <TooltipContent>{running ? STEER_TITLE : SEND_TITLE}</TooltipContent>
          </Tooltip>
        </div>
      </div>
      <p className="flex min-w-0 flex-wrap items-center gap-x-2 px-2 text-xs text-muted-foreground">
        {uploading ? (
          <span className="text-primary">
            Uploading {busy.slice("upload ".length) || "…"}
          </span>
        ) : over ? (
          <span className="text-primary">Drop to attach</span>
        ) : (
          <span>
            {composeKeysHint(running)} Drop or paste files to attach them, or
            press the microphone to dictate.
          </span>
        )}
        {conversation.cwd ? (
          <span className="min-w-0 truncate" title={conversation.cwd}>
            Working in <code className="font-mono">{conversation.cwd}</code>.
          </span>
        ) : null}
        {sealed ? (
          <span className="inline-flex items-center gap-1 text-lock">
            <LockIcon className="size-3" aria-hidden /> In lockdown: only work
            inside the sandbox.
          </span>
        ) : null}
      </p>
    </form>
  );
}

// ----- side panel: project, changes, artifacts ------------------------------

// The conversation's sub-conversations: the ones its agent started with the
// chat tool, or the owner here. Each opens on its own page; a new one takes
// the brief typed here and works in this conversation's workspace,
// reporting back into this transcript.
function SubConversations({
  d,
  busy,
  setBusy,
  pull,
}: {
  d: Loaded;
  busy: string;
  setBusy: (s: string) => void;
  pull: () => Promise<void>;
}) {
  const c = d.conversation;
  const router = useRouter();
  const now = useNow(15000);
  const [open, setOpen] = useState(false);
  const [title, setTitle] = useState("");
  const [text, setText] = useState("");
  const brief = useRef<HTMLTextAreaElement | null>(null);
  const insertSpokenText = useDictation(brief, text, setText);
  const archived = c.status === "archived";
  const kids = d.children;
  if (!kids.length && archived) return null;

  const start = async () => {
    const brief = text.trim();
    if (!brief || busy) return;
    setBusy("sub");
    try {
      const r = await writes.chatCreate({
        text: brief,
        title: title.trim(),
        parent: c.id,
      });
      toast.success(
        "Started " +
          (r.conversation.title || "the sub-session") +
          ". Its replies come back here.",
      );
      setText("");
      setTitle("");
      setOpen(false);
      invalidate("chat");
      void pull();
      router.replace(hrefs.conversations(r.conversation.id), { scroll: false });
    } catch (e) {
      fail(e, "Could not start a sub-session");
    } finally {
      setBusy("");
    }
  };

  return (
    <section className="grid min-w-0 gap-1.5" aria-label="Sub-sessions">
      <h3 className="flex items-center gap-1.5 px-1 text-xs font-semibold tracking-wide text-muted-foreground uppercase">
        <CornerDownRightIcon className="size-3" aria-hidden />
        Sub-sessions
        {kids.length ? (
          <span className="tnum font-normal">{kids.length}</span>
        ) : null}
        {!archived ? (
          <button
            type="button"
            className="ml-auto font-normal normal-case tracking-normal underline-offset-4 hover:text-foreground hover:underline"
            onClick={() => setOpen((v) => !v)}
            aria-expanded={open}
          >
            {open ? "Cancel" : "Start one"}
          </button>
        ) : null}
      </h3>
      {kids.length ? (
        <ul
          className="grid min-w-0 gap-px rounded-lg border bg-muted/20 py-1"
          role="list"
        >
          {kids.map((k) => {
            const km = moodOf(k);
            return (
              <li key={k.id} className="min-w-0">
                <Link
                  href={hrefs.conversations(k.id)}
                  className="flex w-full min-w-0 items-center gap-2 px-2.5 py-1.5 text-left text-xs transition-colors hover:bg-muted/50"
                  title={k.note || k.title}
                >
                  <span
                    className={cn(
                      "live-dot size-1.5 shrink-0 rounded-full",
                      km.dot,
                    )}
                    data-live={(k.live?.status || k.status) === "running"}
                    aria-hidden
                  />
                  <span className="min-w-0 flex-1 truncate text-foreground">
                    {k.title || "Untitled"}
                  </span>
                  <span
                    className={cn("shrink-0 font-medium", TONE_TEXT[km.tone])}
                  >
                    {km.word}
                  </span>
                  <span className="shrink-0 tnum text-muted-foreground">
                    {relShort(k.last_at || k.created_at, now)}
                  </span>
                </Link>
              </li>
            );
          })}
        </ul>
      ) : !open && c.status !== "closed" ? (
        <p className="px-1 text-xs leading-relaxed text-muted-foreground">
          None yet. The agent starts one for a part of the work it wants watched
          on its own page; you can start one here too. Its replies come back
          into this session.
        </p>
      ) : null}
      {open ? (
        <form
          className="grid min-w-0 gap-1.5 px-0.5"
          onSubmit={(e) => {
            e.preventDefault();
            void start();
          }}
        >
          <Input
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            placeholder="Title (optional)"
            aria-label="Sub-session title"
            maxLength={120}
            className="h-8"
          />
          <Textarea
            ref={brief}
            value={text}
            onChange={(e) => setText(e.target.value)}
            placeholder="What should it do? It works in this session's workspace and reports back here."
            aria-label="Sub-session brief"
            rows={3}
            className="min-h-16 text-[13px]"
          />
          <div className="flex items-center gap-1.5">
            <MicButton disabled={!!busy} onText={insertSpokenText} />
            <Button
              type="submit"
              size="sm"
              disabled={!text.trim() || !!busy}
              className="ml-auto"
            >
              {busy === "sub" ? (
                <LoaderCircleIcon className="animate-spin" />
              ) : (
                <MessageSquarePlusIcon />
              )}{" "}
              Start
            </Button>
          </div>
        </form>
      ) : null}
    </section>
  );
}

// What is open on the canvas, as the Changes and Artifacts lists see it:
// the file whose tab is in front, the whole diff, or the artifact.
function canvasView(ctl: CanvasControl, diff: ChatDiff): ChatView | null {
  const t = ctl.tabs.find((x) => x.id === ctl.active);
  if (!t) return null;
  if (t.kind === "diff") return { kind: "changes" };
  if (t.kind === "artifact") return { kind: "artifact", name: t.key };
  if (t.kind === "file") {
    const i = diff.files?.findIndex((f) => f.path === t.key) ?? -1;
    return i >= 0 ? { kind: "changes", file: i } : null;
  }
  return null;
}

function ChangesPanel({
  d,
  diff,
  ctl,
  busy,
  setBusy,
  apply,
}: {
  d: Loaded;
  diff: ChatDiff;
  ctl: CanvasControl;
  busy: string;
  setBusy: (s: string) => void;
  apply: (r: { conversation: Conversation }, sent?: SentMessage) => void;
}) {
  const c = d.conversation;
  const ch = d.changes;
  const archived = c.status === "archived";
  const view = canvasView(ctl, diff);
  const onView = (v: ChatView) => {
    if (v.kind === "artifact") return ctl.open("artifact", v.name);
    const path = v.file === undefined ? "" : diff.files?.[v.file]?.path || "";
    return path ? ctl.open("file", path) : ctl.open("diff");
  };

  const askToLand = async () => {
    if (!ch || busy) return;
    setBusy("land");
    try {
      const text =
        "Please propose this branch for review: commit what is left, run the project's check, then run `pulse land --project " +
        ch.project +
        " --branch " +
        ch.branch +
        " --propose` (it pushes the branch and opens a pull request; the review lead reviews and merges it). Tell me the pull request url.";
      const sentAt = Date.now();
      const shows = sentShowsNow(d.live.status);
      apply(
        await writes.chatSend(c.id, text),
        shows ? sentMessage(text, [], sentAt) : undefined,
      );
      toast.success("Asked the agent to open a pull request for review.");
    } catch (e) {
      fail(e, "The request did not send");
    } finally {
      setBusy("");
    }
  };

  if (!ch)
    return (
      <p className="px-1 text-[13px] leading-relaxed text-muted-foreground">
        Changes appear once the session has a project and a branch.
      </p>
    );
  return (
    <>
      <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 px-1 text-xs text-muted-foreground">
        <code
          className="min-w-0 truncate rounded bg-muted px-1 py-px font-mono text-xs text-foreground"
          title={ch.branch}
        >
          {ch.branch}
        </code>
        <span>
          {ch.commits ? plural(ch.commits, "commit") : "no commits yet"}
        </span>
        {ch.dirty.length ? (
          <span className="font-medium text-you-ink">
            {plural(ch.dirty.length, "file")} not committed
          </span>
        ) : null}
      </div>
      {!ch.commits && !ch.dirty.length ? (
        <EmptyState
          icon={<GitBranchIcon className="size-[18px]" />}
          hint="The agent's edits and commits on its branch show up here, file by file; pick one to read it."
        >
          Nothing changed yet
        </EmptyState>
      ) : (
        <ChangeList diff={diff} view={view} onView={onView} />
      )}
      {(ch.commits || ch.dirty.length) && !archived ? (
        <div className="flex flex-wrap items-center gap-1.5">
          <ReviewAllButton
            active={view?.kind === "changes" && view.file === undefined}
            onClick={() => ctl.open("diff")}
          />
          {ch.commits ? (
            <Button
              type="button"
              size="sm"
              onClick={() => void askToLand()}
              disabled={!!busy}
              title={
                "Asks the agent to run pulse land --project " +
                ch.project +
                " --branch " +
                ch.branch +
                " --propose: a pull request the review lead merges"
              }
            >
              {busy === "land" ? (
                <LoaderCircleIcon className="animate-spin" />
              ) : (
                <CheckIcon />
              )}{" "}
              Open PR for review
            </Button>
          ) : null}
        </div>
      ) : null}
    </>
  );
}

function ArtifactsPanel({
  d,
  ctl,
  pull,
}: {
  d: Loaded;
  ctl: CanvasControl;
  pull: () => Promise<void>;
}) {
  const t = ctl.tabs.find((x) => x.id === ctl.active);
  const view: ChatView | null =
    t?.kind === "artifact" ? { kind: "artifact", name: t.key } : null;
  return (
    <>
      {hasReportPage(d.artifacts) ? (
        <ReportLink
          href={chatArtifactHref(d.conversation.id, REPORT_PAGE)}
          tip={reportTip("session")}
          className="justify-self-start px-1 text-[13px]"
        />
      ) : null}
      {d.artifacts.length ? (
        <ArtifactList
          id={d.conversation.id}
          artifacts={d.artifacts}
          view={view}
          onView={(v) =>
            v.kind === "artifact"
              ? ctl.open("artifact", v.name)
              : ctl.open("diff")
          }
        />
      ) : (
        <EmptyState
          icon={<PaperclipIcon className="size-[18px]" />}
          hint="It knows the folder as $PULSE_ARTIFACTS."
        >
          Files the agent saves for you - screenshots, reports, pages - show up
          here.
        </EmptyState>
      )}
      <button
        type="button"
        onClick={() => void pull()}
        className="flex items-center gap-1 justify-self-start px-1 text-xs text-muted-foreground underline-offset-4 hover:underline"
      >
        <RefreshCwIcon className="size-3" /> Refresh
      </button>
    </>
  );
}

function ProjectPanel({
  d,
  projects,
  apply,
  busy,
  setBusy,
}: {
  d: Loaded;
  projects?: ProjectsPayload;
  apply: (r: { conversation: Conversation }) => void;
  busy: string;
  setBusy: (s: string) => void;
}) {
  const c = d.conversation;
  const [attachTo, setAttachTo] = useState("");
  const list = (projects?.projects ?? []).filter(
    (p) => p.status !== "inactive",
  );
  const archived = c.status === "archived";

  const attach = async () => {
    if (!attachTo || busy) return;
    setBusy("attach");
    try {
      const r = await writes.chatAttach(c.id, attachTo);
      apply({ conversation: r.conversation });
      toast.success(
        r.attached.created
          ? "Attached. The agent moves into " + attachTo + " after this turn."
          : "Attached to " + attachTo + ".",
      );
      setAttachTo("");
    } catch (e) {
      fail(e, "Could not attach the project");
    } finally {
      setBusy("");
    }
  };

  if (c.project) {
    return (
      <div className="grid gap-1.5 px-1 text-sm">
        <ProjectLink slug={c.project} />
        {c.branch ? (
          <span className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
            <GitBranchIcon className="size-3.5 shrink-0" />{" "}
            <code className="truncate font-mono">{c.branch}</code>
          </span>
        ) : null}
        {c.cwd ? (
          <span
            className="truncate text-xs text-muted-foreground"
            title={c.cwd}
          >
            {c.cwd}
          </span>
        ) : null}
        <span className="text-xs text-muted-foreground">
          Model:{" "}
          <span className="text-foreground">
            {c.model || "provider default"}
          </span>
        </span>
      </div>
    );
  }
  return (
    <div className="grid gap-2 px-1">
      <p className="text-[13px] leading-relaxed text-muted-foreground">
        No project yet. The agent reads your projects and attaches one itself,
        or pick it here.
      </p>
      {!archived ? (
        <div className="flex min-w-0 gap-1.5">
          <Select value={attachTo || undefined} onValueChange={setAttachTo}>
            <SelectTrigger
              className="min-w-0 flex-1"
              aria-label="Project to attach"
            >
              <SelectValue placeholder="Pick a project" />
            </SelectTrigger>
            <SelectContent>
              {list.map((p) => (
                <SelectItem key={p.slug} value={p.slug}>
                  {projectLabel(p)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Button
            type="button"
            size="default"
            variant="outline"
            disabled={!attachTo || !!busy}
            onClick={() => void attach()}
          >
            {busy === "attach" ? (
              <LoaderCircleIcon className="animate-spin" />
            ) : (
              "Attach"
            )}
          </Button>
        </div>
      ) : null}
    </div>
  );
}
