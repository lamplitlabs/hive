"use client";

import Link from "next/link";
import { CheckCircle2Icon, ServerIcon } from "lucide-react";
import { AddMachineButton, PairingRow } from "@/components/compute/add-machine";
import { ExecutorRow, MachinePanel } from "@/components/compute/machine";
import { HygienePanel } from "@/components/pulse/hygiene";
import { RunnersAskedElsewhere, RunnersQuestion, runnersAsking } from "@/components/pulse/runners";
import { useState } from "react";
import { CapsPanel, ComputeInboxCard, ComputeRequestCard, ComputeRequestRow } from "@/components/compute/compute";
import { Button } from "@/components/ui/button";
import { CardGrid, SectionHeading } from "@/components/pulse/item-card";
import { Disclosure, EmptyState, ErrorNotice, PageTitle, Panel, SkeletonPage } from "@/components/pulse/panel";
import { RemoveMenuButton } from "@/components/project/project-dialogs";
import { hrefs } from "@/lib/nav";
import { plural } from "@/lib/format";
import type { Compute, ComputeRequest, Executor, Hygiene, Runner } from "@/lib/api";
import { useApi } from "@/lib/use-api";

const SUB = "Where your agents run. Agents can ask for an extra machine for a few hours; nothing starts until you approve it.";

const newestFirst = (a: ComputeRequest, b: ComputeRequest) => (b.at || "").localeCompare(a.at || "");

const LIVE = new Set(["approved", "provisioning", "ready"]);

// Past requests show the latest few; the rest wait behind "Show all".
const PAST_SHOWN = 5;

const quietLink = "text-[13px] font-medium text-muted-foreground underline-offset-2 hover:text-foreground hover:underline";

