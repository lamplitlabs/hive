"use client";

import Link from "next/link";
import { ArrowRightIcon, CheckIcon, ServerIcon, XIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useAnswerDialog } from "@/components/hive/use-answer-dialog";
import { computeAnswer, type ComputeAnswer } from "@/lib/answer-dialog";
import { ConfirmDialog } from "@/components/hive/confirm";
import { ItemCard, ItemCardActions, ItemCardBody, ItemCardHeader, ItemCardKind, ItemCardMeta, ItemCardTitle } from "@/components/hive/item-card";
import { ListRow } from "@/components/hive/list-row";
import { StatusBadge } from "@/components/hive/status-badge";
import { Avatar, Mono, ProjectLink, When } from "@/components/hive/who";
import { agentName, statusWord } from "@/lib/words";
import { writes, type ComputeCaps, type ComputeRequest } from "@/lib/api";
import { num, plural } from "@/lib/format";
import { hrefs } from "@/lib/nav";
import { ANY_PROJECT, ANY_PROJECT_WORD, meterPercent, requestReason, requestTitle } from "./compute-state";
import { Meter } from "./meter";

const CHANGES = (p: string) => p.startsWith("compute") || p.startsWith("executors") || p.startsWith("events") || p.startsWith("runner");

// Approve / deny buttons for one pending request; each confirms with words.
// Approve is the primary move, Deny stays quiet in the bad tone; `sm` by
// default so they are easy to hit on a card, `xs` for dense rows.
export function ComputeDecide({ r, size = "sm" }: { r: ComputeRequest; size?: "xs" | "sm" | "default" }) {
  // Keyed by each opening (lib/answer-dialog.ts): a note typed under Deny is
  // never sent with Approve, and the dialog closes with its exit animation.
  const { answer, key, pick, dialog } = useAnswerDialog<ComputeAnswer>();
  if (r.status !== "pending") return null;
  const approve = answer === "approve";
  const spec = computeAnswer(r.id, answer ?? "approve");
  return (
    <span className="inline-flex flex-wrap items-center gap-2">
      <Button size={size} variant="outline" onClick={() => pick("deny")} aria-label={"Deny " + r.id}>
        <XIcon /> Deny
      </Button>
      <Button size={size} onClick={() => pick("approve")} aria-label={"Approve " + r.id}>
        <CheckIcon /> Approve
      </Button>
      <ConfirmDialog
        key={key}
        {...dialog}
        title={approve ? "Approve the extra machine?" : "Deny the extra machine?"}
        description={
          <span>
            {agentName(r.requested_by)} asks for one extra machine (<Mono>{r.size}</Mono>) for {plural(r.hours, "hour")}. {approve ? "Hive starts it within the limits you set, and removes it when the time is up." : "The agent sees your answer the next time it checks."}
          </span>
        }
        action={spec.action}
        destructive={spec.destructive}
        words={spec.words}
        success={spec.success}
        onConfirm={(w) => spec.send(writes, w)}
        invalidates={CHANGES}
      />
    </span>
  );
}

// Approved requests that are starting or running: their machine is live.
const LIVE = new Set(["approved", "provisioning", "ready"]);

// Whose work a request's machine takes: a link to the one project it was
// asked for, or "Any project" - plain words, since it has no one page - for
// a machine every project can use.
function RequestProject({ slug, className }: { slug: string; className?: string }) {
  if (slug === ANY_PROJECT)
    return (
      <span className={className} title="Every project's work can run on this machine">
        {ANY_PROJECT_WORD}
      </span>
    );
  return <ProjectLink slug={slug} className={className} />;
}

