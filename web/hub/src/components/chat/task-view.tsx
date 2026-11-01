"use client";

// A task, open on the Chat page the way a conversation is: its header, the
// conversation and the composer in the middle; the canvas beside it with the
// steps the lead planned, the brief with its history and facts, and - while
// a step runs - the file its agent edits, its terminal, the pages it reads
// and its plan, opened by themselves (chat/canvas.tsx). The conversation is
// the task's whole feed (tasks/task-activity.tsx): what you asked and said,
// the lead's words with their answers, and every run as it happens, so a
// task reads like a conversation that shows its work.

import Link from "next/link";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  ArrowUpIcon,
  BanIcon,
  CircleCheckIcon,
  CircleDashedIcon,
  ListChecksIcon,
  LoaderCircleIcon,
  MessageCircleQuestionIcon,
  PencilIcon,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Canvas, useCanvas } from "@/components/chat/canvas";
import { useSeen } from "@/components/chat/chat-pane";
import { BackButton, Split } from "@/components/chat/split";
import {
  TONE_TEXT,
  relShort,
  stepWords,
  taskMoodOf,
} from "@/components/chat/task-groups";
import { mergeSteps } from "@/components/harness/harness";
import { ConfirmDialog, showApiError } from "@/components/pulse/confirm";
import { Markdown } from "@/components/pulse/markdown";
import { PulseMascot } from "@/components/pulse/mascots";
import {
  Disclosure,
  ErrorNotice,
  Kv,
  Notice,
  SkeletonRows,
} from "@/components/pulse/panel";
import { Avatar, Mono, ProjectLink, When } from "@/components/pulse/who";
import { TaskActivityPane } from "@/components/tasks/task-activity";
import {
  CancelTaskDialog,
  TaskProgressBar,
} from "@/components/tasks/task-card";
import { AmendTaskDialog } from "@/components/tasks/task-dialogs";
import {
  Ago,
  TaskDeliveryLine,
  TaskWorkerInline,
  idleLine,
} from "@/components/tasks/task-work";
import {
  api,
  enc,
  query,
  taskProgress,
  writes,
  type HarnessSessionResponse,
  type HarnessStep,
  type MessageChoice,
  type Task,
  type TaskDetail,
  type TaskJob,
  type TaskRevision,
} from "@/lib/api";
import { makeTab } from "@/lib/canvas";
import { plural, sentence } from "@/lib/format";
import { hrefs } from "@/lib/nav";
import { isOwner, isReport } from "@/lib/task-message-status";
import { invalidate, useApi, useNow } from "@/lib/use-api";
import { cn } from "@/lib/utils";
import { priorityWord } from "@/lib/words";

const isClosed = (t: Task) => t.status === "done" || t.status === "cancelled";

// The hub answers a task that is not there with "No task …" (400) and an
// older one with "not found" (404); either way the page goes back to the
// list rather than retrying forever.
const GONE = /^No task\b|\bnot found\b|HTTP 404/i;

