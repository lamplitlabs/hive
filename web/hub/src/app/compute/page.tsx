"use client";

import { useState } from "react";
import { CheckCircle2Icon } from "lucide-react";
import { AddMachineButton, PairingRow } from "@/components/compute/add-machine";
import { MachineCard, RevokeDeviceButton } from "@/components/compute/machine";
import { ThisMachineCard } from "@/components/compute/this-machine";
import { CapsPanel, ComputeRequestCard, ComputeRequestListRow, ComputeRequestRow } from "@/components/compute/compute";
import { requestState, waitingNote } from "@/components/compute/compute-state";
import { HygienePanel } from "@/components/hive/hygiene";
import { RunnersAskedElsewhere, RunnersQuestion, runnersAsking } from "@/components/hive/runners";
import { Button } from "@/components/ui/button";
import { CardGrid } from "@/components/hive/item-card";
import { PageHeader } from "@/components/hive/page-header";
import { Disclosure, EmptyState, ErrorNotice, SkeletonPage } from "@/components/hive/panel";
import { SectionHeading } from "@/components/hive/section-heading";
import { RemoveMenuButton } from "@/components/project/project-dialogs";
import { plural } from "@/lib/format";
import type { Compute, ComputeRequest, Executor, Hygiene, Org, Runner } from "@/lib/api";
import { useApi } from "@/lib/use-api";

const SUB = "Where your agents run";

const newestFirst = (a: ComputeRequest, b: ComputeRequest) => (b.at || "").localeCompare(a.at || "");

const LIVE = new Set(["approved", "provisioning", "ready"]);

// Past requests show the latest few; the rest wait behind "Show all".
const PAST_SHOWN = 5;