// One request as a card: who asked for what and why, with Approve / Deny in
// reach while it is pending and your answer once made. There is no detail
// page, so the card is a plain article; the request id sits in tooltips.
// `compact` clamps the reason; `caps` adds the limits to a pending card. The
// daemon's note - the step a provisioning is at, why a request failed or its
// machine was released - sits under the reason, so a failed request says what
// went wrong where it is, not only in the daemon log.
export function ComputeRequestCard({ r, compact, caps }: { r: ComputeRequest; compact?: boolean; caps?: ComputeCaps }) {
  const pending = r.status === "pending";
  const decider = r.decided_by === "human" || r.decided_by === "owner" ? "you" : r.decided_by;
  const note = (r.note ?? "").trim();
  const noteLead = r.status === "failed" ? "Failed: " : r.status === "released" ? "Released: " : r.status === "provisioning" ? "Starting: " : "";
  return (
    <ItemCard highlight={pending} ariaLabel={"Request " + r.id + ": " + statusWord(r.status)}>
      <ItemCardHeader right={<When d={r.at} />}>
        <ItemCardKind icon={ServerIcon} tone={pending ? "you" : LIVE.has(r.status) ? "ok" : "neutral"}>{"Extra machine for " + agentName(r.requested_by)}</ItemCardKind>
        <RequestProject slug={r.project} className="font-normal text-muted-foreground" />
      </ItemCardHeader>
      <ItemCardTitle>
        <span title={"Request " + r.id + ", machine type " + r.size}>{"One extra machine for " + plural(r.hours, "hour")}</span>
      </ItemCardTitle>
      {r.reason ? <ItemCardBody clamp={compact ? 3 : 0}>{"\u201c" + r.reason.trim() + "\u201d"}</ItemCardBody> : null}
      {note ? (
        <ItemCardBody clamp={compact ? 3 : 0} className={r.status === "failed" ? "text-bad" : undefined}>
          {noteLead ? <span className="font-medium">{noteLead}</span> : null}
          {note}
        </ItemCardBody>
      ) : null}
      <ItemCardMeta>
        <StatusBadge status={r.status} />
        {decider && !pending ? <span>{(r.status === "denied" ? "Denied" : "Approved") + " by " + decider}</span> : null}
        {r.executor ? <span>{"Runs on " + r.executor}</span> : null}
        {pending && caps ? (
          <>
            <span>{num(caps.vms_now) + " of " + num(caps.max_vms) + " extra machines in use"}</span>
            <span>{num(caps.hours_today) + " of " + plural(caps.max_hours_per_day, "hour") + " used today"}</span>
          </>
        ) : null}
      </ItemCardMeta>
      {pending ? (
        <ItemCardActions className="justify-end">
          <ComputeDecide r={r} />
        </ItemCardActions>
      ) : null}
    </ItemCard>
  );
}

// A request waiting on you as one compact row (spec "Machines"): the
// asking agent's face, "Extra machine for 8 h · demo-lib" ("… · Any project"
// for a machine every project can use), the reason in
// one muted line, Deny / Approve at the right (ComputeDecide confirms each).
export function ComputeRequestListRow({ r }: { r: ComputeRequest }) {
  const reason = requestReason(r.reason);
  return (
    <ListRow
      lead={<Avatar seed={r.requested_by} size={28} />}
      title={<span title={"Request " + r.id + " from " + agentName(r.requested_by) + ", machine type " + r.size}>{requestTitle(r)}</span>}
      meta={<When d={r.at} />}
      // Deny / Approve stay in sight, not only on hover; on a phone they go
      // under the text, so the title keeps its words.
      actions={<ComputeDecide r={r} size="xs" />}
      actionsVisible
      className="min-h-14 py-2"
      ariaLabel={"Request " + r.id + " from " + agentName(r.requested_by)}
    >
      {reason ? <span title={r.reason.trim()}>{agentName(r.requested_by) + ": " + reason}</span> : agentName(r.requested_by) + " gave no reason"}
    </ListRow>
  );
}

// One settled request as a single line: what, when, and - for a failure - why.
// Past requests pile up (every failed automatic request is one), so they are
// rows, not cards.
export function ComputeRequestRow({ r }: { r: ComputeRequest }) {
  const note = (r.note ?? "").trim();
  const failed = r.status === "failed";
  return (
    <li className="flex min-w-0 items-start gap-3 rounded-lg border bg-card px-3 py-2.5 text-sm" aria-label={"Request " + r.id + ": " + statusWord(r.status)}>
      <div className="min-w-0 flex-1" title={r.reason ? "Asked because: " + r.reason.trim() : undefined}>
        <div className="flex min-w-0 flex-wrap items-baseline gap-x-2">
          <span className="font-medium text-foreground" title={"Request " + r.id + ", machine type " + r.size}>
            {"Extra machine for " + plural(r.hours, "hour")}
          </span>
          <span className="text-[13px] text-muted-foreground">{"asked by " + agentName(r.requested_by)}</span>
          <RequestProject slug={r.project} className="text-[13px] font-normal text-muted-foreground" />
          <span className="ml-auto flex items-center gap-2 text-[13px] text-muted-foreground">
            <StatusBadge status={r.status} />
            <When d={r.at} />
          </span>
        </div>
        {note ? (
          <p className={"mt-1 text-[13px] leading-snug anywhere " + (failed ? "text-bad" : "text-muted-foreground")}>{(failed ? "Why it failed: " : "") + note}</p>
        ) : failed ? (
          <p className="mt-1 text-[13px] text-muted-foreground">No reason was recorded for this one.</p>
        ) : null}
      </div>
    </li>
  );
}

