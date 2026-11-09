"use client";

import Link from "next/link";
import { useMemo, useState } from "react";
import { FileTextIcon, FlaskConicalIcon, GitMergeIcon, ImageIcon, ListTodoIcon, RefreshCwIcon, SearchIcon, TagIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { CardGrid, ItemCard, ItemCardHeader, ItemCardKind, ItemCardMeta, ItemCardTitle, SectionHeading } from "@/components/pulse/item-card";
import { EmptyState, ErrorNotice, Kpi, KpiGrid, Kv, Panel, SkeletonRows, Stack } from "@/components/pulse/panel";
import { Mono, ProjectLink, StatusBadge, When } from "@/components/pulse/who";
import { jobArtifactHref, workReportsPath, type ProjectsPayload, type WorkReport, type WorkReportJob, type WorkReportsPayload } from "@/lib/api";
import { hrefs } from "@/lib/nav";
import { plural, sentence, sizeText, truncate } from "@/lib/format";
import { cleanSummary, commitURL, evidenceKind, landingWords, matchesReport, orderEvidence, reportsEmpty, reportsSummary, taskBadge, taskLine, taskReleases, taskTone, verifyState, type ReportTone } from "@/lib/reports";
import { useApi } from "@/lib/use-api";

const LIMIT = 100;

// A tone as the status badge's kind: an icon plus a word, never colour alone.
const KIND: Record<ReportTone, string> = { ok: "done", warn: "queued", bad: "failed", neutral: "neutral" };

const LINK = "inline-flex items-center gap-1 rounded-sm text-[13px] font-medium text-primary underline-offset-4 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

export function WorkReports({ project }: { project?: string }) {
  const q = useApi<WorkReportsPayload>(workReportsPath({ project: project || undefined, limit: LIMIT }), { refresh: 30000 });
  const registry = useApi<ProjectsPayload>("projects", { refresh: 60000 });
  const remotes = new Map((registry.error ? [] : registry.data?.projects ?? []).map((p) => [p.slug, p.remote]));
  const [words, setWords] = useState("");
  const [open, setOpen] = useState<WorkReport | null>(null);
  const d = q.data;
  const shown = useMemo(() => (d ? d.reports.filter((t) => matchesReport(t, words)) : []), [d, words]);
  const sum = reportsSummary(d);
  const empty = d ? reportsEmpty(d, shown.length) : null;

  return (
    <section id="work-reports" aria-label="Work reports" className="scroll-mt-32">
      {q.error && !d ? <ErrorNotice error={q.error} onRetry={() => void q.reload()} /> : null}
      {!d ? (
        q.error ? null : <SkeletonRows n={4} />
      ) : (
        <Stack gap={4}>
          {q.error ? <ErrorNotice error={q.error} onRetry={() => void q.reload()}>{"Couldn't refresh, so these are the earlier reports (" + q.error + ")."}</ErrorNotice> : null}
          <KpiGrid>
            <Kpi icon={ListTodoIcon} n={sum.tasks} label="Tasks done" sub={d.scope.has_more ? "The latest " + d.reports.length + " are listed" : "Every done task is listed"} />
            <Kpi icon={GitMergeIcon} n={sum.landed} label="Landed" sub="Merged into the default branch" />
            <Kpi icon={TagIcon} n={sum.released} label="Released" sub="In a tagged version" />
            <Kpi icon={FlaskConicalIcon} n={sum.evidence} label="With evidence" sub="Reports, screenshots or files" />
          </KpiGrid>
          <div>
            <SectionHeading
              count={shown.length}
              note="Newest first. Open a task to see each job's landing, its checks and the files it left."
              action={
                <Button size="sm" variant="outline" onClick={() => void q.reload()} disabled={q.loading}>
                  <RefreshCwIcon className={q.loading ? "animate-spin motion-reduce:animate-none" : ""} /> Refresh
                </Button>
              }
            >
              Finished work
            </SectionHeading>
            {d.reports.length ? (
              <label className="relative mb-3 block max-w-sm">
                <span className="sr-only">Search the reports</span>
                <SearchIcon className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden />
                <Input value={words} onChange={(e) => setWords(e.target.value)} placeholder="Search tasks, jobs, agents, releases" className="pl-8" />
              </label>
            ) : null}
            {shown.length ? (
              <CardGrid cols={2}>
                {shown.map((t) => (
                  <ReportCard key={t.id} task={t} onOpen={() => setOpen(t)} />
                ))}
              </CardGrid>
            ) : empty ? (
              <EmptyState title={empty.title} icon={<FileTextIcon className="size-[18px]" />}>
                {empty.body}
              </EmptyState>
            ) : null}
          </div>
        </Stack>
      )}
      <ReportSheet task={open} remote={open ? remotes.get(open.project) : undefined} onClose={() => setOpen(null)} />
    </section>
  );
}

function ReportCard({ task, onOpen }: { task: WorkReport; onOpen: () => void }) {
  const summary = task.jobs.map((j) => cleanSummary(j.summary)).find(Boolean) || "";
  return (
    <ItemCard onOpen={onOpen} ariaLabel={"Report for " + task.id + ": " + task.title} className="min-w-0">
      <ItemCardHeader right={<When d={task.done_at} />}>
        <ItemCardKind icon={ListTodoIcon}>{task.id}</ItemCardKind>
        <ProjectLink slug={task.project} className="-ml-0.5" />
      </ItemCardHeader>
      <ItemCardTitle chevron>{task.title || task.id}</ItemCardTitle>
      {summary ? <p className="line-clamp-2 text-sm text-muted-foreground anywhere">{sentence(summary)}</p> : null}
      <ItemCardMeta>
        <StatusBadge status={KIND[taskTone(task)]} label={taskBadge(task)} size="sm" />
        <span>{taskLine(task)}</span>
      </ItemCardMeta>
    </ItemCard>
  );
}

function ReportSheet({ task, remote, onClose }: { task: WorkReport | null; remote?: string; onClose: () => void }) {
  return (
    <Sheet open={!!task} onOpenChange={(o) => !o && onClose()}>
      <SheetContent side="right" className="flex flex-col gap-0 overflow-hidden rounded-l-xl p-0 data-[side=right]:w-full data-[side=right]:sm:max-w-2xl">
        {task ? <ReportBody task={task} remote={remote} /> : null}
      </SheetContent>
    </Sheet>
  );
}

function ReportBody({ task, remote }: { task: WorkReport; remote?: string }) {
  const rel = taskReleases(task);
  return (
    <>
      <SheetHeader className="gap-2 border-b bg-card px-5 pb-4 pt-5">
        <SheetTitle className="pr-8 text-xl font-semibold leading-snug tracking-tight anywhere">{task.title || task.id}</SheetTitle>
        <SheetDescription asChild>
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 text-[13px] text-muted-foreground">
            <StatusBadge status={KIND[taskTone(task)]} label={taskBadge(task)} />
            <ProjectLink slug={task.project} className="font-normal" />
            <span>
              {"Done "}
              <When d={task.done_at} />
            </span>
            <Link href={hrefs.task(task.id)} className={LINK}>
              {"Open " + task.id}
            </Link>
          </div>
        </SheetDescription>
      </SheetHeader>
      <div className="scroll-thin grid min-h-0 flex-1 content-start gap-4 overflow-y-auto p-5">
        <Panel title="What was asked" icon={ListTodoIcon}>
          <Stack>
            {task.brief ? <p className="whitespace-pre-line text-sm leading-relaxed anywhere">{truncate(task.brief, 600)}</p> : <p className="text-sm text-muted-foreground">No brief was written for this task.</p>}
            <Kv
              rows={[
                ["Filed", <When key="c" d={task.created_at} exact />],
                ["By", task.created_by || "—"],
                ["Release", rel.length ? rel.join(", ") : "Not in a tagged release yet"],
              ]}
            />
          </Stack>
        </Panel>
        {task.jobs.length ? task.jobs.map((j) => <JobReport key={j.id} job={j} remote={remote} />) : <p className="text-sm text-muted-foreground">No job was recorded for this task, so there is nothing landed or evidenced to show.</p>}
      </div>
    </>
  );
}

function JobReport({ job, remote }: { job: WorkReportJob; remote?: string }) {
  const land = landingWords(job.landing);
  const check = verifyState(job.landing?.verify || job.verify);
  const summary = cleanSummary(job.summary);
  const files = orderEvidence(job.artifacts);
  const page = files.find((f) => f.name === "report.html");
  const sha = job.landing?.sha || "";
  const commit = sha ? commitURL(remote, sha) : null;
  return (
    <Panel title={sentence(job.report?.title || job.title || job.id)} icon={GitMergeIcon}>
      <Stack>
        {summary ? <p className="text-sm leading-relaxed anywhere">{sentence(summary)}</p> : null}
        <Kv
          rows={[
            ["Landing", <StatusBadge key="l" status={KIND[land.tone]} label={land.label} size="sm" />],
            ["Commit", sha ? commit ? <a key="c" href={commit} target="_blank" rel="noopener noreferrer" className={LINK} title={"View commit " + sha + " (opens in a new tab)"}><Mono>{sha.slice(0, 10)}</Mono></a> : <Mono key="c">{sha.slice(0, 10)}</Mono> : plural(job.commits, "commit") + ", none landed"],
            ...(job.landing?.url ? ([["Pull request", <a key="p" href={job.landing.url} target="_blank" rel="noopener noreferrer" className={LINK}>{job.landing.number > 0 ? "#" + job.landing.number : "Open"}</a>]] as [string, React.ReactNode][]) : []),
            ["Release", job.release || "Not released yet"],
            ["Checks", check ? <StatusBadge key="v" status={KIND[check.tone]} label={check.text} size="sm" title={job.landing?.verify || job.verify} /> : "No check result recorded"],
            ["Agent", job.agent || "—"],
            ["Finished", <When key="f" d={job.finished_at} exact />],
            ["Job", <Link key="j" href={hrefs.harness({ id: job.id })} className={LINK}><Mono>{job.id}</Mono></Link>],
          ]}
        />
        {job.report?.checks?.length ? (
          <div className="text-sm">
            <p className="mb-1 font-medium">Checks the agent ran</p>
            <ul className="grid list-disc gap-0.5 pl-5 text-muted-foreground">
              {job.report.checks.map((c) => (
                <li key={c} className="anywhere">{c}</li>
              ))}
            </ul>
          </div>
        ) : null}
        <div className="text-sm">
          <p className="mb-1 font-medium">Evidence</p>
          {files.length ? (
            <>
              {page ? (
                <Button asChild size="sm" className="mb-2">
                  <a href={jobArtifactHref(job.id, page.name)} target="_blank" rel="noopener noreferrer">
                    <FileTextIcon /> Open the report
                  </a>
                </Button>
              ) : null}
              <ul className="grid gap-1">
                {files.map((f) => (
                  <li key={f.name} className="flex min-w-0 items-center justify-between gap-3">
                    <a href={jobArtifactHref(job.id, f.name)} target="_blank" rel="noopener noreferrer" className={LINK + " min-w-0"} title={f.name}>
                      {evidenceKind(f.name) === "picture" ? <ImageIcon className="size-3.5 shrink-0" aria-hidden /> : <FileTextIcon className="size-3.5 shrink-0" aria-hidden />}
                      <span className="truncate">{f.name}</span>
                    </a>
                    <span className="shrink-0 text-xs text-muted-foreground">{sizeText(f.size)}</span>
                  </li>
                ))}
              </ul>
              {job.artifacts_total > files.length ? <p className="mt-1 text-xs text-muted-foreground">{"And " + plural(job.artifacts_total - files.length, "more file") + " in the job's folder."}</p> : null}
            </>
          ) : (
            <p className="text-muted-foreground">This job left no evidence files.</p>
          )}
        </div>
      </Stack>
    </Panel>
  );
}