// The Machines page (spec "Machines"): this machine first as a hero card,
// the requests waiting on you as compact rows, every machine as a card,
// the limits in one card, and the past requests and cleanup folded at the
// bottom. One primary action: Add machine.
export default function ComputePage() {
  const q = useApi<Compute>("compute", { refresh: 10000 });
  const runner = useApi<Runner>("runner");
  const org = useApi<Org>("org");
  // The cleanup panel's own answer (same path, same cache), for its question about the other agent programs.
  const hygiene = useApi<Hygiene>("hygiene?limit=30", { refresh: 60000 });
  const [allPast, setAllPast] = useState(false);
  if (q.error && !q.data)
    return (
      <>
        <PageHeader title="Machines" subtitle={SUB} />
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
  // A request a limit refused waits for the owner's answer (Rent it anyway / raise / Do not rent,
  // TASK-165): it sits under Waiting for you, not among the past ones.
  const notRented = c.requests.filter((r) => !!r.limit_hit).sort(newestFirst);
  const past = c.requests.filter((r) => r.status !== "pending" && !LIVE.has(r.status) && !r.limit_hit).sort(newestFirst);
  // The compute route returns the raw executor rows; the runner adds the live
  // running / free counts (and the local machine), so prefer it once loaded.
  // A removed machine is gone for good; it is kept in the store as history, not shown.
  const executors: Executor[] = (runner.data?.executors?.length ? runner.data.executors : (c.executors ?? []).map((e) => ({ ...e, running: e.running ?? 0, free: e.free ?? 0 }))).filter((e) => e.status !== "removed");
  const local = executors.find((e) => e.kind === "local" || e.name === "local") ?? null;
  const pastShown = allPast ? past : past.slice(0, PAST_SHOWN);
  const pastFailed = past.filter((r) => requestState(r).bad).length;
  const active = executors.filter((e) => e.status === "active").length;
  const pairings = c.pairings ?? [];
  // A finished pairing's machine is in the executors list already; the row
  // under it is the report, kept until dismissed.
  const machinesNote = executors.length ? plural(active, "machine") + " taking work right now." : "Every machine your agents can use.";
  const waiting = pending.length + notRented.length + (asking ? 1 : 0);
  return (
    <>
      <PageHeader title="Machines" subtitle={SUB} action={<AddMachineButton caps={c.caps} size="default" variant="default" />} />

      <div className="grid min-w-0 gap-8 [&>*]:min-w-0">
        {/* The card is the section named "This machine"; a second one around it would name the same landmark twice. */}
        <div>
          <ThisMachineCard runner={runner.data === undefined && runner.error ? null : runner.data} host={org.data?.host} local={local} />
        </div>

        {waiting ? (
          <section aria-label="Waiting for you">
            <SectionHeading count={waiting} note={waitingNote(pending.length + notRented.length, asking)}>
              Waiting for you
            </SectionHeading>
            {asking ? <RunnersQuestion note={runnersNote} className={pending.length ? "mb-3" : undefined} /> : null}
            {pending.length ? (
              <ul className="grid gap-2" aria-label="Requests">
                {pending.map((r) => (
                  <li key={r.id}>
                    <ComputeRequestListRow r={r} />
                  </li>
                ))}
              </ul>
            ) : null}
            {notRented.length ? (
              <ul className={pending.length ? "mt-2 grid gap-2" : "grid gap-2"} aria-label="Not rented">
                {notRented.map((r) => (
                  <ComputeRequestRow key={r.id} r={r} />
                ))}
              </ul>
            ) : null}
          </section>
        ) : null}

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

        <section aria-label="All machines">
          <SectionHeading count={executors.length || undefined} note={machinesNote}>
            All machines
          </SectionHeading>
          {executors.length || pairings.length ? (
            <div className="grid gap-3">
              {pairings.length ? (
                <ul className="grid gap-2" aria-label="Machines being set up">
                  {pairings.map((p) => (
                    <PairingRow key={"pairing:" + p.name + ":" + p.started} p={p} />
                  ))}
                </ul>
              ) : null}
              {executors.length ? (
                <CardGrid cols={3}>
                  {executors.map((e) => {
                    // A machine that dials in shows its key's last call and gets Revoke instead of Remove (ADR-070 step 2).
                    const device = e.kind === "dial" ? c.devices?.find((d) => d.executor === e.name) : undefined;
                    // The local executor is this machine and a removed row is already gone: neither gets Remove.
                    const action = device && !device.revoked_at ? <RevokeDeviceButton device={device} name={e.name} /> : e.name !== "local" ? <RemoveMenuButton kind="executor" name={e.name} compact /> : undefined;
                    return <MachineCard key={e.name} e={e} provider={c.caps.provider} host={org.data?.host} device={device} action={action} />;
                  })}
                </CardGrid>
              ) : null}
            </div>
          ) : (
            <EmptyState action={<AddMachineButton caps={c.caps} />}>No other machine yet. Add one with a join code, have Hive set it up over ssh, or approve an agent&apos;s request. It starts taking work after its first check passes.</EmptyState>
          )}
        </section>

        <section aria-label="Limits">
          <SectionHeading note="What agents and the daemon may rent by themselves. Your own requests are never refused by them.">Limits</SectionHeading>
          <CapsPanel caps={c.caps} />
        </section>

        {!waiting ? (
          <p className="flex items-center gap-2.5 text-[13px] text-muted-foreground">
            <CheckCircle2Icon className="size-4 shrink-0 text-ok" aria-hidden />
            <span className="min-w-0 anywhere">Nothing to decide. When an agent asks for an extra machine, it shows up at the top of this page.</span>
          </p>
        ) : null}

        <section aria-label="Past requests" className="border-t pt-4">
          <Disclosure variant="row" summary={<span className="text-base font-semibold">Past requests</span>} hint={past.length ? plural(past.length, "request") + (pastFailed ? ", " + pastFailed + " failed" : "") : "None yet"} contentClassName="mt-3">
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
          </Disclosure>
        </section>

        {/* Housekeeping, folded: agents clean up after themselves and the runner sweeps every 10 minutes, so it rarely needs a look. */}
        <section aria-label="Cleanup" className="border-t pt-4">
          <Disclosure variant="row" summary={<span className="text-base font-semibold">Cleanup</span>} hint="Runs every 10 minutes" contentClassName="mt-3">
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
