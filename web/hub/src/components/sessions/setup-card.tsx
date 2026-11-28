"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState } from "react";
import { BookOpenIcon, CircleCheckIcon, RocketIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { AddProjectDialog } from "@/components/project/project-dialogs";
import { ProviderWizard } from "@/components/settings/provider-wizard";
import type { HarnessStatus, ServicePayload } from "@/lib/api";
import { hrefs } from "@/lib/nav";
import { cn } from "@/lib/utils";
import { FixButton } from "./attention-strip";
import { runningFix, setupSteps, type AttentionDaemon, type SetupSnapshot } from "./sessions-state";

// Sessions before Hive can work (first run: no model provider or no
// project): a three-step setup card above the new-session box (it sat on
// Home until TASK-129).
// Each step is a row with a tick once it is done and, until then, the button
// that opens its wizard (ink on the next step, quiet on the later ones) - the same Connect a provider wizard Settings uses,
// the Add project dialog, and the runner's service dialog. Done steps stay
// listed so the owner sees how far along the machine is.
export function SetupCard({ snapshot, harness, service, daemon }: { snapshot: SetupSnapshot; harness: HarnessStatus | undefined; service: ServicePayload | undefined; daemon: AttentionDaemon }) {
  const router = useRouter();
  const [wizard, setWizard] = useState(false);
  const [addProject, setAddProject] = useState(false);
  const steps = setupSteps(snapshot);
  const done = steps.filter((s) => s.done).length;
  // One step at a time: the first one not done is "next" and gets the ink
  // button and the teal number; the later ones wait with quiet buttons.
  const next = steps.findIndex((s) => !s.done);
  return (
    <section aria-labelledby="sessions-setup" className="min-w-0 rounded-2xl border bg-card p-4 shadow-card sm:p-5">
      <div className="flex min-w-0 items-start justify-between gap-3">
        <div className="min-w-0">
          <h2 id="sessions-setup" className="flex items-center gap-2 text-[17px] font-semibold tracking-tight text-foreground">
            <RocketIcon aria-hidden className="size-[18px] text-link" strokeWidth={1.75} />
            Set up Hive
          </h2>
          <p className="mt-0.5 text-[13px] text-muted-foreground">
            {steps.length} quick steps, {done} done. Each takes about a minute.
          </p>
        </div>
        <Link href={hrefs.record("GUIDE-new-repo")} className="inline-flex shrink-0 items-center gap-1 text-[13px] font-medium text-link hover:underline">
          <BookOpenIcon className="size-3.5" aria-hidden /> Guide
        </Link>
      </div>
      <div className="mt-3 h-1.5 overflow-hidden rounded-full bg-muted" role="progressbar" aria-label="Setup progress" aria-valuemin={0} aria-valuemax={steps.length} aria-valuenow={done}>
        <div className="h-full rounded-full bg-ok transition-[width] duration-300 ease-out-soft motion-reduce:transition-none" style={{ width: (done / steps.length) * 100 + "%" }} />
      </div>
      <ol className="mt-4 grid gap-1" aria-label="Setup steps">
        {steps.map((step, i) => {
          const isNext = i === next;
          const variant = isNext ? "default" : "outline";
          let action: React.ReactNode = null;
          if (!step.done) {
            if (step.id === "provider") {
              action = harness ? (
                <Button size="sm" variant={variant} onClick={() => setWizard(true)}>
                  {step.action}
                </Button>
              ) : (
                <Button size="sm" variant={variant} asChild>
                  <Link href={"/settings/?connect=1#harness"}>{step.action}</Link>
                </Button>
              );
            } else if (step.id === "project") {
              action = (
                <Button size="sm" variant={variant} onClick={() => setAddProject(true)}>
                  {step.action}
                </Button>
              );
            } else {
              action = <FixButton fix={runningFix(daemon, hrefs.settings)} label={step.action} service={service} variant={variant} />;
            }
          }
          return (
            <li
              key={step.id}
              aria-label={step.title + (step.done ? ": done" : ": " + step.action)}
              className={cn("flex min-w-0 items-center gap-3 rounded-xl px-2 py-2.5", isNext && "bg-accent/60")}
            >
              {step.done ? (
                <CircleCheckIcon className="size-7 shrink-0 text-ok" strokeWidth={1.75} aria-hidden />
              ) : (
                <span
                  aria-hidden
                  className={cn(
                    "flex size-7 shrink-0 items-center justify-center rounded-full text-[13px] font-semibold tabular-nums",
                    isNext ? "bg-link text-background" : "border border-border text-muted-foreground",
                  )}
                >
                  {i + 1}
                </span>
              )}
              <div className="min-w-0 flex-1">
                <p className={cn("text-sm font-medium text-foreground", step.done && "text-muted-foreground line-through decoration-muted-foreground/50")}>{step.title}</p>
                <p className="text-[13px] leading-snug text-muted-foreground">{step.note}</p>
              </div>
              {action ? <div className="shrink-0">{action}</div> : null}
            </li>
          );
        })}
      </ol>
      {harness ? <ProviderWizard open={wizard} onOpenChange={setWizard} h={harness} /> : null}
      <AddProjectDialog open={addProject} onOpenChange={setAddProject} onRegistered={(r) => router.push(hrefs.project(r.project.slug))} />
    </section>
  );
}
