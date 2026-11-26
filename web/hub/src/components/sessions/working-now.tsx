"use client";

import Link from "next/link";
import { ActivityIcon } from "lucide-react";
import { ProjectAvatar } from "@/components/hive/avatar";
import { workingRows } from "@/components/hive/cards";
import { ListRow, ListRows } from "@/components/hive/list-row";
import { SectionHeading } from "@/components/hive/section-heading";
import { StatusBadge } from "@/components/hive/status-badge";
import { Avatar, When } from "@/components/hive/who";
import type { Claim, Task } from "@/lib/api";
import { sentence } from "@/lib/format";
import { hrefs } from "@/lib/nav";
import { WORKING_SHOWN } from "./sessions-state";

// "Working now" (on Home until TASK-129, now under the new-session box on
// Sessions): the agents at work, one row each - the
// agent's face, "agent · what", the project and when it started, and a
// running badge. The rows come from workingRows (the same fold the project
// page uses: a task's workers first, then the claims on the board). At most
// six; "See all runs" leads to the Runs page. Nothing running, nothing shown.
export function WorkingNow({ claims, tasks, names }: { claims: Claim[]; tasks: Task[]; names: Map<string, string> }) {
  const rows = workingRows(claims, tasks);
  const shown = rows.slice(0, WORKING_SHOWN);
  if (!shown.length) return null;
  return (
    <section aria-labelledby="sessions-working-now" className="min-w-0">
      <SectionHeading id="sessions-working-now" icon={ActivityIcon} count={rows.length} action={rows.length ? <Link href={hrefs.harness({ status: "running" })}>See all runs</Link> : undefined}>
        Working now
      </SectionHeading>
      <ListRows ariaLabel="Agents working now">
          {shown.map((r) => {
            const agent = r.agent === "(unknown)" ? "An agent" : r.agent;
            const latest = r.claims[0];
            const what = r.task ? sentence(r.task.w.title || r.task.t.title || r.task.w.kind).replace(/[.!?]$/, "") : latest ? sentence(latest.summary).replace(/[.!?]$/, "") : "working";
            const since = r.task?.w.since || latest?.date || latest?.expires || "";
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
                // The words the row shows, then where: the name has to hold what is seen.
                ariaLabel={agent + " · " + what + (name ? ", working in " + name : "")}
              />
            );
          })}
      </ListRows>
    </section>
  );
}