const PROVIDER: Record<string, string> = { azure: "Microsoft Azure", aws: "Amazon Web Services", gcp: "Google Cloud", google: "Google Cloud", hetzner: "Hetzner Cloud", digitalocean: "DigitalOcean", ssh: "Your own machines over SSH" };
export const providerWord = (p?: string | null) => (p ? (PROVIDER[p.toLowerCase()] ?? p.charAt(0).toUpperCase() + p.slice(1)) : "");

// The limits every request must fit (spec "Machines" - Limits): one card
// with two meters, machines in use and hours today, and the budget line.
// They are changed in Settings, linked at the end.
export function CapsPanel({ caps }: { caps: ComputeCaps }) {
  const auto = caps.auto_approve === true || caps.auto_approve === 1;
  const full = caps.max_hours_per_day > 0 && caps.hours_today >= caps.max_hours_per_day;
  return (
    <div className="grid gap-4 rounded-lg border bg-card p-4">
      <div className="grid gap-4 sm:grid-cols-2">
        <Meter label="Machines in use" pct={meterPercent(caps.vms_now, caps.max_vms)} text={num(caps.vms_now) + " of " + num(caps.max_vms)} title="Extra machines running now, of the most allowed at once" />
        <Meter label="Hours today" pct={meterPercent(caps.hours_today, caps.max_hours_per_day)} text={num(caps.hours_today) + " of " + num(caps.max_hours_per_day) + (full ? " · none left" : "")} title="Machine hours used today, of the most allowed per day" />
      </div>
      <p className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-[13px] text-muted-foreground">
        <span className="text-foreground">{"Budget " + num(caps.budget_credits) + " credits"}</span>
        <span title={caps.provider || undefined}>{providerWord(caps.provider) || "No cloud provider yet"}</span>
        <span>{auto ? "Auto-approve on" : "Every request waits for you"}</span>
        <Link className="ml-auto font-medium text-link underline-offset-2 hover:underline" href={hrefs.settings("compute")}>
          Change in Settings
        </Link>
      </p>
    </div>
  );
}

// One pending compute request as an inbox card: who asks for what, for how
// long and why, with Approve / Deny in reach (ComputeDecide confirms each).
// `here` drops the "All requests" link when the card already sits on the Machines page.
export function ComputeInboxCard({ r, caps, here }: { r: ComputeRequest; caps?: ComputeCaps; here?: boolean }) {
  return (
    <ItemCard highlight ariaLabel={"Compute request " + r.id + " from " + r.requested_by}>
      <ItemCardHeader right={<When d={r.at} />}>
        <ItemCardKind icon={ServerIcon} tone="you">{"Extra machine for " + agentName(r.requested_by)}</ItemCardKind>
        <RequestProject slug={r.project} className="font-normal text-muted-foreground" />
      </ItemCardHeader>
      <ItemCardTitle>
        <span title={"Machine type " + r.size}>{"One extra machine for " + plural(r.hours, "hour")}</span>
      </ItemCardTitle>
      {r.reason ? <ItemCardBody clamp={3}>{"“" + r.reason.trim() + "”"}</ItemCardBody> : null}
      {caps ? (
        <ItemCardMeta>
          <span>{num(caps.vms_now) + " of " + num(caps.max_vms) + " extra machines in use"}</span>
          <span>{num(caps.hours_today) + " of " + plural(caps.max_hours_per_day, "hour") + " used today"}</span>
          {caps.provider ? <span title={caps.provider}>{"Runs on " + providerWord(caps.provider)}</span> : null}
        </ItemCardMeta>
      ) : null}
      <ItemCardActions>
        {here ? null : (
        <Link href={hrefs.compute()} title={"Request " + r.id} className="inline-flex items-center gap-1 text-[13px] font-medium text-muted-foreground underline-offset-2 hover:text-foreground hover:underline">
          All requests <ArrowRightIcon className="size-3.5" aria-hidden />
        </Link>
        )}
        <span className="flex w-full justify-start sm:ml-auto sm:w-auto sm:justify-end">
          <ComputeDecide r={r} size="default" />
        </span>
      </ItemCardActions>
    </ItemCard>
  );
}
