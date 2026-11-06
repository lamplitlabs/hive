"use client";

import Link from "next/link";
import { ActivityIcon } from "lucide-react";
import { ProjectAvatar } from "@/components/pulse/avatar";
import { workingRows } from "@/components/pulse/cards";
import { ListRow, ListRows } from "@/components/pulse/list-row";
import { SectionHeading } from "@/components/pulse/section-heading";
import { StatusBadge } from "@/components/pulse/status-badge";
import { Avatar, When } from "@/components/pulse/who";
import type { Claim, Task } from "@/lib/api";
import { sentence } from "@/lib/format";
import { hrefs } from "@/lib/nav";
import { WORKING_SHOWN } from "./home-state";

// "Working now" (spec "Home" 5): the agents at work, one row each - the
// agent's face, "agent · what", the project and when it started, and a
// running badge. The rows come from workingRows (the same fold the project
// page uses: a task's workers first, then the claims on the board). At most
// six; "See all runs" leads to the Runs page.
export function WorkingNow({ claims, tasks, names }: { claims: Claim[]; tasks: Task[]; names: Map<string, string> }) {
  const rows = workingRows(claims, tasks);
  const shown = rows.slice(0, WORKING_SHOWN);
  return (
    <section aria-labelledby="home-working-now" className="min-w-0">
      <SectionHeading id="home-working-now" icon={ActivityIcon} count={rows.length} action={rows.length ? <Link href={hrefs.harness({ status: "running" })}>See all runs</Link> : undefined}>
        Working now
      </SectionHeading>
      {shown.length ? (
        <ListRows ariaLabel="Agents working now">
          {shown.map((r) => {
            const agent = r.agent === "(unknown)" ? "An agent" : r.agent;
            const latest = r.claims[0];
            const what = r.task ? sentence(r.task.w.title || r.task.t.title || r.task.w.kind).replace(/[.!?]$/, "") : latest ? sentence(latest.summary).replace(/[.!?]$/, "") : "working";
            const since = r.task?.w.since || latest?.expires || "";
            const name = names.get(r.slug) || r.slug;
            const href = r.task ? hrefs.task(r.task.t.id) : latest && r.slug ? hrefs.post(r.slug, latest.id) : hrefs.harness({ agent: r.agent });
            return (
              <ListRow
                key={r.key}
                lead={<Avatar seed={r.agent} size={28} />}
                title={
                  <>
                    {agent}
                    <span className="text-muted-foreground"> · </span>
                    <span className="font-normal">{what}</span>
                  </>
                }
                meta={
                  r.slug ? (
                    <>
                      <ProjectAvatar slug={r.slug} name={name} size={20} />
                      <span className="truncate">{name}</span>
                    </>
                  ) : undefined
                }
                status={<StatusBadge status="running" label="Running" size="sm" />}
                time={since ? <When d={since} /> : undefined}
                href={href}
                ariaLabel={agent + " is working on " + what + (name ? " in " + name : "")}
              />
            );
          })}
        </ListRows>
      ) : (
        <p className="rounded-lg border border-dashed px-4 py-3 text-sm text-muted-foreground">Nothing is running.</p>
      )}
    </section>
  );
}