export function TaskView({
  id,
  list,
  onBack,
  onGone,
}: {
  id: string;
  list: ReactNode;
  onBack: () => void;
  onGone: () => void;
}) {
  // Live events already refresh `tasks/*`; the poll is the net under them,
  // closer while agents are at the steps.
  const detail = useApi<TaskDetail>("tasks/" + enc(id), { refresh: 10000 });
  const t = detail.data?.task;
  const working =
    !!t &&
    (taskProgress(t).working.length > 0 ||
      (detail.data?.jobs ?? []).some((j) => j.status === "leased") ||
      detail.data?.lead_work?.state === "running");
  const reload = detail.reload;
  useEffect(() => {
    if (!working) return;
    const tick = () => {
      if (typeof document === "undefined" || !document.hidden) void reload();
    };
    const timer = setInterval(tick, 3000);
    return () => clearInterval(timer);
  }, [working, reload]);
  const goneRef = useRef(onGone);
  useEffect(() => {
    goneRef.current = onGone;
  }, [onGone]);
  const gone = !!detail.error && !detail.data && GONE.test(detail.error);
  useEffect(() => {
    if (gone) goneRef.current();
  }, [gone]);

  if (detail.error && !detail.data) {
    return (
      <Split
        list={list}
        main={
          <div className="grid gap-3">
            <BackButton onBack={onBack} />
            <ErrorNotice
              error={detail.error}
              onRetry={() => void detail.reload()}
            >
              This task did not load.
            </ErrorNotice>
          </div>
        }
      />
    );
  }
  if (!detail.data) {
    return (
      <Split
        list={list}
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
    <TaskBody
      d={detail.data}
      list={list}
      onBack={onBack}
      error={detail.error}
      reload={detail.reload}
    />
  );
}

// Every write lands in the task's own rows and in the Chat list; the
// project's Work tab reads the same messages.
const refreshTask = () =>
  invalidate(
    (p) =>
      p.startsWith("tasks") ||
      p.startsWith("messages") ||
      p.startsWith("chat") ||
      p.startsWith("events"),
  );

function TaskBody({
  d,
  list,
  onBack,
  error,
  reload,
}: {
  d: TaskDetail;
  list: ReactNode;
  onBack: () => void;
  error: string;
  reload: () => Promise<void>;
}) {
  const t = d.task;
  const progress = taskProgress(t);
  const closed = isClosed(t);
  const review = t.status === "review";
  const working =
    progress.working.length > 0 || d.jobs.some((j) => j.status === "leased");
  const mood = taskMoodOf({
    status: t.status,
    lead: t.lead,
    needs_you: review,
    working,
  });
  // The run the canvas follows: the step being worked on now, else the
  // latest one that ran.
  const run =
    progress.working.find((w) => w.harness_session)?.harness_session ||
    latestRun(d.jobs);
  const runSteps = useRunSteps(run, working);
  const ctl = useCanvas({
    session: run,
    steps: runSteps,
    live: working,
    initial: [makeTab("steps"), makeTab("brief")],
  });
  const leadName = t.lead || "the lead agent";
  const [amend, setAmend] = useState(false);
  const [cancel, setCancel] = useState(false);
  const [finish, setFinish] = useState(false);
  const [busy, setBusy] = useState("");
  const messages = useMemo(
    () => [...d.messages].sort((a, b) => a.at.localeCompare(b.at)),
    [d.messages],
  );
  const texts = useMemo(
    () => [
      t.brief,
      ...messages.map((m) => m.body),
      ...runSteps.filter((st) => st.kind === "fetch").map((st) => st.content),
    ],
    [t.brief, messages, runSteps],
  );
  // Sending (a word or a pick) pulls the conversation back to its end.
  const [jump, setJump] = useState(0);

  // The same mark the board's chat keeps for this task, so a message read
  // there is not new here; the "New" line sits before the first one unseen.
  const latest = messages.length ? messages[messages.length - 1].id : undefined;
  const [seen, markSeen] = useSeen(t.project + ":" + t.id, latest);
  useEffect(() => {
    if (!latest || seen === latest) return;
    const id = setTimeout(markSeen, 1500);
    return () => clearTimeout(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [latest, seen]);
  const newBefore = useMemo(() => {
    if (!seen) return null;
    const i = messages.findIndex((m) => m.id === seen);
    return i >= 0 && i + 1 < messages.length ? messages[i + 1].id : null;
  }, [messages, seen]);

  // One tap answers: a reply goes to the lead as the owner's words (and by
  // itself ends a review), "done" closes the task with them, "amend" opens
  // the brief - the same three moves the board's chat offers.
  const pick = async (c: MessageChoice) => {
    if (busy) return;
    if (c.action === "amend") {
      setAmend(true);
      return;
    }
    setBusy("pick");
    try {
      if (c.action === "done")
        await writes.setTaskStatus(t.id, "done", c.label);
      else await writes.sendMessage(t.project, c.label, t.id);
      setJump((n) => n + 1);
      refreshTask();
      await reload();
    } catch (e) {
      showApiError(
        e,
        c.action === "done"
          ? "The task wasn't closed"
          : "Your answer wasn't sent",
      );
    } finally {
      setBusy("");
    }
  };

  const section = (
    <section className="flex min-h-0 min-w-0 flex-col gap-3 lg:flex-1">
      <BackButton onBack={onBack} />
      <header
        className={cn(
          "enter grid min-w-0 gap-2 rounded-2xl border bg-card px-4 py-3 shadow-card transition-colors",
          review && "surface-you",
          working && "border-primary/40",
        )}
      >
        <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2">
          <PulseMascot
            mood={mood.pip}
            size={44}
            className={cn("shrink-0", working && "float")}
            title={mood.word}
          />
          <div className="grid min-w-[14rem] flex-1 gap-0.5">
            {closed ? (
              <span
                className="truncate text-[17px] font-semibold tracking-tight text-foreground"
                title={t.title}
              >
                {t.title}
              </span>
            ) : (
              <button
                type="button"
                onClick={() => setAmend(true)}
                className="group flex min-w-0 items-center gap-1.5 text-left"
                title="Edit the task"
              >
                <span
                  className="truncate text-[17px] font-semibold tracking-tight text-foreground"
                  title={t.title}
                >
                  {t.title}
                </span>
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
                {working ? (
                  <LoaderCircleIcon className="size-3 animate-spin" />
                ) : (
                  <span
                    className={cn("live-dot size-1.5 rounded-full", mood.dot)}
                    aria-hidden
                  />
                )}
                {mood.word}
              </span>
              <span aria-hidden>·</span>
              <ProjectLink slug={t.project} />
              <span aria-hidden>·</span>
              {t.lead ? (
                <span
                  className="inline-flex min-w-0 items-center gap-1"
                  title={"Lead agent " + t.lead}
                >
                  <Avatar seed={t.lead} size={14} />{" "}
                  <span className="truncate">{t.lead}</span>
                </span>
              ) : (
                <span title="The runner keeps one lead agent per active project and wakes it for this task.">
                  {closed ? "no lead agent worked on it" : "no lead agent yet"}
                </span>
              )}
              {d.jobs.length ? (
                <>
                  <span aria-hidden>·</span>
                  <span>{plural(d.jobs.length, "step")}</span>
                </>
              ) : null}
              {t.priority >= 30 ? (
                <>
                  <span aria-hidden>·</span>
                  <span
                    className={cn(
                      "font-medium",
                      t.priority >= 50 && "text-bad",
                    )}
                  >
                    {priorityWord(t.priority).toLowerCase() + " priority"}
                  </span>
                </>
              ) : null}
              <span aria-hidden>·</span>
              <span
                className="font-mono text-[11px]"
                title={t.job_id ? "Planning job " + t.job_id : undefined}
              >
                {t.id}
              </span>
            </p>
          </div>
          {!closed ? (
            <>
              <Button
                type="button"
                size="sm"
                variant="ghost"
                className="h-7 shrink-0 rounded-full"
                onClick={() => setFinish(true)}
                disabled={!!busy}
                title="Mark this task done; the lead agent stops working on it"
              >
                <CircleCheckIcon />
                <span className="sr-only sm:not-sr-only">Finish</span>
              </Button>
              <Button
                type="button"
                size="sm"
                variant="ghost"
                className="h-7 shrink-0 rounded-full text-bad hover:bg-bad/10 hover:text-bad"
                onClick={() => setCancel(true)}
                disabled={!!busy}
                title="Cancel this task"
              >
                <BanIcon />
                <span className="sr-only sm:not-sr-only">Cancel</span>
              </Button>
            </>
          ) : null}
        </div>
      </header>
      {error ? (
        <Notice tone="warn">{"Live updates paused: " + error}</Notice>
      ) : null}
      {review ? (
        <div className="surface-you flex min-w-0 items-start gap-2 rounded-xl border px-3.5 py-3 text-sm leading-relaxed text-foreground">
          <MessageCircleQuestionIcon
            className="mt-0.5 size-4 shrink-0 text-you-ink"
            aria-hidden
          />
          <span className="min-w-0 anywhere">
            {sentence(leadName) +
              " stopped and waits for your decision. " +
              (messages.some(
                (m) => !isOwner(m.author) && !isReport(m) && m.choices?.length,
              )
                ? "Pick an answer under its message, or write one below."
                : "Write your answer below.")}
          </span>
        </div>
      ) : null}
      <TaskDeliveryLine
        id={t.id}
        delivery={d.delivery}
        onChange={() => void reload()}
        className="mt-0"
      />
      {closed ? (
        <Notice tone="info">
          {t.status === "done"
            ? "This task is done. The conversation stays here to read; give a new task to carry on."
            : "This task was cancelled. The conversation stays here to read; give a new task to carry on."}
        </Notice>
      ) : null}

      <div className="flex min-h-0 min-w-0 flex-col overflow-hidden rounded-2xl border bg-card shadow-card lg:flex-1">
        <TaskActivityPane
          task={t}
          chat={{
            messages,
            jobs: d.jobs,
            lead: d.lead_work,
            pick,
            busy: !!busy,
            newBefore,
          }}
          className="max-h-[calc(100dvh-24rem)] min-h-[20rem] lg:max-h-none lg:min-h-0"
          jumpOn={jump}
          footer={
            progress.working.length ? (
              <div
                className="mx-auto grid w-full max-w-3xl gap-1 text-[13px] leading-relaxed text-muted-foreground"
                role="status"
              >
                {progress.working.map((w, i) => (
                  <p key={w.job + i} className="flex min-w-0 items-start gap-2">
                    <LoaderCircleIcon
                      className="mt-1 size-3.5 shrink-0 animate-spin text-primary"
                      aria-hidden
                    />
                    <TaskWorkerInline t={t} w={w} />
                  </p>
                ))}
              </div>
            ) : null
          }
        />
      </div>

      <TaskComposer
        t={t}
        review={review}
        closed={closed}
        onSent={() => {
          setJump((n) => n + 1);
          refreshTask();
          void reload();
        }}
      />

      {amend ? (
        <AmendTaskDialog task={t} open={amend} onOpenChange={setAmend} />
      ) : null}
      <CancelTaskDialog t={t} open={cancel} onOpenChange={setCancel} />
      <ConfirmDialog
        open={finish}
        onOpenChange={setFinish}
        title="Finish this task?"
        description={
          <>
            <span className="block font-medium text-foreground anywhere">
              {t.title}
            </span>
            <span className="mt-2 block">
              {"It is marked done and " +
                leadName +
                " stops working on it. A step already running finishes first; nothing it made is thrown away."}
            </span>
          </>
        }
        action="Finish the task"
        dismiss="Keep it going"
        words={{
          label: "A word for the lead agent (optional)",
          placeholder:
            "For example: good enough, thanks - or what to leave for later",
        }}
        success="Task finished. The lead agent has been told."
        onConfirm={(words) =>
          writes.setTaskStatus(t.id, "done", words || undefined)
        }
        invalidates={(p) =>
          p.startsWith("tasks") ||
          p.startsWith("chat") ||
          p.startsWith("messages") ||
          p.startsWith("events")
        }
      />
    </section>
  );

  const canvas = (
    <Canvas
      ctl={ctl}
      label="Work of this task"
      session={run}
      steps={runSteps}
      live={working}
      texts={texts}
      menu={["terminal", "plan", "steps", "brief", "links"]}
      counts={{ steps: d.jobs.length }}
      panels={{
        steps: (
          <StepsPanel d={d} progress={progress} closed={closed} run={run} />
        ),
        brief: (
          <BriefPanel d={d} closed={closed} onEdit={() => setAmend(true)} />
        ),
      }}
    />
  );

  return (
    <Split list={list} main={section} fill wide={ctl.wide} panel={canvas} />
  );
}

// The job that ran last among those with a run: by when it finished or
// was taken, newest first.
function latestRun(jobs: TaskJob[]): string {
  let best = "";
  let at = "";
  for (const j of jobs) {
    if (!j.harness_session) continue;
    const t =
      j.finished_at || j.leased_at || j.updated_at || j.created_at || "";
    if (!best || t > at) {
      best = j.harness_session;
      at = t;
    }
  }
  return best;
}

// The steps of the run the canvas follows, fetched once and then only past
// the last one held while the step is being worked on. A new run starts
// the list over.
function useRunSteps(sid: string, live: boolean): HarnessStep[] {
  const [held, setHeld] = useState<{ sid: string; steps: HarnessStep[] }>({
    sid: "",
    steps: [],
  });
  const last = useRef({ sid: "", n: -1 });
  useEffect(() => {
    if (!sid) return;
    let stopped = false;
    const tick = async () => {
      const after = last.current.sid === sid ? last.current.n : -1;
      try {
        const r = await api<HarnessSessionResponse>(
          "harness/sessions/" + enc(sid) + (after >= 0 ? query({ after }) : ""),
        );
        if (stopped) return;
        const n = r.steps.reduce((m, s) => Math.max(m, s.n), after);
        last.current = { sid, n };
        setHeld((p) => ({
          sid,
          steps:
            p.sid === sid && after >= 0
              ? mergeSteps(p.steps, r.steps)
              : r.steps,
        }));
      } catch {
        // The canvas keeps what it has; the next tick tries again.
      }
    };
    void tick();
    if (!live)
      return () => {
        stopped = true;
      };
    const timer = setInterval(() => {
      if (typeof document === "undefined" || !document.hidden) void tick();
    }, 3000);
    return () => {
      stopped = true;
      clearInterval(timer);
    };
  }, [sid, live]);
  return held.sid === sid ? held.steps : EMPTY_STEPS;
}

const EMPTY_STEPS: HarnessStep[] = [];

// ----- composer -------------------------------------------------------------

// Your word to the lead agent: it is pinned to this task and wakes the lead.
// A finished task keeps the conversation to read; the composer rests.
function TaskComposer({
  t,
  review,
  closed,
  onSent,
}: {
  t: Task;
  review: boolean;
  closed: boolean;
  onSent: () => void;
}) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const leadName = t.lead || "the lead agent";
  const canSend = !!text.trim() && !busy && !closed;
  const send = async () => {
    if (!canSend) return;
    setBusy(true);
    try {
      await writes.sendMessage(t.project, text.trim(), t.id);
      setText("");
      onSent();
    } catch (e) {
      showApiError(e, "The message did not send");
    } finally {
      setBusy(false);
    }
  };
  return (
    <form
      className="grid min-w-0 shrink-0 gap-1.5"
      onSubmit={(e) => {
        e.preventDefault();
        void send();
      }}
    >
      <div
        className={cn(
          "grid min-w-0 gap-2 rounded-2xl border bg-card p-2 shadow-card transition-colors focus-within:border-ring focus-within:ring-3 focus-within:ring-ring/25",
          closed && "opacity-70",
        )}
      >
        <Textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (
              e.key === "Enter" &&
              !e.shiftKey &&
              !e.nativeEvent.isComposing
            ) {
              e.preventDefault();
              void send();
            }
          }}
          placeholder={
            closed
              ? "This task is finished. Give a new task to carry on."
              : review
                ? "Answer " + leadName + "…"
                : "Ask how it is going, or steer the work…"
          }
          disabled={closed}
          rows={1}
          className="min-h-10 max-h-48 resize-none border-0 bg-transparent px-2.5 py-2 text-[15px] shadow-none focus-visible:ring-0 dark:bg-transparent"
          aria-label={"Message " + leadName + " about " + t.id}
        />
        <div className="flex min-w-0 items-center gap-1.5">
          <span className="min-w-0 flex-1 truncate px-2 text-xs text-muted-foreground">
            {closed
              ? ""
              : t.lead
                ? "To " + t.lead + ", the lead agent of this task"
                : "To the lead agent, once one is woken for this task"}
          </span>
          <Button
            type="submit"
            size="icon"
            disabled={!canSend}
            aria-label="Send"
            className="shrink-0 rounded-xl"
          >
            {busy ? (
              <LoaderCircleIcon className="animate-spin" />
            ) : (
              <ArrowUpIcon />
            )}
          </Button>
        </div>
      </div>
      <p className="px-2 text-[11px] text-muted-foreground">
        {closed
          ? "A finished task takes no more messages."
          : "Enter sends, Shift+Enter for a new line. Your message is about this task; the lead agent reads it on its next turn."}
      </p>
    </form>
  );
}

// ----- canvas panels: the steps; the brief with the facts -------------------

function StepsPanel({
  d,
  progress,
  closed,
  run,
}: {
  d: TaskDetail;
  progress: ReturnType<typeof taskProgress>;
  closed: boolean;
  run: string;
}) {
  const t = d.task;
  const idle = idleLine(t);
  return (
    <>
      <StepList
        jobs={d.jobs}
        t={t}
        progress={progress}
        closed={closed}
        run={run}
      />
      {progress.total ? <TaskProgressBar t={t} className="px-1" /> : null}
      {idle ? (
        <p className="px-1 text-[12px] leading-relaxed text-muted-foreground">
          {idle}
        </p>
      ) : null}
    </>
  );
}

function BriefPanel({
  d,
  closed,
  onEdit,
}: {
  d: TaskDetail;
  closed: boolean;
  onEdit: () => void;
}) {
  const t = d.task;
  const by = (a: string) => (a === "human" || a === "owner" ? "you" : a);
  return (
    <>
      <section className="grid min-w-0 gap-1.5" aria-label="What you asked">
        <h3 className="flex items-center gap-1.5 px-1 text-[11px] font-semibold tracking-wide text-muted-foreground uppercase">
          What you asked
          <span className="tnum font-normal">
            {d.revisions.length > 1 ? "v" + t.current_rev : null}
          </span>
          {!closed ? (
            <button
              type="button"
              className="ml-auto font-normal normal-case tracking-normal underline-offset-4 hover:text-foreground hover:underline"
              onClick={onEdit}
            >
              Edit
            </button>
          ) : null}
        </h3>
        <div className="scroll-thin max-h-72 min-w-0 overflow-y-auto rounded-lg border bg-muted/20 px-3 py-2 text-[13px] leading-relaxed">
          <Markdown source={t.brief} className="[&>:last-child]:mb-0" />
        </div>
      </section>

      <section className="grid min-w-0 gap-1.5" aria-label="Details">
        <h3 className="px-1 text-[11px] font-semibold tracking-wide text-muted-foreground uppercase">
          Details
        </h3>
        <Kv
          className="px-1 text-[13px]"
          rows={[
            ["Task", <Mono key="id">{t.id}</Mono>],
            ["Priority", priorityWord(t.priority)],
            [
              "Lead agent",
              t.lead ? (
                <span key="lead" className="inline-flex items-center gap-1.5">
                  <Avatar seed={t.lead} size={16} />
                  {t.lead}
                </span>
              ) : (
                <span key="lead" className="text-muted-foreground">
                  none yet
                </span>
              ),
            ],
            [
              "Given",
              <span key="given">
                {"by " + by(t.created_by) + ", "}
                <Ago d={t.created_at} />
              </span>,
            ],
            ["Updated", <Ago key="updated" d={t.updated_at} />],
            [
              "Versions",
              d.revisions.length > 1
                ? plural(d.revisions.length, "version") +
                  ", latest " +
                  t.current_rev
                : "not changed since you gave it",
            ],
          ]}
        />
      </section>

      {d.revisions.length > 1 ? (
        <History revisions={d.revisions} current={t.current_rev} />
      ) : null}
    </>
  );
}

// Every version of the brief, newest first: who changed it, the note they
// left, and the words it replaced.
function History({
  revisions,
  current,
}: {
  revisions: TaskRevision[];
  current: number;
}) {
  const by = (a: string) => (a === "human" || a === "owner" ? "you" : a);
  return (
    <section className="grid min-w-0 gap-1.5" aria-label="History">
      <h3 className="px-1 text-[11px] font-semibold tracking-wide text-muted-foreground uppercase">
        History
      </h3>
      <ol
        className="grid min-w-0 gap-px rounded-lg border bg-muted/20 py-1 text-[13px]"
        role="list"
      >
        {[...revisions].reverse().map((r, i, all) => {
          const prev = all[i + 1];
          return (
            <li key={r.rev} className="grid min-w-0 gap-1 px-2.5 py-1.5">
              <p className="flex min-w-0 flex-wrap items-baseline gap-x-2 text-xs text-muted-foreground">
                <span className="font-medium text-foreground">
                  {"v" + r.rev}
                </span>
                {r.rev === current ? (
                  <span className="text-primary">current</span>
                ) : null}
                <span>{"by " + by(r.author)}</span>
                <When d={r.at} />
              </p>
              {prev && prev.title !== r.title ? (
                <p className="min-w-0 text-foreground anywhere">
                  <span className="text-muted-foreground">Titled: </span>
                  {r.title}
                </p>
              ) : null}
              {r.note ? (
                <p className="min-w-0 leading-relaxed text-foreground anywhere">
                  {r.note}
                </p>
              ) : null}
              {prev && prev.brief !== r.brief ? (
                <Disclosure
                  summary={
                    <span className="text-xs text-muted-foreground">
                      What it replaced
                    </span>
                  }
                  contentClassName="mt-1"
                >
                  <div className="scroll-thin max-h-48 overflow-y-auto rounded-md bg-muted/40 px-2.5 py-1.5 text-xs leading-relaxed">
                    <Markdown
                      source={prev.brief}
                      className="[&>:last-child]:mb-0"
                    />
                  </div>
                </Disclosure>
              ) : null}
            </li>
          );
        })}
      </ol>
    </section>
  );
}

// The steps the lead planned, each one a row like a sub-conversation's: who
// is on it, its word, when it moved; a step with a run opens it.
function StepList({
  jobs,
  t,
  progress,
  closed,
  run: shown,
}: {
  jobs: TaskJob[];
  t: Task;
  progress: ReturnType<typeof taskProgress>;
  closed: boolean;
  run: string;
}) {
  const now = useNow(15000);
  return (
    <section className="grid min-w-0 gap-1.5" aria-label="Steps">
      <h3 className="flex items-center gap-1.5 px-1 text-[11px] font-semibold tracking-wide text-muted-foreground uppercase">
        <ListChecksIcon className="size-3" aria-hidden />
        Steps
        {jobs.length ? (
          <span className="tnum font-normal">{jobs.length}</span>
        ) : null}
      </h3>
      {jobs.length ? (
        <ul
          className="grid min-w-0 gap-px rounded-lg border bg-muted/20 py-1"
          role="list"
        >
          {jobs.map((j) => {
            const w = progress.working.find((x) => x.job === j.id);
            const s = stepWords(j.status, !!w);
            const at =
              j.finished_at ||
              j.leased_at ||
              j.updated_at ||
              j.created_at ||
              "";
            const needs = (j.needs ?? "").split(",").filter(Boolean).join(", ");
            const tip = [
              "Job " + j.id,
              j.kind ? "kind: " + j.kind : "",
              j.branch ? "branch " + j.branch : "",
              j.executor ? "machine " + j.executor : "",
              needs ? "needs " + needs : "",
            ]
              .filter(Boolean)
              .join(", ");
            const run = w?.harness_session || j.harness_session || "";
            const row = (
              <>
                <span
                  className={cn(
                    "live-dot size-1.5 shrink-0 rounded-full",
                    s.tone === "run"
                      ? "bg-primary"
                      : s.tone === "ok"
                        ? "bg-ok"
                        : s.tone === "bad"
                          ? "bg-bad"
                          : s.tone === "wait"
                            ? "bg-warn"
                            : "bg-muted-foreground/40",
                  )}
                  data-live={!!w}
                  aria-hidden
                />
                {j.agent || w?.agent ? (
                  <Avatar
                    seed={w?.agent || j.agent}
                    size={14}
                    className="shrink-0"
                  />
                ) : (
                  <CircleDashedIcon
                    className="size-3.5 shrink-0 text-muted-foreground"
                    aria-hidden
                  />
                )}
                <span className="min-w-0 flex-1 truncate text-foreground">
                  {sentence(j.title) || "A step"}
                </span>
                <span className={cn("shrink-0 font-medium", TONE_TEXT[s.tone])}>
                  {s.word}
                </span>
                {at ? (
                  <span className="shrink-0 tnum text-muted-foreground">
                    {relShort(at, now)}
                  </span>
                ) : null}
              </>
            );
            const cls =
              "flex w-full min-w-0 items-center gap-2 px-2.5 py-1.5 text-left text-xs";
            return (
              <li key={j.id} className="min-w-0">
                {run ? (
                  <Link
                    href={hrefs.harness({ id: run })}
                    className={cn(
                      cls,
                      "transition-colors hover:bg-muted/50",
                      run === shown && "bg-primary/[0.06]",
                    )}
                    title={
                      tip +
                      (run === shown
                        ? ". Its work is on the canvas; open the whole run"
                        : ". Open the run")
                    }
                  >
                    {row}
                  </Link>
                ) : (
                  <div className={cls} title={tip}>
                    {row}
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      ) : (
        <p className="px-1 text-[12px] leading-relaxed text-muted-foreground">
          {t.status === "cancelled"
            ? "Cancelled before the lead agent split it into steps."
            : closed
              ? "Finished without separate steps."
              : t.lead
                ? "None yet. " +
                  t.lead +
                  " splits the task into steps; each one shows here with who is on it."
                : "None yet. When the lead agent splits the task into steps, each one shows here with who is on it."}
        </p>
      )}
    </section>
  );
}
