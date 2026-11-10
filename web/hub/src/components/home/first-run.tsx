"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState } from "react";
import { BookOpenIcon, CircleCheckIcon, CircleIcon, RocketIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { AddProjectDialog } from "@/components/project/project-dialogs";
import { ListRow, ListRows } from "@/components/hive/list-row";
import { SectionHeading } from "@/components/hive/section-heading";
import { ProviderWizard } from "@/components/settings/provider-wizard";
import type { HarnessStatus, ServicePayload } from "@/lib/api";
import { hrefs } from "@/lib/nav";
import { cn } from "@/lib/utils";
import { FixButton } from "./attention-strip";
import { runningFix, setupSteps, type AttentionDaemon, type SetupSnapshot } from "./home-state";

// Home before Hive can work (spec "Home" - Other states, first run: no model
// provider or no project): a three-step setup card in place of "Needs you".
// Each step is a row with a tick once it is done and, until then, the button
// that opens its wizard - the same Connect a provider wizard Settings uses,
// the Add project dialog, and the runner's service dialog. Done steps stay
// listed so the owner sees how far along the machine is.
export function SetupCard({ snapshot, harness, service, daemon }: { snapshot: SetupSnapshot; harness: HarnessStatus | undefined; service: ServicePayload | undefined; daemon: AttentionDaemon }) {
  const router = useRouter();
  const [wizard, setWizard] = useState(false);
  const [addProject, setAddProject] = useState(false);
  const steps = setupSteps(snapshot);
  const done = steps.filter((s) => s.done).length;
  return (
    <section aria-labelledby="home-setup" className="min-w-0">
      <SectionHeading
        id="home-setup"
        icon={RocketIcon}
        note={done + " of " + steps.length + " done"}
        action={
          <Link href={hrefs.record("GUIDE-new-repo")} className="inline-flex items-center gap-1">
            <BookOpenIcon className="size-3.5" aria-hidden /> Read the guide
          </Link>
        }
      >
        Set up Hive
      </SectionHeading>
      <ListRows ariaLabel="Setup steps">
        {steps.map((step, i) => {
          let action: React.ReactNode = null;
          if (!step.done) {
            if (step.id === "provider") {
              action = harness ? (
                <Button size="sm" onClick={() => setWizard(true)}>
                  {step.action}
                </Button>
              ) : (
                <Button size="sm" asChild>
                  <Link href={"/settings/?connect=1#harness"}>{step.action}</Link>
                </Button>
              );
            } else if (step.id === "project") {
              action = (
                <Button size="sm" onClick={() => setAddProject(true)}>
                  {step.action}
                </Button>
              );
            } else {
              action = <FixButton fix={runningFix(daemon, hrefs.settings)} label={step.action} service={service} />;
            }
          }
          return (
            <ListRow
              key={step.id}
              lead={
                step.done ? (
                  <CircleCheckIcon className="size-5 text-ok" aria-hidden />
                ) : (
                  <span className="relative flex size-5 items-center justify-center" aria-hidden>
                    <CircleIcon className="size-5 text-muted-foreground/60" />
                    <span className="absolute text-xs font-semibold tabular-nums text-muted-foreground">{i + 1}</span>
                  </span>
                )
              }
              title={<span className={cn(step.done && "text-muted-foreground line-through decoration-muted-foreground/50")}>{step.title}</span>}
              status={action ?? undefined}
              ariaLabel={step.title + (step.done ? ": done" : ": " + step.action)}
              className="py-2.5"
            >
              {step.note}
            </ListRow>
          );
        })}
      </ListRows>
      {harness ? <ProviderWizard open={wizard} onOpenChange={setWizard} h={harness} /> : null}
      <AddProjectDialog open={addProject} onOpenChange={setAddProject} onRegistered={(r) => router.push(hrefs.project(r.project.slug))} />
    </section>
  );
}