export default function ComputePage() {
  const q = useApi<Compute>("compute", { refresh: 10000 });
  const runner = useApi<Runner>("runner");
  // The cleanup panel's own answer (same path, same cache), for its question about the other agent programs.
  const hygiene = useApi<Hygiene>("hygiene?limit=30", { refresh: 60000 });
  const [allPast, setAllPast] = useState(false);
  if (q.error && !q.data)
    return (
      <>
        <PageTitle icon={ServerIcon} sub={SUB}>
          Machines
        </PageTitle>
        <ErrorNotice error={q.error} onRetry={() => void q.reload()} />
      </>
    );
  if (!q.data) return <SkeletonPage rows={3} />;
  const c = q.data;
  const pending = c.requests.filter((r) => r.status === "pending").sort(newestFirst);
  const runnersNote = hygiene.data?.runners;
  const asking = runnersAsking(runnersNote);
  // Approved requests that are starting or running are live and use hours, so
  // they get their own group; "Past requests" keeps the settled ones.
  const live = c.requests.filter((r) => LIVE.has(r.status)).sort(newestFirst);
  const past = c.requests.filter((r) => r.status !== "pending" && !LIVE.has(r.status)).sort(newestFirst);
  // The compute route returns the raw executor rows; the runner adds the live
  // running / free counts (and the local machine), so prefer it once loaded.
  // A removed machine is gone for good; it is kept in the store as history, not shown.
  const executors: Executor[] = (runner.data?.executors?.length ? runner.data.executors : (c.executors ?? []).map((e) => ({ ...e, running: e.running ?? 0, free: e.free ?? 0 }))).filter((e) => e.status !== "removed");
  const pastShown = allPast ? past : past.slice(0, PAST_SHOWN);
  const pastFailed = past.filter((r) => r.status === "failed").length;
  const active = executors.filter((e) => e.status === "active").length;
  const pairings = c.pairings ?? [];
  // A finished pairing's machine is in the executors list already; the row
  // under it is the report, kept until dismissed.
  const machinesNote = executors.length ? plural(active, "machine") + " taking work right now." : "Every machine your agents can use.";
  return (
    <>
      <PageTitle icon={ServerIcon} sub={SUB} actions={<AddMachineButton caps={c.caps} size="default" variant="default" />}>
        Machines
      </PageTitle>

      <div className="grid min-w-0 gap-10 [&>*]:min-w-0">
        <section aria-label="Waiting for you">
          <SectionHeading count={pending.length + (asking ? 1 : 0) || undefined} note={pending.length ? "An agent asked for an extra machine. It starts only if you approve." : undefined}>
            Waiting for you
          </SectionHeading>
          {asking ? <RunnersQuestion note={runnersNote} className={pending.length ? "mb-3 max-w-3xl" : "max-w-3xl"} /> : null}
          {pending.length ? (
            <CardGrid cols={2} className={pending.length === 1 ? "max-w-3xl" : undefined}>
              {pending.map((r) => (
                <ComputeInboxCard key={r.id} r={r} caps={c.caps} here />
              ))}
            </CardGrid>
          ) : asking ? null : (
            <p className="flex items-center gap-2.5 rounded-2xl border bg-card px-4 py-3.5 text-sm text-muted-foreground">
              <CheckCircle2Icon className="size-4 shrink-0 text-ok" aria-hidden />
              <span className="min-w-0 anywhere">Nothing to decide. When an agent asks for an extra machine, it shows up here.</span>
            </p>
          )}
        </section>

        {live.length ? (
          <section aria-label="In use now">
            <SectionHeading count={live.length} note="Extra machines you approved that are starting or running. Each one stops when its hours are up.">
              In use now
            </SectionHeading>
            <CardGrid cols={2} className={live.length === 1 ? "max-w-3xl" : undefined}>
              {live.map((r) => (
                <ComputeRequestCard key={r.id} r={r} />
              ))}
            </CardGrid>
          </section>
        ) : null}

        <div className="grid min-w-0 items-start gap-10 lg:grid-cols-2 lg:gap-6">
          <section aria-label="This machine" className="min-w-0">
            <SectionHeading note="The computer this hub runs on." action={<Link href={hrefs.settings("runner")} className={quietLink}>Runner settings</Link>}>
              This machine
            </SectionHeading>
            <MachinePanel r={runner.data} heading={false} machines={false} />
          </section>

          <div className="grid min-w-0 gap-10 lg:gap-6">
            <section aria-label="All machines" className="min-w-0">
              <SectionHeading count={executors.length || undefined} note={machinesNote}>
                All machines
              </SectionHeading>
              {executors.length || pairings.length ? (
                <ul className="grid gap-2">
                  {pairings.map((p) => (
                    <PairingRow key={"pairing:" + p.name + ":" + p.started} p={p} />
                  ))}
                  {executors.map((e) => (
                    // The local executor is this machine and a removed row is already gone: neither gets Remove.
                    <ExecutorRow key={e.name} e={e} action={e.name !== "local" ? <RemoveMenuButton kind="executor" name={e.name} compact /> : undefined} />
                  ))}
                </ul>
              ) : (
                <EmptyState action={<AddMachineButton caps={c.caps} />}>No other machine yet. Add one and PULSE sets it up over ssh, or approve an agent&apos;s request. It starts taking work after its first check passes.</EmptyState>
              )}
            </section>

            <section aria-label="Limits" className="min-w-0">
              <SectionHeading note="Every request for an extra machine has to fit inside these.">Limits</SectionHeading>
              <Panel>
                <CapsPanel caps={c.caps} />
              </Panel>
            </section>
          </div>
        </div>

        <section aria-label="Past requests">
          <SectionHeading count={past.length || undefined} note={past.length ? "Earlier requests that are over" + (pastFailed ? ", " + pastFailed + " of them failed" : "") + ". Newest first." : undefined}>
            Past requests
          </SectionHeading>
          {past.length ? (
            <div className="grid gap-2">
              <ul className="grid gap-2">
                {pastShown.map((r) => (
                  <ComputeRequestRow key={r.id} r={r} />
                ))}
              </ul>
              {past.length > PAST_SHOWN ? (
                <Button variant="ghost" size="sm" className="justify-self-start" onClick={() => setAllPast((v) => !v)}>
                  {allPast ? "Show fewer" : "Show all " + past.length}
                </Button>
              ) : null}
            </div>
          ) : (
            <p className="text-sm text-muted-foreground">Nothing here yet. Requests you deny, and machines whose time is up, move here.</p>
          )}
        </section>

        {/* Housekeeping, folded: agents clean up after themselves and the runner sweeps every 10 minutes, so it rarely needs a look. */}
        <section aria-label="Cleanup" className="border-t pt-4">
          <Disclosure variant="row" summary={<span className="text-[15px] font-semibold">Cleanup</span>} hint="Runs every 10 minutes" contentClassName="mt-3">
            <p className="mb-3 text-sm text-muted-foreground">Agents clean up after themselves; the runner cleans up what&apos;s left every 10 minutes.</p>
            <RunnersAskedElsewhere.Provider value={true}>
              <HygienePanel heading={false} />
            </RunnersAskedElsewhere.Provider>
          </Disclosure>
        </section>
      </div>
    </>
  );
}
